//! The path rules of `shlwapi.dll`'s `Path*W` functions and of the `PathCch*` family of
//! `kernelbase.dll` (the `api-ms-win-core-path` API sets), as host-tested logic.
//!
//! The rules are those of the spec sheets in `specs/windows/shlwapi.dll/Path*.yaml` and
//! `specs/windows/kernelbase.dll/PathCch*.yaml` (observed on a Windows 11 machine, tables of
//! 36 to 40 strings per group); `tests/winpath_tables.rs` runs every line of the visible tests'
//! recorded results through these functions. Learn's reference pages for the functions name
//! the prototypes and the result conventions.
//!
//! Strings are UTF-16 slices without the terminating NUL.

use alloc::vec::Vec;

const BS: u16 = b'\\' as u16;
const COLON: u16 = b':' as u16;
const DOT: u16 = b'.' as u16;
const QUESTION: u16 = b'?' as u16;
const SPACE: u16 = b' ' as u16;
const SLASH: u16 = b'/' as u16;

/// `MAX_PATH` (`minwindef.h`).
pub const MAX_PATH: usize = 260;

/// `HRESULT`s the PathCch functions return (`winerror.h`): `S_OK`, `E_INVALIDARG`,
/// `HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER)` and `HRESULT_FROM_WIN32(ERROR_FILENAME_EXCED_RANGE)`.
pub const S_OK: u32 = 0;
pub const E_INVALIDARG: u32 = 0x8007_0057;
pub const HR_INSUFFICIENT_BUFFER: u32 = 0x8007_007A;
pub const HR_FILENAME_EXCED_RANGE: u32 = 0x8007_00CE;

/// `PATHCCH_*` flags (`pathcch.h`).
pub const PATHCCH_ALLOW_LONG_PATHS: u32 = 0x1;
pub const PATHCCH_DO_NOT_NORMALIZE_SEGMENTS: u32 = 0x8;
pub const PATHCCH_ENSURE_IS_EXTENDED_LENGTH_PATH: u32 = 0x10;
pub const PATHCCH_ENSURE_TRAILING_SLASH: u32 = 0x20;

fn is_letter(c: u16) -> bool {
    matches!(c, 0x41..=0x5A | 0x61..=0x7A)
}

/// Whether `p` starts with an ASCII letter and a colon.
fn has_drive(p: &[u16]) -> bool {
    p.len() >= 2 && is_letter(p[0]) && p[1] == COLON
}

/// Whether `p` starts with `\\?\`.
fn has_long_prefix(p: &[u16]) -> bool {
    p.len() >= 4 && p[0] == BS && p[1] == BS && p[2] == QUESTION && p[3] == BS
}

/// Whether the units are `UNC` in any letter case.
fn is_unc_word(u: &[u16]) -> bool {
    u.len() == 3 && u.iter().zip(b"unc").all(|(&c, &l)| c < 0x80 && (c as u8).to_ascii_lowercase() == l)
}

/// For `\\?\UNC\...`: the index just after the `\\?\UNC\` prefix; `None` for any other path
/// (`\\?\UNC` without a backslash after it is not the long UNC form).
fn long_unc_end(p: &[u16]) -> Option<usize> {
    if has_long_prefix(p) && p.len() >= 8 && is_unc_word(&p[4..7]) && p[7] == BS { Some(8) } else { None }
}

/// The index of the first backslash at or after `from`, or the length.
fn next_bs(p: &[u16], from: usize) -> usize {
    p[from.min(p.len())..].iter().position(|&c| c == BS).map_or(p.len(), |i| i + from)
}

// ----- PathIs* ------------------------------------------------------------------------------

/// `PathIsRelativeW`: not relative when it starts with a backslash or with a drive letter and a colon.
pub fn is_relative(p: &[u16]) -> bool {
    !(p.first() == Some(&BS) || has_drive(p))
}

