//! Which app opens a file ("Open With").
//!
//! **Handlers** are desktop entries (Desktop Entry Specification 1.5) with
//! a `MimeType=` list. The launcher's parser
//! ([`poly_shell::apps::parse_desktop_entry`]) supplies the [`App`]; this
//! module also keeps the raw `Exec` line, because the launcher's argv has
//! the field codes removed and opening a file needs them expanded. Entries
//! with `NoDisplay=true` are still handlers (many viewers hide from the
//! launcher but open files); `Hidden=true` ones are deleted and are not.
//!
//! **Choosing** follows the MIME Applications Associations spec in a
//! reduced form. Given the text of `mimeapps.list` (several files may be
//! concatenated, most important first: the first `[Default Applications]`
//! line for a type wins), the ranking for type `t` is:
//!
//! 1. the installed apps of `t=` in `[Default Applications]`, in order;
//! 2. those of `t=` in `[Added Associations]`, in order;
//! 3. handlers listing `t` in `MimeType=`;
//! 4. handlers listing the wildcard `top/*`;
//! 5. for text types (`text/*`, scripts, JSON, …), handlers of `text/plain`.
//!
//! Steps 3–5 skip apps named for `t` in `[Removed Associations]`, and each
//! step orders by desktop file ID, so the choice is deterministic. Type
//! names compare case-insensitively.
//!
//! **Exec field codes** ([`command_for`]): `%f`/`%u` the first file, `%F`/
//! `%U` (as a whole argument) every file, `%c` the app's name, `%%` a
//! percent sign; `%i`, `%k` and the deprecated codes are dropped. URLs
//! (`%u`/`%U`) are passed as plain absolute paths, which the spec allows
//! for local files and which every world's apps understand; see
//! [`crate::path::file_uri`] for the `file://` form. When `Exec` has no
//! file code at all, the files are appended (an "Open With" app that
//! ignores the file would be useless). Apps with `Terminal=true` run inside
//! Poly's terminal ([`TERMINAL`]).
//!
//! **Opening** ([`open_action`]): folders are browsed; a desktop entry with
//! an execute bit launches its `Exec`; a regular file with an execute bit
//! that is an ELF program or a `#!` script runs itself; anything else goes
//! to the chosen handler. Requiring the execute bit for desktop entries
//! keeps a downloaded `photo.jpg.desktop` from running on a double click
//! (without the bit it opens in the text editor). Windows programs (`.exe`)
//! are opened by whatever handler claims `application/x-msdownload`.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use poly_shell::apps::{App, parse_desktop_entry, parse_exec};

use crate::fs::{Fs, FsError, Kind, Meta};
use crate::mime;

/// The argv prefix that runs a program in Poly's terminal (the launcher's
/// [`poly_shell::apps::TERMINAL`] with `-e`).
pub const TERMINAL: [&str; 2] = [poly_shell::apps::TERMINAL, "-e"];

/// An app that opens files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handler {
    /// The app as the launcher shows it.
    pub app: App,
    /// The `MimeType=` list.
    pub mime_types: Vec<String>,
    /// The desktop file ID (`org.example.Viewer.desktop`).
    pub id: String,
    /// The raw `Exec` value (string escapes undone, quoting and field codes
    /// kept).
    pub exec: String,
    /// `Terminal=true`.
    pub terminal: bool,
}

/// The keys of a `[Desktop Entry]` group this module needs.
#[derive(Clone, Debug, Default)]
struct Entry {
    kind: Option<String>,
    name: Option<String>,
    generic: Option<String>,
    comment: Option<String>,
    exec: Option<String>,
    mime_types: Vec<String>,
    keywords: Vec<String>,
    terminal: bool,
    hidden: bool,
}

/// The string escapes of desktop entry values: `\s`, `\n`, `\t`, `\r`,
/// `\\`; others kept.
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
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn list(v: &str) -> Vec<String> {
    v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(ToString::to_string).collect()
}

fn parse_entry(text: &str) -> Option<Entry> {
    let mut e = Entry::default();
    let (mut inside, mut seen) = (false, false);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            inside = line == "[Desktop Entry]";
            seen |= inside;
            continue;
        }
        if !inside {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = unescape(v.trim());
        match k.trim() {
            "Type" => e.kind = Some(v),
            "Name" => e.name = Some(v),
            "GenericName" => e.generic = Some(v),
            "Comment" => e.comment = Some(v),
            "Exec" => e.exec = Some(v),
            "MimeType" => e.mime_types = list(&v),
            "Keywords" => e.keywords = list(&v),
            "Terminal" => e.terminal = v == "true",
            "Hidden" => e.hidden = v == "true",
            _ => {}
        }
    }
    seen.then_some(e)
}

