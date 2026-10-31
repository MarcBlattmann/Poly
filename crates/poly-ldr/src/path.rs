//! Module names and the DLL search order (Learn: "Dynamic-link library
//! search order", the standard order for unpackaged applications with safe
//! DLL search mode; `LoadLibraryW` for the `.dll` extension rule).
//!
//! Paths here are DOS paths (`C:\Windows\System32\ntdll.dll`) in UTF-16;
//! [`nt_path`] makes the `\??\` form the file services take.

use alloc::vec::Vec;
use poly_ntobj::name::eq_ignore_case;

/// The directories the search uses, as DOS paths without a trailing `\`.
#[derive(Clone, Debug, Default)]
pub struct Dirs {
    /// The directory the executable was loaded from.
    pub app: Vec<u16>,
    /// `C:\Windows\System32`.
    pub system32: Vec<u16>,
    /// `C:\Windows\System` (the 16-bit system directory).
    pub system16: Vec<u16>,
    /// `C:\Windows`.
    pub windows: Vec<u16>,
    /// The current directory.
    pub current: Vec<u16>,
    /// The `PATH` directories.
    pub path: Vec<Vec<u16>>,
}

/// The known DLLs: always loaded from `System32` (Learn: "Known DLLs").
/// Poly's list is the DLLs Poly provides itself (ADR 0055, 0071).
pub const KNOWN_DLLS: &[&str] = &[
    "ntdll.dll",
    "kernel32.dll",
    "kernelbase.dll",
    "ucrtbase.dll",
    "msvcrt.dll",
    "advapi32.dll",
    "user32.dll",
    "gdi32.dll",
    "ws2_32.dll",
    "ole32.dll",
    "oleaut32.dll",
    "bcrypt.dll",
    "crypt32.dll",
    "shell32.dll",
    "combase.dll",
];

const BACKSLASH: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;
const DOT: u16 = b'.' as u16;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Whether the name contains a path separator.
pub fn has_path(name: &[u16]) -> bool {
    name.iter().any(|&c| c == BACKSLASH || c == SLASH)
}

