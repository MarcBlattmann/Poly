//! The ANSI forms of the locale, string-classification and calendar functions (Learn's `winnls.h`
//! and `stringapiset.h` pages), and the calendar enumeration (ADR 0186, stage 4: the 32-bit
//! programs of the survey, Borland's runtime first, ask the locale for its names with the `A`
//! forms). The rules are `poly_kernelbase::locale`'s and `::nls`'s; the ANSI code page is 1252
//! (`poly_kernelbase::text`).

use crate::base::*;
use crate::locale::CompareStringW;
use alloc::vec::Vec;
use poly_kernelbase::error::*;
use poly_kernelbase::{locale, nls, text};

/// Copy `bytes` and a NUL to `buf` of `cch` bytes the way the locale functions answer: the
/// length with the NUL, or (with `cch` 0) only the size needed; too small a buffer is
/// `ERROR_INSUFFICIENT_BUFFER`.
unsafe fn answer_a(bytes: &[u8], buf: *mut u8, cch: i32) -> i32 {
    let need = bytes.len() as i32 + 1;
    if cch == 0 {
        return need;
    }
    if cch < need || buf.is_null() {
        return fail(ERROR_INSUFFICIENT_BUFFER, 0);
    }
    // SAFETY: the caller's buffer of `cch >= need` bytes.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len());
        buf.add(bytes.len()).write(0);
    }
    need
}

/// `GetLocaleInfoA`: `GetLocaleInfoW`'s answers in the ANSI code page; a number asked for with
/// `LOCALE_RETURN_NUMBER` is a `DWORD` (4 bytes).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetLocaleInfoA(lcid: u32, lctype: u32, buf: *mut u8, cch: i32) -> i32 {
    if !locale::is_valid_lcid(lcid, 0) {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let lctype = lctype & !locale::LOCALE_USE_CP_ACP;
    match locale::info(lctype & !locale::LOCALE_NOUSEROVERRIDE) {
        Some(locale::Info::Text(t)) => {
            if lctype & locale::LOCALE_RETURN_NUMBER != 0 {
                return fail(ERROR_INVALID_FLAGS, 0);
            }
            let wide: Vec<u16> = t.encode_utf16().collect();
            // SAFETY: the caller's buffer.
            unsafe { answer_a(&text::to_ansi(&wide), buf, cch) }
        }
        Some(locale::Info::Number(n)) => {
            if lctype & locale::LOCALE_RETURN_NUMBER == 0 {
                // SAFETY: the caller's buffer.
                return unsafe { answer_a(alloc::format!("{n}").as_bytes(), buf, cch) };
            }
            if cch == 0 {
                return 4;
            }
            if cch < 4 || buf.is_null() {
                return fail(ERROR_INSUFFICIENT_BUFFER, 0);
            }
            // SAFETY: the caller's buffer holds at least four bytes.
            unsafe { buf.cast::<u32>().write_unaligned(n) };
            4
        }
        None => fail(ERROR_INVALID_FLAGS, 0),
    }
}

/// The units of an ANSI input of `cch` bytes (-1: up to the NUL, which counts), or `None` for
/// an invalid length.
///
/// # Safety
/// `src` holds `cch` bytes, or a NUL-terminated string when `cch` is -1.
unsafe fn ansi_input(src: *const u8, cch: i32) -> Option<Vec<u16>> {
    if src.is_null() || cch == 0 || cch < -1 {
        return None;
    }
    let n = if cch == -1 {
        // SAFETY: a NUL-terminated string.
        unsafe { text::strlen(src) + 1 }
    } else {
        cch as usize
    };
    // SAFETY: the caller's contract.
    Some(text::from_ansi(unsafe { core::slice::from_raw_parts(src, n) }))
}

