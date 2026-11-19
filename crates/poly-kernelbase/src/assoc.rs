//! File and protocol associations: what `shlwapi!AssocQueryStringW` answers and what
//! `shell32!SHCreateAssociationRegistration`'s object (`IApplicationAssociationRegistration`)
//! reads. The registry is reached through [`Registry`], so the rules are tested here without one.
//!
//! Sources: the sheets `specs/windows/shlwapi.dll/AssocQueryStringW.yaml` and
//! `specs/windows/shell32.dll/SHCreateAssociationRegistration.yaml`; Learn, "AssocQueryStringW",
//! "ASSOCF", "ASSOCSTR", "File Types and File Associations", "Application Registration" (App
//! Paths, `Applications`), "Merged View of HKEY_CLASSES_ROOT", "Default Programs" (the user's
//! choice under `UrlAssociations` and `FileExts`); the constants are `shlwapi.h`'s,
//! `shobjidl_core.h`'s and `winerror.h`'s.
//!
//! The rules:
//! * A class key is read in the merged view, value by value: the user's classes
//!   (`HKCU\Software\Classes`) first, then the machine's (`HKLM\Software\Classes`), so a value
//!   only the machine's key has is still found (the sheet's `hkcu_over_hklm`).
//! * `pszAssoc` is a ProgID, an extension (its default value names the ProgID; without one, the
//!   first `OpenWithProgids` value whose ProgID exists) or, with `ASSOCF_OPEN_BYEXENAME`, an
//!   executable name (`Applications\<exe>`, and `App Paths\<exe>` for the program).
//! * An extension nothing opens falls back to Windows' "open with" program unless
//!   `ASSOCF_INIT_IGNOREUNKNOWN` is set; an unknown ProgID is no association.

use alloc::vec::Vec;

/// `ASSOCF_OPEN_BYEXENAME` / `ASSOCF_INIT_BYEXENAME` (`shlwapi.h`).
pub const ASSOCF_OPEN_BYEXENAME: u32 = 0x2;
/// `ASSOCF_NOUSERSETTINGS` (`shlwapi.h`): the user's classes are not read.
pub const ASSOCF_NOUSERSETTINGS: u32 = 0x10;
/// `ASSOCF_NOTRUNCATE` (`shlwapi.h`).
pub const ASSOCF_NOTRUNCATE: u32 = 0x20;
/// `ASSOCF_INIT_IGNOREUNKNOWN` (`shlwapi.h`).
pub const ASSOCF_INIT_IGNOREUNKNOWN: u32 = 0x400;

/// `ASSOCSTR_COMMAND` (`shlwapi.h`).
pub const ASSOCSTR_COMMAND: u32 = 1;
/// `ASSOCSTR_EXECUTABLE` (`shlwapi.h`).
pub const ASSOCSTR_EXECUTABLE: u32 = 2;
/// `ASSOCSTR_FRIENDLYDOCNAME` (`shlwapi.h`).
pub const ASSOCSTR_FRIENDLYDOCNAME: u32 = 3;
/// `ASSOCSTR_FRIENDLYAPPNAME` (`shlwapi.h`).
pub const ASSOCSTR_FRIENDLYAPPNAME: u32 = 4;
/// `ASSOCSTR_DEFAULTICON` (`shlwapi.h`).
pub const ASSOCSTR_DEFAULTICON: u32 = 15;
/// `ASSOCSTR_MAX` (`shlwapi.h`): the first value that is no `ASSOCSTR`.
pub const ASSOCSTR_MAX: u32 = 24;

/// `ASSOCIATIONTYPE` (`shobjidl_core.h`): `AT_FILEEXTENSION`.
pub const AT_FILEEXTENSION: u32 = 0;
/// `ASSOCIATIONLEVEL` (`shobjidl_core.h`): `AL_MACHINE`, `AL_EFFECTIVE`, `AL_USER`.
pub const AL_MACHINE: u32 = 0;
pub const AL_EFFECTIVE: u32 = 1;
pub const AL_USER: u32 = 2;

/// `HRESULT_FROM_WIN32(ERROR_NO_ASSOCIATION)` (`winerror.h`: 1155).
pub const E_NO_ASSOCIATION: u32 = 0x8007_0483;
/// `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`.
pub const E_FILE_NOT_FOUND: u32 = 0x8007_0002;
/// `E_INVALIDARG` (`winerror.h`).
pub const E_INVALIDARG: u32 = 0x8007_0057;

