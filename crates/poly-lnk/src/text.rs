//! The text rules of a shell link: `SetPath`'s canonical form, environment variables, the
//! letter case of an existing target, relative paths, ANSI text and the buffer rules of the
//! getters (the sheet `IShellLinkW.yaml`: `path_*`, `other_buffers`, `null_strings`).

use crate::{FileInfo, System, Text, hr};
use alloc::vec::Vec;

const BACKSLASH: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;
const QUOTE: u16 = b'"' as u16;
const COLON: u16 = b':' as u16;
const PERCENT: u16 = b'%' as u16;
const DOT: u16 = b'.' as u16;
const SPACE: u16 = b' ' as u16;

/// `text` as UTF-16.
pub fn wide(text: &str) -> Text {
    text.encode_utf16().collect()
}

fn lower(c: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&c) { c + 32 } else { c }
}

fn upper(c: u16) -> u16 {
    if (u16::from(b'a')..=u16::from(b'z')).contains(&c) { c - 32 } else { c }
}

/// Equal without regard to the case of ASCII letters (as file names compare).
pub fn eq_nocase(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| lower(*x) == lower(*y))
}

/// The drive letter of a drive path (`X:\...` or `X:`), upper case.
pub fn drive_letter(path: &[u16]) -> Option<u16> {
    let letter = path.first().copied().map(upper)?;
    ((u16::from(b'A')..=u16::from(b'Z')).contains(&letter) && path.get(1) == Some(&COLON)).then_some(letter)
}

/// Whether `path` is a full drive path, `X:\...`.
pub fn is_drive_path(path: &[u16]) -> bool {
    drive_letter(path).is_some() && path.get(2) == Some(&BACKSLASH)
}

/// Whether `path` is a UNC name, `\\server\...`.
pub fn is_unc(path: &[u16]) -> bool {
    path.len() > 2 && path[0] == BACKSLASH && path[1] == BACKSLASH
}

/// The last component of a path (the text after its last backslash).
pub fn file_name(path: &[u16]) -> &[u16] {
    match path.iter().rposition(|&c| c == BACKSLASH) {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

/// `SetPath`'s text made canonical: surrounding double quotes removed (`None`, `E_INVALIDARG`,
/// when more text follows the closing quote), `/` turned into `\`, `.` and `..` components
/// collapsed. The flag says whether anything had to change (then the find data stays empty: the
/// sheet's `path_canonical`).
pub fn canonical_target(given: &[u16]) -> Option<(Text, bool)> {
    let mut changed = false;
    let inner: &[u16] = if given.first() == Some(&QUOTE) {
        changed = true;
        match given[1..].iter().position(|&c| c == QUOTE) {
            Some(end) => {
                if given[end + 2..].iter().any(|&c| c != SPACE) {
                    return None;
                }
                &given[1..end + 1]
            }
            None => &given[1..],
        }
    } else {
        given
    };
    let mut text: Text = inner.iter().map(|&c| if c == SLASH { BACKSLASH } else { c }).collect();
    changed |= inner.contains(&SLASH);
    // Collapse "." and "..": the root (`X:` or `\\server\share`) is never removed.
    let root = if is_drive_path(&text) {
        2
    } else if is_unc(&text) {
        let mut n = 0;
        let mut end = text.len();
        for (i, &c) in text.iter().enumerate().skip(2) {
            if c == BACKSLASH {
                n += 1;
                if n == 2 {
                    end = i;
                    break;
                }
            }
        }
        end
    } else {
        return Some((text, changed));
    };
    let parts: Vec<&[u16]> = text[root..].split(|&c| c == BACKSLASH).skip(1).collect();
    let dots = |p: &[u16], n: usize| p.len() == n && p.iter().all(|&c| c == DOT);
    if !parts.iter().any(|p| dots(p, 1) || dots(p, 2)) {
        return Some((text, changed));
    }
    let mut kept: Vec<&[u16]> = Vec::new();
    for p in parts {
        if dots(p, 1) {
            continue;
        }
        if dots(p, 2) {
            kept.pop();
            continue;
        }
        kept.push(p);
    }
    let mut out = text[..root].to_vec();
    if kept.is_empty() {
        out.push(BACKSLASH);
    }
    for p in kept {
        out.push(BACKSLASH);
        out.extend_from_slice(p);
    }
    text = out;
    Some((text, true))
}

/// `text` with each `%NAME%` whose variable is set replaced by its value; an unknown variable
/// stays as written (as `ExpandEnvironmentStringsW` does).
pub fn expand_env(text: &[u16], lookup: impl Fn(&[u16]) -> Option<Text>) -> Text {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i] != PERCENT {
            out.push(text[i]);
            i += 1;
            continue;
        }
        let Some(len) = text[i + 1..].iter().position(|&c| c == PERCENT) else {
            out.extend_from_slice(&text[i..]);
            break;
        };
        let name = &text[i + 1..i + 1 + len];
        match (!name.is_empty()).then(|| lookup(name)).flatten() {
            Some(value) => {
                out.extend_from_slice(&value);
                i += len + 2;
            }
            None => {
                // The closing `%` may open the next name.
                out.extend_from_slice(&text[i..i + 1 + len]);
                i += len + 1;
            }
        }
    }
    out
}