/// `PathGetDriveNumberW`: the letter's index 0 to 25, or -1.
pub fn drive_number(p: &[u16]) -> i32 {
    let p = if has_long_prefix(p) { &p[4..] } else { p };
    if has_drive(p) { i32::from(p[0] | 0x20) - i32::from(b'a') } else { -1 }
}

/// `PathIsFileSpecW`: no backslash and no colon.
pub fn is_file_spec(p: &[u16]) -> bool {
    !p.iter().any(|&c| c == BS || c == COLON)
}

/// `PathIsUNCW`: two leading backslashes (but not `\\?` followed by more), or `\\?\UNC\...`.
pub fn is_unc(p: &[u16]) -> bool {
    if long_unc_end(p).is_some() {
        return true;
    }
    p.len() >= 2 && p[0] == BS && p[1] == BS && !has_long_prefix(p) && !(p.len() == 3 && p[2] == QUESTION)
}

/// What follows `\\` (or `\\?\UNC\`) of a UNC path, if it is one.
fn unc_tail(p: &[u16]) -> Option<&[u16]> {
    if let Some(n) = long_unc_end(p) {
        return Some(&p[n..]);
    }
    if is_unc(p) { Some(&p[2..]) } else { None }
}

/// `PathIsUNCServerW`: `\\` and at most one name with no further backslash.
pub fn is_unc_server(p: &[u16]) -> bool {
    unc_tail(p).is_some_and(|t| !t.contains(&BS))
}

/// `PathIsUNCServerShareW`: `\\server\share` with exactly one backslash after the server and none
/// after the share (either name may be empty).
pub fn is_unc_server_share(p: &[u16]) -> bool {
    unc_tail(p).is_some_and(|t| t.iter().filter(|&&c| c == BS).count() == 1)
}

/// `PathIsNetworkPathW` (no drive of this system is a network drive).
pub fn is_network_path(p: &[u16]) -> bool {
    is_unc(p)
}

/// `PathIsRootW`.
pub fn is_root(p: &[u16]) -> bool {
    match p {
        [] => false,
        [BS] | [BS, BS] => true,
        _ => {
            if has_long_prefix(p) {
                let rest = &p[4..];
                if let Some(n) = long_unc_end(p) {
                    // `\\?\UNC\`, `\\?\UNC\s`, `\\?\UNC\s\sh`.
                    let t = &p[n..];
                    return match t.iter().filter(|&&c| c == BS).count() {
                        0 => true,
                        1 => t.last() != Some(&BS),
                        _ => false,
                    };
                }
                // `\\?\C:\`, `\\?\Volume{guid}\`.
                if has_drive(rest) {
                    return rest.len() == 3 && rest[2] == BS;
                }
                let lower: Vec<u16> = rest.iter().map(|&c| if c < 0x80 { c | 0x20 } else { c }).collect();
                if lower.starts_with(&"volume{".encode_utf16().collect::<Vec<_>>()) {
                    return rest.last() == Some(&BS) && rest[..rest.len() - 1].iter().all(|&c| c != BS);
                }
                return false;
            }
            if has_drive(p) {
                return p.len() == 3 && p[2] == BS;
            }
            if let Some(t) = unc_tail(p) {
                // A server alone, or a server and a share, without a trailing backslash.
                return match t.iter().filter(|&&c| c == BS).count() {
                    0 => true,
                    1 => t.last() != Some(&BS),
                    _ => false,
                };
            }
            false
        }
    }
}

// ----- file name, arguments, backslashes ----------------------------------------------------

/// `PathStripPathW`: the index where the file name starts (the same rule as `PathFindFileNameW`:
/// after the last `\`, `/` or `:` that is followed by a character that is not `\` or `/`).
pub fn strip_path_start(p: &[u16]) -> usize {
    crate::shlwapi::find_file_name(p)
}

/// `PathGetArgsW`: the index just after the first space, or the length.
pub fn args_start(p: &[u16]) -> usize {
    p.iter().position(|&c| c == SPACE).map_or(p.len(), |i| i + 1)
}