/// The handler a desktop file describes, if it is an application with an
/// `Exec` line (see the module docs).
pub fn handler_from_desktop_entry(id: &str, text: &str) -> Option<Handler> {
    let e = parse_entry(text)?;
    if e.kind.as_deref() != Some("Application") || e.hidden {
        return None;
    }
    let exec = e.exec.clone()?;
    let app = match parse_desktop_entry(id, text) {
        Ok(app) => app,
        // Not offered by the launcher (NoDisplay, or a terminal app on an
        // older launcher) but still an app that opens files.
        Err(_) => {
            let argv = parse_exec(&exec).filter(|a| !a.is_empty())?;
            App {
                name: e.name.clone()?,
                comment: e.comment.clone().or(e.generic.clone()).unwrap_or_default(),
                keywords: e.keywords.clone(),
                argv,
                id: id.into(),
                icon: None,
            }
        }
    };
    split_exec(&exec)?;
    Some(Handler { app, mime_types: e.mime_types, id: id.into(), exec, terminal: e.terminal })
}

/// A piece of an `Exec` argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Piece {
    Lit(char),
    Code(char),
}

/// `Exec`'s arguments with field codes kept; `None` for an unclosed quote.
fn split_exec(exec: &str) -> Option<Vec<Vec<Piece>>> {
    let mut args = Vec::new();
    let mut cur: Vec<Piece> = Vec::new();
    let (mut has, mut quoted) = (false, false);
    let mut it = exec.chars();
    while let Some(c) = it.next() {
        match c {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            '\\' if quoted => {
                if let Some(n) = it.next() {
                    cur.push(Piece::Lit(n));
                }
            }
            ' ' | '\t' if !quoted => {
                if has {
                    args.push(core::mem::take(&mut cur));
                    has = false;
                }
            }
            '%' => {
                match it.next() {
                    Some('%') => cur.push(Piece::Lit('%')),
                    Some(code) => cur.push(Piece::Code(code)),
                    None => {}
                }
                has = true;
            }
            c => {
                cur.push(Piece::Lit(c));
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

/// Whether `Exec` takes files, and whether one per run (`%f`/`%u`).
fn file_codes(args: &[Vec<Piece>]) -> (bool, bool) {
    let codes = || args.iter().flatten().filter_map(|p| if let Piece::Code(c) = p { Some(*c) } else { None });
    let any = codes().any(|c| matches!(c, 'f' | 'F' | 'u' | 'U'));
    let many = codes().any(|c| matches!(c, 'F' | 'U'));
    (any, any && !many)
}

/// The argv that opens `paths` with `handler` (one run; with `%f`/`%u`
/// only the first file is passed, see [`commands_for`]).
pub fn command_for(handler: &Handler, paths: &[&str]) -> Vec<String> {
    let Some(args) = split_exec(&handler.exec) else { return Vec::new() };
    let mut argv: Vec<String> = Vec::new();
    if handler.terminal {
        argv.extend(TERMINAL.iter().map(|s| s.to_string()));
    }
    let (takes_files, _) = file_codes(&args);
    for arg in &args {
        if let [Piece::Code('F' | 'U')] = arg.as_slice() {
            argv.extend(paths.iter().map(|p| p.to_string()));
            continue;
        }
        let mut s = String::new();
        let mut only_codes = true;
        for p in arg {
            match p {
                Piece::Lit(c) => {
                    s.push(*c);
                    only_codes = false;
                }
                Piece::Code('f' | 'u' | 'F' | 'U') => s.push_str(paths.first().copied().unwrap_or("")),
                Piece::Code('c') => s.push_str(&handler.app.name),
                Piece::Code(_) => {}
            }
        }
        if !(only_codes && s.is_empty()) {
            argv.push(s);
        }
    }
    if !takes_files {
        argv.extend(paths.iter().map(|p| p.to_string()));
    }
    argv
}

/// The runs that open `paths` with `handler`: one per file when `Exec`
/// takes a single file (`%f`/`%u`), else one for all.
pub fn commands_for(handler: &Handler, paths: &[&str]) -> Vec<Vec<String>> {
    let single = split_exec(&handler.exec).is_some_and(|a| file_codes(&a).1);
    if single && paths.len() > 1 {
        paths.iter().map(|p| command_for(handler, &[p])).collect()
    } else {
        alloc::vec![command_for(handler, paths)]
    }
}

/// The app IDs listed for `mime` in `group` of a `mimeapps.list` text
/// (first matching line only).
fn listed<'a>(defaults: &'a str, group: &str, mime: &str) -> Vec<&'a str> {
    let mut inside = false;
    for line in defaults.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == group;
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case(mime)
        {
            return v.split(';').map(str::trim).filter(|s| !s.is_empty()).collect();
        }
    }
    Vec::new()
}

/// Every handler for `mime`, best first (see the module docs).
pub fn handlers_for<'a>(handlers: &'a [Handler], mime: &str, defaults: Option<&str>) -> Vec<&'a Handler> {
    let mut out: Vec<&Handler> = Vec::new();
    let push = |h: &'a Handler, out: &mut Vec<&'a Handler>| {
        if !out.iter().any(|o| o.id == h.id) {
            out.push(h);
        }
    };
    let by_id = |id: &str| handlers.iter().find(|h| h.id == id);
    let removed = defaults.map(|d| listed(d, "[Removed Associations]", mime)).unwrap_or_default();
    if let Some(d) = defaults {
        for group in ["[Default Applications]", "[Added Associations]"] {
            for id in listed(d, group, mime) {
                if let Some(h) = by_id(id) {
                    push(h, &mut out);
                }
            }
        }
    }
    let mut sorted: Vec<&Handler> = handlers.iter().filter(|h| !removed.contains(&h.id.as_str())).collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let has = |h: &Handler, t: &str| h.mime_types.iter().any(|m| m.eq_ignore_ascii_case(t));
    let top = mime.split_once('/').map(|(t, _)| t).unwrap_or(mime);
    let wildcard = alloc::format!("{top}/*");
    for want in [mime, wildcard.as_str()] {
        for h in &sorted {
            if has(h, want) {
                push(h, &mut out);
            }
        }
    }
    if mime::is_text(mime) {
        for h in &sorted {
            if has(h, mime::TEXT) {
                push(h, &mut out);
            }
        }
    }
    out
}

