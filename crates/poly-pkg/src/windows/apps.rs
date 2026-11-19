//! What the launcher gets from a Windows installer (ADR 0230).
//!
//! After an installer has run, Poly reads what Windows' own shell would show:
//!
//! - the **shortcuts** it left in the Start menu (`C:\ProgramData\Microsoft\
//!   Windows\Start Menu\Programs`, the user's own) and on the desktop (the
//!   public one, the user's own). Each shortcut to a program becomes a
//!   launcher entry, a desktop file in the user's applications folder:
//!   `Exec` is the shortcut's target, a path of Poly's file system, and its
//!   arguments; the icon is the program's own (its PE icon resources, as
//!   ADR 0216 did for Chrome);
//! - the app's **`Uninstall` key** (Learn, "Uninstall Registry Key":
//!   `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\<app>` with
//!   `DisplayName`, `DisplayVersion`, `Publisher`, `UninstallString`,
//!   `InstallLocation`, `DisplayIcon`; the 32-bit view under `WOW6432Node`;
//!   `HKCU` for a per-user install). Its values go into the desktop file, so
//!   the Applications folder names the publisher and version and offers
//!   **Uninstall**, which runs `UninstallString`.
//!
//! The registry is read from Poly's hive files (`poly_registry::hive`), which
//! the kernel writes when an NT process ends (ADR 0066).

use crate::windows::shlink::{self, Link};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use poly_registry::hive::{self, KeyImage};

/// Where `C:` is in Poly's file system (ADR 0067, 0073).
pub const C_ROOT: &str = "/nt/c";
/// The registry's hive files, next to `C:` on the disk that keeps them
/// (`<disk>/poly/nt/registry`, ADR 0066): `C:`'s parent, through the link.
pub const HIVE_DIR: &str = "/nt/c/../registry";
/// The hive files of `HKLM\SOFTWARE` and of Poly's user.
pub const SOFTWARE_HIVE: &str = "SOFTWARE.hive";
pub const USER_HIVE: &str = "USER.hive";

/// The user's name on the NT personality (`poly_registry::defaults`).
pub const USER: &str = poly_registry::defaults::POLY_USER_NAME;

/// The folders whose shortcuts become launcher entries, as Windows paths:
/// the common Start menu, the user's, the public desktop, the user's
/// (Learn, "KNOWNFOLDERID": `FOLDERID_CommonPrograms`, `FOLDERID_Programs`,
/// `FOLDERID_PublicDesktop`, `FOLDERID_Desktop`).
pub fn link_folders() -> [String; 4] {
    [
        "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs".to_string(),
        alloc::format!("C:\\Users\\{USER}\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs"),
        "C:\\Users\\Public\\Desktop".to_string(),
        alloc::format!("C:\\Users\\{USER}\\Desktop"),
    ]
}

/// The prefix of the desktop file IDs Poly writes for Windows apps.
pub const ID_PREFIX: &str = "poly-windows-";
/// The keys of such a desktop file that say where it came from.
pub const LINK_KEY: &str = "X-Poly-Windows-Link";
pub const TARGET_KEY: &str = "X-Poly-Windows-Target";
pub const UNINSTALL_KEY: &str = "X-Poly-Windows-Uninstall";
pub const PUBLISHER_KEY: &str = "X-Poly-Windows-Publisher";
pub const VERSION_KEY: &str = "X-Poly-Windows-Version";
pub const REGISTRY_KEY: &str = "X-Poly-Windows-Key";

/// The file system as this module reads it.
pub trait Files {
    /// The names in a directory (empty if it is missing).
    fn list(&self, dir: &str) -> Vec<String>;
    /// Whether `path` is a directory.
    fn is_dir(&self, path: &str) -> bool;
    /// Whether `path` is there (any kind).
    fn exists(&self, path: &str) -> bool;
}

/// `name` in `dir`, as it is spelt there: Windows' names ignore case.
fn find_in(files: &dyn Files, dir: &str, name: &str) -> Option<String> {
    let names = files.list(dir);
    if names.iter().any(|n| n == name) {
        return Some(name.to_string());
    }
    names.into_iter().find(|n| n.eq_ignore_ascii_case(name) || n.to_lowercase() == name.to_lowercase())
}

