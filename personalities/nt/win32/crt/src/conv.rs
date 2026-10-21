//! Conversions (C11 7.22.1, 7.29.4.1; Learn: `strtol`, `strtod`, `atoi`,
//! `_itoa`, `mbstowcs`, `wcstombs`, `mbtowc`, `wctomb`): numbers on
//! [`poly_crt::convert`] and [`poly_crt::float`], multibyte characters in
//! the "C" locale (one byte per character, ISO-8859-1, see
//! [`poly_crt::unit::c_locale_to_wide`]).

use crate::global;
use poly_crt::convert::{self, Kind};
use poly_crt::ctype;
use poly_crt::errno::{EILSEQ, EINVAL, ERANGE};
use poly_crt::float::{self, Format};
use poly_crt::unit::{self, Unit};

/// # Safety
/// `s` is a valid C string; `end` NULL or writable.
unsafe fn strto<C: Unit>(s: *const C, end: *mut *mut C, base: i32, kind: Kind) -> u64 {
    if s.is_null() || !(base == 0 || (2..=36).contains(&base)) {
        global::invalid(EINVAL);
        if !end.is_null() {
            // SAFETY: writable.
            unsafe { *end = s as *mut C };
        }
        return 0;
    }
    // SAFETY: the caller's string.
    let text = unsafe { unit::slice(s) };
    let r = convert::parse(text, base as u32, kind);
    if r.overflow {
        global::set_errno(ERANGE);
    }
    if !end.is_null() {
        // SAFETY: writable; `len` units are inside the string.
        unsafe { *end = s.add(r.len) as *mut C };
    }
    r.value
}

