//! `poly-installer-app`: Poly's installer (ADR 0132: one installer for
//! every package format; ADR 0173: `.deb`), a native Poly program and a
//! Wayland client of the compositor (`poly-app`).
//!
//! ```text
//! poly-installer FILE                       ask the desktop to open the installer for FILE
//! poly-installer --window FILE              the installer's window (what the desktop starts)
//! poly-installer --uninstall ID             ask the desktop for the window that uninstalls an app
//! poly-installer --window --uninstall ID    that window (ADR 0180)
//! ```
//!
//! It is what opens a package file: Files starts it for a double-clicked
//! `.deb`, and `xdg-open` (Chromium's "open" of a download) starts it
//! through its desktop file (`poly-installer.desktop`, the default app for
//! `application/vnd.debian.binary-package`). The window says what the
//! package is (read from the file itself: `poly-pkg`), **Install** runs the
//! distribution's own `apt-get` and `dpkg` in the Debian base root, and when
//! they are done the desktop reads its apps again, so the new one is in the
//! launcher and in Files' Applications.
//!
//! **Who installs.** Opening a package is the desktop's job, as opening an
//! app is (ADR 0175): whoever runs `poly-installer FILE` (Files, or
//! Chromium, which runs as user `poly`) only asks the desktop, through the
//! compositor's control pipe (`install <path>`), and ends. The desktop
//! starts the window as its own user, who is the one that installs (root on
//! today's disks); the click on **Install** there is the consent. Without a
//! desktop to ask, the window opens from where it was started.
//!
//! **Windows programs** (ADR 0230): a `.exe` opens here too. The window says
//! what it is from its version resource; an installer's **Install** runs it
//! on the NT personality with its own windows, waits for it and for every
//! process it started (their parents in `/proc`), then makes launcher
//! entries of the shortcuts it left in the Start menu and on the desktop,
//! with the app's `Uninstall` key from the registry. A plain program gets
//! **Run**.
//!
//! [`view`] is the window; [`Native`] is what touches the system (the
//! package tools as child processes with their output on pipes, dpkg's
//! database, the control pipe).

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod view;

use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use poly_pkg::installer::Installer;
use poly_pkg::windows::apps::{self as winapps, Seen, Uninstall};
use poly_pkg::windows::program;
use poly_pkg::{apt, deb, dpkgdb, format};
use poly_rt::fs::File;
use poly_rt::io::Read;
use poly_rt::process::{Child, Command, Stdio};
use poly_wayland::launch::Control;

/// The argument the desktop starts the window with
/// (`poly_shell::apps::installer_entry`).
pub const WINDOW_FLAG: &str = poly_shell::apps::INSTALLER_WINDOW;

/// The argument that makes the window uninstall an app: the desktop file
/// ID follows (`poly_shell::apps::uninstaller_entry`).
pub const UNINSTALL_FLAG: &str = poly_shell::apps::INSTALLER_UNINSTALL;

/// What the program was asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request<'a> {
    /// Install the package file at this path.
    Install(&'a str),
    /// Uninstall the app with this desktop file ID.
    Uninstall(&'a str),
}

/// What the program was asked, and whether to be the window (`--window`) or
/// to ask the desktop for it. `None`: nothing to do.
pub fn parse_args<'a>(args: impl IntoIterator<Item = &'a str>) -> Option<(bool, Request<'a>)> {
    let mut window = false;
    let mut uninstall = false;
    let mut file = None;
    for arg in args {
        if arg == WINDOW_FLAG {
            window = true;
        } else if arg == UNINSTALL_FLAG {
            uninstall = true;
        } else if file.is_none() && !arg.is_empty() {
            // A `file://` URL (a desktop file's `%u`) is its path.
            file = Some(arg.strip_prefix("file://").unwrap_or(arg));
        }
    }
    file.map(|f| (window, if uninstall { Request::Uninstall(f) } else { Request::Install(f) }))
}

/// `path` as an absolute path: itself, or below the directory in `pwd`
/// (the environment's `PWD`, which the shells and `xdg-open` keep).
pub fn absolute(path: &str, pwd: Option<&str>) -> String {
    if path.starts_with('/') {
        return path.to_string();
    }
    let dir = pwd.filter(|d| d.starts_with('/')).unwrap_or("/");
    let path = path.strip_prefix("./").unwrap_or(path);
    alloc::format!("{}/{path}", dir.trim_end_matches('/'))
}

/// The lines in `buf` that are complete, taken out of it (without their
/// newlines; a carriage return ends a line too: `apt` rewrites its progress
/// line with them).
pub fn take_lines(buf: &mut Vec<u8>) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(end) = buf.iter().position(|&b| b == b'\n' || b == b'\r') {
        let line: Vec<u8> = buf.drain(..=end).collect();
        let text = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
        if !text.trim().is_empty() {
            out.push(text);
        }
    }
    out
}

/// The first bytes of the file at `path` (at most `len`).
fn read_head(file: &mut File, len: usize) -> Result<Vec<u8>, poly_rt::Error> {
    let mut buf = alloc::vec![0u8; len];
    let mut got = 0;
    while got < len {
        match file.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(poly_rt::Error::Interrupted) => {}
            Err(e) => return Err(e),
        }
    }
    buf.truncate(got);
    Ok(buf)
}

