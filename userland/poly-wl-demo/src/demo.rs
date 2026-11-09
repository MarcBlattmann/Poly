//! `/bin/wl-demo`: a small interactive Wayland client for Poly's desktop
//! (ADR 0107), the launcher's "Poly Demo".
//!
//! It opens a 360x240 toplevel with a server-side decoration and stays
//! until its window is closed (the frame's close button) or Q is pressed.
//! When the compositor configures another size (maximized, tiled,
//! restored; ADR 0117) it draws a buffer of that size and says so
//! (`wl-demo: size WxH`). It binds the seat and reports what reaches it,
//! one line each, so a boot test can check the compositor's input routing:
//! the keymap (its handle mapped and checked to hold an XKB keymap),
//! keyboard enter, keys, modifiers, pointer enter, motion (the first only),
//! buttons and the wheel. A left click swaps the window's colour
//! ([`DEMO_A`] and [`DEMO_B`]) and commits again, which shows on the screen.

use crate::wire::{Arg, Conn, DISPLAY, Event, bind_args};
use alloc::string::String;
use poly_rt::abi::Handle;
use poly_rt::fs::File;
use poly_rt::mem;
use poly_rt::println;

/// The window's size until the compositor asks for another.
pub const WIDTH: u32 = 360;
pub const HEIGHT: u32 = 240;
/// The window's two looks, a teal and an amber, with a lighter band.
pub const DEMO_A: u32 = 0x2f7d6d;
pub const DEMO_B: u32 = 0xc08a2e;
pub const BAND: u32 = 0xe8eef0;

const FORMAT_XRGB8888: u32 = 1;
const MODE_SERVER_SIDE: u32 = 2;
const KEY_Q: u32 = 16;
const BTN_LEFT: u32 = 0x110;

/// Draw the window into `pixels` (`width` x `height`, xrgb8888): the
/// colour with the band at the same share of the height as in the 240-row
/// window.
pub fn draw(pixels: &mut [u32], width: u32, height: u32, colour_b: bool) {
    let base = if colour_b { DEMO_B } else { DEMO_A };
    let (top, bottom) = (height * 100 / HEIGHT, height * 140 / HEIGHT);
    for (y, row) in pixels.chunks_mut(width.max(1) as usize).take(height as usize).enumerate() {
        let colour = if (top..bottom).contains(&(y as u32)) { BAND } else { base };
        row.fill(colour);
    }
}

/// Whether `map` (a `wl_keyboard.keymap` of `size` bytes) is an XKB
/// keymap in text form: `xkb_keymap {` first, NUL-terminated.
pub fn is_xkb_keymap(map: &[u8]) -> bool {
    map.len() > 16 && map.starts_with(b"xkb_keymap {") && map.last() == Some(&0)
}

/// Memory shared with the compositor: a memory object, mapped.
#[derive(Debug)]
pub struct Shared {
    pub file: File,
    ptr: *mut u8,
    len: usize,
}

impl Shared {
    /// `len` bytes, zeroed.
    pub fn new(len: usize) -> Result<Shared, String> {
        let file = mem::memory_object(len as u64).map_err(|e| alloc::format!("a memory object: {e:?}"))?;
        let ptr = mem::map_handle(file.handle(), 0, len, mem::READ | mem::WRITE, true)
            .map_err(|e| alloc::format!("map the memory object: {e:?}"))?;
        Ok(Shared { file, ptr, len })
    }

    /// The memory as pixels.
    pub fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: `ptr` is a page-aligned mapping of `len` bytes, readable
        // and writable, ours until `drop`; `&mut self` keeps it unaliased
        // on our side (the compositor only reads it).
        unsafe { core::slice::from_raw_parts_mut(self.ptr.cast::<u32>(), self.len / 4) }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        // SAFETY: the mapping is ours and nothing refers to it any more
        // (`pixels` borrows `self`).
        let _ = unsafe { mem::unmap(self.ptr, self.len) };
    }
}

