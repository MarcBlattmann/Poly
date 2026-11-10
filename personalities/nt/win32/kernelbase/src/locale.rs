//! The user's locale and string classification and mapping (Learn's
//! `winnls.h` and `stringapiset.h` pages), the first slice Chrome's
//! startup needed (ADR 0155). Poly has one locale, English (United
//! States), for the system, the user and the thread: `LCID` 0x0409, the
//! name "en-US". The rules are `poly_kernelbase::locale`'s.

use crate::base::*;
use poly_kernelbase::error::*;
use poly_kernelbase::locale;

/// `GetUserDefaultLCID`, `GetSystemDefaultLCID`.
#[unsafe(no_mangle)]
pub extern "system" fn GetUserDefaultLCID() -> u32 {
    locale::LCID_EN_US
}

#[unsafe(no_mangle)]
pub extern "system" fn GetSystemDefaultLCID() -> u32 {
    locale::LCID_EN_US
}

/// `GetUserDefaultLangID`, `GetSystemDefaultLangID`: the language of the
/// locale (the low word).
#[unsafe(no_mangle)]
pub extern "system" fn GetUserDefaultLangID() -> u16 {
    locale::LCID_EN_US as u16
}

#[unsafe(no_mangle)]
pub extern "system" fn GetSystemDefaultLangID() -> u16 {
    locale::LCID_EN_US as u16
}

/// Copy `text` and a NUL to `buf` of `cch` units the way the locale
/// functions answer: the length with the NUL, or (with `cch` 0) only the
/// size needed; a buffer too small is `ERROR_INSUFFICIENT_BUFFER`.
unsafe fn answer(text: &[u16], buf: *mut u16, cch: i32) -> i32 {
    let need = text.len() as i32 + 1;
    if cch == 0 {
        return need;
    }
    if cch < need || buf.is_null() {
        return fail(ERROR_INSUFFICIENT_BUFFER, 0);
    }
    // SAFETY: the caller's buffer of `cch >= need` units.
    unsafe { put_wide(buf, text) };
    need
}

