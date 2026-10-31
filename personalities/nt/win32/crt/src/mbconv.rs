//! Multibyte characters by `LC_CTYPE` (ADR 0094; C11 7.22.7, 7.22.8,
//! 7.29.6; Learn's pages of each function): `mbtowc`, `wctomb`,
//! `mbstowcs`, `wcstombs`, the restartable `mbrtowc` / `wcrtomb` family,
//! the `_s` forms, `_mbstrlen`, and the `_l` forms.
//!
//! In "C" a byte is the character of the same value (ISO-8859-1, ADR
//! 0084); in a locale with a code page, characters are that code page's
//! (one byte, a lead and a trail byte, or UTF-8's one to four bytes),
//! converted by `kernel32`. A UTF-8 character above U+FFFF is two
//! `wchar_t` for `mbstowcs` and `wcstombs`; `mbrtowc` cannot return it in
//! one `wchar_t` and fails with `EILSEQ` (Poly's choice, to be confirmed).
//!
//! `mbstate_t` (`wchar.h`, 8 bytes) holds what a restartable function
//! keeps between calls: the count of pending bytes and up to three of
//! them, or a pending high surrogate for `wcrtomb`.

use crate::global::{self, Global};
use crate::locale::{self, Loc, LocaleT, loc, loc_of};
use alloc::vec::Vec;
use poly_crt::errno::{EILSEQ, EINVAL, ERANGE, STRUNCATE};
use poly_crt::unit;
use poly_nls::codepage::Decoded;

type L = *const LocaleT;

/// `(size_t)-1`, `(size_t)-2`, `_TRUNCATE`.
const FAIL: usize = usize::MAX;
const INCOMPLETE: usize = usize::MAX - 1;
const TRUNCATE: usize = usize::MAX;
/// `WEOF`, `EOF`.
const WEOF: u16 = 0xFFFF;
const EOF: i32 = -1;

/// The character at the front of `bytes` (not empty) in `l`'s `LC_CTYPE`:
/// its units and length.
fn decode(l: &Loc, bytes: &[u8]) -> Decoded {
    let b = bytes[0];
    if l.ctype_c {
        return Decoded::Char { units: [u16::from(b), 0], len: 1 };
    }
    if l.ctype_cp == poly_crt::locale::UTF8 {
        return poly_nls::codepage::decode_utf8(bytes);
    }
    let len = if l.is_lead(b) {
        match bytes.get(1) {
            None => return Decoded::Incomplete,
            Some(0) => return Decoded::Invalid { len: 1 },
            Some(_) => 2,
        }
    } else {
        1
    };
    match locale::to_wide(l.ctype_cp, &bytes[..len]).as_deref() {
        Some(&[w]) => Decoded::Char { units: [w, 0], len },
        _ => Decoded::Invalid { len },
    }
}

/// The bytes of UTF-16 units (one character) in `l`'s `LC_CTYPE`.
fn encode(l: &Loc, units: &[u16]) -> Option<Vec<u8>> {
    locale::from_wide(if l.ctype_c { 0 } else { l.ctype_cp }, units)
}

fn is_high(u: u16) -> bool {
    (0xD800..0xDC00).contains(&u)
}

/// The characters of a NUL-terminated string as UTF-16 (no NUL), or
/// `Err(())` at an invalid character; `max_units` stops early.
fn to_units(l: &Loc, s: &[u8], max_units: usize) -> Result<(Vec<u16>, usize), ()> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() && out.len() < max_units {
        match decode(l, &s[i..]) {
            Decoded::Char { units, len } => {
                let n = if units[1] != 0 { 2 } else { 1 };
                if out.len() + n > max_units {
                    break;
                }
                out.extend_from_slice(&units[..n]);
                i += len;
            }
            _ => return Err(()),
        }
    }
    Ok((out, i))
}