/// Rows marked `#[unsafe(no_mangle)]` are exported by both DLLs; the others
/// are `ucrtbase`'s alone, which exports them itself.
macro_rules! strto_exports {
    ($($(#[$m:meta])* $name:ident: $unit:ty => $ret:ty, $kind:expr;)*) => {
        $(
            /// # Safety
            /// `s` is a valid C string; `end` NULL or writable.
            $(#[$m])*
            pub unsafe extern "C" fn $name(s: *const $unit, end: *mut *mut $unit, base: i32) -> $ret {
                // SAFETY: the caller's arguments.
                unsafe { strto(s, end, base, $kind) as $ret }
            }
        )*
    };
}

strto_exports! {
    #[unsafe(no_mangle)] strtol: u8 => i32, Kind::I32;
    #[unsafe(no_mangle)] strtoul: u8 => u32, Kind::U32;
    strtoll: u8 => i64, Kind::I64;
    strtoull: u8 => u64, Kind::U64;
    #[unsafe(no_mangle)] _strtoi64: u8 => i64, Kind::I64;
    #[unsafe(no_mangle)] _strtoui64: u8 => u64, Kind::U64;
    #[unsafe(no_mangle)] wcstol: u16 => i32, Kind::I32;
    #[unsafe(no_mangle)] wcstoul: u16 => u32, Kind::U32;
    wcstoll: u16 => i64, Kind::I64;
    wcstoull: u16 => u64, Kind::U64;
    #[unsafe(no_mangle)] _wcstoi64: u16 => i64, Kind::I64;
    #[unsafe(no_mangle)] _wcstoui64: u16 => u64, Kind::U64;
}

/// # Safety
/// `s` is a valid C string.
unsafe fn ato<C: Unit>(s: *const C, wide64: bool) -> i64 {
    if s.is_null() {
        global::invalid(EINVAL);
        return 0;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(s) };
    let (v, over) = if wide64 { convert::atoi64(t) } else { (i64::from(convert::atoi(t).0), convert::atoi(t).1) };
    if over {
        global::set_errno(ERANGE);
    }
    v
}

macro_rules! ato_exports {
    ($($(#[$m:meta])* $name:ident: $unit:ty => $ret:ty, $wide:expr;)*) => {
        $(
            /// # Safety
            /// `s` is a valid C string.
            $(#[$m])*
            pub unsafe extern "C" fn $name(s: *const $unit) -> $ret {
                // SAFETY: the caller's string.
                unsafe { ato(s, $wide) as $ret }
            }
        )*
    };
}

ato_exports! {
    #[unsafe(no_mangle)] atoi: u8 => i32, false;
    #[unsafe(no_mangle)] atol: u8 => i32, false;
    atoll: u8 => i64, true;
    #[unsafe(no_mangle)] _atoi64: u8 => i64, true;
    #[unsafe(no_mangle)] _wtoi: u16 => i32, false;
    #[unsafe(no_mangle)] _wtol: u16 => i32, false;
    _wtoll: u16 => i64, true;
    #[unsafe(no_mangle)] _wtoi64: u16 => i64, true;
}

/// # Safety
/// `s` is a valid C string; `end` NULL or writable.
unsafe fn strtofloat<C: Unit>(s: *const C, end: *mut *mut C, fmt: Format) -> u64 {
    if s.is_null() {
        global::invalid(EINVAL);
        return 0;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(s) };
    let lead = t.iter().take_while(|c| ctype::is_space_unit(c.to_u32(), C::WIDE)).count();
    let p = float::parse(&t[lead..], fmt);
    let used = if p.len == 0 { 0 } else { lead + p.len };
    if p.range_error {
        global::set_errno(ERANGE);
    }
    if !end.is_null() {
        // SAFETY: writable; inside the string.
        unsafe { *end = s.add(used) as *mut C };
    }
    p.bits
}

/// # Safety
/// `s` is a valid C string; `end` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtod(s: *const u8, end: *mut *mut u8) -> f64 {
    // SAFETY: the caller's arguments.
    f64::from_bits(unsafe { strtofloat(s, end, float::DOUBLE) })
}

/// # Safety
/// As `strtod`.
pub unsafe extern "C" fn strtof(s: *const u8, end: *mut *mut u8) -> f32 {
    // SAFETY: the caller's arguments.
    f32::from_bits(unsafe { strtofloat(s, end, float::SINGLE) } as u32)
}

/// # Safety
/// As `strtod`.
pub unsafe extern "C" fn strtold(s: *const u8, end: *mut *mut u8) -> f64 {
    // SAFETY: the caller's arguments.
    unsafe { strtod(s, end) }
}

/// # Safety
/// As `strtod`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstod(s: *const u16, end: *mut *mut u16) -> f64 {
    // SAFETY: the caller's arguments.
    f64::from_bits(unsafe { strtofloat(s, end, float::DOUBLE) })
}

/// # Safety
/// As `strtod`.
pub unsafe extern "C" fn wcstof(s: *const u16, end: *mut *mut u16) -> f32 {
    // SAFETY: the caller's arguments.
    f32::from_bits(unsafe { strtofloat(s, end, float::SINGLE) } as u32)
}

/// # Safety
/// `s` is a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atof(s: *const u8) -> f64 {
    // SAFETY: the caller's string.
    unsafe { strtod(s, core::ptr::null_mut()) }
}

/// # Safety
/// As `atof`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wtof(s: *const u16) -> f64 {
    // SAFETY: the caller's string.
    unsafe { wcstod(s, core::ptr::null_mut()) }
}

// ----- integers to text -------------------------------------------------------------

/// Write `_itoa`'s digits (and NUL) to `buf`.
///
/// # Safety
/// `buf` has room (Learn: up to 33 units for 32-bit, 65 for 64-bit).
unsafe fn itoa<C: Unit>(v: u64, bits: u32, signed: bool, buf: *mut C, radix: i32) -> *mut C {
    if buf.is_null() || !(2..=36).contains(&radix) {
        global::invalid(EINVAL);
        if !buf.is_null() {
            // SAFETY: at least one unit.
            unsafe { buf.write(C::default()) };
        }
        return buf;
    }
    let d = convert::itoa(v, bits, signed, radix as u32);
    for (i, &b) in d.iter().enumerate() {
        // SAFETY: the caller's room.
        unsafe { buf.add(i).write(C::ascii(b)) };
    }
    // SAFETY: as above.
    unsafe { buf.add(d.len()).write(C::default()) };
    buf
}

macro_rules! itoa_exports {
    ($($name:ident: $arg:ty, $unit:ty, $bits:expr, $signed:expr;)*) => {
        $(
            /// # Safety
            /// `buf` has room for the digits, a sign and a NUL.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name(v: $arg, buf: *mut $unit, radix: i32) -> *mut $unit {
                // SAFETY: the caller's buffer.
                unsafe { itoa(v as u64, $bits, $signed, buf, radix) }
            }
        )*
    };
}

itoa_exports! {
    _itoa: i32, u8, 32, true;
    _ltoa: i32, u8, 32, true;
    _ultoa: u32, u8, 32, false;
    _i64toa: i64, u8, 64, true;
    _ui64toa: u64, u8, 64, false;
    _itow: i32, u16, 32, true;
    _ltow: i32, u16, 32, true;
    _ultow: u32, u16, 32, false;
    _i64tow: i64, u16, 64, true;
    _ui64tow: u64, u16, 64, false;
}

// ----- multibyte characters ("C" locale) ------------------------------------------------

/// `mbtowc`: one byte is one character.
///
/// # Safety
/// `out` NULL or writable; `s` NULL or `n` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbtowc(out: *mut u16, s: *const u8, n: usize) -> i32 {
    if s.is_null() {
        return 0; // no shift states
    }
    if n == 0 {
        return -1;
    }
    // SAFETY: one readable byte.
    let b = unsafe { *s };
    if !out.is_null() {
        // SAFETY: writable.
        unsafe { *out = unit::c_locale_to_wide(b) };
    }
    i32::from(b != 0)
}

