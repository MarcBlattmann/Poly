//! `poly-files-app`: Files, Poly's own file explorer (ADR 0134), a native
//! Poly program and a Wayland client of the compositor (`poly-app`), so a
//! crash never takes the desktop with it.
//!
//! ```text
//! poly-files [FOLDER]
//! ```
//!
//! The browsing core is `poly-files` (listing, sorting, search, selection,
//! the trash, file operations, which app opens a file), which the file
//! picker of Phase 8 reuses; this crate is the window ([`view`]) and the
//! native file system ([`native_fs`]). Files open with the app their
//! desktop entry's `MimeType=` names (`/usr/share/applications`,
//! `~/.local/share/applications`, the `mimeapps.list` files of the MIME
//! Applications Associations specification: the user's, then the system's in
//! `$XDG_CONFIG_DIRS`, as `xdg-open` reads them), programs and `.desktop`
//! files launch, folders open in Terminal from the menu.
//!
//! The Applications folder (ADR 0175) lists every installed app: the
//! launcher's own catalog (`poly_shell::apps::installed`, read from the same
//! disk by the same code), shown by [`applications`]. Opening one asks the
//! compositor to start it (`open <id>` on its control pipe), so it starts
//! exactly as from the launcher.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod applications;
pub mod native_fs;
pub mod view;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use poly_files::apps::Handler;

/// The folders desktop entries are read from (XDG data directories).
pub fn application_dirs(home: &str, data_home: Option<&str>) -> Vec<String> {
    let user = match data_home.filter(|d| d.starts_with('/')) {
        Some(d) => poly_files::path::join(d, "applications"),
        None => poly_files::path::join(home, ".local/share/applications"),
    };
    alloc::vec![user, "/usr/local/share/applications".into(), "/usr/share/applications".into()]
}

/// The `mimeapps.list` files to read, most important first (MIME
/// Applications Associations specification, "File locations", without the
/// desktop-specific `$desktop-mimeapps.list` of a desktop Poly is not):
/// `$XDG_CONFIG_HOME`, each of `$XDG_CONFIG_DIRS` (default `/etc/xdg`), then
/// the deprecated `applications/` of `$XDG_DATA_HOME` and each of
/// `$XDG_DATA_DIRS` (default `/usr/local/share:/usr/share`).
pub fn mimeapps_paths(
    config_home: &str,
    config_dirs: Option<&str>,
    data_home: &str,
    data_dirs: Option<&str>,
) -> Vec<String> {
    let dirs = |list: Option<&str>, default: &'static str| -> Vec<String> {
        list.filter(|l| !l.is_empty())
            .unwrap_or(default)
            .split(':')
            .filter(|d| d.starts_with('/'))
            .map(String::from)
            .collect()
    };
    let apps = |d: &str| poly_files::path::join(&poly_files::path::join(d, "applications"), "mimeapps.list");
    let mut out = alloc::vec![poly_files::path::join(config_home, "mimeapps.list")];
    out.extend(dirs(config_dirs, "/etc/xdg").iter().map(|d| poly_files::path::join(d, "mimeapps.list")));
    out.push(apps(data_home));
    out.extend(dirs(data_dirs, "/usr/local/share:/usr/share").iter().map(|d| apps(d)));
    out
}

/// What the `mimeapps.list` files in `paths` (most important first) say,
/// as one text (the order [`poly_files::apps::handlers_for`] reads it in);
/// `None` if there is none.
pub fn load_mimeapps(fs: &dyn poly_files::Fs, paths: &[String]) -> Option<String> {
    let texts: Vec<String> = paths.iter().filter_map(|p| fs.read_to_string(p).ok()).collect();
    (!texts.is_empty()).then(|| {
        texts.join(
            "
",
        )
    })
}

/// Every desktop entry in `dirs` that opens files (earlier folders win for
/// the same ID).
pub fn load_handlers(fs: &dyn poly_files::Fs, dirs: &[String]) -> Vec<Handler> {
    let mut out: Vec<Handler> = Vec::new();
    for d in dirs {
        let Ok(entries) = fs.read_dir(d) else { continue };
        for e in entries.iter().filter(|e| e.name.ends_with(".desktop")) {
            if out.iter().any(|h| h.id == e.name) {
                continue;
            }
            if let Ok(text) = fs.read_to_string(&poly_files::path::join(d, &e.name))
                && let Some(h) = poly_files::apps::handler_from_desktop_entry(&e.name, &text)
            {
                out.push(h);
            }
        }
    }
    out
}

/// The program `name` runs: itself with a `/`, else the first in `PATH`.
pub fn resolve(name: &str, path_var: Option<&str>, exists: &dyn Fn(&str) -> bool) -> String {
    if name.contains('/') || name.is_empty() {
        return name.into();
    }
    path_var
        .unwrap_or("/usr/local/bin:/usr/bin:/bin")
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| poly_files::path::join(d, name))
        .find(|p| exists(p))
        .unwrap_or_else(|| name.into())
}