/// UTF-16 (no NUL) as bytes, or `Err(())` at a character the locale
/// lacks; `max_bytes` stops early before a character that does not fit.
fn to_bytes(l: &Loc, s: &[u16], max_bytes: usize) -> Result<(Vec<u8>, usize), ()> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let n = if is_high(s[i]) && s.get(i + 1).is_some_and(|&u| (0xDC00..0xE000).contains(&u)) { 2 } else { 1 };
        let b = encode(l, &s[i..i + n]).ok_or(())?;
        if out.len() + b.len() > max_bytes {
            break;
        }
        out.extend_from_slice(&b);
        i += n;
    }
    Ok((out, i))
}

// ----- one character ----------------------------------------------------------------------

/// # Safety
/// `out` NULL or writable; `s` NULL or `n` readable bytes (or to a NUL).
unsafe fn mbtowc_in(out: *mut u16, s: *const u8, n: usize, l: &Loc) -> i32 {
    if s.is_null() {
        return 0; // no shift states
    }
    if n == 0 {
        return -1;
    }
    // SAFETY: up to `n` bytes, stopping at a NUL.
    let bytes = unsafe { unit::slice_max(s, n.min(4)) };
    if bytes.is_empty() {
        if !out.is_null() {
            // SAFETY: writable.
            unsafe { *out = 0 };
        }
        return 0;
    }
    match decode(l, bytes) {
        Decoded::Char { units, len } if units[1] == 0 => {
            if !out.is_null() {
                // SAFETY: writable.
                unsafe { *out = units[0] };
            }
            len as i32
        }
        _ => {
            global::set_errno(EILSEQ);
            -1
        }
    }
}

/// # Safety
/// As `mbtowc_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbtowc(out: *mut u16, s: *const u8, n: usize) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { mbtowc_in(out, s, n, loc()) }
}

/// # Safety
/// As `mbtowc_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mbtowc_l(out: *mut u16, s: *const u8, n: usize, l: L) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { mbtowc_in(out, s, n, loc_of(l)) }
}

/// # Safety
/// As `mbtowc_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblen(s: *const u8, n: usize) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { mbtowc_in(core::ptr::null_mut(), s, n, loc()) }
}

/// # Safety
/// As `mbtowc_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mblen_l(s: *const u8, n: usize, l: L) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { mbtowc_in(core::ptr::null_mut(), s, n, loc_of(l)) }
}

/// # Safety
/// `out` NULL or `MB_CUR_MAX` writable bytes.
unsafe fn wctomb_in(out: *mut u8, c: u16, l: &Loc) -> i32 {
    if out.is_null() {
        return 0;
    }
    match encode(l, &[c]) {
        Some(b) => {
            // SAFETY: at most MB_CUR_MAX bytes.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, b.len()) };
            b.len() as i32
        }
        None => {
            global::set_errno(EILSEQ);
            -1
        }
    }
}

/// # Safety
/// As `wctomb_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wctomb(out: *mut u8, c: u16) -> i32 {
    // SAFETY: the caller's buffer.
    unsafe { wctomb_in(out, c, loc()) }
}

/// # Safety
/// As `wctomb_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wctomb_l(out: *mut u8, c: u16, l: L) -> i32 {
    // SAFETY: the caller's buffer.
    unsafe { wctomb_in(out, c, loc_of(l)) }
}

/// `wctomb_s` / `_wctomb_s_l`.
///
/// # Safety
/// `ret` NULL or writable; `out` NULL or `size` writable bytes.
unsafe fn wctomb_s_in(ret: *mut i32, out: *mut u8, size: usize, c: u16, l: &Loc) -> i32 {
    let put = |v: i32| {
        if !ret.is_null() {
            // SAFETY: writable.
            unsafe { *ret = v };
        }
    };
    if out.is_null() {
        if size != 0 {
            global::invalid(EINVAL);
            return EINVAL;
        }
        put(0);
        return 0;
    }
    match encode(l, &[c]) {
        Some(b) if b.len() <= size => {
            // SAFETY: b.len() <= size.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, b.len()) };
            put(b.len() as i32);
            0
        }
        Some(_) => {
            global::invalid(ERANGE);
            ERANGE
        }
        None => {
            put(-1);
            global::set_errno(EILSEQ);
            EILSEQ
        }
    }
}

