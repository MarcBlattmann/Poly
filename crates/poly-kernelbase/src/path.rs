//! DOS paths: what kind a path is, `GetFullPathNameW`'s normalisation, and
//! the NT path (`\??\...`) the file services take.
//!
//! Sources: Learn's "Naming Files, Paths, and Namespaces" (the kinds of
//! path, `\\?\` and `\\.\`, the reserved device names), "File path formats
//! on Windows systems" (the normalisation steps: legacy devices, applying
//! the current directory, separators, relative segments, trimming), and the
//! `GetFullPathNameW` and `GetTempPathW` pages. The `\??\` form is the
//! object-manager name of the DOS devices directory (ADR 0067).
//!
//! Choices to be confirmed by the spec agent (ADR 0076): an empty path is
//! `ERROR_INVALID_NAME`; `\??\` given to a Win32 function is passed through
//! like `\\?\`; a legacy device name is recognised only as the whole last
//! segment, optionally followed by an extension.

use crate::error::{ERROR_FILE_NOT_FOUND, ERROR_INVALID_NAME, ERROR_PATH_NOT_FOUND};
use alloc::vec::Vec;

const BS: u16 = b'\\' as u16;
const SL: u16 = b'/' as u16;
const DOT: u16 = b'.' as u16;
const COLON: u16 = b':' as u16;
const SPACE: u16 = b' ' as u16;
const QM: u16 = b'?' as u16;

fn sep(c: u16) -> bool {
    c == BS || c == SL
}

fn letter(c: u16) -> bool {
    (u16::from(b'A')..=u16::from(b'Z')).contains(&c) || (u16::from(b'a')..=u16::from(b'z')).contains(&c)
}

fn upper(c: u16) -> u16 {
    if (u16::from(b'a')..=u16::from(b'z')).contains(&c) { c - 32 } else { c }
}

/// The kinds of path Learn's "File path formats on Windows systems" names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `\\?\...`: verbatim, never normalised (also `\??\...`, the NT form).
    Verbatim,
    /// `\\.\...` (or `//./`, `//?/`): a device path, normalised after its
    /// root.
    Device,
    /// `\\server\share\...`.
    Unc,
    /// `C:\...`.
    DriveAbsolute,
    /// `C:file`: relative to drive C's current directory.
    DriveRelative,
    /// `\file`: relative to the current drive's root.
    Rooted,
    /// `file`: relative to the current directory.
    Relative,
}

/// The kind of `p`.
pub fn kind(p: &[u16]) -> Kind {
    if p.len() >= 4 && p[0] == BS && p[1] == BS && p[2] == QM && p[3] == BS {
        return Kind::Verbatim;
    }
    if p.len() >= 4 && p[0] == BS && p[1] == QM && p[2] == QM && p[3] == BS {
        return Kind::Verbatim;
    }
    if p.len() >= 4 && sep(p[0]) && sep(p[1]) && (p[2] == DOT || p[2] == QM) && sep(p[3]) {
        return Kind::Device;
    }
    if p.len() >= 2 && sep(p[0]) && sep(p[1]) {
        return Kind::Unc;
    }
    if p.len() >= 2 && letter(p[0]) && p[1] == COLON {
        return if p.len() >= 3 && sep(p[2]) { Kind::DriveAbsolute } else { Kind::DriveRelative };
    }
    if !p.is_empty() && sep(p[0]) {
        return Kind::Rooted;
    }
    Kind::Relative
}

/// The length of the root of an already absolute (normalised or not) path:
/// `C:\` (3), `\\server\share` (up to the separator after the share),
/// `\\.\name` (up to the separator after the name); 0 if it has none.
pub fn root_len(p: &[u16]) -> usize {
    match kind(p) {
        Kind::DriveAbsolute => 3,
        Kind::DriveRelative => 2,
        Kind::Verbatim => 4,
        Kind::Device => {
            // \\.\name
            p[4..].iter().position(|&c| sep(c)).map_or(p.len(), |i| 4 + i)
        }
        Kind::Unc => {
            // \\server\share
            let server_end = p[2..].iter().position(|&c| sep(c)).map_or(p.len(), |i| 2 + i);
            if server_end >= p.len() {
                return p.len();
            }
            p[server_end + 1..].iter().position(|&c| sep(c)).map_or(p.len(), |i| server_end + 1 + i)
        }
        Kind::Rooted | Kind::Relative => 0,
    }
}

