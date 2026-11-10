//! `poly-installer-app`: Poly's installer (ADR 0132: one installer for
//! every package format; ADR 0173: `.deb`), a native Poly program and a
//! Wayland client of the compositor (`poly-app`).
//!
//! ```text
//! poly-installer FILE             ask the desktop to open the installer for FILE
//! poly-installer --window FILE    the installer's window (what the desktop starts)
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
//! [`view`] is the window; [`Native`] is what touches the system (the
//! package tools as child processes with their output on pipes, dpkg's
//! database, the control pipe).

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod view;

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use poly_pkg::installer::Installer;
use poly_pkg::{apt, deb, dpkgdb, format};
use poly_rt::fs::File;
use poly_rt::io::Read;
use poly_rt::process::{Child, Command, Stdio};
use poly_wayland::launch::Control;

/// The argument the desktop starts the window with
/// (`poly_shell::apps::installer_entry`).
pub const WINDOW_FLAG: &str = poly_shell::apps::INSTALLER_WINDOW;

/// What the program was asked: the package file, and whether to be the
/// window (`--window`) or to ask the desktop for it. `None`: no file.
pub fn parse_args<'a>(args: impl IntoIterator<Item = &'a str>) -> Option<(bool, &'a str)> {
    let mut window = false;
    let mut file = None;
    for arg in args {
        if arg == WINDOW_FLAG {
            window = true;
        } else if file.is_none() && !arg.is_empty() {
            // A `file://` URL (a desktop file's `%u`) is its path.
            file = Some(arg.strip_prefix("file://").unwrap_or(arg));
        }
    }
    file.map(|f| (window, f))
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

/// The installer for `path`, with what the base root has.
fn model_for(path: &str) -> Installer {
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

/// The disk, for the icon theme's files.
#[derive(Clone, Copy, Debug, Default)]
struct Disk;

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

/// The system behind the window: the package tools as children, their
/// output and progress on pipes.
pub struct Native {
    running: Option<Running>,
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
        out
    }

    fn pump(&mut self) -> view::Pumped {
        let mut pumped = view::Pumped::default();
        Native::drain(&mut self.status_read, &mut self.status_pending);
        pumped.status = take_lines(&mut self.status_pending);
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

/// The program: the window until it is closed.
pub fn main() -> i32 {
    let Some((window, arg)) = parse_args(poly_rt::env::args_str().skip(1)) else {
        poly_rt::println!("usage: poly-installer [--window] FILE");
        return 2;
    };
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
    poly_rt::println!(
        "poly-installer: {path}: {} {} ({:?})",
        model.name(),
        model.package.as_ref().map_or("", |p| p.version.as_str()),
        model.phase
    );
    let mut app = view::InstallerApp::new(model, Box::new(host));
    app.set_icon_files(Box::new(Disk));
    let title = app.title();
    let code = poly_app::run(&mut app, &title, "poly-installer", view::SIZE, view::MIN_SIZE);
    poly_rt::println!("poly-installer: {}: closed ({:?})", app.model.name(), app.model.phase);
    code
}

#[cfg(test)]
mod tests;