/// `GetStringTypeA` and `GetStringTypeExA`: the types of each byte of the ANSI code page
/// (`GetStringTypeW`'s rules on the character it stands for).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetStringTypeExA(
    _lcid: u32,
    info_type: u32,
    src: *const u8,
    cch: i32,
    out: *mut u16,
) -> i32 {
    // SAFETY: the caller's string.
    let Some(units) = (unsafe { ansi_input(src, cch) }) else { return fail(ERROR_INVALID_PARAMETER, 0) };
    if out.is_null() || !matches!(info_type, locale::CT_CTYPE1 | locale::CT_CTYPE2 | locale::CT_CTYPE3) {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    for (i, &u) in units.iter().enumerate() {
        let t = locale::string_type(info_type, u).unwrap_or(0);
        // SAFETY: `out` holds as many units as the input has bytes.
        unsafe { out.add(i).write(t) };
    }
    TRUE
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetStringTypeA(
    lcid: u32,
    info_type: u32,
    src: *const u8,
    cch: i32,
    out: *mut u16,
) -> i32 {
    // SAFETY: the caller's buffers.
    unsafe { GetStringTypeExA(lcid, info_type, src, cch, out) }
}

/// `GetStringTypeExW`: `GetStringTypeW` with a locale first.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetStringTypeExW(
    _lcid: u32,
    info_type: u32,
    src: *const u16,
    cch: i32,
    out: *mut u16,
) -> i32 {
    // SAFETY: the caller's buffers.
    unsafe { crate::locale::GetStringTypeW(info_type, src, cch, out) }
}

/// `LCMapStringA` for `LCMAP_LOWERCASE` and `LCMAP_UPPERCASE`: the same rules as `LCMapStringW`
/// on bytes of the ANSI code page; a letter whose mapping the code page lacks stays.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LCMapStringA(
    _lcid: u32,
    flags: u32,
    src: *const u8,
    cch: i32,
    out: *mut u8,
    cch_out: i32,
) -> i32 {
    if src.is_null() || cch == 0 || cch < -1 || cch_out < 0 {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let Some(kind) = locale::map_kind(flags) else { return fail(ERROR_INVALID_FLAGS, 0) };
    let n = if cch == -1 {
        // SAFETY: a NUL-terminated string; the result counts the NUL.
        unsafe { text::strlen(src) + 1 }
    } else {
        cch as usize
    };
    if cch_out == 0 {
        return n as i32;
    }
    if (cch_out as usize) < n || out.is_null() {
        return fail(ERROR_INSUFFICIENT_BUFFER, 0);
    }
    for i in 0..n {
        // SAFETY: `src` holds `n` bytes, `out` at least `n`.
        unsafe { out.add(i).write(locale::map_ansi_byte(kind, src.add(i).read())) };
    }
    n as i32
}

/// `CompareStringA`: `CompareStringW` on the characters of the bytes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CompareStringA(
    lcid: u32,
    flags: u32,
    a: *const u8,
    a_len: i32,
    b: *const u8,
    b_len: i32,
) -> i32 {
    if a.is_null() || b.is_null() || a_len < -1 || b_len < -1 || flags & !locale::COMPARE_FLAGS_KNOWN != 0 {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let units = |p: *const u8, n: i32| {
        let len = if n == -1 {
            // SAFETY: a NUL-terminated string.
            unsafe { text::strlen(p) }
        } else {
            n as usize
        };
        // SAFETY: the caller's string of that length.
        text::from_ansi(unsafe { core::slice::from_raw_parts(p, len) })
    };
    let (wa, wb) = (units(a, a_len), units(b, b_len));
    // SAFETY: the two vectors are the strings, with their lengths.
    unsafe { CompareStringW(lcid, flags, wa.as_ptr(), wa.len() as i32, wb.as_ptr(), wb.len() as i32) }
}

/// `GetCPInfoExW` and `GetCPInfoExA`: `CPINFOEX` of a code page; the flags must be 0.
unsafe fn cp_info_ex(code_page: u32, flags: u32, info: *mut u8, wide: bool) -> i32 {
    if info.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    if flags != 0 {
        return fail(ERROR_INVALID_FLAGS, FALSE);
    }
    match nls::code_page(code_page) {
        Ok(cp) => {
            let b = nls::cp_info_ex(cp, wide);
            // SAFETY: the caller's CPINFOEX.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), info, b.len()) };
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCPInfoExW(code_page: u32, flags: u32, info: *mut u8) -> i32 {
    // SAFETY: the caller's CPINFOEXW.
    unsafe { cp_info_ex(code_page, flags, info, true) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCPInfoExA(code_page: u32, flags: u32, info: *mut u8) -> i32 {
    // SAFETY: the caller's CPINFOEXA.
    unsafe { cp_info_ex(code_page, flags, info, false) }
}

/// What `EnumCalendarInfo*` calls for a calendar and type: the string of that type (the
/// calendar has one answer per type), or `None` for a calendar or type Poly lacks.
fn calendar_text(lcid: u32, calendar: u32, caltype: u32) -> Result<Vec<u16>, u32> {
    if !locale::is_valid_lcid(lcid, 0) || !locale::is_known_calendar(calendar, true) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if caltype & locale::CAL_RETURN_NUMBER != 0 {
        return Err(ERROR_INVALID_FLAGS);
    }
    match locale::calendar_info(caltype) {
        Some(locale::Info::Text(t)) => Ok(t.encode_utf16().collect()),
        Some(locale::Info::Number(n)) => Ok(alloc::format!("{n}").encode_utf16().collect()),
        None => Err(ERROR_INVALID_FLAGS),
    }
}

/// `EnumCalendarInfoW`: calls `proc` with the string of the calendar information (one per
/// type for Poly's one calendar) until it returns FALSE.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumCalendarInfoW(
    proc: Option<unsafe extern "system" fn(*const u16) -> i32>,
    lcid: u32,
    calendar: u32,
    caltype: u32,
) -> i32 {
    let Some(proc) = proc else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    match calendar_text(lcid, calendar, caltype) {
        Ok(mut s) => {
            s.push(0);
            // SAFETY: a NUL-terminated string that outlives the call.
            unsafe { proc(s.as_ptr()) };
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

/// `EnumCalendarInfoExW`: as `EnumCalendarInfoW`, the callback also gets the calendar.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumCalendarInfoExW(
    proc: Option<unsafe extern "system" fn(*const u16, u32) -> i32>,
    lcid: u32,
    calendar: u32,
    caltype: u32,
) -> i32 {
    let Some(proc) = proc else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    match calendar_text(lcid, calendar, caltype) {
        Ok(mut s) => {
            s.push(0);
            // SAFETY: a NUL-terminated string that outlives the call.
            unsafe { proc(s.as_ptr(), locale::CAL_GREGORIAN) };
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

/// `EnumCalendarInfoA`: the string in the ANSI code page.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumCalendarInfoA(
    proc: Option<unsafe extern "system" fn(*const u8) -> i32>,
    lcid: u32,
    calendar: u32,
    caltype: u32,
) -> i32 {
    let Some(proc) = proc else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    match calendar_text(lcid, calendar, caltype) {
        Ok(s) => {
            let mut a = text::to_ansi(&s);
            a.push(0);
            // SAFETY: a NUL-terminated string that outlives the call.
            unsafe { proc(a.as_ptr()) };
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

/// `GetCalendarInfoW`: the string, or (with `CAL_RETURN_NUMBER`) the number, of a calendar
/// information type of the Gregorian calendar.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCalendarInfoW(
    lcid: u32,
    calendar: u32,
    caltype: u32,
    buf: *mut u16,
    cch: i32,
    value: *mut u32,
) -> i32 {
    if !locale::is_valid_lcid(lcid, 0) || !locale::is_known_calendar(calendar, false) {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    match locale::calendar_info(caltype) {
        Some(locale::Info::Number(n)) if caltype & locale::CAL_RETURN_NUMBER != 0 => {
            if value.is_null() {
                return fail(ERROR_INVALID_PARAMETER, 0);
            }
            // SAFETY: the caller's DWORD.
            unsafe { value.write_unaligned(n) };
            2
        }
        Some(locale::Info::Text(_)) if caltype & locale::CAL_RETURN_NUMBER != 0 => fail(ERROR_INVALID_FLAGS, 0),
        Some(locale::Info::Text(t)) => {
            let w: Vec<u16> = t.encode_utf16().collect();
            let need = w.len() as i32 + 1;
            if cch == 0 {
                return need;
            }
            if cch < need || buf.is_null() {
                return fail(ERROR_INSUFFICIENT_BUFFER, 0);
            }
            // SAFETY: the caller's buffer of `cch >= need` units.
            unsafe { put_wide(buf, &w) };
            need
        }
        Some(locale::Info::Number(n)) => {
            let w: Vec<u16> = alloc::format!("{n}").encode_utf16().collect();
            let need = w.len() as i32 + 1;
            if cch == 0 {
                return need;
            }
            if cch < need || buf.is_null() {
                return fail(ERROR_INSUFFICIENT_BUFFER, 0);
            }
            // SAFETY: the caller's buffer of `cch >= need` units.
            unsafe { put_wide(buf, &w) };
            need
        }
        None => fail(ERROR_INVALID_FLAGS, 0),
    }
}