/// The path in Poly's file system of the Windows path `dos` on `C:`, each
/// component found without regard to case; `None` when it is on another
/// drive or a part of it is missing.
pub fn vfs_path(dos: &str, files: &dyn Files) -> Option<String> {
    let dos = dos.trim().trim_matches('"');
    let rest = dos.strip_prefix("C:").or_else(|| dos.strip_prefix("c:"))?;
    let mut path = C_ROOT.to_string();
    for part in rest.split(['\\', '/']).filter(|p| !p.is_empty() && *p != ".") {
        if part == ".." {
            if path.len() > C_ROOT.len() {
                path.truncate(path.rfind('/').unwrap_or(C_ROOT.len()));
            }
            continue;
        }
        let found = find_in(files, &path, part)?;
        path.push('/');
        path.push_str(&found);
    }
    Some(path)
}

/// The Windows path of a path under [`C_ROOT`].
pub fn dos_path(vfs: &str) -> Option<String> {
    let rest = vfs.strip_prefix(C_ROOT)?;
    if !rest.is_empty() && !rest.starts_with('/') {
        return None;
    }
    Some(alloc::format!("C:{}", if rest.is_empty() { "\\".to_string() } else { rest.replace('/', "\\") }))
}

/// `%NAME%` references of the environment Poly's NT processes get
/// (`poly_registry::defaults`, ADR 0073), expanded; unknown names stay.
pub fn expand(text: &str) -> String {
    let profile = alloc::format!("C:\\Users\\{USER}");
    let value = |name: &str| -> Option<String> {
        Some(match name.to_ascii_lowercase().as_str() {
            "programfiles" | "programw6432" => "C:\\Program Files".into(),
            "programfiles(x86)" => "C:\\Program Files (x86)".into(),
            "commonprogramfiles" | "commonprogramw6432" => "C:\\Program Files\\Common Files".into(),
            "commonprogramfiles(x86)" => "C:\\Program Files (x86)\\Common Files".into(),
            "systemroot" | "windir" => "C:\\Windows".into(),
            "systemdrive" | "homedrive" => "C:".into(),
            "programdata" | "allusersprofile" => "C:\\ProgramData".into(),
            "public" => "C:\\Users\\Public".into(),
            "userprofile" => profile.clone(),
            "homepath" => alloc::format!("\\Users\\{USER}"),
            "appdata" => alloc::format!("{profile}\\AppData\\Roaming"),
            "localappdata" => alloc::format!("{profile}\\AppData\\Local"),
            _ => return None,
        })
    };
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => match value(&after[..end]) {
                Some(v) => {
                    out.push_str(&v);
                    rest = &after[end + 1..];
                }
                None => {
                    out.push('%');
                    rest = after;
                }
            },
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// A Windows command line split into arguments (Learn, "Parsing C++
/// command-line arguments"): white space separates; `"` starts and ends a
/// quoted part; `2n` backslashes before a `"` are `n` and the quote counts,
/// `2n + 1` are `n` and a literal `"`; other backslashes are themselves; in
/// a quoted part `""` is a literal `"`.
pub fn split_command_line(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut have = false;
    let mut quoted = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' if !quoted => {
                if have {
                    out.push(core::mem::take(&mut cur));
                    have = false;
                }
                i += 1;
            }
            '\\' => {
                let mut n = 0;
                while i < chars.len() && chars[i] == '\\' {
                    n += 1;
                    i += 1;
                }
                if i < chars.len() && chars[i] == '"' {
                    cur.extend(core::iter::repeat_n('\\', n / 2));
                    if n % 2 == 1 {
                        cur.push('"');
                        i += 1;
                    }
                } else {
                    cur.extend(core::iter::repeat_n('\\', n));
                }
                have = true;
            }
            '"' => {
                if quoted && chars.get(i + 1) == Some(&'"') {
                    cur.push('"');
                    i += 2;
                } else {
                    quoted = !quoted;
                    i += 1;
                }
                have = true;
            }
            c => {
                cur.push(c);
                have = true;
                i += 1;
            }
        }
    }
    if have {
        out.push(cur);
    }
    out
}

