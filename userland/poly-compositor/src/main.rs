//! `poly-compositor`: Poly's Wayland compositor for the Linux personality
//! (ADR 0106, 0107).
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
//!   `$XDG_RUNTIME_DIR/poly-compositor.ppm`): written on `SIGUSR1`, with
//!   the windows and the `--probe` pixels printed, and with `--dump` when
//!   the compositor ends.
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
//!   wallpaper, new windows centred. `SIGUSR1` still writes the picture.
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
//! ```text
//! poly-compositor [--socket NAME] [--size WxH] [--scale N|auto] [--dump FILE]
//!                 [--probe X,Y]... [--desktop [--fb PATH] [--keymap NAME] [--settings FILE]]
//!                 [--kms] [--drm PATH] [--no-kms] [--exec PROGRAM [ARGS...]]
//! ```
//!
//! `--exec` starts a client once the socket is up (with `WAYLAND_DISPLAY`,
//! `XDG_RUNTIME_DIR` and `POLY_COMPOSITOR_DUMP`, the dump's path, set) and
//! ends the compositor when it ends, with its exit status: how the boot
//! test runs it as Poly's first process, the client asking for a dump
//! with `SIGUSR1` while its window is up. Without `XDG_RUNTIME_DIR` it
//! uses `/run/user/<uid>`, made if missing.
//!
//! I/O is blocking and driven by `poll`: one `recvmsg` per readable
//! client, `sendmsg` of what the server queued. A client that stops
//! reading could stall it; non-blocking sends come with real workloads.

mod desk;
mod desktop;
mod evdev;
mod fbdev;
mod kms;
mod mixer;
mod session;
mod status;
mod unix;

use poly_wayland::compose::{Framebuffer, MemoryFramebuffer};
use poly_wayland::seat::InputSource;
use poly_wayland::server::{ClientId, Config, FrameTime, OutputConfig, Server, present_flags};
use poly_wayland::wire::Fd;
use std::collections::BTreeMap;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

/// The shortest time between two frames: 60 Hz.
const FRAME_NS: u64 = 1_000_000_000 / 60;

/// Set by `SIGUSR1`: write the picture.
static DUMP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigusr1(_: libc::c_int) {
    DUMP.store(true, Ordering::Relaxed);
}

#[derive(Debug)]
struct Options {
    socket: String,
    width: u32,
    height: u32,
    /// `None`: automatic (from the screen, or `POLY_DISPLAY_SCALE`).
    scale: Option<u32>,
    dump: Option<PathBuf>,
    probes: Vec<(u32, u32)>,
    desktop: bool,
    fb: String,
    /// `--keymap` as given (it wins over the settings).
    keymap: Option<String>,
    settings: Option<PathBuf>,
    /// On the screen without the shell.
    kms: bool,
    drm: String,
    no_kms: bool,
    exec: Vec<String>,
}

