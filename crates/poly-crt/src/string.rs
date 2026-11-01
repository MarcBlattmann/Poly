//! The string functions (C11 7.24, 7.29.4) on NUL-terminated strings of
//! either width, and the bounds-checked `_s` forms with Learn's documented
//! results (`strcpy_s`, `strcat_s`, `strncpy_s`, `strncat_s`): an invalid
//! argument is the invalid parameter handler and `EINVAL`, a destination
//! too small is the handler and `ERANGE`, and in both cases the destination
//! is emptied when it can be. `_TRUNCATE` copies what fits and returns
//! `STRUNCATE`.
//!
//! Case-insensitive comparisons (`_stricmp`, `_wcsicmp`, `_memicmp`) fold
//! with the "C" locale's `tolower` / `towlower` ([`crate::ctype`]) and
//! compare the folded units as unsigned values.

use crate::ctype;
use crate::errno::{EINVAL, ERANGE, STRUNCATE};
use crate::unit::{self, Unit};

/// `strcmp`, `wcscmp` (and `strcoll` in the "C" locale): the sign of the
/// first differing unit, compared unsigned.
pub fn compare<C: Unit>(a: &[C], b: &[C]) -> i32 {
    for (x, y) in a.iter().zip(b) {
        if x != y {
            return if x.to_u32() < y.to_u32() { -1 } else { 1 };
        }
    }
    (a.len() as i64 - b.len() as i64).signum() as i32
}

/// The "C" locale's lower case of a unit.
pub fn fold<C: Unit>(c: C) -> u32 {
    if C::WIDE { u32::from(ctype::to_lower_wide(c.to_u32() as u16)) } else { ctype::to_lower(c.to_u32() as i32) as u32 }
}

/// `_stricmp`, `_wcsicmp`: [`compare`] after [`fold`].
pub fn icompare<C: Unit>(a: &[C], b: &[C]) -> i32 {
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (fold(*x), fold(*y));
        if x != y {
            return if x < y { -1 } else { 1 };
        }
    }
    (a.len() as i64 - b.len() as i64).signum() as i32
}

/// At most `n` units of a NUL-terminated string (`strncmp`'s view).
///
/// # Safety
/// `s` is readable up to its NUL or `n` units.
pub unsafe fn prefix<'a, C: Unit>(s: *const C, n: usize) -> &'a [C] {
    // SAFETY: the caller's contract.
    unsafe { unit::slice_max(s, n) }
}

/// `strstr` / `wcsstr`: the index of the first occurrence (an empty needle
/// is found at 0).
pub fn find<C: Unit>(hay: &[C], needle: &[C]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// `strspn` (`accept` true) and `strcspn` (false).
pub fn span<C: Unit>(s: &[C], set: &[C], accept: bool) -> usize {
    s.iter().position(|c| set.contains(c) != accept).unwrap_or(s.len())
}

/// `strtok` / `strtok_s` / `wcstok_s`: one step. `s` is the string to scan
/// (the new string, or the saved context); returns the token's start and
/// end (where a NUL is to be written, if the token is followed by a
/// delimiter) and where the next scan starts.
pub fn token<C: Unit>(s: &[C], delims: &[C]) -> Option<(usize, usize, usize)> {
    let start = span(s, delims, true);
    if start == s.len() {
        return None;
    }
    let len = span(&s[start..], delims, false);
    let end = start + len;
    let next = if end < s.len() { end + 1 } else { end };
    Some((start, end, next))
}

/// What a bounds-checked copy decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checked {
    /// Copy these units from the source and add a NUL; return 0.
    Copy(usize),
    /// `_TRUNCATE`: copy these units and a NUL; return `STRUNCATE`.
    Truncate(usize),
    /// The invalid parameter handler, then return this `errno` (`EINVAL` or
    /// `ERANGE`); `clear` says whether `dest[0] = 0` is written first.
    Fail { errno: i32, clear: bool },
    /// Nothing to do; return 0.
    Nothing,
}

/// `strcpy_s(dest, size, src)` with `src`'s length (`None`: null).
pub fn copy_s(dest_null: bool, size: usize, src_len: Option<usize>) -> Checked {
    if dest_null || size == 0 {
        return Checked::Fail { errno: EINVAL, clear: false };
    }
    match src_len {
        None => Checked::Fail { errno: EINVAL, clear: true },
        Some(n) if n >= size => Checked::Fail { errno: ERANGE, clear: true },
        Some(n) => Checked::Copy(n),
    }
}

/// `strncpy_s(dest, size, src, count)` (`count` may be `_TRUNCATE`).
pub fn ncopy_s(dest_null: bool, size: usize, src_len: Option<usize>, count: usize) -> Checked {
    if dest_null && size == 0 && count == 0 {
        return Checked::Nothing;
    }
    if dest_null || size == 0 {
        return Checked::Fail { errno: EINVAL, clear: false };
    }
    let Some(len) = src_len else {
        return if count == 0 { Checked::Copy(0) } else { Checked::Fail { errno: EINVAL, clear: true } };
    };
    if count == crate::printf::TRUNCATE {
        return if len < size { Checked::Copy(len) } else { Checked::Truncate(size - 1) };
    }
    let n = len.min(count);
    if n >= size { Checked::Fail { errno: ERANGE, clear: true } } else { Checked::Copy(n) }
}

/// `strcat_s(dest, size, src)`: `dest_len` is the destination's length
/// within `size` (`None`: no NUL in the first `size` units). The result's
/// counts are the units to append.
pub fn cat_s(dest_null: bool, size: usize, dest_len: Option<usize>, src_len: Option<usize>) -> Checked {
    if dest_null || size == 0 {
        return Checked::Fail { errno: EINVAL, clear: false };
    }
    let (Some(d), Some(s)) = (dest_len, src_len) else {
        return Checked::Fail { errno: EINVAL, clear: true };
    };
    if d + s >= size { Checked::Fail { errno: ERANGE, clear: true } } else { Checked::Copy(s) }
}

