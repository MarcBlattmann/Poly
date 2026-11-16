//! File names in the file dialogs: where the name and the extension start
//! (`nFileOffset`, `nFileExtension`), default extensions, joining and
//! navigating, and filling the caller's `lpstrFile` buffer (Learn,
//! "OPENFILENAMEW structure").

use alloc::vec::Vec;

const BS: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;
const DOT: u16 = b'.' as u16;
const COLON: u16 = b':' as u16;

fn is_sep(c: u16) -> bool {
    c == BS || c == SLASH
}

/// Index of the file name in `path`: after the last separator (or the drive's colon).
pub fn name_start(path: &[u16]) -> usize {
    path.iter().rposition(|&c| is_sep(c) || c == COLON).map_or(0, |i| i + 1)
}

/// `nFileOffset` and `nFileExtension` of a path: the offsets of the name and of the extension
/// after its period (0: no extension; a trailing period points at the terminating NUL, which is
/// `path.len()`).
pub fn offsets(path: &[u16]) -> (u16, u16) {
    let start = name_start(path);
    let ext = path[start..].iter().rposition(|&c| c == DOT).map_or(0, |i| start + i + 1);
    (start as u16, ext as u16)
}

/// `lpstrDefExt` applied: when the name part of `path` has no period, `.` and the first three
/// characters of `def_ext` are appended (Learn: "only the first three characters are appended").
pub fn with_default_extension(path: &[u16], def_ext: &[u16]) -> Vec<u16> {
    let mut out = path.to_vec();
    let start = name_start(path);
    if def_ext.is_empty() || path[start..].contains(&DOT) {
        return out;
    }
    out.push(DOT);
    out.extend(def_ext.iter().take(3));
    out
}

/// `dir` and `name` as one path: a separator between them unless `dir` ends in one (or is empty).
pub fn join(dir: &[u16], name: &[u16]) -> Vec<u16> {
    let mut out = dir.to_vec();
    if !out.is_empty() && !is_sep(*out.last().unwrap()) {
        out.push(BS);
    }
    out.extend_from_slice(name);
    out
}

/// Whether `p` names a place without a directory to start from: `C:\...`, `\\server\...`, `\...`.
pub fn is_absolute(p: &[u16]) -> bool {
    matches!(p, [a, b, c, ..] if *b == COLON && is_sep(*c) && (*a as u8).is_ascii_alphabetic() && *a < 128)
        || matches!(p, [a, b, ..] if is_sep(*a) && is_sep(*b))
        || matches!(p, [a, ..] if is_sep(*a))
}

/// `input` as the user typed it, made absolute in `cwd` with `.` and `..` resolved
/// (forward slashes become backslashes). `C:` alone is the root of that drive.
pub fn resolve(cwd: &[u16], input: &[u16]) -> Vec<u16> {
    let input: Vec<u16> = input.iter().map(|&c| if c == SLASH { BS } else { c }).collect();
    let full: Vec<u16> = if matches!(input.as_slice(), [a, b] if *b == COLON && (*a as u8).is_ascii_alphabetic() && *a < 128)
    {
        let mut v = input.clone();
        v.push(BS);
        v
    } else if matches!(input.as_slice(), [a, b, c, ..] if *b == COLON && *c == BS && (*a as u8).is_ascii_alphabetic() && *a < 128)
        || input.starts_with(&[BS, BS])
    {
        input
    } else if input.first() == Some(&BS) {
        // The root of the current drive.
        let mut v: Vec<u16> = cwd.iter().take(2).copied().collect();
        v.extend_from_slice(&input);
        v
    } else {
        join(cwd, &input)
    };
    // Resolve dot segments after the root.
    let root_len = if full.starts_with(&[BS, BS]) {
        // \\server\share\
        let mut seen = 0;
        let mut at = full.len();
        for (i, &c) in full.iter().enumerate().skip(2) {
            if c == BS {
                seen += 1;
                if seen == 2 {
                    at = i + 1;
                    break;
                }
            }
        }
        at.min(full.len())
    } else {
        3.min(full.len())
    };
    let (root, rest) = full.split_at(root_len);
    let mut segs: Vec<&[u16]> = Vec::new();
    for s in rest.split(|&c| c == BS) {
        if s.is_empty() || s == [DOT] {
            continue;
        }
        if s == [DOT, DOT] {
            segs.pop();
            continue;
        }
        segs.push(s);
    }
    let mut out = root.to_vec();
    for (i, s) in segs.iter().enumerate() {
        if i > 0 {
            out.push(BS);
        }
        out.extend_from_slice(s);
    }
    out
}

