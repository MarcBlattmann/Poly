//! The apps the launcher offers: `.desktop` files (the freedesktop.org
//! Desktop Entry Specification, version 1.5: the `[Desktop Entry]` group,
//! `Type`, `Name`, `GenericName`, `Comment`, `Exec` with its quoting and
//! field codes, `NoDisplay`, `Hidden`, `Terminal`, `Keywords`) and Poly's
//! built-ins, Files and Terminal among them (ADR 0134). An entry with
//! `Terminal=true` runs in Poly's Terminal (`poly-terminal -e ...`). The
//! `Icon` key is kept for the launcher to draw ([`crate::appicon`], ADR 0146).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// An app the launcher can start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub name: String,
    /// What it is (`Comment`, else `GenericName`), shown under the name.
    pub comment: String,
    /// Extra words it is found by (`Keywords`).
    pub keywords: Vec<String>,
    /// The program and its arguments, field codes removed.
    pub argv: Vec<String>,
    /// The desktop file's ID (`foo.desktop`), or `poly:<name>` for a
    /// built-in.
    pub id: String,
    /// The desktop file's `Icon`: a name for the icon theme or an absolute
    /// path; Poly's own apps have `poly-<name>`, artwork compiled into the
    /// shell. `None` for open windows without one: the shell draws its own
    /// glyph.
    pub icon: Option<String>,
}

/// Why a desktop file is not offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// No `[Desktop Entry]` group, or no `Name`/`Exec`.
    Incomplete,
    /// `Type` is not `Application`.
    NotAnApplication,
    /// `NoDisplay=true` or `Hidden=true`.
    Hidden,
    /// `Exec` is malformed (an unclosed quote).
    BadExec,
}

/// A desktop file's contents as an app, or why not. `id` is the file name.
pub fn parse_desktop_entry(id: &str, text: &str) -> Result<App, Skip> {
    let mut in_entry = false;
    let mut seen_entry = false;
    let (mut name, mut generic, mut comment, mut exec, mut kind) = (None, None, None, None, None);
    let mut icon = None;
    let mut keywords = Vec::new();
    let (mut hidden, mut terminal) = (false, false);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            seen_entry |= in_entry;
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        // Localised keys (`Name[de]`) are skipped: the unlocalised value is
        // the default.
        let (key, value) = (key.trim(), unescape(value.trim()));
        match key {
            "Type" => kind = Some(value),
            "Name" => name = Some(value),
            "GenericName" => generic = Some(value),
            "Comment" => comment = Some(value),
            "Exec" => exec = Some(value),
            "Icon" => icon = Some(value).filter(|v| !v.is_empty()),
            "Keywords" => keywords = value.split(';').filter(|k| !k.is_empty()).map(ToString::to_string).collect(),
            "NoDisplay" | "Hidden" => hidden |= value == "true",
            "Terminal" => terminal = value == "true",
            _ => {}
        }
    }
    if !seen_entry {
        return Err(Skip::Incomplete);
    }
    if kind.as_deref() != Some("Application") {
        return Err(Skip::NotAnApplication);
    }
    if hidden {
        return Err(Skip::Hidden);
    }
    let (Some(name), Some(exec)) = (name, exec) else { return Err(Skip::Incomplete) };
    let mut argv = parse_exec(&exec).ok_or(Skip::BadExec)?;
    if argv.is_empty() {
        return Err(Skip::Incomplete);
    }
    if terminal {
        // The program runs in Poly's Terminal, which runs what follows
        // `-e` instead of the shell (as xterm's `-e`).
        argv.splice(0..0, [String::from(TERMINAL), String::from("-e")]);
    }
    let comment = comment.or(generic).unwrap_or_default();
    Ok(App { name, comment, keywords, argv, id: id.into(), icon })
}

