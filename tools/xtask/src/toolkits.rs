//! GTK, Qt and VLC on Poly's desktop (roadmap Phase 6, "GTK and Qt apps
//! run; VLC plays a video with sound"; ADR 0120).
//!
//! The toolkits' root disk is the persistent root disk of ADR 0059
//! (`rootdisk.rs`) with more of the host's files in it, collected the same
//! way (each at its host path, the `DT_NEEDED` closure of every ELF file,
//! nothing run on the host to find anything), never committed:
//!
//! - the test apps of `tests/linux/toolkits/`, built here against the
//!   host's GTK 3, GTK 4 and Qt 6 (`pkg-config`): `gtk3-hello`,
//!   `gtk4-hello`, `qt-hello`; and the toolkits' own `gtk3-demo` and
//!   `gtk4-demo`;
//! - what the toolkits load at run time rather than link: GDK-Pixbuf's
//!   loaders and their cache, GTK 3's input method modules, Qt 5's and
//!   Qt 6's Wayland platform plugin with its shell, decoration and buffer
//!   integration plugins ([`QT_PLUGINS`]), VLC's plugins ([`VLC_PLUGINS`]);
//! - data: the compiled GSettings schemas, the Adwaita and hicolor icon
//!   themes, the shared MIME database's compiled files, the `C.UTF-8`
//!   locale, XKB data, ALSA's configuration (`/usr/share/alsa`, and
//!   `/etc/asound.conf` making card 0 the default);
//! - VLC (`vlc`, `cvlc`) and ALSA's `aplay`;
//! - the compositor (`poly-compositor`, as on the desktop of ADR 0107),
//!   its demo client, desktop files for the launcher, and
//!   `/etc/poly-desktop.env`: the environment every app gets (the
//!   `XDG_*` directories, `LANG=C.UTF-8`, `GDK_BACKEND=wayland`,
//!   `GSK_RENDERER=cairo` until Mesa runs, `QT_QPA_PLATFORM=wayland`);
//! - user `poly` and `poly-runas` (`tests/linux/toolkits/poly-runas.c`),
//!   which runs a program as that user: VLC will not run as root;
//! - a short test video ([`MEDIA`]), made by the host's `ffmpeg`: Ogg
//!   Theora with Vorbis sound, three seconds of solid colours over a
//!   440 Hz tone.
//!
//! Boot: BusyBox's init from an initramfs mounts the disk and chroots into
//! it twice: once to start the desktop (`/usr/local/sbin/poly-desktop`,
//! the compositor with `--desktop`), and on the serial console the disk's
//! bash, which the boot test types into (their output goes to the console
//! too).
//!
//! `cargo xtask toolkits [--fresh-disk] [STEP...]` boots it and runs the
//! steps (see [`Step`]), for trying things; boot group `toolkits` is the
//! test ([`boot_test`]).

use crate::busybox::Session;
use crate::rootdisk::{self, Collector, Contents};
use crate::symbols::Symbols;
use crate::{Result, compositor, root, screen};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The image `cargo xtask toolkits` boots (kept between runs).
pub const FILE_NAME: &str = "poly-toolkits-disk.img";
/// The image the boot test makes afresh.
pub const TEST_FILE_NAME: &str = "poly-toolkits-disk-test.img";

/// The host's multiarch library directory.
const LIBDIR: &str = "/usr/lib/x86_64-linux-gnu";

/// Where the test apps are built.
const OUT: &str = "target/xtask-programs/toolkits";

/// The test apps: name, source, `pkg-config` module, compiler.
const APPS: [(&str, &str, &str, &str); 3] = [
    ("gtk3-hello", "tests/linux/toolkits/gtk3-hello.c", "gtk+-3.0", "cc"),
    ("gtk4-hello", "tests/linux/toolkits/gtk4-hello.c", "gtk4", "cc"),
    ("qt-hello", "tests/linux/toolkits/qt-hello.cpp", "Qt6Widgets", "c++"),
];

/// Qt's plugins, under `qt5/plugins` and `qt6/plugins`: the Wayland
/// platform plugin and what it loads (shell integrations: xdg-shell;
/// decorations; buffer integrations, the EGL ones failing over to SHM
/// without a GPU), and image formats and icon engines for icons.
pub const QT_PLUGINS: &[&str] = &[
    "platforms/libqwayland-generic.so",
    "wayland-shell-integration",
    "wayland-decoration-client",
    "wayland-graphics-integration-client",
    "imageformats",
    "iconengines",
];

/// VLC's plugins, under `vlc/plugins` (a directory names all of it): what
/// playing an Ogg file with Theora video and Vorbis sound through ALSA
/// onto a Wayland window (or into a file) needs, its Qt interface, and the
/// dummy interface `cvlc` uses.
pub const VLC_PLUGINS: &[&str] = &[
    "access/libfilesystem_plugin.so",
    "audio_filter",
    "audio_mixer",
    "audio_output/libalsa_plugin.so",
    "audio_output/libafile_plugin.so",
    "audio_output/libadummy_plugin.so",
    "codec/libtheora_plugin.so",
    "codec/libvorbis_plugin.so",
    "codec/libaraw_plugin.so",
    "codec/librawvideo_plugin.so",
    "control/libdummy_plugin.so",
    "control/libhotkeys_plugin.so",
    "demux/libogg_plugin.so",
    "demux/libwav_plugin.so",
    "gui/libqt_plugin.so",
    "logger/libconsole_logger_plugin.so",
    "spu",
    "video_chroma",
    "video_filter",
    "video_output/libwl_shm_plugin.so",
    "video_output/libxdg_shell_plugin.so",
    "video_output/libyuv_plugin.so",
    "video_output/libvdummy_plugin.so",
];

/// The test video on the disk.
pub const MEDIA: &str = "usr/share/poly/media/poly-test.ogv";
/// Its tone.
pub const MEDIA_HZ: f64 = 440.0;
/// Its colours, one second each (RGB; ffmpeg's colour names `red`,
/// `lime`, `blue`).
pub const MEDIA_COLOURS: [[u8; 3]; 3] = [[0xff, 0, 0], [0, 0xff, 0], [0, 0, 0xff]];

/// The environment every app gets: the compositor's (the launcher passes
/// its own on), and the console's bash reads it from `/etc/profile`.
pub const DESKTOP_ENV: &str = "\
# The desktop's environment (tools/xtask/src/toolkits.rs, ADR 0120).
export XDG_RUNTIME_DIR=/run/user/0
export WAYLAND_DISPLAY=wayland-0
export XDG_CONFIG_HOME=/root/.config
export XDG_DATA_HOME=/root/.local/share
export XDG_CACHE_HOME=/root/.cache
export XDG_STATE_HOME=/root/.local/state
export XDG_DATA_DIRS=/usr/local/share:/usr/share
export XDG_CONFIG_DIRS=/etc/xdg
export XDG_SESSION_TYPE=wayland
export LANG=C.UTF-8
# GTK on Wayland, drawn with cairo in software (no GL until Mesa runs).
export GDK_BACKEND=wayland
export GSK_RENDERER=cairo
# Qt on Wayland.
export QT_QPA_PLATFORM=wayland
";

