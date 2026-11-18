//! Locale names and identifiers, the locale enumerations, the user's preferred UI languages
//! and date and time formatting (`winnls.h`; the spec sheets of `GetTimeFormatW`,
//! `GetDateFormatW`, `EnumSystemLocalesW`, `GetUserPreferredUILanguages`, `GetTimeFormatEx`,
//! `GetDateFormatEx`, `EnumSystemLocalesEx`, `LCIDToLocaleName`, `LocaleNameToLCID`). The rules
//! are `poly_kernelbase::nlsfmt`'s; this file moves the arguments in and the answers out.

use crate::base::*;
use alloc::vec::Vec;
use poly_kernelbase::error::*;
use poly_kernelbase::nlsfmt::{self, NameAnswer, SystemTime};

/// The longest locale name or picture read from a caller (units); a longer string is not a
/// locale name, and a picture is cut there.
const MAX_STRING: usize = 4096;

/// The units of the caller's NUL-terminated string, or `None` for NULL.
///
/// # Safety
/// `p` is NULL or a NUL-terminated UTF-16 string.
unsafe fn string<'a>(p: *const u16) -> Option<&'a [u16]> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the caller's NUL-terminated string.
    let n = unsafe { poly_kernelbase::text::wcslen(p, MAX_STRING) };
    // SAFETY: `n` units precede the NUL.
    Some(unsafe { core::slice::from_raw_parts(p, n) })
}

/// The caller's `SYSTEMTIME`, or the current local time for NULL.
///
/// # Safety
/// `p` is NULL or points to a `SYSTEMTIME`.
unsafe fn system_time(p: *const u16) -> SystemTime {
    let mut parts = [0u16; 8];
    if p.is_null() {
        // SAFETY: a SYSTEMTIME of our own.
        unsafe { crate::misc::GetLocalTime(parts.as_mut_ptr().cast()) };
    } else {
        for (i, v) in parts.iter_mut().enumerate() {
            // SAFETY: the caller's SYSTEMTIME of eight WORDs.
            *v = unsafe { p.add(i).read_unaligned() };
        }
    }
    SystemTime(parts)
}

/// Answer a formatting call: the length with the NUL, the size with `cch` 0, or 0 with the last
/// error; a buffer that is too small is not written.
///
/// # Safety
/// `out` holds `cch` units when it is not NULL.
unsafe fn answer(text: Result<Vec<u16>, u32>, out: *mut u16, cch: i32) -> i32 {
    let text = match text {
        Ok(t) => t,
        Err(e) => return fail(e, 0),
    };
    match nlsfmt::sized(text, cch, !out.is_null()) {
        Ok((n, Some(units))) => {
            // SAFETY: the caller's buffer holds `cch >= units.len()` units.
            unsafe { core::ptr::copy_nonoverlapping(units.as_ptr(), out, units.len()) };
            n
        }
        Ok((n, None)) => n,
        Err(e) => fail(e, 0),
    }
}

/// `GetTimeFormatW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTimeFormatW(
    lcid: u32,
    flags: u32,
    time: *const u16,
    picture: *const u16,
    out: *mut u16,
    cch: i32,
) -> i32 {
    if let Err(e) = nlsfmt::check_buffer(cch, !out.is_null()) {
        return fail(e, 0);
    }
    let text = nlsfmt::formats_of_lcid(lcid).and_then(|p| {
        // SAFETY: the caller's SYSTEMTIME (or NULL) and picture (or NULL).
        let (t, pic) = unsafe { (system_time(time), string(picture)) };
        nlsfmt::time_text(p, flags, &t, pic)
    });
    // SAFETY: the caller's buffer of `cch` units.
    unsafe { answer(text, out, cch) }
}

/// `GetTimeFormatEx`: the locale by name.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTimeFormatEx(
    name: *const u16,
    flags: u32,
    time: *const u16,
    picture: *const u16,
    out: *mut u16,
    cch: i32,
) -> i32 {
    if let Err(e) = nlsfmt::check_buffer(cch, !out.is_null()) {
        return fail(e, 0);
    }
    // SAFETY: the caller's locale name (or NULL).
    let text = nlsfmt::formats_of_name(unsafe { string(name) }).and_then(|p| {
        // SAFETY: the caller's SYSTEMTIME (or NULL) and picture (or NULL).
        let (t, pic) = unsafe { (system_time(time), string(picture)) };
        nlsfmt::time_text(p, flags, &t, pic)
    });
    // SAFETY: the caller's buffer of `cch` units.
    unsafe { answer(text, out, cch) }
}

/// `GetDateFormatW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetDateFormatW(
    lcid: u32,
    flags: u32,
    date: *const u16,
    picture: *const u16,
    out: *mut u16,
    cch: i32,
) -> i32 {
    if let Err(e) = nlsfmt::check_buffer(cch, !out.is_null()) {
        return fail(e, 0);
    }
    let text = nlsfmt::formats_of_lcid(lcid).and_then(|p| {
        // SAFETY: the caller's SYSTEMTIME (or NULL) and picture (or NULL).
        let (d, pic) = unsafe { (system_time(date), string(picture)) };
        nlsfmt::date_text(p, flags, &d, pic)
    });
    // SAFETY: the caller's buffer of `cch` units.
    unsafe { answer(text, out, cch) }
}