/// The escapes of string values: `\s` space, `\n`, `\t`, `\r`, `\\`.
fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            // Others (`\;`, and the quoting backslashes of `Exec`) stay.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// `Exec`'s arguments: separated by spaces, double quotes around an
/// argument with `\"`, `` \` ``, `\$` and `\\` inside; the field codes
/// that stand for files or URLs (`%f %F %u %U`) and the deprecated ones
/// dropped, `%i`, `%c`, `%k` dropped too (the icon is the `Icon` key's, no
/// translated names, no file location), `%%` a percent sign. `None` for an unclosed quote.
pub fn parse_exec(exec: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let mut quoted = false;
    let mut it = exec.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            '\\' if quoted => {
                if let Some(n) = it.next() {
                    cur.push(n);
                }
            }
            ' ' | '\t' if !quoted => {
                if has {
                    args.push(core::mem::take(&mut cur));
                    has = false;
                }
            }
            // `%%` is a percent sign; every other field code stands for
            // nothing here.
            '%' => {
                if it.next() == Some('%') {
                    cur.push('%');
                    has = true;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if has {
        args.push(cur);
    }
    Some(args)
}

/// Poly's own apps, offered whatever is installed. Test fixtures (the demo
/// clients) are not among them: a desktop that wants them gets them from
/// desktop files, which `cargo xtask desktop --test-apps` puts in the image.
pub fn builtins() -> Vec<App> {
    let app = |name: &str, comment: &str, keywords: &[&str], argv: &[&str], icon: &str| App {
        name: name.into(),
        comment: comment.into(),
        keywords: keywords.iter().map(|k| (*k).into()).collect(),
        argv: argv.iter().map(|a| (*a).into()).collect(),
        id: alloc::format!("poly:{name}"),
        icon: Some(icon.into()),
    };
    alloc::vec![
        app(
            "Settings",
            "Display, keyboard, wallpaper, sound, time zone",
            &["preferences", "volume", "keyboard", "layout", "wallpaper", "display", "scale", "timezone", "about"],
            &[SETTINGS],
            "poly-settings"
        ),
        app(
            "Notifications",
            "What came in, newest first",
            &["history", "messages"],
            &[NOTIFICATIONS],
            "poly-notifications"
        ),
        app(
            "Files",
            "Browse and organise your files",
            &["file", "explorer", "folder", "directory", "manager", "browse", "documents", "downloads", "trash"],
            &[FILES],
            "poly-files"
        ),
        app(
            "Terminal",
            "A shell in a window",
            &["shell", "console", "command", "prompt", "bash", "sh", "cli", "tty"],
            &[TERMINAL],
            "poly-terminal"
        ),
    ]
}

/// Poly's own apps, native programs (ADR 0134).
pub const FILES: &str = "/bin/poly-files";
pub const TERMINAL: &str = "/bin/poly-terminal";

/// Poly's installer (ADR 0132, 0173): the program that opens a package
/// file. It is not an app of the launcher (it does nothing without a
/// file); its desktop file ([`installer_desktop_file`]) makes it the app
/// that opens packages.
pub const INSTALLER: &str = "/bin/poly-installer";

/// The argument that makes the installer open its window; without it the
/// program asks the desktop to (so the window is always the desktop's).
pub const INSTALLER_WINDOW: &str = "--window";

/// The argument that makes the installer's window uninstall the app with
/// the desktop file ID that follows (ADR 0180).
pub const INSTALLER_UNINSTALL: &str = "--uninstall";

/// The installer's desktop file ID.
pub const INSTALLER_ID: &str = "poly-installer.desktop";

/// The MIME types of the package files the installer opens (the shared
/// MIME database's names): Debian packages, which it installs, and the
/// formats it knows by sight and says so about (RPM, Flatpak references,
/// snaps; ADR 0132).
pub const PACKAGE_MIME_TYPES: [&str; 4] = [
    "application/vnd.debian.binary-package",
    "application/x-rpm",
    "application/vnd.flatpak.ref",
    "application/vnd.snap",
];

/// The installer's desktop file: it opens the package types and is not
/// shown among the apps (`NoDisplay`; Desktop Entry Specification).
pub fn installer_desktop_file() -> String {
    alloc::format!(
        "[Desktop Entry]\nType=Application\nName=Installer\nComment=Install an app from a package file\n\
         Exec={INSTALLER} %f\nNoDisplay=true\nMimeType={};\n",
        PACKAGE_MIME_TYPES.join(";")
    )
}

/// The installer's window for the package file at `path`, as an app the
/// desktop starts (whoever opened the file asked for it, ADR 0173).
pub fn installer_entry(path: &str) -> App {
    App {
        name: "Installer".into(),
        comment: "Install an app from a package file".into(),
        keywords: Vec::new(),
        argv: alloc::vec![INSTALLER.into(), INSTALLER_WINDOW.into(), path.into()],
        id: INSTALLER_ID.into(),
        icon: None,
    }
}

/// The installer's window that uninstalls the app with desktop file ID
/// `id`, as an app the desktop starts (Files asked for it, ADR 0180).
pub fn uninstaller_entry(id: &str) -> App {
    App {
        name: "Installer".into(),
        comment: "Uninstall an app".into(),
        keywords: Vec::new(),
        argv: alloc::vec![INSTALLER.into(), INSTALLER_WINDOW.into(), INSTALLER_UNINSTALL.into(), id.into()],
        id: INSTALLER_ID.into(),
        icon: None,
    }
}

/// The `argv` of the built-ins the shell itself opens (ADR 0117).
pub const SETTINGS: &str = "poly-shell:settings";
pub const NOTIFICATIONS: &str = "poly-shell:notifications";

/// The `argv` prefix of an open window listed in the launcher (Alpine lists
/// windows with the apps); the window's ID follows.
pub const WINDOW: &str = "poly-shell:window:";

/// A launcher entry for open window `id` titled `title` (of app `app_id`).
pub fn window_entry(id: u32, title: &str, app_id: &str) -> App {
    App {
        name: if title.is_empty() { app_id.into() } else { title.into() },
        comment: if app_id.is_empty() { "Window".into() } else { app_id.into() },
        keywords: Vec::new(),
        argv: alloc::vec![alloc::format!("{WINDOW}{id}")],
        id: alloc::format!("{WINDOW}{id}"),
        icon: None,
    }
}

impl App {
    /// The open window this entry stands for, if it is one.
    pub fn window(&self) -> Option<u32> {
        self.argv.first()?.strip_prefix(WINDOW)?.parse().ok()
    }

    /// The shell's own part this app opens, if it is one.
    pub fn internal(&self) -> Option<&str> {
        self.argv.first().map(String::as_str).filter(|a| a.starts_with("poly-shell:"))
    }
}

/// The catalog: built-ins first, then the desktop files, sorted by name,
/// each name once (a desktop file with a built-in's name replaces it).
pub fn catalog(desktop: Vec<App>) -> Vec<App> {
    let mut all = builtins();
    for app in desktop {
        all.retain(|a| a.name != app.name);
        all.push(app);
    }
    all.sort_by_key(|a| a.name.to_lowercase());
    all
}

/// Where desktop files are (the Desktop Entry Specification's
/// `$XDG_DATA_DIRS/applications`, with the default data directory).
pub const APPLICATIONS: &str = "/usr/share/applications";

/// The disk as the catalog's reader sees it: the compositor's and Files'
/// own (the shell does no I/O).
pub trait AppFiles {
    /// The names in a directory (empty if it is missing).
    fn list(&self, dir: &str) -> Vec<String>;
    /// A file's text, `None` if it is missing or unreadable.
    fn read(&self, path: &str) -> Option<String>;
}

/// The apps of every readable desktop file in `dir`, in the order of the
/// files' names (so two readers of one disk get one list); `skipped` hears
/// of each file that is not offered and why.
pub fn desktop_apps(files: &dyn AppFiles, dir: &str, skipped: &mut dyn FnMut(&str, Skip)) -> Vec<App> {
    let mut names: Vec<String> = files.list(dir).into_iter().filter(|n| n.ends_with(".desktop")).collect();
    names.sort();
    let mut apps = Vec::new();
    for name in names {
        let Some(text) = files.read(&alloc::format!("{}/{name}", dir.trim_end_matches('/'))) else { continue };
        match parse_desktop_entry(&name, &text) {
            Ok(app) => apps.push(app),
            Err(why) => skipped(&name, why),
        }
    }
    apps
}

/// The user's data directory: `$XDG_DATA_HOME` if it is an absolute path,
/// else `~/.local/share` of `$HOME`, else root's (the desktop's user today).
pub fn data_home(xdg_data_home: Option<&str>, home: Option<&str>) -> String {
    match xdg_data_home.filter(|d| d.starts_with('/')) {
        Some(d) => d.trim_end_matches('/').into(),
        None => alloc::format!(
            "{}/.local/share",
            home.filter(|h| h.starts_with('/')).unwrap_or("/root").trim_end_matches('/')
        ),
    }
}

/// The user's applications directory below the data directory `data_home`:
/// the desktop files of what the installer put there for the user.
pub fn user_applications(data_home: &str) -> String {
    alloc::format!("{}/applications", data_home.trim_end_matches('/'))
}

/// Every installed app, with the user's own desktop files in
/// `user_dir` (`$XDG_DATA_HOME/applications`: the apps the installer
/// put there, ADR 0180) read after the system's: for one file ID the
/// user's wins, as the Desktop Entry Specification says.
pub fn installed_for_user(files: &dyn AppFiles, user_dir: &str, skipped: &mut dyn FnMut(&str, Skip)) -> Vec<App> {
    let mut apps = desktop_apps(files, APPLICATIONS, skipped);
    let user = desktop_apps(files, user_dir, skipped);
    apps.retain(|a| !user.iter().any(|u| u.id == a.id));
    apps.extend(user);
    apps.sort_by(|a, b| a.id.cmp(&b.id));
    catalog(apps)
}

/// Every installed app: the one list the launcher offers and Files'
/// Applications folder shows (ADR 0175), [`catalog`] over the desktop files
/// in [`APPLICATIONS`].
pub fn installed(files: &dyn AppFiles, skipped: &mut dyn FnMut(&str, Skip)) -> Vec<App> {
    catalog(desktop_apps(files, APPLICATIONS, skipped))
}

impl App {
    /// The glyph drawn for the app where it has no icon of its own (no
    /// `Icon`, or none in the icon theme).
    pub fn glyph(&self) -> crate::icons::Icon {
        use crate::icons::lucide;
        match self.internal() {
            _ if self.window().is_some() => lucide::APP_WINDOW,
            Some(SETTINGS) => lucide::SETTINGS,
            Some(NOTIFICATIONS) => lucide::BELL,
            _ => match self.argv.first().map(String::as_str) {
                Some(FILES) => lucide::FOLDER,
                Some(TERMINAL) if self.argv.len() == 1 => lucide::SQUARE_TERMINAL,
                _ => lucide::APP_WINDOW,
            },
        }
    }
}

/// Desktop-file apps for the shell's tests, standing in for what an image
/// carries: not Poly's built-ins and not test fixtures.
#[cfg(test)]
pub(crate) fn sample_apps() -> Vec<App> {
    let app = |name: &str, keywords: &[&str], prog: &str| App {
        name: name.into(),
        comment: String::new(),
        keywords: keywords.iter().map(|k| (*k).into()).collect(),
        argv: alloc::vec![prog.into()],
        id: alloc::format!("{prog}.desktop"),
        icon: None,
    };
    catalog(alloc::vec![
        app("Pixel Viewer", &["picture", "image"], "/bin/viewer"),
        app("Quill Editor", &["write", "text"], "/bin/editor"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn a_desktop_file() {
        let text = "# comment\n[Desktop Entry]\nType=Application\nName=Text Editor\nName[de]=Texteditor\n\
                    Comment=Edit text files\nExec=gedit --new-window %U\nIcon=gedit\nKeywords=Text;Plain;\n\
                    \n[Desktop Action new]\nName=New\nExec=other\n";
        let app = parse_desktop_entry("org.gnome.gedit.desktop", text).unwrap();
        assert_eq!(app.name, "Text Editor");
        assert_eq!(app.comment, "Edit text files");
        assert_eq!(app.argv, ["gedit", "--new-window"]);
        assert_eq!(app.keywords, ["Text", "Plain"]);
        assert_eq!(app.id, "org.gnome.gedit.desktop");
        assert_eq!(app.icon.as_deref(), Some("gedit"));
    }

    #[test]
    fn skipped_desktop_files() {
        let e = |body: &str| parse_desktop_entry("x.desktop", &alloc::format!("[Desktop Entry]\n{body}"));
        assert_eq!(e("Type=Link\nName=A\nURL=x"), Err(Skip::NotAnApplication));
        assert_eq!(e("Type=Application\nName=A\nExec=a\nNoDisplay=true"), Err(Skip::Hidden));
        assert_eq!(e("Type=Application\nName=A\nExec=a\nHidden=true"), Err(Skip::Hidden));
        let vim = e("Type=Application\nName=A\nExec=vim %F\nTerminal=true").unwrap();
        assert_eq!(vim.argv, ["/bin/poly-terminal", "-e", "vim"], "Terminal=true runs in Poly's Terminal");
        assert_eq!(e("Type=Application\nName=A"), Err(Skip::Incomplete));
        assert_eq!(e("Type=Application\nName=A\nExec=\"a"), Err(Skip::BadExec));
        assert_eq!(parse_desktop_entry("x", "Name=A\nExec=a"), Err(Skip::Incomplete));
        // GenericName stands in for a missing comment.
        let app = e("Type=Application\nName=A\nGenericName=Thing\nExec=a").unwrap();
        assert_eq!(app.comment, "Thing");
    }

    #[test]
    fn exec_quoting_and_field_codes() {
        assert_eq!(parse_exec("a b  c").unwrap(), ["a", "b", "c"]);
        assert_eq!(parse_exec(r#""/opt/my app/run" --x="1 2" %f"#).unwrap(), ["/opt/my app/run", "--x=1 2"]);
        assert_eq!(parse_exec(r#"echo "a \"q\" \$HOME \\ b""#).unwrap(), ["echo", r#"a "q" $HOME \ b"#]);
        assert_eq!(parse_exec("prog 100%% %i %c %k").unwrap(), ["prog", "100%"]);
        assert_eq!(parse_exec(r#"say """#).unwrap(), ["say", ""]);
        assert_eq!(parse_exec(r#""open"#), None);
        assert_eq!(unescape(r"a\sb\\c\;"), r"a b\c\;");
    }

    #[test]
    fn the_catalog_is_sorted_and_names_are_unique() {
        let file = |name: &str| App {
            name: name.into(),
            comment: String::new(),
            keywords: vec![],
            argv: vec!["x".into()],
            id: "f.desktop".into(),
            icon: None,
        };
        let all = catalog(vec![file("zebra"), file("Settings"), file("apple")]);
        let names: Vec<&str> = all.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["apple", "Files", "Notifications", "Settings", "Terminal", "zebra"]);
        assert_eq!(all[3].id, "f.desktop", "the desktop file replaced the built-in");
        assert_eq!(all[2].internal(), Some(NOTIFICATIONS));
        assert_eq!(all[3].internal(), None);
        assert_eq!(all[1].argv, [FILES]);
        assert_eq!(all[4].argv, [TERMINAL]);
    }

    /// A disk of desktop files, listed in the order given.
    pub(crate) struct Disk(pub Vec<(&'static str, &'static str)>);

    impl AppFiles for Disk {
        fn list(&self, dir: &str) -> Vec<String> {
            let dir = alloc::format!("{dir}/");
            self.0.iter().filter_map(|(p, _)| p.strip_prefix(dir.as_str())).map(String::from).collect()
        }

        fn read(&self, path: &str) -> Option<String> {
            self.0.iter().find(|(p, _)| *p == path).map(|(_, t)| (*t).into())
        }
    }

    #[test]
    fn the_users_desktop_files_join_the_systems_and_win_for_one_id() {
        const SYS: &str = "[Desktop Entry]\nType=Application\nName=System Name\nExec=s\n";
        const USER: &str = "[Desktop Entry]\nType=Application\nName=User Name\nExec=u\n";
        const OWN: &str = "[Desktop Entry]\nType=Application\nName=Hello Pixels\nExec=/h/x.AppImage\n";
        let disk = Disk(vec![
            ("/usr/share/applications/same.desktop", SYS),
            ("/usr/share/applications/other.desktop", SYS),
            ("/root/.local/share/applications/same.desktop", USER),
            ("/root/.local/share/applications/poly-appimage-hello-pixels.desktop", OWN),
        ]);
        let apps = installed_for_user(&disk, "/root/.local/share/applications", &mut |_, _| {});
        let found: Vec<(&str, &str)> =
            apps.iter().filter(|a| !a.id.starts_with("poly:")).map(|a| (a.id.as_str(), a.name.as_str())).collect();
        assert_eq!(
            found,
            [
                ("poly-appimage-hello-pixels.desktop", "Hello Pixels"),
                ("other.desktop", "System Name"),
                ("same.desktop", "User Name"),
            ]
        );
        // Without a user directory nothing changes.
        let none = installed_for_user(&disk, "/nowhere", &mut |_, _| {});
        assert_eq!(none.iter().find(|a| a.id == "same.desktop").map(|a| a.name.as_str()), Some("System Name"));
        assert_eq!(data_home(Some("/data/"), Some("/home/x")), "/data");
        assert_eq!(data_home(Some("relative"), Some("/home/x/")), "/home/x/.local/share");
        assert_eq!(data_home(None, None), "/root/.local/share");
    }

    #[test]
    fn the_uninstaller_is_the_installers_window_for_an_app() {
        let e = uninstaller_entry("galculator.desktop");
        assert_eq!(e.argv, [INSTALLER, "--window", "--uninstall", "galculator.desktop"]);
        assert_eq!(e.id, INSTALLER_ID);
    }

    #[test]
    fn the_installed_apps_are_read_in_one_order_whatever_the_disk_says() {
        const A: &str = "[Desktop Entry]\nType=Application\nName=Editor\nExec=a\n";
        const B: &str = "[Desktop Entry]\nType=Application\nName=Editor\nExec=b\n";
        const HIDDEN: &str = "[Desktop Entry]\nType=Application\nName=Helper\nExec=h\nNoDisplay=true\n";
        let disk = |files: Vec<(&'static str, &'static str)>| {
            let mut skipped = Vec::new();
            let apps = installed(&Disk(files), &mut |n, why| skipped.push((String::from(n), why)));
            (apps, skipped)
        };
        let (one, skipped) = disk(vec![
            ("/usr/share/applications/b.desktop", B),
            ("/usr/share/applications/a.desktop", A),
            ("/usr/share/applications/h.desktop", HIDDEN),
            ("/usr/share/applications/notes.txt", "x"),
            ("/elsewhere/c.desktop", A),
        ]);
        let (other, _) = disk(vec![
            ("/usr/share/applications/h.desktop", HIDDEN),
            ("/usr/share/applications/a.desktop", A),
            ("/usr/share/applications/b.desktop", B),
        ]);
        assert_eq!(one, other, "the directory's order does not matter");
        let names: Vec<&str> = one.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Editor", "Files", "Notifications", "Settings", "Terminal"]);
        assert_eq!(one[0].id, "b.desktop", "of two files with one name the later file name wins");
        assert_eq!(skipped, [(String::from("h.desktop"), Skip::Hidden)]);
    }

    #[test]
    fn glyphs_of_apps_without_an_icon() {
        use crate::icons::lucide;
        let all = catalog(Vec::new());
        let glyphs: Vec<_> = all.iter().map(App::glyph).collect();
        assert_eq!(glyphs, [lucide::FOLDER, lucide::BELL, lucide::SETTINGS, lucide::SQUARE_TERMINAL]);
        assert_eq!(window_entry(3, "A", "a").glyph(), lucide::APP_WINDOW);
        assert_eq!(sample_apps()[2].glyph(), lucide::APP_WINDOW);
    }

    #[test]
    fn the_demo_clients_are_not_built_in() {
        // Test fixtures arrive only as desktop files an image carries.
        let all = catalog(Vec::new());
        let names: Vec<&str> = all.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Files", "Notifications", "Settings", "Terminal"]);
        assert!(all.iter().all(|a| !a.argv.iter().any(|p| p.contains("wl-demo") || p.contains("wl-client"))));
    }
}
