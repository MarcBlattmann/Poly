//! The C string functions (C11 7.24, and `strlcpy`/`strlcat`/`strnlen`/
//! `stpcpy` of the macOS manual pages) and the number parsing of `strtol`
//! and friends (C11 7.22.1.4), over raw pointers as C passes them. The
//! dylib exports them under their C names.

/// `strlen`.
///
/// # Safety
/// `s` is a NUL-terminated string.
pub unsafe fn strlen(s: *const u8) -> usize {
    let mut n = 0;
    // SAFETY: the caller's contract.
    while unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    n
}

/// `strnlen`: at most `max` bytes are read.
///
/// # Safety
/// `s` is a string or has `max` readable bytes.
pub unsafe fn strnlen(s: *const u8, max: usize) -> usize {
    let mut n = 0;
    // SAFETY: the caller's contract.
    while n < max && unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    n
}

/// `strncmp` (`strcmp` with `usize::MAX`): bytes compared as unsigned.
///
/// # Safety
/// Both are strings (or have `n` readable bytes).
pub unsafe fn strncmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    for i in 0..n {
        // SAFETY: the caller's contract; a NUL in either ends the loop.
        let (x, y) = unsafe { (*a.add(i), *b.add(i)) };
        if x != y {
            return i32::from(x) - i32::from(y);
        }
        if x == 0 {
            return 0;
        }
    }
    0
}

/// `memcmp`.
///
/// # Safety
/// Both have `n` readable bytes.
pub unsafe fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    for i in 0..n {
        // SAFETY: the caller's contract.
        let (x, y) = unsafe { (*a.add(i), *b.add(i)) };
        if x != y {
            return i32::from(x) - i32::from(y);
        }
    }
    0
}

/// `memchr`: the first `c` (as an unsigned char) in `n` bytes.
///
/// # Safety
/// `s` has `n` readable bytes.
pub unsafe fn memchr(s: *const u8, c: i32, n: usize) -> *const u8 {
    for i in 0..n {
        // SAFETY: the caller's contract.
        if unsafe { *s.add(i) } == c as u8 {
            // SAFETY: inside the area.
            return unsafe { s.add(i) };
        }
    }
    core::ptr::null()
}

/// `strchr`: the first `c` (as a char), which may be the NUL.
///
/// # Safety
/// `s` is a string.
pub unsafe fn strchr(s: *const u8, c: i32) -> *const u8 {
    let c = c as u8;
    let mut p = s;
    loop {
        // SAFETY: the caller's contract; the loop stops at the NUL.
        let x = unsafe { *p };
        if x == c {
            return p;
        }
        if x == 0 {
            return core::ptr::null();
        }
        // SAFETY: before the NUL.
        p = unsafe { p.add(1) };
    }
}

/// `strrchr`: the last `c` (as a char), which may be the NUL.
///
/// # Safety
/// `s` is a string.
pub unsafe fn strrchr(s: *const u8, c: i32) -> *const u8 {
    let c = c as u8;
    let mut found = core::ptr::null();
    let mut p = s;
    loop {
        // SAFETY: the caller's contract.
        let x = unsafe { *p };
        if x == c {
            found = p;
        }
        if x == 0 {
            return found;
        }
        // SAFETY: before the NUL.
        p = unsafe { p.add(1) };
    }
}

/// `strstr`: the first occurrence of `needle`; `hay` for an empty needle.
///
/// # Safety
/// Both are strings.
pub unsafe fn strstr(hay: *const u8, needle: *const u8) -> *const u8 {
    // SAFETY: the caller's contract.
    let n = unsafe { strlen(needle) };
    if n == 0 {
        return hay;
    }
    let mut p = hay;
    // SAFETY: the comparison stops at the first difference, so it never
    // reads past `hay`'s NUL (which differs from the needle's bytes).
    while unsafe { *p } != 0 {
        // SAFETY: as above.
        if unsafe { strncmp(p, needle, n) } == 0 {
            return p;
        }
        // SAFETY: before the NUL.
        p = unsafe { p.add(1) };
    }
    core::ptr::null()
}