/// The app that opens `mime`, if any (the first of [`handlers_for`]).
pub fn choose<'a>(handlers: &'a [Handler], mime: &str, defaults: Option<&str>) -> Option<&'a Handler> {
    handlers_for(handlers, mime, defaults).into_iter().next()
}

/// What opening a file does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Open {
    /// Browse this folder.
    Folder(String),
    /// Run this argv (a program, a script, a desktop entry's `Exec`).
    Launch(Vec<String>),
    /// Run this app argv with the file.
    With(Vec<String>),
    /// Nothing opens this type.
    NoApp { mime: &'static str },
}

/// What opening `path` does, given its metadata (links followed), its
/// type (best guessed with content, [`mime::guess`]) and, for regular
/// files, its first bytes (a desktop entry needs all of it, so pass up to
/// [`DESKTOP_MAX`] bytes).
pub fn open_action(
    path: &str,
    meta: &Meta,
    mime: &'static str,
    contents: Option<&[u8]>,
    handlers: &[Handler],
    defaults: Option<&str>,
) -> Open {
    if meta.kind == Kind::Dir {
        return Open::Folder(crate::path::normalize(path));
    }
    let exec = meta.kind == Kind::File && meta.executable();
    if exec && mime == mime::DESKTOP {
        let text = contents.map(String::from_utf8_lossy);
        let id = crate::path::file_name(path);
        if let Some(h) = text.and_then(|t| handler_from_desktop_entry(id, &t)) {
            let argv = command_for(&h, &[]);
            if !argv.is_empty() {
                return Open::Launch(argv);
            }
        }
    }
    let program = mime == mime::EXECUTABLE
        || mime == "application/vnd.appimage"
        || contents.is_some_and(|c| c.starts_with(b"#!") || mime::sniff(c) == Some(mime::EXECUTABLE));
    if exec && program {
        return Open::Launch(alloc::vec![crate::path::normalize(path)]);
    }
    match choose(handlers, mime, defaults) {
        Some(h) => Open::With(command_for(h, &[path])),
        None => Open::NoApp { mime },
    }
}

/// How much of a file [`open`] reads.
pub const DESKTOP_MAX: usize = 64 * 1024;