/// The part of the registry a key path is relative to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hive {
    /// `HKCU\Software\Classes`.
    UserClasses,
    /// `HKLM\Software\Classes`.
    MachineClasses,
    /// `HKEY_LOCAL_MACHINE`.
    Machine,
    /// `HKEY_CURRENT_USER`.
    User,
}

/// What the rules read from the registry and the file system.
pub trait Registry {
    /// A string value (`REG_SZ`, or `REG_EXPAND_SZ` with its variables expanded) of a key,
    /// without its NUL; `name` `None` is the default value. `None` when the key or the value is
    /// not there.
    fn value(&self, hive: Hive, key: &[u16], name: Option<&[u16]>) -> Option<Vec<u16>>;
    /// Whether the key exists.
    fn has_key(&self, hive: Hive, key: &[u16]) -> bool;
    /// The names of the values of a key, in the registry's order (empty when it is not there).
    fn value_names(&self, hive: Hive, key: &[u16]) -> Vec<Vec<u16>>;
    /// The description of a program (the `FileDescription` of its version resource), if it has
    /// one.
    fn file_description(&self, path: &[u16]) -> Option<Vec<u16>>;
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn join(parts: &[&[u16]]) -> Vec<u16> {
    let mut k = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            k.push(u16::from(b'\\'));
        }
        k.extend_from_slice(p);
    }
    k
}

fn lower(c: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&c) { c + 32 } else { c }
}

fn eq_nocase(a: &[u16], b: &str) -> bool {
    let b = w(b);
    a.len() == b.len() && a.iter().zip(&b).all(|(&x, &y)| lower(x) == lower(y))
}

/// Reads class keys in the merged view, value by value (user first unless `machine_only`).
struct Classes<'a, R: Registry> {
    reg: &'a R,
    machine_only: bool,
}

impl<R: Registry> Classes<'_, R> {
    fn value(&self, key: &[u16], name: Option<&str>) -> Option<Vec<u16>> {
        let name = name.map(w);
        let name = name.as_deref();
        if !self.machine_only
            && let Some(v) = self.reg.value(Hive::UserClasses, key, name)
        {
            return Some(v);
        }
        self.reg.value(Hive::MachineClasses, key, name)
    }

    fn has_key(&self, key: &[u16]) -> bool {
        (!self.machine_only && self.reg.has_key(Hive::UserClasses, key)) || self.reg.has_key(Hive::MachineClasses, key)
    }

    fn value_names(&self, key: &[u16]) -> Vec<Vec<u16>> {
        let mut names = if self.machine_only { Vec::new() } else { self.reg.value_names(Hive::UserClasses, key) };
        for n in self.reg.value_names(Hive::MachineClasses, key) {
            if !names
                .iter()
                .any(|m: &Vec<u16>| m.len() == n.len() && m.iter().zip(&n).all(|(&a, &b)| lower(a) == lower(b)))
            {
                names.push(n);
            }
        }
        names
    }
}

/// What `pszAssoc` stands for.
#[derive(Debug, PartialEq, Eq)]
enum Class {
    /// A class key (a ProgID, an extension's own key, or `Applications\<exe>`).
    Key(Vec<u16>),
    /// An executable named with `ASSOCF_OPEN_BYEXENAME` (its `Applications` key, maybe absent).
    Exe { name: Vec<u16>, key: Vec<u16> },
    /// An extension nothing opens: Windows' "open with" program.
    OpenWith,
}

fn resolve<R: Registry>(c: &Classes<R>, assoc: &[u16], flags: u32) -> Result<Class, u32> {
    if flags & ASSOCF_OPEN_BYEXENAME != 0 {
        let key = join(&[&w("Applications"), assoc]);
        return Ok(Class::Exe { name: assoc.to_vec(), key });
    }
    if assoc.first() == Some(&u16::from(b'.')) {
        if let Some(progid) = c.value(assoc, None).filter(|p| !p.is_empty())
            && c.has_key(&progid)
        {
            return Ok(Class::Key(progid));
        }
        let open_with = join(&[assoc, &w("OpenWithProgids")]);
        if let Some(progid) = c.value_names(&open_with).into_iter().find(|p| !p.is_empty() && c.has_key(p)) {
            return Ok(Class::Key(progid));
        }
        // An extension key with verbs of its own.
        if c.has_key(&join(&[assoc, &w("shell")])) {
            return Ok(Class::Key(assoc.to_vec()));
        }
        return if flags & ASSOCF_INIT_IGNOREUNKNOWN != 0 { Err(E_NO_ASSOCIATION) } else { Ok(Class::OpenWith) };
    }
    if !assoc.is_empty() && c.has_key(assoc) { Ok(Class::Key(assoc.to_vec())) } else { Err(E_NO_ASSOCIATION) }
}