const USAGE: &str = "usage: poly-compositor [--socket NAME] [--size WxH] [--scale N|auto] [--dump FILE] [--probe X,Y]... [--desktop [--fb PATH] [--keymap NAME] [--settings FILE]] [--kms] [--drm PATH] [--no-kms] [--exec PROGRAM [ARGS...]]";

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
            "--dump" => o.dump = Some(value()?.into()),
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
            "--settings" => o.settings = Some(value()?.into()),
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
fn runtime_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(dir.into());
    }
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let dir = PathBuf::from(format!("/run/user/{uid}"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
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
        match unix::send(conn.stream.as_raw_fd(), &conn.out, &conn.out_fds) {
            Ok(n) => {
                conn.out.drain(..n);
                let sent = conn.out_fds.len().min(unix::MAX_FDS);
                for fd in conn.out_fds.drain(..sent) {
                    unix::close(fd.0);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
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
    if let Some(s) = std::env::var("POLY_DISPLAY_SCALE").ok().and_then(|v| parse_scale(&v).ok().flatten()) {
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
fn write_dump(fb: &mut dyn Framebuffer, path: &Path) {
    let tmp = path.with_extension("ppm.tmp");
    let (w, h) = (fb.width(), fb.height());
    match std::fs::write(&tmp, ppm(fb)).and_then(|()| std::fs::rename(&tmp, path)) {
        Ok(()) => println!("poly-compositor: wrote {} ({w}x{h})", path.display()),
        Err(e) => eprintln!("poly-compositor: {}: {e}", path.display()),
    }
}

/// Describe the windows and the probed pixels, write the picture: what
/// `SIGUSR1` asks for (after the caller drew what changed).
fn report(server: &Server, fb: &mut dyn Framebuffer, probes: &[(u32, u32)], path: &Path) {
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("poly-compositor: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The environment a client gets: ours, with the display's variables and
/// the session bus's address (ADR 0117), if there is one.
fn client_env(socket: &str, dir: &Path, dump: &Path, bus: Option<&str>) -> Vec<String> {
    let ours = ["WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "POLY_COMPOSITOR_DUMP", "DBUS_SESSION_BUS_ADDRESS"];
    let mut env: Vec<String> =
        std::env::vars().filter(|(k, _)| !ours.contains(&k.as_str())).map(|(k, v)| format!("{k}={v}")).collect();
    env.push(format!("WAYLAND_DISPLAY={socket}"));
    env.push(format!("XDG_RUNTIME_DIR={}", dir.display()));
    // Where SIGUSR1 puts the picture, for a client that checks it.
    env.push(format!("POLY_COMPOSITOR_DUMP={}", dump.display()));
    if let Some(addr) = bus {
        env.push(format!("DBUS_SESSION_BUS_ADDRESS={addr}"));
    }
    env
}

/// The shell's font, if the font store has it (kept for the program's
/// life).
fn shell_font() -> Option<poly_font::Font<'static>> {
    let font = std::fs::read(desktop::FONT).ok().and_then(|data| {
        let data: &'static [u8] = Box::leak(data.into_boxed_slice());
        poly_font::Font::parse(data).ok()
    });
    if font.is_none() {
        println!("poly-compositor: no {} (fonts not fetched?): the shell uses the bitmap font", desktop::FONT);
    }
    font
}

fn run(args: &[String]) -> Result<u8, String> {
    let o = parse_args(args)?;
    let dir = runtime_dir()?;
    let path = dir.join(&o.socket);
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let dump_path = o.dump.clone().unwrap_or_else(|| dir.join("poly-compositor.ppm"));
    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe { libc::signal(libc::SIGUSR1, on_sigusr1 as *const () as libc::sighandler_t) };

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
    let mut host = unix::UnixHost::default();
    let keymap = host.keymap.clone();
    let mut output = OutputConfig { width, height, scale, ..OutputConfig::default() };
    if let Some(k) = screen.kms() {
        let out = k.output().clone();
        output.refresh_mhz = out.refresh_mhz();
        output.physical_mm = (out.mm.0 as i32, out.mm.1 as i32);
        // Clients may share dma-bufs made on the render node.
        host.dmabuf_device = unix::device_number("/dev/dri/renderD128").or_else(|| unix::device_number(&o.drm));
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
    println!("poly-compositor: listening on {} ({width}x{height}, scale {scale})", path.display());

    let mut desk = if o.desktop {
        let display = if matches!(screen, Screen::Kms(_)) { format!("{} (KMS)", o.drm) } else { o.fb.clone() };
        let opts = desk::Options {
            dir: &dir,
            fb_name: &display,
            keymap: o.keymap.as_deref(),
            scale: o.scale,
            settings: o.settings.clone(),
        };
        Some(desk::Desktop::new(&opts, &mut server, keymap, shell_font()))
    } else {
        None
    };
    let bus_address = desk.as_ref().and_then(|d| d.bus.as_ref()).map(session::SessionBus::address);

    let child = if o.exec.is_empty() {
        None
    } else {
        let env = client_env(&o.socket, &dir, &dump_path, bus_address.as_deref());
        let pid = unix::spawn(&o.exec, &env).map_err(|e| format!("{}: {e}", o.exec[0]))?;
        println!("poly-compositor: started {} (pid {pid})", o.exec[0]);
        Some(pid)
    };

    let mut conns: BTreeMap<ClientId, Connection> = BTreeMap::new();
    let mut next_frame = 0u64;
    let mut buf = vec![0u8; 4096];
    loop {
        // What to wait for, and how long.
        let mut fds = vec![libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
        let ids: Vec<ClientId> = conns.keys().copied().collect();
        fds.extend(ids.iter().map(|id| libc::pollfd {
            fd: conns[id].stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }));
        let input_at = fds.len();
        if let Some(d) = &desk {
            fds.extend(d.input.devices.iter().map(|dev| libc::pollfd { fd: dev.fd, events: libc::POLLIN, revents: 0 }));
        }
        // The session bus and the mixer (ADR 0117).
        let desk_at = fds.len();
        if let Some(d) = &desk {
            fds.extend(d.poll_fds());
        }
        let desk_end = fds.len();
        // The card's flip events.
        let card_fd = screen.kms().and_then(|k| k.fd());
        if let Some(fd) = card_fd {
            fds.push(libc::pollfd { fd, events: libc::POLLIN, revents: 0 });
        }
        let flip_pending = screen.kms().is_some_and(|k| k.is_pending());
        let now = unix::monotonic_ns();
        let timeout = if server.wants_frame() && !flip_pending {
            (next_frame.saturating_sub(now) / 1_000_000) as i32
        } else if flip_pending && card_fd.is_none() {
            1
        } else if let Some(d) = &desk {
            // The clock and the status, toasts that expire, apps that ended.
            d.timeout_ms()
        } else if child.is_some() {
            50
        } else {
            -1
        };
        unix::poll(&mut fds, timeout).map_err(|e| format!("poll: {e}"))?;

        if fds[0].revents & libc::POLLIN != 0 {
            match listener.accept() {
                Ok((stream, _)) => {
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
            match unix::recv(pfd.fd, &mut buf, &mut received) {
                Ok(0) => gone.push(*id),
                Ok(n) => server.receive(*id, &buf[..n], received),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
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
            let ms = (unix::monotonic_ns() / 1_000_000) as u32;
            for (i, pfd) in fds[input_at..desk_at].iter().enumerate() {
                if pfd.revents != 0 {
                    d.input.read(i, size, ms);
                }
            }
            d.polled(&fds[desk_at..desk_end]);
            while let Some(ev) = d.input.next_event() {
                if let Some(app) = d.route(ev, &mut server) {
                    if d.open_internal(&app, &mut server) {
                        continue;
                    }
                    let env = client_env(&o.socket, &dir, &dump_path, bus_address.as_deref());
                    match unix::spawn(&app.argv, &env) {
                        Ok(pid) => {
                            println!("poly-compositor: launched {} (pid {pid})", app.name);
                            d.children.push((pid, app.name.clone()));
                        }
                        Err(e) => eprintln!("poly-compositor: {}: {e}", app.argv[0]),
                    }
                }
            }
            d.sync(&mut server);
        }

        // A frame when one is due: with KMS whenever no flip is pending (the
        // flips pace it), else at most every 1/60 s.
        let now = unix::monotonic_ns();
        let kms = matches!(screen, Screen::Kms(_));
        let due = if kms { !screen.kms().is_some_and(|k| k.is_pending()) } else { now >= next_frame };
        if server.wants_frame() && due {
            if let Some(k) = screen.kms() {
                k.begin_frame();
            }
            let fb = screen.fb();
            let drawn = match &desk {
                Some(d) => server.render_with(fb, Some(&d.shell.below()), Some(&d.shell.above())),
                None => server.render(fb),
            };
            let shown = unix::monotonic_ns();
            if let Some(k) = screen.kms() {
                // Drawn frames flipped themselves (`present`); clients
                // waiting with nothing new to draw are paced by a flip too.
                if drawn.is_empty()
                    && let Err(e) = k.flip(&[])
                {
                    eprintln!("poly-compositor: page flip: {e}");
                }
                server.frame_submitted();
            } else {
                // Presented: the frame is on the screen once the device's
                // requests returned.
                server.frame_presented(FrameTime { monotonic_ns: shown });
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

        if DUMP.swap(false, Ordering::Relaxed) {
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

        if let Some(status) = child.and_then(unix::try_wait) {
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
            let _ = std::fs::remove_file(&path);
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

    /// The compositor's own transport: a client's bytes and a file
    /// descriptor through a socket pair, its shared memory mapped.
    #[test]
    fn passes_file_descriptors_and_maps_shared_memory() {
        let (a, b) = UnixStream::pair().unwrap();
        let path = std::env::temp_dir().join(format!("poly-compositor-test-{}", std::process::id()));
        std::fs::write(&path, [1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(unix::send(a.as_raw_fd(), b"hello", &[Fd(file.as_raw_fd())]).unwrap(), 5);
        let (mut buf, mut fds) = ([0u8; 16], Vec::new());
        assert_eq!(unix::recv(b.as_raw_fd(), &mut buf, &mut fds).unwrap(), 5);
        assert_eq!((&buf[..5], fds.len()), (&b"hello"[..], 1));
        use poly_wayland::server::Host;
        let mut map = unix::UnixHost::default().map_shm(fds[0], 8).unwrap();
        let mut out = [0u8; 4];
        assert!(map.read(4, &mut out));
        assert_eq!(out, [5, 6, 7, 8]);
        assert!(!map.read(6, &mut out));
        assert!(map.resize(8));
    }

    /// The desktop's layers through the scene into memory: the ridge, the
    /// launcher, the cursor; the picture as a PPM.
    #[test]
    fn the_desktop_composes() {
        let host = unix::UnixHost::default();
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