/// `PathAddBackslashW`: whether a backslash is appended (not to an empty string or one that
/// already ends with a backslash).
pub fn add_backslash(p: &mut Vec<u16>) -> bool {
    if p.is_empty() || p.last() == Some(&BS) {
        return false;
    }
    p.push(BS);
    true
}

/// `PathRemoveBackslashW`: one trailing backslash goes, unless the path is a root. Returns whether
/// it was removed.
pub fn remove_backslash(p: &mut Vec<u16>) -> bool {
    if p.last() != Some(&BS) || is_root(p) {
        return false;
    }
    p.pop();
    true
}

// ----- roots ---------------------------------------------------------------------------------

/// `PathSkipRootW`: the index after the root, or `None`.
pub fn skip_root(p: &[u16]) -> Option<usize> {
    if has_drive(p) {
        return if p.len() >= 3 && p[2] == BS { Some(3) } else { None };
    }
    if let Some(n) = long_unc_end(p) {
        return Some(unc_skip(p, n));
    }
    if has_long_prefix(p) {
        return None;
    }
    if is_unc(p) {
        return Some(unc_skip(p, 2));
    }
    if p.first() == Some(&BS) && p.get(1) != Some(&BS) { Some(1) } else { None }
}

/// The index after `server\share\` of a UNC path whose names start at `from`: an empty share is
/// not skipped past its backslash.
fn unc_skip(p: &[u16], from: usize) -> usize {
    let server_end = next_bs(p, from);
    if server_end >= p.len() {
        return p.len();
    }
    let share_start = server_end + 1;
    if p.get(share_start) == Some(&BS) {
        return share_start;
    }
    let share_end = next_bs(p, share_start);
    if share_end >= p.len() { p.len() } else { share_end + 1 }
}

/// `PathCchSkipRoot`: the index after the root, or `None` (`E_INVALIDARG`). Unlike
/// [`skip_root`], a drive without a backslash and the long forms have a root.
pub fn pathcch_skip_root(p: &[u16]) -> Option<usize> {
    if has_long_prefix(p) {
        let rest = &p[4..];
        if let Some(n) = long_unc_end(p) {
            return Some(unc_skip(p, n));
        }
        if has_drive(rest) {
            return Some(if rest.len() >= 3 && rest[2] == BS { 7 } else { p.len().min(6) });
        }
        let lower: Vec<u16> = rest.iter().map(|&c| if c < 0x80 { c | 0x20 } else { c }).collect();
        if lower.starts_with(&"volume{".encode_utf16().collect::<Vec<_>>()) {
            let end = next_bs(p, 4);
            return Some(if end < p.len() { end + 1 } else { p.len() });
        }
        return None;
    }
    if has_drive(p) {
        return Some(if p.len() >= 3 && p[2] == BS { 3 } else { 2 });
    }
    if is_unc(p) {
        return Some(unc_skip(p, 2));
    }
    if p.first() == Some(&BS) && p.get(1) != Some(&BS) { Some(1) } else { None }
}

