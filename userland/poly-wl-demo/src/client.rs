//! `/bin/wl-client`: the compositor's protocol test as a native program
//! (the launcher's "Wayland Test Client"; the C `wl-client` of
//! `tests/linux/wayland/` is the same test as a Linux program, for boot
//! group `wayland`).
//!
//! It connects, binds the globals, makes an `xdg_toplevel` with a
//! server-side decoration, acks its configure, shares a 64x48 xrgb8888
//! buffer through a memory object (its handle passed with the request),
//! commits it and waits for the frame callback and the buffer's release.
//! Started by `poly-compositor --exec` (which sets `POLY_COMPOSITOR_DUMP`),
//! it then asks the compositor for a picture of the output (a `dump` line
//! on the named pipe `$POLY_COMPOSITOR_CONTROL`) and finds its own pixels
//! in it. A second connection checks that a request to an unknown object
//! is a protocol error.
//!
//! Output, one line per check: `ok <check>` or `not ok <check> (...)`,
//! then `summary <passed> passed, <failed> failed` and, if all passed,
//! `wl-client: ok`. Exit status 0 when every check passed.

use crate::demo::Shared;
use crate::wire::{Arg, Conn, DISPLAY, Event, bind_args};
use alloc::format;
use alloc::string::String;
use core::time::Duration;
use poly_rt::println;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;
/// The picture: the left half one colour, the right half another.
pub const LEFT: u32 = 0x2060c0;
pub const RIGHT: u32 = 0xe07020;
const FORMAT_ARGB8888: u32 = 0;
const FORMAT_XRGB8888: u32 = 1;
const ERROR_INVALID_OBJECT: u32 = 0;
const MODE_SERVER_SIDE: u32 = 2;
/// How long to wait for an event.
const EVENT_WAIT: Duration = Duration::from_secs(10);

/// A binary PPM (`P6 W H 255\n` and the pixels): its size and how many
/// pixels are [`LEFT`] and [`RIGHT`]; `None` if it is not one.
pub fn count_ppm(data: &[u8]) -> Option<(u32, u32, u64, u64)> {
    let mut fields = [0u32; 3];
    let mut at = 2;
    if !data.starts_with(b"P6") {
        return None;
    }
    for f in &mut fields {
        while data.get(at)?.is_ascii_whitespace() {
            at += 1;
        }
        let start = at;
        while data.get(at)?.is_ascii_digit() {
            at += 1;
        }
        *f = core::str::from_utf8(&data[start..at]).ok()?.parse().ok()?;
    }
    if data.get(at) != Some(&b'\n') || fields[2] != 255 {
        return None;
    }
    let (mut left, mut right) = (0, 0);
    for rgb in data[at + 1..].chunks_exact(3) {
        let p = u32::from(rgb[0]) << 16 | u32::from(rgb[1]) << 8 | u32::from(rgb[2]);
        left += u64::from(p == LEFT);
        right += u64::from(p == RIGHT);
    }
    Some((fields[0], fields[1], left, right))
}

/// Fill the buffer: the left half [`LEFT`], the right [`RIGHT`].
pub fn draw(pixels: &mut [u32]) {
    for (i, p) in pixels.iter_mut().enumerate() {
        *p = if (i as u32 % WIDTH) < WIDTH / 2 { LEFT } else { RIGHT };
    }
}

#[derive(Debug, Default)]
struct State {
    registry: u32,
    sync: u32,
    shm: u32,
    wm_base: u32,
    output: u32,
    surface: u32,
    xdg_surface: u32,
    toplevel: u32,
    decoration: u32,
    buffer: u32,
    frame: u32,
    names: [u32; 5],
    formats: u32,
    synced: bool,
    configured: bool,
    released: bool,
    frame_done: bool,
    frame_deleted: bool,
    entered: bool,
    protocol_error: bool,
    configure_serial: u32,
    decoration_mode: u32,
    preferred_scale: u32,
    mode: (i32, i32),
    output_scale: i32,
}

const GLOBALS: [&str; 5] = ["wl_compositor", "wl_shm", "xdg_wm_base", "zxdg_decoration_manager_v1", "wl_output"];

struct Test {
    c: Conn,
    s: State,
    passed: u32,
    failed: u32,
}

