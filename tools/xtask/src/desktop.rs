//! Poly's desktop (ADR 0107): `cargo xtask desktop` boots straight into
//! the compositor and the shell on a virtio GPU, with a virtio keyboard
//! and tablet; boot group `desktop` does the same and drives it through
//! QMP.
//!
//! The image: `/bin/poly-compositor` (as init, `--desktop`), the C clients
//! `/bin/wl-demo` (`tests/linux/wayland/wl-demo.c`, the launcher's "Poly
//! Demo") and `/bin/wl-client`, a desktop file
//! `/usr/share/applications/demo-window.desktop` ("Demo Window", which
//! starts `wl-demo` too, so the launcher's desktop file path is used), and
//! DejaVu Sans from the font store when it is fetched (`cargo xtask fonts
//! fetch`; without it the shell draws with the bitmap font).
//!
//! The boot test, on QEMU's screen (`screendump` of the virtio GPU) and
//! its input (`input-send-event`, QEMU's QMP reference):
//!
//! 1. the desktop is up: the ridge (rgb(32, 32, 32) along the top), the
//!    launcher's panel in the middle, the wallpaper elsewhere;
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
//! Screenshots go to `$POLY_DESKTOP_SHOTS` (default `target/desktop/`) as
//! PNGs, one per step.

use super::{Boot, Result, Symbols, boot_with, compositor, has_desktop, initramfs, qemu, root, screen};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// The screen the desktop boots on.
pub const SIZE: (u32, u32) = (1280, 800);
/// What the compositor prints when the first frame is on the screen.
const READY: &str = "poly-compositor: desktop ready";
/// The demo client's colours (wl-demo.c) and the ridge's.
const DEMO_A: [u8; 3] = [0x2f, 0x7d, 0x6d];
const DEMO_B: [u8; 3] = [0xc0, 0x8a, 0x2e];
const RIDGE: [u8; 3] = [0x20, 0x20, 0x20];
/// The desktop file the image carries.
const DESKTOP_FILE: &str = "[Desktop Entry]\nType=Application\nName=Demo Window\n\
    Comment=The demo client, from a desktop file\nExec=/bin/wl-demo\nKeywords=demo;sample;\n";
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
    let (w, h) = SIZE;
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

/// Build the desktop's initramfs; returns its path.
pub fn image() -> Result<PathBuf> {
    let compositor = compositor::build_compositor()?;
    let demo = compositor::build_c("tests/linux/wayland/wl-demo.c", "wl-demo")?;
    let client = compositor::build_client()?;
    let mut tree = initramfs::Tree::default();
    tree.files.push(("usr/share/applications/demo-window.desktop".into(), 0o644, DESKTOP_FILE.as_bytes().into()));
    let font = crate::fonts::cache_dir().join("DejaVuSans.ttf");
    match std::fs::read(&font) {
        Ok(data) => tree.files.push(("usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into(), 0o644, data)),
        Err(_) => eprintln!("==> no DejaVu fonts (`cargo xtask fonts fetch`): the shell will use the bitmap font"),
    }
    let programs: [(&str, &[u8]); 3] = [("poly-compositor", &compositor), ("wl-demo", &demo), ("wl-client", &client)];
    initramfs::write(&root(), "poly-desktop-initramfs.cpio", &initramfs::programs_image_with(&programs, &tree))
}

/// `cargo xtask desktop [--release] [kernel args...]`: boot into the
/// desktop in a QEMU window (no window without a desktop: then a VNC
/// display on port 5900).
pub fn command(args: &[String]) -> Result {
    let release = args.iter().any(|a| a == "--release");
    let extra: Vec<&str> = args.iter().map(String::as_str).filter(|a| !a.starts_with("--")).collect();
    let kernel = super::build_kernel(release)?;
    let image = image()?;
    let mut cmd = qemu(&kernel, MEMORY, &append(&extra.join(" ")), super::Devices::None, "mon:stdio");
    cmd.arg("-initrd").arg(&image).args(devices());
    if has_desktop() {
        eprintln!("==> the desktop is in QEMU's window; the Windows key or Ctrl+Space toggles the launcher");
    } else {
        cmd.args(["-display", "vnc=0.0.0.0:0"]);
        eprintln!("==> no desktop here: the screen is on VNC, port 5900 (Ctrl-A X quits)");
    }
    let _ = cmd.status().map_err(|e| format!("qemu: {e}"))?;
    Ok(())
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
    expect("the wallpaper is below the ridge", img.pixel(640, 26) != RIDGE && img.pixel(100, 700) != RIDGE)?;
    // The launcher's search row: 720 wide, x 280..1000, y 296..344.
    expect(
        "the launcher's panel is in the middle",
        img.pixel(900, 320) == RIDGE && img.pixel(281 + 300, 340) == RIDGE,
    )?;
    expect("not outside the panel", img.pixel(200, 320) != RIDGE)?;

    // 2. Type, list, launch.
    d.type_text(&["d", "e", "m", "o"])?;
    d.wait("launcher query \"demo\"", 30)?;
    thread::sleep(Duration::from_millis(800));
    let img = d.shot("typed")?;
    // Rows: the first, selected, is lighter than the panel.
    let row = img.pixel(640, 344 + 1 + 28 + 10);
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
    expect("the launcher is gone", img.pixel(900, 320) != RIDGE)?;

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
    expect("the launcher is back on the empty desktop", img.pixel(900, 320) == RIDGE)?;
    d.qmp.quit();
    Ok(())
}

/// The boot test.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    if cfg!(not(unix)) {
        eprintln!("==> skipped: the desktop boot test needs a Unix host (QMP)");
        return Ok(());
    }
    if let Err(why) = compositor::build_c("tests/linux/wayland/wl-demo.c", "wl-demo") {
        eprintln!("==> skipped: the desktop boot test: {why}");
        return Ok(());
    }
    let image = image()?;
    eprintln!(
        "==> boot test: the desktop on virtio-gpu through /dev/fb0 (ridge, launcher, typing, a client launched, \
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