/// The reserved device names (Learn: "Naming Files, Paths, and
/// Namespaces": `CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`).
fn device_name(segment: &[u16]) -> bool {
    // The name before any extension or stream, trailing spaces ignored.
    let end = segment.iter().position(|&c| c == DOT || c == COLON).unwrap_or(segment.len());
    let mut name = &segment[..end];
    while name.last() == Some(&SPACE) {
        name = &name[..name.len() - 1];
    }
    let n: Vec<u8> = name.iter().map(|&c| if c < 0x80 { upper(c) as u8 } else { 0 }).collect();
    match n.as_slice() {
        b"CON" | b"PRN" | b"AUX" | b"NUL" => true,
        [b'C', b'O', b'M', d] | [b'L', b'P', b'T', d] => (b'1'..=b'9').contains(d),
        _ => false,
    }
}

/// Where a relative path is resolved from: the current directory (an
/// absolute DOS path, `C:\dir` or `C:\`), and the per-drive current
/// directories (the `=C:` environment variables, Learn: "Naming Files" /
/// `SetCurrentDirectory`).
pub struct Base<'a> {
    pub current: &'a [u16],
    pub drive_dir: &'a dyn Fn(u16) -> Option<Vec<u16>>,
}

impl core::fmt::Debug for Base<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Base").field("current", &self.current).finish_non_exhaustive()
    }
}