/// The program of a command line: the text between its first pair of quotation marks, or up to
/// its first space.
pub fn command_program(cmd: &[u16]) -> Vec<u16> {
    let quote = u16::from(b'"');
    let cmd = &cmd[cmd.iter().take_while(|&&c| c == u16::from(b' ')).count()..];
    if cmd.first() == Some(&quote) {
        let rest = &cmd[1..];
        return rest[..rest.iter().position(|&c| c == quote).unwrap_or(rest.len())].to_vec();
    }
    cmd[..cmd.iter().position(|&c| c == u16::from(b' ')).unwrap_or(cmd.len())].to_vec()
}

/// The file name of a path, without its directory.
fn file_name(path: &[u16]) -> &[u16] {
    let start = path.iter().rposition(|&c| c == u16::from(b'\\') || c == u16::from(b'/')).map_or(0, |i| i + 1);
    &path[start..]
}

/// `AssocQueryStringW(flags, what, assoc, extra)`: the string, or the `HRESULT` of the failure.
/// `windows` is the Windows directory (`C:\Windows`), where the "open with" program is.
pub fn query_string<R: Registry>(
    reg: &R,
    flags: u32,
    what: u32,
    assoc: &[u16],
    verb: Option<&[u16]>,
    windows: &[u16],
) -> Result<Vec<u16>, u32> {
    if what == 0 || what >= ASSOCSTR_MAX {
        return Err(E_INVALIDARG);
    }
    let c = Classes { reg, machine_only: flags & ASSOCF_NOUSERSETTINGS != 0 };
    let class = resolve(&c, assoc, flags)?;
    let open_with = || {
        let mut p = windows.to_vec();
        p.extend(w("\\system32\\OpenWith.exe"));
        p
    };
    let key = match &class {
        Class::Key(k) | Class::Exe { key: k, .. } => k.clone(),
        Class::OpenWith => Vec::new(),
    };
    let command = || -> Option<Vec<u16>> {
        if class == Class::OpenWith {
            let mut cmd = open_with();
            cmd.extend(w(" \"%1\""));
            return Some(cmd);
        }
        let verb = match verb.filter(|v| !v.is_empty()) {
            Some(v) => v.to_vec(),
            None => c.value(&join(&[&key, &w("shell")]), None).filter(|v| !v.is_empty()).unwrap_or_else(|| w("open")),
        };
        c.value(&join(&[&key, &w("shell"), &verb, &w("command")]), None)
    };
    let program = || -> Option<Vec<u16>> {
        if let Class::Exe { name, .. } = &class {
            let app_paths = join(&[&w("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths"), name]);
            if let Some(p) = reg.value(Hive::Machine, &app_paths, None).filter(|p| !p.is_empty()) {
                return Some(command_program(&p));
            }
        }
        command().map(|cmd| command_program(&cmd)).filter(|p| !p.is_empty())
    };
    let found = match what {
        ASSOCSTR_COMMAND => command(),
        ASSOCSTR_EXECUTABLE => program(),
        ASSOCSTR_FRIENDLYDOCNAME if class != Class::OpenWith => c.value(&key, None).filter(|v| !v.is_empty()),
        ASSOCSTR_FRIENDLYAPPNAME => program().map(|p| {
            // The program's description; one without (not recorded) is named by its file name.
            reg.file_description(&p).filter(|d| !d.is_empty()).unwrap_or_else(|| file_name(&p).to_vec())
        }),
        ASSOCSTR_DEFAULTICON if class != Class::OpenWith => {
            c.value(&join(&[&key, &w("DefaultIcon")]), None).filter(|v| !v.is_empty())
        }
        _ => None,
    };
    found.ok_or(E_NO_ASSOCIATION)
}

