//! The logic of our `shlwapi.dll` (ADR 0155, step 9): the handful of "shell lightweight
//! utility" functions Chrome's browser delay-loads. Without the DLL the browser ends itself
//! with a breakpoint (the delay load of `SHLWAPI.dll` fails).
//!
//! Sources, all Microsoft Learn (`shlwapi.h`): "PathMatchSpecW", "PathMatchSpecExW" (what a
//! pattern list is: patterns separated by semicolons, leading spaces of each ignored),
//! "PathFindExtensionW", "PathFindFileNameW", "PathRemoveExtensionW", "IsOS" (the table of
//! `OS_*` values and which of them are always FALSE or TRUE), "QISearch" and "QITAB". None of
//! it was observed on Windows.

use alloc::vec::Vec;

const BACKSLASH: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;
const COLON: u16 = b':' as u16;
const DOT: u16 = b'.' as u16;
const SPACE: u16 = b' ' as u16;
const STAR: u16 = b'*' as u16;
const QUESTION: u16 = b'?' as u16;
const SEMICOLON: u16 = b';' as u16;

/// Upper case of one UTF-16 unit, for the case-insensitive comparison of file names (the simple
/// one-to-one mappings; a unit without one is itself).
fn upper(c: u16) -> u16 {
    match char::from_u32(u32::from(c)) {
        Some(ch) => {
            let mut up = ch.to_uppercase();
            match (up.next(), up.next()) {
                (Some(u), None) if (u as u32) <= 0xFFFF => u as u32 as u16,
                _ => c,
            }
        }
        None => c,
    }
}

/// Whether `name` matches one MS-DOS wildcard pattern: `*` stands for any run of characters
/// (also none), `?` for any one character, everything else for itself without regard to case.
pub fn wildcard_match(name: &[u16], pattern: &[u16]) -> bool {
    // The classic two-pointer match with one backtrack point (the last `*`).
    let (mut n, mut p) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && pattern[p] == STAR {
            star = Some((p, n));
            p += 1;
        } else if p < pattern.len() && (pattern[p] == QUESTION || upper(pattern[p]) == upper(name[n])) {
            n += 1;
            p += 1;
        } else if let Some((sp, sn)) = star {
            // Let the last `*` take one more character.
            star = Some((sp, sn + 1));
            p = sp + 1;
            n = sn + 1;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == STAR)
}

/// `PathMatchSpecW(pszFile, pszSpec)`: whether the name matches the specification. Learn's
/// page for the function shows one pattern (`*.c`); `PathMatchSpecExW` says how a list is
/// written (patterns separated by semicolons, leading spaces of each ignored) and that the
/// older function is that one with its defaults. Poly reads the specification as such a list:
/// the name matches if it matches any pattern. An empty pattern matches nothing but an empty
/// name.
pub fn path_match_spec(file: &[u16], spec: &[u16]) -> bool {
    spec.split(|&c| c == SEMICOLON).any(|pattern| {
        let start = pattern.iter().position(|&c| c != SPACE).unwrap_or(pattern.len());
        wildcard_match(file, &pattern[start..])
    })
}

/// `PathFindExtensionW`: the index of the `.` that starts the extension, or `path.len()` (the
/// terminating NUL) when there is none. Learn: "Returns the address of the "." that precedes
/// the extension within pszPath if an extension is found, or the address of the terminating
/// null character otherwise"; a valid extension "cannot contain a space", and it belongs to the
/// last component, so a `.` before a backslash or a space does not count.
pub fn find_extension(path: &[u16]) -> usize {
    let mut dot = None;
    for (i, &c) in path.iter().enumerate() {
        match c {
            DOT => dot = Some(i),
            BACKSLASH | SPACE => dot = None,
            _ => {}
        }
    }
    dot.unwrap_or(path.len())
}

/// `PathFindFileNameW`: the index where the file name starts. Learn: "Returns a pointer to the
/// address of the string if successful, or a pointer to the beginning of the path otherwise";
/// its examples: `c:\path\file` gives `file`, `c:\path` gives `path`, `c:\path\` gives `path\`,
/// `c:\` and `c:` and `path` give themselves. So the name starts after the last `\`, `/` or `:`
/// that is followed by a character that is not `\` or `/`.
pub fn find_file_name(path: &[u16]) -> usize {
    let mut start = 0;
    for i in 0..path.len() {
        let sep = matches!(path[i], BACKSLASH | SLASH | COLON);
        if sep && i + 1 < path.len() && !matches!(path[i + 1], BACKSLASH | SLASH) {
            start = i + 1;
        }
    }
    start
}

/// `OS_*` values of `IsOS` (`shlwapi.h`).
pub const OS_NT: u32 = 1;
pub const OS_WIN2000ORGREATER: u32 = 7;
pub const OS_PROFESSIONAL: u32 = 20;
pub const OS_PERSONALTERMINALSERVER: u32 = 25;
pub const OS_DOMAINMEMBER: u32 = 28;
pub const OS_ANYSERVER: u32 = 29;

/// `IsOS(dwOS)` for Poly, which shows programs a Windows workstation (`VER_NT_WORKSTATION`, the
/// product type of `GetVersionExW`) that is in no domain, with one user at the console, in a
/// 64-bit process. From Learn's table: `OS_NT` is always TRUE; `OS_WIN2000ORGREATER`,
/// `OS_PROFESSIONAL` (a workstation) and `OS_PERSONALTERMINALSERVER` (Windows XP or a successor,
/// Home or Professional) hold; the values the table calls always FALSE, the server, embedded,
/// terminal-server, tablet, media-center and domain values do not, nor does an unknown value.
pub fn is_os(os: u32) -> bool {
    matches!(os, OS_NT | OS_WIN2000ORGREATER | OS_PROFESSIONAL | OS_PERSONALTERMINALSERVER)
}