/// `GetFullPathNameW`: the absolute, normalised form of `path`, and where
/// its last component starts (`lpFilePart`; `None` when it ends with a
/// separator). Errors are Win32 codes.
pub fn full_path(path: &[u16], base: &Base<'_>) -> Result<(Vec<u16>, Option<usize>), u32> {
    if path.is_empty() {
        return Err(ERROR_INVALID_NAME);
    }
    let k = kind(path);
    if k == Kind::Verbatim {
        let fp = path.iter().rposition(|&c| c == BS).map(|i| i + 1).filter(|&i| i < path.len());
        return Ok((path.to_vec(), fp));
    }
    // 1. The root and the part to normalise below it.
    let (root, rest): (Vec<u16>, Vec<u16>) = match k {
        Kind::Device | Kind::Unc => {
            let r = root_len(path);
            let mut root: Vec<u16> = path[..r].iter().map(|&c| if c == SL { BS } else { c }).collect();
            if k == Kind::Device {
                root[2] = DOT;
            }
            (root, path[r..].to_vec())
        }
        Kind::DriveAbsolute => (alloc::vec![upper(path[0]), COLON, BS], path[3..].to_vec()),
        Kind::DriveRelative => {
            let drive = upper(path[0]);
            let cur = base.current;
            let dir = if cur.len() >= 2 && upper(cur[0]) == drive && cur[1] == COLON {
                cur.to_vec()
            } else {
                (base.drive_dir)(drive).unwrap_or_else(|| alloc::vec![drive, COLON, BS])
            };
            let r = root_len(&dir).max(2);
            let mut rest = dir[r..].to_vec();
            rest.push(BS);
            rest.extend_from_slice(&path[2..]);
            (alloc::vec![drive, COLON, BS], rest)
        }
        Kind::Rooted => {
            let cur = base.current;
            let r = root_len(cur);
            let mut root: Vec<u16> = cur[..r].to_vec();
            if kind(&root) == Kind::DriveAbsolute || kind(&root) == Kind::DriveRelative {
                root.truncate(2);
                root.push(BS);
            }
            (root, path.to_vec())
        }
        Kind::Relative => {
            let cur = base.current;
            let r = root_len(cur);
            let mut root = cur[..r].to_vec();
            if kind(&root) == Kind::DriveRelative {
                root.push(BS);
            }
            let mut rest = cur[r..].to_vec();
            rest.push(BS);
            rest.extend_from_slice(path);
            (root, rest)
        }
        Kind::Verbatim => unreachable!(),
    };
    // 2. Legacy devices: a drive or relative path whose last segment is a
    //    reserved name is that device.
    if matches!(k, Kind::DriveAbsolute | Kind::DriveRelative | Kind::Rooted | Kind::Relative) {
        let last_start = path.iter().rposition(|&c| sep(c) || c == COLON).map_or(0, |i| i + 1);
        let last = &path[last_start..];
        if !last.is_empty() && device_name(last) {
            let end = last.iter().position(|&c| c == DOT || c == COLON).unwrap_or(last.len());
            let mut name = &last[..end];
            while name.last() == Some(&SPACE) {
                name = &name[..name.len() - 1];
            }
            let mut out = alloc::vec![BS, BS, DOT, BS];
            out.extend_from_slice(name);
            return Ok((out, Some(4)));
        }
    }
    // 3. Separators, relative segments and trimming.
    let trailing_sep = path.last().is_some_and(|&c| sep(c));
    let mut parts: Vec<Vec<u16>> = Vec::new();
    for seg in rest.split(|&c| sep(c)) {
        if seg.is_empty() || seg == [DOT] {
            continue;
        }
        if seg == [DOT, DOT] {
            parts.pop();
            continue;
        }
        let mut s = seg.to_vec();
        // A segment ending in a single period loses it ("a." -> "a"; "a.."
        // keeps both, three or more periods are a name).
        if s.len() >= 2 && s[s.len() - 1] == DOT && s[s.len() - 2] != DOT {
            s.pop();
        }
        parts.push(s);
    }
    if !trailing_sep {
        // The last segment loses trailing periods and spaces.
        if let Some(last) = parts.last_mut() {
            while last.last().is_some_and(|&c| c == DOT || c == SPACE) {
                last.pop();
            }
            if last.is_empty() {
                parts.pop();
            }
        }
    }
    let mut out = root;
    let root_ends_with_sep = out.last().is_some_and(|&c| c == BS);
    let mut file_part = None;
    for (i, p) in parts.iter().enumerate() {
        if i > 0 || !root_ends_with_sep {
            out.push(BS);
        }
        if i == parts.len() - 1 {
            file_part = Some(out.len());
        }
        out.extend_from_slice(p);
    }
    if trailing_sep && !parts.is_empty() {
        out.push(BS);
        file_part = None;
    }
    if parts.is_empty() {
        file_part = None;
        if matches!(k, Kind::Unc | Kind::Device) && trailing_sep {
            out.push(BS);
        }
    }
    Ok((out, file_part))
}

/// The NT path of an absolute DOS path (`RtlDosPathNameToNtPathName_U`'s
/// prefixes): `C:\x` → `\??\C:\x`, `\\server\share\x` →
/// `\??\UNC\server\share\x`, `\\.\x` and `\\?\x` → `\??\x`; `\??\x` stays.
pub fn to_nt(full: &[u16]) -> Vec<u16> {
    let mut out: Vec<u16> = crate::wide("\\??\\");
    match kind(full) {
        Kind::Verbatim | Kind::Device => out.extend_from_slice(&full[4..]),
        Kind::Unc => {
            out.extend_from_slice(&crate::wide("UNC\\"));
            out.extend_from_slice(&full[2..]);
        }
        _ => out.extend_from_slice(full),
    }
    out
}

/// A Win32 path to the NT path a file service takes: normalised against
/// `base` (a verbatim path is taken as it is).
pub fn dos_to_nt(path: &[u16], base: &Base<'_>) -> Result<Vec<u16>, u32> {
    if path.is_empty() {
        return Err(ERROR_PATH_NOT_FOUND);
    }
    let (full, _) = full_path(path, base)?;
    Ok(to_nt(&full))
}

