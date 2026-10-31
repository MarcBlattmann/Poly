//! Conversions (C11 7.22.1, 7.29.4.1; Learn: `strtol`, `strtod`, `atoi`,
//! `_itoa`, `mbstowcs`, `wcstombs`, `mbtowc`, `wctomb`): numbers on
//! [`poly_crt::convert`] and [`poly_crt::float`] (the decimal point is
//! `LC_NUMERIC`'s, ADR 0094); multibyte characters are `mbconv.rs`'s.

use crate::global;
use poly_crt::convert::{self, Kind};
use poly_crt::ctype;
use poly_crt::errno::{EINVAL, ERANGE};
use poly_crt::float::{self, Format};
use poly_crt::unit::{self, Unit};

/// # Safety
/// `s` is a valid C string; `end` NULL or writable.
pub(crate) unsafe fn strto<C: Unit>(s: *const C, end: *mut *mut C, base: i32, kind: Kind) -> u64 {
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
pub(crate) unsafe fn ato<C: Unit>(s: *const C, wide64: bool) -> i64 {
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
pub(crate) unsafe fn strtofloat<C: Unit>(s: *const C, end: *mut *mut C, fmt: Format, point: u8) -> u64 {
    if s.is_null() {
        global::invalid(EINVAL);
        return 0;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(s) };
    let lead = t.iter().take_while(|c| ctype::is_space_unit(c.to_u32(), C::WIDE)).count();
    let p = float::parse_point(&t[lead..], fmt, point);
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
    f64::from_bits(unsafe { strtofloat(s, end, float::DOUBLE, crate::locale::loc().point) })
}

/// # Safety
/// As `strtod`.
pub unsafe extern "C" fn strtof(s: *const u8, end: *mut *mut u8) -> f32 {
    // SAFETY: the caller's arguments.
    f32::from_bits(unsafe { strtofloat(s, end, float::SINGLE, crate::locale::loc().point) } as u32)
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
    f64::from_bits(unsafe { strtofloat(s, end, float::DOUBLE, crate::locale::loc().point) })
}

/// # Safety
/// As `strtod`.
pub unsafe extern "C" fn wcstof(s: *const u16, end: *mut *mut u16) -> f32 {
    // SAFETY: the caller's arguments.
    f32::from_bits(unsafe { strtofloat(s, end, float::SINGLE, crate::locale::loc().point) } as u32)
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