impl Test {
    fn check(&mut self, ok: bool, what: &str, got: &str) {
        if ok {
            self.passed += 1;
            println!("ok {what}");
        } else {
            self.failed += 1;
            println!("not ok {what} ({got})");
        }
    }

    fn handle(&mut self, e: &Event) {
        let s = &mut self.s;
        if e.object == DISPLAY && e.opcode == 0 {
            s.protocol_error = true;
            println!("wl-client: protocol error on object {}, code {}: {}", e.arg(0), e.arg(1), e.string(2));
        } else if e.object == DISPLAY && e.opcode == 1 {
            if e.arg(0) == s.frame {
                s.frame_deleted = true;
            }
        } else if e.object == s.registry && e.opcode == 0 {
            let iface = e.string(1);
            if let Some(i) = GLOBALS.iter().position(|g| *g == iface) {
                s.names[i] = e.arg(0);
            }
        } else if e.object == s.sync && e.opcode == 0 {
            s.synced = true;
        } else if e.object == s.shm && e.opcode == 0 {
            match e.arg(0) {
                FORMAT_ARGB8888 => s.formats |= 1,
                FORMAT_XRGB8888 => s.formats |= 2,
                _ => {}
            }
        } else if e.object == s.output && e.opcode == 1 {
            s.mode = (e.arg(1) as i32, e.arg(2) as i32);
        } else if e.object == s.output && e.opcode == 3 {
            s.output_scale = e.arg(0) as i32;
        } else if e.object == s.wm_base && e.opcode == 0 {
            let wm = s.wm_base;
            self.c.request(wm, 3, &[Arg::U(e.arg(0))]); // pong
            let _ = self.c.flush();
        } else if e.object == s.surface && e.opcode == 0 {
            s.entered = true;
        } else if e.object == s.surface && e.opcode == 2 {
            s.preferred_scale = e.arg(0);
        } else if e.object == s.xdg_surface && e.opcode == 0 {
            s.configure_serial = e.arg(0);
            s.configured = true;
        } else if e.object == s.decoration && e.opcode == 0 {
            s.decoration_mode = e.arg(0);
        } else if e.object == s.buffer && e.opcode == 0 {
            s.released = true;
        } else if e.object == s.frame && e.opcode == 0 {
            s.frame_done = true;
        }
    }

    /// Send what is queued, then handle events until `flag` says so.
    fn wait_for(&mut self, flag: fn(&State) -> bool) -> bool {
        if let Err(e) = self.c.flush() {
            println!("wl-client: {e}");
            return false;
        }
        while !flag(&self.s) && !self.s.protocol_error {
            let Some(e) = self.c.next_event(Some(EVENT_WAIT)) else { return false };
            self.handle(&e);
        }
        flag(&self.s)
    }

    fn roundtrip(&mut self) -> bool {
        self.s.sync = self.c.new_id();
        self.s.synced = false;
        let sync = self.s.sync;
        self.c.request(DISPLAY, 0, &[Arg::U(sync)]);
        self.wait_for(|s| s.synced)
    }

    fn bind(&mut self, global: usize, version: u32) -> u32 {
        let id = self.c.new_id();
        let (registry, name) = (self.s.registry, self.s.names[global]);
        self.c.request(registry, 0, &bind_args(name, GLOBALS[global], version, id));
        id
    }

    /// Ask the compositor that started us for a picture of the output and
    /// count our window's pixels in it: each half of the buffer at the
    /// output's scale.
    fn check_composed(&mut self) {
        let Some(path) = poly_rt::env::var("POLY_COMPOSITOR_DUMP") else {
            println!("wl-client: not started by poly-compositor --exec: no picture to check");
            return;
        };
        let _ = poly_rt::fs::remove_file(path);
        if let Some(ctl) = poly_rt::env::var("POLY_COMPOSITOR_CONTROL") {
            let pipe = poly_rt::fs::OpenOptions::new().write(true).nonblocking(true).open(ctl);
            if pipe.and_then(|p| p.write_all(b"dump\n")).is_err() {
                println!("wl-client: the control pipe took no request");
            }
        }
        let mut waited = 0;
        while !poly_rt::fs::exists(path) && waited < 10_000 {
            let _ = poly_rt::time::sleep(Duration::from_millis(20));
            waited += 20;
        }
        let Some((w, h, left, right)) = poly_rt::fs::read(path).ok().as_deref().and_then(count_ppm) else {
            let got = if poly_rt::fs::exists(path) { "not a PPM" } else { "no file" };
            self.check(false, "the compositor wrote a picture of the output", got);
            return;
        };
        let (mw, mh) = self.s.mode;
        self.check((w as i32, h as i32) == (mw, mh), "the picture is the output's size", &format!("{w}x{h}"));
        let scale = self.s.output_scale.max(1) as u64;
        let want = u64::from(WIDTH / 2) * u64::from(HEIGHT) * scale * scale;
        let got = format!("{left} and {right} pixels, want {want} each");
        self.check(left == want && right == want, "the window's pixels are on the output", &got);
    }

