//! Poly's desktop (ADR 0107): `cargo xtask desktop` boots straight into
//! the compositor and the shell on a virtio GPU, with a virtio keyboard
//! and tablet; boot group `desktop` does the same and drives it through
//! QMP.
//!
//! The image: `/bin/poly-compositor` (as init, `--desktop`), the Wayland
//! test clients `/bin/wl-demo` (the launcher's "Poly Demo") and
//! `/bin/wl-client` ("Wayland Test Client"), native programs of
//! `userland/poly-wl-demo` built on every host (ADR 0133), the Windows GUI
//! test program of `desktop-win32` (`/win/win_gui_fill.exe`, "Poly Fill",
//! below) where it can be built, a desktop file
//! `/usr/share/applications/demo-window.desktop` ("Demo Window", which
//! starts `wl-demo` too, so the launcher's desktop file path is used), and
//! DejaVu Sans from the font store and Alpine Shell's fonts (Geist, Geist
//! Mono, Averia Serif Libre; ADR 0127) when they are fetched (`cargo xtask
//! fonts fetch`; without them the shell draws in DejaVu Sans or with the
//! bitmap font).
//!
//! The boot test, on QEMU's screen (`screendump` of the virtio GPU) and
//! its input (`input-send-event`, QEMU's QMP reference):
//!
//! 1. the desktop is up: the ridge (rgb(32, 32, 32) along the top), the
//!    launcher hanging from it (ADR 0127), the wallpaper elsewhere;
//! 2. typing `demo` into the launcher lists the apps (the compositor logs
//!    the query); Enter starts the first, "Demo Window" (from the desktop
//!    file), whose window must appear, centred below the ridge, in its
//!    colour;
//! 3. input reaches it: a tablet move onto the window and a click (the
//!    client logs `wl_pointer` enter and the button, and swaps its colour,
//!    which the next screenshot shows), a key (it logs the keymap it
//!    mapped, keyboard enter and the key);
//! 4. dragging the title bar moves the window (its colour found at the new
//!    place);
//! 5. Q closes the client, and the launcher comes back on the empty
//!    desktop.
//!
//! Boot group `desktop-shell` (ADR 0117) boots the same desktop with a
//! virtio-sound card (the mixer) and an ext2 disk mounted on `/mnt` for
//! the settings (`--settings /mnt/poly/shell.conf`), and drives window
//! management, notifications and the settings ([`shell_boot_test`]); a
//! second boot from the same disk finds the settings it saved.
//!
//! Boot group `desktop-win32` (ADR 0131) boots the same desktop with a
//! Windows GUI program in the image (`tests/windows/win_gui_fill.rs`, built
//! by `pipeline win build`, behind a desktop file "Poly Fill"): typed into
//! the launcher, it creates an overlapped window and fills its client area
//! with one colour through GetDC / FillRect / ReleaseDC; through win32k's
//! desktop link (`/dev/win32k`) the window must show up on the desktop at
//! its client rectangle, framed by Poly, in that colour
//! ([`win32_boot_test`]). Skipped where the Windows test programs cannot be
//! built.
//!
//! Screenshots go to `$POLY_DESKTOP_SHOTS` (default `target/desktop/`) as
//! PNGs, one per step.

use super::{Boot, Result, Symbols, boot_with, has_desktop, initramfs, qemu, root, screen};
use crate::hostscreen;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// The screen the desktop boots on.
pub const SIZE: (u32, u32) = (1280, 800);
/// What the compositor prints when the first frame is on the screen.
const READY: &str = "poly-compositor: desktop ready";
/// The demo client's colours (`poly_wl_demo::demo`) and the ridge's.
const DEMO_A: [u8; 3] = [0x2f, 0x7d, 0x6d];
const DEMO_B: [u8; 3] = [0xc0, 0x8a, 0x2e];
const RIDGE: [u8; 3] = [0x20, 0x20, 0x20];
/// The desktop file the image carries.
const DESKTOP_FILE: &str = "[Desktop Entry]\nType=Application\nName=Demo Window\n\
    Comment=The demo client, from a desktop file\nExec=/bin/wl-demo\nKeywords=demo;sample;\n";
/// A desktop file for `notify-send` (ADR 0117): the launcher sends a
/// notification through `org.freedesktop.Notifications` with it.
const NOTIFY_FILE: &str = "[Desktop Entry]\nType=Application\nName=Send Test Notification\n\
    Comment=notify-send through org.freedesktop.Notifications\n\
    Exec=/bin/notify-send -a \"Poly Test\" \"Hello from a test client\" \"Sent through org.freedesktop.Notifications on the session bus\"\n\
    Keywords=notification;test;\n";
/// Guest memory: the shadow frame buffer and the clients need more than
/// the default.
const MEMORY: &str = "512M";

/// The kernel command line of a desktop boot.
pub fn append(extra: &str) -> String {
    format!("poly.init=/bin/poly-compositor poly.init.arg=--desktop poly.terminal=off {extra}").trim_end().to_string()
}

/// QEMU's devices for the desktop: a virtio GPU of [`SIZE`], a virtio
/// keyboard and tablet.
pub fn devices() -> Vec<String> {
    devices_sized(SIZE)
}

/// [`devices`] with a virtio GPU of `(w, h)`.
pub fn devices_sized((w, h): (u32, u32)) -> Vec<String> {
    [
        "-vga",
        "none",
        "-device",
        &format!("virtio-gpu-pci,xres={w},yres={h}"),
        "-device",
        "virtio-keyboard-pci",
        "-device",
        "virtio-tablet-pci",
    ]
    .map(String::from)
    .to_vec()
}

/// The programs in the desktop's image (`/bin/<name>`), all native
/// programs built on every host (ADR 0125, ADR 0133), Poly's own Files
/// and Terminal among them (ADR 0134).
pub const PROGRAMS: [&str; 6] =
    ["poly-compositor", "wl-demo", "wl-client", "notify-send", "poly-files", "poly-terminal"];

/// Whether this checkout has the Windows GUI test program of
/// `desktop-win32` ([`WIN32_TEST`], ADR 0131), which every desktop image
/// carries behind a desktop file ("Poly Fill") when it can be built here.
fn has_windows_app() -> bool {
    root().join(format!("tests/windows/{WIN32_TEST}.rs")).is_file()
}

/// The name a desktop file gives its app.
fn desktop_name(file: &str) -> &str {
    file.lines().find_map(|l| l.strip_prefix("Name=")).unwrap_or("?")
}

/// What the desktop's launcher offers, beyond its settings and
/// notifications: (name in the launcher, what starts it). For `cargo xtask
/// start`'s menu.
pub fn launcher_apps() -> Vec<(String, String)> {
    let mut apps: Vec<(String, String)> = [
        ("Poly Demo", "/bin/wl-demo (built in)"),
        ("Wayland Test Client", "/bin/wl-client (built in)"),
        (desktop_name(DESKTOP_FILE), "/bin/wl-demo (demo-window.desktop)"),
        (desktop_name(NOTIFY_FILE), "/bin/notify-send (notify-test.desktop)"),
        ("Files", "/bin/poly-files (built in)"),
        ("Terminal", "/bin/poly-terminal (built in)"),
    ]
    .into_iter()
    .map(|(n, w)| (n.to_string(), w.to_string()))
    .collect();
    if has_windows_app() {
        apps.push((desktop_name(WIN32_DESKTOP_FILE).to_string(), format!("/win/{WIN32_TEST}.exe (poly-fill.desktop)")));
    }
    apps
}

/// The Windows GUI test program built (on any host, [`crate::winapp`]),
/// with its desktop file; left out, saying why, where it cannot be built.
fn windows_app_files() -> Vec<(String, u16, Vec<u8>)> {
    if !has_windows_app() {
        return Vec::new();
    }
    match crate::winapp::build(&[WIN32_TEST]) {
        Ok(built) => built
            .into_iter()
            .flat_map(|(name, exe)| {
                [
                    (format!("win/{name}.exe"), 0o755, exe),
                    (
                        "usr/share/applications/poly-fill.desktop".to_string(),
                        0o644,
                        WIN32_DESKTOP_FILE.as_bytes().to_vec(),
                    ),
                ]
            })
            .collect(),
        Err(e) => {
            crate::progress::note(&format!("without the Windows test app {WIN32_TEST}: {e}"));
            Vec::new()
        }
    }
}

