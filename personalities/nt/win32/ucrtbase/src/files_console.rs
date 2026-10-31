//! What only the UCRT exports of ADR 0093's slice: the `_finddata32i64_t`
//! searches and `_wfindnext32`, the console functions' `_nolock` forms,
//! `_cgets_s`, `_cgetws`, `_cgetws_s`, and the `__conio_common_vc*printf`
//! entry points `conio.h`'s inline `_cprintf` calls (options and locale as
//! `__stdio_common_vfprintf`'s).

use poly_crt::find::Layout;
use poly_win32_crt::{conio, find};

/// # Safety
/// `pattern` is a C string; `out` a `_finddata32i64_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _findfirst32i64(pattern: *const u8, out: *mut u8) -> isize {
    // SAFETY: the caller's arguments.
    unsafe { find::first(pattern, Layout::T32S64, false, out) }
}

/// # Safety
/// `out` is a `_finddata32i64_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _findnext32i64(handle: isize, out: *mut u8) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { find::next(handle, Layout::T32S64, false, out) }
}

/// # Safety
/// `pattern` is a wide C string; `out` a `_wfinddata32i64_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wfindfirst32i64(pattern: *const u16, out: *mut u8) -> isize {
    // SAFETY: the caller's arguments.
    unsafe { find::first(pattern as *const u8, Layout::T32S64, true, out) }
}

/// # Safety
/// `out` is a `_wfinddata32i64_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wfindnext32i64(handle: isize, out: *mut u8) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { find::next(handle, Layout::T32S64, true, out) }
}

/// # Safety
/// `out` is a `_wfinddata32_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wfindnext32(handle: isize, out: *mut u8) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { find::next(handle, Layout::T32S32, true, out) }
}

#[unsafe(no_mangle)]
pub extern "C" fn _getch_nolock() -> i32 {
    conio::getch_nolock(false, false) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn _getche_nolock() -> i32 {
    conio::getch_nolock(false, true) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn _getwch_nolock() -> u16 {
    conio::getch_nolock(true, false) as u16
}

#[unsafe(no_mangle)]
pub extern "C" fn _getwche_nolock() -> u16 {
    conio::getch_nolock(true, true) as u16
}

#[unsafe(no_mangle)]
pub extern "C" fn _putch_nolock(c: i32) -> i32 {
    conio::putch_nolock(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn _putwch_nolock(c: u16) -> u16 {
    conio::putwch_nolock(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn _ungetch_nolock(c: i32) -> i32 {
    conio::ungetch_nolock(c as u32, false) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn _ungetwch_nolock(c: u16) -> u16 {
    conio::ungetch_nolock(u32::from(c), true) as u16
}

/// # Safety
/// `buffer` holds `count` bytes; `read` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _cgets_s(buffer: *mut u8, count: usize, read: *mut usize) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { conio::cgets_s(buffer, count, read) }
}

/// # Safety
/// `buffer` holds `count` units; `read` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _cgetws_s(buffer: *mut u16, count: usize, read: *mut usize) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { conio::cgets_s(buffer, count, read) }
}

/// # Safety
/// `buffer` holds `buffer[0] + 3` units.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _cgetws(buffer: *mut u16) -> *mut u16 {
    // SAFETY: the caller's buffer.
    unsafe { conio::cgetws(buffer) }
}

/// # Safety
/// `format` is a C string and `args` its `va_list`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __conio_common_vcprintf(
    options: u64,
    format: *const u8,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { conio::vcprintf_va(format, args, options) }
}

/// # Safety
/// As `__conio_common_vcprintf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __conio_common_vcprintf_s(
    options: u64,
    format: *const u8,
    locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { __conio_common_vcprintf(options, format, locale, args) }
}

/// # Safety
/// As `__conio_common_vcprintf`, with a wide format.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __conio_common_vcwprintf(
    options: u64,
    format: *const u16,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { conio::vcprintf_va(format, args, options) }
}

/// # Safety
/// As `__conio_common_vcwprintf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __conio_common_vcwprintf_s(
    options: u64,
    format: *const u16,
    locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { __conio_common_vcwprintf(options, format, locale, args) }
}