/// A current directory as Windows keeps it: absolute, without a trailing
/// separator unless it is a root (`C:\`).
pub fn as_current_dir(full: &[u16]) -> Vec<u16> {
    let mut d = full.to_vec();
    let r = root_len(&d);
    while d.len() > r.max(1) && d.last() == Some(&BS) {
        d.pop();
    }
    if kind(&d) == Kind::DriveRelative && d.len() == 2 {
        d.push(BS);
    }
    d
}

/// `FindFirstFileW`'s argument split into the directory to open (absolute,
/// normalised) and the pattern to match in it (`*.*` becomes `*`, which is
/// what Learn's "matches every file" means). A path that ends in a
/// separator names no file: `ERROR_FILE_NOT_FOUND` (to be confirmed).
pub fn split_find(pattern: &[u16], base: &Base<'_>) -> Result<(Vec<u16>, Vec<u16>), u32> {
    if pattern.is_empty() || pattern.last().is_some_and(|&c| sep(c)) {
        return Err(ERROR_FILE_NOT_FOUND);
    }
    let cut = pattern.iter().rposition(|&c| sep(c)).map(|i| i + 1);
    let (dir_part, mask) = match cut {
        Some(i) => (pattern[..i].to_vec(), pattern[i..].to_vec()),
        None if kind(pattern) == Kind::DriveRelative => (pattern[..2].to_vec(), pattern[2..].to_vec()),
        None => (alloc::vec![DOT], pattern.to_vec()),
    };
    if mask.is_empty() {
        return Err(ERROR_FILE_NOT_FOUND);
    }
    let dir = if kind(&dir_part) == Kind::Verbatim {
        dir_part
    } else {
        let mut d = dir_part;
        if !d.last().is_some_and(|&c| sep(c)) && kind(&d) != Kind::DriveRelative {
            d.push(BS);
        }
        full_path(&d, base)?.0
    };
    let mask = if mask == crate::wide("*.*") { crate::wide("*") } else { mask };
    Ok((dir, mask))
}