/// What [`query_string`]'s text gives the caller's buffer of `capacity` units (counting the NUL):
/// the `HRESULT`, the count to report, and how many units of the text to write (followed by a
/// NUL). `notruncate` is `ASSOCF_NOTRUNCATE`; `has_buffer` is false for a NULL `pszOut`.
pub fn buffer_result(len: usize, capacity: u32, has_buffer: bool, notruncate: bool) -> (u32, u32, Option<usize>) {
    /// `S_OK`, `S_FALSE`, `E_POINTER`, `HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER)` (`winerror.h`).
    const S_OK: u32 = 0;
    const S_FALSE: u32 = 1;
    const E_POINTER: u32 = 0x8000_4003;
    const E_INSUFFICIENT_BUFFER: u32 = 0x8007_007A;
    let needed = len as u32 + 1;
    if !has_buffer {
        return (S_FALSE, needed, None);
    }
    if needed <= capacity {
        return (S_OK, needed, Some(len));
    }
    if notruncate || capacity == 0 {
        return (E_POINTER, needed, None);
    }
    (E_INSUFFICIENT_BUFFER, needed, Some(capacity as usize - 1))
}

/// The types Poly's desktop opens itself with the web browser Poly ships (not a Windows program):
/// they have a default even when no Windows program registered one, as `http`, `https`, `.htm`
/// and `.html` have on a Windows installation (the sheet's `query_current_default_effective`).
const DESKTOP_DEFAULTS: &[(&str, &str)] = &[
    ("http", "Poly.Desktop.Url"),
    ("https", "Poly.Desktop.Url"),
    (".htm", "Poly.Desktop.Html"),
    (".html", "Poly.Desktop.Html"),
];

/// The ProgID the user chose for `query` (the `UserChoice` key Learn's "Default Programs"
/// describes), if any.
fn user_choice<R: Registry>(reg: &R, query: &[u16], kind: u32) -> Option<Vec<u16>> {
    let base = if kind == AT_FILEEXTENSION {
        w("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts")
    } else {
        w("Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations")
    };
    reg.value(Hive::User, &join(&[&base, query, &w("UserChoice")]), Some(&w("ProgId"))).filter(|p| !p.is_empty())
}

/// `IApplicationAssociationRegistration::QueryCurrentDefault(query, type, level)`: the default's
/// ProgID (`AL_EFFECTIVE`, `AL_USER`) or its open command (`AL_MACHINE`), or the `HRESULT` of
/// the failure. Types other than `AT_FILEEXTENSION` are looked up as protocols are.
pub fn current_default<R: Registry>(reg: &R, query: &[u16], kind: u32, level: u32) -> Result<Vec<u16>, u32> {
    if level > AL_USER {
        return Err(E_INVALIDARG);
    }
    if query.is_empty() {
        return Err(E_NO_ASSOCIATION);
    }
    if level == AL_USER {
        return user_choice(reg, query, kind).ok_or(E_NO_ASSOCIATION);
    }
    let machine = Classes { reg, machine_only: level == AL_MACHINE };
    // The class the registry names for it: an extension's ProgID, or a protocol's own key.
    let class = if kind == AT_FILEEXTENSION {
        machine.value(query, None).filter(|p| !p.is_empty() && machine.has_key(p))
    } else {
        machine.value(query, Some("URL Protocol")).map(|_| query.to_vec())
    };
    if level == AL_MACHINE {
        let class = class.ok_or(E_NO_ASSOCIATION)?;
        return machine.value(&join(&[&class, &w("shell\\open\\command")]), None).ok_or(E_NO_ASSOCIATION);
    }
    if let Some(p) = user_choice(reg, query, kind).or(class) {
        return Ok(p);
    }
    let builtin = DESKTOP_DEFAULTS
        .iter()
        .find(|(name, _)| eq_nocase(query, name) && (kind == AT_FILEEXTENSION) == name.starts_with('.'));
    builtin.map(|(_, progid)| w(progid)).ok_or(E_NO_ASSOCIATION)
}