/// The directory above `dir`: `C:\a\b` gives `C:\a`, `C:\a` gives `C:\`, a root gives nothing
/// (the list of drives, for the dialogs).
pub fn parent(dir: &[u16]) -> Option<Vec<u16>> {
    let trimmed: &[u16] = match dir.split_last() {
        Some((&l, r)) if is_sep(l) && r.len() > 2 => r,
        _ => dir,
    };
    let at = trimmed.iter().rposition(|&c| is_sep(c))?;
    if at < 3 {
        // `C:\x` -> `C:\`; `C:\` has no parent.
        return if trimmed.len() > at + 1 { Some(trimmed[..=at].to_vec()) } else { None };
    }
    Some(trimmed[..at].to_vec())
}

/// What filling the caller's buffer did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    /// Written; the offsets are for `nFileOffset` and `nFileExtension`.
    Done { offset: u16, ext: u16 },
    /// The buffer is too small: this many characters are needed (`FNERR_BUFFERTOOSMALL`;
    /// the first character of the buffer then holds the number).
    TooSmall(u32),
}

fn put(buf: &mut [u16], at: &mut usize, s: &[u16]) {
    buf[*at..*at + s.len()].copy_from_slice(s);
    *at += s.len();
    buf[*at] = 0;
    *at += 1;
}

/// One selected file: the full path and its terminating NUL.
pub fn fill_single(buf: &mut [u16], full: &[u16]) -> Fill {
    let need = full.len() + 1;
    if need > buf.len() {
        return Fill::TooSmall(need as u32);
    }
    let mut at = 0;
    put(buf, &mut at, full);
    let (offset, ext) = offsets(full);
    Fill::Done { offset, ext }
}

/// Several files, Explorer style: `dir NUL name NUL name NUL NUL`; `nFileOffset` is where the
/// first name starts, `nFileExtension` 0.
pub fn fill_multi_explorer(buf: &mut [u16], dir: &[u16], names: &[Vec<u16>]) -> Fill {
    let need = dir.len() + 1 + names.iter().map(|n| n.len() + 1).sum::<usize>() + 1;
    if need > buf.len() {
        return Fill::TooSmall(need as u32);
    }
    let mut at = 0;
    put(buf, &mut at, dir);
    for n in names {
        put(buf, &mut at, n);
    }
    buf[at] = 0;
    Fill::Done { offset: (dir.len() + 1) as u16, ext: 0 }
}

/// Several files, old style: `dir name name` separated by spaces; a name with a space is quoted.
pub fn fill_multi_old(buf: &mut [u16], dir: &[u16], names: &[Vec<u16>]) -> Fill {
    let mut joined: Vec<u16> = dir.to_vec();
    for n in names {
        joined.push(b' ' as u16);
        if n.contains(&(b' ' as u16)) {
            joined.push(b'"' as u16);
            joined.extend_from_slice(n);
            joined.push(b'"' as u16);
        } else {
            joined.extend_from_slice(n);
        }
    }
    let need = joined.len() + 1;
    if need > buf.len() {
        return Fill::TooSmall(need as u32);
    }
    let mut at = 0;
    put(buf, &mut at, &joined);
    Fill::Done { offset: (dir.len() + 1) as u16, ext: 0 }
}

/// `GetFileTitle`: the name part of `path` (a drive and directories are cut away).
pub fn file_title(path: &[u16]) -> &[u16] {
    &path[name_start(path)..]
}

