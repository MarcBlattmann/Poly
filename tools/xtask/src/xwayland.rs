//! X11 apps on Poly's desktop through Xwayland (roadmap Phase 6, "XWayland
//! for X11 apps"; ADR 0118).
//!
//! **On the root disk** ([`add_to_root`], called by `rootdisk::collect`):
//! the host's unmodified `Xwayland` server, `xkbcomp` (the server compiles
//! its keymaps with it), X11 test programs ([`PROGRAMS`]) and the data they
//! read ([`DATA`]: the XKB database, X's locale tables and error database,
//! app-defaults, the core fonts, xterm's desktop file), with their shared
//! objects through the root disk's `DT_NEEDED` closure, as ADR 0059 brings
//! host programs. They are Ubuntu's packages (`xwayland`, `x11-apps`,
//! `x11-utils`, `xterm`, `x11-xkb-utils`, `xkb-data`, `libx11-data`,
//! `xfonts-base`) in the `poly-dev` image; a host without them gets a root
//! disk without X and the boot test skips. Nothing of it is committed.
//!
//! **The boot test** (group `xwayland`): the root disk with
//! `/usr/bin/poly-compositor` and desktop files for xeyes and xclock
//! added, booted through BusyBox's init into the disk's bash on the serial
//! console, with the desktop's virtio GPU, keyboard and tablet
//! ([`crate::desktop::devices`]) and `poly.terminal=off`. Typed into bash,
//! driven and checked through QMP (`input-send-event`, `screendump`):
//!
//! 1. `poly-compositor --desktop --xwayland-lazy` in the background: the
//!    desktop comes up and the X display `:0` listens (path and abstract
//!    sockets);
//! 2. `xdpyinfo`, the first X client, makes the compositor start Xwayland
//!    (which says it is ready on `:0`; the X window manager runs), exits 0
//!    and names the X.Org vendor, display `:0`, a screen of the desktop's
//!    size and the extensions (XKEYBOARD, RENDER, Composite, MIT-SHM);
//!    `xprop` finds the window manager on the root
//!    (`_NET_SUPPORTING_WM_CHECK`, named "Poly", `_NET_SUPPORTED`);
//! 3. typing `xeyes` into the launcher and Enter starts it from its
//!    desktop file; its window appears with Poly's frame (title bar and
//!    border in the active colours) and its eyes drawn; it is the active X
//!    window (`_NET_ACTIVE_WINDOW`, `_NET_CLIENT_LIST`);
//! 4. `xclock` from the shell: a second framed window that takes the
//!    focus (xeyes' frame turns inactive), its face drawn;
//! 5. `xev`: it gets the X input focus, a key pressed through QMP arrives
//!    as `KeyPress` with keycode 38 (`a`), the pointer as `MotionNotify`;
//! 6. dragging xev's title bar moves it, and `xwininfo` finds it at the
//!    new place in X too;
//! 7. the close buttons end xev, xclock and xeyes (`WM_DELETE_WINDOW`;
//!    xeyes exits 0), their frames are gone, `_NET_CLIENT_LIST` is empty;
//! 8. typing `xterm` into the launcher (which is back on the empty
//!    desktop) starts XTerm from its own desktop file; a command typed into
//!    it runs in its shell, and its close button ends it.
//!
//! Screenshots go to `$POLY_XWAYLAND_SHOTS` (default `target/xwayland/`)
//! as PNGs, one per step.

use crate::rootdisk::Collector;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The X server.
pub const XWAYLAND: &str = "/usr/bin/Xwayland";

/// X11 programs for the root disk: the keymap compiler the server runs,
/// and test clients. Each is optional but Xwayland and `xkbcomp`.
pub const PROGRAMS: &[&str] =
    &["xkbcomp", "xeyes", "xclock", "xdpyinfo", "xev", "xterm", "xprop", "xwininfo", "xlogo", "xmessage"];

/// Data directories and files X programs read, each copied if the host has
/// it: the XKB database (`xkbcomp`), X's locale tables (Xlib's `XOpenIM`,
/// `setlocale` in Xt), its error database, colour names, the app-defaults
/// (Xt programs' resources: xterm, xclock, xeyes), and the core fonts
/// (xterm's `fixed`, the cursor font) with their encodings.
pub const DATA: &[&str] = &[
    "/usr/share/X11/xkb",
    "/usr/share/X11/locale",
    "/usr/share/X11/XErrorDB",
    "/usr/share/X11/rgb.txt",
    "/etc/X11/app-defaults",
    "/usr/share/fonts/X11/misc",
    "/usr/share/fonts/X11/encodings",
    "/usr/lib/xorg/protocol.txt",
    // xterm's desktop file: the launcher lists XTerm.
    "/usr/share/applications/debian-xterm.desktop",
    "/usr/share/terminfo/x",
    "/lib/terminfo/x",
];

