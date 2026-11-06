//! `poly-compositor`: Poly's Wayland compositor (ADR 0106, 0107), a native
//! Poly program on `poly-rt` (ADR 0125): this crate is its code, built for
//! the native target by `userland/native/compositor` and tested on the
//! host. Linux apps reach it as they reach any Wayland compositor: on the
//! socket path, through the Linux personality's sockets.
//!
//! It listens on `$XDG_RUNTIME_DIR/<socket>` (`wayland-0` by default) as
//! the Wayland book's "Wayland display" describes, hands every client's
//! bytes and file descriptors to `poly-wayland`'s server, and composes the
//! scene at up to 60 frames a second (each frame answers the frame
//! callbacks committed before it, and `wp_presentation` feedback with the
//! time the frame was shown).
//!
//! Two ways to run:
//!
//! - **Off-screen** (the default): into a framebuffer in memory, which
//!   goes nowhere but a PPM file (`--dump`, else
//!   `$XDG_RUNTIME_DIR/poly-compositor.ppm`): written when a line `dump`
//!   comes in on the control pipe (`$POLY_COMPOSITOR_CONTROL`, a named pipe
//!   in the runtime directory; `SIGUSR1` before ADR 0125), with the windows
//!   and the `--probe` pixels printed, and with `--dump` when the
//!   compositor ends.
//! - **`--desktop`**: on the screen, with the keyboards and pointers of
//!   `/dev/input/event*` routed to the clients (`wl_pointer`, `wl_keyboard`
//!   with an XKB keymap of the settings' or `--keymap`'s layout), a
//!   software cursor, and Poly's shell (`poly-shell`, `desk.rs`): the ridge
//!   along the top, the launcher in the middle (shown on an empty desktop;
//!   the Windows key or Ctrl+Space toggles it; Enter starts the app chosen
//!   with `WAYLAND_DISPLAY` set), window management (the frame's buttons,
//!   snapping, Alt+Tab, Super+arrows, Alt+F4), notifications through a
//!   session bus at `$XDG_RUNTIME_DIR/bus`, the settings window, kept in
//!   `--settings` (default `$XDG_CONFIG_HOME/poly/shell.conf`), the
//!   wallpaper, new windows centred. The control pipe still asks for the
//!   picture.
//! - **`--kms`**: on the screen without the shell (what the boot tests of
//!   the protocols use).
//!
//! On the screen means DRM/KMS when `/dev/dri/card0` (`--drm`) has a
//! connected output (ADR 0116): two dumb buffers flipped at vblank, frames
//! paced by the flips' completion (real vsync), dma-buf client buffers
//! taken (`zwp_linux_dmabuf_v1`), the scale from the screen's physical
//! size unless `--scale` or `POLY_DISPLAY_SCALE` (the kernel's
//! `poly.display.scale=`) says otherwise. Without DRM (or with
//! `--no-kms`) `--desktop` falls back to `/dev/fb0` (`--fb`, ADR 0107).
//!
//! - **`--xwayland`** (or `--xwayland-lazy`): an X display for X11 apps
//!   through the unmodified Xwayland, started with the compositor (or
//!   with the first X client), with Poly's X window manager; launched apps
//!   get `DISPLAY` (ADR 0118, `xwayland.rs`).
//! - Windows apps' top-level windows come over the desktop link from the
//!   NT personality's win32k (ADR 0128, `win32.rs`: its records as framed
//!   scene windows; the `/dev/win32k` device and its place in the poll
//!   loop come with the first Phase 7 slice).
//!
//! ```text
//! poly-compositor [--socket NAME] [--size WxH] [--scale N|auto] [--dump FILE] [--stats]
//!                 [--probe X,Y]... [--desktop [--fb PATH] [--keymap NAME] [--settings FILE]]
//!                 [--kms] [--drm PATH] [--no-kms] [--xwayland | --xwayland-lazy]
//!                 [--exec PROGRAM [ARGS...]]
//! ```
//!
//! `--exec` starts a client once the socket is up (with `WAYLAND_DISPLAY`,
//! `XDG_RUNTIME_DIR` and `POLY_COMPOSITOR_DUMP`, the dump's path, set) and
//! ends the compositor when it ends, with its exit status: how the boot
//! test runs it as Poly's first process, the client asking for a dump
//! on the control pipe while its window is up. Without `XDG_RUNTIME_DIR`
//! it uses `/run/user/<uid>`, made if missing.
//!
//! I/O is blocking and driven by `poll`: one receive per readable client,
//! a send of what the server queued. A client that stops reading could
//! stall it; non-blocking sends come with real workloads. Children (the
//! client to run, launched apps, Xwayland) are watched through their
//! process handles, polled with the rest (`os.rs`).

#![no_std]

#[macro_use]
extern crate alloc;
#[macro_use]
extern crate poly_rt;
#[cfg(test)]
extern crate std;

mod desk;
mod desktop;
mod evdev;
mod fbdev;
pub mod io;
mod kms;
mod mixer;
pub mod os;
mod session;
mod status;
pub mod win32;
mod xwayland;

/// What every module of the crate uses from `alloc`, which `std`'s
/// prelude would have given.
mod prelude {
    pub use alloc::boxed::Box;
    pub use alloc::string::{String, ToString};
    pub use alloc::vec::Vec;
}