/// An existing target spelled as the file system spells it (an upper-case drive letter, each
/// component's own case), with its data; `None` when it does not exist.
pub fn spell(path: &[u16], sys: &dyn System) -> Option<(Text, FileInfo)> {
    if !is_drive_path(path) {
        return sys.entry(path).map(|(_, info)| (path.to_vec(), info));
    }
    let mut out: Text = Vec::with_capacity(path.len());
    out.push(drive_letter(path)?);
    out.extend_from_slice(&[COLON, BACKSLASH]);
    let rest = &path[3..];
    if rest.is_empty() {
        return sys.entry(&out).map(|(_, info)| (out, info));
    }
    let mut info = FileInfo::default();
    for (i, part) in rest.split(|&c| c == BACKSLASH).enumerate() {
        if i > 0 {
            out.push(BACKSLASH);
        }
        let start = out.len();
        out.extend_from_slice(part);
        if part.is_empty() {
            continue;
        }
        let (name, data) = sys.entry(&out)?;
        if eq_nocase(&name, part) {
            out.truncate(start);
            out.extend_from_slice(&name);
        }
        info = data;
    }
    Some((out, info))
}

/// The components of a drive path after `X:\` (empty ones dropped).
fn components(path: &[u16]) -> Vec<&[u16]> {
    path[3..].split(|&c| c == BACKSLASH).filter(|p| !p.is_empty()).collect()
}

/// The `RELATIVE_PATH` of a target for a link file `link` (both full drive paths on one drive):
/// `.\rest` when the target is below the link's directory, else one `..\` per directory to go up
/// (the sheet's `strings` rule). `None` for paths on different drives or not drive paths.
pub fn relative_path(target: &[u16], link: &[u16]) -> Option<Text> {
    if !is_drive_path(target) || !is_drive_path(link) || drive_letter(target) != drive_letter(link) {
        return None;
    }
    let to = components(target);
    let mut from = components(link);
    from.pop();
    let common = to.iter().zip(&from).take_while(|(a, b)| eq_nocase(a, b)).count();
    let mut out = Vec::new();
    if common == from.len() {
        out.extend_from_slice(&[DOT, BACKSLASH]);
    }
    for _ in common..from.len() {
        out.extend_from_slice(&[DOT, DOT, BACKSLASH]);
    }
    for (i, part) in to[common..].iter().enumerate() {
        if i > 0 {
            out.push(BACKSLASH);
        }
        out.extend_from_slice(part);
    }
    Some(out)
}

/// The ANSI form of a text (Poly's ANSI code page keeps the first 256 code points; anything else
/// is `?`).
pub fn to_ansi(text: &[u16]) -> Vec<u8> {
    text.iter().map(|&c| if c < 0x100 { c as u8 } else { b'?' }).collect()
}

/// ANSI text as UTF-16 (the first 256 code points).
pub fn from_ansi(bytes: &[u8]) -> Text {
    bytes.iter().map(|&b| u16::from(b)).collect()
}

/// The units a string getter with a buffer of `cch` units copies before its NUL: the text cut to
/// `cch - 1` (`GetDescription`, `GetArguments`, `GetWorkingDirectory`, `GetIconLocation`: always
/// `S_OK`); `None` for `cch` 0 or less: nothing is written.
pub fn truncated(len: usize, cch: i32) -> Option<usize> {
    let cch = usize::try_from(cch).ok().filter(|&c| c > 0)?;
    Some(len.min(cch - 1))
}

/// `GetPath`'s strict buffer rule (`path_buffer`): with no path `S_FALSE` and an empty string;
/// `cch` 0 `E_INVALIDARG` with nothing written, a negative `cch` `E_INVALIDARG` with a NUL in the
/// first unit; a buffer too small
/// `HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER)` with the text cut to `cch - 1`; else `S_OK`.
/// `len` is the path's length (`None`: no path); the second value is how many units to copy
/// before the NUL (`None`: write nothing).
pub fn get_path(len: Option<usize>, cch: i32) -> (i32, Option<usize>) {
    let Some(len) = len else {
        return (hr::S_FALSE, (cch >= 0).then_some(0));
    };
    match usize::try_from(cch) {
        // A negative count still gets a NUL in the first unit (sl_path: "first unit untouched false").
        Err(_) => (hr::E_INVALIDARG, Some(0)),
        Ok(0) => (hr::E_INVALIDARG, None),
        Ok(c) if c <= len => (hr::E_INSUFFICIENT_BUFFER, Some(c - 1)),
        Ok(_) => (hr::S_OK, Some(len)),
    }
}