/// Add Xwayland, [`PROGRAMS`] and [`DATA`] to the root disk being
/// collected; `have` gets `"xwayland"` when the server and `xkbcomp` are
/// there.
pub fn add_to_root(
    c: &mut Collector,
    have: &mut BTreeSet<&'static str>,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    if !Path::new(XWAYLAND).is_file() {
        notes.push(format!("xwayland: no {XWAYLAND} on this host (Ubuntu: apt install xwayland x11-apps x11-utils)"));
        return Ok(());
    }
    c.add(Path::new(XWAYLAND))?;
    let mut missing = Vec::new();
    for name in PROGRAMS {
        match ["/usr/bin", "/bin"].iter().map(|d| Path::new(d).join(name)).find(|p| p.exists()) {
            Some(p) => c.add(&p)?,
            None => missing.push(*name),
        }
    }
    for path in DATA {
        let p = Path::new(path);
        if p.is_dir() {
            c.add_tree(p, &[])?;
        } else if p.is_file() {
            c.add(p)?;
        }
    }
    // Where the server writes the keymaps xkbcomp compiles (Debian's
    // XKM output directory).
    c.tree.dir("var/lib/xkb", 0o755);
    if missing.contains(&"xkbcomp") || !Path::new("/usr/share/X11/xkb/rules").is_dir() {
        notes.push("xwayland: no xkbcomp or XKB data (x11-xkb-utils, xkb-data): Xwayland would not start".into());
    } else {
        have.insert("xwayland");
    }
    if !missing.is_empty() {
        notes.push(format!("xwayland: left out (not on this host): {}", missing.join(", ")));
    }
    Ok(())
}

// ---- the boot test -------------------------------------------------------

/// The image the boot test makes afresh.
const IMAGE: &str = "poly-root-disk-xwayland.img";
/// Desktop files for X apps that ship none, so the launcher lists them.
const DESKTOP_FILES: [(&str, &str); 2] = [
    (
        "xeyes.desktop",
        "[Desktop Entry]\nType=Application\nName=XEyes\nComment=Eyes that follow the pointer (X11)\nExec=xeyes\n",
    ),
    (
        "xclock.desktop",
        "[Desktop Entry]\nType=Application\nName=XClock\nComment=An analog clock (X11)\nExec=xclock -update 1\n",
    ),
];
/// The desktop's screen.
const SIZE: (u32, u32) = crate::desktop::SIZE;
/// Poly's frame colours (`poly_wayland::compose::Theme::NEUTRAL`).
const TITLE_ACTIVE: [u8; 3] = [0x3c, 0x3c, 0x3c];
const TITLE_INACTIVE: [u8; 3] = [0x2a, 0x2a, 0x2a];
const BORDER_ACTIVE: [u8; 3] = [0x5a, 0x5a, 0x5a];
const BORDER_INACTIVE: [u8; 3] = [0x38, 0x38, 0x38];
/// How long one step may take (Xwayland's start under TCG is slow).
const STEP_TIMEOUT: Duration = Duration::from_secs(400);
/// How long the whole session may take.
const SESSION_TIMEOUT: Duration = Duration::from_secs(1800);

/// Where the screenshots go.
fn shots_dir() -> PathBuf {
    std::env::var_os("POLY_XWAYLAND_SHOTS").map_or_else(|| crate::root().join("target").join("xwayland"), PathBuf::from)
}

/// The root disk with the compositor and the desktop files added.
fn image() -> crate::Result<Option<PathBuf>> {
    let mut contents = crate::rootdisk::collect()?;
    for note in &contents.notes {
        eprintln!("==> note: root disk: {note}");
    }
    if !contents.have.contains("xwayland") || !contents.have.contains("bash") {
        return Ok(None);
    }
    let compositor = crate::compositor::build_compositor()?;
    contents.tree.file("usr/bin/poly-compositor", 0o755, &compositor);
    for (file, text) in DESKTOP_FILES {
        contents.tree.file(&format!("usr/share/applications/{file}"), 0o644, text.as_bytes());
    }
    let path = crate::root().join("target").join(IMAGE);
    let how = crate::rootdisk::write_image(&contents.tree, &path)?;
    eprintln!("==> root disk {} ({how})", path.display());
    Ok(Some(path))
}