/// `/usr/local/sbin/poly-desktop`: the compositor on the screen, with the
/// desktop's environment. Its runtime directory stays private (mode
/// 0700); programs run as `poly` (`poly-runas`: VLC will not run as root)
/// reach the socket through a hard link in their own.
const DESKTOP_SCRIPT: &str = "#!/bin/bash
# Started by init in the chroot (tools/xtask/src/toolkits.rs).
. /etc/poly-desktop.env
mkdir -p \"$XDG_RUNTIME_DIR\" && chmod 700 \"$XDG_RUNTIME_DIR\"
mkdir -p \"$XDG_CONFIG_HOME\" \"$XDG_DATA_HOME\" \"$XDG_CACHE_HOME\" \"$XDG_STATE_HOME\"
cd /root
exec /usr/local/bin/poly-compositor --desktop
";

/// `/etc/asound.conf`: ALSA's `default` device is card 0 through the `plug`
/// plugin (converting to what the card takes), so programs that ask for
/// no device in particular play there (asoundrc documentation).
pub const ASOUND_CONF: &str = "\
# Poly's sound card for ALSA's default (tools/xtask/src/toolkits.rs).
pcm.!default {
    type plug
    slave.pcm \"hw:0,0\"
}
ctl.!default {
    type hw
    card 0
}
";

/// The user programs that refuse root run as (`poly-runas poly ...`).
pub const USER: &str = "poly:x:1000:1000:Poly:/home/poly:/bin/bash\n";
/// Its group.
pub const USER_GROUP: &str = "poly:x:1000:\n";

/// BusyBox init's table: mount the disk, start the desktop in it, and the
/// disk's bash on the console.
pub const INITTAB: &str = "\
# BusyBox init(8) table for Poly's toolkits disk (tools/xtask/src/toolkits.rs)
::sysinit:/etc/init.d/rcS
::sysinit:/etc/init.d/rootdisk
::once:/sbin/chroot /mnt/root /usr/local/sbin/poly-desktop
::respawn:/bin/cttyhack /sbin/chroot /mnt/root /bin/bash -l
::ctrlaltdel:/sbin/reboot
::shutdown:/bin/umount -a -r
";

/// The launcher's desktop files: file name, name, command.
const DESKTOP_FILES: [(&str, &str, &str); 6] = [
    ("poly-gtk3-hello.desktop", "GTK 3 Hello", "/usr/local/bin/gtk3-hello"),
    ("poly-gtk4-hello.desktop", "GTK 4 Hello", "/usr/local/bin/gtk4-hello"),
    ("poly-qt-hello.desktop", "Qt Hello", "/usr/local/bin/qt-hello"),
    ("poly-gtk3-demo.desktop", "GTK 3 Demo", "/usr/bin/gtk3-demo"),
    ("poly-gtk4-demo.desktop", "GTK 4 Demo", "/usr/bin/gtk4-demo"),
    (
        "poly-vlc.desktop",
        "VLC",
        "/usr/local/bin/poly-runas poly /usr/bin/vlc --no-qt-privacy-ask --loop /usr/share/poly/media/poly-test.ogv",
    ),
];

/// Build one test app against the host's toolkit; the binary, or why not.
fn build_app(name: &str, source: &str, module: &str, compiler: &str) -> std::result::Result<PathBuf, String> {
    let out = root().join(OUT);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let flags = Command::new("pkg-config")
        .args(["--cflags", "--libs", module])
        .output()
        .map_err(|e| format!("pkg-config: {e}"))?;
    if !flags.status.success() {
        return Err(format!("no {module} on this host (pkg-config)"));
    }
    let flags = String::from_utf8_lossy(&flags.stdout).into_owned();
    let binary = out.join(name);
    let mut cmd = Command::new(compiler);
    cmd.current_dir(root()).args(["-O2", "-Wall", "-Wextra", "-Werror", "-o"]).arg(&binary).arg(source);
    if compiler == "c++" {
        // Qt's headers ask for position-independent code.
        cmd.arg("-fPIC");
    }
    cmd.args(flags.split_whitespace());
    let built = cmd.output().map_err(|e| format!("{compiler}: {e}"))?;
    if !built.status.success() {
        return Err(format!("{source} did not build: {}", String::from_utf8_lossy(&built.stderr).trim()));
    }
    Ok(binary)
}