/// `GetUserDefaultLocaleName`, `GetSystemDefaultLocaleName`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetUserDefaultLocaleName(buf: *mut u16, cch: i32) -> i32 {
    let name: alloc::vec::Vec<u16> = locale::NAME_EN_US.encode_utf16().collect();
    // SAFETY: the caller's buffer.
    unsafe { answer(&name, buf, cch) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSystemDefaultLocaleName(buf: *mut u16, cch: i32) -> i32 {
    // SAFETY: as above.
    unsafe { GetUserDefaultLocaleName(buf, cch) }
}

/// `GetThreadPreferredUILanguages`: the list `poly_kernelbase::uilang` builds (Poly has one
/// UI language), as a double-NUL-terminated list; with no buffer and a size of 0 only the
/// count and the size needed.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetThreadPreferredUILanguages(
    flags: u32,
    count: *mut u32,
    buf: *mut u16,
    cch: *mut u32,
) -> i32 {
    use poly_kernelbase::uilang;
    if count.is_null() || cch.is_null() {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let list = match uilang::thread_preferred(flags) {
        Ok(l) => l,
        Err(e) => return fail(e, 0),
    };
    // SAFETY: the caller's size variable.
    let a = uilang::answer(&list, !buf.is_null(), unsafe { cch.read_unaligned() });
    // SAFETY: the caller's out variables; the buffer holds `*cch >= a.size` characters when
    // there is text to copy.
    unsafe {
        if a.error != Some(ERROR_INVALID_PARAMETER) {
            count.write_unaligned(a.count);
            cch.write_unaligned(a.size);
        }
        if let Some(text) = &a.text {
            core::ptr::copy_nonoverlapping(text.as_ptr(), buf, text.len());
        }
    }
    match a.error {
        Some(e) => fail(e, 0),
        None => 1,
    }
}

/// `IsValidLocale`.
#[unsafe(no_mangle)]
pub extern "system" fn IsValidLocale(lcid: u32, flags: u32) -> i32 {
    i32::from(locale::is_valid_lcid(lcid, flags))
}

/// `GetLocaleInfoW`: the answers `poly_kernelbase::locale::info` has.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetLocaleInfoW(lcid: u32, lctype: u32, buf: *mut u16, cch: i32) -> i32 {
    if !locale::is_valid_lcid(lcid, 0) {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    match locale::info(lctype & !locale::LOCALE_NOUSEROVERRIDE) {
        Some(locale::Info::Text(t)) => {
            if lctype & locale::LOCALE_RETURN_NUMBER != 0 {
                return fail(ERROR_INVALID_FLAGS, 0);
            }
            let text: alloc::vec::Vec<u16> = t.encode_utf16().collect();
            // SAFETY: the caller's buffer.
            unsafe { answer(&text, buf, cch) }
        }
        Some(locale::Info::Number(n)) => {
            if lctype & locale::LOCALE_RETURN_NUMBER == 0 {
                let text: alloc::vec::Vec<u16> = alloc::format!("{n}").encode_utf16().collect();
                // SAFETY: the caller's buffer.
                return unsafe { answer(&text, buf, cch) };
            }
            // The number is returned as a DWORD in the buffer: two units.
            if cch == 0 {
                return 2;
            }
            if cch < 2 || buf.is_null() {
                return fail(ERROR_INSUFFICIENT_BUFFER, 0);
            }
            // SAFETY: the caller's buffer holds at least two units.
            unsafe { buf.cast::<u32>().write_unaligned(n) };
            2
        }
        None => fail(ERROR_INVALID_FLAGS, 0),
    }
}

/// `GetStringTypeW`: `CT_CTYPE1` classification of each unit (Learn:
/// "GetStringTypeW function", "Character Type 1 Flags"); the other
/// information types are `ERROR_INVALID_PARAMETER` for now.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetStringTypeW(info_type: u32, src: *const u16, cch: i32, out: *mut u16) -> i32 {
    if src.is_null() || out.is_null() || cch == 0 || cch < -1 || info_type != locale::CT_CTYPE1 {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let n = if cch == -1 {
        // SAFETY: a NUL-terminated string; the result counts the NUL.
        unsafe { poly_kernelbase::text::wcslen(src, 0x7FFF_FFFF) + 1 }
    } else {
        cch as usize
    };
    for i in 0..n {
        // SAFETY: `src` and `out` hold `n` units.
        unsafe { out.add(i).write(locale::ctype1(src.add(i).read())) };
    }
    TRUE
}

/// `LCMapStringW` for `LCMAP_LOWERCASE` and `LCMAP_UPPERCASE` (with or
/// without `LCMAP_LINGUISTIC_CASING`); the other mappings are
/// `ERROR_INVALID_FLAGS`. The length is in units, or -1 up to the NUL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LCMapStringW(
    _lcid: u32,
    flags: u32,
    src: *const u16,
    cch: i32,
    out: *mut u16,
    cch_out: i32,
) -> i32 {
    if src.is_null() || cch == 0 || cch < -1 || cch_out < 0 {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let Some(kind) = locale::map_kind(flags) else { return fail(ERROR_INVALID_FLAGS, 0) };
    let n = if cch == -1 {
        // SAFETY: a NUL-terminated string; the result counts the NUL.
        unsafe { poly_kernelbase::text::wcslen(src, 0x7FFF_FFFF) + 1 }
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
        // SAFETY: `src` holds `n` units, `out` at least `n`.
        unsafe { out.add(i).write(locale::map_unit(kind, src.add(i).read())) };
    }
    n as i32
}

/// `LCMapStringEx`: the locale name instead of the LCID; the version and
/// reserved arguments must be NULL/0 (Learn).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LCMapStringEx(
    _name: *const u16,
    flags: u32,
    src: *const u16,
    cch: i32,
    out: *mut u16,
    cch_out: i32,
    _version: *mut u8,
    _reserved: *mut u8,
    _sort_handle: isize,
) -> i32 {
    // SAFETY: the caller's buffers.
    unsafe { LCMapStringW(locale::LCID_EN_US, flags, src, cch, out, cch_out) }
}

/// `CompareStringW` and `CompareStringEx`: `CSTR_*` with the flags
/// `NORM_IGNORECASE` honoured; the comparison is by code unit after case
/// folding, not the linguistic sort of the locale (to be confirmed against
/// Windows by the spec agent).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CompareStringW(
    _lcid: u32,
    flags: u32,
    a: *const u16,
    a_len: i32,
    b: *const u16,
    b_len: i32,
) -> i32 {
    if a.is_null() || b.is_null() || a_len < -1 || b_len < -1 || flags & !locale::COMPARE_FLAGS_KNOWN != 0 {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let len = |p: *const u16, n: i32| {
        if n == -1 {
            // SAFETY: a NUL-terminated string.
            unsafe { poly_kernelbase::text::wcslen(p, 0x7FFF_FFFF) }
        } else {
            n as usize
        }
    };
    // SAFETY: the caller's strings of the lengths computed.
    let (a, b) =
        unsafe { (core::slice::from_raw_parts(a, len(a, a_len)), core::slice::from_raw_parts(b, len(b, b_len))) };
    poly_kernelbase::nls::compare_ordinal(a, b, flags & locale::NORM_IGNORECASE != 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CompareStringEx(
    _name: *const u16,
    flags: u32,
    a: *const u16,
    a_len: i32,
    b: *const u16,
    b_len: i32,
    _version: *mut u8,
    _reserved: *mut u8,
    _param: isize,
) -> i32 {
    // SAFETY: the caller's strings.
    unsafe { CompareStringW(locale::LCID_EN_US, flags, a, a_len, b, b_len) }
}