/// `GetDateFormatEx`: the locale by name; the calendar must be NULL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetDateFormatEx(
    name: *const u16,
    flags: u32,
    date: *const u16,
    picture: *const u16,
    out: *mut u16,
    cch: i32,
    calendar: *const u16,
) -> i32 {
    if !calendar.is_null() {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    if let Err(e) = nlsfmt::check_buffer(cch, !out.is_null()) {
        return fail(e, 0);
    }
    // SAFETY: the caller's locale name (or NULL).
    let text = nlsfmt::formats_of_name(unsafe { string(name) }).and_then(|p| {
        // SAFETY: the caller's SYSTEMTIME (or NULL) and picture (or NULL).
        let (d, pic) = unsafe { (system_time(date), string(picture)) };
        nlsfmt::date_text(p, flags, &d, pic)
    });
    // SAFETY: the caller's buffer of `cch` units.
    unsafe { answer(text, out, cch) }
}

/// `LCIDToLocaleName`: a buffer too small gets as much of the name as fits.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LCIDToLocaleName(lcid: u32, out: *mut u16, cch: i32, flags: u32) -> i32 {
    let (units, result) = match nlsfmt::lcid_to_name(lcid, cch, !out.is_null(), flags) {
        Ok(NameAnswer::Size(n)) => return n,
        Ok(NameAnswer::Name(units)) => {
            let n = units.len() as i32;
            (units, n)
        }
        Ok(NameAnswer::Truncated(units)) => (units, 0),
        Err(e) => return fail(e, 0),
    };
    // SAFETY: the caller's buffer holds `cch >= units.len()` units.
    unsafe { core::ptr::copy_nonoverlapping(units.as_ptr(), out, units.len()) };
    if result == 0 { fail(ERROR_INSUFFICIENT_BUFFER, 0) } else { result }
}

/// `LocaleNameToLCID`: 0 and the last error for a malformed name or a bad flag.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LocaleNameToLCID(name: *const u16, flags: u32) -> u32 {
    // SAFETY: the caller's locale name (or NULL).
    match nlsfmt::name_to_lcid(unsafe { string(name) }, flags) {
        Ok(lcid) => lcid,
        Err(e) => fail(e, 0),
    }
}

/// `LOCALE_ENUMPROCW` (`winnls.h`).
type LocaleEnumProcW = unsafe extern "system" fn(*mut u16) -> i32;
/// `LOCALE_ENUMPROCEX` (`winnls.h`).
type LocaleEnumProcEx = unsafe extern "system" fn(*mut u16, u32, isize) -> i32;

/// `EnumSystemLocalesW`: each identifier as eight hexadecimal digits, on the calling thread,
/// until the list ends or the callback returns FALSE.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumSystemLocalesW(callback: Option<LocaleEnumProcW>, flags: u32) -> i32 {
    let Some(callback) = callback else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let list = match nlsfmt::enum_lcids(flags) {
        Ok(l) => l,
        Err(e) => return fail(e, FALSE),
    };
    for mut s in list {
        s.push(0);
        // SAFETY: the caller's callback, with a NUL-terminated string that lives over the call.
        if unsafe { callback(s.as_mut_ptr()) } == 0 {
            break;
        }
    }
    TRUE
}

/// `EnumSystemLocalesEx`: each name with its kind and the caller's `lParam`; `lpReserved` must
/// be NULL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumSystemLocalesEx(
    callback: Option<LocaleEnumProcEx>,
    flags: u32,
    lparam: isize,
    reserved: *const u8,
) -> i32 {
    if !reserved.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let Some(callback) = callback else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let list = match nlsfmt::enum_names(flags) {
        Ok(l) => l,
        Err(e) => return fail(e, FALSE),
    };
    for (mut s, kind) in list {
        s.push(0);
        // SAFETY: the caller's callback, with a NUL-terminated string that lives over the call.
        if unsafe { callback(s.as_mut_ptr(), kind, lparam) } == 0 {
            break;
        }
    }
    TRUE
}

/// `GetUserPreferredUILanguages`: the user's one UI language as a double-NUL-terminated list.
/// With no buffer and a size of 0 only the count and the size; a buffer that is too small
/// gets as much of the list as fits, the size needed and `ERROR_INSUFFICIENT_BUFFER` (the count
/// is not written).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetUserPreferredUILanguages(
    flags: u32,
    count: *mut u32,
    buf: *mut u16,
    cch: *mut u32,
) -> i32 {
    if count.is_null() || cch.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let list = match nlsfmt::user_preferred_ui_languages(flags) {
        Ok(l) => l,
        Err(e) => return fail(e, FALSE),
    };
    let mut text: Vec<u16> = Vec::new();
    for l in &list {
        text.extend_from_slice(l);
        text.push(0);
    }
    text.push(0);
    let size = text.len() as u32;
    // SAFETY: the caller's size variable.
    let given = unsafe { cch.read_unaligned() };
    if buf.is_null() {
        if given != 0 {
            return fail(ERROR_INVALID_PARAMETER, FALSE);
        }
        // SAFETY: the caller's out variables.
        unsafe {
            count.write_unaligned(list.len() as u32);
            cch.write_unaligned(size);
        }
        return TRUE;
    }
    let fits = given >= size;
    let n = (given.min(size)) as usize;
    // SAFETY: the caller's buffer holds `given >= n` units; the out variables are the caller's.
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), buf, n);
        cch.write_unaligned(size);
        if fits {
            count.write_unaligned(list.len() as u32);
        }
    }
    if fits { TRUE } else { fail(ERROR_INSUFFICIENT_BUFFER, FALSE) }
}