/// What was read of a package file: its first bytes and, for a `.deb`,
/// its control member.
fn read_package(path: &str) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    let mut file = File::open(path).map_err(|e| alloc::format!("{e}"))?;
    let head = read_head(&mut file, deb::HEAD.max(format::HEAD)).map_err(|e| alloc::format!("{e}"))?;
    let control = deb::control_member(&head).ok().and_then(|m| {
        file.seek(m.offset as i64, poly_rt::abi::Whence::Start).ok()?;
        let data = read_head(&mut file, m.size as usize).ok()?;
        (data.len() as u64 == m.size).then_some(data)
    });
    Ok((head, control))
}

/// What was read of the AppImage at `path` (its first bytes are `head`):
/// its desktop file and icon, from the image inside it.
fn read_image(path: &str, head: &[u8]) -> Result<poly_pkg::appimage::Image, poly_pkg::Error> {
    let size = poly_rt::fs::metadata(path).map_err(|_| poly_pkg::Error::Truncated)?.len();
    let owned = path.to_string();
    poly_pkg::appimage::read(head, size, move |at, buf| {
        let Ok(mut file) = File::open(&owned) else { return false };
        if file.seek(at as i64, poly_rt::abi::Whence::Start).is_err() {
            return false;
        }
        let mut got = 0;
        while got < buf.len() {
            match file.read(&mut buf[got..]) {
                Ok(0) => return false,
                Ok(n) => got += n,
                Err(poly_rt::Error::Interrupted) => {}
                Err(_) => return false,
            }
        }
        true
    })
}

/// `from` copied to `to`, a megabyte at a time.
/// Whether an AppImage is started by extracting it: only on a system
/// without FUSE; otherwise its runtime mounts the image (ADR 0180).
fn extracting() -> bool {
    poly_pkg::appimage::runs_by_extracting(poly_rt::fs::metadata(poly_pkg::appimage::FUSE_DEVICE).is_ok())
}

fn copy_file(from: &str, to: &str) -> Result<(), String> {
    let mut src = File::open(from).map_err(|e| alloc::format!("{from}: {e}"))?;
    let dst = File::create(to).map_err(|e| alloc::format!("{to}: {e}"))?;
    let mut buf = alloc::vec![0u8; 1 << 20];
    loop {
        match src.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => dst.write_all(&buf[..n]).map_err(|e| alloc::format!("{to}: {e}"))?,
            Err(poly_rt::Error::Interrupted) => {}
            Err(e) => return Err(alloc::format!("{from}: {e}")),
        }
    }
}

/// A reader of the file at `path` at any offset, for `poly_pkg::windows`.
fn reader_of(path: &str) -> Option<impl FnMut(u64, &mut [u8]) -> usize> {
    let mut file = File::open(path).ok()?;
    Some(move |at: u64, buf: &mut [u8]| {
        if file.seek(at as i64, poly_rt::abi::Whence::Start).is_err() {
            return 0;
        }
        loop {
            match file.read(buf) {
                Ok(n) => return n,
                Err(poly_rt::Error::Interrupted) => {}
                Err(_) => return 0,
            }
        }
    })
}

/// What the Windows program at `path` says about itself (ADR 0230).
fn read_windows(path: &str) -> Result<program::Program, poly_pkg::Error> {
    let size = poly_rt::fs::metadata(path).map_err(|_| poly_pkg::Error::Truncated)?.len();
    let mut read = reader_of(path).ok_or(poly_pkg::Error::Truncated)?;
    program::read_program(path, size, &mut read)
}

/// The installer for `path`, with what the base root has.
fn model_for(path: &str) -> Installer {
    if let Ok((head, _)) = read_package(path) {
        match format::detect(&head) {
            Some(format::Format::AppImage) => return Installer::for_image(path, read_image(path, &head)),
            Some(format::Format::WindowsProgram) => return Installer::for_windows(path, read_windows(path)),
            Some(format::Format::WindowsInstaller) if !reader_of(path).is_some_and(|mut r| program::is_msi(&mut r)) => {
                return Installer::new(path, Ok(b""), None);
            }
            _ => {}
        }
    }
    let mut model = match read_package(path) {
        Ok((head, control)) => Installer::new(path, Ok(&head), control.as_deref()),
        Err(why) => Installer::new(path, Err(&why), None),
    };
    let status = poly_rt::fs::read_to_string(apt::DPKG_STATUS).unwrap_or_default();
    let installed = model.package.as_ref().and_then(|p| dpkgdb::installed_version(&status, &p.name));
    let have_apt = poly_rt::fs::metadata(apt::APT_GET).is_ok_and(|m| m.is_file()) && !status.is_empty();
    let have_lists =
        poly_rt::fs::read_dir(apt::LISTS_DIR).is_ok_and(|names| names.iter().any(|e| apt::is_package_list(&e.name)));
    model.base_root(installed, have_apt, have_lists, dpkgdb::unconfigured(&status));
    model
}