/// The last component of a path.
pub fn base_name(path: &[u16]) -> &[u16] {
    match path.iter().rposition(|&c| c == BACKSLASH || c == SLASH) {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

/// The directory part of a path (without the trailing separator).
pub fn dir_name(path: &[u16]) -> &[u16] {
    match path.iter().rposition(|&c| c == BACKSLASH || c == SLASH) {
        Some(i) => &path[..i],
        None => &[],
    }
}

/// `LoadLibraryW`'s extension rule on the last component: a name without
/// an extension gets `.dll`; a trailing `.` means "no extension" and is
/// dropped.
pub fn with_extension(name: &[u16]) -> Vec<u16> {
    let mut n = name.to_vec();
    let base = base_name(name);
    if base.last() == Some(&DOT) {
        n.pop();
    } else if !base.contains(&DOT) {
        n.extend(wide(".dll"));
    }
    n
}

/// `dir\name`.
pub fn join(dir: &[u16], name: &[u16]) -> Vec<u16> {
    let mut p = dir.to_vec();
    if !p.is_empty() && p.last() != Some(&BACKSLASH) {
        p.push(BACKSLASH);
    }
    p.extend_from_slice(name);
    p
}

/// The NT path of a DOS path: `\??\` in front (an NT path stays as it is),
/// `/` turned into `\`.
pub fn nt_path(dos: &[u16]) -> Vec<u16> {
    let prefix = wide("\\??\\");
    let mut p: Vec<u16> = dos.iter().map(|&c| if c == SLASH { BACKSLASH } else { c }).collect();
    if !p.starts_with(&prefix) {
        let mut q = prefix;
        q.append(&mut p);
        p = q;
    }
    p
}

/// Whether two module names are the same (ignoring case).
pub fn same_name(a: &[u16], b: &[u16]) -> bool {
    eq_ignore_case(a, b)
}

/// Whether a base name is one of the [`KNOWN_DLLS`].
pub fn is_known(base: &[u16]) -> bool {
    KNOWN_DLLS.iter().any(|k| same_name(&wide(k), base))
}

/// Split a `;`-separated search path.
pub fn split_search_path(path: &[u16]) -> Vec<Vec<u16>> {
    path.split(|&c| c == u16::from(b';')).filter(|d| !d.is_empty()).map(|d| d.to_vec()).collect()
}

/// The files to try, in order, for a name without a path (already given
/// its extension): known DLLs only in `System32`; otherwise the search
/// path when one is given, else the standard directories.
pub fn candidates(name: &[u16], dirs: &Dirs, search_path: Option<&[u16]>) -> Vec<Vec<u16>> {
    if is_known(name) {
        return alloc::vec![join(&dirs.system32, name)];
    }
    let list: Vec<Vec<u16>> = match search_path {
        Some(sp) => split_search_path(sp),
        None => {
            let mut v = alloc::vec![
                dirs.app.clone(),
                dirs.system32.clone(),
                dirs.system16.clone(),
                dirs.windows.clone(),
                dirs.current.clone(),
            ];
            v.extend(dirs.path.iter().cloned());
            v
        }
    };
    let mut out: Vec<Vec<u16>> = Vec::new();
    for d in list.iter().filter(|d| !d.is_empty()) {
        let c = join(d, name);
        if !out.iter().any(|o| same_name(o, &c)) {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[u16]) -> alloc::string::String {
        alloc::string::String::from_utf16(v).unwrap()
    }

    fn dirs() -> Dirs {
        Dirs {
            app: wide("C:\\app"),
            system32: wide("C:\\Windows\\System32"),
            system16: wide("C:\\Windows\\System"),
            windows: wide("C:\\Windows"),
            current: wide("C:\\app"),
            path: alloc::vec![wide("C:\\tools"), wide("")],
        }
    }

    #[test]
    fn extension_rules() {
        assert_eq!(s(&with_extension(&wide("foo"))), "foo.dll");
        assert_eq!(s(&with_extension(&wide("foo.drv"))), "foo.drv");
        assert_eq!(s(&with_extension(&wide("foo."))), "foo");
        assert_eq!(s(&with_extension(&wide("C:\\x.y\\foo"))), "C:\\x.y\\foo.dll");
    }

    #[test]
    fn names_and_paths() {
        assert!(has_path(&wide("a\\b.dll")) && has_path(&wide("a/b.dll")) && !has_path(&wide("b.dll")));
        assert_eq!(s(base_name(&wide("C:\\a\\b.dll"))), "b.dll");
        assert_eq!(s(dir_name(&wide("C:\\a\\b.dll"))), "C:\\a");
        assert_eq!(s(&nt_path(&wide("C:/a/b.dll"))), "\\??\\C:\\a\\b.dll");
        assert_eq!(s(&nt_path(&wide("\\??\\C:\\b.dll"))), "\\??\\C:\\b.dll");
        assert!(same_name(&wide("NTDLL.DLL"), &wide("ntdll.dll")));
    }

    #[test]
    fn standard_order() {
        let c: alloc::vec::Vec<_> = candidates(&wide("mine.dll"), &dirs(), None).iter().map(|v| s(v)).collect();
        assert_eq!(
            c,
            [
                "C:\\app\\mine.dll",
                "C:\\Windows\\System32\\mine.dll",
                "C:\\Windows\\System\\mine.dll",
                "C:\\Windows\\mine.dll",
                "C:\\tools\\mine.dll",
            ],
            "the current directory equals the app's and is tried once"
        );
        let k: alloc::vec::Vec<_> = candidates(&wide("KERNEL32.dll"), &dirs(), None).iter().map(|v| s(v)).collect();
        assert_eq!(k, ["C:\\Windows\\System32\\KERNEL32.dll"]);
        let sp = wide("D:\\one;;D:\\two");
        let c: alloc::vec::Vec<_> = candidates(&wide("x.dll"), &dirs(), Some(&sp)).iter().map(|v| s(v)).collect();
        assert_eq!(c, ["D:\\one\\x.dll", "D:\\two\\x.dll"]);
    }
}