use crate::prelude::*;
use alloc::collections::BTreeMap;
use poly_rt::fs;
use poly_rt::net::{UnixListener, UnixStream};
use poly_wayland::compose::{Framebuffer, MemoryFramebuffer, Rect};
use poly_wayland::seat::{BTN_LEFT, InputEvent, InputSource};
use poly_wayland::server::{ClientId, Config, FrameTime, OutputConfig, Server, present_flags};
use poly_wayland::wire::Fd;

/// The shortest time between two frames: 60 Hz.
const FRAME_NS: u64 = 1_000_000_000 / 60;

#[derive(Debug)]
struct Options {
    socket: String,
    width: u32,
    height: u32,
    /// `None`: automatic (from the screen, or `POLY_DISPLAY_SCALE`).
    scale: Option<u32>,
    dump: Option<String>,
    probes: Vec<(u32, u32)>,
    desktop: bool,
    fb: String,
    /// `--keymap` as given (it wins over the settings).
    keymap: Option<String>,
    settings: Option<String>,
    /// On the screen without the shell.
    kms: bool,
    drm: String,
    no_kms: bool,
    /// Print compose/present/damage summaries every few seconds.
    stats: bool,
    xwayland: Option<xwayland::Start>,
    exec: Vec<String>,
}

const USAGE: &str = "usage: poly-compositor [--socket NAME] [--size WxH] [--scale N|auto] [--dump FILE] [--stats] [--probe X,Y]... [--desktop [--fb PATH] [--keymap NAME] [--settings FILE]] [--kms] [--drm PATH] [--no-kms] [--xwayland | --xwayland-lazy] [--exec PROGRAM [ARGS...]]";

/// A scale given as `N` (1 to 8) or `auto`.
fn parse_scale(v: &str) -> Result<Option<u32>, String> {
    if v == "auto" {
        return Ok(None);
    }
    v.parse().ok().filter(|s| (1..=8).contains(s)).map(Some).ok_or(format!("bad scale {v}"))
}

fn pair(s: &str, sep: char) -> Option<(u32, u32)> {
    let (a, b) = s.split_once(sep)?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        socket: "wayland-0".into(),
        width: 1024,
        height: 768,
        scale: None,
        dump: None,
        probes: Vec::new(),
        desktop: false,
        fb: "/dev/fb0".into(),
        keymap: None,
        settings: None,
        kms: false,
        drm: "/dev/dri/card0".into(),
        no_kms: false,
        stats: false,
        xwayland: None,
        exec: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{a} needs a value\n{USAGE}"));
        match a.as_str() {
            "--socket" => o.socket = value()?,
            "--size" => {
                let v = value()?;
                (o.width, o.height) = pair(&v, 'x').filter(|(w, h)| *w > 0 && *h > 0).ok_or(format!("bad size {v}"))?;
            }
            "--scale" => o.scale = parse_scale(&value()?)?,
            "--kms" => o.kms = true,
            "--drm" => o.drm = value()?,
            "--no-kms" => o.no_kms = true,
            "--stats" => o.stats = true,
            "--dump" => o.dump = Some(value()?),
            "--probe" => {
                let v = value()?;
                o.probes.push(pair(&v, ',').ok_or(format!("bad probe {v}"))?);
            }
            "--desktop" => o.desktop = true,
            "--fb" => o.fb = value()?,
            "--keymap" => {
                let v = value()?;
                poly_input::layout::by_name(&v).ok_or(format!("no keyboard layout {v} (us, de-ch, de)"))?;
                o.keymap = Some(v);
            }
            "--settings" => o.settings = Some(value()?),
            "--xwayland" => o.xwayland = Some(xwayland::Start::Now),
            "--xwayland-lazy" => o.xwayland = Some(xwayland::Start::Lazy),
            "--exec" => {
                o.exec = it.by_ref().cloned().collect();
                if o.exec.is_empty() {
                    return Err(format!("--exec needs a program\n{USAGE}"));
                }
            }
            "--help" | "-h" => return Err(USAGE.into()),
            _ => return Err(format!("unknown argument {a}\n{USAGE}")),
        }
    }
    Ok(o)
}

/// `$XDG_RUNTIME_DIR`, or `/run/user/<uid>` made with mode 0700.
fn runtime_dir() -> Result<String, String> {
    if let Some(dir) = poly_rt::env::var("XDG_RUNTIME_DIR") {
        return Ok(dir.into());
    }
    let dir = format!("/run/user/{}", os::getuid());
    fs::create_dir_all(&dir).map_err(|e| format!("{dir}: {e}"))?;
    let _ = fs::set_mode(&dir, 0o700);
    Ok(dir)
}

/// A connected client: its socket and what is still to be sent.
#[derive(Debug)]
struct Connection {
    stream: UnixStream,
    out: Vec<u8>,
    out_fds: Vec<Fd>,
}