    /// A request to an object that does not exist is `wl_display.error`
    /// `invalid_object`, on a connection of its own.
    fn check_unknown_object(&mut self) {
        let mut bad = match Conn::connect() {
            Ok(c) => c,
            Err(e) => return self.check(false, "a second connection", &e),
        };
        bad.request(99, 0, &[]);
        let _ = bad.flush();
        let e = bad.next_event(Some(EVENT_WAIT));
        let why = e
            .as_ref()
            .map_or_else(|| String::from("object 0 opcode 0"), |e| format!("object {} opcode {}", e.object, e.opcode));
        let ok = e.is_some_and(|e| e.object == DISPLAY && e.opcode == 0 && e.arg(1) == ERROR_INVALID_OBJECT);
        self.check(ok, "a request to an unknown object is an invalid_object error", &why);
    }

    fn run(&mut self) -> Result<(), String> {
        self.s.registry = self.c.new_id();
        let registry = self.s.registry;
        self.c.request(DISPLAY, 1, &[Arg::U(registry)]); // get_registry
        let ok = self.roundtrip();
        self.check(ok, "the registry's globals arrive before the sync callback", "no callback");
        let all = self.s.names.iter().all(|&n| n != 0);
        let what = "wl_compositor, wl_shm, xdg_wm_base, zxdg_decoration_manager_v1 and wl_output are offered";
        self.check(all, what, "one is missing");
        if !all {
            return Ok(());
        }
        let compositor = self.bind(0, 4);
        self.s.shm = self.bind(1, 1);
        self.s.wm_base = self.bind(2, 1);
        let decoration_manager = self.bind(3, 1);
        self.s.output = self.bind(4, 2);
        self.roundtrip();
        let formats = self.s.formats;
        self.check(formats == 3, "wl_shm offers argb8888 and xrgb8888", &format!("formats {formats}"));
        let ((mw, mh), scale) = (self.s.mode, self.s.output_scale);
        println!("wl-client: output {mw}x{mh} scale {scale}");
        self.check(mw > 0 && mh > 0 && scale >= 1, "wl_output describes its mode and scale", "no mode");

        let c = &mut self.c;
        let s = &mut self.s;
        s.surface = c.new_id();
        c.request(compositor, 0, &[Arg::U(s.surface)]); // create_surface
        s.xdg_surface = c.new_id();
        c.request(s.wm_base, 2, &[Arg::U(s.xdg_surface), Arg::U(s.surface)]); // get_xdg_surface
        s.toplevel = c.new_id();
        c.request(s.xdg_surface, 1, &[Arg::U(s.toplevel)]); // get_toplevel
        c.request(s.toplevel, 2, &[Arg::S("wl-client")]); // set_title
        c.request(s.toplevel, 3, &[Arg::S("poly.wl-client")]); // set_app_id
        s.decoration = c.new_id();
        c.request(decoration_manager, 1, &[Arg::U(s.decoration), Arg::U(s.toplevel)]); // get_toplevel_decoration
        c.request(s.decoration, 1, &[Arg::U(MODE_SERVER_SIDE)]); // set_mode
        c.request(s.surface, 6, &[]); // commit, no buffer
        let ok = self.wait_for(|s| s.configured);
        self.check(ok, "the initial commit is answered with xdg_surface.configure", "none");
        let mode = self.s.decoration_mode;
        self.check(mode == MODE_SERVER_SIDE, "the decoration is server-side", &format!("mode {mode}"));
        let (xs, serial) = (self.s.xdg_surface, self.s.configure_serial);
        self.c.request(xs, 4, &[Arg::U(serial)]); // ack_configure

        // Shared memory: a memory object, mapped.
        let size = WIDTH * HEIGHT * 4;
        let mut shared = Shared::new(size as usize)?;
        draw(shared.pixels());
        self.check(true, "a shared memory buffer in a memory object", "");
        let pool = self.c.new_id();
        let shm = self.s.shm;
        self.c.request_with_handle(shm, 0, &[Arg::U(pool), Arg::U(size)], shared.file.handle()); // create_pool
        let c = &mut self.c;
        let s = &mut self.s;
        s.buffer = c.new_id();
        let args =
            [Arg::U(s.buffer), Arg::U(0), Arg::U(WIDTH), Arg::U(HEIGHT), Arg::U(WIDTH * 4), Arg::U(FORMAT_XRGB8888)];
        c.request(pool, 0, &args); // create_buffer
        c.request(pool, 1, &[]); // the pool may go; the buffer keeps it
        c.request(s.surface, 1, &[Arg::U(s.buffer), Arg::U(0), Arg::U(0)]); // attach
        c.request(s.surface, 9, &[Arg::U(0), Arg::U(0), Arg::U(WIDTH), Arg::U(HEIGHT)]); // damage_buffer
        s.frame = c.new_id();
        c.request(s.surface, 3, &[Arg::U(s.frame)]); // frame
        c.request(s.surface, 6, &[]); // commit
        let ok = self.wait_for(|s| s.frame_done);
        self.check(ok, "the frame callback is done after the commit", "no callback");
        let ok = self.s.released || self.wait_for(|s| s.released);
        self.check(ok, "the buffer is released", "no release");
        self.roundtrip();
        let s = &self.s;
        let (deleted, entered, error, preferred) = (s.frame_deleted, s.entered, s.protocol_error, s.preferred_scale);
        self.check(deleted, "the frame callback's id is deleted", "no delete_id");
        self.check(entered, "the surface entered the output", "no enter");
        self.check(!error, "no protocol error", "an error");
        self.check(preferred >= 1, "a preferred buffer scale", &format!("scale {preferred}"));
        println!("wl-client: window {WIDTH}x{HEIGHT} mapped, title \"wl-client\"");
        self.check_composed();
        self.check_unknown_object();
        drop(shared);
        Ok(())
    }
}

