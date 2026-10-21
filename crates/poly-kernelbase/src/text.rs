//! Strings handed back to callers, and the ANSI code page.
//!
//! Two documented conventions for a string copied to a caller's buffer of
//! `nBufferLength` characters:
//!
//! - **Size** (`GetCurrentDirectoryW`, `GetEnvironmentVariableW`,
//!   `GetTempPathW`, `GetFullPathNameW`, `ExpandEnvironmentStringsW`'s
//!   cousin rules): if the string and its terminating NUL fit, it is copied
//!   and the length *without* the NUL is returned; otherwise nothing is
//!   copied and the size the buffer needs, *with* the NUL, is returned.
//! - **Truncate** (`GetModuleFileNameW`): if it does not fit, the string is
//!   truncated to `nSize - 1` characters plus a NUL, `nSize` is returned and
//!   the last error is `ERROR_INSUFFICIENT_BUFFER`.
//!
//! The ANSI functions (`CreateFileA`, `GetCommandLineA`) use the process's
//! ANSI code page, Poly's Windows-1252 ([`poly_rtl::codepage`]).

use alloc::vec::Vec;
use poly_rtl::codepage;

/// What a "size"-convention function does with a string of `len`
/// characters and a buffer of `capacity` characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sized {
    /// Copy the string and a NUL; return `len`.
    Copy(u32),
    /// Copy nothing; return the size needed (`len + 1`).
    Needed(u32),
}

/// The "size" convention.
pub fn sized(len: usize, capacity: u32) -> Sized {
    let len = u32::try_from(len).unwrap_or(u32::MAX - 1);
    if (capacity as u64) > len as u64 { Sized::Copy(len) } else { Sized::Needed(len + 1) }
}

/// What a "truncate"-convention function copies and returns: `(characters
/// to copy before the NUL, return value, insufficient buffer)`. With a zero
/// capacity nothing is copied, not even the NUL.
pub fn truncated(len: usize, capacity: u32) -> (usize, u32, bool) {
    let cap = capacity as usize;
    if len < cap {
        (len, len as u32, false)
    } else if cap == 0 {
        (0, 0, true)
    } else {
        (cap - 1, capacity, true)
    }
}

/// ANSI (code page 1252) bytes to UTF-16, one unit per byte.
pub fn from_ansi(bytes: &[u8]) -> Vec<u16> {
    bytes.iter().map(|&b| codepage::to_unicode(b)).collect()
}

/// UTF-16 to ANSI (code page 1252); a character the code page lacks
/// becomes `?` (its default character).
pub fn to_ansi(units: &[u16]) -> Vec<u8> {
    units.iter().map(|&c| codepage::from_unicode(c)).collect()
}

/// The length of a NUL-terminated string at `p`, at most `limit` units.
///
/// # Safety
/// `p` points at a NUL-terminated UTF-16 string or at `limit` readable
/// units.
pub unsafe fn wcslen(p: *const u16, limit: usize) -> usize {
    let mut n = 0;
    // SAFETY: the caller's contract.
    while n < limit && unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    n
}

/// A NUL-terminated string at `p` as a slice (without the NUL).
///
/// # Safety
/// As [`wcslen`]; the slice must not outlive the string.
pub unsafe fn wide_at<'a>(p: *const u16) -> &'a [u16] {
    // SAFETY: the caller's contract.
    unsafe { core::slice::from_raw_parts(p, wcslen(p, 0x7FFF_FFFF)) }
}

/// The length of a NUL-terminated byte string.
///
/// # Safety
/// `p` points at a NUL-terminated string.
pub unsafe fn strlen(p: *const u8) -> usize {
    let mut n = 0;
    // SAFETY: the caller's contract.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    n
}

/// ASCII-only upper case of a UTF-16 unit (for names compared without case
/// where Learn says so; [`poly_ntobj::name::upcase`] covers the rest).
pub fn eq_ignore_case(a: &[u16], b: &[u16]) -> bool {
    poly_ntobj::name::eq_ignore_case(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_size_convention() {
        assert_eq!(sized(5, 6), Sized::Copy(5));
        assert_eq!(sized(5, 5), Sized::Needed(6));
        assert_eq!(sized(5, 0), Sized::Needed(6));
        assert_eq!(sized(0, 1), Sized::Copy(0));
        assert_eq!(sized(0, 0), Sized::Needed(1));
    }

    #[test]
    fn the_truncate_convention() {
        assert_eq!(truncated(5, 10), (5, 5, false));
        assert_eq!(truncated(5, 5), (4, 5, true));
        assert_eq!(truncated(5, 1), (0, 1, true));
        assert_eq!(truncated(5, 0), (0, 0, true));
    }

    #[test]
    fn ansi_round_trips_1252() {
        let w = from_ansi(b"caf\xE9 \x80");
        assert_eq!(w, [0x63, 0x61, 0x66, 0xE9, 0x20, 0x20AC]);
        assert_eq!(to_ansi(&w), b"caf\xE9 \x80");
        assert_eq!(to_ansi(&[0x4E2D]), b"?");
    }

    #[test]
    fn lengths() {
        let s = [0x41u16, 0x42, 0];
        // SAFETY: a NUL-terminated buffer.
        assert_eq!(unsafe { wcslen(s.as_ptr(), 10) }, 2);
        // SAFETY: as above.
        assert_eq!(unsafe { wide_at(s.as_ptr()) }, &[0x41, 0x42]);
        // SAFETY: a NUL-terminated string.
        assert_eq!(unsafe { strlen(c"abc".as_ptr().cast()) }, 3);
    }
}
