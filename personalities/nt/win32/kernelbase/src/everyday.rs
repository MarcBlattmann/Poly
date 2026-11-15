//! Everyday kernel32 functions that start-up code of 32-bit programs (Borland's and Delphi's
//! runtimes, VCL) calls before anything else (ADR 0186, stage 3): the arguments and results are
//! Learn's for each function.

use crate::base::*;
use poly_kernelbase::error::*;
use poly_kernelbase::{locale, uilang};

/// `SetHandleCount` (obsolete; Learn: "For Win32-based applications, the number of open files is
/// limited only by available memory"): the return value is the number asked for, which the
/// C runtimes of the 1990s take as the size of their handle table.
#[unsafe(no_mangle)]
pub extern "system" fn SetHandleCount(number: u32) -> u32 {
    number
}

/// `GetThreadUILanguage`: the language of the thread's UI: Poly has one, English (United States).
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadUILanguage() -> u16 {
    locale::LCID_EN_US as u16
}

/// `SetThreadPreferredUILanguages(dwFlags, pwszLanguagesBuffer, pulNumLanguages)`: Poly keeps no
/// per-thread list (there is only one UI language: [`uilang`]), so the call succeeds for flags
/// Learn allows and counts the languages of the list that Poly has: `*pulNumLanguages` is how
/// many were set. A NULL list clears the thread's list (0 set).
///
/// # Safety
/// `languages` is NULL or a double-NUL-terminated list; `count` is NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetThreadPreferredUILanguages(flags: u32, languages: *const u16, count: *mut u32) -> i32 {
    let known = uilang::MUI_LANGUAGE_ID | uilang::MUI_LANGUAGE_NAME | 0x10 | 0x20 | 0x40 | 0x100;
    if flags & !known != 0 {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let mut set = 0u32;
    if !languages.is_null() {
        let mut at = 0usize;
        loop {
            // SAFETY: the caller's double-NUL-terminated list.
            let len = unsafe { (0..).take_while(|&i| languages.add(at + i).read() != 0).count() };
            if len == 0 {
                break;
            }
            // SAFETY: `len` units were just read.
            let name = unsafe { core::slice::from_raw_parts(languages.add(at), len) };
            let wanted = locale::NAME_EN_US.encode_utf16();
            if name.iter().copied().eq(wanted) || flags & uilang::MUI_LANGUAGE_ID != 0 {
                set += 1;
            }
            at += len + 1;
        }
    }
    if !count.is_null() {
        // SAFETY: the caller's out variable.
        unsafe { count.write_unaligned(set) };
    }
    TRUE
}

/// `MulDiv(nNumber, nNumerator, nDenominator)`: see [`poly_kernelbase::muldiv`].
#[unsafe(no_mangle)]
pub extern "system" fn MulDiv(number: i32, numerator: i32, denominator: i32) -> i32 {
    poly_kernelbase::muldiv::mul_div(number, numerator, denominator)
}