/// The user's data directory, where AppImages are (ADR 0180).
fn data_home() -> String {
    poly_shell::apps::data_home(poly_rt::env::var("XDG_DATA_HOME"), poly_rt::env::var("HOME"))
}

/// The disk, for the icon theme's files and for what an uninstall reads.
#[derive(Clone, Copy, Debug, Default)]
struct Disk;

impl poly_pkg::remove::Disk for Disk {
    fn list(&self, dir: &str) -> Vec<String> {
        poly_rt::fs::read_dir(dir).map(|entries| entries.into_iter().map(|e| e.name).collect()).unwrap_or_default()
    }

    fn read(&self, path: &str) -> Option<String> {
        poly_rt::fs::read_to_string(path).ok()
    }

    fn size(&self, path: &str) -> Option<u64> {
        poly_rt::fs::metadata(path).ok().map(|m| m.len())
    }
}

/// The installer for uninstalling the app with desktop file ID `id`, and
/// its icon's name, from what the disk says about it.
fn model_for_uninstall(id: &str) -> (Installer, Option<String>) {
    let plan = poly_pkg::remove::plan(id, &data_home(), &Disk);
    let icon = plan.as_ref().ok().and_then(|plan| {
        plan.apps.iter().find(|(i, _)| i == id).or(plan.apps.first()).and_then(|(i, _)| {
            let user = alloc::format!("{}/{i}", poly_shell::apps::user_applications(&data_home()));
            let text = poly_rt::fs::read_to_string(&user)
                .or_else(|_| poly_rt::fs::read_to_string(&alloc::format!("{}/{i}", poly_shell::apps::APPLICATIONS)))
                .ok()?;
            poly_shell::apps::parse_desktop_entry(i, &text).ok()?.icon
        })
    });
    let refused = plan.is_err();
    let mut model = Installer::for_removal(id, plan);
    if refused {
        // The window still says which app it is about.
        let user = alloc::format!("{}/{id}", poly_shell::apps::user_applications(&data_home()));
        let text = poly_rt::fs::read_to_string(&user)
            .or_else(|_| poly_rt::fs::read_to_string(&alloc::format!("{}/{id}", poly_shell::apps::APPLICATIONS)))
            .ok();
        model.title = text.and_then(|t| {
            poly_pkg::appimage::desktop_keys(&t).into_iter().find(|(k, _)| *k == "Name").map(|(_, v)| v.to_string())
        });
    }
    (model, icon)
}

impl poly_shell::appicon::IconFiles for Disk {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let meta = poly_rt::fs::metadata(path).ok()?;
        if !meta.is_file() || meta.len() > poly_shell::appicon::MAX_FILE as u64 {
            return None;
        }
        poly_rt::fs::read(path).ok()
    }

    fn list(&self, dir: &str) -> Vec<String> {
        poly_rt::fs::read_dir(dir).map(|entries| entries.into_iter().map(|e| e.name).collect()).unwrap_or_default()
    }
}

/// A running command: the child and the read end of its output pipe.
struct Running {
    child: Child,
    output: File,
    pending: Vec<u8>,
}

/// A running Windows installer or uninstaller (ADR 0230): the program, its
/// output, and the processes it started.
struct WindowsRun {
    child: Child,
    output: File,
    pending: Vec<u8>,
    /// The program's pid and every process seen below it.
    tree: BTreeSet<u32>,
    /// It has ended: whether with exit code 0.
    ended: Option<bool>,
    /// The others still running, as last told.
    told: Option<usize>,
    /// Waiting was stopped.
    stop: bool,
    /// Started through `polyopen.exe`, which waits for everything the program started
    /// itself (a job object) and says so: the window waits for it alone.
    opener: bool,
}

/// Make the file at `path` executable if it is not: Poly's exec wants the
/// bit, Windows has no such thing, and a download comes without it.
fn make_executable(path: &str) -> Result<(), String> {
    let executable = poly_rt::fs::metadata(path).is_ok_and(|m| m.mode() & 0o111 != 0);
    if !executable {
        poly_rt::fs::set_mode(path, 0o755).map_err(|e| alloc::format!("{path}: {e}"))?;
    }
    Ok(())
}

/// How a Windows program is started from the window: through
/// `polyopen.exe` (ADR 0230), which opens it as the Windows shell does
/// (`ShellExecuteExW`): its manifest's `requestedExecutionLevel` is read and
/// the consent prompt asked first (ADR 0222). `argv[0]` is the program (a
/// path of Poly's file system: the kernel hands it on as its DOS path). On a
/// disk without `polyopen.exe` the program is started itself.
fn opened(argv: &[String]) -> Result<(Vec<String>, bool), String> {
    let program = argv.first().ok_or("nothing to run")?;
    match winapps::vfs_path(winapps::POLYOPEN, &VfsFiles).filter(|p| poly_rt::fs::exists(p)) {
        Some(opener) => {
            make_executable(&opener)?;
            Ok((core::iter::once(opener).chain(argv.iter().cloned()).collect(), true))
        }
        None => {
            make_executable(program)?;
            Ok((argv.to_vec(), false))
        }
    }
}

