//! File names: NT's UTF-16 names over the VFS's byte names, the checks a
//! name component passes, case-insensitive comparison and the wildcard
//! expressions of `NtQueryDirectoryFile`.
//!
//! # Encoding
//!
//! The VFS stores names as bytes, which Poly reads as UTF-8 (what Linux
//! programs write). An NT name goes to the VFS as UTF-8 (an unpaired
//! surrogate cannot be encoded and makes the name invalid); a VFS name comes
//! back as UTF-16, with every byte sequence that is not UTF-8 replaced by
//! U+FFFD. Such a file is listed but cannot be opened by that name.
//!
//! # Case
//!
//! Names compare without case by upcasing each UTF-16 unit with
//! `poly_ntobj::name::upcase` (ADR 0030's rule; to be confirmed by the spec
//! agent). How a lookup chooses among names that differ only in case is in
//! [`super::walk`].
//!
//! # Valid names
//!
//! Learn ("Naming Files, Paths, and Namespaces"): a name may not contain
//! `< > : " / \ | ? *` or characters 1–31, and a component is at most 255
//! characters (`MaximumComponentNameLength` of our volumes). NT paths are
//! not normalised: a `.` or `..` component is `STATUS_OBJECT_NAME_INVALID`
//! (Win32 removes them before calling `NtCreateFile`; to be confirmed by the
//! spec agent). Stream names (`file:stream`) are refused the same way: Poly
//! has no alternate data streams.

use super::consts::*;
use alloc::string::String;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::name::upcase;
use poly_ntobj::status::STATUS_OBJECT_NAME_INVALID;

/// The longest name component, in UTF-16 units.
pub const MAX_COMPONENT: usize = 255;

/// A name component as UTF-8 bytes, or `STATUS_OBJECT_NAME_INVALID` for a
/// name NT does not allow or the VFS cannot store.
pub fn component_to_vfs(name: &[u16]) -> Result<Vec<u8>, NtStatus> {
    check_component(name)?;
    let s: String = char::decode_utf16(name.iter().copied())
        .collect::<Result<String, _>>()
        .map_err(|_| STATUS_OBJECT_NAME_INVALID)?;
    Ok(s.into_bytes())
}

/// `name` without a `::$DATA` suffix (any case), the default data stream's
/// full name (Learn: "File Streams (Local File Systems)"); other names as
/// they are.
pub fn without_default_stream(name: &[u16]) -> &[u16] {
    let suffix: Vec<u16> = "::$DATA".encode_utf16().collect();
    match name.len().checked_sub(suffix.len()) {
        Some(at) if at > 0 && eq_fold(&name[at..], &suffix) => &name[..at],
        _ => name,
    }
}