/// `stpcpy`: copies `src` with its NUL; the address of the copied NUL.
///
/// # Safety
/// `src` is a string; `dst` has room for it; they do not overlap.
pub unsafe fn stpcpy(dst: *mut u8, src: *const u8) -> *mut u8 {
    // SAFETY: the caller's contract.
    let n = unsafe { strlen(src) };
    // SAFETY: as above.
    unsafe {
        core::ptr::copy_nonoverlapping(src, dst, n + 1);
        dst.add(n)
    }
}

/// `strncpy`: at most `n` bytes, the rest of `n` filled with NULs.
///
/// # Safety
/// `dst` has `n` bytes; `src` is a string or has `n` bytes.
pub unsafe fn strncpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    // SAFETY: the caller's contract.
    let len = unsafe { strnlen(src, n) };
    // SAFETY: as above.
    unsafe {
        core::ptr::copy_nonoverlapping(src, dst, len);
        core::ptr::write_bytes(dst.add(len), 0, n - len);
    }
    dst
}

/// `strlcpy`: copies what fits in `size - 1` bytes and a NUL (if `size` is
/// not 0); the length of `src`.
///
/// # Safety
/// `src` is a string; `dst` has `size` bytes.
pub unsafe fn strlcpy(dst: *mut u8, src: *const u8, size: usize) -> usize {
    // SAFETY: the caller's contract.
    let len = unsafe { strlen(src) };
    if size > 0 {
        let n = len.min(size - 1);
        // SAFETY: as above.
        unsafe {
            core::ptr::copy_nonoverlapping(src, dst, n);
            *dst.add(n) = 0;
        }
    }
    len
}

/// `strlcat`: appends what fits in a buffer of `size` bytes; the length of
/// the string it tried to make (`size` + the length of `src` if `dst` has
/// no NUL within `size` bytes).
///
/// # Safety
/// `src` is a string; `dst` has `size` bytes.
pub unsafe fn strlcat(dst: *mut u8, src: *const u8, size: usize) -> usize {
    // SAFETY: the caller's contract.
    let have = unsafe { strnlen(dst, size) };
    if have == size {
        // SAFETY: as above.
        return size + unsafe { strlen(src) };
    }
    // SAFETY: as above.
    have + unsafe { strlcpy(dst.add(have), src, size - have) }
}

/// `strncat`: appends at most `n` bytes of `src` and a NUL.
///
/// # Safety
/// `dst` is a string with room for the result; `src` is a string or has
/// `n` bytes.
pub unsafe fn strncat(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    // SAFETY: the caller's contract.
    unsafe {
        let at = dst.add(strlen(dst));
        let len = strnlen(src, n);
        core::ptr::copy_nonoverlapping(src, at, len);
        *at.add(len) = 0;
    }
    dst
}

/// What `strtol` and friends found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parsed {
    /// The value, already clamped to the type's range on overflow.
    pub value: u64,
    /// How many bytes were used (0: no number; `endptr` is the start).
    pub used: usize,
    /// Out of range: `ERANGE`.
    pub range: bool,
    /// A base other than 0 and 2..=36: `EINVAL`.
    pub bad_base: bool,
}