/// # Safety
/// As `wctomb_s_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wctomb_s(ret: *mut i32, out: *mut u8, size: usize, c: u16) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { wctomb_s_in(ret, out, size, c, loc()) }
}

/// # Safety
/// As `wctomb_s_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wctomb_s_l(ret: *mut i32, out: *mut u8, size: usize, c: u16, l: L) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { wctomb_s_in(ret, out, size, c, loc_of(l)) }
}

/// `btowc`: a single-byte character's wide form, `WEOF` otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn btowc(c: i32) -> u16 {
    let l = loc();
    if !(0..=255).contains(&c) || l.is_lead(c as u8) {
        return WEOF;
    }
    match decode(l, &[c as u8]) {
        Decoded::Char { units, len: 1 } if units[1] == 0 => units[0],
        _ => WEOF,
    }
}

/// `wctob`: the single byte of a wide character, `EOF` otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn wctob(c: u16) -> i32 {
    match encode(loc(), &[c]).as_deref() {
        Some(&[b]) => i32::from(b),
        _ => EOF,
    }
}

// ----- strings ----------------------------------------------------------------------------

/// # Safety
/// `s` a valid C string; `out` NULL or `n` writable units.
unsafe fn mbstowcs_in(out: *mut u16, s: *const u8, n: usize, l: &Loc) -> usize {
    if s.is_null() || (out.is_null() && n != 0 && false) {
        global::invalid(EINVAL);
        return FAIL;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(s) };
    let max = if out.is_null() { usize::MAX } else { n };
    let Ok((units, _)) = to_units(l, t, max) else {
        global::set_errno(EILSEQ);
        return FAIL;
    };
    if !out.is_null() {
        // SAFETY: units.len() <= n.
        unsafe { core::ptr::copy_nonoverlapping(units.as_ptr(), out, units.len()) };
        if units.len() < n {
            // SAFETY: as above.
            unsafe { out.add(units.len()).write(0) };
        }
    }
    units.len()
}

/// # Safety
/// As `mbstowcs_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbstowcs(out: *mut u16, s: *const u8, n: usize) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { mbstowcs_in(out, s, n, loc()) }
}

/// # Safety
/// As `mbstowcs_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mbstowcs_l(out: *mut u16, s: *const u8, n: usize, l: L) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { mbstowcs_in(out, s, n, loc_of(l)) }
}

/// # Safety
/// `s` a valid wide C string; `out` NULL or `n` writable bytes.
unsafe fn wcstombs_in(out: *mut u8, s: *const u16, n: usize, l: &Loc) -> usize {
    if s.is_null() {
        global::invalid(EINVAL);
        return FAIL;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(s) };
    let max = if out.is_null() { usize::MAX } else { n };
    let Ok((bytes, _)) = to_bytes(l, t, max) else {
        global::set_errno(EILSEQ);
        return FAIL;
    };
    if !out.is_null() {
        // SAFETY: bytes.len() <= n.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len()) };
        if bytes.len() < n {
            // SAFETY: as above.
            unsafe { out.add(bytes.len()).write(0) };
        }
    }
    bytes.len()
}

/// # Safety
/// As `wcstombs_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstombs(out: *mut u8, s: *const u16, n: usize) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { wcstombs_in(out, s, n, loc()) }
}

/// # Safety
/// As `wcstombs_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcstombs_l(out: *mut u8, s: *const u16, n: usize, l: L) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { wcstombs_in(out, s, n, loc_of(l)) }
}