/// `PathStripToRootW`: the root of the path (empty and `false` when it has none).
pub fn strip_to_root(p: &[u16]) -> (Vec<u16>, bool) {
    // Long forms.
    if has_long_prefix(p) {
        let rest = &p[4..];
        if let Some(n) = long_unc_end(p) {
            // `\\?\UNC\s\sh` at most; `\\?\UNC\s\` loses its backslash.
            let server_end = next_bs(p, n);
            if server_end >= p.len() {
                return (p.to_vec(), true);
            }
            let share_end = next_bs(p, server_end + 1);
            let end = if share_end == server_end + 1 { server_end } else { share_end };
            return (p[..end].to_vec(), true);
        }
        if has_drive(rest) {
            let end = if rest.len() >= 3 && rest[2] == BS { 7 } else { p.len().min(6) };
            return (p[..end].to_vec(), true);
        }
        let lower: Vec<u16> = rest.iter().map(|&c| if c < 0x80 { c | 0x20 } else { c }).collect();
        if lower.starts_with(&"volume{".encode_utf16().collect::<Vec<_>>()) {
            let end = next_bs(p, 4);
            return (p[..if end < p.len() { end + 1 } else { p.len() }].to_vec(), true);
        }
        return (Vec::new(), false);
    }
    if has_drive(p) {
        if p.len() >= 3 && p[2] == BS {
            return (p[..3].to_vec(), true);
        }
        return (p[..2].to_vec(), true);
    }
    if is_unc(p) {
        // `\\server\share`: the share name is cut at the next backslash; an empty share is dropped.
        let server_end = next_bs(p, 2);
        if server_end >= p.len() {
            return (p.to_vec(), true);
        }
        let share_end = next_bs(p, server_end + 1);
        let end = if share_end == server_end + 1 { server_end } else { share_end };
        return (p[..end].to_vec(), true);
    }
    if p.first() == Some(&BS) && p.get(1) != Some(&BS) {
        return (alloc::vec![BS], true);
    }
    (Vec::new(), false)
}

/// `PathRemoveFileSpecW`: the path with its last component removed, and whether it changed.
pub fn remove_file_spec(p: &[u16]) -> (Vec<u16>, bool) {
    // A root without anything behind it stays as it is.
    if is_remove_root(p) {
        return (p.to_vec(), false);
    }
    let Some(last) = p.iter().rposition(|&c| c == BS) else {
        // No backslash: all of it goes, except a leading drive and colon.
        let keep = if has_drive(p) { 2 } else { 0 };
        return (p[..keep].to_vec(), true);
    };
    // The text from the last backslash on goes, then the backslashes that end what is left.
    let mut end = last;
    while end > 0 && p[end - 1] == BS {
        end -= 1;
    }
    let mut out = p[..end].to_vec();
    out = restore_root(p, out);
    if is_unc(p) && !has_long_prefix(p) {
        // `\\a` is `\\`; `\\\a` is `\\\`.
        if p.len() > 3 && p[2] == BS && out.len() <= 2 {
            out = alloc::vec![BS, BS, BS];
        } else if out.len() < 2 {
            out = alloc::vec![BS, BS];
        }
    }
    let changed = out != p;
    (out, changed)
}

/// The forms `PathRemoveFileSpecW` calls roots: the empty string, `\`, `\\`, a drive (`C:`, `C:\`),
/// `\\?\C:` and `\\?\Volume{guid}` (with or without a backslash).
fn is_remove_root(p: &[u16]) -> bool {
    match p {
        [] | [BS] | [BS, BS] => true,
        _ if has_drive(p) => p.len() == 2 || (p.len() == 3 && p[2] == BS),
        _ if has_long_prefix(p) => {
            let rest = &p[4..];
            let body = if rest.last() == Some(&BS) { &rest[..rest.len() - 1] } else { rest };
            let lower: Vec<u16> = body.iter().map(|&c| if c < 0x80 { c | 0x20 } else { c }).collect();
            (has_drive(body) && body.len() == 2)
                || (lower.starts_with(&"volume{".encode_utf16().collect::<Vec<_>>()) && !body.contains(&BS))
        }
        _ => false,
    }
}

/// When a cut leaves a root without its backslash, the backslash comes back (`\a` becomes `\`,
/// `C:\a` becomes `C:\`, `\\a` becomes `\\`).
fn restore_root(orig: &[u16], cut: Vec<u16>) -> Vec<u16> {
    let c = &cut[..];
    let root = c.is_empty() && orig.first() == Some(&BS) // `\a`
        || (has_drive(c) && c.len() == 2 && orig.get(2) == Some(&BS))
        || (c.len() == 1 && c[0] == BS && orig.len() >= 2 && orig[1] == BS) // `\\a`
        || (has_long_prefix(orig) && is_remove_root(c) && !c.ends_with(&[BS]));
    if !root {
        return cut;
    }
    let mut out = cut;
    out.push(BS);
    out
}

