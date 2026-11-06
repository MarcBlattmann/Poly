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
    /// path. `None` for Poly's own apps and open windows without one: the
    /// shell draws its own glyph.
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
    let app = |name: &str, comment: &str, keywords: &[&str], argv: &[&str]| App {
        name: name.into(),
        comment: comment.into(),
        keywords: keywords.iter().map(|k| (*k).into()).collect(),
        argv: argv.iter().map(|a| (*a).into()).collect(),
        id: alloc::format!("poly:{name}"),
        icon: None,
    };
    alloc::vec![
        app(
            "Settings",
            "Display, keyboard, wallpaper, sound, time zone",
            &["preferences", "volume", "keyboard", "layout", "wallpaper", "display", "scale", "timezone", "about"],
            &[SETTINGS]
        ),
        app("Notifications", "What came in, newest first", &["history", "messages"], &[NOTIFICATIONS]),
        app(
            "Files",
            "Browse and organise your files",
            &["file", "explorer", "folder", "directory", "manager", "browse", "documents", "downloads", "trash"],
            &[FILES]
        ),
        app(
            "Terminal",
            "A shell in a window",
            &["shell", "console", "command", "prompt", "bash", "sh", "cli", "tty"],
            &[TERMINAL]
        ),
    ]
}

/// Poly's own apps, native programs (ADR 0134).
pub const FILES: &str = "/bin/poly-files";
pub const TERMINAL: &str = "/bin/poly-terminal";

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

    #[test]
    fn the_demo_clients_are_not_built_in() {
        // Test fixtures arrive only as desktop files an image carries.
        let all = catalog(Vec::new());
        let names: Vec<&str> = all.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Files", "Notifications", "Settings", "Terminal"]);
        assert!(all.iter().all(|a| !a.argv.iter().any(|p| p.contains("wl-demo") || p.contains("wl-client"))));
    }
}
