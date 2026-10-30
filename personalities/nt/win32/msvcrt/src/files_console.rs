//! What only `msvcrt.dll` exports of ADR 0093's slice: `_findfirst` /
//! `_findnext` and their `i64` and wide forms, whose `time_t` is 64-bit
//! here (`_finddata64i32_t`, and `__finddata64_t` for `i64`, as
//! `msvcrt`'s `time` is, ADR 0084), and the classic console `printf`s
//! (`MSVCRT_OPTIONS`; the variadic ones through the shared runtime's
//! thunks; the `_l` forms ignore the locale, which is always "C").

use poly_crt::find::Layout;
use poly_crt::printf::MSVCRT_OPTIONS;
use poly_win32_crt::{conio, find};

macro_rules! finds {
    ($($first:ident, $next:ident, $layout:ident, $wide:literal, $c:ty;)*) => {$(
        /// # Safety
        /// `pattern` is a C string; `out` the `_finddata` structure.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $first(pattern: *const $c, out: *mut u8) -> isize {
            // SAFETY: the caller's arguments.
            unsafe { find::first(pattern as *const u8, Layout::$layout, $wide, out) }
        }

        /// # Safety
        /// `out` is the `_finddata` structure.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $next(handle: isize, out: *mut u8) -> i32 {
            // SAFETY: the caller's arguments.
            unsafe { find::next(handle, Layout::$layout, $wide, out) }
        }
    )*};
}

finds! {
    _findfirst, _findnext, T64S32, false, u8;
    _findfirsti64, _findnexti64, T64S64, false, u8;
    _wfindfirst, _wfindnext, T64S32, true, u16;
    _wfindfirsti64, _wfindnexti64, T64S64, true, u16;
}

/// The `va_list` console `printf`s (the thunks call these too).
pub mod va {
    use super::*;

    /// # Safety
    /// `f` is a C string and `va` its arguments.
    pub unsafe extern "C" fn cprintf(f: *const u8, va: *const u64) -> i32 {
        // SAFETY: as documented.
        unsafe { conio::vcprintf_va(f, va, MSVCRT_OPTIONS) }
    }

    /// # Safety
    /// As `cprintf`.
    pub unsafe extern "C" fn cprintf_l(f: *const u8, _locale: *mut u8, va: *const u64) -> i32 {
        // SAFETY: as documented.
        unsafe { conio::vcprintf_va(f, va, MSVCRT_OPTIONS) }
    }

    /// # Safety
    /// As `cprintf`, with a wide format.
    pub unsafe extern "C" fn cwprintf(f: *const u16, va: *const u64) -> i32 {
        // SAFETY: as documented.
        unsafe { conio::vcprintf_va(f, va, MSVCRT_OPTIONS) }
    }

    /// # Safety
    /// As `cwprintf`.
    pub unsafe extern "C" fn cwprintf_l(f: *const u16, _locale: *mut u8, va: *const u64) -> i32 {
        // SAFETY: as documented.
        unsafe { conio::vcprintf_va(f, va, MSVCRT_OPTIONS) }
    }
}

poly_win32_crt::va_thunk!("_cprintf", 1, va::cprintf);
poly_win32_crt::va_thunk!("_cprintf_l", 2, va::cprintf_l);
poly_win32_crt::va_thunk!("_cwprintf", 1, va::cwprintf);
poly_win32_crt::va_thunk!("_cwprintf_l", 2, va::cwprintf_l);

/// # Safety
/// As `_cprintf`, with a `va_list`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _vcprintf(f: *const u8, va: *const u64) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { va::cprintf(f, va) }
}

/// # Safety
/// As `_vcprintf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _vcprintf_l(f: *const u8, locale: *mut u8, va: *const u64) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { va::cprintf_l(f, locale, va) }
}

/// # Safety
/// As `_vcprintf`, with a wide format.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _vcwprintf(f: *const u16, va: *const u64) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { va::cwprintf(f, va) }
}

/// # Safety
/// As `_vcwprintf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _vcwprintf_l(f: *const u16, locale: *mut u8, va: *const u64) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { va::cwprintf_l(f, locale, va) }
}