/// Whether a typed name has characters a file name cannot have (`FNERR_INVALIDFILENAME`).
pub fn invalid_name(name: &[u16]) -> bool {
    name.iter().any(|&c| c < 0x20 || matches!(c as u8 as char, '<' | '>' | '"' | '|' | '*' | '?') && c < 128)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn s(v: &[u16]) -> String {
        String::from_utf16(v).unwrap()
    }

    #[test]
    fn offsets_of_a_path() {
        assert_eq!(offsets(&w("c:\\dir1\\dir2\\file.ext")), (13, 18));
        assert_eq!(offsets(&w("c:\\dir\\file")), (7, 0));
        assert_eq!(offsets(&w("c:\\dir\\file.")), (7, 12));
        assert_eq!(offsets(&w("file.tar.gz")), (0, 9));
        assert_eq!(offsets(&w("")), (0, 0));
    }

    #[test]
    fn default_extension() {
        assert_eq!(s(&with_default_extension(&w("c:\\a\\note"), &w("txt"))), "c:\\a\\note.txt");
        assert_eq!(s(&with_default_extension(&w("c:\\a\\note"), &w("text"))), "c:\\a\\note.tex");
        assert_eq!(s(&with_default_extension(&w("c:\\a.b\\note"), &w("txt"))), "c:\\a.b\\note.txt");
        assert_eq!(s(&with_default_extension(&w("note.md"), &w("txt"))), "note.md");
        assert_eq!(s(&with_default_extension(&w("note"), &w(""))), "note");
    }

    #[test]
    fn joining_and_resolving() {
        assert_eq!(s(&join(&w("C:\\a"), &w("b"))), "C:\\a\\b");
        assert_eq!(s(&join(&w("C:\\"), &w("b"))), "C:\\b");
        assert_eq!(s(&resolve(&w("C:\\a\\b"), &w("..\\c"))), "C:\\a\\c");
        assert_eq!(s(&resolve(&w("C:\\a\\b"), &w("../../.."))), "C:\\");
        assert_eq!(s(&resolve(&w("C:\\a"), &w("D:"))), "D:\\");
        assert_eq!(s(&resolve(&w("C:\\a"), &w("D:\\x\\.\\y\\"))), "D:\\x\\y");
        assert_eq!(s(&resolve(&w("C:\\a"), &w("\\z"))), "C:\\z");
        assert_eq!(s(&resolve(&w("C:\\a"), &w("f.txt"))), "C:\\a\\f.txt");
        assert_eq!(s(&resolve(&w("C:\\a"), &w("\\\\srv\\share\\d\\..\\e"))), "\\\\srv\\share\\e");
        assert!(is_absolute(&w("C:\\x")));
        assert!(is_absolute(&w("\\\\s\\x")));
        assert!(!is_absolute(&w("x\\y")));
    }

    #[test]
    fn parents() {
        assert_eq!(parent(&w("C:\\a\\b")).map(|p| s(&p)), Some("C:\\a".into()));
        assert_eq!(parent(&w("C:\\a")).map(|p| s(&p)), Some("C:\\".into()));
        assert_eq!(parent(&w("C:\\")), None);
        assert_eq!(parent(&w("C:\\a\\")).map(|p| s(&p)), Some("C:\\".into()));
    }

    #[test]
    fn single_and_multiple_buffers() {
        let mut buf = [9u16; 32];
        assert_eq!(fill_single(&mut buf, &w("C:\\a\\f.txt")), Fill::Done { offset: 5, ext: 7 });
        assert_eq!(s(&buf[..10]), "C:\\a\\f.txt");
        assert_eq!(buf[10], 0);
        let mut small = [0u16; 4];
        assert_eq!(fill_single(&mut small, &w("C:\\a\\f.txt")), Fill::TooSmall(11));

        let names = [w("a.txt"), w("b c.txt")];
        let mut buf = [7u16; 40];
        assert_eq!(fill_multi_explorer(&mut buf, &w("C:\\d"), &names), Fill::Done { offset: 5, ext: 0 });
        let expect: Vec<u16> = w("C:\\d\0a.txt\0b c.txt\0\0");
        assert_eq!(&buf[..expect.len()], expect.as_slice());
        let mut tiny = [0u16; 10];
        assert_eq!(fill_multi_explorer(&mut tiny, &w("C:\\d"), &names), Fill::TooSmall(20));

        let mut buf = [7u16; 40];
        assert_eq!(fill_multi_old(&mut buf, &w("C:\\d"), &names), Fill::Done { offset: 5, ext: 0 });
        let expect: Vec<u16> = w("C:\\d a.txt \"b c.txt\"\0");
        assert_eq!(&buf[..expect.len()], expect.as_slice());
    }

    #[test]
    fn titles_and_bad_names() {
        assert_eq!(s(file_title(&w("C:\\a\\b.txt"))), "b.txt");
        assert_eq!(s(file_title(&w("C:b.txt"))), "b.txt");
        assert!(invalid_name(&w("a*b")));
        assert!(invalid_name(&w("a|b")));
        assert!(!invalid_name(&w("a b.txt")));
    }
}