// ----- canonical form ------------------------------------------------------------------------

/// How [`canonicalize`] differs between the callers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Canon {
    /// Trailing dots of the last segment are removed.
    pub trailing_dots: bool,
    /// A bare drive (`C:`) gets its backslash.
    pub complete_drive: bool,
}

/// The `PathCanonicalizeW` mode: trailing dots go and a bare drive completes.
pub const CANON_SHLWAPI: Canon = Canon { trailing_dots: true, complete_drive: true };

/// Removes the `\\?\` prefix of a drive or UNC path (`\\?\C:\a` becomes `C:\a`, `\\?\UNC\s\sh`
/// becomes `\\s\sh`); every other path is returned as it is.
fn strip_long_prefix(p: &[u16]) -> Vec<u16> {
    if has_long_prefix(p) {
        if let Some(n) = long_unc_end(p) {
            let mut v = alloc::vec![BS, BS];
            v.extend_from_slice(&p[n..]);
            return v;
        }
        if has_drive(&p[4..]) {
            return p[4..].to_vec();
        }
    }
    p.to_vec()
}

/// Whether `p` starts with `\\?\Volume{` in any letter case.
fn has_volume_prefix(p: &[u16]) -> bool {
    has_long_prefix(p)
        && p.len() >= 11
        && p[4..11].iter().zip(b"volume{").all(|(&c, &l)| c < 0x80 && (c as u8).to_ascii_lowercase() == l)
}

/// The canonical form of a path: dot segments resolved, the `\\?\` prefix of a drive or UNC path
/// dropped, trailing dots of the last segment removed, a bare drive completed (see [`Canon`]).
/// Doubled backslashes, forward slashes and letter case stay.
pub fn canonicalize(path: &[u16], mode: Canon) -> Vec<u16> {
    let p = strip_long_prefix(path);
    // The NT prefix `\??\` is copied unchanged.
    if p.starts_with(&[BS, QUESTION, QUESTION, BS]) {
        return p;
    }
    // A path is `name (\ name)*`; a token is a name with the backslash before it (the first has none).
    let names: Vec<&[u16]> = p.split(|&c| c == BS).collect();
    // How many leading names are the root: they are never removed (`C:`, ``, `\\`, `\\?\Volume{guid}`).
    let protected = if has_volume_prefix(&p) {
        4
    } else if p.len() >= 2 && p[0] == BS && p[1] == BS {
        2
    } else if p.first() == Some(&BS) || (has_drive(&p) && (p.len() == 2 || p[2] == BS)) {
        1
    } else {
        0
    };
    let mut out: Vec<(usize, &[u16])> = Vec::new();
    // `..\a` and `.\a` are `a`: leading dots of a relative path go with the backslash behind them.
    let mut bare_next = false;
    // The last name was a dot segment that was dropped (not used to remove a name).
    let mut dropped_last = false;
    for (i, name) in names.iter().copied().enumerate() {
        let dot = name == [DOT];
        let dotdot = name == [DOT, DOT];
        dropped_last = false;
        if i < protected || !(dot || dotdot) {
            out.push((i, name));
        } else if protected == 0 && i == 0 {
            bare_next = true;
            dropped_last = true;
        } else if dotdot && out.len() > protected {
            out.pop();
        } else {
            dropped_last = true;
        }
    }
    let mut result: Vec<u16> = Vec::with_capacity(p.len());
    for (n, &(i, name)) in out.iter().enumerate() {
        if i > 0 && !(n == 0 && bare_next) {
            result.push(BS);
        }
        result.extend_from_slice(name);
    }
    // A dot segment dropped right behind the root leaves the root's backslash (`C:\.` is `C:\`).
    if dropped_last && protected > 0 && out.len() == protected {
        result.push(BS);
    }
    finish(result, mode)
}