/// The file system as `poly_pkg::windows::apps` reads it.
struct VfsFiles;

impl winapps::Files for VfsFiles {
    fn list(&self, dir: &str) -> Vec<String> {
        poly_rt::fs::read_dir(dir).map(|e| e.into_iter().map(|e| e.name).collect()).unwrap_or_default()
    }

    fn is_dir(&self, path: &str) -> bool {
        poly_rt::fs::metadata(path).is_ok_and(|m| m.is_dir())
    }

    fn exists(&self, path: &str) -> bool {
        poly_rt::fs::exists(path)
    }
}

/// The processes there are: pid, parent's pid, whether it has ended (a
/// zombie), from `/proc/<pid>/status`.
fn processes() -> Vec<(u32, u32, bool)> {
    let mut out = Vec::new();
    for e in poly_rt::fs::read_dir("/proc").unwrap_or_default() {
        let Ok(pid) = e.name.parse::<u32>() else { continue };
        let Ok(status) = poly_rt::fs::read_to_string(&alloc::format!("/proc/{pid}/status")) else { continue };
        let field = |name: &str| status.lines().find_map(|l| l.strip_prefix(name)).map(str::trim);
        let ppid = field("PPid:").and_then(|v| v.parse().ok()).unwrap_or(0);
        let ended = field("State:").is_some_and(|v| v.starts_with('Z') || v.starts_with('X'));
        out.push((pid, ppid, ended));
    }
    out
}

/// Add to `tree` the processes whose parent is in it; how many of the tree
/// other than `main` still run.
fn grow(tree: &mut BTreeSet<u32>, main: u32, procs: &[(u32, u32, bool)]) -> usize {
    loop {
        let before = tree.len();
        for &(pid, ppid, _) in procs {
            if ppid != 0 && tree.contains(&ppid) {
                tree.insert(pid);
            }
        }
        if tree.len() == before {
            break;
        }
    }
    let me = poly_rt::process::id();
    procs.iter().filter(|(pid, _, ended)| *pid != main && *pid != me && !*ended && tree.contains(pid)).count()
}