/// Check one component of a path that is opened or created.
pub fn check_component(name: &[u16]) -> Result<(), NtStatus> {
    if name.is_empty() || name.len() > MAX_COMPONENT {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    if name == [b'.' as u16] || name == [b'.' as u16, b'.' as u16] {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    if name.iter().any(|&c| is_invalid_char(c)) {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    Ok(())
}

fn is_invalid_char(c: u16) -> bool {
    c < 0x20 || matches!(c, 0x22 | 0x2A | 0x2F | 0x3A | 0x3C | 0x3E | 0x3F | 0x5C | 0x7C)
}

/// A VFS name as NT shows it.
pub fn vfs_to_wide(name: &[u8]) -> Vec<u16> {
    String::from_utf8_lossy(name).encode_utf16().collect()
}

/// `a` and `b` are the same name without case.
pub fn eq_fold(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| upcase(x) == upcase(y))
}

/// The upcased form of a name, for keys.
pub fn fold(name: &[u16]) -> Vec<u16> {
    name.iter().map(|&c| upcase(c)).collect()
}

/// Split an NT path below a device (`\a\b`, or a relative `a\b`) into its
/// components. An empty component in the middle (`a\\b`) is
/// `STATUS_OBJECT_NAME_INVALID`; a trailing `\` is allowed and reported, so
/// the caller can demand a directory.
pub fn split_path(path: &[u16]) -> Result<(Vec<&[u16]>, bool), NtStatus> {
    let sep = b'\\' as u16;
    let body = path.strip_prefix(&[sep]).unwrap_or(path);
    if body.is_empty() {
        return Ok((Vec::new(), false));
    }
    let trailing = body.last() == Some(&sep);
    let body = if trailing { &body[..body.len() - 1] } else { body };
    let parts: Vec<&[u16]> = body.split(|&c| c == sep).collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    Ok((parts, trailing))
}

// ----- wildcards (Learn: "FsRtlIsNameInExpression", "ZwQueryDirectoryFile")

/// `DOS_STAR`, `DOS_QM` and `DOS_DOT` (`ntifs.h`): the expression
/// characters `FindFirstFile` translates `*.*`-style patterns into.
pub const DOS_STAR: u16 = b'<' as u16;
pub const DOS_QM: u16 = b'>' as u16;
pub const DOS_DOT: u16 = b'"' as u16;
const STAR: u16 = b'*' as u16;
const QM: u16 = b'?' as u16;
const DOT: u16 = b'.' as u16;

/// Whether `expr` has a wildcard (else it names one file).
pub fn has_wildcards(expr: &[u16]) -> bool {
    expr.iter().any(|&c| matches!(c, STAR | QM | DOS_STAR | DOS_QM | DOS_DOT))
}

/// Whether `expr` matches exactly the names equal to it without case: not
/// empty (which matches everything), no wildcard, not `.` or `..` (which
/// name the directory's own entries).
pub fn is_literal(expr: &[u16]) -> bool {
    !expr.is_empty() && !has_wildcards(expr) && expr != [DOT] && expr != [DOT, DOT]
}

/// Whether `name` matches the expression `expr`, without case, with the
/// documented meanings (Learn, "FsRtlIsNameInExpression"):
///
/// - `*` matches zero or more characters; `?` exactly one;
/// - `<` (`DOS_STAR`) matches zero or more characters up to the last `.`
///   of the name (it cannot consume that period);
/// - `>` (`DOS_QM`) matches one character, or nothing at a period or at the
///   end of the name (so `file>>>.txt` matches `file.txt`);
/// - `"` (`DOS_DOT`) matches a period, or nothing at the end of the name.
///
/// Everything else matches itself, compared upcased. An empty expression
/// matches everything (`FileName` absent).
pub fn matches(expr: &[u16], name: &[u16]) -> bool {
    if expr.is_empty() {
        return true;
    }
    let e: Vec<u16> = fold(expr);
    let n: Vec<u16> = fold(name);
    let last_dot = n.iter().rposition(|&c| c == DOT);
    // Dynamic programming over (expression position, name position):
    // `reach[j]` says the expression so far can consume exactly n[..j].
    let mut reach = alloc::vec![false; n.len() + 1];
    reach[0] = true;
    for &c in &e {
        let mut next = alloc::vec![false; n.len() + 1];
        for j in 0..=n.len() {
            if !reach[j] {
                continue;
            }
            match c {
                STAR => {
                    for slot in next.iter_mut().skip(j) {
                        *slot = true;
                    }
                }
                QM => {
                    if j < n.len() {
                        next[j + 1] = true;
                    }
                }
                DOS_STAR => {
                    // Up to (not past) the last period; with none, to the end.
                    let limit = match last_dot {
                        Some(d) if j <= d => d,
                        Some(_) => n.len(),
                        None => n.len(),
                    };
                    for slot in next.iter_mut().take(limit + 1).skip(j) {
                        *slot = true;
                    }
                }
                DOS_QM => {
                    if j == n.len() || n[j] == DOT {
                        next[j] = true;
                    } else {
                        next[j + 1] = true;
                    }
                }
                DOS_DOT => {
                    if j < n.len() && n[j] == DOT {
                        next[j + 1] = true;
                    }
                    if j == n.len() {
                        next[j] = true;
                    }
                }
                lit => {
                    if j < n.len() && n[j] == lit {
                        next[j + 1] = true;
                    }
                }
            }
        }
        reach = next;
    }
    reach[n.len()]
}

/// A name for messages and tests.
pub fn display(name: &[u16]) -> String {
    char::decode_utf16(name.iter().copied()).map(|c| c.unwrap_or('\u{FFFD}')).collect()
}

/// The `FILE_ATTRIBUTE_*` bits a create may ask for that we keep.
pub const fn create_attributes(requested: u32) -> u32 {
    requested & FILE_ATTRIBUTE_VALID_SET_FLAGS & !FILE_ATTRIBUTE_NORMAL
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_ntobj::name::wide;

    fn m(e: &str, n: &str) -> bool {
        matches(&wide(e), &wide(n))
    }

    #[test]
    fn star_and_question_mark() {
        assert!(m("*", "anything.txt"));
        assert!(m("*", ""));
        assert!(m("*.txt", "a.TXT"));
        assert!(!m("*.txt", "a.txt.bak"));
        assert!(m("a?c", "ABC"));
        assert!(!m("a?c", "ac"));
        assert!(m("*a*b*", "xxaYYbzz"));
        assert!(!m("*a*b*", "xxbYYazz"));
        assert!(m("file.txt", "FILE.TXT"));
        assert!(!m("file.txt", "file.tx"));
        // `*.*` taken literally needs a period (Win32 rewrites it first).
        assert!(m("*.*", "a.b"));
        assert!(!m("*.*", "ab"));
        assert!(m("", "whatever"));
    }

    #[test]
    fn dos_wildcards() {
        // `<` stops at the last period.
        assert!(m("<.txt", "a.b.txt"));
        assert!(m("<", "abc"));
        assert!(!m("<", "a.b"));
        assert!(m("<\"<", "a.b"));
        // `>` matches one character, or nothing at a period or the end.
        assert!(m("file>>>.txt", "file.txt"));
        assert!(m("file>>>.txt", "fileab.txt"));
        assert!(!m("file>.txt", "fileab.txt"));
        assert!(m("ab>>", "ab"));
        // `"` is a period, or nothing at the end.
        assert!(m("abc\"", "abc"));
        assert!(m("abc\"txt", "abc.txt"));
        assert!(!m("abc\"txt", "abcxtxt"));
    }

    #[test]
    fn component_rules() {
        assert_eq!(component_to_vfs(&wide("Hello.txt")), Ok(b"Hello.txt".to_vec()));
        assert_eq!(component_to_vfs(&wide("é")), Ok("é".as_bytes().to_vec()));
        for bad in ["", ".", "..", "a:b", "a*", "a?", "a\"b", "a<", "a>", "a|b", "a/b", "\u{1}"] {
            assert_eq!(component_to_vfs(&wide(bad)), Err(STATUS_OBJECT_NAME_INVALID), "{bad:?}");
        }
        assert_eq!(component_to_vfs(&[0xD800]), Err(STATUS_OBJECT_NAME_INVALID));
        assert!(component_to_vfs(&alloc::vec![b'a' as u16; 255]).is_ok());
        assert!(component_to_vfs(&alloc::vec![b'a' as u16; 256]).is_err());
        // Bytes that are not UTF-8 come back with U+FFFD.
        assert_eq!(vfs_to_wide(b"a\xFFb"), alloc::vec![0x61, 0xFFFD, 0x62]);
    }

    #[test]
    fn paths_split() {
        let p = wide("\\Windows\\System32\\");
        let (parts, trailing) = split_path(&p).unwrap();
        assert_eq!(parts.len(), 2);
        assert!(trailing);
        assert_eq!(split_path(&wide("\\")).unwrap().0.len(), 0);
        assert_eq!(split_path(&wide("")).unwrap().0.len(), 0);
        assert_eq!(split_path(&wide("a\\b")).unwrap().0.len(), 2);
        assert_eq!(split_path(&wide("\\a\\\\b")), Err(STATUS_OBJECT_NAME_INVALID));
        assert!(eq_fold(&wide("ReadMe.TXT"), &wide("readme.txt")));
        assert!(!eq_fold(&wide("ReadMe.TXT"), &wide("readme.txt2")));
    }
}