/// Map a keymap's handle and check it ([`is_xkb_keymap`]); the handle is closed.
fn keymap_ok(handle: Option<Handle>, size: u32) -> bool {
    let Some(h) = handle else { return false };
    let file = File::from_handle(h);
    if size <= 16 {
        return false;
    }
    let Ok(ptr) = mem::map_handle(file.handle(), 0, size as usize, mem::READ, false) else { return false };
    // SAFETY: a fresh private read-only mapping of `size` bytes.
    let ok = is_xkb_keymap(unsafe { core::slice::from_raw_parts(ptr, size as usize) });
    // SAFETY: the mapping is ours and the slice above is gone.
    let _ = unsafe { mem::unmap(ptr, size as usize) };
    ok
}

#[derive(Debug, Default)]
struct Ids {
    registry: u32,
    compositor: u32,
    shm: u32,
    wm_base: u32,
    decoration_manager: u32,
    seat: u32,
    surface: u32,
    xdg_surface: u32,
    toplevel: u32,
    buffer: u32,
    pointer: u32,
    keyboard: u32,
    sync: u32,
}

#[derive(Debug, Default)]
struct Names {
    compositor: u32,
    shm: u32,
    wm_base: u32,
    decoration: u32,
    seat: u32,
}

struct Demo {
    c: Conn,
    id: Ids,
    names: Names,
    shared: Option<Shared>,
    width: u32,
    height: u32,
    pending: (u32, u32),
    configured: bool,
    closed: bool,
    synced: bool,
    motion_reported: bool,
    colour_b: bool,
}

impl Demo {
    fn bind(&mut self, name: u32, iface: &str, version: u32) -> u32 {
        let id = self.c.new_id();
        self.c.request(self.id.registry, 0, &bind_args(name, iface, version, id));
        id
    }

    fn draw(&mut self) {
        let (w, h, b) = (self.width, self.height, self.colour_b);
        if let Some(s) = self.shared.as_mut() {
            draw(s.pixels(), w, h, b);
        }
    }

    /// A buffer of the window's size in memory of its own, shared as a
    /// pool of its own; the old buffer goes.
    fn make_buffer(&mut self) -> Result<(), String> {
        let size = self.width * self.height * 4;
        let shared = Shared::new(size as usize)?;
        let handle = shared.file.handle();
        self.shared = Some(shared);
        self.draw();
        if self.id.buffer != 0 {
            self.c.request(self.id.buffer, 0, &[]); // wl_buffer.destroy
        }
        let pool = self.c.new_id();
        // wl_shm.create_pool(new_id, fd, size)
        self.c.request_with_handle(self.id.shm, 0, &[Arg::U(pool), Arg::U(size)], handle);
        self.id.buffer = self.c.new_id();
        let (w, h) = (self.width, self.height);
        let args = [Arg::U(self.id.buffer), Arg::U(0), Arg::U(w), Arg::U(h), Arg::U(w * 4), Arg::U(FORMAT_XRGB8888)];
        self.c.request(pool, 0, &args); // create_buffer
        self.c.request(pool, 1, &[]); // destroy the pool: the buffer keeps it
        Ok(())
    }

    fn commit(&mut self) {
        let s = self.id.surface;
        self.c.request(s, 1, &[Arg::U(self.id.buffer), Arg::U(0), Arg::U(0)]); // attach
        self.c.request(s, 9, &[Arg::U(0), Arg::U(0), Arg::U(self.width), Arg::U(self.height)]); // damage_buffer
        self.c.request(s, 6, &[]); // commit
        self.flush();
    }

    fn flush(&mut self) {
        if let Err(e) = self.c.flush() {
            println!("wl-demo: {e}");
        }
    }