/// Build the desktop's initramfs, with the Windows test app when it can
/// be built; returns its path.
pub fn image() -> Result<PathBuf> {
    build_image("poly-desktop-initramfs.cpio", windows_app_files)
}

/// [`image`] with `extra` files (path, mode, bytes) instead of the Windows
/// app, written to `name`.
fn image_with(name: &str, extra: Vec<(String, u16, Vec<u8>)>) -> Result<PathBuf> {
    build_image(name, move || extra)
}

/// The desktop's initramfs with the files `extra` gives, written to `name`.
fn build_image(name: &str, extra: impl FnOnce() -> Vec<(String, u16, Vec<u8>)> + Send) -> Result<PathBuf> {
    // Every program is a native program, built on any host (ADR 0125;
    // the Wayland test clients since ADR 0133). The files are gathered
    // while they build.
    let (built, files, extra) = thread::scope(|s| {
        let files = s.spawn(|| crate::progress::task("desktop files (fonts, wallpapers)", image_files));
        let extra = s.spawn(extra);
        let built = crate::native::build();
        let files = files.join().unwrap_or_else(|_| Err("gathering the desktop's files panicked".into()));
        (built, files, extra.join().unwrap_or_default())
    });
    let (built, mut tree) = (built?, files?);
    tree.files.extend(extra);
    let programs: Vec<(&str, &[u8])> = PROGRAMS
        .iter()
        .map(|name| {
            let elf = built.iter().find(|(n, _)| n == name).map(|(_, e)| e.as_slice());
            elf.map(|e| (*name, e)).ok_or_else(|| format!("no native program {name}"))
        })
        .collect::<Result<_>>()?;
    crate::progress::task("desktop image (initramfs)", || {
        initramfs::write(&root(), name, &initramfs::programs_image_with(&programs, &tree))
    })
}