/// The shortcut files in the folders an installer puts them in, with a
/// checksum of each (sub-folders of the Start menu too).
fn scan_links() -> Vec<Seen> {
    fn walk(dir: &str, depth: usize, out: &mut Vec<Seen>) {
        for e in poly_rt::fs::read_dir(dir).unwrap_or_default() {
            let path = alloc::format!("{dir}/{}", e.name);
            if winapps::is_link_name(&e.name) {
                if let Ok(bytes) = poly_rt::fs::read(&path)
                    && bytes.len() < 1 << 20
                {
                    out.push(Seen { path, sum: winapps::checksum(&bytes) });
                }
            } else if depth < 4 && poly_rt::fs::metadata(&path).is_ok_and(|m| m.is_dir()) {
                walk(&path, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    for folder in winapps::link_folders() {
        if let Some(dir) = winapps::vfs_path(&folder, &VfsFiles) {
            walk(&dir, 0, &mut out);
        }
    }
    out
}

/// The registry's hive files (ADR 0066): next to `C:` on the disk that keeps
/// them; where `C:` is in memory only, there are none to read.
fn hive_dir() -> Option<String> {
    let mut candidates = alloc::vec![winapps::HIVE_DIR.to_string(), "/poly/nt/registry".to_string()];
    for e in poly_rt::fs::read_dir("/").unwrap_or_default() {
        candidates.push(alloc::format!("/{}/poly/nt/registry", e.name));
    }
    candidates.into_iter().find(|d| poly_rt::fs::exists(&alloc::format!("{d}/{}", winapps::SOFTWARE_HIVE)))
}

/// The `Uninstall` keys in the registry as its hive files have them.
fn uninstall_keys() -> Vec<Uninstall> {
    let Some(dir) = hive_dir() else { return Vec::new() };
    let software = poly_rt::fs::read(&alloc::format!("{dir}/{}", winapps::SOFTWARE_HIVE)).ok();
    let user = poly_rt::fs::read(&alloc::format!("{dir}/{}", winapps::USER_HIVE)).ok();
    winapps::uninstall_entries(software.as_deref(), user.as_deref())
}

/// The system behind the window: the package tools as children, their
/// output and progress on pipes.
pub struct Native {
    running: Option<Running>,
    /// A Windows installer or uninstaller that runs (ADR 0230).
    windows: Option<WindowsRun>,
    /// The shortcuts and `Uninstall` keys there were before it started.
    before: (Vec<Seen>, Vec<Uninstall>),
    /// The progress pipe, kept for the program's life: the commands inherit
    /// its write end under its own number ([`Installer::status_fd`]).
    status_read: File,
    status_write: File,
    status_pending: Vec<u8>,
}

impl core::fmt::Debug for Native {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Native").field("running", &self.running.is_some()).finish_non_exhaustive()
    }
}

impl Native {
    pub fn new() -> Result<Native, poly_rt::Error> {
        let [read, write] = poly_rt::sys::pipe()?;
        poly_rt::sys::set_nonblocking(read, true)?;
        Ok(Native {
            running: None,
            windows: None,
            before: (Vec::new(), Vec::new()),
            status_read: File::from_handle(read),
            status_write: File::from_handle(write),
            status_pending: Vec::new(),
        })
    }

    /// The descriptor number the commands write their progress to.
    pub fn status_fd(&self) -> u32 {
        self.status_write.handle()
    }

    /// Everything `file` has to read now, appended to `into`; `true` at
    /// the end of the stream.
    fn drain(file: &mut File, into: &mut Vec<u8>) -> bool {
        let mut buf = [0u8; 4096];
        loop {
            match file.read(&mut buf) {
                Ok(0) => return true,
                Ok(n) => into.extend_from_slice(&buf[..n]),
                Err(poly_rt::Error::Interrupted) => {}
                Err(_) => return false,
            }
        }
    }
}

impl Native {
    /// A look at the running Windows program and what it started.
    fn pump_windows(&mut self, mut pumped: view::Pumped) -> view::Pumped {
        let Some(w) = &mut self.windows else { return pumped };
        Native::drain(&mut w.output, &mut w.pending);
        pumped.output = take_lines(&mut w.pending);
        if w.ended.is_none() {
            match w.child.try_wait() {
                Ok(Some(status)) => w.ended = Some(matches!(status, poly_rt::abi::ExitStatus::Exited(0))),
                Ok(None) => {}
                Err(_) => w.ended = Some(false),
            }
        }
        if w.opener {
            // polyopen.exe counts what the program started, exactly (its job).
            if let Some(n) = pumped.output.iter().rev().find_map(|l| poly_pkg::installer::open_waiting(l))
                && w.told != Some(n)
            {
                w.told = Some(n);
                pumped.waiting = Some(n);
            }
            if w.ended.is_none() && !w.stop {
                return pumped;
            }
            if !w.pending.is_empty() {
                pumped.output.push(String::from_utf8_lossy(&w.pending).into_owned());
            }
            pumped.ended = Some(w.ended.unwrap_or(true));
            self.windows = None;
            return pumped;
        }
        let main = w.child.id();
        let others = grow(&mut w.tree, main, &processes());
        let Some(ok) = w.ended else { return pumped };
        if others > 0 && !w.stop {
            if w.told != Some(others) {
                w.told = Some(others);
                pumped.waiting = Some(others);
            }
            return pumped;
        }
        if !w.pending.is_empty() {
            pumped.output.push(String::from_utf8_lossy(&w.pending).into_owned());
        }
        pumped.ended = Some(ok);
        self.windows = None;
        pumped
    }

    /// The icon of a launcher entry: the program's own (its PE icon
    /// resources), written as a PNG file where the entry's icons go.
    fn write_icon(&self, entry: &winapps::Entry) -> Option<String> {
        let (file, index) = &entry.icon_source;
        let mut read = reader_of(file)?;
        let mut image = program::Image::open(&mut read).ok()?;
        let png = image.icon(*index)?;
        let path = winapps::icon_path(&data_home(), &entry.id);
        let dir = path.rsplit_once('/').map_or("/", |(d, _)| d);
        poly_rt::fs::create_dir_all(dir).ok()?;
        poly_rt::fs::write(&path, &png).ok()?;
        Some(path)
    }
}

impl view::Host for Native {
    fn start(&mut self, argv: &[String]) -> Result<(), String> {
        let program = argv.first().ok_or("nothing to run")?;
        let [read, write] = poly_rt::sys::pipe().map_err(|e| alloc::format!("a pipe: {e}"))?;
        let (read, write) = (File::from_handle(read), File::from_handle(write));
        poly_rt::sys::set_nonblocking(read.handle(), true).map_err(|e| alloc::format!("a pipe: {e}"))?;
        let mut cmd = Command::new(program);
        cmd.args(argv.iter().skip(1).map(String::as_str))
            .stdin(Stdio::Null)
            .stdout(Stdio::Handle(write.handle()))
            .stderr(Stdio::Handle(write.handle()))
            .keep(self.status_write.handle());
        for (name, value) in apt::ENVIRONMENT {
            cmd.env(name, value);
        }
        let child = cmd.spawn().map_err(|e| alloc::format!("{program}: {e}"))?;
        // Our copy of the write end closes here: the pipe ends with the
        // command and what it started.
        drop(write);
        self.running = Some(Running { child, output: read, pending: Vec::new() });
        Ok(())
    }

    fn handles(&self) -> Vec<(u32, u16)> {
        let mut out = alloc::vec![(self.status_read.handle(), poly_rt::poll::READABLE)];
        if let Some(r) = &self.running {
            out.push((r.output.handle(), poly_rt::poll::READABLE));
        }
        if let Some(w) = &self.windows {
            out.push((w.output.handle(), poly_rt::poll::READABLE));
        }
        out
    }

    fn pump(&mut self) -> view::Pumped {
        let mut pumped = view::Pumped::default();
        Native::drain(&mut self.status_read, &mut self.status_pending);
        pumped.status = take_lines(&mut self.status_pending);
        if self.windows.is_some() {
            return self.pump_windows(pumped);
        }
        let Some(r) = &mut self.running else { return pumped };
        let ended = Native::drain(&mut r.output, &mut r.pending);
        pumped.output = take_lines(&mut r.pending);
        // The command is over when it has exited; what it left in the pipe
        // was read above (a daemon it started may keep the pipe open).
        match r.child.try_wait() {
            Ok(Some(status)) => {
                if !ended {
                    Native::drain(&mut r.output, &mut r.pending);
                    pumped.output.append(&mut take_lines(&mut r.pending));
                }
                if !r.pending.is_empty() {
                    pumped.output.push(String::from_utf8_lossy(&r.pending).into_owned());
                }
                pumped.ended = Some(matches!(status, poly_rt::abi::ExitStatus::Exited(0)));
                self.running = None;
            }
            Ok(None) => {}
            Err(_) => {
                pumped.ended = Some(false);
                self.running = None;
            }
        }
        pumped
    }

    fn apps_of(&mut self, name: &str) -> Vec<(String, String, Option<String>)> {
        // The package's file list: `<name>.list`, or with its architecture
        // for a `Multi-Arch: same` package.
        let lists = poly_rt::fs::read_dir(apt::DPKG_INFO).unwrap_or_default();
        let prefix = alloc::format!("{name}:");
        let list = lists
            .iter()
            .map(|e| e.name.as_str())
            .find(|n| {
                n.strip_suffix(".list").is_some_and(|stem| {
                    stem == name || (stem.starts_with(&prefix) && !stem[prefix.len()..].contains('.'))
                })
            })
            .and_then(|n| poly_rt::fs::read_to_string(&alloc::format!("{}/{n}", apt::DPKG_INFO)).ok())
            .unwrap_or_default();
        dpkgdb::desktop_ids(&list)
            .into_iter()
            .filter_map(|id| {
                let text =
                    poly_rt::fs::read_to_string(&alloc::format!("{}/{id}", poly_shell::apps::APPLICATIONS)).ok()?;
                let app = poly_shell::apps::parse_desktop_entry(&id, &text).ok()?;
                Some((id, app.name, app.icon))
            })
            .collect()
    }

    fn place_image(&mut self, path: &str, image: &poly_pkg::appimage::Image) -> Result<view::Placed, String> {
        use poly_pkg::appimage;
        let layout = appimage::layout(&data_home(), &appimage::slug(&image.name));
        let dir_of = |p: &str| p.rsplit_once('/').map_or("/", |(d, _)| d).to_string();
        for dir in [layout.dir.clone(), dir_of(&layout.desktop)] {
            poly_rt::fs::create_dir_all(&dir).map_err(|e| alloc::format!("{dir}: {e}"))?;
        }
        // The copy under a name of its own, then in its place: a cut-off
        // copy is never the app.
        let part = alloc::format!("{}.part", layout.image);
        copy_file(path, &part)?;
        poly_rt::fs::set_mode(&part, 0o755).map_err(|e| alloc::format!("{part}: {e}"))?;
        poly_rt::fs::rename(&part, &layout.image).map_err(|e| alloc::format!("{}: {e}", layout.image))?;
        let icon = match &image.icon {
            Some(png) => {
                poly_rt::fs::write(&layout.icon, png).map_err(|e| alloc::format!("{}: {e}", layout.icon))?;
                Some(layout.icon.clone())
            }
            None => None,
        };
        let text = appimage::desktop_file(image, &layout, extracting());
        poly_rt::fs::write(&layout.desktop, text.as_bytes()).map_err(|e| alloc::format!("{}: {e}", layout.desktop))?;
        Ok(view::Placed { id: layout.id, icon })
    }

    fn run_image(&mut self, path: &str) -> Result<(), String> {
        // The AppImage specification's first step: it has to be executable.
        let executable = poly_rt::fs::metadata(path).is_ok_and(|m| m.mode() & 0o111 != 0);
        if !executable {
            poly_rt::fs::set_mode(path, 0o755).map_err(|e| alloc::format!("{path}: {e}"))?;
        }
        // The desktop shows a busy pointer until the app has a window.
        let mut startup = poly_app::startup::Startup::begin(path);
        let mut cmd = Command::new(path);
        cmd.arg0(path)
            .envs(
                poly_rt::env::vars()
                    .filter_map(|v| core::str::from_utf8(v).ok())
                    .filter(|v| !poly_app::startup::is_token_var(v)),
            )
            .envs(startup.environment().iter().map(String::as_str))
            .stdin(Stdio::Null);
        for (name, value) in poly_pkg::appimage::run_environment(extracting()) {
            cmd.env(name, value);
        }
        match cmd.spawn() {
            Ok(child) => {
                startup.started(child.id());
                Ok(())
            }
            Err(e) => {
                startup.failed();
                Err(alloc::format!("{e}"))
            }
        }
    }

    fn delete_image(&mut self, slug: &str) -> Result<(), String> {
        if !poly_pkg::appimage::is_slug(slug) {
            return Err(alloc::format!("{slug}: not an AppImage of Poly's installer"));
        }
        let layout = poly_pkg::appimage::layout(&data_home(), slug);
        // The desktop file first: the app is out of the launcher even if a
        // file of its directory cannot be removed.
        match poly_rt::fs::remove_file(&layout.desktop) {
            Ok(()) | Err(poly_rt::Error::NotFound) => {}
            Err(e) => return Err(alloc::format!("{}: {e}", layout.desktop)),
        }
        match poly_rt::fs::remove_dir_all(&layout.dir) {
            Ok(()) | Err(poly_rt::Error::NotFound) => Ok(()),
            Err(e) => Err(alloc::format!("{}: {e}", layout.dir)),
        }
    }

    fn squirrel_installer(&mut self) -> Option<String> {
        winapps::vfs_path(winapps::POLYSQUIRREL, &VfsFiles).filter(|p| poly_rt::fs::exists(p))
    }

    fn run_windows(&mut self, argv: &[String]) -> Result<(), String> {
        let (argv, opener) = opened(argv)?;
        self.before = (scan_links(), uninstall_keys());
        let [read, write] = poly_rt::sys::pipe().map_err(|e| alloc::format!("a pipe: {e}"))?;
        let (read, write) = (File::from_handle(read), File::from_handle(write));
        poly_rt::sys::set_nonblocking(read.handle(), true).map_err(|e| alloc::format!("a pipe: {e}"))?;
        let mut cmd = Command::new(&argv[0]);
        cmd.args(argv.iter().skip(1).map(String::as_str))
            .envs(
                poly_rt::env::vars()
                    .filter_map(|v| core::str::from_utf8(v).ok())
                    .filter(|v| !poly_app::startup::is_token_var(v)),
            )
            .stdin(Stdio::Null)
            .stdout(Stdio::Handle(write.handle()))
            .stderr(Stdio::Handle(write.handle()));
        let child = cmd.spawn().map_err(|e| alloc::format!("{e}"))?;
        drop(write);
        // The window itself is the tree's root: a process whose parent ended is adopted by
        // its nearest living ancestor (the kernel, ADR 0230), which may be this window.
        let mut tree = BTreeSet::new();
        tree.insert(poly_rt::process::id());
        tree.insert(child.id());
        self.windows = Some(WindowsRun {
            child,
            output: read,
            pending: Vec::new(),
            tree,
            ended: None,
            told: None,
            stop: false,
            opener,
        });
        Ok(())
    }

    fn stop_waiting(&mut self) {
        if let Some(w) = &mut self.windows {
            w.stop = true;
        }
    }

    fn collect_windows(&mut self) -> Vec<(String, String, Option<String>)> {
        let changed = winapps::changed(&self.before.0, &scan_links());
        let mut entries = Vec::new();
        for path in changed {
            let Ok(bytes) = poly_rt::fs::read(&path) else { continue };
            let made = winapps::entry_from_file(&path, &bytes, &VfsFiles);
            match made {
                Ok(e) => entries.push(e),
                Err(why) => self.report(&alloc::format!("shortcut left out: {why}")),
            }
        }
        let made = winapps::new_uninstall_entries(&self.before.1, uninstall_keys());
        for u in &made {
            self.report(&alloc::format!(
                "registered {} {} ({}) by {}: {}\\{}",
                u.display_name,
                u.display_version,
                u.uninstall_string,
                u.publisher,
                u.view,
                u.key
            ));
        }
        let apps_dir = poly_pkg::appimage::applications_dir(&data_home());
        let mut out = Vec::new();
        for e in winapps::dedup(entries) {
            // Files a Windows program writes have no execute bit; the
            // launcher's exec wants one.
            if let Err(why) = make_executable(&e.argv[0]) {
                self.report(&why);
            }
            let icon = self.write_icon(&e);
            let registration = winapps::registration_for(&e.target, &made);
            let text = winapps::desktop_file(&e, icon.as_deref(), registration);
            let path = alloc::format!("{apps_dir}/{}", e.id);
            let written =
                poly_rt::fs::create_dir_all(&apps_dir).and_then(|()| poly_rt::fs::write(&path, text.as_bytes()));
            match written {
                Ok(()) => {
                    self.report(&alloc::format!("launcher entry {} for {} from {}", e.id, e.target, e.link));
                    out.push((e.id, e.name, icon));
                }
                Err(err) => self.report(&alloc::format!("{path}: {err}")),
            }
        }
        out
    }

    fn forget_windows(&mut self, apps: &[(String, String)]) -> Vec<(String, String)> {
        let apps_dir = poly_pkg::appimage::applications_dir(&data_home());
        let mut left = Vec::new();
        for (id, name) in apps {
            if !winapps::is_windows_id(id) {
                continue;
            }
            let path = alloc::format!("{apps_dir}/{id}");
            let text = poly_rt::fs::read_to_string(&path).unwrap_or_default();
            let target = winapps::desktop_value(&text, winapps::TARGET_KEY).unwrap_or("");
            if winapps::vfs_path(target, &VfsFiles).is_some_and(|p| poly_rt::fs::exists(&p)) {
                left.push((id.clone(), name.clone()));
                continue;
            }
            let _ = poly_rt::fs::remove_file(&path);
            let _ = poly_rt::fs::remove_file(&winapps::icon_path(&data_home(), id));
        }
        left
    }

    fn run_program(&mut self, path: &str) -> Result<(), String> {
        let (argv, _) = opened(&[path.to_string()])?;
        // The desktop shows a busy pointer until the program has a window.
        let mut startup = poly_app::startup::Startup::begin(path);
        let mut cmd = Command::new(&argv[0]);
        cmd.args(argv.iter().skip(1).map(String::as_str))
            .envs(
                poly_rt::env::vars()
                    .filter_map(|v| core::str::from_utf8(v).ok())
                    .filter(|v| !poly_app::startup::is_token_var(v)),
            )
            .envs(startup.environment().iter().map(String::as_str))
            .stdin(Stdio::Null);
        match cmd.spawn() {
            Ok(child) => {
                startup.started(child.id());
                Ok(())
            }
            Err(e) => {
                startup.failed();
                Err(alloc::format!("{e}"))
            }
        }
    }

    fn apps_changed(&mut self) {
        poly_app::startup::ask(&Control::Rescan.line());
    }

    fn open(&mut self, id: &str) {
        poly_app::startup::ask(&Control::Open { id: id.to_string() }.line());
    }

    fn report(&mut self, line: &str) {
        poly_rt::println!("poly-installer: {line}");
    }
}

/// The window that uninstalls the app with desktop file ID `id`: asked of
/// the desktop unless this is the window the desktop started (ADR 0180).
fn uninstall_main(window: bool, id: &str) -> i32 {
    if !window && poly_app::startup::ask(&Control::Uninstall { id: id.to_string() }.line()) {
        if let Some(token) = poly_app::startup::TOKEN_VARS.iter().find_map(|v| poly_rt::env::var(v)) {
            poly_app::startup::ask(&Control::Cancel { token: token.into() }.line());
        }
        poly_rt::println!("poly-installer: {id}: handed to the desktop");
        return 0;
    }
    let (mut model, icon) = model_for_uninstall(id);
    let host = match Native::new() {
        Ok(host) => host,
        Err(e) => {
            poly_rt::println!("poly-installer: {e}");
            return 1;
        }
    };
    model.status_fd = host.status_fd();
    poly_rt::println!("poly-installer: uninstall {id}: {} ({:?})", model.name(), model.phase);
    let mut app = view::InstallerApp::new(model, Box::new(host));
    app.set_icon_files(Box::new(Disk));
    app.set_app_icon(icon);
    let title = app.title();
    let code = poly_app::run(&mut app, &title, "poly-installer", view::SIZE, view::MIN_SIZE);
    poly_rt::println!("poly-installer: {}: closed ({:?})", app.model.name(), app.model.phase);
    code
}

/// The program: the window until it is closed.
pub fn main() -> i32 {
    let Some((window, request)) = parse_args(poly_rt::env::args_str().skip(1)) else {
        poly_rt::println!("usage: poly-installer [--window] FILE | poly-installer [--window] --uninstall ID");
        return 2;
    };
    if let Request::Uninstall(id) = request {
        return uninstall_main(window, id);
    }
    let Request::Install(arg) = request else { return 2 };
    let path = absolute(arg, poly_rt::env::var("PWD"));
    // Opening a package is the desktop's job: ask it, and end.
    if !window && poly_app::startup::ask(&Control::Install { path: path.clone() }.line()) {
        // Whoever started this program (Files) announced a launch that waits
        // for its window (ADR 0167): there will be none, the desktop's own
        // launch of the window took its place.
        if let Some(token) = poly_app::startup::TOKEN_VARS.iter().find_map(|v| poly_rt::env::var(v)) {
            poly_app::startup::ask(&Control::Cancel { token: token.into() }.line());
        }
        poly_rt::println!("poly-installer: {path}: handed to the desktop");
        return 0;
    }
    let mut model = model_for(&path);
    let host = match Native::new() {
        Ok(host) => host,
        Err(e) => {
            poly_rt::println!("poly-installer: {e}");
            return 1;
        }
    };
    model.status_fd = host.status_fd();
    let version = match (&model.package, &model.program) {
        (Some(p), _) => p.version.as_str(),
        (None, Some(w)) => w.version.as_str(),
        _ => "",
    };
    poly_rt::println!("poly-installer: {path}: {} {version} ({:?})", model.name(), model.phase);
    if let Some(w) = &model.program {
        poly_rt::println!("poly-installer: {path}: {}, {}", w.kind(), w.publisher_text());
    }
    let mut app = view::InstallerApp::new(model, Box::new(host));
    app.set_icon_files(Box::new(Disk));
    let title = app.title();
    let code = poly_app::run(&mut app, &title, "poly-installer", view::SIZE, view::MIN_SIZE);
    poly_rt::println!("poly-installer: {}: closed ({:?})", app.model.name(), app.model.phase);
    code
}

#[cfg(test)]
mod tests;