/// The `_s` rule (Learn, `mbstowcs_s`): `count` units at most (or
/// `_TRUNCATE`: as many as fit), and the NUL, in `size`; `*ret` the units
/// written with the NUL, or (`dst` NULL, `size` 0) the units needed.
///
/// # Safety
/// `ret` NULL or writable; `dst` NULL or `size` writable units.
unsafe fn finish_s<T: Copy + Default>(
    ret: *mut usize,
    dst: *mut T,
    size: usize,
    count: usize,
    all: &[T],
    whole: usize,
) -> i32 {
    let put = |v: usize| {
        if !ret.is_null() {
            // SAFETY: writable.
            unsafe { *ret = v };
        }
    };
    if dst.is_null() {
        put(whole + 1);
        return 0;
    }
    let want = all.len().min(count);
    if want < size {
        // SAFETY: want + 1 <= size.
        unsafe {
            core::ptr::copy_nonoverlapping(all.as_ptr(), dst, want);
            dst.add(want).write(T::default());
        }
        put(want + 1);
        return 0;
    }
    if count == TRUNCATE {
        let keep = size - 1;
        // SAFETY: keep + 1 == size.
        unsafe {
            core::ptr::copy_nonoverlapping(all.as_ptr(), dst, keep);
            dst.add(keep).write(T::default());
        }
        put(size);
        return STRUNCATE;
    }
    // SAFETY: size >= 1 (checked by the callers).
    unsafe { dst.write(T::default()) };
    put(0);
    global::invalid(ERANGE);
    ERANGE
}

/// # Safety
/// As `finish_s`; `s` a C string.
unsafe fn mbstowcs_s_in(ret: *mut usize, dst: *mut u16, size: usize, s: *const u8, count: usize, l: &Loc) -> i32 {
    if s.is_null() || (dst.is_null() && size != 0) || (!dst.is_null() && size == 0) {
        if !dst.is_null() && size != 0 {
            // SAFETY: one writable unit.
            unsafe { dst.write(0) };
        }
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(s) };
    match to_units(l, t, usize::MAX) {
        // SAFETY: the caller's buffers.
        Ok((units, _)) => unsafe { finish_s(ret, dst, size, count, &units, units.len()) },
        Err(()) => {
            if !dst.is_null() {
                // SAFETY: one writable unit.
                unsafe { dst.write(0) };
            }
            global::set_errno(EILSEQ);
            EILSEQ
        }
    }
}

/// # Safety
/// As `mbstowcs_s_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbstowcs_s(ret: *mut usize, dst: *mut u16, size: usize, s: *const u8, count: usize) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { mbstowcs_s_in(ret, dst, size, s, count, loc()) }
}

/// # Safety
/// As `mbstowcs_s_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mbstowcs_s_l(
    ret: *mut usize,
    dst: *mut u16,
    size: usize,
    s: *const u8,
    count: usize,
    l: L,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { mbstowcs_s_in(ret, dst, size, s, count, loc_of(l)) }
}

/// # Safety
/// As `finish_s`; `s` a wide C string.
unsafe fn wcstombs_s_in(ret: *mut usize, dst: *mut u8, size: usize, s: *const u16, count: usize, l: &Loc) -> i32 {
    if s.is_null() || (dst.is_null() && size != 0) || (!dst.is_null() && size == 0) {
        if !dst.is_null() && size != 0 {
            // SAFETY: one writable byte.
            unsafe { dst.write(0) };
        }
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(s) };
    // Whole characters only: as many bytes as `count` and the buffer allow.
    let room = if dst.is_null() {
        usize::MAX
    } else if count == TRUNCATE {
        size - 1
    } else {
        count.min(size)
    };
    let all = to_bytes(l, t, usize::MAX);
    match (all, to_bytes(l, t, room)) {
        (Ok((whole, _)), Ok((bytes, _))) => {
            let fits = if count == TRUNCATE || bytes.len() < size { bytes } else { whole.clone() };
            // SAFETY: the caller's buffers.
            unsafe { finish_s(ret, dst, size, count.min(fits.len()).max(fits.len().min(count)), &fits, whole.len()) }
        }
        _ => {
            if !dst.is_null() {
                // SAFETY: one writable byte.
                unsafe { dst.write(0) };
            }
            global::set_errno(EILSEQ);
            EILSEQ
        }
    }
}

/// # Safety
/// As `wcstombs_s_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstombs_s(ret: *mut usize, dst: *mut u8, size: usize, s: *const u16, count: usize) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { wcstombs_s_in(ret, dst, size, s, count, loc()) }
}