/// `strtol` (`signed`, limits `min`..=`max`) and `strtoul` (unsigned, a
/// `-` negates in the unsigned type) for a type of `bits` bits.
///
/// # Safety
/// `s` is a string.
pub unsafe fn strto(s: *const u8, base: i32, signed: bool, bits: u32) -> Parsed {
    let at = |i: usize| -> u8 {
        // SAFETY: the caller's contract; reading stops at the NUL, which no
        // test below accepts.
        unsafe { *s.add(i) }
    };
    let none = Parsed { value: 0, used: 0, range: false, bad_base: false };
    if base != 0 && !(2..=36).contains(&base) {
        return Parsed { bad_base: true, ..none };
    }
    let mut i = 0;
    while matches!(at(i), b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') {
        i += 1;
    }
    let neg = at(i) == b'-';
    if matches!(at(i), b'+' | b'-') {
        i += 1;
    }
    let mut base = base as u64;
    let hex_prefix = at(i) == b'0' && matches!(at(i + 1), b'x' | b'X') && digit(at(i + 2)).is_some_and(|d| d < 16);
    if (base == 0 || base == 16) && hex_prefix {
        i += 2;
        base = 16;
    } else if base == 0 {
        base = if at(i) == b'0' { 8 } else { 10 };
    }
    let start = i;
    let mut v: u64 = 0;
    let mut over = false;
    while let Some(d) = digit(at(i)).filter(|&d| u64::from(d) < base) {
        match v.checked_mul(base).and_then(|x| x.checked_add(u64::from(d))) {
            Some(x) => v = x,
            None => over = true,
        }
        i += 1;
    }
    if i == start {
        return none;
    }
    let umax = if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 };
    let (value, range) = if signed {
        let max = umax >> 1;
        if neg {
            if over || v > max + 1 { ((max + 1).wrapping_neg(), true) } else { (v.wrapping_neg(), false) }
        } else if over || v > max {
            (max, true)
        } else {
            (v, false)
        }
    } else if over || v > umax {
        (umax, true)
    } else if neg {
        (v.wrapping_neg() & umax, false)
    } else {
        (v, false)
    };
    Parsed { value, used: i, range, bad_base: false }
}

fn digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'z' => Some(c - b'a' + 10),
        b'A'..=b'Z' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// `getenv`: the value of `name` in an `environ` array, or null.