/// One argument of a desktop file's `Exec` (Desktop Entry Specification,
/// "The Exec key"): quoted when it has a reserved character, with `"`,
/// `` ` ``, `$` and `\` escaped inside the quotes; `%` doubled.
pub fn exec_arg(arg: &str) -> String {
    const RESERVED: &[char] =
        &[' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')', '`'];
    let arg = arg.replace('%', "%%");
    if !arg.is_empty() && !arg.contains(RESERVED) {
        return arg;
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The slug of a launcher entry's name: lower-case letters and digits,
/// `-` between words (as an AppImage's, `crate::appimage::slug`).
pub fn slug(name: &str) -> String {
    crate::appimage::slug(name)
}

/// The desktop file ID of the Windows app called `name`.
pub fn id_of(name: &str) -> String {
    alloc::format!("{ID_PREFIX}{}.desktop", slug(name))
}

/// Whether `id` is one of Poly's Windows app entries.
pub fn is_windows_id(id: &str) -> bool {
    id.strip_prefix(ID_PREFIX)
        .and_then(|s| s.strip_suffix(".desktop"))
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
}

/// Where an entry's icon goes (`data_home` is the user's data folder).
pub fn icon_path(data_home: &str, id: &str) -> String {
    let stem = id.strip_suffix(".desktop").unwrap_or(id);
    alloc::format!("{}/poly/windows/{stem}.png", data_home.trim_end_matches('/'))
}

/// A launcher entry made from a shortcut.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// Desktop file ID ([`id_of`]).
    pub id: String,
    /// The shortcut's name (its file name without `.lnk`).
    pub name: String,
    /// The shortcut's description.
    pub comment: String,
    /// The program (a path in Poly's file system) and its arguments.
    pub argv: Vec<String>,
    /// The target as a Windows path.
    pub target: String,
    /// The folder it starts in, if the shortcut names one that is there.
    pub working_dir: Option<String>,
    /// The file the icon is read from (Poly's path) and the icon index.
    pub icon_source: (String, i32),
    /// The shortcut file (Poly's path).
    pub link: String,
}

/// Whether a file name is a program the launcher starts (`.exe`).
fn is_program(path: &str) -> bool {
    path.len() > 4 && path.is_char_boundary(path.len() - 4) && path[path.len() - 4..].eq_ignore_ascii_case(".exe")
}

/// The launcher entry for the shortcut at `link_path` (a path in Poly's
/// file system), or why there is none.
pub fn entry_for(link_path: &str, link: &Link, files: &dyn Files) -> Result<Entry, String> {
    let file = link_path.rsplit('/').next().unwrap_or(link_path);
    let name = if shlink::is_link_name(file) { &file[..file.len() - 4] } else { file };
    if name.to_ascii_lowercase().starts_with("uninstall") {
        return Err(alloc::format!("{name}: an uninstaller, not an app"));
    }
    let dir = link_path.rsplit_once('/').map_or("", |(d, _)| d);
    let link_dir = dos_path(dir).unwrap_or_default();
    let target = shlink::program(link, &link_dir).ok_or_else(|| alloc::format!("{name}: names no program"))?;
    let target = expand(&target);
    if !is_program(&target) {
        return Err(alloc::format!("{name}: {target} is not a program"));
    }
    let program = vfs_path(&target, files).ok_or_else(|| alloc::format!("{name}: {target} is not there"))?;
    let mut argv = alloc::vec![program.clone()];
    argv.extend(split_command_line(&expand(&link.arguments)));
    let working_dir = Some(expand(&link.working_dir))
        .filter(|w| !w.is_empty())
        .and_then(|w| vfs_path(&w, files))
        .filter(|w| files.is_dir(w));
    let icon_source = Some(expand(&link.icon_location))
        .filter(|i| !i.is_empty())
        .and_then(|i| vfs_path(&i, files))
        .map_or_else(|| (program.clone(), 0), |i| (i, link.icon_index));
    Ok(Entry {
        id: id_of(name),
        name: name.to_string(),
        comment: link.description.clone(),
        argv,
        target,
        working_dir,
        icon_source,
        link: link_path.to_string(),
    })
}

/// The entries of several shortcuts: one per program and arguments (a
/// Start menu entry and a desktop shortcut of the same app are one), the
/// first kept; two apps with one slug get the second a number.
pub fn dedup(entries: Vec<Entry>) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for mut e in entries {
        if out.iter().any(|o| o.argv == e.argv) {
            continue;
        }
        let mut n = 2;
        let base = e.id.trim_end_matches(".desktop").to_string();
        while out.iter().any(|o| o.id == e.id) {
            e.id = alloc::format!("{base}-{n}.desktop");
            n += 1;
        }
        out.push(e);
    }
    out
}

/// An app's `Uninstall` key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Uninstall {
    /// Where it is: `HKLM`, `HKLM (32-bit)` or `HKCU`, and the subkey's name.
    pub view: &'static str,
    pub key: String,
    pub display_name: String,
    pub display_version: String,
    pub publisher: String,
    pub uninstall_string: String,
    pub install_location: String,
    pub display_icon: String,
}

/// The key at `path` (components separated by `\`) below `root`, names
/// compared without case.
fn subkey<'k>(root: &'k KeyImage, path: &str) -> Option<&'k KeyImage> {
    let mut key = root;
    for part in path.split('\\').filter(|p| !p.is_empty()) {
        let want: Vec<u16> = part.encode_utf16().collect();
        key = key.subkeys.iter().find(|k| poly_registry::tree::fold(&k.name) == poly_registry::tree::fold(&want))?;
    }
    Some(key)
}

/// A value of `key` as text (`REG_SZ`, `REG_EXPAND_SZ`), or a `REG_DWORD`.
fn text_value(key: &KeyImage, name: &str) -> Option<String> {
    let want: Vec<u16> = name.encode_utf16().collect();
    let v = key.values.iter().find(|v| poly_registry::tree::fold(&v.name) == poly_registry::tree::fold(&want))?;
    match v.kind {
        // REG_SZ, REG_EXPAND_SZ (winnt.h).
        1 | 2 => {
            let units: Vec<u16> = v.data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
            Some(String::from_utf16_lossy(&units[..end]))
        }
        // REG_DWORD.
        4 => v.data.get(..4).map(|d| alloc::format!("{}", u32::from_le_bytes([d[0], d[1], d[2], d[3]]))),
        _ => None,
    }
}

const UNINSTALL_PATH: &str = "Microsoft\\Windows\\CurrentVersion\\Uninstall";

/// The `Uninstall` keys of the hive files' bytes (either may be missing or
/// unreadable): the ones with a `DisplayName` that are not
/// `SystemComponent`s.
pub fn uninstall_entries(software: Option<&[u8]>, user: Option<&[u8]>) -> Vec<Uninstall> {
    let mut out = Vec::new();
    let software = software.and_then(|b| hive::decode(b).ok());
    let user = user.and_then(|b| hive::decode(b).ok());
    let mut places: Vec<(&'static str, &KeyImage)> = Vec::new();
    if let Some(h) = &software {
        if let Some(k) = subkey(&h.root, UNINSTALL_PATH) {
            places.push(("HKLM", k));
        }
        if let Some(k) = subkey(&h.root, &alloc::format!("WOW6432Node\\{UNINSTALL_PATH}")) {
            places.push(("HKLM (32-bit)", k));
        }
    }
    if let Some(h) = &user
        && let Some(k) = subkey(&h.root, &alloc::format!("Software\\{UNINSTALL_PATH}"))
    {
        places.push(("HKCU", k));
    }
    for (view, parent) in places {
        for k in &parent.subkeys {
            let get = |n: &str| text_value(k, n).unwrap_or_default();
            if get("DisplayName").trim().is_empty() || get("SystemComponent") == "1" {
                continue;
            }
            out.push(Uninstall {
                view,
                key: String::from_utf16_lossy(&k.name),
                display_name: get("DisplayName").trim().to_string(),
                display_version: get("DisplayVersion").trim().to_string(),
                publisher: get("Publisher").trim().to_string(),
                uninstall_string: get("UninstallString").trim().to_string(),
                install_location: get("InstallLocation").trim().to_string(),
                display_icon: get("DisplayIcon").trim().to_string(),
            });
        }
    }
    out
}

/// The entries of `after` that `before` did not have (by view and key).
pub fn new_uninstall_entries(before: &[Uninstall], after: Vec<Uninstall>) -> Vec<Uninstall> {
    after.into_iter().filter(|a| !before.iter().any(|b| b.view == a.view && b.key == a.key)).collect()
}

fn starts_with_ignoring_case(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s.is_char_boundary(prefix.len()) && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// The folder of a Windows path.
fn dos_dir(path: &str) -> &str {
    path.rsplit_once('\\').map_or("", |(d, _)| d)
}

/// The program of an `Uninstall` key's `DisplayIcon` (`path[,index]`,
/// perhaps quoted).
fn icon_program(display_icon: &str) -> String {
    let s = display_icon.trim();
    let s = match s.rsplit_once(',') {
        Some((p, i)) if i.trim().parse::<i32>().is_ok() => p,
        _ => s,
    };
    expand(s.trim().trim_matches('"'))
}

/// The program of an `UninstallString` as a Windows path: the quoted first
/// argument, or, unquoted, everything up to the first `.exe`.
fn uninstall_program(uninstall: &str) -> Option<String> {
    let text = expand(uninstall.trim());
    if text.starts_with('"') {
        return split_command_line(&text).into_iter().next();
    }
    let lower = text.to_ascii_lowercase();
    let end = lower.find(".exe")? + 4;
    Some(text[..end].to_string())
}

/// The `Uninstall` key that belongs to the app whose program is `target`
/// (a Windows path), of the keys the installer made: the one whose
/// `InstallLocation` holds it, or whose `DisplayIcon` is it, or whose
/// uninstaller sits next to it; else the only one there is.
pub fn registration_for<'u>(target: &str, made: &'u [Uninstall]) -> Option<&'u Uninstall> {
    let target = expand(target);
    let holds = |u: &&Uninstall| {
        let loc = expand(u.install_location.trim_end_matches('\\'));
        (!loc.is_empty() && starts_with_ignoring_case(&target, &alloc::format!("{loc}\\")))
            || icon_program(&u.display_icon).eq_ignore_ascii_case(&target)
            || uninstall_program(&u.uninstall_string).is_some_and(|p| {
                // Its folder, not a drive's root.
                let dir = dos_dir(&p);
                dir.contains('\\') && starts_with_ignoring_case(&target, &alloc::format!("{dir}\\"))
            })
    };
    made.iter().find(holds).or(if made.len() == 1 { made.first() } else { None })
}

/// The program and arguments an `UninstallString` runs, with the program as
/// a path in Poly's file system. The string is a command line whose program
/// may be unquoted and hold spaces: the longest start of it that is a file
/// is the program.
pub fn uninstall_argv(uninstall: &str, files: &dyn Files) -> Option<Vec<String>> {
    let text = expand(uninstall.trim());
    if text.starts_with('"') {
        let mut args = split_command_line(&text);
        if args.is_empty() {
            return None;
        }
        let program = vfs_path(&args[0], files).filter(|p| files.exists(p))?;
        args[0] = program;
        return Some(args);
    }
    let words: Vec<&str> = text.split(' ').collect();
    for n in (1..=words.len()).rev() {
        let candidate = words[..n].join(" ");
        if let Some(p) = vfs_path(&candidate, files).filter(|p| files.exists(p) && !files.is_dir(p)) {
            let mut argv = alloc::vec![p];
            argv.extend(split_command_line(&words[n..].join(" ")));
            return Some(argv);
        }
    }
    None
}

/// The desktop file of `entry` (with its icon file, if one was written, and
/// its app's `Uninstall` key, if it has one).
pub fn desktop_file(entry: &Entry, icon: Option<&str>, registration: Option<&Uninstall>) -> String {
    let clean = |v: &str| v.replace(['\n', '\r'], " ");
    let mut out = String::from("[Desktop Entry]\nType=Application\n");
    out.push_str(&alloc::format!("Name={}\n", clean(&entry.name)));
    let comment = if entry.comment.is_empty() {
        registration.map_or_else(|| "A Windows app".to_string(), |r| clean(&r.display_name))
    } else {
        clean(&entry.comment)
    };
    out.push_str(&alloc::format!("Comment={comment}\n"));
    let exec: Vec<String> = entry.argv.iter().map(|a| exec_arg(a)).collect();
    out.push_str(&alloc::format!("Exec={}\n", exec.join(" ")));
    if let Some(dir) = &entry.working_dir {
        out.push_str(&alloc::format!("Path={dir}\n"));
    }
    if let Some(icon) = icon {
        out.push_str(&alloc::format!("Icon={icon}\n"));
    }
    out.push_str("Keywords=windows;\n");
    out.push_str(&alloc::format!("{LINK_KEY}={}\n", clean(&entry.link)));
    out.push_str(&alloc::format!("{TARGET_KEY}={}\n", clean(&entry.target)));
    if let Some(r) = registration {
        out.push_str(&alloc::format!("{REGISTRY_KEY}={}\\{}\n", r.view, clean(&r.key)));
        if !r.uninstall_string.is_empty() {
            out.push_str(&alloc::format!("{UNINSTALL_KEY}={}\n", clean(&r.uninstall_string)));
        }
        if !r.publisher.is_empty() {
            out.push_str(&alloc::format!("{PUBLISHER_KEY}={}\n", clean(&r.publisher)));
        }
        if !r.display_version.is_empty() {
            out.push_str(&alloc::format!("{VERSION_KEY}={}\n", clean(&r.display_version)));
        }
    }
    out
}

/// The value of `key` in a desktop file's text.
pub fn desktop_value<'t>(text: &'t str, key: &str) -> Option<&'t str> {
    crate::appimage::desktop_keys(text).into_iter().find(|(k, _)| *k == key).map(|(_, v)| v)
}

/// A shortcut file as it was seen: its path and a checksum of its bytes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Seen {
    pub path: String,
    pub sum: u32,
}

/// The shortcuts that are new or changed in `after`.
pub fn changed(before: &[Seen], after: &[Seen]) -> Vec<String> {
    after.iter().filter(|a| !before.contains(a)).map(|a| a.path.clone()).collect()
}

/// A checksum of a shortcut's bytes.
pub fn checksum(bytes: &[u8]) -> u32 {
    poly_png::crc32(&[bytes])
}