/// # Safety
/// As `wcstombs_s_in`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcstombs_s_l(
    ret: *mut usize,
    dst: *mut u8,
    size: usize,
    s: *const u16,
    count: usize,
    l: L,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { wcstombs_s_in(ret, dst, size, s, count, loc_of(l)) }
}

// ----- restartable ------------------------------------------------------------------------

/// The internal states of `mbrtowc`, `mbrlen`, `wcrtomb` … for a NULL
/// `mbstate_t *` (C11: "its own internal mbstate_t object").
static STATE_MBRTOWC: Global<[u8; 8]> = Global::new([0; 8]);
static STATE_MBRLEN: Global<[u8; 8]> = Global::new([0; 8]);
static STATE_WCRTOMB: Global<[u8; 8]> = Global::new([0; 8]);
static STATE_MBSRTOWCS: Global<[u8; 8]> = Global::new([0; 8]);
static STATE_WCSRTOMBS: Global<[u8; 8]> = Global::new([0; 8]);

/// The `mbstate_t` a function uses: the caller's, or its own static one.
#[allow(clippy::mut_from_ref)]
fn state(ps: *mut [u8; 8], own: &'static Global<[u8; 8]>) -> &'static mut [u8; 8] {
    // SAFETY: the caller's `mbstate_t` (8 bytes, `wchar.h`), or the
    // function's own (C11 lets those race).
    unsafe { &mut *(if ps.is_null() { own.get() } else { ps }) }
}

/// # Safety
/// `out` NULL or writable; `s` NULL or `n` readable bytes.
unsafe fn mbrtowc_in(out: *mut u16, s: *const u8, n: usize, st: &mut [u8; 8], l: &Loc) -> usize {
    if s.is_null() {
        *st = [0; 8];
        return 0;
    }
    if n == 0 {
        return INCOMPLETE;
    }
    let pending = usize::from(st[0].min(3));
    let mut buf: Vec<u8> = st[1..1 + pending].to_vec();
    // SAFETY: up to `n` bytes are the caller's.
    buf.extend_from_slice(unsafe { core::slice::from_raw_parts(s, n.min(4 - pending)) });
    match decode(l, &buf) {
        Decoded::Char { units, len } if units[1] == 0 && len > pending => {
            *st = [0; 8];
            if !out.is_null() {
                // SAFETY: writable.
                unsafe { *out = units[0] };
            }
            if units[0] == 0 { 0 } else { len - pending }
        }
        Decoded::Incomplete if buf.len() <= 3 => {
            st[0] = buf.len() as u8;
            st[1..1 + buf.len()].copy_from_slice(&buf);
            INCOMPLETE
        }
        _ => {
            *st = [0; 8];
            global::set_errno(EILSEQ);
            FAIL
        }
    }
}

/// # Safety
/// As `mbrtowc_in`; `ps` NULL or an `mbstate_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbrtowc(out: *mut u16, s: *const u8, n: usize, ps: *mut [u8; 8]) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { mbrtowc_in(out, s, n, state(ps, &STATE_MBRTOWC), loc()) }
}

/// # Safety
/// As `mbrtowc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbrlen(s: *const u8, n: usize, ps: *mut [u8; 8]) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { mbrtowc_in(core::ptr::null_mut(), s, n, state(ps, &STATE_MBRLEN), loc()) }
}

/// # Safety
/// `out` NULL or `MB_CUR_MAX` writable bytes.
unsafe fn wcrtomb_in(out: *mut u8, c: u16, st: &mut [u8; 8], l: &Loc) -> usize {
    if out.is_null() {
        *st = [0; 8];
        return 1;
    }
    let high = u16::from_le_bytes([st[4], st[5]]);
    let units: Vec<u16> = if is_high(c) && l.ctype_cp == poly_crt::locale::UTF8 && !l.ctype_c {
        st[4..6].copy_from_slice(&c.to_le_bytes());
        return 0;
    } else if high != 0 {
        alloc::vec![high, c]
    } else {
        alloc::vec![c]
    };
    *st = [0; 8];
    match encode(l, &units) {
        Some(b) => {
            // SAFETY: at most MB_CUR_MAX bytes.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, b.len()) };
            b.len()
        }
        None => {
            global::set_errno(EILSEQ);
            FAIL
        }
    }
}