///
/// # Safety
/// `name` is a string; `env` is null or a null-terminated array of
/// strings.
pub unsafe fn getenv(env: *const *const u8, name: *const u8) -> *const u8 {
    if env.is_null() {
        return core::ptr::null();
    }
    // SAFETY: the caller's contract.
    let n = unsafe { strlen(name) };
    let mut e = env;
    // SAFETY: the array ends with null.
    while let Some(entry) = Some(unsafe { *e }).filter(|p| !p.is_null()) {
        // SAFETY: entries are strings; the comparison stops at a difference.
        if unsafe { strncmp(entry, name, n) } == 0 && unsafe { *entry.add(n) } == b'=' {
            // SAFETY: after the '='.
            return unsafe { entry.add(n + 1) };
        }
        // SAFETY: before the terminating null.
        e = unsafe { e.add(1) };
    }
    core::ptr::null()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &[u8]) -> *const u8 {
        s.as_ptr()
    }

    #[test]
    fn lengths_and_comparisons() {
        // SAFETY: NUL-terminated literals.
        unsafe {
            assert_eq!(strlen(p(b"hello\0")), 5);
            assert_eq!(strlen(p(b"\0")), 0);
            assert_eq!(strnlen(p(b"hello\0"), 3), 3);
            assert_eq!(strnlen(p(b"hi\0"), 9), 2);
            assert_eq!(strncmp(p(b"abc\0"), p(b"abc\0"), usize::MAX), 0);
            assert!(strncmp(p(b"abc\0"), p(b"abd\0"), usize::MAX) < 0);
            assert!(strncmp(p(b"ab\0"), p(b"abc\0"), usize::MAX) < 0);
            assert!(strncmp(p(b"\xff\0"), p(b"a\0"), usize::MAX) > 0);
            assert_eq!(strncmp(p(b"abcx\0"), p(b"abcy\0"), 3), 0);
            assert_eq!(memcmp(p(b"ab"), p(b"ab"), 2), 0);
            assert!(memcmp(p(b"\x01"), p(b"\xff"), 1) < 0);
        }
    }

    #[test]
    fn searches() {
        let s = b"hello world\0";
        // SAFETY: NUL-terminated literals.
        unsafe {
            assert_eq!(strchr(p(s), i32::from(b'o')), p(s).add(4));
            assert_eq!(strrchr(p(s), i32::from(b'o')), p(s).add(7));
            assert_eq!(strchr(p(s), 0), p(s).add(11));
            assert!(strchr(p(s), i32::from(b'z')).is_null());
            assert_eq!(memchr(p(s), i32::from(b'w'), 11), p(s).add(6));
            assert!(memchr(p(s), i32::from(b'w'), 6).is_null());
            assert_eq!(strstr(p(s), p(b"wor\0")), p(s).add(6));
            assert_eq!(strstr(p(s), p(b"\0")), p(s));
            assert!(strstr(p(s), p(b"word\0")).is_null());
        }
    }

    #[test]
    fn copies() {
        let mut buf = [0xaau8; 8];
        let d = buf.as_mut_ptr();
        // SAFETY: the buffer has 8 bytes; literals are NUL-terminated.
        unsafe {
            assert_eq!(stpcpy(d, p(b"abc\0")), d.add(3));
            assert_eq!(&buf[..4], b"abc\0");
            strncpy(d, p(b"xy\0"), 5);
            assert_eq!(&buf[..6], b"xy\0\0\0\xaa");
            assert_eq!(strlcpy(d, p(b"truncated\0"), 4), 9);
            assert_eq!(&buf[..4], b"tru\0");
            assert_eq!(strlcat(d, p(b"ncate\0"), 8), 8);
            assert_eq!(&buf, b"truncat\0");
            assert_eq!(strlcat(d, p(b"xx\0"), 4), 6);
            strlcpy(d, p(b"ab\0"), 8);
            strncat(d, p(b"cdef\0"), 2);
            assert_eq!(&buf[..5], b"abcd\0");
        }
    }

    fn sl(s: &[u8], base: i32) -> (i64, usize, bool) {
        // SAFETY: NUL-terminated.
        let r = unsafe { strto(s.as_ptr(), base, true, 64) };
        (r.value as i64, r.used, r.range)
    }

    #[test]
    fn string_to_numbers() {
        assert_eq!(sl(b"  42xyz\0", 10), (42, 4, false));
        assert_eq!(sl(b"-17\0", 10), (-17, 3, false));
        assert_eq!(sl(b"+0x1F\0", 0), (31, 5, false));
        assert_eq!(sl(b"0x1F\0", 16), (31, 4, false));
        assert_eq!(sl(b"017\0", 0), (15, 3, false));
        assert_eq!(sl(b"0x\0", 0), (0, 1, false));
        assert_eq!(sl(b"zz\0", 36), (36 * 35 + 35, 2, false));
        assert_eq!(sl(b"abc\0", 10), (0, 0, false));
        assert_eq!(sl(b"9223372036854775807\0", 10), (i64::MAX, 19, false));
        assert_eq!(sl(b"9223372036854775808\0", 10), (i64::MAX, 19, true));
        assert_eq!(sl(b"-9223372036854775808\0", 10), (i64::MIN, 20, false));
        assert_eq!(sl(b"-99999999999999999999\0", 10), (i64::MIN, 21, true));
        // SAFETY: NUL-terminated.
        unsafe {
            assert!(strto(p(b"1\0"), 1, true, 64).bad_base);
            let r = strto(p(b"2147483648\0"), 10, true, 32);
            assert_eq!((r.value, r.range), (0x7fff_ffff, true));
            let r = strto(p(b"-1\0"), 10, false, 64);
            assert_eq!((r.value, r.range), (u64::MAX, false));
            let r = strto(p(b"-1\0"), 10, false, 32);
            assert_eq!((r.value, r.range), (0xffff_ffff, false));
            let r = strto(p(b"4294967296\0"), 10, false, 32);
            assert_eq!((r.value, r.range), (0xffff_ffff, true));
        }
    }

    #[test]
    fn environment() {
        let a = b"HOME=/root\0";
        let b = b"HOMEX=no\0";
        let c = b"PATH=/bin\0";
        let env = [b.as_ptr(), a.as_ptr(), c.as_ptr(), core::ptr::null()];
        // SAFETY: a null-terminated array of strings.
        unsafe {
            assert_eq!(getenv(env.as_ptr(), p(b"HOME\0")), a.as_ptr().add(5));
            assert_eq!(getenv(env.as_ptr(), p(b"PATH\0")), c.as_ptr().add(5));
            assert!(getenv(env.as_ptr(), p(b"HOM\0")).is_null());
            assert!(getenv(core::ptr::null(), p(b"HOME\0")).is_null());
        }
    }
}