/// Make the test video with the host's `ffmpeg`: the file, or why not.
fn make_media() -> std::result::Result<PathBuf, String> {
    let path = root().join("target/poly-test.ogv");
    if path.is_file() {
        return Ok(path);
    }
    let colours = ["red", "lime", "blue"];
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-loglevel", "error", "-y"]);
    for c in colours {
        cmd.args(["-f", "lavfi", "-i", &format!("color=c={c}:s=320x240:r=15:d=1")]);
    }
    cmd.args(["-f", "lavfi", "-i", &format!("sine=frequency={MEDIA_HZ}:sample_rate=48000:duration=3")]);
    cmd.args(["-filter_complex", "[0:v][1:v][2:v]concat=n=3:v=1:a=0[v]", "-map", "[v]", "-map", "3:a"]);
    cmd.args(["-c:v", "libtheora", "-q:v", "8", "-c:a", "libvorbis", "-q:a", "5", "-ac", "2"]);
    let tmp = path.with_extension("tmp.ogv");
    cmd.arg(&tmp);
    let out = cmd.output().map_err(|e| format!("no ffmpeg ({e})"))?;
    if !out.status.success() {
        return Err(format!("ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Add `dir/part` for every part that exists; notes the missing ones.
fn add_parts(
    c: &mut Collector,
    dir: &Path,
    parts: &[&str],
    notes: &mut Vec<String>,
) -> std::result::Result<(), String> {
    for part in parts {
        let p = dir.join(part);
        if p.is_dir() {
            c.add_tree(&p, &[])?;
        } else if p.exists() {
            c.add(&p)?;
        } else {
            notes.push(format!("{}: not on this host", p.display()));
        }
    }
    Ok(())
}

/// The host files of the toolkits (see the module documentation).
fn add_toolkits(
    c: &mut Collector,
    have: &mut BTreeSet<&'static str>,
    notes: &mut Vec<String>,
) -> std::result::Result<(), String> {
    let lib = Path::new(LIBDIR);
    for (name, source, module, compiler) in APPS {
        match build_app(name, source, module, compiler) {
            Ok(binary) => {
                let data = std::fs::read(&binary).map_err(|e| format!("{}: {e}", binary.display()))?;
                c.tree.file(&format!("usr/local/bin/{name}"), 0o755, &data);
                // Its libraries: the closure of the host's copy.
                c.add_elf_needs(&binary);
                have.insert(name);
            }
            Err(why) => notes.push(format!("{name}: {why}")),
        }
    }
    for program in ["gtk3-demo", "gtk4-demo", "vlc", "cvlc", "aplay"] {
        let p = Path::new("/usr/bin").join(program);
        if p.exists() {
            c.add(&p)?;
            have.insert(program);
        } else {
            notes.push(format!("{program}: not on this host"));
        }
    }
    // GTK's run-time modules and data.
    add_parts(c, lib, &["gdk-pixbuf-2.0", "gtk-3.0/3.0.0/immodules", "gtk-3.0/3.0.0/immodules.cache"], notes)?;
    let share = Path::new("/usr/share");
    add_parts(c, share, &["glib-2.0/schemas/gschemas.compiled", "icons/hicolor", "icons/Adwaita"], notes)?;
    // The shared MIME database's compiled files (not the XML they come
    // from): GIO guesses content types with it, and GDK-Pixbuf picks a
    // loader by content type, so without it no icon (SVG) loads.
    let mime = share.join("mime");
    if let Ok(dir) = std::fs::read_dir(&mime) {
        let mut files: Vec<PathBuf> = dir.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_file()).collect();
        files.sort();
        for f in files {
            c.add(&f)?;
        }
    } else {
        notes.push(format!("{}: not on this host (icons will not load)", mime.display()));
    }
    // Qt's plugins.
    for qt in ["qt5", "qt6"] {
        let dir = lib.join(qt).join("plugins");
        if dir.is_dir() {
            add_parts(c, &dir, QT_PLUGINS, notes)?;
        }
    }
    // VLC's plugins and ALSA's configuration.
    let vlc = lib.join("vlc/plugins");
    if vlc.is_dir() {
        add_parts(c, &vlc, VLC_PLUGINS, notes)?;
    }
    add_parts(c, share, &["alsa"], notes)?;
    // The C.UTF-8 locale (glibc builds it as a locale like any other, so
    // it is files) and the XKB data xkbcommon looks for (the keymap itself
    // comes from the compositor).
    add_parts(c, Path::new("/usr/lib/locale"), &["C.utf8"], notes)?;
    add_parts(c, share, &["X11/xkb"], notes)?;
    Ok(())
}

/// Collect the toolkits' disk.
pub fn collect() -> std::result::Result<Contents, String> {
    let mut contents = rootdisk::collect_with(add_toolkits)?;
    let tree = &mut contents.tree;
    // POLY_TOOLKITS_COMPOSITOR=<file>: another build of the compositor
    // (to try the apps against a compositor in the making).
    let compositor = match std::env::var_os("POLY_TOOLKITS_COMPOSITOR") {
        Some(file) => std::fs::read(&file).map_err(|e| format!("{}: {e}", Path::new(&file).display()))?,
        None => compositor::build_compositor()?,
    };
    tree.file("usr/local/bin/poly-compositor", 0o755, &compositor);
    match compositor::build_c("tests/linux/wayland/wl-demo.c", "wl-demo") {
        Ok(demo) => tree.file("usr/local/bin/wl-demo", 0o755, &demo),
        Err(why) => contents.notes.push(format!("wl-demo: {why}")),
    }
    match compositor::build_c("tests/linux/toolkits/poly-runas.c", "poly-runas") {
        Ok(runas) => {
            tree.file("usr/local/bin/poly-runas", 0o755, &runas);
            contents.have.insert("poly-runas");
        }
        Err(why) => contents.notes.push(format!("poly-runas: {why}")),
    }
    let passwd = format!("{}{USER}", crate::initramfs::PASSWD.replace("/bin/sh", "/bin/bash"));
    tree.file("etc/passwd", 0o644, passwd.as_bytes());
    tree.file("etc/group", 0o644, format!("{}{USER_GROUP}", crate::initramfs::GROUP).as_bytes());
    tree.file("etc/poly-desktop.env", 0o644, DESKTOP_ENV.as_bytes());
    tree.file("etc/asound.conf", 0o644, ASOUND_CONF.as_bytes());
    let profile = format!("{}. /etc/poly-desktop.env\n", rootdisk::PROFILE);
    tree.file("etc/profile", 0o644, profile.as_bytes());
    tree.file("usr/local/sbin/poly-desktop", 0o755, DESKTOP_SCRIPT.as_bytes());
    for (file, name, exec) in DESKTOP_FILES {
        let entry = format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\n");
        tree.file(&format!("usr/share/applications/{file}"), 0o644, entry.as_bytes());
    }
    for dir in ["run/user", "root/.config", "root/.cache", "root/.local/share"] {
        tree.dir(dir, 0o755);
    }
    match make_media() {
        Ok(path) => {
            let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            tree.file(MEDIA, 0o644, &data);
            contents.have.insert("media");
        }
        Err(why) => contents.notes.push(format!("the test video: {why}")),
    }
    Ok(contents)
}

/// Collect the disk and write it to `target/<name>`.
pub fn make(name: &str) -> std::result::Result<(PathBuf, Contents), String> {
    let contents = collect()?;
    for note in &contents.notes {
        eprintln!("==> note: toolkits disk: {note}");
    }
    let path = root().join("target").join(name);
    let how = rootdisk::write_image(&contents.tree, &path)?;
    eprintln!(
        "==> toolkits disk {} ({how}): {} entries; {}",
        path.display(),
        contents.tree.entries.len(),
        contents.have.iter().copied().collect::<Vec<_>>().join(", ")
    );
    Ok((path, contents))
}

/// The initramfs: BusyBox's init with [`INITTAB`].
fn write_initramfs() -> Result<std::result::Result<PathBuf, String>> {
    let binary = match crate::busybox::obtain()? {
        Ok(b) => b,
        Err(why) => return Ok(Err(why)),
    };
    let image = crate::initramfs::disk_root_image_with(&binary.elf, INITTAB);
    Ok(Ok(crate::initramfs::write(&root(), "poly-toolkits-initramfs.cpio", &image)?))
}

/// Guest memory: GTK 4, Qt and VLC side by side.
const MEMORY: &str = "2G";
/// The screen.
pub const SIZE: (u32, u32) = crate::desktop::SIZE;
/// What the compositor prints when the desktop is up.
const READY: &str = "poly-compositor: desktop ready";

/// A boot of the toolkits' disk: the serial session, QMP, the recording.
struct Machine {
    s: Session,
    qmp: screen::Qmp,
    shots: PathBuf,
    shot_no: u32,
    wav: PathBuf,
    qmp_path: PathBuf,
}

/// One thing to do in a boot (`cargo xtask toolkits` steps, as `kind:arg`).
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// `run:LINE`: type a command line and wait for the prompt.
    Run(String),
    /// `send:LINE`: type a command line, do not wait.
    Send(String),
    /// `expect:TEXT`: wait for text on the console.
    Expect(String),
    /// `shot:NAME`: a screenshot.
    Shot(String),
    /// `click:X,Y`: move the pointer there and click.
    Click(i32, i32),
    /// `key:QCODE`: press and release a key (QEMU's key names).
    Key(String),
    /// `type:TEXT`: type lower-case letters, digits, spaces and `-`.
    Type(String),
    /// `sleep:SECONDS`.
    Sleep(u64),
}

impl Step {
    pub fn parse(s: &str) -> std::result::Result<Step, String> {
        let (kind, arg) = s.split_once(':').ok_or_else(|| format!("step {s:?}: not KIND:ARG"))?;
        let bad = || format!("step {s:?}: bad argument");
        Ok(match kind {
            "run" => Step::Run(arg.into()),
            "send" => Step::Send(arg.into()),
            "expect" => Step::Expect(arg.into()),
            "shot" => Step::Shot(arg.into()),
            "click" => {
                let (x, y) = arg.split_once(',').ok_or_else(bad)?;
                Step::Click(x.trim().parse().map_err(|_| bad())?, y.trim().parse().map_err(|_| bad())?)
            }
            "key" => Step::Key(arg.into()),
            "type" => {
                if arg.chars().any(|c| qcode(c).is_none()) {
                    return Err(bad());
                }
                Step::Type(arg.into())
            }
            "sleep" => Step::Sleep(arg.parse().map_err(|_| bad())?),
            _ => {
                return Err(format!("step {s:?}: no kind {kind:?} (run, send, expect, shot, click, key, type, sleep)"));
            }
        })
    }
}

/// QEMU's key name for a character [`Step::Type`] types.
fn qcode(c: char) -> Option<String> {
    match c {
        'a'..='z' | '0'..='9' => Some(c.to_string()),
        ' ' => Some("spc".into()),
        '-' => Some("minus".into()),
        _ => None,
    }
}

fn key_event(qcode: &str, down: bool) -> String {
    format!(r#"{{"type": "key", "data": {{"down": {down}, "key": {{"type": "qcode", "data": "{qcode}"}}}}}}"#)
}

fn button_event(down: bool) -> String {
    format!(r#"{{"type": "btn", "data": {{"down": {down}, "button": "left"}}}}"#)
}

/// A tablet position for logical pixel (`x`, `y`) (as `desktop.rs`).
fn abs_events(x: i32, y: i32) -> Vec<String> {
    let (w, h) = (i64::from(SIZE.0) - 1, i64::from(SIZE.1) - 1);
    let ax = (i64::from(x) * 0x7fff + w - 1) / w;
    let ay = (i64::from(y) * 0x7fff + h - 1) / h;
    vec![
        format!(r#"{{"type": "abs", "data": {{"axis": "x", "value": {ax}}}}}"#),
        format!(r#"{{"type": "abs", "data": {{"axis": "y", "value": {ay}}}}}"#),
    ]
}

impl Machine {
    /// Boot `image`.
    fn boot(kernel: &Path, syms: &Arc<Symbols>, image: &Path, initrd: &Path, tag: &str) -> Result<Machine> {
        let qmp_path = std::env::temp_dir().join(format!("poly-toolkits-qmp-{}-{tag}.sock", std::process::id()));
        let _ = std::fs::remove_file(&qmp_path);
        let wav = root().join(format!("target/poly-toolkits-{tag}.wav"));
        let _ = std::fs::remove_file(&wav);
        // POLY_TOOLKITS_APPEND: more kernel arguments (`poly.strace=...`).
        let append = format!(
            "{} console=ttyS0 poly.terminal=off {}",
            crate::busybox::INIT_ARGS,
            std::env::var("POLY_TOOLKITS_APPEND").unwrap_or_default()
        );
        let mut cmd = crate::qemu(kernel, MEMORY, append.trim_end(), crate::Devices::None, "stdio");
        cmd.args(["-display", "none"])
            .args(rootdisk::drive_args(image))
            .arg("-initrd")
            .arg(initrd)
            .args(crate::desktop::devices())
            .args(["-audiodev", &format!("wav,id=snd0,path={}", wav.display())])
            .args(["-device", "virtio-sound-pci,audiodev=snd0"])
            .args(["-qmp", &format!("unix:{},server=on,wait=off", qmp_path.display())]);
        let mut s = Session::spawn(cmd, syms)?;
        s.set_timeouts(Duration::from_secs(600), Duration::from_secs(3600));
        let qmp = screen::Qmp::connect(&qmp_path, Duration::from_secs(30))?;
        let shots = shots_dir();
        std::fs::create_dir_all(&shots).map_err(|e| format!("{}: {e}", shots.display()))?;
        Ok(Machine { s, qmp, shots, shot_no: 0, wav, qmp_path })
    }

    fn send_events(&mut self, events: &[String]) -> Result {
        self.qmp.execute("input-send-event", &format!(r#"{{"events": [{}]}}"#, events.join(", ")))?;
        std::thread::sleep(Duration::from_millis(150));
        Ok(())
    }

    fn click(&mut self, x: i32, y: i32) -> Result {
        self.send_events(&abs_events(x, y))?;
        std::thread::sleep(Duration::from_millis(300));
        self.send_events(&[button_event(true)])?;
        self.send_events(&[button_event(false)])
    }

    fn key(&mut self, qcode: &str) -> Result {
        self.send_events(&[key_event(qcode, true)])?;
        self.send_events(&[key_event(qcode, false)])
    }

    fn type_text(&mut self, text: &str) -> Result {
        for c in text.chars() {
            self.key(&qcode(c).ok_or_else(|| format!("cannot type {c:?}"))?)?;
        }
        Ok(())
    }

    /// How much console text there is so far (for [`Machine::seen`]).
    fn text_len(&mut self) -> usize {
        self.s.text().len()
    }

    /// Wait until `needle` is in the console text after `from`, in any
    /// order with other lines (an app and the compositor print at once);
    /// the rest of its line once the line is complete.
    fn seen(&mut self, from: usize, needle: &str, timeout: Duration) -> Result<String> {
        self.seen_any(from, &[needle], timeout).map(|(_, rest)| rest)
    }

    /// [`Machine::seen`] for the first of `needles` to come: which, and the
    /// rest of its line.
    fn seen_any(&mut self, from: usize, needles: &[&str], timeout: Duration) -> Result<(usize, String)> {
        let deadline = Instant::now() + timeout;
        loop {
            let text = self.s.text();
            let after = text.get(from..).unwrap_or_default();
            let first = needles
                .iter()
                .enumerate()
                .filter_map(|(k, n)| after.find(n).map(|i| (i, k, i + n.len())))
                .filter(|&(_, _, end)| after[end..].contains('\n'))
                .min();
            if let Some((_, k, end)) = first {
                let rest = &after[end..];
                return Ok((k, rest[..rest.find('\n').unwrap_or(rest.len())].trim_end().to_string()));
            }
            if Instant::now() > deadline {
                return Err(format!("no {needles:?} within {} s", timeout.as_secs()));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// A screenshot (PNG for the record, read back from a PPM).
    fn shot(&mut self, what: &str) -> Result<screen::Image> {
        self.shot_no += 1;
        let png = self.shots.join(format!("toolkits-{:02}-{what}.png", self.shot_no));
        let ppm = png.with_extension("ppm");
        self.qmp.screendump(&png)?;
        self.qmp.screendump(&ppm)?;
        let img = screen::Image::read_ppm(&ppm)?;
        let _ = std::fs::remove_file(&ppm);
        eprintln!("==> screenshot {}", png.display());
        Ok(img)
    }

    fn step(&mut self, step: &Step) -> Result {
        match step {
            Step::Run(line) => {
                let out = self.s.run(line)?;
                eprintln!("==> {line}: {out:?}");
            }
            Step::Send(line) => self.s.send(&format!("{line}\r"))?,
            Step::Expect(text) => {
                self.s.expect(text)?;
            }
            Step::Shot(name) => {
                self.shot(name)?;
            }
            Step::Click(x, y) => self.click(*x, *y)?,
            Step::Key(k) => self.key(k)?,
            Step::Type(text) => self.type_text(text)?,
            Step::Sleep(secs) => std::thread::sleep(Duration::from_secs(*secs)),
        }
        Ok(())
    }

    /// Power off and wait for QEMU to end.
    fn poweroff(&mut self) -> Result {
        self.s.send("poweroff\r")?;
        let r = self.s.expect("reboot: Power down.");
        let _ = self.s.finish();
        r.map(drop)
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.qmp_path);
    }
}

/// Where the screenshots go: `$POLY_TOOLKITS_SHOTS`, else
/// `target/toolkits/`.
fn shots_dir() -> PathBuf {
    std::env::var_os("POLY_TOOLKITS_SHOTS").map_or_else(|| root().join("target").join("toolkits"), PathBuf::from)
}

/// `cargo xtask toolkits [--release] [--fresh-disk] [STEP...]`: boot the
/// toolkits' disk to the desktop and the console's bash, run the steps,
/// power off.
pub fn command(args: &[String]) -> Result {
    let release = args.iter().any(|a| a == "--release");
    let fresh = args.iter().any(|a| a == "--fresh-disk");
    let steps = args.iter().filter(|a| !a.starts_with("--")).map(|a| Step::parse(a)).collect::<Result<Vec<_>>>()?;
    let kernel = crate::build_kernel(release)?;
    let syms = Arc::new(Symbols::from_file(&kernel)?);
    let image = root().join("target").join(FILE_NAME);
    if fresh || !image.exists() {
        make(FILE_NAME)?;
    }
    let initrd = write_initramfs()?.map_err(|why| format!("toolkits: {why}"))?;
    let mut m = Machine::boot(&kernel, &syms, &image, &initrd, "probe")?;
    let started = Instant::now();
    m.s.expect("rootdisk: /dev/vda mounted on /mnt/root")?;
    m.s.expect(READY)?;
    // The prompt may have come before the compositor's lines: ask again.
    m.s.send("\r")?;
    m.s.prompt()?;
    eprintln!("==> desktop up after {} s", started.elapsed().as_secs());
    let mut result = Ok(());
    for step in &steps {
        if let Err(e) = m.step(step) {
            result = Err(format!("{step:?}: {e}"));
            break;
        }
    }
    let log = root().join("target/poly-toolkits-probe.log");
    let _ = std::fs::write(&log, m.s.text());
    eprintln!("==> the console's output is in {}", log.display());
    let off = m.poweroff();
    result.and(off)
}

// ---- the boot test -------------------------------------------------------

/// An app the boot test starts from the launcher.
struct App {
    /// The program, and how its lines start.
    name: &'static str,
    /// What is typed into the launcher to find it.
    query: &'static str,
    /// Its desktop file's name (the compositor logs the launch by it).
    launcher_name: &'static str,
    /// Its window's title.
    title: &'static str,
    /// The button's colour before and after the click.
    before: [u8; 3],
    after: [u8; 3],
    /// What it prints for a typed `a`.
    key_line: &'static str,
}

/// The apps (their colours are in their sources).
const TEST_APPS: [App; 3] = [
    App {
        name: "gtk3-hello",
        query: "3 hello",
        launcher_name: "GTK 3 Hello",
        title: "Poly GTK 3",
        before: [0x2a, 0x6f, 0xb0],
        after: [0xb0, 0x3a, 0x2a],
        key_line: "gtk3-hello: key a",
    },
    App {
        name: "gtk4-hello",
        query: "4 hello",
        launcher_name: "GTK 4 Hello",
        title: "Poly GTK 4",
        before: [0x2a, 0x8a, 0x4a],
        after: [0x8a, 0x2a, 0x7a],
        key_line: "gtk4-hello: key a",
    },
    App {
        name: "qt-hello",
        query: "qt",
        launcher_name: "Qt Hello",
        title: "Poly Qt",
        before: [0x7a, 0x5a, 0x1a],
        after: [0x1a, 0x6a, 0x7a],
        key_line: "qt-hello: key 65 a",
    },
];

/// The active title bar of Poly's frame (`poly_wayland::compose::frame`,
/// `Theme::NEUTRAL`).
const TITLE_ACTIVE: [u8; 3] = [0x3c, 0x3c, 0x3c];
/// The inactive one (a new window is not always activated yet).
const TITLE_INACTIVE: [u8; 3] = [0x2a, 0x2a, 0x2a];
/// How far above the button Poly's title bar may be: GTK's own title bar
/// (client-side decorations) can be between them.
const TITLE_SEARCH: u32 = 90;

/// A window's place from the compositor's line `window "T" shown at X,Y
/// size WxH` (the part after the title).
fn window_place(rest: &str) -> Option<(i32, i32, i32, i32)> {
    let rest = rest.split(" shown at ").nth(1)?;
    let (pos, size) = rest.split_once(" size ")?;
    let (x, y) = pos.split_once(',')?;
    let (w, h) = size.trim().split_once('x')?;
    Some((x.parse().ok()?, y.parse().ok()?, w.parse().ok()?, h.parse().ok()?))
}

/// Whether two colours are within `tolerance` in every channel.
fn near(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= tolerance)
}

/// The pixels of `colour` (within `tolerance`) inside the rectangle
/// `(x, y, w, h)`: their bounding box `(x0, y0, x1, y1)` (inclusive) and
/// how many there are.
fn find_colour(
    img: &screen::Image,
    colour: [u8; 3],
    tolerance: u8,
    (x, y, w, h): (i32, i32, i32, i32),
) -> Option<((u32, u32, u32, u32), usize)> {
    let (mut x0, mut y0, mut x1, mut y1, mut n) = (u32::MAX, u32::MAX, 0, 0, 0);
    let xs = x.max(0) as u32..((x + w).max(0) as u32).min(img.width);
    for py in y.max(0) as u32..((y + h).max(0) as u32).min(img.height) {
        for px in xs.clone() {
            if near(img.pixel(px, py), colour, tolerance) {
                (x0, y0, x1, y1) = (x0.min(px), y0.min(py), x1.max(px), y1.max(py));
                n += 1;
            }
        }
    }
    (n > 0).then_some(((x0, y0, x1, y1), n))
}

fn expect(what: &str, ok: bool) -> Result {
    if ok { Ok(()) } else { Err(what.to_string()) }
}

impl Machine {
    /// One app: started from the launcher, its window shown in Poly's
    /// frame with its button's colour, a click and a key reach it, Q ends
    /// it.
    fn check_app(&mut self, app: &App) -> Result {
        self.type_text(app.query)?;
        self.s.expect(&format!("launcher query {:?}", app.query))?;
        std::thread::sleep(Duration::from_millis(500));
        let from = self.text_len();
        self.key("ret")?;
        let wait = Duration::from_secs(300);
        self.seen(from, &format!("poly-compositor: launched {}", app.launcher_name), wait)?;
        let rest = self.seen(from, &format!("poly-compositor: window {:?}", app.title), wait)?;
        let place @ (x, y, w, h) = window_place(&rest).ok_or_else(|| format!("unreadable window line {rest:?}"))?;
        self.seen(from, &format!("{}: drawn", app.name), wait)?;
        std::thread::sleep(Duration::from_secs(2));
        let img = self.shot(app.name)?;
        let ((x0, y0, x1, y1), n) = find_colour(&img, app.before, 3, place)
            .ok_or_else(|| format!("no pixel of the button's colour {:02x?} in the window at {place:?}", app.before))?;
        expect(
            &format!("the button fills most of the window ({n} pixels of {w}x{h})"),
            n as i64 * 3 >= i64::from(w) * i64::from(h),
        )?;
        // Poly's title bar is above the window's content: right above the
        // button, or above the title bar a GTK app draws itself (the
        // reported place includes its shadows; Poly's frame goes around
        // the window geometry).
        let title = |p: [u8; 3]| near(p, TITLE_ACTIVE, 2) || near(p, TITLE_INACTIVE, 2);
        let framed = (y0.saturating_sub(TITLE_SEARCH)..y0).any(|py| title(img.pixel(x0 + 3, py)));
        expect("Poly's title bar is above the window", framed && (x, y) <= (x0 as i32, y0 as i32))?;
        let (cx, cy) = (((x0 + x1) / 2) as i32, ((y0 + y1) / 2) as i32);
        self.click(cx, cy)?;
        self.s.expect(&format!("{}: clicked", app.name))?;
        std::thread::sleep(Duration::from_millis(2500));
        let img = self.shot(&format!("{}-clicked", app.name))?;
        // Most of the button has the other colour now (the label and the
        // cursor are in the middle of it).
        let button = (x0 as i32, y0 as i32, (x1 - x0 + 1) as i32, (y1 - y0 + 1) as i32);
        let now = find_colour(&img, app.after, 3, button).map_or(0, |(_, n)| n);
        let corner = img.pixel(x0 + 12, y0 + 12);
        expect(&format!("the click recoloured the button ({now} of {n} pixels; {corner:02x?})"), now * 4 >= n * 3)?;
        self.key("a")?;
        self.s.expect(app.key_line)?;
        self.key("q")?;
        self.s.expect(&format!("{}: closed, status 0", app.name))?;
        std::thread::sleep(Duration::from_millis(1500));
        Ok(())
    }

    /// VLC without its interface, as user `poly` (it will not run as
    /// root): the test video's sound through ALSA, its pictures into a
    /// YUV4MPEG2 file on the disk. Both are checked after the power-off
    /// (the recording, and the file read back from the image).
    fn check_vlc(&mut self) -> Result {
        // The launcher is back on the empty desktop: hide it.
        self.key("esc")?;
        // (The quotes keep the typed line, which the terminal echoes, from
        // matching what is waited for.)
        let line = format!(
            "poly-runas poly cvlc {VLC_ARGS} /{MEDIA} 2>/dev/null; echo \"vlc-status:\" $?; sync; echo vlc: \"synced\""
        );
        let from = self.text_len();
        self.s.send(&format!("{line}\r"))?;
        let status = self.seen(from, "vlc-status: ", Duration::from_secs(600))?;
        expect(&format!("VLC played the video and exited 0 (status {status})"), status == "0")?;
        self.seen(from, "vlc: synced", Duration::from_secs(120)).map(drop)
    }

    /// VLC's Qt interface (Qt 5 on Wayland), started from the launcher: its
    /// window, framed, shows the video (one of its colours fills a good
    /// part of the window); Ctrl+Q ends it.
    fn check_vlc_gui(&mut self) -> Result {
        // Back on the empty desktop after cvlc: the launcher is hidden.
        self.key("esc")?;
        self.key("meta_l")?;
        std::thread::sleep(Duration::from_millis(500));
        self.type_text("vlc")?;
        self.s.expect("launcher query \"vlc\"")?;
        std::thread::sleep(Duration::from_millis(500));
        let from = self.text_len();
        self.key("ret")?;
        let wait = Duration::from_secs(300);
        self.seen(from, "poly-compositor: launched VLC", wait)?;
        // (The compositor reaps its children only when it wakes for
        // something else: the kernel's line about a fatal signal is seen
        // first.)
        let ends = ["VLC media player\"", "poly-compositor: VLC (pid ", "killed by signal"];
        let (i, rest) = self.seen_any(from, &ends, wait)?;
        expect(&format!("VLC's window appeared (VLC ended: {rest:?})"), i == 0)?;
        let place @ (x, y, w, h) = window_place(&rest).ok_or_else(|| format!("unreadable window line {rest:?}"))?;
        // Its first pictures take a while under emulation.
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            std::thread::sleep(Duration::from_secs(3));
            let img = self.shot("vlc-gui")?;
            let shown = MEDIA_COLOURS
                .iter()
                .filter_map(|&c| find_colour(&img, c, 64, place))
                .any(|(_, n)| n as i64 * 5 >= i64::from(w) * i64::from(h));
            if shown {
                break;
            }
            expect(&format!("VLC's window at {place:?} shows the video"), Instant::now() < deadline)?;
        }
        // Focus it through Poly's title bar, then Ctrl+Q.
        self.click(x + 40, y - 12)?;
        self.send_events(&[key_event("ctrl", true), key_event("q", true)])?;
        self.send_events(&[key_event("q", false), key_event("ctrl", false)])?;
        self.seen(from, "poly-compositor: VLC (pid ", wait).map(drop)
    }
}

/// How `cvlc` plays the test video: once, through ALSA's `plughw:0,0`
/// (converting to what the card takes), its pictures written to
/// [`VLC_FRAMES`] as YUV4MPEG2 by VLC's `yuv` video output.
pub const VLC_ARGS: &str = "--play-and-exit --no-osd --aout alsa --alsa-audio-device plughw:0,0 \
     --vout yuv --yuv-yuv4mpeg2 --yuv-chroma I420 --yuv-file /home/poly/poly-frames.y4m";
/// Where VLC's pictures are on the disk.
pub const VLC_FRAMES: &str = "home/poly/poly-frames.y4m";

/// The frames of a YUV4MPEG2 stream of 4:2:0 pictures: width, height, and
/// each frame's mean colour converted to RGB (BT.601, limited range).
fn y4m_colours(data: &[u8]) -> Result<(u32, u32, Vec<[u8; 3]>)> {
    let line_end = |at: usize| data[at..].iter().position(|&b| b == b'\n').map(|i| at + i);
    let end = line_end(0).ok_or("no YUV4MPEG2 header")?;
    let header = std::str::from_utf8(&data[..end]).map_err(|_| "a header that is not text")?;
    let mut words = header.split(' ');
    if words.next() != Some("YUV4MPEG2") {
        return Err(format!("not YUV4MPEG2: {header:?}"));
    }
    let (mut w, mut h) = (0u32, 0u32);
    for word in words {
        match word.split_at(1) {
            ("W", v) => w = v.parse().map_err(|_| format!("width {v:?}"))?,
            ("H", v) => h = v.parse().map_err(|_| format!("height {v:?}"))?,
            ("C", v) if !v.starts_with("420") => return Err(format!("chroma {v} (want 4:2:0)")),
            _ => {}
        }
    }
    if w == 0 || h == 0 {
        return Err(format!("no size in {header:?}"));
    }
    let (luma, chroma) = (w as usize * h as usize, w.div_ceil(2) as usize * h.div_ceil(2) as usize);
    let mut colours = Vec::new();
    let mut at = end + 1;
    while at < data.len() {
        let end = line_end(at).ok_or("a frame header without its line end")?;
        if !data[at..end].starts_with(b"FRAME") {
            return Err(format!("no FRAME at byte {at}"));
        }
        let pixels = data.get(end + 1..end + 1 + luma + 2 * chroma).ok_or("a frame cut short")?;
        let mean = |p: &[u8]| p.iter().map(|&v| f64::from(v)).sum::<f64>() / p.len() as f64;
        let (y, u, v) = (mean(&pixels[..luma]), mean(&pixels[luma..luma + chroma]), mean(&pixels[luma + chroma..]));
        let (y, u, v) = (1.164 * (y - 16.0), u - 128.0, v - 128.0);
        let clamp = |x: f64| x.round().clamp(0.0, 255.0) as u8;
        colours.push([clamp(y + 1.596 * v), clamp(y - 0.392 * u - 0.813 * v), clamp(y + 2.017 * u)]);
        at = end + 1 + luma + 2 * chroma;
    }
    Ok((w, h, colours))
}

/// Which of the video's colours (an index into [`MEDIA_COLOURS`]) the
/// frames show, in order, runs of one colour counted once; frames of none
/// of them are left out.
fn colour_runs(colours: &[[u8; 3]]) -> Vec<usize> {
    let mut runs: Vec<usize> = Vec::new();
    for c in colours {
        if let Some(i) = MEDIA_COLOURS.iter().position(|&m| near(*c, m, 48))
            && runs.last() != Some(&i)
        {
            runs.push(i);
        }
    }
    runs
}

/// VLC's pictures, read back from the image: the video's size, and its
/// three colours in their order.
fn check_frames(image: &Path) -> Result<usize> {
    let data = rootdisk::read_back(image, VLC_FRAMES)?;
    let (w, h, colours) = y4m_colours(&data)?;
    expect(&format!("VLC's pictures are 320x240 (they are {w}x{h})"), (w, h) == (320, 240))?;
    let runs = colour_runs(&colours);
    expect(
        &format!("VLC's pictures are red, then green, then blue (they are {runs:?} of {} frames)", colours.len()),
        runs == [0, 1, 2],
    )?;
    Ok(colours.len())
}

/// Lines the kernel logs on the console about missing system calls.
fn unimplemented_lines(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter(|l| (l.starts_with("syscall: pid ") || l.starts_with("linux: pid ")) && l.contains(" not implemented"))
        .map(|l| l.trim_end().to_string())
        .collect()
}

/// The boot test (see the module documentation): GTK 3, GTK 4 and Qt apps
/// started from the launcher, framed, clicked and typed into; VLC playing
/// the test video with sound, heard in QEMU's recording.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    if cfg!(not(unix)) {
        eprintln!("==> skipped: the toolkits boot test needs a Unix host (QMP)");
        return Ok(());
    }
    let initrd = match write_initramfs()? {
        Ok(path) => path,
        Err(why) => {
            eprintln!("==> skipped: the toolkits boot test boots through BusyBox's init: {why}");
            return Ok(());
        }
    };
    let (image, contents) = make(TEST_FILE_NAME)?;
    // POLY_TOOLKITS_ONLY=<name>,...: only these apps (`vlc` for VLC).
    let only = std::env::var("POLY_TOOLKITS_ONLY").ok().filter(|s| !s.is_empty());
    let wanted = |name: &str| only.as_ref().is_none_or(|o| o.split(',').any(|w| w.trim() == name));
    let apps: Vec<&App> = TEST_APPS.iter().filter(|a| contents.have.contains(a.name) && wanted(a.name)).collect();
    let vlc = ["cvlc", "media", "poly-runas"].iter().all(|n| contents.have.contains(n)) && wanted("vlc");
    if apps.is_empty() && !vlc {
        eprintln!("==> skipped: the toolkits boot test: no GTK, Qt or VLC on this host (the poly-dev image has them)");
        return Ok(());
    }
    eprintln!(
        "==> boot test: {} on the desktop from the toolkits' root disk{}",
        apps.iter().map(|a| a.name).collect::<Vec<_>>().join(", "),
        if vlc { ", and VLC playing a video with sound" } else { "" }
    );
    let mut m = Machine::boot(kernel, syms, &image, &initrd, "test")?;
    m.s.set_timeouts(Duration::from_secs(300), Duration::from_secs(2400));
    m.s.expect("rootdisk: /dev/vda mounted on /mnt/root")?;
    m.s.expect(READY)?;
    let mut failed = Vec::new();
    for app in &apps {
        eprintln!("==> {}", app.name);
        if let Err(e) = m.check_app(app) {
            eprintln!("==> FAILED: {}: {e}", app.name);
            failed.push(format!("{}: {e}", app.name));
            break;
        }
    }
    if vlc && failed.is_empty() {
        eprintln!("==> VLC");
        if let Err(e) = m.check_vlc() {
            failed.push(format!("VLC: {e}"));
        } else {
            // Reported, not required yet: against the compositor in the
            // making, VLC's Qt interface is sometimes cut off
            // (`get_viewport: object N is a wl_subsurface, not a
            // wl_surface`, ADR 0120). POLY_TOOLKITS_VLC_GUI=required makes
            // it count.
            eprintln!("==> VLC's Qt interface");
            if let Err(e) = m.check_vlc_gui() {
                if std::env::var("POLY_TOOLKITS_VLC_GUI").is_ok_and(|v| v == "required") {
                    failed.push(format!("VLC's Qt interface: {e}"));
                } else {
                    eprintln!("==> not counted: VLC's Qt interface: {e}");
                }
            }
        }
    }
    let text = m.s.text();
    let log = root().join("target/poly-toolkits-boot.log");
    let _ = std::fs::write(&log, &text);
    if !failed.is_empty() {
        eprintln!("==> the console's output is in {}", log.display());
        return Err(format!("toolkits on Poly: {}", failed.join("; ")));
    }
    m.s.send("\r")?;
    m.s.prompt()?;
    m.poweroff()?;
    let missing = unimplemented_lines(&text);
    if !missing.is_empty() {
        failed.push(format!("system calls Poly does not implement: {missing:?}"));
    }
    if vlc {
        match check_recording(&m.wav) {
            Ok(n) => eprintln!("==> VLC's {MEDIA_HZ} Hz tone heard in {n} windows of 100 ms"),
            Err(e) => failed.push(format!("VLC's sound: {e}")),
        }
        match check_frames(&image) {
            Ok(n) => eprintln!("==> VLC's pictures: {n} frames, red, green and blue in order"),
            Err(e) => failed.push(format!("VLC's pictures: {e}")),
        }
    }
    if failed.is_empty() { Ok(()) } else { Err(format!("toolkits on Poly: {}", failed.join("; "))) }
}

/// The recording holds the video's tone: at least two of its three
/// seconds of 100 ms windows at [`MEDIA_HZ`] (within 2 %) on both
/// channels, not quiet. Returns how many.
fn check_recording(wav: &Path) -> Result<usize> {
    let data = std::fs::read(wav).map_err(|e| format!("{}: {e}", wav.display()))?;
    let w = crate::sound::parse_wav(&data)?;
    let n = tone_windows(&crate::sound::windows(&w, 0.1));
    if n >= 20 { Ok(n) } else { Err(format!("the tone is in only {n} windows of 100 ms (want 20)")) }
}

/// Windows of the tone at a level that is clearly not silence.
fn tone_windows(ws: &[crate::sound::Window]) -> usize {
    ws.iter().filter(|w| (w.hz - MEDIA_HZ).abs() <= MEDIA_HZ * 0.02 && w.rms.iter().all(|&r| r > 300.0)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_parse() {
        assert_eq!(Step::parse("run:ls -l /").unwrap(), Step::Run("ls -l /".into()));
        assert_eq!(Step::parse("click: 10, 20").unwrap(), Step::Click(10, 20));
        assert_eq!(Step::parse("type:4 hello").unwrap(), Step::Type("4 hello".into()));
        assert_eq!(Step::parse("sleep:3").unwrap(), Step::Sleep(3));
        assert!(Step::parse("type:Hello").is_err());
        assert!(Step::parse("click:1").is_err());
        assert!(Step::parse("nothing").is_err());
        assert!(Step::parse("dance:1").is_err());
    }

    #[test]
    fn reads_window_lines() {
        assert_eq!(window_place("\"Poly Qt\" shown at 460,302 size 360x240"), Some((460, 302, 360, 240)));
        assert_eq!(window_place("\"x\" shown at a,1 size 1x1"), None);
    }

    #[test]
    fn finds_colours() {
        let mut img = screen::Image { width: 8, height: 6, rgb: vec![0; 8 * 6 * 3] };
        for (x, y) in [(2, 1), (5, 3), (3, 2)] {
            let i = (y * 8 + x) * 3;
            img.rgb[i..i + 3].copy_from_slice(&[0x2a, 0x6f, 0xb2]);
        }
        assert_eq!(find_colour(&img, [0x2a, 0x6f, 0xb0], 3, (0, 0, 8, 6)), Some(((2, 1, 5, 3), 3)));
        assert_eq!(find_colour(&img, [0x2a, 0x6f, 0xb0], 1, (0, 0, 8, 6)), None);
        // Only inside the rectangle, clipped to the image.
        assert_eq!(find_colour(&img, [0x2a, 0x6f, 0xb0], 3, (4, -2, 10, 10)), Some(((5, 3, 5, 3), 1)));
    }

    #[test]
    fn hears_the_tone() {
        let rate = 44100;
        let samples: Vec<i16> = (0..rate * 3)
            .flat_map(|i| {
                let v = (4096.0 * (2.0 * std::f64::consts::PI * MEDIA_HZ * i as f64 / rate as f64).sin()) as i16;
                [v, v]
            })
            .collect();
        let w = crate::sound::Wav { rate: rate as u32, channels: 2, samples };
        assert_eq!(tone_windows(&crate::sound::windows(&w, 0.1)), 30);
        let quiet = crate::sound::Wav { rate: 44100, channels: 2, samples: vec![0; 44100 * 2] };
        assert_eq!(tone_windows(&crate::sound::windows(&quiet, 0.1)), 0);
    }

    /// A YUV4MPEG2 stream of `frames` (Y, U, V) solid pictures.
    fn y4m(w: usize, h: usize, frames: &[(u8, u8, u8)]) -> Vec<u8> {
        let mut out = format!("YUV4MPEG2 W{w} H{h} F15:1 Ip A1:1 C420jpeg\n").into_bytes();
        for &(y, u, v) in frames {
            out.extend_from_slice(b"FRAME\n");
            out.extend(std::iter::repeat_n(y, w * h));
            out.extend(std::iter::repeat_n(u, w.div_ceil(2) * h.div_ceil(2)));
            out.extend(std::iter::repeat_n(v, w.div_ceil(2) * h.div_ceil(2)));
        }
        out
    }

    #[test]
    fn reads_vlcs_pictures() {
        // BT.601 limited range: red, green, blue, and a grey.
        let (red, green, blue, grey) = ((81, 90, 240), (145, 54, 34), (41, 240, 110), (126, 128, 128));
        let data = y4m(6, 4, &[grey, red, red, green, blue, blue, grey]);
        let (w, h, colours) = y4m_colours(&data).unwrap();
        assert_eq!((w, h, colours.len()), (6, 4, 7));
        assert!(near(colours[1], [0xff, 0, 0], 8), "{:?}", colours[1]);
        assert!(near(colours[3], [0, 0xff, 0], 8), "{:?}", colours[3]);
        assert!(near(colours[4], [0, 0, 0xff], 8), "{:?}", colours[4]);
        assert_eq!(colour_runs(&colours), [0, 1, 2]);
        assert_eq!(colour_runs(&y4m_colours(&y4m(2, 2, &[blue, red])).unwrap().2), [2, 0]);
        // Not YUV4MPEG2, not 4:2:0, cut short.
        assert!(y4m_colours(b"P6\n1 1\n").is_err());
        assert!(y4m_colours(b"YUV4MPEG2 W2 H2 C444\nFRAME\n").is_err());
        assert!(y4m_colours(&data[..data.len() - 1]).is_err());
    }

    #[test]
    fn the_inittab_starts_the_desktop_and_a_shell() {
        assert!(INITTAB.contains("::once:/sbin/chroot /mnt/root /usr/local/sbin/poly-desktop"));
        assert!(INITTAB.contains("::respawn:/bin/cttyhack /sbin/chroot /mnt/root /bin/bash -l"));
        assert!(DESKTOP_SCRIPT.contains(". /etc/poly-desktop.env"));
        for var in ["XDG_RUNTIME_DIR=", "WAYLAND_DISPLAY=", "GDK_BACKEND=wayland", "QT_QPA_PLATFORM=wayland"] {
            assert!(DESKTOP_ENV.contains(var), "{var}");
        }
    }
}