/// [`open_action`] for `path` on `fs`: stats it, reads its head and
/// guesses its type by name and content.
pub fn open(fs: &dyn Fs, path: &str, handlers: &[Handler], defaults: Option<&str>) -> Result<Open, FsError> {
    let meta = fs.stat(path)?;
    let head = if meta.kind == Kind::File { Some(fs.read_head(path, DESKTOP_MAX)?) } else { None };
    let sniff = head.as_deref().map(|h| h.get(..mime::SNIFF_LEN).unwrap_or(h));
    let m = mime::guess(crate::path::file_name(path), meta.kind, sniff);
    Ok(open_action(path, &meta, m, head.as_deref(), handlers, defaults))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfs::MemFs;
    use alloc::vec;

    fn entry(id: &str, exec: &str, mimes: &str) -> Handler {
        let text = alloc::format!("[Desktop Entry]\nType=Application\nName={id}\nExec={exec}\nMimeType={mimes}\n");
        handler_from_desktop_entry(id, &text).unwrap()
    }

    fn handlers() -> Vec<Handler> {
        vec![
            entry("viewer.desktop", "viewer %f", "image/png;image/jpeg;"),
            entry("gallery.desktop", "gallery %U", "image/*;"),
            entry("edit.desktop", "edit %F", "text/plain;"),
            entry("code.desktop", "code %F", "text/rust;text/plain;"),
            entry("aaa-paint.desktop", "paint %f", "image/png;"),
        ]
    }

    fn ids(v: Vec<&Handler>) -> Vec<&str> {
        v.into_iter().map(|h| h.id.as_str()).collect()
    }

    #[test]
    fn handlers_from_desktop_files() {
        let h = handler_from_desktop_entry(
            "v.desktop",
            "[Desktop Entry]\nType=Application\nName=V\nExec=v --open %U\nMimeType=image/png; image/gif\nNoDisplay=true\n",
        )
        .unwrap();
        assert_eq!(h.mime_types, ["image/png", "image/gif"]);
        assert_eq!(h.exec, "v --open %U");
        assert_eq!(h.app.argv, ["v", "--open"]);
        assert!(!h.terminal);
        let t = handler_from_desktop_entry(
            "vi.desktop",
            "[Desktop Entry]\nType=Application\nName=Vi\nExec=vi %F\nTerminal=true\n",
        )
        .unwrap();
        assert!(t.terminal);
        assert_eq!(command_for(&t, &["/a"]), ["/bin/poly-terminal", "-e", "vi", "/a"]);
        assert!(handler_from_desktop_entry("x", "[Desktop Entry]\nType=Link\nName=L\nURL=x\n").is_none());
        assert!(
            handler_from_desktop_entry("x", "[Desktop Entry]\nType=Application\nName=A\nExec=a\nHidden=true\n")
                .is_none()
        );
        assert!(handler_from_desktop_entry("x", "[Desktop Entry]\nType=Application\nName=A\nExec=\"a\n").is_none());
        assert!(handler_from_desktop_entry("x", "Name=A\nExec=a").is_none());
    }

    #[test]
    fn field_codes() {
        let h = entry("x.desktop", r#""/opt/my app/x" --title=%c 100%% %i %k "--file=%f" %U"#, "");
        assert_eq!(
            command_for(&h, &["/a b", "/c"]),
            ["/opt/my app/x", "--title=x.desktop", "100%", "--file=/a b", "/a b", "/c"]
        );
        assert_eq!(command_for(&h, &[]), ["/opt/my app/x", "--title=x.desktop", "100%", "--file="]);
        let single = entry("s.desktop", "s %u", "");
        assert_eq!(commands_for(&single, &["/a", "/b"]), [vec!["s", "/a"], vec!["s", "/b"]]);
        assert_eq!(commands_for(&single, &[]), [vec!["s"]]);
        let many = entry("m.desktop", "m %F", "");
        assert_eq!(commands_for(&many, &["/a", "/b"]), [vec!["m", "/a", "/b"]]);
        // No file code: the files are appended.
        let none = entry("n.desktop", "n --new-window", "");
        assert_eq!(command_for(&none, &["/a"]), ["n", "--new-window", "/a"]);
        // Escapes inside quotes.
        let q = entry("q.desktop", r#"sh -c "echo \"\$1\"" %f"#, "");
        assert_eq!(command_for(&q, &["/f"]), ["sh", "-c", r#"echo "$1""#, "/f"]);
    }

    #[test]
    fn choosing() {
        let hs = handlers();
        assert_eq!(choose(&hs, "image/png", None).unwrap().id, "aaa-paint.desktop", "exact, by id");
        assert_eq!(
            ids(handlers_for(&hs, "image/png", None)),
            ["aaa-paint.desktop", "viewer.desktop", "gallery.desktop"]
        );
        assert_eq!(choose(&hs, "image/webp", None).unwrap().id, "gallery.desktop", "wildcard");
        assert_eq!(choose(&hs, "text/rust", None).unwrap().id, "code.desktop");
        assert_eq!(choose(&hs, "text/x-python", None).unwrap().id, "code.desktop", "text/plain fallback");
        assert_eq!(ids(handlers_for(&hs, "application/json", None)), ["code.desktop", "edit.desktop"]);
        assert!(choose(&hs, "application/pdf", None).is_none());

        let list = "[Default Applications]\nimage/png=missing.desktop;viewer.desktop;\ntext/plain=edit.desktop\n\
                    image/png=gallery.desktop\n[Added Associations]\nimage/webp=viewer.desktop;\n\
                    [Removed Associations]\nimage/png=aaa-paint.desktop;\n";
        assert_eq!(choose(&hs, "image/png", Some(list)).unwrap().id, "viewer.desktop", "mimeapps.list default");
        assert_eq!(ids(handlers_for(&hs, "image/png", Some(list))), ["viewer.desktop", "gallery.desktop"]);
        assert_eq!(choose(&hs, "image/webp", Some(list)).unwrap().id, "viewer.desktop", "added association");
        assert_eq!(choose(&hs, "TEXT/PLAIN", Some(list)).unwrap().id, "edit.desktop");
    }

    #[test]
    fn opening() {
        let hs = handlers();
        let file = |mode| Meta { kind: Kind::File, size: 1, mode, ..Meta::default() };
        let dir = Meta { kind: Kind::Dir, ..Meta::default() };
        assert_eq!(open_action("/a/b/", &dir, mime::DIRECTORY, None, &hs, None), Open::Folder("/a/b".into()));
        assert_eq!(
            open_action("/n.txt", &file(0o644), mime::TEXT, Some(b"hi"), &hs, None),
            Open::With(vec!["code".into(), "/n.txt".into()])
        );
        assert_eq!(
            open_action("/x.pdf", &file(0o644), "application/pdf", None, &hs, None),
            Open::NoApp { mime: "application/pdf" }
        );
        // Programs and scripts with the execute bit run.
        assert_eq!(
            open_action("/bin/p", &file(0o755), mime::EXECUTABLE, None, &hs, None),
            Open::Launch(vec!["/bin/p".into()])
        );
        assert_eq!(
            open_action("/run.sh", &file(0o700), mime::SHELLSCRIPT, Some(b"#!/bin/sh\n"), &hs, None),
            Open::Launch(vec!["/run.sh".into()])
        );
        // Without it they are files.
        assert_eq!(
            open_action("/run.sh", &file(0o644), mime::SHELLSCRIPT, Some(b"#!/bin/sh\n"), &hs, None),
            Open::With(vec!["code".into(), "/run.sh".into()])
        );
        // A desktop entry launches its Exec only with the execute bit.
        let d = b"[Desktop Entry]\nType=Application\nName=Game\nExec=game --full %U\n";
        assert_eq!(
            open_action("/g.desktop", &file(0o755), mime::DESKTOP, Some(d), &hs, None),
            Open::Launch(vec!["game".into(), "--full".into()])
        );
        assert_eq!(
            open_action("/g.desktop", &file(0o644), mime::DESKTOP, Some(d), &hs, None),
            Open::With(vec!["code".into(), "/g.desktop".into()])
        );
    }

    #[test]
    fn opening_from_a_file_system() {
        let fs = MemFs::new().with(&[("/h/a.txt", "text"), ("/h/prog", "\x7fELF\x02\x01"), ("/h/d/", "")]);
        fs.set_mode("/h/prog", 0o755).unwrap();
        let hs = handlers();
        assert_eq!(open(&fs, "/h/d", &hs, None).unwrap(), Open::Folder("/h/d".into()));
        assert_eq!(open(&fs, "/h/a.txt", &hs, None).unwrap(), Open::With(vec!["code".into(), "/h/a.txt".into()]));
        assert_eq!(open(&fs, "/h/prog", &hs, None).unwrap(), Open::Launch(vec!["/h/prog".into()]));
        assert_eq!(open(&fs, "/h/none", &hs, None), Err(FsError::NotFound));
    }
}