/// `GetTempPathW`: the first of `TMP`, `TEMP`, `USERPROFILE` that is set
/// and not empty, else the Windows directory; made absolute; ending with a
/// separator. Learn: "The GetTempPath function does not verify that the
/// path exists".
pub fn temp_path(vars: [Option<&[u16]>; 3], windows: &[u16], base: &Base<'_>) -> Result<Vec<u16>, u32> {
    let chosen = vars.into_iter().flatten().find(|v| !v.is_empty()).unwrap_or(windows);
    let (mut full, _) = full_path(chosen, base)?;
    if full.last() != Some(&BS) {
        full.push(BS);
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use crate::wide as w;
    use std::string::String;

    fn s(v: &[u16]) -> String {
        String::from_utf16_lossy(v)
    }

    fn no_drives(_: u16) -> Option<Vec<u16>> {
        None
    }

    fn full(p: &str, cur: &str) -> String {
        let cur = w(cur);
        let base = Base { current: &cur, drive_dir: &no_drives };
        s(&full_path(&w(p), &base).unwrap().0)
    }

    #[test]
    fn kinds() {
        assert_eq!(kind(&w("\\\\?\\C:\\x")), Kind::Verbatim);
        assert_eq!(kind(&w("\\??\\C:\\x")), Kind::Verbatim);
        assert_eq!(kind(&w("\\\\.\\COM1")), Kind::Device);
        assert_eq!(kind(&w("//./COM1")), Kind::Device);
        assert_eq!(kind(&w("\\\\server\\share")), Kind::Unc);
        assert_eq!(kind(&w("C:\\x")), Kind::DriveAbsolute);
        assert_eq!(kind(&w("c:/x")), Kind::DriveAbsolute);
        assert_eq!(kind(&w("C:x")), Kind::DriveRelative);
        assert_eq!(kind(&w("C:")), Kind::DriveRelative);
        assert_eq!(kind(&w("\\x")), Kind::Rooted);
        assert_eq!(kind(&w("x\\y")), Kind::Relative);
    }

    #[test]
    fn absolute_paths_are_normalised() {
        assert_eq!(full("C:\\a\\b\\..\\c", "C:\\"), "C:\\a\\c");
        assert_eq!(full("c:/a//b/./c/", "C:\\"), "C:\\a\\b\\c\\");
        assert_eq!(full("C:\\..\\..\\a", "C:\\"), "C:\\a");
        assert_eq!(full("C:\\a\\.", "C:\\"), "C:\\a");
        assert_eq!(full("C:\\a\\b\\..", "C:\\"), "C:\\a");
        assert_eq!(full("C:\\", "D:\\"), "C:\\");
        assert_eq!(full("C:\\..", "D:\\"), "C:\\");
    }

    #[test]
    fn trailing_periods_and_spaces_are_trimmed() {
        assert_eq!(full("C:\\a.\\b", "C:\\"), "C:\\a\\b");
        assert_eq!(full("C:\\a...\\b", "C:\\"), "C:\\a...\\b");
        assert_eq!(full("C:\\dir\\file. . ", "C:\\"), "C:\\dir\\file");
        // A trailing separator keeps a trailing space.
        assert_eq!(full("C:\\dir \\", "C:\\"), "C:\\dir \\");
    }

    #[test]
    fn relative_paths_use_the_current_directory() {
        assert_eq!(full("x.txt", "C:\\Users\\poly"), "C:\\Users\\poly\\x.txt");
        assert_eq!(full("..\\x", "C:\\Users\\poly"), "C:\\Users\\x");
        assert_eq!(full(".", "C:\\Users\\poly"), "C:\\Users\\poly");
        assert_eq!(full("x", "C:\\"), "C:\\x");
        assert_eq!(full("\\Windows", "C:\\Users\\poly"), "C:\\Windows");
        assert_eq!(full("\\x", "\\\\srv\\share\\dir"), "\\\\srv\\share\\x");
        assert_eq!(full("y", "\\\\srv\\share\\dir"), "\\\\srv\\share\\dir\\y");
    }

    #[test]
    fn drive_relative_paths() {
        assert_eq!(full("C:x", "C:\\Users"), "C:\\Users\\x");
        assert_eq!(full("D:x", "C:\\Users"), "D:\\x");
        assert_eq!(full("d:", "C:\\Users"), "D:\\");
        let cur = w("C:\\Users");
        let dd = |d: u16| if d == u16::from(b'D') { Some(w("D:\\data")) } else { None };
        let base = Base { current: &cur, drive_dir: &dd };
        assert_eq!(s(&full_path(&w("D:x"), &base).unwrap().0), "D:\\data\\x");
    }

    #[test]
    fn unc_and_device_paths() {
        assert_eq!(full("\\\\srv\\share\\a\\..\\..\\b", "C:\\"), "\\\\srv\\share\\b");
        assert_eq!(full("//srv/share/a", "C:\\"), "\\\\srv\\share\\a");
        assert_eq!(full("\\\\.\\C:\\a\\..\\b", "C:\\"), "\\\\.\\C:\\b");
        assert_eq!(full("\\\\?\\C:\\a\\..\\b", "C:\\"), "\\\\?\\C:\\a\\..\\b");
    }

    #[test]
    fn legacy_devices() {
        assert_eq!(full("nul", "C:\\x"), "\\\\.\\nul");
        assert_eq!(full("C:\\temp\\CON.txt", "C:\\x"), "\\\\.\\CON");
        assert_eq!(full("COM1", "C:\\x"), "\\\\.\\COM1");
        assert_eq!(full("COM0", "C:\\x"), "C:\\x\\COM0");
        assert_eq!(full("console", "C:\\x"), "C:\\x\\console");
    }

    #[test]
    fn the_file_part() {
        let cur = w("C:\\d");
        let base = Base { current: &cur, drive_dir: &no_drives };
        let (p, fp) = full_path(&w("a\\b.txt"), &base).unwrap();
        assert_eq!(s(&p[fp.unwrap()..]), "b.txt");
        assert_eq!(full_path(&w("a\\"), &base).unwrap().1, None);
        assert_eq!(full_path(&w("C:\\"), &base).unwrap().1, None);
        assert_eq!(full_path(&w(""), &base), Err(ERROR_INVALID_NAME));
    }

    #[test]
    fn nt_paths() {
        assert_eq!(s(&to_nt(&w("C:\\x"))), "\\??\\C:\\x");
        assert_eq!(s(&to_nt(&w("\\\\srv\\sh\\x"))), "\\??\\UNC\\srv\\sh\\x");
        assert_eq!(s(&to_nt(&w("\\\\.\\COM1"))), "\\??\\COM1");
        assert_eq!(s(&to_nt(&w("\\\\?\\C:\\a\\..\\b"))), "\\??\\C:\\a\\..\\b");
        assert_eq!(s(&to_nt(&w("\\??\\C:\\x"))), "\\??\\C:\\x");
        let cur = w("C:\\Users\\poly");
        let base = Base { current: &cur, drive_dir: &no_drives };
        assert_eq!(s(&dos_to_nt(&w("f.txt"), &base).unwrap()), "\\??\\C:\\Users\\poly\\f.txt");
        assert_eq!(dos_to_nt(&w(""), &base), Err(ERROR_PATH_NOT_FOUND));
    }

    #[test]
    fn current_directories() {
        assert_eq!(s(&as_current_dir(&w("C:\\a\\b\\"))), "C:\\a\\b");
        assert_eq!(s(&as_current_dir(&w("C:\\"))), "C:\\");
        assert_eq!(s(&as_current_dir(&w("\\\\srv\\sh\\"))), "\\\\srv\\sh");
    }

    #[test]
    fn find_patterns() {
        let cur = w("C:\\Users\\poly");
        let base = Base { current: &cur, drive_dir: &no_drives };
        let f = |p: &str| split_find(&w(p), &base).map(|(d, m)| (s(&d), s(&m)));
        assert_eq!(f("C:\\Windows\\*.*"), Ok(("C:\\Windows\\".into(), "*".into())));
        assert_eq!(f("*.txt"), Ok(("C:\\Users\\poly\\".into(), "*.txt".into())));
        assert_eq!(f("sub/x?.dat"), Ok(("C:\\Users\\poly\\sub\\".into(), "x?.dat".into())));
        assert_eq!(f("C:\\Windows"), Ok(("C:\\".into(), "Windows".into())));
        assert_eq!(f("C:\\Windows\\"), Err(ERROR_FILE_NOT_FOUND));
        assert_eq!(f(""), Err(ERROR_FILE_NOT_FOUND));
        // A mask keeps its trailing period (only the directory is
        // normalised).
        assert_eq!(f("C:\\d\\*."), Ok(("C:\\d\\".into(), "*.".into())));
    }

    #[test]
    fn temp_paths() {
        let cur = w("C:\\Users\\poly");
        let base = Base { current: &cur, drive_dir: &no_drives };
        let tmp = w("C:\\Users\\poly\\AppData\\Local\\Temp");
        let empty = w("");
        let prof = w("C:\\Users\\poly");
        let win = w("C:\\Windows");
        let t = |v: [Option<&[u16]>; 3]| s(&temp_path(v, &win, &base).unwrap());
        assert_eq!(t([Some(&tmp), None, None]), "C:\\Users\\poly\\AppData\\Local\\Temp\\");
        assert_eq!(t([Some(&empty), None, Some(&prof)]), "C:\\Users\\poly\\");
        assert_eq!(t([None, None, None]), "C:\\Windows\\");
        let rel = w("tmp");
        assert_eq!(t([None, Some(&rel), None]), "C:\\Users\\poly\\tmp\\");
    }
}