/// The program.
pub fn main() -> i32 {
    let mut t = match Conn::connect() {
        Ok(c) => Test { c, s: State { preferred_scale: 1, output_scale: 1, ..State::default() }, passed: 0, failed: 0 },
        Err(e) => {
            println!("not ok connect to the compositor ({e})");
            println!("summary 0 passed, 1 failed");
            return 1;
        }
    };
    t.check(true, "connected to the compositor", "");
    if let Err(e) = t.run() {
        t.check(false, "the test ran to the end", &e);
    }
    println!("summary {} passed, {} failed", t.passed, t.failed);
    if t.failed == 0 {
        println!("wl-client: ok");
    }
    i32::from(t.failed != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn counts_the_halves_in_a_ppm() {
        let mut ppm = b"P6\n4 2\n255\n".to_vec();
        for p in [LEFT, LEFT, RIGHT, 0, RIGHT, LEFT, 0xffffff, RIGHT] {
            ppm.extend_from_slice(&p.to_be_bytes()[1..]);
        }
        assert_eq!(count_ppm(&ppm), Some((4, 2, 3, 3)));
        assert_eq!(count_ppm(b"P5\n4 2\n255\n"), None);
        assert_eq!(count_ppm(b"P6 4 2 65535\n"), None);
        assert_eq!(count_ppm(b"P6 4"), None);
    }

    #[test]
    fn draws_two_halves() {
        let mut p: Vec<u32> = alloc::vec![0; (WIDTH * HEIGHT) as usize];
        draw(&mut p);
        assert_eq!(p.iter().filter(|&&v| v == LEFT).count(), (WIDTH / 2 * HEIGHT) as usize);
        assert_eq!((p[31], p[32], p[64]), (LEFT, RIGHT, LEFT));
    }
}