/// `strncat_s(dest, size, src, count)`.
pub fn ncat_s(dest_null: bool, size: usize, dest_len: Option<usize>, src_len: Option<usize>, count: usize) -> Checked {
    if dest_null && size == 0 && count == 0 {
        return Checked::Nothing;
    }
    if dest_null || size == 0 {
        return Checked::Fail { errno: EINVAL, clear: false };
    }
    let Some(d) = dest_len else {
        return Checked::Fail { errno: EINVAL, clear: true };
    };
    let Some(s) = src_len else {
        return if count == 0 { Checked::Nothing } else { Checked::Fail { errno: EINVAL, clear: true } };
    };
    if count == crate::printf::TRUNCATE {
        return if d + s < size { Checked::Copy(s) } else { Checked::Truncate(size - 1 - d) };
    }
    let n = s.min(count);
    if d + n >= size { Checked::Fail { errno: ERANGE, clear: true } } else { Checked::Copy(n) }
}

/// The result value of a [`Checked`] (what the function returns).
pub fn checked_result(c: Checked) -> i32 {
    match c {
        Checked::Copy(_) | Checked::Nothing => 0,
        Checked::Truncate(_) => STRUNCATE,
        Checked::Fail { errno, .. } => errno,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparisons() {
        assert_eq!(compare(b"abc", b"abd"), -1);
        assert_eq!(compare(b"abc", b"ab"), 1);
        assert_eq!(compare(b"", b""), 0);
        assert_eq!(compare(&[0x80u8], b"a"), 1, "unsigned");
        // `msvcrt`'s `memcmp` (equal lengths): the sign, not the
        // difference (ADR 0114).
        assert_eq!(compare(&[0u8, 0x6a], &[0u8, 0x01]), 1);
        assert_eq!(compare(&[1u8, 0xff], &[0xa1u8, 0]), -1);
        assert_eq!(compare(b"same", b"same"), 0);
        assert_eq!(icompare(b"HeLLo", b"hello"), 0);
        assert_eq!(icompare(b"a", b"B"), -1);
        assert_eq!(icompare(b"_", b"A"), -1, "both folded to lower: '_' < 'a'");
        let (a, b): (alloc::vec::Vec<u16>, alloc::vec::Vec<u16>) =
            ("\u{c9}T\u{c9}".encode_utf16().collect(), "\u{e9}t\u{e9}".encode_utf16().collect());
        assert_eq!(icompare(&a, &b), 0);
        let s = b"abcdef\0";
        // SAFETY: NUL-terminated.
        assert_eq!(unsafe { prefix(s.as_ptr(), 3) }, b"abc");
    }

    #[test]
    fn searching_and_tokens() {
        assert_eq!(find(b"hello world", b"o w"), Some(4));
        assert_eq!(find(b"hello", b""), Some(0));
        assert_eq!(find(b"hello", b"xyz"), None);
        assert_eq!(span(b"aabbc", b"ab", true), 4);
        assert_eq!(span(b"xyz,abc", b",;", false), 3);
        assert_eq!(span(b"xyz", b",", false), 3);
        assert_eq!(token(b"  a bc ", b" "), Some((2, 3, 4)));
        assert_eq!(token(b"bc ", b" "), Some((0, 2, 3)));
        assert_eq!(token(b"bc", b" "), Some((0, 2, 2)));
        assert_eq!(token(b"   ", b" "), None);
        assert_eq!(token(b"", b" "), None);
    }

    #[test]
    fn bounds_checked_rules() {
        assert_eq!(copy_s(false, 10, Some(3)), Checked::Copy(3));
        assert_eq!(copy_s(false, 3, Some(3)), Checked::Fail { errno: ERANGE, clear: true });
        assert_eq!(copy_s(true, 3, Some(1)), Checked::Fail { errno: EINVAL, clear: false });
        assert_eq!(copy_s(false, 0, Some(1)), Checked::Fail { errno: EINVAL, clear: false });
        assert_eq!(copy_s(false, 3, None), Checked::Fail { errno: EINVAL, clear: true });
        assert_eq!(ncopy_s(false, 5, Some(10), crate::printf::TRUNCATE), Checked::Truncate(4));
        assert_eq!(ncopy_s(false, 5, Some(3), crate::printf::TRUNCATE), Checked::Copy(3));
        assert_eq!(ncopy_s(false, 5, Some(10), 4), Checked::Copy(4));
        assert_eq!(ncopy_s(false, 5, Some(10), 5), Checked::Fail { errno: ERANGE, clear: true });
        assert_eq!(ncopy_s(true, 0, None, 0), Checked::Nothing);
        assert_eq!(ncopy_s(false, 5, None, 0), Checked::Copy(0));
        assert_eq!(cat_s(false, 10, Some(4), Some(5)), Checked::Copy(5));
        assert_eq!(cat_s(false, 10, Some(4), Some(6)), Checked::Fail { errno: ERANGE, clear: true });
        assert_eq!(cat_s(false, 10, None, Some(1)), Checked::Fail { errno: EINVAL, clear: true });
        assert_eq!(ncat_s(false, 10, Some(4), Some(9), crate::printf::TRUNCATE), Checked::Truncate(5));
        assert_eq!(ncat_s(false, 10, Some(4), Some(9), 2), Checked::Copy(2));
        assert_eq!(checked_result(Checked::Truncate(1)), STRUNCATE);
        assert_eq!(checked_result(Checked::Fail { errno: ERANGE, clear: true }), ERANGE);
    }
}