/// `IApplicationAssociationRegistration::SetAppAsDefaultAll(app)`: the application must be
/// registered (`HKLM\SOFTWARE\RegisteredApplications`); nothing is changed (the sheet:
/// `S_OK`, else `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`).
pub fn set_app_as_default_all<R: Registry>(reg: &R, app: &[u16]) -> u32 {
    let registered = reg.value(Hive::Machine, &w("SOFTWARE\\RegisteredApplications"), Some(app)).is_some();
    if registered && !app.is_empty() { 0 } else { E_FILE_NOT_FOUND }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::collections::BTreeMap;
    use std::string::String;

    /// A registry of (hive, key in lower case) -> values (name in lower case, "" = default).
    #[derive(Default)]
    struct Fake {
        keys: BTreeMap<(u8, String), Vec<(String, String)>>,
        descriptions: Vec<(String, String)>,
    }

    fn hive_id(h: Hive) -> u8 {
        h as u8
    }

    impl Fake {
        fn set(&mut self, hive: Hive, key: &str, name: &str, value: &str) {
            // Every parent exists too.
            let mut path = String::new();
            for part in key.split('\\') {
                if !path.is_empty() {
                    path.push('\\');
                }
                path.push_str(&part.to_lowercase());
                self.keys.entry((hive_id(hive), path.clone())).or_default();
            }
            let values = self.keys.get_mut(&(hive_id(hive), key.to_lowercase())).unwrap();
            values.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
            values.push((name.into(), value.into()));
        }
        fn remove(&mut self, hive: Hive, key: &str) {
            let k = key.to_lowercase();
            self.keys.retain(|(h, p), _| !(*h == hive_id(hive) && (*p == k || p.starts_with(&(k.clone() + "\\")))));
        }
    }

    fn s(u: &[u16]) -> String {
        String::from_utf16(u).unwrap()
    }

    impl Registry for Fake {
        fn value(&self, hive: Hive, key: &[u16], name: Option<&[u16]>) -> Option<Vec<u16>> {
            let name = name.map(s).unwrap_or_default();
            let values = self.keys.get(&(hive_id(hive), s(key).to_lowercase()))?;
            values.iter().find(|(n, _)| n.eq_ignore_ascii_case(&name)).map(|(_, v)| w(v))
        }
        fn has_key(&self, hive: Hive, key: &[u16]) -> bool {
            self.keys.contains_key(&(hive_id(hive), s(key).to_lowercase()))
        }
        fn value_names(&self, hive: Hive, key: &[u16]) -> Vec<Vec<u16>> {
            self.keys
                .get(&(hive_id(hive), s(key).to_lowercase()))
                .map(|v| v.iter().map(|(n, _)| w(n)).collect())
                .unwrap_or_default()
        }
        fn file_description(&self, path: &[u16]) -> Option<Vec<u16>> {
            let p = s(path);
            self.descriptions.iter().find(|(f, _)| f.eq_ignore_ascii_case(&p)).map(|(_, d)| w(d))
        }
    }

    const CMD: &str = "\"C:\\Windows\\System32\\cmd.exe\" /c exit \"%1\"";

    fn chrome_like() -> Fake {
        let mut r = Fake::default();
        let m = Hive::MachineClasses;
        r.set(m, "PolySpecB4HTML", "", "PolySpecB4 HTML Document");
        r.set(m, "PolySpecB4HTML\\DefaultIcon", "", "C:\\Windows\\System32\\cmd.exe,0");
        r.set(m, "PolySpecB4HTML\\shell\\open\\command", "", CMD);
        r.set(m, ".polyspecb4a", "", "PolySpecB4HTML");
        r.set(m, ".polyspecb4b\\OpenWithProgids", "PolySpecB4HTML", "");
        r.set(m, ".polyspecb4c\\OpenWithProgids", "PolySpecB4Other", "");
        r.set(m, ".polyspecb4c\\OpenWithProgids", "PolySpecB4HTML", "");
        r.descriptions.push(("C:\\Windows\\System32\\cmd.exe".into(), "Windows Command Processor".into()));
        r
    }

    fn q(r: &Fake, flags: u32, what: u32, assoc: &str, verb: Option<&str>) -> Result<String, u32> {
        let verb = verb.map(w);
        query_string(r, flags, what, &w(assoc), verb.as_deref(), &w("C:\\Windows")).map(|v| s(&v))
    }

    #[test]
    fn a_progid_and_its_extensions() {
        let r = chrome_like();
        for a in ["PolySpecB4HTML", ".polyspecb4a", ".polyspecb4b"] {
            assert_eq!(q(&r, 0, ASSOCSTR_COMMAND, a, Some("open")).as_deref(), Ok(CMD), "{a}");
            assert_eq!(q(&r, 0, ASSOCSTR_COMMAND, a, None).as_deref(), Ok(CMD), "{a}");
            assert_eq!(q(&r, 0, ASSOCSTR_EXECUTABLE, a, None).as_deref(), Ok("C:\\Windows\\System32\\cmd.exe"));
            assert_eq!(q(&r, 0, ASSOCSTR_FRIENDLYDOCNAME, a, None).as_deref(), Ok("PolySpecB4 HTML Document"));
            assert_eq!(q(&r, 0, ASSOCSTR_FRIENDLYAPPNAME, a, None).as_deref(), Ok("Windows Command Processor"));
            assert_eq!(q(&r, 0, ASSOCSTR_DEFAULTICON, a, None).as_deref(), Ok("C:\\Windows\\System32\\cmd.exe,0"));
        }
        assert_eq!(q(&r, ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_COMMAND, ".polyspecb4b", None).as_deref(), Ok(CMD));
        // Two OpenWithProgids, the first one unregistered.
        assert_eq!(
            q(&r, ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_COMMAND, ".polyspecb4c", Some("open")).as_deref(),
            Ok(CMD)
        );
        assert_eq!(q(&r, 0, ASSOCSTR_COMMAND, "PolySpecB4HTML", Some("edit")), Err(E_NO_ASSOCIATION));
    }

    #[test]
    fn unknown_names() {
        let r = chrome_like();
        assert_eq!(
            q(&r, 0, ASSOCSTR_COMMAND, ".polyspecb4zz", Some("open")).as_deref(),
            Ok("C:\\Windows\\system32\\OpenWith.exe \"%1\"")
        );
        assert_eq!(q(&r, ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_COMMAND, ".polyspecb4zz", None), Err(E_NO_ASSOCIATION));
        assert_eq!(q(&r, 0, ASSOCSTR_COMMAND, "PolySpecB4Missing", None), Err(E_NO_ASSOCIATION));
        assert_eq!(
            q(&r, ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_COMMAND, "PolySpecB4Missing", None),
            Err(E_NO_ASSOCIATION)
        );
        assert_eq!(q(&r, 0, 99, "PolySpecB4HTML", None), Err(E_INVALIDARG));
        assert_eq!(q(&r, 0, 0, "PolySpecB4HTML", None), Err(E_INVALIDARG));
    }

    #[test]
    fn the_users_values_come_first_value_by_value() {
        let mut r = Fake::default();
        r.set(Hive::MachineClasses, "Both\\shell\\open\\command", "", "machine command");
        r.set(Hive::MachineClasses, "Both", "", "machine name");
        assert_eq!(q(&r, 0, ASSOCSTR_COMMAND, "Both", Some("open")).as_deref(), Ok("machine command"));
        r.set(Hive::UserClasses, "Both\\shell\\open\\command", "", "user command");
        assert_eq!(q(&r, 0, ASSOCSTR_COMMAND, "Both", Some("open")).as_deref(), Ok("user command"));
        assert_eq!(q(&r, 0, ASSOCSTR_FRIENDLYDOCNAME, "Both", None).as_deref(), Ok("machine name"));
        assert_eq!(q(&r, ASSOCF_NOUSERSETTINGS, ASSOCSTR_COMMAND, "Both", None).as_deref(), Ok("machine command"));
        r.set(Hive::UserClasses, "Both", "", "user name");
        assert_eq!(q(&r, 0, ASSOCSTR_FRIENDLYDOCNAME, "Both", None).as_deref(), Ok("user name"));
        r.remove(Hive::UserClasses, "Both");
        assert_eq!(q(&r, 0, ASSOCSTR_COMMAND, "Both", Some("open")).as_deref(), Ok("machine command"));
    }

    #[test]
    fn an_executable_by_name() {
        let mut r = chrome_like();
        r.set(
            Hive::MachineClasses,
            "Applications\\polyspecb4app.exe\\shell\\open\\command",
            "",
            "\"C:\\x\\a.exe\" \"%1\"",
        );
        r.set(
            Hive::Machine,
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\polyspecb4app.exe",
            "",
            "C:\\Windows\\System32\\cmd.exe",
        );
        let f = ASSOCF_OPEN_BYEXENAME;
        assert_eq!(
            q(&r, f, ASSOCSTR_COMMAND, "polyspecb4app.exe", Some("open")).as_deref(),
            Ok("\"C:\\x\\a.exe\" \"%1\"")
        );
        assert_eq!(
            q(&r, f, ASSOCSTR_EXECUTABLE, "polyspecb4app.exe", None).as_deref(),
            Ok("C:\\Windows\\System32\\cmd.exe")
        );
        assert_eq!(
            q(&r, f, ASSOCSTR_FRIENDLYAPPNAME, "polyspecb4app.exe", None).as_deref(),
            Ok("Windows Command Processor")
        );
        assert_eq!(q(&r, 0, ASSOCSTR_COMMAND, "polyspecb4app.exe", Some("open")), Err(E_NO_ASSOCIATION));
    }

    #[test]
    fn the_buffer_rules() {
        // 42 units of text: 43 with the NUL.
        assert_eq!(buffer_result(42, 300, true, false), (0, 43, Some(42)));
        assert_eq!(buffer_result(42, 43, true, false), (0, 43, Some(42)));
        assert_eq!(buffer_result(42, 8, true, false), (0x8007_007A, 43, Some(7)));
        assert_eq!(buffer_result(42, 8, true, true), (0x8000_4003, 43, None));
        assert_eq!(buffer_result(42, 0, false, false), (1, 43, None));
    }

    #[test]
    fn current_defaults() {
        let mut r = Fake::default();
        let cd = |r: &Fake, q: &str, t: u32, l: u32| current_default(r, &w(q), t, l).map(|v| s(&v));
        // Poly's desktop opens web pages and links.
        assert_eq!(cd(&r, "http", 1, AL_EFFECTIVE).as_deref(), Ok("Poly.Desktop.Url"));
        assert_eq!(cd(&r, "HTTPS", 1, AL_EFFECTIVE).as_deref(), Ok("Poly.Desktop.Url"));
        assert_eq!(cd(&r, ".html", 0, AL_EFFECTIVE).as_deref(), Ok("Poly.Desktop.Html"));
        assert_eq!(cd(&r, ".htm", 0, AL_EFFECTIVE).as_deref(), Ok("Poly.Desktop.Html"));
        assert_eq!(cd(&r, "http", 9, AL_EFFECTIVE).as_deref(), Ok("Poly.Desktop.Url"));
        assert_eq!(cd(&r, "http", 1, 9), Err(E_INVALIDARG));
        for l in [AL_MACHINE, AL_EFFECTIVE, AL_USER] {
            assert_eq!(cd(&r, ".polyspecb4", 0, l), Err(E_NO_ASSOCIATION));
            assert_eq!(cd(&r, "polyspecb4proto", 1, l), Err(E_NO_ASSOCIATION));
        }
        // A protocol registered machine-wide.
        r.set(Hive::MachineClasses, "PolySpecB4Proto", "URL Protocol", "");
        r.set(
            Hive::MachineClasses,
            "PolySpecB4Proto\\shell\\open\\command",
            "",
            "\"C:\\Windows\\System32\\cmd.exe\" /c exit",
        );
        assert_eq!(cd(&r, "polyspecb4proto", 1, AL_EFFECTIVE).as_deref(), Ok("polyspecb4proto"));
        assert_eq!(
            cd(&r, "polyspecb4proto", 1, AL_MACHINE).as_deref(),
            Ok("\"C:\\Windows\\System32\\cmd.exe\" /c exit")
        );
        assert_eq!(cd(&r, "polyspecb4proto", 1, AL_USER), Err(E_NO_ASSOCIATION));
        // The user's choice.
        r.set(
            Hive::User,
            "Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\http\\UserChoice",
            "ProgId",
            "ChromeHTML",
        );
        assert_eq!(cd(&r, "http", 1, AL_USER).as_deref(), Ok("ChromeHTML"));
        assert_eq!(cd(&r, "http", 1, AL_EFFECTIVE).as_deref(), Ok("ChromeHTML"));
    }

    #[test]
    fn set_as_default_all_needs_a_registered_application() {
        let mut r = Fake::default();
        assert_eq!(set_app_as_default_all(&r, &w("PolySpecB4App")), E_FILE_NOT_FOUND);
        r.set(Hive::Machine, "SOFTWARE\\RegisteredApplications", "PolySpecB4App", "Software\\Clients\\X\\Capabilities");
        assert_eq!(set_app_as_default_all(&r, &w("PolySpecB4App")), 0);
    }

    #[test]
    fn the_program_of_a_command() {
        assert_eq!(s(&command_program(&w(CMD))), "C:\\Windows\\System32\\cmd.exe");
        assert_eq!(s(&command_program(&w("C:\\a\\b.exe %1"))), "C:\\a\\b.exe");
        assert_eq!(s(&command_program(&w("\"C:\\a b\\c.exe"))), "C:\\a b\\c.exe");
    }
}