/// The desktop image's files besides the programs: the desktop files, the
/// fonts, the wallpapers.
fn image_files() -> Result<initramfs::Tree> {
    let mut tree = initramfs::Tree::default();
    tree.files.push(("usr/share/applications/demo-window.desktop".into(), 0o644, DESKTOP_FILE.as_bytes().into()));
    tree.files.push(("usr/share/applications/notify-test.desktop".into(), 0o644, NOTIFY_FILE.as_bytes().into()));
    let font = crate::fonts::cache_dir().join("DejaVuSans.ttf");
    match std::fs::read(&font) {
        Ok(data) => tree.files.push(("usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into(), 0o644, data)),
        Err(_) => {
            crate::progress::note("no DejaVu fonts (`cargo xtask fonts fetch`): the shell will use the bitmap font")
        }
    }
    // Alpine Shell's wallpapers and fonts (ADR 0127).
    for (path, data) in crate::wallpapers::tree()? {
        tree.files.push((path, 0o644, data));
    }
    let shell_fonts = crate::fonts::shell_tree();
    if shell_fonts.is_empty() {
        crate::progress::note("no shell fonts (`cargo xtask fonts fetch`): the shell draws in DejaVu Sans");
    }
    for (path, data) in shell_fonts {
        tree.files.push((path, 0o644, data));
    }
    // BusyBox, when there is one (a shell for the Terminal, ADR 0134):
    // `/bin/busybox` and its applets, a home and a `/tmp`.
    match crate::busybox::obtain()? {
        Ok(bb) => {
            tree.files.push(("bin/busybox".into(), 0o755, bb.elf));
            for (dir, applets) in initramfs::BUSYBOX_APPLETS {
                for applet in *applets {
                    tree.symlinks.push((format!("{dir}/{applet}"), "/bin/busybox".into()));
                }
            }
        }
        Err(why) => {
            crate::progress::note(&format!("no BusyBox in the desktop image, so the Terminal has no shell: {why}"))
        }
    }
    tree.files.push(("root/.profile".into(), 0o644, b"export PS1='\\w \\$ '\n".to_vec()));
    tree.files.push(("tmp/.keep".into(), 0o644, Vec::new()));
    Ok(tree)
}

/// The settings disk `cargo xtask desktop` keeps between runs (ext2,
/// mounted on `/mnt`; the settings, keyboard shortcuts included, in
/// `/mnt/poly/shell.conf`).
pub const STATE_FILE: &str = "poly-desktop-state.img";

/// `cargo xtask desktop [--release] [--fresh-state] [--size=WxH]
/// [--scale=N] [kernel args...]`: boot into the desktop in a QEMU window
/// (no window without a desktop: then a VNC display on port 5900). The
/// settings live on a small disk, `target/poly-desktop-state.img`, so they
/// survive from one run to the next (`--fresh-state` starts again with the
/// defaults).
///
/// The screen is [`SIZE`] times the host's display scale, with Poly's own
/// scale (`poly.display.scale=`) set to it, so text and icons are drawn
/// sharp at the host's pixel density instead of stretched by the host; a
/// smaller scale (or screen) where that would not fit the host's main
/// screen ([`hostscreen::fit`]). `--scale=N` and `--size=WxH` override them.
/// A scale other than 1 is given to the compositor as `--scale`, so it wins
/// over the scale in the saved settings.
///
/// `--build-only` builds everything and stops before the boot.
pub fn command(args: &[String]) -> Result {
    let ready = prepare(args)?;
    if args.iter().any(|a| a == "--build-only") {
        eprintln!("==> built: {}, {}, {}", ready.kernel.display(), ready.image.display(), ready.state.display());
        return Ok(());
    }
    boot(ready)
}

/// What [`prepare`] made for [`boot`].
#[derive(Debug)]
pub struct Prepared {
    kernel: PathBuf,
    image: PathBuf,
    state: PathBuf,
    size: (u32, u32),
    scale: u32,
    /// Whether `--scale` was given.
    asked: bool,
    host: Option<hostscreen::Screen>,
    extra: Vec<String>,
}

/// The options of [`command`] read, and everything built for the boot: the
/// kernel (with the native programs), the image and the settings disk, at
/// the same time (`cargo xtask start` shows them as a table, [`crate::progress`]).
pub fn prepare(args: &[String]) -> Result<Prepared> {
    let release = args.iter().any(|a| a == "--release");
    let fresh = args.iter().any(|a| a == "--fresh-state");
    let value = |name: &str| args.iter().find_map(|a| a.strip_prefix(name)).map(str::to_string);
    let asked = match value("--scale=") {
        Some(v) => {
            Some(v.parse::<u32>().ok().filter(|s| (1..=4).contains(s)).ok_or(format!("--scale={v}: expected 1 to 4"))?)
        }
        None => None,
    };
    let host = hostscreen::main_screen();
    // (size, scale) for the host's main screen.
    let fitted = host.map(|h| hostscreen::fit(SIZE, h, WINDOW_TITLE));
    let scale = asked.or(fitted.map(|f| f.1)).unwrap_or(1);
    let size = match value("--size=") {
        Some(v) => parse_size(&v).ok_or(format!("--size={v}: expected WxH, such as 2560x1600"))?,
        None => match fitted {
            Some((size, s)) if asked.is_none() || asked == Some(s) => size,
            _ => (SIZE.0 * scale, SIZE.1 * scale),
        },
    };
    let extra: Vec<String> = args.iter().filter(|a| !a.starts_with("--")).cloned().collect();
    let state = root().join("target").join(STATE_FILE);
    let (kernel, image, disk) = thread::scope(|s| {
        let kernel = s.spawn(|| super::build_kernel(release));
        let disk = s.spawn(|| -> Result {
            if fresh || !state.exists() {
                let disk = crate::ext2disk::image()?;
                std::fs::write(&state, disk).map_err(|e| format!("{}: {e}", state.display()))?;
                crate::progress::note(&format!("a new settings disk {}", state.display()));
            } else {
                crate::progress::note(&format!(
                    "the settings disk {} is kept from before (--fresh-state starts again)",
                    state.display()
                ));
            }
            Ok(())
        });
        let image = image();
        let kernel = kernel.join().unwrap_or_else(|_| Err("building the kernel panicked".into()));
        (kernel, image, disk.join().unwrap_or_else(|_| Err("making the settings disk panicked".into())))
    });
    let (kernel, image) = (kernel?, image?);
    disk?;
    Ok(Prepared { kernel, image, state, size, scale, asked: asked.is_some(), host, extra })
}

/// Boot what [`prepare`] made, in QEMU's window.
pub fn boot(p: Prepared) -> Result {
    let Prepared { kernel, image, state, size, scale, asked, host, extra } = p;
    // The compositor's `--scale` wins over the saved settings' scale (which
    // a run at another scale may have saved): asked for, or the host's when
    // that is not 1. At scale 1 the settings choose, as before.
    let force =
        if asked || scale != 1 { format!("poly.init.arg=--scale poly.init.arg={scale}") } else { String::new() };
    let kernel_args = format!("{SETTINGS_ARGS} poly.display.scale={scale} {force} {}", extra.join(" "));
    let mut cmd = qemu(&kernel, MEMORY, &append(kernel_args.trim_end()), super::Devices::None, "mon:stdio");
    cmd.arg("-initrd").arg(&image).args(devices_sized(size)).args(state_drive(&state));
    eprintln!("==> screen {}x{} at scale {scale} (--size=WxH, --scale=N)", size.0, size.1);
    // A QMP socket to drive the desktop from outside (scripted checks on a
    // host without Unix sockets): `POLY_QEMU_QMP=tcp:127.0.0.1:4444`.
    if let Some(qmp) = std::env::var("POLY_QEMU_QMP").ok().filter(|q| !q.is_empty()) {
        cmd.args(["-qmp", &format!("{qmp},server=on,wait=off")]);
        eprintln!("==> QMP on {qmp} (POLY_QEMU_QMP)");
    }
    let sdl = window_display(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("qemu: {e}"))?;
    if let (true, Some(h)) = (sdl, host) {
        hostscreen::place_window(child.id(), size, h);
    }
    let _ = child.wait().map_err(|e| format!("qemu: {e}"))?;
    Ok(())
}

/// The height of QEMU's window title bar, in logical pixels (room left
/// for it when the window is fitted to the screen).
const WINDOW_TITLE: u32 = 32;

/// `WxH` as a screen size (each side 320 to 8192).
fn parse_size(v: &str) -> Option<(u32, u32)> {
    let (w, h) = v.split_once(['x', 'X'])?;
    let side = |s: &str| s.parse::<u32>().ok().filter(|n| (320..=8192).contains(n));
    Some((side(w)?, side(h)?))
}

/// The QEMU window an interactive desktop boot shows on (`POLY_QEMU_DISPLAY`
/// overrides it; VNC on port 5900 without a desktop); whether it is SDL's.
/// On Windows that is SDL, marked DPI-aware so Windows does not stretch it:
/// QEMU's GTK window there tells the guest its size in scaled units (a
/// 1280x800 screen became 640x480, stretched back up).
pub fn window_display(cmd: &mut std::process::Command) -> bool {
    if let Some(display) = std::env::var("POLY_QEMU_DISPLAY").ok().filter(|d| !d.is_empty()) {
        cmd.args(["-display", &display]);
        eprintln!("==> QEMU display {display} (POLY_QEMU_DISPLAY)");
        return display.starts_with("sdl");
    } else if cfg!(windows) {
        cmd.args(["-display", "sdl"]).env("SDL_WINDOWS_DPI_AWARENESS", "permonitorv2");
        eprintln!("==> the desktop is in QEMU's window; the Windows key or Ctrl+Space toggles the launcher");
        return true;
    } else if has_desktop() {
        eprintln!("==> the desktop is in QEMU's window; the Windows key or Ctrl+Space toggles the launcher");
    } else {
        cmd.args(["-display", "vnc=0.0.0.0:0"]);
        eprintln!("==> no desktop here: the screen is on VNC, port 5900 (Ctrl-A X quits)");
    }
    false
}

/// Where the screenshots go.
fn shots_dir() -> PathBuf {
    std::env::var_os("POLY_DESKTOP_SHOTS").map_or_else(|| root().join("target").join("desktop"), PathBuf::from)
}

/// The serial lines so far, shared with the thread that drives QEMU.
type Lines = Arc<Mutex<Vec<String>>>;

/// Wait until a line containing `text` came (`timeout`); returns it.
fn wait_line(lines: &Lines, text: &str, timeout: Duration) -> std::result::Result<String, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(l) = lines.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|l| l.contains(text)) {
            return Ok(l.clone());
        }
        if Instant::now() > deadline {
            return Err(format!("no line {text:?} within {} s", timeout.as_secs()));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn key(qcode: &str, down: bool) -> String {
    format!(r#"{{"type": "key", "data": {{"down": {down}, "key": {{"type": "qcode", "data": "{qcode}"}}}}}}"#)
}

fn button(down: bool) -> String {
    format!(r#"{{"type": "btn", "data": {{"down": {down}, "button": "left"}}}}"#)
}

/// A tablet position for logical pixel (`x`, `y`) of the screen: the
/// compositor scales 0..0x7fff onto 0..size-1.
fn abs(x: i32, y: i32) -> Vec<String> {
    let (w, h) = (SIZE.0 as i64 - 1, SIZE.1 as i64 - 1);
    let ax = (i64::from(x) * 0x7fff + w - 1) / w;
    let ay = (i64::from(y) * 0x7fff + h - 1) / h;
    vec![
        format!(r#"{{"type": "abs", "data": {{"axis": "x", "value": {ax}}}}}"#),
        format!(r#"{{"type": "abs", "data": {{"axis": "y", "value": {ay}}}}}"#),
    ]
}

/// The driver: QMP commands and the checks between them.
struct Driver {
    qmp: screen::Qmp,
    lines: Lines,
    shots: PathBuf,
    step: u32,
}

impl Driver {
    fn send(&mut self, events: &[String]) -> std::result::Result<(), String> {
        self.qmp.execute("input-send-event", &format!(r#"{{"events": [{}]}}"#, events.join(", ")))?;
        thread::sleep(Duration::from_millis(120));
        Ok(())
    }

    fn type_text(&mut self, keys: &[&str]) -> std::result::Result<(), String> {
        for k in keys {
            self.send(&[key(k, true)])?;
            self.send(&[key(k, false)])?;
        }
        Ok(())
    }

    /// A screenshot named for the step, read back.
    fn shot(&mut self, what: &str) -> std::result::Result<screen::Image, String> {
        self.step += 1;
        let png = self.shots.join(format!("desktop-{}-{what}.png", self.step));
        let ppm = self.shots.join(format!("desktop-{}-{what}.ppm", self.step));
        self.qmp.screendump(&png)?;
        self.qmp.screendump(&ppm)?;
        let img = screen::Image::read_ppm(&ppm)?;
        let _ = std::fs::remove_file(&ppm);
        eprintln!("==> screenshot {}", png.display());
        Ok(img)
    }

    fn wait(&self, text: &str, secs: u64) -> std::result::Result<String, String> {
        wait_line(&self.lines, text, Duration::from_secs(secs))
    }
}

/// The window's position and size from the compositor's line
/// `window "Poly Demo" shown at X,Y size WxH`.
fn window_place(line: &str) -> Option<(i32, i32, i32, i32)> {
    let rest = line.split(" shown at ").nth(1)?;
    let (pos, size) = rest.split_once(" size ")?;
    let (x, y) = pos.split_once(',')?;
    let (w, h) = size.trim().split_once('x')?;
    Some((x.parse().ok()?, y.parse().ok()?, w.parse().ok()?, h.parse().ok()?))
}

fn expect(what: &str, ok: bool) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(format!("the desktop boot test: {what}")) }
}

/// Every step of the test (see the module documentation).
fn drive(socket: &Path, lines: Lines) -> std::result::Result<(), String> {
    let shots = shots_dir();
    std::fs::create_dir_all(&shots).map_err(|e| format!("{}: {e}", shots.display()))?;
    let qmp = screen::Qmp::connect(socket, Duration::from_secs(20))?;
    let mut d = Driver { qmp, lines, shots, step: 0 };
    thread::sleep(Duration::from_millis(500));

    // 1. The ridge and the launcher.
    let img = d.shot("launcher")?;
    expect("the screen is 1280x800", (img.width, img.height) == SIZE)?;
    expect("the ridge runs along the top", img.pixel(640, 6) == RIDGE && img.pixel(640, 20) == RIDGE)?;
    expect("the wallpaper is below the ridge", img.pixel(100, 26) != RIDGE && img.pixel(100, 700) != RIDGE)?;
    // The launcher hangs from the ridge (Alpine Shell's look, ADR 0127):
    // its search row 720 wide, x 280..1000, y 24..72, in the ridge's colour.
    expect("the launcher hangs from the ridge", img.pixel(900, 40) == RIDGE && img.pixel(581, 64) == RIDGE)?;
    expect("not outside the launcher", img.pixel(200, 40) != RIDGE && img.pixel(640, 90) != RIDGE)?;

    // 2. Type, list, launch.
    d.type_text(&["d", "e", "m", "o"])?;
    d.wait("launcher query \"demo\"", 30)?;
    thread::sleep(Duration::from_millis(800));
    let img = d.shot("typed")?;
    // Rows: the first, selected, is lighter than the panel.
    let row = img.pixel(640, 72 + 1 + 28 + 10);
    expect("the launcher lists apps (a selected row)", row != RIDGE && row[0] > RIDGE[0])?;
    d.type_text(&["ret"])?;
    d.wait("poly-compositor: launched Demo Window", 30)?;
    let line = d.wait("window \"Poly Demo\" shown at", 90)?;
    let (x, y, w, _h) = window_place(&line).ok_or_else(|| format!("unreadable window line {line:?}"))?;
    expect("the window is centred below the ridge", (x + w / 2 - 640).abs() <= 2 && y > 24)?;
    thread::sleep(Duration::from_millis(1500));
    let img = d.shot("window")?;
    let inside = (x as u32 + 20, y as u32 + 20);
    expect("the client's window is on the screen", img.pixel(inside.0, inside.1) == DEMO_A)?;
    expect("the launcher is gone", img.pixel(900, 40) != RIDGE)?;

    // 3. Pointer and keyboard reach the client.
    let mut ev = abs(x + w / 2, y + 30);
    d.send(&ev)?;
    d.send(&[button(true)])?;
    d.send(&[button(false)])?;
    d.type_text(&["a"])?;
    for line in [
        "wl-demo: keymap format 1 size",
        "wl-demo: keyboard enter",
        "wl-demo: pointer enter",
        "wl-demo: button 272 pressed",
        "wl-demo: key 30 pressed",
    ] {
        d.wait(line, 30)?;
    }
    expect("the keymap the client mapped is an XKB keymap", d.wait("wl-demo: keymap format 1", 1)?.contains("xkb ok"))?;
    thread::sleep(Duration::from_millis(1500));
    let img = d.shot("clicked")?;
    expect("the click recoloured the window", img.pixel(inside.0, inside.1) == DEMO_B)?;

    // 4. Drag the title bar (12 pixels above the content) by (100, 60).
    ev = abs(x + 40, y - 12);
    d.send(&ev)?;
    d.send(&[button(true)])?;
    for i in 1..=4 {
        d.send(&abs(x + 40 + 25 * i, y - 12 + 15 * i))?;
    }
    d.send(&[button(false)])?;
    thread::sleep(Duration::from_millis(1500));
    let img = d.shot("moved")?;
    let moved = (inside.0 + 100, inside.1 + 60);
    expect("the window moved with the drag", img.pixel(moved.0, moved.1) == DEMO_B)?;

    // 5. Q closes it; the launcher returns on the empty desktop.
    d.send(&abs(x + 100 + w / 2, y + 60 + 30))?;
    d.type_text(&["q"])?;
    d.wait("wl-demo: closed", 30)?;
    thread::sleep(Duration::from_millis(1500));
    let img = d.shot("empty-again")?;
    expect("the launcher is back on the empty desktop", img.pixel(900, 40) == RIDGE)?;
    d.qmp.quit();
    Ok(())
}

/// A left click at logical (`x`, `y`).
fn click(d: &mut Driver, x: i32, y: i32) -> std::result::Result<(), String> {
    d.send(&abs(x, y))?;
    d.send(&[button(true)])?;
    d.send(&[button(false)])
}

/// Keys pressed together (in order) and released (in reverse).
fn chord(d: &mut Driver, keys: &[&str]) -> std::result::Result<(), String> {
    for k in keys {
        d.send(&[key(k, true)])?;
    }
    for k in keys.iter().rev() {
        d.send(&[key(k, false)])?;
    }
    Ok(())
}

/// The window's place from the newest line `window "TITLE" shown at`.
fn shown(d: &Driver, title: &str, secs: u64) -> std::result::Result<(i32, i32, i32, i32), String> {
    let line = d.wait(&format!("window \"{title}\" shown at"), secs)?;
    window_place(&line).ok_or_else(|| format!("unreadable window line {line:?}"))
}

/// What the compositor prints when a shortcut shows or hides the launcher.
const LAUNCHER_SHOWN: &str = "poly-compositor: launcher shown";
const LAUNCHER_HIDDEN: &str = "poly-compositor: launcher hidden";

/// How many lines so far contain `text`.
fn count(lines: &Lines, text: &str) -> usize {
    lines.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|l| l.contains(text)).count()
}

/// Wait for the `n`th line containing `text`.
fn wait_nth(lines: &Lines, text: &str, n: usize, secs: u64) -> std::result::Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let count = lines.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|l| l.contains(text)).count();
        if count >= n {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("{count} of {n} lines {text:?} within {secs} s"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// The work area's frame for a 1280×800 screen with the 24-pixel ridge:
/// what a maximized demo window's buffer is (less the frame).
const MAXIMIZED: &str = "wl-demo: size 1278x750";
/// The left or right half (Alpine's 8-pixel gap), less the frame.
const HALF: &str = "wl-demo: size 630x742";

/// Every step of the shell's boot test (see [`shell_boot_test`]).
fn drive_shell(socket: &Path, lines: Lines, sound: bool) -> std::result::Result<(), String> {
    let shots = shots_dir();
    std::fs::create_dir_all(&shots).map_err(|e| format!("{}: {e}", shots.display()))?;
    let qmp = screen::Qmp::connect(socket, Duration::from_secs(20))?;
    let mut d = Driver { qmp, lines: Arc::clone(&lines), shots, step: 100 };
    thread::sleep(Duration::from_millis(500));
    d.wait("session bus on unix:path=", 1)?;

    // 1. The demo window, maximized with the frame's middle button.
    d.type_text(&["d", "e", "m", "o"])?;
    d.wait("launcher query \"demo\"", 30)?;
    d.type_text(&["ret"])?;
    let (x, y, w, _h) = shown(&d, "Poly Demo", 90)?;
    thread::sleep(Duration::from_millis(1000));
    let max_button = (x + w - 4 - 8 - 22, y - 12);
    click(&mut d, max_button.0, max_button.1)?;
    d.wait(MAXIMIZED, 30)?;
    thread::sleep(Duration::from_millis(1500));
    let img = d.shot("maximized")?;
    expect("the maximized window fills the work area", img.pixel(40, 60) == DEMO_A && img.pixel(1240, 760) == DEMO_A)?;
    expect("the ridge stays above it", img.pixel(640, 6) == RIDGE)?;

    // 2. A double click on the title bar restores it.
    d.send(&abs(400, 24 + 12))?;
    d.send(&[button(true)])?;
    d.send(&[button(false)])?;
    d.send(&[button(true)])?;
    d.send(&[button(false)])?;
    d.wait("wl-demo: size 360x240", 30)?;
    thread::sleep(Duration::from_millis(1500));
    let img = d.shot("restored")?;
    expect("restored: the wallpaper shows again", img.pixel(40, 60) != DEMO_A)?;

    // 3. Super+Left tiles it to the left half.
    chord(&mut d, &["meta_l", "left"])?;
    d.wait("snapped Left to 4,28 632x768", 30)?;
    d.wait(HALF, 30)?;
    thread::sleep(Duration::from_millis(1500));
    let img = d.shot("tiled-left")?;
    expect("the left half", img.pixel(300, 700) == DEMO_A && img.pixel(1000, 700) != DEMO_A)?;
    let wallpaper = img.pixel(1000, 600);

    // 4. Dragged by its title bar to the right edge: the preview, then the
    // right half.
    d.send(&abs(300, 28 + 1 + 12))?;
    d.send(&[button(true)])?;
    for i in 1..=8 {
        d.send(&abs(300 + 122 * i, 41 + 45 * i))?;
    }
    d.send(&abs(1279, 400))?;
    thread::sleep(Duration::from_millis(1200));
    let img = d.shot("snap-preview")?;
    expect("the snap preview over the right half", img.pixel(1000, 600) != wallpaper)?;
    d.send(&[button(false)])?;
    d.wait("snapped Right to 644,28 632x768", 30)?;
    wait_nth(&lines, HALF, 2, 30)?;
    thread::sleep(Duration::from_millis(1500));
    let img = d.shot("tiled-right")?;
    expect("the right half", img.pixel(1000, 700) == DEMO_A && img.pixel(300, 700) != DEMO_A)?;

    // 5. A notification from a test client through the session bus.
    chord(&mut d, &["ctrl", "spc"])?;
    d.type_text(&["s", "e", "n", "d"])?;
    d.wait("launcher query \"send\"", 30)?;
    d.type_text(&["ret"])?;
    d.wait("notification 1 from Poly Test: \"Hello from a test client\"", 60)?;
    let exit = d.wait("exited with status", 60)?;
    expect("notify-send exited 0", exit.contains("Send Test Notification") && exit.contains("status 0"))?;
    thread::sleep(Duration::from_millis(1500));
    let img = d.shot("notification")?;
    expect("the toast in the top right corner", img.pixel(1280 - 8 - 60, 24 + 8 + 40) == RIDGE)?;

    // 6. The settings window: the volume and the keyboard layout.
    chord(&mut d, &["meta_l", "i"])?;
    d.wait("settings window opened", 30)?;
    let (sx, sy, sw, sh) = shown(&d, "Settings", 30)?;
    expect("the settings window is 900x720", (sw, sh) == (900, 720))?;
    click(&mut d, sx + 400, sy + 339)?; // Home's card to Sound (ADR 0127: Alpine's current settings)
    thread::sleep(Duration::from_millis(1000));
    click(&mut d, sx + 691, sy + 178)?; // 30 % of the volume slider
    d.wait("settings saved to /mnt/poly/shell.conf", 30)?;
    if sound {
        d.wait("poly-compositor: volume 30", 30)?;
    }
    thread::sleep(Duration::from_millis(1500));
    d.shot("settings-sound")?;
    click(&mut d, sx + 100, sy + 138)?; // Keyboard in the Devices pages
    thread::sleep(Duration::from_millis(1000));
    click(&mut d, sx + 500, sy + 221)?; // German (Switzerland)
    d.wait("poly-compositor: keymap de-ch", 30)?;
    // The demo client got the new keymap.
    wait_nth(&lines, "wl-demo: keymap format 1", 2, 30)?;
    thread::sleep(Duration::from_millis(1500));
    d.shot("settings-keyboard")?;

    // 7. Keyboard shortcuts (ADR 0129): the Search hotkey rebound to
    // Ctrl+Alt+K in the settings with the pointer and keys; the new
    // combination opens and closes the launcher, the Windows key no longer
    // does; Restore all defaults brings the Windows key back.
    d.type_text(&["up", "up", "up"])?; // Keyboard -> Sound -> Displays -> Keyboard shortcuts
    thread::sleep(Duration::from_millis(800));
    click(&mut d, sx + 858, sy + 178)?; // the Search hotkey's Set
    thread::sleep(Duration::from_millis(800));
    d.shot("shortcut-recording")?;
    chord(&mut d, &["ctrl", "alt", "k"])?;
    d.wait("shortcut launcher is Ctrl+Alt+K", 30)?;
    thread::sleep(Duration::from_millis(1000));
    d.shot("shortcut-changed")?;
    let (shown, hidden) = (count(&lines, LAUNCHER_SHOWN), count(&lines, LAUNCHER_HIDDEN));
    d.type_text(&["meta_l"])?;
    thread::sleep(Duration::from_millis(1500));
    expect("the Windows key no longer toggles the launcher", count(&lines, LAUNCHER_SHOWN) == shown)?;
    chord(&mut d, &["ctrl", "alt", "k"])?;
    wait_nth(&lines, LAUNCHER_SHOWN, shown + 1, 30)?;
    thread::sleep(Duration::from_millis(1500));
    let img = d.shot("shortcut-launcher")?;
    expect("Ctrl+Alt+K opened the launcher", img.pixel(900, 40) == RIDGE)?;
    chord(&mut d, &["ctrl", "alt", "k"])?;
    wait_nth(&lines, LAUNCHER_HIDDEN, hidden + 1, 30)?;
    thread::sleep(Duration::from_millis(1000));
    click(&mut d, sx + 872, sy + 77)?; // Restore all defaults
    d.wait("every shortcut back to its default", 30)?;
    d.type_text(&["meta_l"])?;
    wait_nth(&lines, LAUNCHER_SHOWN, shown + 2, 30)?;
    d.type_text(&["meta_l"])?;
    wait_nth(&lines, LAUNCHER_HIDDEN, hidden + 2, 30)?;
    thread::sleep(Duration::from_millis(1000));

    // 8. Alt+Tab: the switcher over the windows; releasing Alt switches
    // back to the demo.
    d.send(&[key("alt", true)])?;
    d.send(&[key("tab", true)])?;
    d.send(&[key("tab", false)])?;
    d.wait("switcher open: [\"Settings\", \"Poly Demo\"]", 30)?;
    thread::sleep(Duration::from_millis(1500));
    let img = d.shot("switcher")?;
    expect("the switcher hangs from the ridge", img.pixel(640, 30) == RIDGE)?;
    d.send(&[key("alt", false)])?;
    d.wait("switched to \"Poly Demo\"", 30)?;

    // 9. Minimized with the frame's button, back with Alt+Tab.
    let min_button = (645 + 630 - 4 - 8 - 44, 53 - 12);
    click(&mut d, min_button.0, min_button.1)?;
    thread::sleep(Duration::from_millis(4000));
    let img = d.shot("minimized")?;
    expect("minimized: the right half shows no demo", img.pixel(1000, 700) != DEMO_A)?;
    d.send(&[key("alt", true)])?;
    d.send(&[key("tab", true)])?;
    d.send(&[key("tab", false)])?;
    d.send(&[key("alt", false)])?;
    wait_nth(&lines, "switched to \"Poly Demo\"", 2, 30)?;
    thread::sleep(Duration::from_millis(3000));
    let img = d.shot("unminimized")?;
    expect("back from the switcher", img.pixel(1000, 700) == DEMO_A)?;

    // 10. Alt+F4 closes it.
    chord(&mut d, &["alt", "f4"])?;
    d.wait("wl-demo: closed", 30)?;
    thread::sleep(Duration::from_millis(1000));
    d.qmp.quit();
    Ok(())
}

/// QEMU's devices for the shell's boot: the desktop's, a virtio-sound card
/// if QEMU has one, and the settings disk (the first virtio-blk device,
/// mounted on `/mnt`).
fn shell_devices(state: &Path, sound: bool) -> Vec<String> {
    let mut v = devices();
    if sound {
        v.extend(["-device", "virtio-sound-pci,audiodev=snd0", "-audiodev", "none,id=snd0"].map(String::from));
    }
    v.extend(state_drive(state));
    v
}

/// The settings disk as the first virtio-blk device.
fn state_drive(state: &Path) -> Vec<String> {
    vec![
        "-drive".to_string(),
        format!("if=none,id=state,format=raw,file={}", state.display()),
        "-device".to_string(),
        "virtio-blk-pci,drive=state,disable-legacy=on".to_string(),
    ]
}

/// The kernel's arguments for the settings disk: mounted on `/mnt`, the
/// compositor's settings file on it.
const SETTINGS_ARGS: &str = "poly.mount=virtio0:/mnt poly.init.arg=--settings poly.init.arg=/mnt/poly/shell.conf";

/// The kernel command line of the shell's boots.
fn shell_append() -> String {
    append(SETTINGS_ARGS)
}

/// One boot of the shell test with `drive` run once the desktop is up.
fn shell_boot(
    kernel: &Path,
    syms: &Arc<Symbols>,
    image: &Path,
    state: &Path,
    sound: bool,
    drive: fn(&Path, Lines, bool) -> std::result::Result<(), String>,
) -> Result<(Boot, Option<std::result::Result<(), String>>)> {
    let socket = std::env::temp_dir().join(format!("poly-shell-qmp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let mut qemu_args = vec![
        "-initrd".to_string(),
        image.display().to_string(),
        "-qmp".to_string(),
        format!("unix:{},server=on,wait=off", socket.display()),
    ];
    qemu_args.extend(shell_devices(state, sound));
    let lines: Lines = Arc::default();
    let result: Arc<Mutex<Option<std::result::Result<(), String>>>> = Arc::default();
    let (seen, outcome, path) = (Arc::clone(&lines), Arc::clone(&result), socket.clone());
    let mut started = false;
    let on_line = move |line: &str| {
        seen.lock().unwrap_or_else(|e| e.into_inner()).push(line.to_string());
        if !started && line.contains(READY) {
            started = true;
            let (lines, outcome, path) = (Arc::clone(&seen), Arc::clone(&outcome), path.clone());
            thread::spawn(move || {
                let r = drive(&path, lines, sound);
                *outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            });
        }
    };
    let b: Result<Boot> = super::with_boot_timeout(Duration::from_secs(600), || {
        boot_with(kernel, syms, MEMORY, &shell_append(), super::Devices::None, &qemu_args, None, on_line)
    });
    let _ = std::fs::remove_file(&socket);
    let b = b?;
    let r = result.lock().unwrap_or_else(|e| e.into_inner()).take();
    Ok((b, r))
}

/// The second boot: up, then quit.
fn just_up(socket: &Path, _lines: Lines, _sound: bool) -> std::result::Result<(), String> {
    let mut qmp = screen::Qmp::connect(socket, Duration::from_secs(20))?;
    thread::sleep(Duration::from_millis(500));
    qmp.quit();
    Ok(())
}

/// Boot group `desktop-shell` (ADR 0117): window management (the frame's
/// maximize and minimize buttons, a double click on the title, Super+Left,
/// dragging to the right edge with the snap preview, Alt+Tab with the
/// switcher, Alt+F4), a notification sent by `notify-send` through
/// `org.freedesktop.Notifications` on the session bus, the settings
/// window changing the volume (through the mixer) and the keyboard layout
/// (the client gets the new keymap), and a keyboard shortcut rebound,
/// used and reset (ADR 0129); screenshots of each. A second boot
/// from the same disk reads the settings the first saved.
pub fn shell_boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    if cfg!(not(unix)) {
        eprintln!("==> skipped: the desktop shell boot test needs a Unix host (QMP)");
        return Ok(());
    }
    let image = image()?;
    let state = root().join("target").join("poly-shell-state.img");
    std::fs::write(&state, crate::ext2disk::image()?).map_err(|e| format!("{}: {e}", state.display()))?;
    let sound = crate::sound::qemu_has_virtio_sound();
    eprintln!(
        "==> boot test: the desktop shell (window management, notifications on the session bus, settings{})",
        if sound { ", the mixer on virtio-sound" } else { "; no virtio-sound in this QEMU" }
    );
    let (b, r) = shell_boot(kernel, syms, &image, &state, sound, drive_shell)?;
    for l in
        b.output.lines().filter(|l| l.contains("poly-compositor:") && (l.contains("cut off") || l.contains(": No ")))
    {
        eprintln!("    {l}");
    }
    match r {
        Some(Ok(())) => {}
        Some(Err(e)) => return Err(e),
        None if !b.output.contains(READY) => {
            return Err(format!("the desktop never came up (QEMU status {:?}, timed out: {})", b.status, b.timed_out));
        }
        None => return Err("the desktop shell boot test did not finish".into()),
    }
    eprintln!("==> boot test: the settings saved in the first boot are read in the second");
    let (b, r) = shell_boot(kernel, syms, &image, &state, sound, just_up)?;
    if let Some(Err(e)) = r {
        return Err(e);
    }
    expect("the second boot read the settings", b.output.contains("settings read from /mnt/poly/shell.conf"))?;
    expect("with the layout chosen in the first", b.output.contains("keymap de-ch"))?;
    Ok(())
}

/// The boot test.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    if cfg!(not(unix)) {
        eprintln!("==> skipped: the desktop boot test needs a Unix host (QMP)");
        return Ok(());
    }
    let image = image()?;
    eprintln!(
        "==> boot test: the desktop on virtio-gpu through DRM/KMS (ridge, launcher, typing, a client launched, \
         pointer and keyboard routed, a window dragged)"
    );
    let socket = std::env::temp_dir().join(format!("poly-desktop-qmp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let mut qemu_args = vec![
        "-initrd".to_string(),
        image.display().to_string(),
        "-qmp".to_string(),
        format!("unix:{},server=on,wait=off", socket.display()),
    ];
    qemu_args.extend(devices());
    let lines: Lines = Arc::default();
    let result: Arc<Mutex<Option<std::result::Result<(), String>>>> = Arc::default();
    let (seen, outcome, path) = (Arc::clone(&lines), Arc::clone(&result), socket.clone());
    let mut started = false;
    let on_line = move |line: &str| {
        seen.lock().unwrap_or_else(|e| e.into_inner()).push(line.to_string());
        if !started && line.contains(READY) {
            started = true;
            let (lines, outcome, path) = (Arc::clone(&seen), Arc::clone(&outcome), path.clone());
            thread::spawn(move || {
                let r = drive(&path, lines);
                *outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            });
        }
    };
    let b: Result<Boot> = super::with_boot_timeout(Duration::from_secs(420), || {
        boot_with(kernel, syms, MEMORY, &append(""), super::Devices::None, &qemu_args, None, on_line)
    });
    let _ = std::fs::remove_file(&socket);
    let b = b?;
    for l in b.output.lines().filter(|l| l.contains("poly-compositor:") && l.contains("cut off")) {
        eprintln!("    {l}");
    }
    match result.lock().unwrap_or_else(|e| e.into_inner()).take() {
        Some(Ok(())) => Ok(()),
        Some(Err(e)) => Err(e),
        None if !b.output.contains(READY) => {
            Err(format!("the desktop never came up (QEMU status {:?}, timed out: {})", b.status, b.timed_out))
        }
        None => Err("the desktop boot test did not finish".into()),
    }
}

/// The Windows GUI program of `desktop-win32` (`tests/windows/<name>.rs`).
const WIN32_TEST: &str = "win_gui_fill";
/// Where `pipeline win build` puts it.
const WIN32_BIN: &str = "target/pipeline/windows/desktop-bin";
/// Its desktop file.
const WIN32_DESKTOP_FILE: &str = "[Desktop Entry]\nType=Application\nName=Poly Fill\n\
    Comment=A Windows GUI program that fills its window\nExec=/win/win_gui_fill.exe\nKeywords=fill;windows;\n";
/// The colour it fills its client area with (`FILL`, RGB).
const WIN32_FILL: [u8; 3] = [0x8a, 0x47, 0xc0];
/// Its client rectangle: the window at (40, 400), 400 x 300, less
/// Windows' non-client frame of an overlapped window (8, 31, 8, 8).
const WIN32_CLIENT: (i32, i32, i32, i32) = (48, 431, 384, 261);

/// Every step of `desktop-win32` (see the module documentation).
fn drive_win32(socket: &Path, lines: Lines) -> std::result::Result<(), String> {
    let shots = shots_dir();
    std::fs::create_dir_all(&shots).map_err(|e| format!("{}: {e}", shots.display()))?;
    let qmp = screen::Qmp::connect(socket, Duration::from_secs(20))?;
    let mut d = Driver { qmp, lines, shots, step: 200 };
    thread::sleep(Duration::from_millis(500));
    d.wait("poly-compositor: desktop link to Windows apps on /dev/win32k", 1)?;
    let before = d.shot("win32-before")?;

    // Typed into the launcher, the program starts and shows its window.
    d.type_text(&["f", "i", "l", "l"])?;
    d.wait("launcher query \"fill\"", 30)?;
    thread::sleep(Duration::from_millis(800));
    d.type_text(&["ret"])?;
    d.wait("poly-compositor: launched Poly Fill", 30)?;
    let (x, y, w, h) = shown(&d, "Poly Fill", 180)?;
    expect(
        &format!("the window is at its client rectangle {WIN32_CLIENT:?} (got {:?})", (x, y, w, h)),
        (x, y, w, h) == WIN32_CLIENT,
    )?;
    // The fill comes with the first commit; give the frame time.
    thread::sleep(Duration::from_millis(3000));
    let img = d.shot("win32-window")?;
    let at = |dx: i32, dy: i32| img.pixel((x + dx) as u32, (y + dy) as u32);
    for (dx, dy) in [(1, 1), (w / 2, h / 2), (w - 2, h - 2), (1, h - 2), (w - 2, 1)] {
        expect(&format!("the client area is filled at +{dx},+{dy} (got {:?})", at(dx, dy)), at(dx, dy) == WIN32_FILL)?;
    }
    expect("nothing filled outside the client area", at(w + 6, h / 2) != WIN32_FILL && at(w / 2, h + 6) != WIN32_FILL)?;
    // Poly's frame: the title bar above the client area is drawn (not the
    // wallpaper of before, not the fill).
    let title = ((x + w / 2) as u32, (y - 12) as u32);
    expect(
        "Poly's title bar is above the client area",
        img.pixel(title.0, title.1) != before.pixel(title.0, title.1) && img.pixel(title.0, title.1) != WIN32_FILL,
    )?;
    d.qmp.quit();
    Ok(())
}

/// Boot group `desktop-win32`: a Windows GUI program's window on Poly's
/// desktop through win32k's desktop link (ADR 0131).
pub fn win32_boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    if cfg!(not(unix)) {
        eprintln!("==> skipped: the desktop-win32 boot test needs a Unix host (QMP)");
        return Ok(());
    }
    match crate::sevenzip::pipeline(&["win", "build", "--tests", WIN32_TEST, "--out", WIN32_BIN]) {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("==> skipped: the desktop-win32 boot test: building {WIN32_TEST}.exe failed ({s})");
            return Ok(());
        }
        Err(e) => {
            eprintln!("==> skipped: the desktop-win32 boot test: {e}");
            return Ok(());
        }
    }
    let exe_path = root().join(WIN32_BIN).join(format!("{WIN32_TEST}.exe"));
    let exe = std::fs::read(&exe_path).map_err(|e| format!("{}: {e}", exe_path.display()))?;
    let extra = vec![
        (format!("win/{WIN32_TEST}.exe"), 0o755, exe),
        ("usr/share/applications/poly-fill.desktop".to_string(), 0o644, WIN32_DESKTOP_FILE.as_bytes().to_vec()),
    ];
    let image = image_with("poly-desktop-win32-initramfs.cpio", extra)?;
    eprintln!(
        "==> boot test: a Windows GUI program's window on the desktop through /dev/win32k (launched, framed, filled)"
    );
    let socket = std::env::temp_dir().join(format!("poly-desktop-win32-qmp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let mut qemu_args = vec![
        "-initrd".to_string(),
        image.display().to_string(),
        "-qmp".to_string(),
        format!("unix:{},server=on,wait=off", socket.display()),
    ];
    qemu_args.extend(devices());
    let lines: Lines = Arc::default();
    let result: Arc<Mutex<Option<std::result::Result<(), String>>>> = Arc::default();
    let (seen, outcome, path) = (Arc::clone(&lines), Arc::clone(&result), socket.clone());
    let mut started = false;
    let on_line = move |line: &str| {
        seen.lock().unwrap_or_else(|e| e.into_inner()).push(line.to_string());
        if !started && line.contains(READY) {
            started = true;
            let (lines, outcome, path) = (Arc::clone(&seen), Arc::clone(&outcome), path.clone());
            thread::spawn(move || {
                let r = drive_win32(&path, lines);
                *outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            });
        }
    };
    let b: Result<Boot> = super::with_boot_timeout(Duration::from_secs(420), || {
        boot_with(kernel, syms, MEMORY, &append(""), super::Devices::None, &qemu_args, None, on_line)
    });
    let _ = std::fs::remove_file(&socket);
    let b = b?;
    match result.lock().unwrap_or_else(|e| e.into_inner()).take() {
        Some(Ok(())) => Ok(()),
        Some(Err(e)) => {
            for l in b.output.lines().filter(|l| l.contains("win32k") || l.contains("nt:") || l.contains("Poly Fill")) {
                eprintln!("    {l}");
            }
            Err(e)
        }
        None if !b.output.contains(READY) => {
            Err(format!("the desktop never came up (QEMU status {:?}, timed out: {})", b.status, b.timed_out))
        }
        None => Err("the desktop-win32 boot test did not finish".into()),
    }
}

/// The qcode (and whether Shift is held) that types `c` on the US layout.
fn qcode(c: char) -> Option<(String, bool)> {
    let plain = |s: &str| Some((s.to_string(), false));
    let shifted = |s: &str| Some((s.to_string(), true));
    match c {
        'a'..='z' | '0'..='9' => plain(&c.to_string()),
        'A'..='Z' => shifted(&c.to_ascii_lowercase().to_string()),
        ' ' => plain("spc"),
        '\n' => plain("ret"),
        '/' => plain("slash"),
        '.' => plain("dot"),
        '-' => plain("minus"),
        ';' => plain("semicolon"),
        '\'' => plain("apostrophe"),
        '\\' => plain("backslash"),
        ']' => plain("bracket_right"),
        '[' => plain("bracket_left"),
        '=' => plain("equal"),
        '>' => shifted("dot"),
        '<' => shifted("comma"),
        '|' => shifted("backslash"),
        '_' => shifted("minus"),
        '$' => shifted("4"),
        _ => None,
    }
}

/// Type `s` (US layout) as key presses.
fn type_str(d: &mut Driver, s: &str) -> std::result::Result<(), String> {
    for c in s.chars() {
        let (q, shift) = qcode(c).ok_or_else(|| format!("no key for {c:?}"))?;
        if shift {
            chord(d, &["shift", &q])?;
        } else {
            d.type_text(&[&q])?;
        }
    }
    Ok(())
}

/// Boot group `desktop-apps` (ADR 0134): Poly's own Terminal and Files.
/// Terminal from the launcher runs BusyBox's shell on a pseudo-terminal;
/// `echo hi > /tmp/x` is typed, and the file's contents come back through
/// the terminal. Files from the launcher goes to `/tmp` (Ctrl+L) and lists
/// `x`. Ctrl+Alt+T opens another Terminal, where `echo` and `ls` are typed
/// and their output is captured; Super+E opens Files again. Screenshots of
/// each step.
pub fn apps_boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    if cfg!(not(unix)) {
        eprintln!("==> skipped: the desktop apps boot test needs a Unix host (QMP)");
        return Ok(());
    }
    if let Err(why) = crate::busybox::obtain()? {
        eprintln!("==> skipped: the desktop apps boot test needs BusyBox for the Terminal's shell: {why}");
        return Ok(());
    }
    let image = image()?;
    eprintln!("==> boot test: Poly's Files and Terminal (launched, a shell command typed, its file listed)");
    let socket = std::env::temp_dir().join(format!("poly-apps-qmp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let mut qemu_args = vec![
        "-initrd".to_string(),
        image.display().to_string(),
        "-qmp".to_string(),
        format!("unix:{},server=on,wait=off", socket.display()),
    ];
    qemu_args.extend(devices());
    let lines: Lines = Arc::default();
    let result: Arc<Mutex<Option<std::result::Result<(), String>>>> = Arc::default();
    let (seen, outcome, path) = (Arc::clone(&lines), Arc::clone(&result), socket.clone());
    let mut started = false;
    let on_line = move |line: &str| {
        seen.lock().unwrap_or_else(|e| e.into_inner()).push(line.to_string());
        if !started && line.contains(READY) {
            started = true;
            let (lines, outcome, path) = (Arc::clone(&seen), Arc::clone(&outcome), path.clone());
            thread::spawn(move || {
                let r = drive_apps(&path, lines);
                *outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            });
        }
    };
    let b: Result<Boot> = super::with_boot_timeout(Duration::from_secs(600), || {
        boot_with(kernel, syms, MEMORY, &append(""), super::Devices::None, &qemu_args, None, on_line)
    });
    let _ = std::fs::remove_file(&socket);
    let b = b?;
    for l in b.output.lines().filter(|l| l.contains("poly-files:") || l.contains("poly-terminal:")) {
        eprintln!("    {l}");
    }
    let r = result.lock().unwrap_or_else(|e| e.into_inner()).take();
    match r {
        Some(Ok(())) => {
            eprintln!("    ok: Terminal ran a command in BusyBox's shell; Files listed its file");
            Ok(())
        }
        Some(Err(e)) => Err(e),
        None if !b.output.contains(READY) => {
            Err(format!("the desktop never came up (QEMU status {:?}, timed out: {})", b.status, b.timed_out))
        }
        None => Err("the desktop apps boot test did not finish".into()),
    }
}

/// The steps of `desktop-apps`.
fn drive_apps(socket: &Path, lines: Lines) -> std::result::Result<(), String> {
    let shots = shots_dir();
    std::fs::create_dir_all(&shots).map_err(|e| format!("{}: {e}", shots.display()))?;
    let qmp = screen::Qmp::connect(socket, Duration::from_secs(20))?;
    let mut d = Driver { qmp, lines, shots, step: 100 };
    thread::sleep(Duration::from_millis(500));

    // 1. Terminal from the launcher.
    type_str(&mut d, "terminal")?;
    d.wait("launcher query \"terminal\"", 30)?;
    thread::sleep(Duration::from_millis(500));
    d.type_text(&["ret"])?;
    d.wait("poly-compositor: launched Terminal", 30)?;
    let (tx, ty, tw, th) = shown(&d, "Terminal", 90)?;
    let shell = d.wait("poly-terminal: ", 30)?;
    expect("the terminal started a shell on a pseudo-terminal", shell.contains(" on /dev/pts/"))?;
    // The prompt.
    thread::sleep(Duration::from_secs(4));
    let img = d.shot("terminal")?;
    // The header on the ground, the screen on the page (dark theme).
    let page = img.pixel((tx + tw / 2) as u32, (ty + th - 6) as u32);
    expect("the terminal's page is Alpine's surface 1", page == [0x21, 0x21, 0x20])?;
    expect(
        "the terminal's header is the ground",
        img.pixel((tx + 200) as u32, (ty + 20) as u32) == [0x1a, 0x1a, 0x19],
    )?;

    // 2. A command, and its output read back through the terminal.
    type_str(&mut d, "echo hi > /tmp/x\n")?;
    thread::sleep(Duration::from_secs(2));
    type_str(&mut d, "printf '\\033]0;'; head -c 2 /tmp/x; printf '\\007'\n")?;
    let title = d.wait("poly-terminal: title ", 60)?;
    expect(
        &format!("the shell wrote hi into /tmp/x and the terminal read it ({title:?})"),
        title.ends_with("title hi"),
    )?;
    type_str(&mut d, "ls -l /tmp\n")?;
    thread::sleep(Duration::from_secs(3));
    d.shot("terminal-command")?;

    // 3. Files from the launcher, at /tmp.
    chord(&mut d, &["ctrl", "spc"])?;
    d.wait(LAUNCHER_SHOWN, 30)?;
    type_str(&mut d, "files")?;
    d.wait("launcher query \"files\"", 30)?;
    thread::sleep(Duration::from_millis(500));
    d.type_text(&["ret"])?;
    d.wait("poly-compositor: launched Files", 30)?;
    let (fx, fy, fw, fh) = shown(&d, "Files", 90)?;
    thread::sleep(Duration::from_secs(3));
    d.shot("files-home")?;
    // A click into its page gives it the keyboard.
    click(&mut d, fx + fw / 2, fy + fh / 2)?;
    thread::sleep(Duration::from_secs(2));
    chord(&mut d, &["ctrl", "l"])?;
    thread::sleep(Duration::from_secs(3));
    d.shot("files-path")?;
    type_str(&mut d, "/tmp")?;
    thread::sleep(Duration::from_secs(2));
    d.shot("files-path-typed")?;
    d.type_text(&["ret"])?;
    let listing = d.wait("poly-files: /tmp (", 60)?;
    expect(
        &format!("Files lists the terminal's file /tmp/x ({listing:?})"),
        listing.split(": ").nth(2).is_some_and(|names| names.split(' ').any(|n| n == "x")),
    )?;
    thread::sleep(Duration::from_secs(3));
    let img = d.shot("files-tmp")?;
    expect(
        "the Files sidebar is the ground",
        img.pixel((fx + 100) as u32, (fy + fh - 20) as u32) == [0x1a, 0x1a, 0x19],
    )?;
    expect(
        "the Files page is surface 1",
        img.pixel((fx + fw - 40) as u32, (fy + fh / 2) as u32) == [0x21, 0x21, 0x20],
    )?;
    // Grid view (Ctrl+2), for the picture.
    chord(&mut d, &["ctrl", "2"])?;
    thread::sleep(Duration::from_secs(2));
    d.shot("files-grid")?;

    // 4. Ctrl+Alt+T opens a new terminal; type commands and check their output.
    let terminals = count(&d.lines, "poly-compositor: launched Terminal");
    let terminal_starts = count(&d.lines, " on /dev/pts/");
    chord(&mut d, &["ctrl", "alt", "t"])?;
    wait_nth(&d.lines, "poly-compositor: launched Terminal", terminals + 1, 30)?;
    wait_nth(&d.lines, " on /dev/pts/", terminal_starts + 1, 30)?;
    thread::sleep(Duration::from_secs(4));
    type_str(&mut d, "echo shortcut; ls -l /tmp; printf '\\033]0;'; ls -1 /tmp | tr -d '\\n'; printf '\\007'\n")?;
    let title = d.wait("poly-terminal: title x", 60)?;
    expect("the shortcut terminal ran ls and received its output", title.ends_with("title x"))?;
    thread::sleep(Duration::from_secs(2));
    d.shot("terminal-shortcut-output")?;

    // Super+E opens Files again.
    let files = count(&d.lines, "poly-compositor: launched Files");
    let file_windows = count(&d.lines, "window \"Files\" shown at");
    chord(&mut d, &["meta_l", "e"])?;
    wait_nth(&d.lines, "poly-compositor: launched Files", files + 1, 30)?;
    wait_nth(&d.lines, "window \"Files\" shown at", file_windows + 1, 90)?;
    thread::sleep(Duration::from_secs(4));
    d.shot("shortcuts")?;
    d.qmp.quit();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_window_line() {
        let l = "    | poly-compositor: window \"Poly Demo\" shown at 460,302 size 360x240";
        assert_eq!(window_place(l), Some((460, 302, 360, 240)));
        assert_eq!(window_place("poly-compositor: window \"x\" shown at a,b size 1x1"), None);
    }

    #[test]
    fn tablet_positions_round_trip() {
        // The compositor maps v to v * 1279 / 0x7fff (rounded down).
        for x in [0, 1, 640, 1279] {
            let e = abs(x, 0);
            let v: i64 = e[0].split("\"value\": ").nth(1).unwrap().trim_end_matches("}}").parse().unwrap();
            assert_eq!(v * 1279 / 0x7fff, i64::from(x), "{x}");
        }
    }
}