/// `IID_IUnknown` (`unknwn.h`), as the 16 bytes it has in memory.
pub const IID_IUNKNOWN: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];

/// `QISearch`: the offset of the interface `riid` asks for, from a `QITAB` given as its
/// `(interface id, offset)` entries up to the terminating one. Learn: "If the requested
/// interface is IUnknown, then QISearch uses the first entry"; otherwise the entry with the
/// matching id; `None` is `E_NOINTERFACE`.
pub fn qi_search(table: &[([u8; 16], u32)], riid: &[u8; 16]) -> Option<u32> {
    if let Some(&(_, offset)) = table.iter().find(|(iid, _)| iid == riid) {
        return Some(offset);
    }
    if *riid == IID_IUNKNOWN {
        return table.first().map(|&(_, offset)| offset);
    }
    None
}

/// A UTF-16 string from text (tests and tables).
pub fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(file: &str, spec: &str) -> bool {
        path_match_spec(&w(file), &w(spec))
    }

    #[test]
    fn wildcards_match_as_ms_dos_patterns_do() {
        assert!(m("main.c", "*.c"));
        assert!(m("MAIN.C", "*.c"));
        assert!(!m("main.cpp", "*.c"));
        assert!(m("main.cpp", "*.c*"));
        assert!(m("a", "?"));
        assert!(!m("ab", "?"));
        assert!(!m("", "?"));
        assert!(m("", "*"));
        assert!(m("anything.at.all", "*"));
        assert!(m("chrome.exe", "chrome.exe"));
        assert!(m("chrome.exe", "c*e.e?e"));
        assert!(m("abcabc", "*abc"));
        assert!(m("xaybzc", "*a*b*c"));
        assert!(!m("xaybz", "*a*b*c"));
        assert!(m("C:\\dir\\file.txt", "*.txt"));
        assert!(m("", ""));
        assert!(!m("a", ""));
        // Case folding beyond ASCII.
        assert!(m("\u{e4}.txt", "\u{c4}.TXT"));
    }

    #[test]
    fn a_specification_is_a_list_of_patterns() {
        assert!(m("a.txt", "*.c;*.txt"));
        assert!(m("a.c", "*.c;*.txt"));
        assert!(!m("a.h", "*.c;*.txt"));
        // Leading spaces of each pattern are ignored.
        assert!(m("a.txt", "*.c; *.txt"));
        assert!(m("a.c", "  *.c;*.txt"));
        // Trailing spaces are part of the pattern.
        assert!(!m("a.c", "*.c ;*.txt"));
    }

    #[test]
    fn the_extension_is_the_last_dot_of_the_last_component() {
        let ext = |s: &str| {
            let p = w(s);
            alloc::string::String::from_utf16(&p[find_extension(&p)..]).unwrap()
        };
        assert_eq!(ext("C:\\TEST\\file.txt"), ".txt");
        assert_eq!(ext("file.tar.gz"), ".gz");
        assert_eq!(ext("C:\\dir.d\\file"), "");
        assert_eq!(ext("file"), "");
        assert_eq!(ext("file."), ".");
        assert_eq!(ext("a.b c"), "");
        assert_eq!(ext(""), "");
    }

    #[test]
    fn the_file_name_follows_learns_examples() {
        let name = |s: &str| {
            let p = w(s);
            alloc::string::String::from_utf16(&p[find_file_name(&p)..]).unwrap()
        };
        assert_eq!(name("c:\\path\\file"), "file");
        assert_eq!(name("c:\\path"), "path");
        assert_eq!(name("c:\\path\\"), "path\\");
        assert_eq!(name("c:\\"), "c:\\");
        assert_eq!(name("c:"), "c:");
        assert_eq!(name("path"), "path");
        assert_eq!(name("a/b/c.txt"), "c.txt");
        assert_eq!(name(""), "");
    }

    #[test]
    fn is_os_describes_a_workstation() {
        assert!(is_os(OS_NT));
        assert!(is_os(OS_WIN2000ORGREATER));
        assert!(is_os(OS_PROFESSIONAL));
        assert!(is_os(OS_PERSONALTERMINALSERVER));
        // OS_WINDOWS and the values Learn calls always FALSE.
        for os in [0, 2, 3, 5, 6, 16, 17, 18, 19, 27, 31] {
            assert!(!is_os(os), "{os}");
        }
        assert!(!is_os(OS_ANYSERVER));
        assert!(!is_os(OS_DOMAINMEMBER));
        assert!(!is_os(30));
        assert!(!is_os(1000));
    }

    #[test]
    fn qi_search_finds_the_entry_or_the_first_for_iunknown() {
        let a = [1u8; 16];
        let b = [2u8; 16];
        let table = [(a, 8u32), (b, 16)];
        assert_eq!(qi_search(&table, &b), Some(16));
        assert_eq!(qi_search(&table, &a), Some(8));
        assert_eq!(qi_search(&table, &IID_IUNKNOWN), Some(8));
        assert_eq!(qi_search(&table, &[3u8; 16]), None);
        assert_eq!(qi_search(&[], &IID_IUNKNOWN), None);
        // IUnknown listed itself: its own entry wins.
        assert_eq!(qi_search(&[(a, 8), (IID_IUNKNOWN, 0)], &IID_IUNKNOWN), Some(0));
    }
}