/// Start `argv` with our environment, detached from us.
fn spawn(argv: &[String]) -> Result<(), String> {
    let first = argv.first().ok_or("nothing to run")?;
    let exists = |p: &str| poly_rt::fs::metadata(p).is_ok_and(|m| m.is_file() && m.mode() & 0o111 != 0);
    let program = resolve(first, poly_rt::env::var("PATH"), &exists);
    // The desktop shows a busy pointer until the app has a window (ADR 0167).
    let mut startup = poly_app::startup::Startup::begin(first);
    let mut cmd = poly_rt::process::Command::new(&program);
    cmd.arg0(first)
        .args(argv.iter().skip(1).map(String::as_str))
        .envs(
            poly_rt::env::vars()
                .filter_map(|v| core::str::from_utf8(v).ok())
                .filter(|v| !poly_app::startup::is_token_var(v)),
        )
        .envs(startup.environment().iter().map(String::as_str))
        .stdin(poly_rt::process::Stdio::Null);
    match cmd.spawn() {
        Ok(child) => {
            startup.started(child.id());
            Ok(())
        }
        Err(e) => {
            startup.failed();
            Err(alloc::format!("{first}: {e}"))
        }
    }
}

/// The user's data directory (`$XDG_DATA_HOME`), where the AppImages the
/// installer put there are (ADR 0180).
fn data_home() -> String {
    poly_shell::apps::data_home(poly_rt::env::var("XDG_DATA_HOME"), poly_rt::env::var("HOME"))
}

/// Every installed app, as the launcher reads them.
fn installed_apps() -> Vec<poly_shell::App> {
    let user = poly_shell::apps::user_applications(&data_home());
    poly_shell::apps::installed_for_user(&native_fs::Disk, &user, &mut |_, _| {})
}

/// The apps the installer can uninstall: those of packages that were
/// installed after the base root was made, and its AppImages.
fn removable_apps() -> Vec<String> {
    poly_pkg::remove::removable(&native_fs::Disk, &data_home())
}

/// Ask the desktop to show the installer's window that uninstalls `item`'s
/// app; it asks before anything is removed (ADR 0180).
fn uninstall_app(item: &poly_shell::applications::Item) -> Result<(), String> {
    if poly_app::startup::ask(&poly_shell::applications::uninstall_request(item)) {
        Ok(())
    } else {
        Err("the desktop cannot be reached".into())
    }
}

/// Ask the desktop to start `item`'s app as its launcher does. Without a
/// desktop to ask (no control pipe), a program is started from here, as
/// Files starts what opens a file; the shell's own windows cannot be.
fn open_app(item: &poly_shell::applications::Item) -> Result<(), String> {
    if poly_app::startup::ask(&poly_shell::applications::open_request(item)) {
        return Ok(());
    }
    if item.app.internal().is_some() {
        return Err("the desktop cannot be reached".into());
    }
    spawn(&item.app.argv)
}

/// The program: the window until it is closed.
pub fn main() -> i32 {
    let fs = native_fs::PolyFs;
    let home = String::from(poly_rt::env::var("HOME").filter(|h| h.starts_with('/')).unwrap_or("/root"));
    let config =
        poly_rt::env::var("XDG_CONFIG_HOME").map(String::from).unwrap_or_else(|| poly_rt::fs::join(&home, ".config"));
    let user_dirs = native_fs::read_text(&poly_rt::fs::join(&config, "user-dirs.dirs"));
    let mounts = native_fs::read_text("/proc/mounts").unwrap_or_default();
    let exists = |p: &str| poly_rt::fs::metadata(p).is_ok_and(|m| m.is_dir());
    let places = poly_files::places::places(&home, user_dirs.as_deref(), &mounts, &exists);
    let data_home = poly_rt::env::var("XDG_DATA_HOME");
    let trash = poly_files::trash::trash_dir(&home, data_home);
    let (with_apps, folder) = applications::overlay(Box::new(fs), Box::new(installed_apps), Box::new(removable_apps));
    let mut app = view::FilesApp::new(with_apps, &home, &trash, places, Box::new(spawn));
    app.set_applications(folder, Box::new(open_app));
    app.uninstall_app = Box::new(uninstall_app);
    app.set_icon_files(Box::new(native_fs::Disk));
    app.handlers = load_handlers(&fs, &application_dirs(&home, data_home));
    let data_home_dir = match data_home.filter(|d| d.starts_with('/')) {
        Some(d) => String::from(d),
        None => poly_files::path::join(&home, ".local/share"),
    };
    let lists = mimeapps_paths(
        &config,
        poly_rt::env::var("XDG_CONFIG_DIRS"),
        &data_home_dir,
        poly_rt::env::var("XDG_DATA_DIRS"),
    );
    app.defaults = load_mimeapps(&fs, &lists);
    app.clock = Box::new(|| poly_rt::time::since_epoch().as_secs());
    let start = poly_rt::env::args_str().nth(1).map(String::from).unwrap_or_else(|| home.clone());
    if !app.go(&start) && !app.go(&home) {
        app.go("/");
    }
    poly_rt::println!("poly-files: {} apps open files", app.handlers.len());
    poly_app::run(&mut app, "Files", "poly-files", (900, 580), (560, 360))
}

#[cfg(test)]
mod tests;
