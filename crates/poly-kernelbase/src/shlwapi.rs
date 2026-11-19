//! The logic of our `shlwapi.dll` (ADR 0155, step 9): the handful of "shell lightweight
//! utility" functions Chrome's browser delay-loads. Without the DLL the browser ends itself
//! with a breakpoint (the delay load of `SHLWAPI.dll` fails).
//!
//! Sources: `PathMatchSpecW` follows its spec sheet
//! (`specs/windows/shlwapi.dll/PathMatchSpecW.yaml`). The others are Microsoft Learn's
//! (`shlwapi.h`): "PathFindExtensionW", "PathFindFileNameW", "PathRemoveExtensionW", "IsOS" (the
//! table of `OS_*` values and which of them are always FALSE or TRUE), "QISearch" and "QITAB";
//! those were not observed on Windows.

use alloc::vec::Vec;

const BACKSLASH: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;
const COLON: u16 = b':' as u16;
const DOT: u16 = b'.' as u16;
const SPACE: u16 = b' ' as u16;
const STAR: u16 = b'*' as u16;
const QUESTION: u16 = b'?' as u16;
const SEMICOLON: u16 = b';' as u16;

/// The character two letters are compared by: the sheet's rule `case`. Letters match "where the
/// two letters are a simple upper/lower pair of the old Windows upper-case table". That table
/// has the one-to-one pairs whose lower-case letter is the lower case of its upper-case letter
/// (so the long s, the dotless i, the micro sign, the final sigma and the title-case digraphs
/// stand alone), and none of the pairs Unicode added later (Georgian, Cherokee).
fn fold(c: u16) -> u16 {
    if matches!(c, 0x10D0..=0x10FF | 0x13A0..=0x13FF | 0x1C90..=0x1CBF | 0xAB70..=0xABBF) {
        return c;
    }
    let Some(ch) = char::from_u32(u32::from(c)) else { return c };
    let mut up = ch.to_uppercase();
    let (Some(u), None) = (up.next(), up.next()) else { return c };
    let mut back = u.to_lowercase();
    match (back.next(), back.next()) {
        (Some(l), None) if l == ch && (u as u32) <= 0xFFFF => u as u32 as u16,
        _ => c,
    }
}

/// `pattern` without the spaces (U+0020) it starts with.
fn skip_spaces(pattern: &[u16]) -> &[u16] {
    let start = pattern.iter().position(|&c| c != SPACE).unwrap_or(pattern.len());
    &pattern[start..]
}

/// Whether `rest` (what follows a wildcard) is `.?` or `.*`: an extension that may be missing.
fn optional_extension(rest: &[u16]) -> bool {
    rest.len() == 2 && rest[0] == DOT && (rest[1] == QUESTION || rest[1] == STAR)
}

/// Whether `name` matches one pattern, by the rules of
/// `specs/windows/shlwapi.dll/PathMatchSpecW.yaml` (named in the comments).
pub fn wildcard_match(name: &[u16], pattern: &[u16]) -> bool {
    // star_dot_star: a pattern that is exactly `*.*` matches whatever is left, also nothing.
    if pattern == [STAR, DOT, STAR] {
        return true;
    }
    let Some((&first, raw_rest)) = pattern.split_first() else {
        // empty_pattern: matches only the empty name.
        return name.is_empty();
    };
    match first {
        STAR => {
            // star: a `*` that is the last character matches whatever is left.
            if raw_rest.is_empty() {
                return true;
            }
            // missing_extension: at the name's end, `.?` or `.*` right behind the wildcard.
            if name.is_empty() && optional_extension(raw_rest) {
                return true;
            }
            // spaces_after_star: spaces right after the `*` are dropped, and spaces right after
            // a dot that follows the `*` directly; a `*` followed only by spaces matches nothing.
            let after = skip_spaces(raw_rest);
            if after.is_empty() {
                return false;
            }
            let mut rest: Vec<u16> = Vec::with_capacity(after.len());
            if after[0] == DOT {
                rest.push(DOT);
                rest.extend_from_slice(skip_spaces(&after[1..]));
            } else {
                rest.extend_from_slice(after);
            }
            // star: the rest must match from a character that is still there.
            (0..name.len()).any(|i| wildcard_match(&name[i..], &rest))
        }
        QUESTION => match name.split_first() {
            Some((_, tail)) => wildcard_match(tail, raw_rest),
            // question_mark: at the name's end a `?` matches if it is the pattern's last
            // character; missing_extension: or if `.?` / `.*` is all that follows.
            None => raw_rest.is_empty() || optional_extension(raw_rest),
        },
        // literals: every other character matches itself, without regard to case.
        c => match name.split_first() {
            Some((&n, tail)) => fold(n) == fold(c) && wildcard_match(tail, raw_rest),
            None => false,
        },
    }
}