    fn handle(&mut self, e: &Event) -> Result<(), String> {
        let id = &self.id;
        if e.object == DISPLAY && e.opcode == 0 {
            println!("wl-demo: protocol error on object {}, code {}: {}", e.arg(0), e.arg(1), e.string(2));
            return Err("protocol error".into());
        } else if e.object == id.registry && e.opcode == 0 {
            let name = e.arg(0);
            match e.string(1).as_str() {
                "wl_compositor" => self.names.compositor = name,
                "wl_shm" => self.names.shm = name,
                "xdg_wm_base" => self.names.wm_base = name,
                "zxdg_decoration_manager_v1" => self.names.decoration = name,
                "wl_seat" => self.names.seat = name,
                _ => {}
            }
        } else if e.object == id.sync && e.opcode == 0 {
            self.synced = true;
        } else if e.object == id.wm_base && e.opcode == 0 {
            let wm = id.wm_base;
            self.c.request(wm, 3, &[Arg::U(e.arg(0))]); // pong
            self.flush();
        } else if e.object == id.toplevel && e.opcode == 0 {
            // The size to use (0: ours to choose).
            self.pending = (e.arg(0), e.arg(1));
        } else if e.object == id.xdg_surface && e.opcode == 0 {
            let xs = id.xdg_surface;
            self.c.request(xs, 4, &[Arg::U(e.arg(0))]); // ack_configure
            if !self.configured {
                println!("wl-demo: configured");
            }
            self.configured = true;
            let (w, h) = self.pending;
            let fits = |v: u32| v > 0 && v <= 8192;
            if fits(w) && fits(h) && (w, h) != (self.width, self.height) {
                (self.width, self.height) = (w, h);
                self.make_buffer()?;
                println!("wl-demo: size {w}x{h}");
            }
            self.commit();
        } else if e.object == id.toplevel && e.opcode == 1 {
            self.closed = true;
        } else if e.object == id.seat && e.opcode == 0 {
            let caps = e.arg(0);
            let seat = id.seat;
            if caps & 1 != 0 && self.id.pointer == 0 {
                self.id.pointer = self.c.new_id();
                let p = self.id.pointer;
                self.c.request(seat, 0, &[Arg::U(p)]); // get_pointer
            }
            if caps & 2 != 0 && self.id.keyboard == 0 {
                self.id.keyboard = self.c.new_id();
                let k = self.id.keyboard;
                self.c.request(seat, 1, &[Arg::U(k)]); // get_keyboard
            }
            self.flush();
            println!("wl-demo: seat capabilities {caps}");
        } else if id.keyboard != 0 && e.object == id.keyboard {
            match e.opcode {
                0 => {
                    let ok = keymap_ok(self.c.take_handle(), e.arg(1));
                    let how = if ok { "xkb ok" } else { "unreadable" };
                    println!("wl-demo: keymap format {} size {} {how}", e.arg(0), e.arg(1));
                }
                1 => println!("wl-demo: keyboard enter"),
                2 => println!("wl-demo: keyboard leave"),
                3 => {
                    let pressed = e.arg(3) != 0;
                    println!("wl-demo: key {} {}", e.arg(2), if pressed { "pressed" } else { "released" });
                    if e.arg(2) == KEY_Q && pressed {
                        self.closed = true;
                    }
                }
                4 => println!("wl-demo: modifiers depressed {} locked {}", e.arg(1), e.arg(3)),
                5 => println!("wl-demo: repeat rate {} delay {}", e.arg(0), e.arg(1)),
                _ => {}
            }
        } else if id.pointer != 0 && e.object == id.pointer {
            let fixed = |w: u32| w as i32 / 256;
            match e.opcode {
                0 => {
                    println!("wl-demo: pointer enter {},{}", fixed(e.arg(2)), fixed(e.arg(3)));
                    self.motion_reported = false;
                }
                1 => println!("wl-demo: pointer leave"),
                2 => {
                    if !self.motion_reported {
                        println!("wl-demo: pointer motion {},{}", fixed(e.arg(1)), fixed(e.arg(2)));
                    }
                    self.motion_reported = true;
                }
                3 => {
                    let pressed = e.arg(3) != 0;
                    println!("wl-demo: button {} {}", e.arg(2), if pressed { "pressed" } else { "released" });
                    if e.arg(2) == BTN_LEFT && pressed {
                        self.colour_b = !self.colour_b;
                        self.draw();
                        self.commit();
                    }
                }
                4 => println!("wl-demo: axis {} {}", e.arg(1), fixed(e.arg(2))),
                _ => {}
            }
        }
        Ok(())
    }

    fn roundtrip(&mut self) -> Result<(), String> {
        self.id.sync = self.c.new_id();
        self.synced = false;
        let sync = self.id.sync;
        self.c.request(DISPLAY, 0, &[Arg::U(sync)]); // wl_display.sync
        self.flush();
        while !self.synced {
            let e = self.c.next_event(None).ok_or("roundtrip")?;
            self.handle(&e)?;
        }
        Ok(())
    }