/// # Safety
/// As `wcrtomb_in`; `ps` NULL or an `mbstate_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcrtomb(out: *mut u8, c: u16, ps: *mut [u8; 8]) -> usize {
    // SAFETY: the caller's arguments.
    unsafe { wcrtomb_in(out, c, state(ps, &STATE_WCRTOMB), loc()) }
}

/// `wcrtomb_s`.
///
/// # Safety
/// `ret` NULL or writable; `out` NULL or `size` writable bytes; `ps` NULL
/// or an `mbstate_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcrtomb_s(ret: *mut usize, out: *mut u8, size: usize, c: u16, ps: *mut [u8; 8]) -> i32 {
    let mut buf = [0u8; 8];
    let st = state(ps, &STATE_WCRTOMB);
    let target = if out.is_null() { core::ptr::null_mut() } else { buf.as_mut_ptr() };
    // SAFETY: our buffer holds MB_CUR_MAX bytes.
    let n = unsafe { wcrtomb_in(target, c, st, loc()) };
    let put = |v: usize| {
        if !ret.is_null() {
            // SAFETY: writable.
            unsafe { *ret = v };
        }
    };
    if n == FAIL {
        put(FAIL);
        return EILSEQ;
    }
    if !out.is_null() {
        if n > size {
            global::invalid(ERANGE);
            return ERANGE;
        }
        // SAFETY: n <= size.
        unsafe { core::ptr::copy_nonoverlapping(buf.as_ptr(), out, n) };
    }
    put(n);
    0
}

/// # Safety
/// `src` points at a C string pointer; `dst` NULL or `len` writable units.
unsafe fn mbsrtowcs_in(dst: *mut u16, src: *mut *const u8, len: usize, l: &Loc) -> usize {
    // SAFETY: the caller's pointer.
    if src.is_null() || unsafe { (*src).is_null() } {
        global::invalid(EINVAL);
        return FAIL;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(*src) };
    let max = if dst.is_null() { usize::MAX } else { len };
    let Ok((units, used)) = to_units(l, t, max) else {
        global::set_errno(EILSEQ);
        return FAIL;
    };
    if !dst.is_null() {
        // SAFETY: units.len() <= len.
        unsafe { core::ptr::copy_nonoverlapping(units.as_ptr(), dst, units.len()) };
        if used == t.len() && units.len() < len {
            // SAFETY: room for the NUL; the whole string was converted.
            unsafe {
                dst.add(units.len()).write(0);
                *src = core::ptr::null();
            }
        } else {
            // SAFETY: inside the string.
            unsafe { *src = (*src).add(used) };
        }
    }
    units.len()
}

/// # Safety
/// As `mbsrtowcs_in`; `ps` NULL or an `mbstate_t` (no shift states: unused).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbsrtowcs(dst: *mut u16, src: *mut *const u8, len: usize, ps: *mut [u8; 8]) -> usize {
    let _ = state(ps, &STATE_MBSRTOWCS);
    // SAFETY: the caller's arguments.
    unsafe { mbsrtowcs_in(dst, src, len, loc()) }
}

/// # Safety
/// `src` points at a wide C string pointer; `dst` NULL or `len` writable
/// bytes.
unsafe fn wcsrtombs_in(dst: *mut u8, src: *mut *const u16, len: usize, l: &Loc) -> usize {
    // SAFETY: the caller's pointer.
    if src.is_null() || unsafe { (*src).is_null() } {
        global::invalid(EINVAL);
        return FAIL;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(*src) };
    let max = if dst.is_null() { usize::MAX } else { len };
    let Ok((bytes, used)) = to_bytes(l, t, max) else {
        global::set_errno(EILSEQ);
        return FAIL;
    };
    if !dst.is_null() {
        // SAFETY: bytes.len() <= len.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len()) };
        if used == t.len() && bytes.len() < len {
            // SAFETY: room for the NUL; the whole string was converted.
            unsafe {
                dst.add(bytes.len()).write(0);
                *src = core::ptr::null();
            }
        } else {
            // SAFETY: inside the string.
            unsafe { *src = (*src).add(used) };
        }
    }
    bytes.len()
}