/// The last steps of [`canonicalize`]: empty results, trailing dots, a bare drive.
fn finish(mut result: Vec<u16>, mode: Canon) -> Vec<u16> {
    if mode.trailing_dots {
        let start = result.iter().rposition(|&c| c == BS).map_or(0, |i| i + 1);
        while result.len() > start && result.last() == Some(&DOT) {
            result.pop();
        }
    }
    if result.is_empty() {
        return alloc::vec![BS];
    }
    if mode.complete_drive && has_drive(&result) && result.len() == 2 {
        result.push(BS);
    }
    result
}

/// The `\\?\` form of an already canonical path (`PATHCCH_ENSURE_IS_EXTENDED_LENGTH_PATH`):
/// a drive path gets `\\?\`, a UNC path `\\?\UNC\`; every other path stays.
pub fn to_extended_length(p: &[u16]) -> Vec<u16> {
    if has_long_prefix(p) || p.starts_with(&[BS, QUESTION, QUESTION, BS]) {
        return p.to_vec();
    }
    let mut out: Vec<u16> = Vec::with_capacity(p.len() + 8);
    if has_drive(p) {
        out.extend_from_slice(&[BS, BS, QUESTION, BS]);
        out.extend_from_slice(p);
    } else if is_unc(p) {
        out.extend_from_slice(&[BS, BS, QUESTION, BS]);
        out.extend("UNC\\".encode_utf16());
        out.extend_from_slice(&p[2..]);
    } else {
        out.extend_from_slice(p);
    }
    out
}

/// `PathCanonicalizeW`: `Err(())` when the source is `MAX_PATH` units or longer
/// (`ERROR_FILENAME_EXCED_RANGE`).
pub fn path_canonicalize(src: &[u16]) -> Result<Vec<u16>, TooLong> {
    if src.len() >= MAX_PATH {
        return Err(TooLong);
    }
    let out = canonicalize(src, CANON_SHLWAPI);
    if out.len() >= MAX_PATH { Err(TooLong) } else { Ok(out) }
}

/// A path of `MAX_PATH` units or more (`ERROR_FILENAME_EXCED_RANGE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLong;

// ----- combining -------------------------------------------------------------------------------

/// Whether `file` is a path that replaces the directory when combined: it has a drive, or starts
/// with two backslashes (or the NT prefix), or is a `\\?\` path. A path with a single leading
/// backslash is rooted, not absolute.
fn replaces_dir(file: &[u16]) -> bool {
    has_drive(file) || (file.len() >= 2 && file[0] == BS && file[1] == BS)
}

/// The joined, not yet canonical text of `dir` and `file` for `PathCombineW`/`PathAppendW`, or
/// `None` when the combination fails (a rooted second part with a directory that has no root).
/// `keep_backslash` is `PathCombineW`'s way with a single leading backslash (the root of `dir` is
/// taken); `PathAppendW` drops that backslash instead.
fn join(dir: &[u16], file: &[u16], append: bool) -> Option<Vec<u16>> {
    if file.is_empty() {
        return Some(dir.to_vec());
    }
    if dir.is_empty() && append && file[0] == BS && file.get(1) != Some(&BS) {
        return Some(file[1..].to_vec());
    }
    if dir.is_empty() || replaces_dir(file) {
        return Some(file.to_vec());
    }
    if file[0] == BS && !append {
        // Rooted: from the root of `dir`.
        let (root, has_root) = strip_to_root(dir);
        if !has_root || root.is_empty() {
            return None;
        }
        let mut r = root;
        if r.last() == Some(&BS) {
            r.pop();
        }
        r.extend_from_slice(file);
        return Some(r);
    }
    let more = if append && file[0] == BS { &file[1..] } else { file };
    if more.is_empty() {
        return Some(dir.to_vec());
    }
    let mut r = dir.to_vec();
    if r.last() != Some(&BS) {
        r.push(BS);
    }
    r.extend_from_slice(more);
    Some(r)
}