    fn run(&mut self) -> Result<i32, String> {
        self.id.registry = self.c.new_id();
        let registry = self.id.registry;
        self.c.request(DISPLAY, 1, &[Arg::U(registry)]); // get_registry
        self.roundtrip()?;
        let n = &self.names;
        if n.compositor == 0 || n.shm == 0 || n.wm_base == 0 {
            println!("wl-demo: the compositor lacks a global");
            return Ok(1);
        }
        let (nc, ns, nw, nd, nseat) = (n.compositor, n.shm, n.wm_base, n.decoration, n.seat);
        self.id.compositor = self.bind(nc, "wl_compositor", 4);
        self.id.shm = self.bind(ns, "wl_shm", 1);
        self.id.wm_base = self.bind(nw, "xdg_wm_base", 1);
        if nd != 0 {
            self.id.decoration_manager = self.bind(nd, "zxdg_decoration_manager_v1", 1);
        }
        if nseat != 0 {
            self.id.seat = self.bind(nseat, "wl_seat", 5);
        }
        self.make_buffer()?;
        let c = &mut self.c;
        self.id.surface = c.new_id();
        c.request(self.id.compositor, 0, &[Arg::U(self.id.surface)]); // create_surface
        self.id.xdg_surface = c.new_id();
        c.request(self.id.wm_base, 2, &[Arg::U(self.id.xdg_surface), Arg::U(self.id.surface)]); // get_xdg_surface
        self.id.toplevel = c.new_id();
        c.request(self.id.xdg_surface, 1, &[Arg::U(self.id.toplevel)]); // get_toplevel
        c.request(self.id.toplevel, 2, &[Arg::S("Poly Demo")]); // set_title
        c.request(self.id.toplevel, 3, &[Arg::S("poly.demo")]); // set_app_id
        if self.id.decoration_manager != 0 {
            let decoration = c.new_id();
            c.request(self.id.decoration_manager, 1, &[Arg::U(decoration), Arg::U(self.id.toplevel)]);
            c.request(decoration, 1, &[Arg::U(MODE_SERVER_SIDE)]); // set_mode
        }
        c.request(self.id.surface, 6, &[]); // commit without a buffer: configure comes
        self.flush();
        while !self.closed {
            let Some(e) = self.c.next_event(None) else {
                println!("wl-demo: the compositor went away");
                return Ok(1);
            };
            self.handle(&e)?;
        }
        println!("wl-demo: closed");
        Ok(0)
    }
}

/// The program.
pub fn main() -> i32 {
    let c = match Conn::connect() {
        Ok(c) => c,
        Err(e) => {
            println!("wl-demo: {e}");
            return 1;
        }
    };
    let mut d = Demo {
        c,
        id: Ids::default(),
        names: Names::default(),
        shared: None,
        width: WIDTH,
        height: HEIGHT,
        pending: (0, 0),
        configured: false,
        closed: false,
        synced: false,
        motion_reported: false,
        colour_b: false,
    };
    match d.run() {
        Ok(status) => status,
        Err(e) => {
            println!("wl-demo: {e} failed");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn draws_the_band_at_its_share() {
        let mut p = vec![0u32; (WIDTH * HEIGHT) as usize];
        draw(&mut p, WIDTH, HEIGHT, false);
        let at = |x: u32, y: u32| p[(y * WIDTH + x) as usize];
        assert_eq!(at(0, 0), DEMO_A);
        assert_eq!(at(359, 99), DEMO_A);
        assert_eq!(at(0, 100), BAND);
        assert_eq!(at(200, 139), BAND);
        assert_eq!(at(0, 140), DEMO_A);
        draw(&mut p, WIDTH, HEIGHT, true);
        assert_eq!(p[0], DEMO_B);
        // A maximized size: the band scales with it.
        let (w, h) = (1278, 750);
        let mut p = vec![0u32; (w * h) as usize];
        draw(&mut p, w, h, false);
        assert_eq!(p[(312 * w) as usize], BAND);
        assert_eq!(p[(311 * w) as usize], DEMO_A);
        assert_eq!(p[(w * h - 1) as usize], DEMO_A);
    }

    #[test]
    fn checks_the_keymap() {
        assert!(is_xkb_keymap(b"xkb_keymap {\n  xkb_keycodes {};\n};\n\0"));
        assert!(!is_xkb_keymap(b"xkb_keymap {\n  xkb_keycodes {};\n};\n"));
        assert!(!is_xkb_keymap(b"something else entirely\0"));
        assert!(!is_xkb_keymap(b"xkb_keymap {\0"));
    }
}