/// # Safety
/// As `wcsrtombs_in`; `ps` NULL or an `mbstate_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsrtombs(dst: *mut u8, src: *mut *const u16, len: usize, ps: *mut [u8; 8]) -> usize {
    let _ = state(ps, &STATE_WCSRTOMBS);
    // SAFETY: the caller's arguments.
    unsafe { wcsrtombs_in(dst, src, len, loc()) }
}

/// `mbsrtowcs_s`.
///
/// # Safety
/// As `mbstowcs_s`, with `src` pointing at the string pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbsrtowcs_s(
    ret: *mut usize,
    dst: *mut u16,
    size: usize,
    src: *mut *const u8,
    count: usize,
    _ps: *mut [u8; 8],
) -> i32 {
    // SAFETY: the caller's pointer.
    if src.is_null() || unsafe { (*src).is_null() } {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's arguments.
    let r = unsafe { mbstowcs_s_in(ret, dst, size, *src, count, loc()) };
    if r == 0 && !dst.is_null() {
        // SAFETY: the caller's pointer.
        unsafe { *src = core::ptr::null() };
    }
    r
}

/// `wcsrtombs_s`.
///
/// # Safety
/// As `wcstombs_s`, with `src` pointing at the string pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsrtombs_s(
    ret: *mut usize,
    dst: *mut u8,
    size: usize,
    src: *mut *const u16,
    count: usize,
    _ps: *mut [u8; 8],
) -> i32 {
    // SAFETY: the caller's pointer.
    if src.is_null() || unsafe { (*src).is_null() } {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's arguments.
    let r = unsafe { wcstombs_s_in(ret, dst, size, *src, count, loc()) };
    if r == 0 && !dst.is_null() {
        // SAFETY: the caller's pointer.
        unsafe { *src = core::ptr::null() };
    }
    r
}

// ----- counting ---------------------------------------------------------------------------

/// # Safety
/// `s` a C string, read up to `max` bytes.
unsafe fn mbstrnlen_in(s: *const u8, max: usize, l: &Loc) -> usize {
    if s.is_null() {
        global::invalid(EINVAL);
        return FAIL;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice_max(s, max) };
    let mut i = 0;
    let mut k = 0;
    while i < t.len() {
        match decode(l, &t[i..]) {
            Decoded::Char { len, .. } => {
                i += len;
                k += 1;
            }
            Decoded::Incomplete => break,
            Decoded::Invalid { .. } => {
                global::set_errno(EILSEQ);
                return FAIL;
            }
        }
    }
    k
}

/// `_mbstrlen`: the characters of `s` by `LC_CTYPE`; −1 and `EILSEQ` at an
/// invalid one.
///
/// # Safety
/// `s` a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mbstrlen(s: *const u8) -> usize {
    // SAFETY: the caller's string.
    unsafe { mbstrnlen_in(s, usize::MAX, loc()) }
}

/// # Safety
/// `s` a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mbstrlen_l(s: *const u8, l: L) -> usize {
    // SAFETY: the caller's string.
    unsafe { mbstrnlen_in(s, usize::MAX, loc_of(l)) }
}

/// # Safety
/// `s` a C string.
pub unsafe extern "C" fn _mbstrnlen(s: *const u8, max: usize) -> usize {
    // SAFETY: the caller's string.
    unsafe { mbstrnlen_in(s, max, loc()) }
}

/// # Safety
/// `s` a C string.
pub unsafe extern "C" fn _mbstrnlen_l(s: *const u8, max: usize, l: L) -> usize {
    // SAFETY: the caller's string.
    unsafe { mbstrnlen_in(s, max, loc_of(l)) }
}