/// Send what is queued; false if the connection is broken.
fn flush(conn: &mut Connection) -> bool {
    while !conn.out.is_empty() {
        match os::send(conn.stream.handle() as os::RawFd, &conn.out, &conn.out_fds) {
            Ok(n) => {
                conn.out.drain(..n);
                let sent = conn.out_fds.len().min(os::MAX_FDS);
                for fd in conn.out_fds.drain(..sent) {
                    os::close(fd.0);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
    true
}

/// Where frames go: memory or the screen (KMS, or fbdev).
#[derive(Debug)]
enum Screen {
    Memory(MemoryFramebuffer),
    Fb(fbdev::FbFramebuffer),
    Kms(Box<kms::Kms<kms::CardFile>>),
}

impl Screen {
    fn fb(&mut self) -> &mut dyn Framebuffer {
        match self {
            Screen::Memory(m) => m,
            Screen::Fb(f) => f,
            Screen::Kms(k) => &mut **k,
        }
    }

    /// Run `f` on what is shown (for KMS the front buffer; the others are
    /// drawn in place).
    fn shown<R>(&mut self, f: impl FnOnce(&mut dyn Framebuffer) -> R) -> R {
        match self {
            Screen::Kms(k) => f(&mut k.front_view()),
            other => f(other.fb()),
        }
    }

    fn kms(&mut self) -> Option<&mut kms::Kms<kms::CardFile>> {
        match self {
            Screen::Kms(k) => Some(k),
            _ => None,
        }
    }
}

/// How many flips completed and at what rate (the boot test reads it).
fn log_flips(k: &kms::Kms<kms::CardFile>) {
    let span = k.first_flip_ns.map_or(0, |first| k.last_flip_ns - first);
    let rate = (k.flips.saturating_sub(1) * 1_000_000_000_000).checked_div(span).unwrap_or(0);
    println!(
        "poly-compositor: {} flips in {} ms ({}.{:03} per second)",
        k.flips,
        span / 1_000_000,
        rate / 1000,
        rate % 1000
    );
}

/// A framebuffer that measures the part of a frame spent presenting it.
/// Only used when `--stats` is set.
struct TimedFramebuffer<'a> {
    inner: &'a mut dyn Framebuffer,
    present_ns: &'a mut u64,
}

impl Framebuffer for TimedFramebuffer<'_> {
    fn width(&self) -> u32 {
        self.inner.width()
    }

    fn height(&self) -> u32 {
        self.inner.height()
    }

    fn row_mut(&mut self, y: u32) -> &mut [u32] {
        self.inner.row_mut(y)
    }

    fn present(&mut self, damage: &[Rect]) {
        let start = os::monotonic_ns();
        self.inner.present(damage);
        *self.present_ns = self.present_ns.saturating_add(os::monotonic_ns().saturating_sub(start));
    }
}

/// Low-overhead frame timings, reported in three-second windows.
#[derive(Default)]
struct FrameStats {
    started_ns: u64,
    frames: u64,
    compose_ns: u64,
    compose_max_ns: u64,
    present_ns: u64,
    present_max_ns: u64,
    damaged_pixels: u64,
    copied_bytes: u64,
    copied_max_bytes: u64,
}

impl FrameStats {
    fn frame(
        &mut self,
        now_ns: u64,
        compose_ns: u64,
        present_ns: u64,
        damaged_pixels: u64,
        copied_bytes: u64,
        output_pixels: u64,
    ) {
        if self.started_ns == 0 {
            self.started_ns = now_ns;
        }
        self.frames += 1;
        self.compose_ns = self.compose_ns.saturating_add(compose_ns);
        self.compose_max_ns = self.compose_max_ns.max(compose_ns);
        self.present_ns = self.present_ns.saturating_add(present_ns);
        self.present_max_ns = self.present_max_ns.max(present_ns);
        self.damaged_pixels = self.damaged_pixels.saturating_add(damaged_pixels);
        self.copied_bytes = self.copied_bytes.saturating_add(copied_bytes);
        self.copied_max_bytes = self.copied_max_bytes.max(copied_bytes);

        let elapsed_ns = now_ns.saturating_sub(self.started_ns);
        if elapsed_ns < 3_000_000_000 {
            return;
        }
        let frames = self.frames.max(1);
        let fps_milli = frames.saturating_mul(1_000_000_000_000) / elapsed_ns.max(1);
        let compose_avg_us = self.compose_ns / frames / 1_000;
        let present_avg_us = self.present_ns / frames / 1_000;
        let damage_avg = self.damaged_pixels / frames;
        let damage_avg_bytes = damage_avg.saturating_mul(core::mem::size_of::<u32>() as u64);
        let copied_avg_bytes = self.copied_bytes / frames;
        let damage_pct_milli =
            self.damaged_pixels.saturating_mul(100_000) / frames.saturating_mul(output_pixels.max(1));
        println!(
            "poly-compositor: stats frames={frames} window_ms={} fps={}.{:03} compose_avg_ms={}.{:03} compose_max_ms={}.{:03} present_avg_ms={}.{:03} present_max_ms={}.{:03} copy_avg_bytes={copied_avg_bytes} copy_max_bytes={} damage_avg={damage_avg}px damage_avg_bytes={damage_avg_bytes} ({}.{:03}%)",
            elapsed_ns / 1_000_000,
            fps_milli / 1_000,
            fps_milli % 1_000,
            compose_avg_us / 1_000,
            compose_avg_us % 1_000,
            self.compose_max_ns / 1_000_000,
            (self.compose_max_ns / 1_000) % 1_000,
            present_avg_us / 1_000,
            present_avg_us % 1_000,
            self.present_max_ns / 1_000_000,
            (self.present_max_ns / 1_000) % 1_000,
            self.copied_max_bytes,
            damage_pct_milli / 1_000,
            damage_pct_milli % 1_000,
        );
        *self = FrameStats { started_ns: now_ns, ..FrameStats::default() };
    }
}

/// The union area of the non-empty physical damage rectangles.
fn damaged_area(rects: &[Rect]) -> u64 {
    const MAX_RECTS: usize = 32;
    if rects.len() > MAX_RECTS {
        return rects
            .iter()
            .fold(0u64, |area, r| area.saturating_add((r.width.max(0) as u64) * (r.height.max(0) as u64)));
    }
    let mut ys = [0i32; MAX_RECTS * 2];
    let mut y_count = 0;
    for r in rects.iter().filter(|r| !r.is_empty()) {
        ys[y_count] = r.y;
        ys[y_count + 1] = r.bottom();
        y_count += 2;
    }
    ys[..y_count].sort_unstable();
    let mut unique_y_count = 0;
    for i in 0..y_count {
        if unique_y_count == 0 || ys[i] != ys[unique_y_count - 1] {
            ys[unique_y_count] = ys[i];
            unique_y_count += 1;
        }
    }

    let mut intervals = [(0i32, 0i32); MAX_RECTS];
    let mut area = 0u64;
    for band in ys[..unique_y_count].windows(2) {
        let (top, bottom) = (band[0], band[1]);
        if bottom <= top {
            continue;
        }
        let mut interval_count = 0;
        for r in rects.iter().filter(|r| !r.is_empty() && r.y < bottom && r.bottom() > top) {
            intervals[interval_count] = (r.x, r.right());
            interval_count += 1;
        }
        intervals[..interval_count].sort_unstable_by_key(|range| range.0);
        let mut merged: Option<(i32, i32)> = None;
        let mut row_width = 0u64;
        for &(left, right) in &intervals[..interval_count] {
            match merged {
                Some((lo, hi)) if left <= hi => merged = Some((lo, hi.max(right))),
                Some((lo, hi)) => {
                    row_width += (hi - lo).max(0) as u64;
                    merged = Some((left, right));
                }
                None => merged = Some((left, right)),
            }
        }
        if let Some((lo, hi)) = merged {
            row_width += (hi - lo).max(0) as u64;
        }
        area = area.saturating_add(row_width.saturating_mul((bottom - top) as u64));
    }
    area
}

/// Take the screen through KMS, if the card has an output.
fn open_kms(path: &str) -> Result<kms::Kms<kms::CardFile>, String> {
    let card = kms::CardFile::open(path).map_err(|e| format!("{path}: {e}"))?;
    kms::Kms::open(card).map_err(|e| format!("{path}: {e}"))
}

/// The scale: asked for, else `POLY_DISPLAY_SCALE`, else from the screen
/// (KMS) or 1.
fn choose_scale(asked: Option<u32>, screen: &Screen) -> u32 {
    if let Some(s) = asked {
        return s;
    }
    if let Some(s) = poly_rt::env::var("POLY_DISPLAY_SCALE").and_then(|v| parse_scale(v).ok().flatten()) {
        return s;
    }
    match screen {
        Screen::Kms(k) => {
            let o = k.output();
            kms::auto_scale(o.size().0, o.mm.0)
        }
        _ => 1,
    }
}

/// The picture as a binary PPM (netpbm's `P6`).
fn ppm(fb: &mut dyn Framebuffer) -> Vec<u8> {
    let (w, h) = (fb.width(), fb.height());
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for y in 0..h {
        for p in &fb.row_mut(y)[..w as usize] {
            out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
        }
    }
    out
}

/// Write the picture as a PPM: to a temporary file renamed into place, so
/// a reader never sees half of it.
fn write_dump(fb: &mut dyn Framebuffer, path: &str) {
    let tmp = format!("{path}.tmp");
    let (w, h) = (fb.width(), fb.height());
    match fs::write(&tmp, &ppm(fb)).and_then(|()| fs::rename(&tmp, path)) {
        Ok(()) => println!("poly-compositor: wrote {path} ({w}x{h})"),
        Err(e) => eprintln!("poly-compositor: {path}: {e}"),
    }
}

/// Describe the windows and the probed pixels, write the picture: what a
/// `dump` request asks for (after the caller drew what changed).
fn report(server: &Server, fb: &mut dyn Framebuffer, probes: &[(u32, u32)], path: &str) {
    for w in server.scene().windows() {
        let (x, y) = w.position();
        let (cw, ch) = w.content().logical_size();
        println!("poly-compositor: window {:?} at {x},{y} size {cw}x{ch}", w.title());
    }
    for &(x, y) in probes {
        if x < fb.width() && y < fb.height() {
            let p = fb.row_mut(y)[x as usize] & 0x00ff_ffff;
            println!("poly-compositor: pixel {x},{y} = #{p:06x}");
        } else {
            println!("poly-compositor: pixel {x},{y} is outside {}x{}", fb.width(), fb.height());
        }
    }
    write_dump(fb, path);
}

/// The compositor's `main`: its arguments from the command line; returns
/// the exit status (the `--exec` client's, or 1 on an error).
pub fn main() -> i32 {
    let args: Vec<String> = poly_rt::env::args_str().skip(1).map(String::from).collect();
    match run(&args) {
        Ok(code) => i32::from(code),
        Err(e) => {
            eprintln!("poly-compositor: {e}");
            1
        }
    }
}

/// The environment a client gets: ours, with the display's variables, the
/// session bus's address (ADR 0117), `DISPLAY=...` for the X display
/// (ADR 0118), if there are, and the control pipe.
fn client_env(socket: &str, dir: &str, dump: &str, bus: Option<&str>, display: Option<String>) -> Vec<String> {
    let ours = [
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "POLY_COMPOSITOR_DUMP",
        "POLY_COMPOSITOR_CONTROL",
        "DBUS_SESSION_BUS_ADDRESS",
    ];
    let mut env: Vec<String> = poly_rt::env::vars_str()
        .filter(|(k, _)| !ours.contains(k) && (*k != "DISPLAY" || display.is_none()))
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    env.extend(display);
    env.push(format!("WAYLAND_DISPLAY={socket}"));
    env.push(format!("XDG_RUNTIME_DIR={dir}"));
    // Where a `dump` request puts the picture, and where to ask, for a
    // client that checks it.
    env.push(format!("POLY_COMPOSITOR_DUMP={dump}"));
    env.push(format!("POLY_COMPOSITOR_CONTROL={}", control_path(dir)));
    if let Some(addr) = bus {
        env.push(format!("DBUS_SESSION_BUS_ADDRESS={addr}"));
    }
    env
}

/// The control pipe in the runtime directory `dir`.
fn control_path(dir: &str) -> String {
    fs::join(dir, "poly-compositor.ctl")
}

fn run(args: &[String]) -> Result<u8, String> {
    let o = parse_args(args)?;
    let dir = runtime_dir()?;
    let path = fs::join(&dir, &o.socket);
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| format!("{path}: {e}"))?;
    let dump_path = o.dump.clone().unwrap_or_else(|| fs::join(&dir, "poly-compositor.ppm"));
    // Requests for a picture (what SIGUSR1 was on Linux).
    let control = os::Control::open(&control_path(&dir)).map_err(|e| format!("the control pipe: {e}"))?;

    // The screen: KMS if the card has an output, else fbdev for the
    // desktop (ADR 0116).
    let on_screen = o.desktop || o.kms;
    let mut screen = if on_screen && !o.no_kms {
        match open_kms(&o.drm) {
            Ok(k) => Screen::Kms(Box::new(k)),
            Err(e) if o.desktop => {
                println!("poly-compositor: no KMS ({e}): the desktop goes through {}", o.fb);
                Screen::Fb(fbdev::FbFramebuffer::open(&o.fb).map_err(|e| format!("{}: {e}", o.fb))?)
            }
            Err(e) => return Err(e),
        }
    } else if o.desktop {
        Screen::Fb(fbdev::FbFramebuffer::open(&o.fb).map_err(|e| format!("{}: {e}", o.fb))?)
    } else {
        Screen::Memory(MemoryFramebuffer::new(o.width, o.height))
    };
    let (width, height) = (screen.fb().width(), screen.fb().height());
    let scale = choose_scale(o.scale, &screen);
    let mut host = os::UnixHost::default();
    let keymap = host.keymap.clone();
    let mut output = OutputConfig { width, height, scale, ..OutputConfig::default() };
    if let Some(k) = screen.kms() {
        let out = k.output().clone();
        output.refresh_mhz = out.refresh_mhz();
        output.physical_mm = (out.mm.0 as i32, out.mm.1 as i32);
        // Clients may share dma-bufs made on the render node.
        host.dmabuf_device = os::device_number("/dev/dri/renderD128").or_else(|| os::device_number(&o.drm));
        let r = output.refresh_mhz;
        println!(
            "poly-compositor: KMS on {}: connector {}, CRTC {}, {width}x{height} at {}.{:03} Hz, {}x{} mm, scale {scale}{}",
            o.drm,
            out.connector,
            out.crtc,
            r / 1000,
            r % 1000,
            out.mm.0,
            out.mm.1,
            if host.dmabuf_device.is_some() { ", dma-bufs taken" } else { "" }
        );
    }
    let mut server = Server::new(Config { output, ..Config::default() }, Box::new(host));
    println!("poly-compositor: listening on {path} ({width}x{height}, scale {scale})");

    let mut desk = if o.desktop {
        let display = if matches!(screen, Screen::Kms(_)) { format!("{} (KMS)", o.drm) } else { o.fb.clone() };
        let opts = desk::Options {
            dir: &dir,
            fb_name: &display,
            keymap: o.keymap.as_deref(),
            scale: o.scale,
            settings: o.settings.clone(),
        };
        Some(desk::Desktop::new(&opts, &mut server, keymap, desktop::shell_fonts()))
    } else {
        None
    };
    let bus_address = desk.as_ref().and_then(|d| d.bus.as_ref()).map(session::SessionBus::address);

    let mut xw = match o.xwayland {
        Some(start) => Some(xwayland::Xwayland::new(start)?),
        None => None,
    };
    // Windows apps' top-level windows through win32k's desktop link (ADR 0131).
    let mut winlink = if o.desktop { win32::Link::open(server.scene()) } else { None };
    let mut env = client_env(&o.socket, &dir, &dump_path, bus_address.as_deref(), xw.as_ref().map(|x| x.display_env()));
    // Poly's own apps follow the desktop's theme and keyboard layout from
    // the settings file (ADR 0134).
    if let Some(path) = desk.as_ref().and_then(desk::Desktop::settings_file) {
        env.retain(|v| !v.starts_with("POLY_SHELL_SETTINGS="));
        env.push(format!("POLY_SHELL_SETTINGS={path}"));
    }
    // Children that end wake the loop (their process handles are polled),
    // which reaps them at once.
    let child = if o.exec.is_empty() {
        None
    } else {
        let pid = os::spawn(&o.exec, &env).map_err(|e| format!("{}: {e}", o.exec[0]))?;
        println!("poly-compositor: started {} (pid {pid})", o.exec[0]);
        Some(pid)
    };

    let mut conns: BTreeMap<ClientId, Connection> = BTreeMap::new();
    if let Some(x) = &mut xw
        && x.wants_start()
    {
        let (id, stream) = x.start(&mut server, &env)?;
        conns.insert(id, Connection { stream, out: Vec::new(), out_fds: Vec::new() });
    }
    let mut next_frame = 0u64;
    let mut stats = FrameStats::default();
    let mut buf = vec![0u8; 4096];
    loop {
        // What to wait for, and how long.
        let mut fds = vec![os::PollFd::readable(listener.handle() as os::RawFd)];
        let ids: Vec<ClientId> = conns.keys().copied().collect();
        fds.extend(ids.iter().map(|id| os::PollFd::readable(conns[id].stream.handle() as os::RawFd)));
        let input_at = fds.len();
        if let Some(d) = &desk {
            fds.extend(d.input.devices.iter().map(|dev| os::PollFd::readable(dev.fd)));
        }
        // The session bus and the mixer (ADR 0117).
        let desk_at = fds.len();
        if let Some(d) = &desk {
            fds.extend(d.poll_fds());
        }
        let desk_end = fds.len();
        // A child ended (launched apps, Xwayland, the client to run), a
        // request for a picture.
        fds.extend(os::child_poll_fds());
        let control_at = fds.len();
        fds.push(os::PollFd::readable(control.fd()));
        // The card's flip events.
        let card_fd = screen.kms().and_then(|k| k.fd());
        if let Some(fd) = card_fd {
            fds.push(os::PollFd::readable(fd));
        }
        let flip_pending = screen.kms().is_some_and(|k| k.is_pending());
        let x_at = fds.len();
        if let Some(x) = &xw {
            fds.extend(x.poll_fds());
        }
        let win_at = fds.len();
        if let Some(l) = &winlink {
            fds.push(os::PollFd::readable(l.fd()));
        }
        let now = os::monotonic_ns();
        let timeout = if server.wants_frame() && !flip_pending {
            (next_frame.saturating_sub(now) / 1_000_000) as i32
        } else if flip_pending && card_fd.is_none() {
            1
        } else if let Some(d) = &desk {
            // The clock and the status, toasts that expire, apps that ended.
            d.timeout_ms()
        } else {
            -1
        };
        os::poll(&mut fds, timeout).map_err(|e| format!("poll: {e}"))?;
        // What ended is reaped below (the desktop's apps in `sync`,
        // Xwayland in `reap`, the client to run at the end).
        let mut dump = false;
        if fds[control_at].revents != 0 {
            dump = control.take_lines().iter().any(|l| l.trim() == "dump");
        }

        if fds[0].revents & os::POLLIN != 0 {
            match listener.accept() {
                Ok(stream) => {
                    let id = server.add_client();
                    conns.insert(id, Connection { stream, out: Vec::new(), out_fds: Vec::new() });
                }
                Err(e) => eprintln!("poly-compositor: accept: {e}"),
            }
        }
        let mut gone = Vec::new();
        for (pfd, id) in fds[1..input_at].iter().zip(&ids) {
            if pfd.revents == 0 {
                continue;
            }
            let mut received = Vec::new();
            match os::recv(pfd.fd, &mut buf, &mut received) {
                Ok(0) => gone.push(*id),
                Ok(n) => server.receive(*id, &buf[..n], received),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => gone.push(*id),
            }
        }

        // A flip completed: the frame it showed is on the screen since its
        // vblank; its frame callbacks and feedback are answered with that
        // time and sequence (vsync).
        if let Some(k) = screen.kms()
            && k.is_pending()
        {
            match k.dispatch() {
                Ok(Some(flip)) => {
                    let flags = present_flags::VSYNC | present_flags::HW_CLOCK | present_flags::HW_COMPLETION;
                    let time = FrameTime { monotonic_ns: flip.monotonic_ns };
                    server.frame_completed(time, Some(u64::from(flip.sequence)), flags);
                }
                Ok(None) => {}
                Err(e) => return Err(format!("{}: reading events: {e}", o.drm)),
            }
        }

        // Input, then the shell.
        if let Some(d) = &mut desk {
            let size = server.scene().logical_size();
            let ms = (os::monotonic_ns() / 1_000_000) as u32;
            for (i, pfd) in fds[input_at..desk_at].iter().enumerate() {
                if pfd.revents != 0 {
                    d.input.read(i, size, ms);
                }
            }
            d.polled(&fds[desk_at..desk_end]);
            while let Some(ev) = d.input.next_event() {
                if let InputEvent::PointerButton { button: BTN_LEFT, pressed: true, .. } = &ev
                    && let Some(link) = &mut winlink
                {
                    let (x, y) = server.pointer();
                    link.close_at(server.scene(), x.floor(), y.floor());
                }
                if let Some(app) = d.route(ev, &mut server) {
                    if d.open_internal(&app, &mut server) {
                        continue;
                    }
                    match os::spawn(&app.argv, &env) {
                        Ok(pid) => {
                            println!("poly-compositor: launched {} (pid {pid})", app.name);
                            d.children.push((pid, app.name.clone()));
                        }
                        Err(e) => eprintln!("poly-compositor: {}: {e}", app.argv[0]),
                    }
                }
                let closes = core::mem::take(&mut d.close_requests);
                if let Some(link) = &mut winlink {
                    for window in closes {
                        link.close_window(window);
                    }
                    link.sync_geometry(server.scene());
                }
            }
            d.sync(&mut server);
        }

        // Xwayland: a client waiting for it, its window manager, what the
        // user did to X windows.
        if let Some(x) = &mut xw {
            if let Some((id, stream)) = x.handle(&fds[x_at..], &mut server, &env) {
                conns.insert(id, Connection { stream, out: Vec::new(), out_fds: Vec::new() });
            }
            x.pump(&mut server);
            if let Some(c) = x.reap(&mut server) {
                gone.push(c);
            }
        }

        // Windows apps' windows: what win32k reported.
        if let Some(l) = &mut winlink
            && fds.get(win_at).is_some_and(|p| p.revents != 0)
        {
            l.pump(server.scene_mut());
        }

        // A frame when one is due: with KMS whenever no flip is pending (the
        // flips pace it), else at most every 1/60 s.
        let now = os::monotonic_ns();
        let kms = matches!(screen, Screen::Kms(_));
        let due = if kms { !screen.kms().is_some_and(|k| k.is_pending()) } else { now >= next_frame };
        if server.wants_frame() && due {
            let frame_start = if o.stats { os::monotonic_ns() } else { 0 };
            let mut present_ns = 0;
            let copied_bytes = screen.kms().map_or(0, |k| k.begin_frame());
            let drawn = if o.stats {
                let fb = screen.fb();
                let mut timed = TimedFramebuffer { inner: fb, present_ns: &mut present_ns };
                match &desk {
                    Some(d) => server.render_with(&mut timed, Some(&d.shell.below()), Some(&d.shell.above())),
                    None => server.render(&mut timed),
                }
            } else {
                let fb = screen.fb();
                match &desk {
                    Some(d) => server.render_with(fb, Some(&d.shell.below()), Some(&d.shell.above())),
                    None => server.render(fb),
                }
            };
            let compose_ns =
                if o.stats { os::monotonic_ns().saturating_sub(frame_start).saturating_sub(present_ns) } else { 0 };
            let shown = os::monotonic_ns();
            if let Some(k) = screen.kms() {
                // Drawn frames flipped themselves (`present`); clients
                // waiting with nothing new to draw are paced by a flip too.
                if drawn.is_empty() {
                    let start = if o.stats { os::monotonic_ns() } else { 0 };
                    if let Err(e) = k.flip(&[]) {
                        eprintln!("poly-compositor: page flip: {e}");
                    }
                    if o.stats {
                        present_ns = present_ns.saturating_add(os::monotonic_ns().saturating_sub(start));
                    }
                }
                server.frame_submitted();
            } else {
                // Presented: the frame is on the screen once the device's
                // requests returned.
                server.frame_presented(FrameTime { monotonic_ns: shown });
            }
            if o.stats {
                stats.frame(
                    os::monotonic_ns(),
                    compose_ns,
                    present_ns,
                    damaged_area(&drawn),
                    copied_bytes,
                    u64::from(width) * u64::from(height),
                );
            }
            next_frame = now + FRAME_NS;
            if let Some(d) = &mut desk
                && !d.ready
            {
                d.ready = true;
                let ms = (shown - now) / 1_000_000;
                println!("poly-compositor: desktop ready (first frame drawn in {ms} ms; launcher shown)");
            }
        }

        // Send what the server queued; cut off failed clients.
        for (id, conn) in conns.iter_mut() {
            let (bytes, out_fds) = server.take_output(*id);
            conn.out.extend_from_slice(&bytes);
            conn.out_fds.extend(out_fds);
            if !flush(conn) {
                gone.push(*id);
            }
            if let Some(why) = server.failure(*id) {
                eprintln!("poly-compositor: client {} cut off: {why}", id.0);
                gone.push(*id);
            }
        }
        gone.sort_unstable();
        gone.dedup();
        for id in gone {
            server.remove_client(id);
            conns.remove(&id);
        }

        if dump {
            // Drawn in place, what changed is drawn first; with KMS the
            // picture is the buffer on the screen.
            if !kms {
                let fb = screen.fb();
                match &desk {
                    Some(d) => server.render_with(fb, Some(&d.shell.below()), Some(&d.shell.above())),
                    None => server.render(fb),
                };
            }
            screen.shown(|fb| report(&server, fb, &o.probes, &dump_path));
        }

        if let Some(status) = child.and_then(os::try_wait) {
            if o.dump.is_some() {
                if !kms {
                    server.render(screen.fb());
                }
                screen.shown(|fb| write_dump(fb, &dump_path));
            }
            if let Some(k) = screen.kms() {
                log_flips(k);
            }
            println!("poly-compositor: client exited with status {status}");
            let _ = fs::remove_file(&path);
            return Ok(status.clamp(0, 255) as u8);
        }
    }
}

/// A rectangle of the whole output, for tests.
#[cfg(test)]
fn whole(w: u32, h: u32) -> poly_wayland::compose::Rect {
    poly_wayland::compose::Rect::new(0, 0, w as i32, h as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_its_arguments() {
        let o = parse_args(&args("--size 640x480 --scale 2 --probe 1,2 --probe 3,4 --exec /bin/c -x")).unwrap();
        assert_eq!((o.width, o.height, o.scale), (640, 480, Some(2)));
        assert_eq!(o.probes, [(1, 2), (3, 4)]);
        let k = parse_args(&args("--kms --drm /dev/dri/card1 --scale auto")).unwrap();
        assert_eq!((k.kms, k.drm.as_str(), k.scale), (true, "/dev/dri/card1", None));
        assert!(parse_args(&args("--no-kms")).unwrap().no_kms);
        assert_eq!(o.exec, ["/bin/c", "-x"]);
        assert_eq!(o.socket, "wayland-0");
        assert!(!o.desktop);
        assert!(!o.stats);
        let measured = parse_args(&args("--stats")).unwrap();
        assert!(measured.stats);
        let o = parse_args(&args("--desktop --fb /dev/fb1 --keymap de-ch")).unwrap();
        assert!(o.desktop);
        assert_eq!((o.fb.as_str(), o.keymap.as_deref()), ("/dev/fb1", Some("de-ch")));
        let o = parse_args(&args("--desktop --settings /tmp/s.conf --scale 2")).unwrap();
        assert_eq!((o.settings, o.scale, o.keymap), (Some("/tmp/s.conf".into()), Some(2), None));
        assert!(parse_args(&args("--keymap xx")).is_err());
        assert!(parse_args(&args("--size 0x4")).is_err());
        assert!(parse_args(&args("--scale 9")).is_err());
        assert!(parse_args(&args("--exec")).is_err());
        assert!(parse_args(&args("--what")).is_err());
    }

    #[test]
    fn damaged_area_counts_overlap_once() {
        assert_eq!(damaged_area(&[Rect::new(0, 0, 4, 4), Rect::new(2, 2, 4, 4)]), 28);
        assert_eq!(damaged_area(&[Rect::new(0, 0, 0, 5), Rect::new(1, 1, 2, 3)]), 6);
    }

    /// What a client is given: the display's variables and the control
    /// pipe, and the compositor's own ones not passed on twice.
    #[test]
    fn a_client_environment() {
        let env = client_env("wayland-0", "/run/user/0", "/run/user/0/p.ppm", Some("unix:path=/b"), None);
        assert!(env.contains(&"WAYLAND_DISPLAY=wayland-0".to_string()));
        assert!(env.contains(&"POLY_COMPOSITOR_CONTROL=/run/user/0/poly-compositor.ctl".to_string()));
        assert!(env.contains(&"DBUS_SESSION_BUS_ADDRESS=unix:path=/b".to_string()));
        assert_eq!(env.iter().filter(|v| v.starts_with("WAYLAND_DISPLAY=")).count(), 1);
    }

    /// The desktop's layers through the scene into memory: the ridge, the
    /// launcher, the cursor; the picture as a PPM.
    #[test]
    fn the_desktop_composes() {
        let host = os::UnixHost::default();
        let output = OutputConfig { width: 800, height: 600, ..OutputConfig::default() };
        let mut server = Server::new(Config { output, ..Config::default() }, Box::new(host));
        let shell = poly_shell::Shell::new(800, 600, poly_shell::apps::builtins(), poly_shell::draw::Text::new(None));
        server.set_cursor_visible(true);
        let mut fb = MemoryFramebuffer::new(800, 600);
        let drawn = server.render_with(&mut fb, Some(&shell.below()), Some(&shell.above()));
        assert_eq!(drawn, [whole(800, 600)]);
        assert_eq!(fb.pixel(400, 5), Some(0x20_20_20), "the ridge");
        let p = ppm(&mut fb);
        assert!(p.starts_with(b"P6\n800 600\n255\n"));
        assert_eq!(p.len(), 15 + 800 * 600 * 3);
    }
}