/// # Safety
/// As `mbtowc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblen(s: *const u8, n: usize) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { mbtowc(core::ptr::null_mut(), s, n) }
}

/// `wctomb`: −1 (`EILSEQ`) for a character the "C" locale lacks.
///
/// # Safety
/// `out` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wctomb(out: *mut u8, c: u16) -> i32 {
    if out.is_null() {
        return 0;
    }
    match unit::c_locale_to_narrow(c) {
        Some(b) => {
            // SAFETY: writable.
            unsafe { *out = b };
            1
        }
        None => {
            global::set_errno(EILSEQ);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn btowc(c: i32) -> u16 {
    if (0..=255).contains(&c) { unit::c_locale_to_wide(c as u8) } else { 0xFFFF }
}

#[unsafe(no_mangle)]
pub extern "C" fn wctob(c: u16) -> i32 {
    unit::c_locale_to_narrow(c).map_or(-1, i32::from)
}

/// `mbstowcs`: the characters converted (without the NUL), or the length
/// needed when `out` is NULL.
///
/// # Safety
/// `s` a valid C string; `out` NULL or `n` writable units.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbstowcs(out: *mut u16, s: *const u8, n: usize) -> usize {
    if s.is_null() {
        global::invalid(EINVAL);
        return usize::MAX;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(s) };
    if out.is_null() {
        return t.len();
    }
    let m = t.len().min(n);
    for (i, &b) in t[..m].iter().enumerate() {
        // SAFETY: i < n.
        unsafe { out.add(i).write(unit::c_locale_to_wide(b)) };
    }
    if m < n {
        // SAFETY: m < n.
        unsafe { out.add(m).write(0) };
    }
    m
}

/// `wcstombs`: the bytes converted, or −1 (`EILSEQ`) at a character the
/// "C" locale lacks.
///
/// # Safety
/// `s` a valid wide C string; `out` NULL or `n` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstombs(out: *mut u8, s: *const u16, n: usize) -> usize {
    if s.is_null() {
        global::invalid(EINVAL);
        return usize::MAX;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(s) };
    let limit = if out.is_null() { t.len() } else { t.len().min(n) };
    for (i, &c) in t[..limit].iter().enumerate() {
        let Some(b) = unit::c_locale_to_narrow(c) else {
            global::set_errno(EILSEQ);
            return usize::MAX;
        };
        if !out.is_null() {
            // SAFETY: i < n.
            unsafe { out.add(i).write(b) };
        }
    }
    if !out.is_null() && limit < n {
        // SAFETY: limit < n.
        unsafe { out.add(limit).write(0) };
    }
    limit
}

/// `mbrtowc` (no shift states in the "C" locale).
///
/// # Safety
/// As `mbtowc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbrtowc(out: *mut u16, s: *const u8, n: usize, _state: *mut u32) -> usize {
    if s.is_null() {
        return 0;
    }
    if n == 0 {
        return usize::MAX - 1; // (size_t)-2: incomplete
    }
    // SAFETY: the caller's arguments.
    unsafe { mbtowc(out, s, n) as usize }
}

/// `wcrtomb`.
///
/// # Safety
/// `out` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcrtomb(out: *mut u8, c: u16, _state: *mut u32) -> usize {
    if out.is_null() {
        return 1;
    }
    // SAFETY: the caller's buffer.
    match unsafe { wctomb(out, c) } {
        -1 => usize::MAX,
        n => n as usize,
    }
}