/// `PathMatchSpecW(pszFile, pszSpec)` (`specs/windows/shlwapi.dll/PathMatchSpecW.yaml`): an
/// empty spec matches every name (rule `empty_spec`); otherwise the spec is cut at every
/// semicolon, the spaces each pattern starts with are dropped (`leading_spaces`), and the name
/// matches if it matches any pattern (`list`).
pub fn path_match_spec(file: &[u16], spec: &[u16]) -> bool {
    spec.is_empty() || spec.split(|&c| c == SEMICOLON).any(|pattern| wildcard_match(file, skip_spaces(pattern)))
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

    /// The examples of the sheet, rule by rule.
    #[test]
    fn path_match_spec_follows_the_sheet() {
        let yes = |file: &str, spec: &str| assert!(m(file, spec), "{file:?} ~ {spec:?} should match");
        let no = |file: &str, spec: &str| assert!(!m(file, spec), "{file:?} ~ {spec:?} should not match");
        // empty_spec, list, leading_spaces, empty_pattern.
        yes("a", "");
        yes("", "");
        yes("a.c", "*.c;*.txt");
        yes("a.txt", "*.c;*.txt");
        no("a.h", "*.c;*.txt");
        yes("a;b", "a*b");
        no("a;b", "a;b");
        yes("a.txt", " *.txt");
        yes("a.doc", "*.txt; *.doc");
        no("a.txt", "*.txt ");
        yes("a.doc", "*.txt ;*.doc");
        no("a.txt", "*.txt ;*.doc");
        yes("", ";");
        no("a", ";");
        yes("", " ");
        no("a", " ");
        // literals.
        no("file", "file.");
        no("file.", "file");
        yes("MAIN.C", "*.c");
        // question_mark.
        yes("a", "a?");
        yes("ab", "a?");
        yes("ab", "???");
        yes("abc", "???");
        yes("", "?");
        no("a", "a??");
        yes("ac", "a??");
        // star.
        yes("abc", "abc*");
        yes("", "*");
        no("a", "a**");
        no("", "**");
        no("", "*?");
        no("", "?*");
        no("abc", "abc*?");
        yes("ab", "a**");
        yes("ab", "a*?");
        // star_dot_star.
        yes("file", "*.*");
        yes("CURRENT", "*.*");
        yes("", "*.*");
        yes("f", "f*.*");
        yes("file", "f*.*");
        yes("file.", "*.*.*");
        yes(".file", "*.*.*");
        no("file", "*.*.*");
        // missing_extension.
        for spec in ["*.?", "?.?", "?.*"] {
            yes("", spec);
        }
        for spec in ["f*.?", "f?.?", "f?.*"] {
            yes("f", spec);
        }
        for spec in ["*.?.?", "*.*.?"] {
            yes("f.", spec);
            yes(".", spec);
        }
        no("f", "*.?");
        no("", ".?");
        no("", "*. ?");
        // spaces_after_star.
        for spec in ["* .txt", "*. txt", "*  .txt"] {
            yes("a.txt", spec);
            no("a. txt", spec);
        }
        yes("ab.txt", "a*. txt");
        no("a", "* ");
        no("", "* ");
        yes("a .txt", "a .txt");
        no("a.txt", "a .txt");
        no("a.txt", "?. txt");
        yes("a. txt", "?. txt");
        // examples_extension.
        for name in ["file.txt", ".txt", "a.b.txt", "dir\\file.txt"] {
            yes(name, "*.txt");
        }
        for name in ["file", "file.", "file.txt.bak", "file.tx", "file.txtx"] {
            no(name, "*.txt");
        }
        for name in ["file.txt", "file.txt.bak", "file.tx"] {
            yes(name, "*.???");
        }
        yes("file.", "file.*");
        yes("file.txt", "file.*");
        no("file", "file.*");
    }

    /// The sheet's rule `case`: the pairs that match and the letters that stand alone.
    #[test]
    fn letters_match_as_the_old_upper_case_table_pairs_them() {
        let pair = |a: u16, b: u16| path_match_spec(&[a], &[b]) && path_match_spec(&[b], &[a]);
        for (a, b) in [
            (0x61, 0x41),
            (0xE4, 0xC4),
            (0xFF, 0x178),
            (0x3C3, 0x3A3),
            (0x3BC, 0x39C),
            (0x436, 0x416),
            (0x1C6, 0x1C4),
            (0xFF41, 0xFF21),
            (0x69, 0x49),
        ] {
            assert!(pair(a, b), "{a:#x} and {b:#x} should match");
        }
        for (a, b) in [
            (0x17F, 0x73),
            (0x17F, 0x53),
            (0x131, 0x69),
            (0x131, 0x49),
            (0x130, 0x69),
            (0x131, 0x130),
            (0x212A, 0x6B),
            (0x212A, 0x4B),
            (0xB5, 0x3BC),
            (0xB5, 0x39C),
            (0x3C2, 0x3C3),
            (0x3C2, 0x3A3),
            (0x1C5, 0x1C6),
            (0x1C5, 0x1C4),
            (0x10D0, 0x1C90),
            (0x13A0, 0xAB70),
            (0xFF41, 0x61),
            (0xDF, 0x1E9E),
        ] {
            assert!(!pair(a, b), "{a:#x} and {b:#x} should not match");
        }
        // A sharp s is not "ss".
        assert!(!m("\u{df}", "ss"));
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