/// `PathCombineW`: the canonical form of the combination, `Err(len)` when it fails (`len` is the
/// length the result would have, `usize::MAX` when the combination itself is impossible).
pub fn path_combine(dir: &[u16], file: &[u16]) -> Result<Vec<u16>, usize> {
    let Some(j) = join(dir, file, false) else { return Err(usize::MAX) };
    let out = canonicalize(&j, CANON_SHLWAPI);
    if out.len() >= MAX_PATH { Err(out.len()) } else { Ok(out) }
}

/// The canonical text of `PathCombineW`'s combination, however long (`None` when it cannot be made).
pub fn path_combine_text(dir: &[u16], file: &[u16]) -> Option<Vec<u16>> {
    join(dir, file, false).map(|j| canonicalize(&j, CANON_SHLWAPI))
}

/// `PathAppendW`: as [`path_combine`], except that one leading backslash of `more` is dropped.
pub fn path_append(path: &[u16], more: &[u16]) -> Result<Vec<u16>, usize> {
    let Some(j) = join(path, more, true) else { return Err(usize::MAX) };
    let out = canonicalize(&j, CANON_SHLWAPI);
    if out.len() >= MAX_PATH { Err(out.len()) } else { Ok(out) }
}

/// `PathCchCanonicalizeEx`'s text for `flags` (before the size checks): the canonical form (with
/// `PATHCCH_DO_NOT_NORMALIZE_SEGMENTS` and the extended-length flag the trailing dots stay, and the
/// extended-length form does not complete a bare drive), then the extended or trailing-slash form.
pub fn pathcch_canonical_text(src: &[u16], flags: u32) -> Vec<u16> {
    let extended = flags & PATHCCH_ENSURE_IS_EXTENDED_LENGTH_PATH != 0;
    let mode = Canon {
        trailing_dots: flags
            & (PATHCCH_DO_NOT_NORMALIZE_SEGMENTS
                | PATHCCH_ENSURE_IS_EXTENDED_LENGTH_PATH
                | PATHCCH_ENSURE_TRAILING_SLASH)
            == 0,
        complete_drive: !extended,
    };
    let mut out = canonicalize(src, mode);
    if extended {
        out = to_extended_length(&out);
    }
    if flags & PATHCCH_ENSURE_TRAILING_SLASH != 0 && out.last() != Some(&BS) {
        out.push(BS);
    }
    out
}

/// What `PathCchCanonicalizeEx` and `PathCchCombineEx` return for a text and a size: the text on
/// success, else the `HRESULT`. `cch` is the destination size in units with the NUL.
pub fn pathcch_fit(text: Vec<u16>, cch: usize, flags: u32) -> Result<Vec<u16>, u32> {
    if cch == 0 {
        return Err(E_INVALIDARG);
    }
    let mut text = text;
    if flags & PATHCCH_ALLOW_LONG_PATHS != 0 {
        // A path of MAX_PATH units or more carries the `\\?\` prefix.
        if text.len() >= MAX_PATH {
            text = to_extended_length(&text);
        }
    } else if text.len() >= MAX_PATH {
        return Err(HR_FILENAME_EXCED_RANGE);
    }
    if text.len() + 1 > cch {
        return Err(HR_INSUFFICIENT_BUFFER);
    }
    Ok(text)
}

/// `PathCchCombineEx`'s text: [`path_combine`]'s join with the flags' canonical form and without
/// `MAX_PATH` limit (that is [`pathcch_fit`]'s).
pub fn pathcch_combine_text(dir: &[u16], more: &[u16], flags: u32) -> Option<Vec<u16>> {
    let j = join(dir, more, false)?;
    Some(pathcch_canonical_text(&j, flags))
}

/// A UTF-16 string from text (tests).
pub fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[allow(dead_code)]
const _: (u16, u16) = (SLASH, SPACE);