/// The kernel's serial input ring holds 256 bytes (`kernel/src/shell/serial.rs`)
/// and drops what does not fit: on KVM a whole typed line arrives before the
/// shell reads any of it, so every line the test types stays shorter (a unit
/// test checks it).
#[cfg(test)]
const SERIAL_RING: usize = 256;

/// A shell function for [`wait_for`], typed once: `w FILE TEXT N` waits up
/// to N half seconds until FILE contains TEXT, then prints the matching
/// lines, or `TIMEOUT TEXT` and the file's end.
const WAIT_FN: &str = "w() { for i in $(seq 1 $3); do grep -q -- \"$2\" $1 && break; sleep 0.5; done; \
                       grep -- \"$2\" $1 || { echo \"TIMEOUT $2\"; tail -n 30 $1; }; }";

/// Wait in the guest (up to `secs`) until `file` contains `text`, then
/// print the matching lines: a shell line for [`crate::busybox::Session::run`]
/// (after [`WAIT_FN`]).
fn wait_for(file: &str, text: &str, secs: u32) -> String {
    format!("w {file} '{text}' {n}", n = secs * 2)
}

/// The boot test (see the module documentation).
pub fn boot_test(kernel: &Path, syms: &Arc<crate::symbols::Symbols>) -> crate::Result {
    if cfg!(not(unix)) {
        eprintln!("==> skipped: the XWayland boot test needs a Unix host (QMP)");
        return Ok(());
    }
    let (initrd, _origin) = match crate::rootdisk::write_initramfs()? {
        Ok(found) => found,
        Err(why) => {
            eprintln!("==> skipped: the XWayland boot test boots through BusyBox's init: {why}");
            return Ok(());
        }
    };
    let Some(image) = image()? else {
        eprintln!("==> skipped: the XWayland boot test: no Xwayland (or bash) on this host");
        return Ok(());
    };
    eprintln!("==> boot test: X11 apps through Xwayland on Poly's desktop (xdpyinfo, xeyes, xclock, xev)");
    let shots = shots_dir();
    std::fs::create_dir_all(&shots).map_err(|e| format!("{}: {e}", shots.display()))?;
    let socket = std::env::temp_dir().join(format!("poly-xwayland-qmp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let append = format!(
        "{} poly.terminal=off {}",
        crate::rootdisk::boot_args(),
        std::env::var("POLY_XWAYLAND_APPEND").unwrap_or_default()
    );
    let mut cmd = crate::qemu(kernel, crate::rootdisk::MEMORY, append.trim_end(), crate::Devices::None, "stdio");
    cmd.args(["-display", "none"])
        .args(crate::rootdisk::drive_args(&image))
        .arg("-initrd")
        .arg(&initrd)
        .arg("-qmp")
        .arg(format!("unix:{},server=on,wait=off", socket.display()))
        .args(crate::desktop::devices());
    let mut s = crate::busybox::Session::spawn(cmd, syms)?;
    s.set_timeouts(STEP_TIMEOUT, SESSION_TIMEOUT);
    let result = session(&mut s, &socket, &shots);
    let _ = std::fs::remove_file(&socket);
    if result.is_err() {
        // The compositor's log, for the record (if the shell still answers).
        s.set_timeouts(Duration::from_secs(30), SESSION_TIMEOUT + Duration::from_secs(60));
        let _ = s.run("grep -v Warning /tmp/comp.log | tail -n 80");
        let log = crate::root().join("target/poly-xwayland-boot.log");
        let _ = std::fs::write(&log, s.text());
        eprintln!("==> the session's output is in {}", log.display());
    }
    result
}

// QMP input (QEMU's `input-send-event`).

fn key(qcode: &str, down: bool) -> String {
    format!(r#"{{"type": "key", "data": {{"down": {down}, "key": {{"type": "qcode", "data": "{qcode}"}}}}}}"#)
}

fn button(down: bool) -> String {
    format!(r#"{{"type": "btn", "data": {{"down": {down}, "button": "left"}}}}"#)
}

/// A tablet position for logical pixel (`x`, `y`) (the compositor scales
/// 0..0x7fff onto the screen).
fn abs(x: i32, y: i32) -> Vec<String> {
    let (w, h) = (i64::from(SIZE.0) - 1, i64::from(SIZE.1) - 1);
    let ax = (i64::from(x) * 0x7fff + w - 1) / w;
    let ay = (i64::from(y) * 0x7fff + h - 1) / h;
    vec![
        format!(r#"{{"type": "abs", "data": {{"axis": "x", "value": {ax}}}}}"#),
        format!(r#"{{"type": "abs", "data": {{"axis": "y", "value": {ay}}}}}"#),
    ]
}

/// The screen and the input devices through QMP.
struct Screen {
    qmp: crate::screen::Qmp,
    shots: PathBuf,
    step: u32,
}

impl Screen {
    fn send(&mut self, events: &[String]) -> crate::Result {
        self.qmp.execute("input-send-event", &format!(r#"{{"events": [{}]}}"#, events.join(", ")))?;
        std::thread::sleep(Duration::from_millis(150));
        Ok(())
    }

    fn type_keys(&mut self, keys: &[&str]) -> crate::Result {
        for k in keys {
            self.send(&[key(k, true)])?;
            self.send(&[key(k, false)])?;
        }
        Ok(())
    }

    fn click(&mut self, x: i32, y: i32) -> crate::Result {
        self.send(&abs(x, y))?;
        self.send(&[button(true)])?;
        self.send(&[button(false)])
    }

    /// A screenshot named for the step, read back.
    fn shot(&mut self, what: &str) -> crate::Result<crate::screen::Image> {
        self.step += 1;
        let png = self.shots.join(format!("xwayland-{}-{what}.png", self.step));
        let ppm = self.shots.join(format!("xwayland-{}-{what}.ppm", self.step));
        self.qmp.screendump(&png)?;
        self.qmp.screendump(&ppm)?;
        let img = crate::screen::Image::read_ppm(&ppm)?;
        let _ = std::fs::remove_file(&ppm);
        eprintln!("==> screenshot {}", png.display());
        Ok(img)
    }
}

/// Wait for `text` in the compositor's log; the matching lines.
fn log_wait(s: &mut crate::busybox::Session, text: &str, secs: u32) -> crate::Result<String> {
    let out = s.run(&wait_for("/tmp/comp.log", text, secs))?;
    if out.contains(&format!("TIMEOUT {text}")) {
        // What X has, for the log.
        let _ = s.run("DISPLAY=:0 xwininfo -root -tree | head -30; grep 'xwm:' /tmp/comp.log | tail -40");
        return Err(format!("XWayland on Poly: no {text:?} from the compositor within {secs} s: {out}"));
    }
    Ok(out)
}

/// The window's content position and size from the compositor's line
/// `window "xeyes" shown at X,Y size WxH`.
fn window_place(out: &str) -> Option<(i32, i32, i32, i32)> {
    let line = out.lines().find(|l| l.contains(" shown at "))?;
    let rest = line.split(" shown at ").nth(1)?;
    let (pos, size) = rest.split_once(" size ")?;
    let (x, y) = pos.split_once(',')?;
    let (w, h) = size.trim().split_once('x')?;
    Some((x.parse().ok()?, y.parse().ok()?, w.parse().ok()?, h.parse().ok()?))
}

fn expect(what: &str, ok: bool) -> crate::Result {
    if ok { Ok(()) } else { Err(format!("XWayland on Poly: {what}")) }
}

/// Poly's frame around content at (x, y): the title bar above it and the
/// border at its left, in the active or inactive colours (looked at on
/// the left, which a later window placed down and right leaves visible).
fn framed(img: &crate::screen::Image, (x, y, _, _): (i32, i32, i32, i32), active: bool) -> bool {
    let (title, border) = if active { (TITLE_ACTIVE, BORDER_ACTIVE) } else { (TITLE_INACTIVE, BORDER_INACTIVE) };
    let px = |x: i32, y: i32| img.pixel(x as u32, y as u32);
    px(x + 2, y - 2) == title && px(x + 2, y - 22) == title && px(x - 1, y + 5) == border
}

/// How many pixels of the rectangle are `rgb`.
fn count(img: &crate::screen::Image, (x, y, w, h): (i32, i32, i32, i32), rgb: [u8; 3]) -> usize {
    let mut n = 0;
    for yy in y.max(0)..(y + h).min(SIZE.1 as i32) {
        for xx in x.max(0)..(x + w).min(SIZE.0 as i32) {
            if img.pixel(xx as u32, yy as u32) == rgb {
                n += 1;
            }
        }
    }
    n
}

/// Start X program `line` from the shell in the background, its output in
/// `/tmp/<log>.log`; its pid.
fn start_x(s: &mut crate::busybox::Session, line: &str, log: &str) -> crate::Result<u32> {
    let out = s.run(&format!("DISPLAY=:0 {line} > /tmp/{log}.log 2>&1 & echo pid=$!"))?;
    out.lines()
        .find_map(|l| l.trim().strip_prefix("pid=")?.parse().ok())
        .ok_or_else(|| format!("XWayland on Poly: no pid for {line} (printed {out:?})"))
}

/// Whether process `pid` ends within a minute.
fn gone(s: &mut crate::busybox::Session, pid: u32) -> crate::Result<bool> {
    let out = s.run(&format!(
        "for i in $(seq 1 120); do kill -0 {pid} 2>/dev/null || break; sleep 0.5; done; \
         kill -0 {pid} 2>/dev/null && echo alive || echo gone"
    ))?;
    Ok(out.contains("gone"))
}

fn session(s: &mut crate::busybox::Session, socket: &Path, shots: &Path) -> crate::Result {
    s.expect("rootdisk: /dev/vda mounted on /mnt/root")?;
    s.prompt()?;
    let qmp = crate::screen::Qmp::connect(socket, Duration::from_secs(20))?;
    let mut d = Screen { qmp, shots: shots.to_path_buf(), step: 0 };
    s.run("export XDG_RUNTIME_DIR=/run/user/0; mkdir -p -m 700 $XDG_RUNTIME_DIR")?;
    s.run(WAIT_FN)?;
    s.run("POLY_XWM_DEBUG=1 poly-compositor --desktop --xwayland-lazy > /tmp/comp.log 2>&1 &")?;
    log_wait(s, "desktop ready", 120)?;
    log_wait(s, "X display :0 on /tmp/.X11-unix/X0 and 2 socket(s)", 30)?;
    let first = d.shot("desktop")?;
    expect("the screen is the desktop's size", (first.width, first.height) == SIZE)?;

    // 2. xdpyinfo, the first X client: Xwayland starts for it, then it
    // reports the server, its screen and extensions.
    let out = s.run("DISPLAY=:0 xdpyinfo > /tmp/xdpyinfo.txt 2>&1; echo status=$?")?;
    expect(&format!("xdpyinfo exits 0 (printed {out:?})"), out.contains("status=0"))?;
    log_wait(s, "an X client connected to :0", 30)?;
    log_wait(s, "Xwayland ready on :0", 60)?;
    log_wait(s, "X window manager ready", 60)?;
    let info = s.run(
        "grep -E '^(name of display|vendor string|X.Org version):|dimensions:|depth of root|^    (XKEYBOARD|RENDER|MIT-SHM|Composite|RANDR|XFIXES)$' /tmp/xdpyinfo.txt",
    )?;
    eprintln!("==> xdpyinfo on Poly:\n{info}");
    for want in [
        "name of display:    :0",
        "vendor string:    The X.Org Foundation",
        &format!("dimensions:    {}x{} pixels", SIZE.0, SIZE.1),
        "XKEYBOARD",
        "RENDER",
        "Composite",
        "MIT-SHM",
    ] {
        expect(&format!("xdpyinfo reports {want:?} (got {info:?})"), info.contains(want))?;
    }
    // The window manager as X clients see it (EWMH).
    let root = s.run("DISPLAY=:0 xprop -root _NET_SUPPORTING_WM_CHECK _NET_SUPPORTED")?;
    expect(
        &format!("the root names the window manager (xprop: {root:?})"),
        root.contains("_NET_SUPPORTING_WM_CHECK(WINDOW): window id # 0x"),
    )?;
    expect("_NET_SUPPORTED lists _NET_ACTIVE_WINDOW", root.contains("_NET_ACTIVE_WINDOW"))?;
    let check = root.split("window id # ").nth(1).and_then(|r| r.split_whitespace().next()).unwrap_or("0");
    let name = s.run(&format!("DISPLAY=:0 xprop -id {check} _NET_WM_NAME"))?;
    expect(&format!("the window manager is called Poly (xprop: {name:?})"), name.contains("\"Poly\""))?;

    // 3. xeyes from the launcher (shown on the empty desktop), by its
    // desktop file.
    d.type_keys(&["x", "e", "y", "e", "s"])?;
    log_wait(s, "launcher query \"xeyes\"", 30)?;
    d.type_keys(&["ret"])?;
    log_wait(s, "launched XEyes", 30)?;
    let out = log_wait(s, "window \"xeyes\" shown at", 120)?;
    let eyes = window_place(&out).ok_or_else(|| format!("unreadable window line {out:?}"))?;
    std::thread::sleep(Duration::from_millis(2500));
    let img = d.shot("xeyes")?;
    expect(&format!("xeyes at {eyes:?} has Poly's frame, active"), framed(&img, eyes, true))?;
    let white = count(&img, eyes, [0xff, 0xff, 0xff]);
    expect(&format!("xeyes drew its eyes ({white} white pixels)"), white > 200)?;
    let (ex, ey, ew, eh) = eyes;
    let xprop = s.run("DISPLAY=:0 xprop -root _NET_ACTIVE_WINDOW _NET_CLIENT_LIST")?;
    expect(
        &format!("xeyes is the active X window ({xprop:?})"),
        !xprop.contains("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x0\n")
            && xprop.contains("_NET_CLIENT_LIST(WINDOW): window id # 0x"),
    )?;
    // The eyes follow the pointer: pointer motion reaches Xwayland.
    d.send(&abs(ex + ew + 40, ey + eh / 2))?;

    // 4. xclock from the shell, a second framed window.
    let clock_pid = start_x(s, "xclock -update 1", "xclock")?;
    let out = log_wait(s, "window \"xclock\" shown at", 120)?;
    let clock = window_place(&out).ok_or_else(|| format!("unreadable window line {out:?}"))?;
    std::thread::sleep(Duration::from_millis(2500));
    let img = d.shot("xclock")?;
    expect(&format!("xclock at {clock:?} is framed and active"), framed(&img, clock, true))?;
    expect("xeyes lost the focus to xclock", framed(&img, eyes, false))?;
    let inner = (clock.0 + 4, clock.1 + 4, clock.2 - 8, clock.3 - 8);
    let paper = count(&img, inner, [0xff, 0xff, 0xff]);
    expect(&format!("xclock drew its face ({paper} white pixels)"), paper > (clock.2 * clock.3 / 4) as usize)?;

    // 5. xev: a key pressed through QMP reaches the X client.
    let xev_pid = start_x(s, "xev -geometry 360x240", "xev")?;
    let out = log_wait(s, "window \"Event Tester\" shown at", 120)?;
    let xev = window_place(&out).ok_or_else(|| format!("unreadable window line {out:?}"))?;
    let focus = s.run(&wait_for("/tmp/xev.log", "FocusIn event", 60))?;
    expect(&format!("xev got the X input focus ({focus:?})"), !focus.contains("TIMEOUT"))?;
    d.type_keys(&["a"])?;
    let pressed = s.run(&wait_for("/tmp/xev.log", "KeyPress event", 60))?;
    expect(&format!("xev saw the key press ({pressed:?})"), !pressed.contains("TIMEOUT"))?;
    let detail = s.run("grep -A2 'KeyPress event' /tmp/xev.log | head -3")?;
    eprintln!("==> xev on Poly:\n{detail}");
    expect(
        &format!("the key is 'a', keycode 38 ({detail:?})"),
        detail.contains("keycode 38") && detail.contains(", a)"),
    )?;
    d.send(&abs(xev.0 + 100, xev.1 + 100))?;
    let moved = s.run(&wait_for("/tmp/xev.log", "MotionNotify event", 60))?;
    expect(&format!("xev saw the pointer ({moved:?})"), !moved.contains("TIMEOUT"))?;
    std::thread::sleep(Duration::from_millis(1500));
    let img = d.shot("xev")?;
    expect(&format!("xev at {xev:?} is framed"), framed(&img, xev, true))?;

    // 6. Dragging xev's title bar moves it, and X knows where it is.
    let (dx, dy) = (-400, 120);
    d.send(&abs(xev.0 + 40, xev.1 - 12))?;
    d.send(&[button(true)])?;
    for i in 1..=4 {
        d.send(&abs(xev.0 + 40 + dx * i / 4, xev.1 - 12 + dy * i / 4))?;
    }
    d.send(&[button(false)])?;
    let to = (xev.0 + dx, xev.1 + dy, xev.2, xev.3);
    std::thread::sleep(Duration::from_millis(1500));
    let info = s.run("DISPLAY=:0 xwininfo -name 'Event Tester' | grep -E 'Absolute upper-left (X|Y)'")?;
    let want = format!("Absolute upper-left X:  {}\n  Absolute upper-left Y:  {}", to.0, to.1);
    expect(&format!("X has xev where it was dragged, {to:?} ({info:?})"), info.contains(&want))?;
    let img = d.shot("dragged")?;
    expect(&format!("xev is framed at {to:?}"), framed(&img, to, true))?;

    // 7. The close buttons (WM_DELETE_WINDOW): xev, xclock, xeyes.
    d.click(to.0 + to.2 - 12, to.1 - 12)?;
    expect("xev ended after its close button", gone(s, xev_pid)?)?;
    d.click(clock.0 + clock.2 - 12, clock.1 - 12)?;
    expect("xclock ended after its close button", gone(s, clock_pid)?)?;
    d.click(ex + ew - 12, ey - 12)?;
    let l = log_wait(s, "XEyes (pid [0-9]*) exited", 60)?;
    expect(&format!("xeyes exited 0 after its close button ({l:?})"), l.contains("exited with status 0"))?;
    std::thread::sleep(Duration::from_millis(2000));
    let img = d.shot("closed")?;
    for (what, w) in [("xev", to), ("xclock", clock), ("xeyes", eyes)] {
        expect(&format!("{what}'s frame is gone"), !framed(&img, w, true) && !framed(&img, w, false))?;
    }
    let left = s.run("DISPLAY=:0 xprop -root _NET_CLIENT_LIST")?;
    expect(&format!("no X windows are left ({left:?})"), !left.contains("window id # 0x"))?;

    // 8. xterm from the launcher (its own desktop file): a command typed
    // into it runs in its shell (a pty), then its close button ends it.
    d.type_keys(&["x", "t", "e", "r", "m"])?;
    log_wait(s, "launcher query \"xterm\"", 30)?;
    d.type_keys(&["ret"])?;
    log_wait(s, "launched XTerm", 30)?;
    let out = log_wait(s, "window \"xterm\" shown at", 120)?;
    let term = window_place(&out).ok_or_else(|| format!("unreadable window line {out:?}"))?;
    std::thread::sleep(Duration::from_millis(3000));
    d.send(&abs(term.0 + term.2 / 2, term.1 + term.3 / 2))?;
    d.type_keys(&["t", "o", "u", "c", "h", "spc", "slash", "t", "m", "p", "slash", "x", "t", "ret"])?;
    let typed = s.run("for i in $(seq 1 60); do test -e /tmp/xt && break; sleep 0.5; done; ls /tmp/xt")?;
    expect(
        &format!("the command typed into xterm ran ({typed:?})"),
        typed.contains("/tmp/xt") && !typed.contains("No such"),
    )?;
    let img = d.shot("xterm")?;
    expect(&format!("xterm at {term:?} is framed"), framed(&img, term, true))?;
    d.click(term.0 + term.2 - 12, term.1 - 12)?;
    log_wait(s, "XTerm (pid [0-9]*) exited", 60)?;

    s.send("poweroff\r")?;
    s.expect("reboot: Power down.")?;
    let _ = s.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_window_lines() {
        let out = "poly-compositor: window \"xeyes\" shown at 565,330 size 150x100\n";
        assert_eq!(window_place(out), Some((565, 330, 150, 100)));
        assert_eq!(window_place("nothing"), None);
    }

    #[test]
    fn the_guest_waits_for_log_lines() {
        assert_eq!(wait_for("/tmp/x.log", "ready", 3), "w /tmp/x.log 'ready' 6");
        assert!(WAIT_FN.contains("echo \"TIMEOUT $2\""));
    }

    #[test]
    fn typed_lines_fit_the_serial_ring() {
        let longest = wait_for("/tmp/comp.log", "X display :0 on /tmp/.X11-unix/X0 and 2 socket(s)", 30);
        assert!(WAIT_FN.len() < SERIAL_RING && longest.len() < SERIAL_RING);
    }
}
