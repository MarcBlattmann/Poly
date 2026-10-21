//! National-language support basics (Learn's `stringapiset.h` and
//! `winnls.h` pages): the code pages, `MultiByteToWideChar`,
//! `WideCharToMultiByte`, `CompareStringOrdinal`. The conversions and
//! their rules are `poly_kernelbase::nls`'s (ADR 0079, second slice).

use crate::base::*;
use poly_kernelbase::error::*;
use poly_kernelbase::nls::{self, CodePage};

/// `GetACP`: Windows-1252.
#[unsafe(no_mangle)]
pub extern "system" fn GetACP() -> u32 {
    nls::ANSI_CODE_PAGE
}

/// `GetOEMCP`: 437.
#[unsafe(no_mangle)]
pub extern "system" fn GetOEMCP() -> u32 {
    nls::OEM_CODE_PAGE
}

/// `IsValidCodePage`.
#[unsafe(no_mangle)]
pub extern "system" fn IsValidCodePage(code_page: u32) -> i32 {
    nls::is_valid_code_page(code_page) as i32
}

/// `GetCPInfo`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCPInfo(code_page: u32, info: *mut u8) -> i32 {
    if info.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    match CodePage::of(code_page) {
        Ok(cp) => {
            let b = nls::cp_info(cp);
            // SAFETY: the caller's CPINFO.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), info, b.len()) };
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

/// The input of a conversion: `len` -1 is NUL-terminated, the NUL
/// included in what is converted (Learn); 0 or below -1 is
/// `ERROR_INVALID_PARAMETER`.
///
/// # Safety
/// `p` holds `len` elements, or a NUL-terminated string when `len` is -1.
unsafe fn input<'a, T: Copy + PartialEq + Default>(p: *const T, len: i32) -> Result<&'a [T], u32> {
    if p.is_null() || len == 0 || len < -1 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let n = if len == -1 {
        let mut n = 0;
        // SAFETY: the caller's NUL-terminated string.
        while unsafe { *p.add(n) } != T::default() {
            n += 1;
        }
        n + 1
    } else {
        len as usize
    };
    // SAFETY: the caller's contract.
    Ok(unsafe { core::slice::from_raw_parts(p, n) })
}

/// `MultiByteToWideChar`: the characters written, or (`cchWideChar` 0)
/// needed; 0 with the error on failure.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MultiByteToWideChar(
    code_page: u32,
    flags: u32,
    src: *const u8,
    src_len: i32,
    dst: *mut u16,
    dst_len: i32,
) -> i32 {
    // SAFETY: the caller's string.
    let r = unsafe { input(src, src_len) }.and_then(|s| nls::multi_byte_to_wide(code_page, flags, s));
    let out = match r {
        Ok(o) => o,
        Err(e) => return fail(e, 0),
    };
    match nls::fits(out.len(), dst_len) {
        Ok(true) => {
            // SAFETY: the caller's buffer holds `dst_len` >= `out.len()`.
            unsafe { core::ptr::copy_nonoverlapping(out.as_ptr(), dst, out.len()) };
            out.len() as i32
        }
        Ok(false) => out.len() as i32,
        Err(e) => fail(e, 0),
    }
}

/// `WideCharToMultiByte`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WideCharToMultiByte(
    code_page: u32,
    flags: u32,
    src: *const u16,
    src_len: i32,
    dst: *mut u8,
    dst_len: i32,
    default_char: *const u8,
    used_default: *mut i32,
) -> i32 {
    // SAFETY: the caller's string and optional default character.
    let r = unsafe { input(src, src_len) }.and_then(|s| {
        let d = if default_char.is_null() { None } else { Some(unsafe { *default_char }) };
        nls::wide_to_multi_byte(code_page, flags, s, d, !used_default.is_null())
    });
    let out = match r {
        Ok(o) => o,
        Err(e) => return fail(e, 0),
    };
    let n = out.bytes.len();
    match nls::fits(n, dst_len) {
        Ok(fits) => {
            if fits {
                // SAFETY: the caller's buffer holds `dst_len` >= `n` bytes.
                unsafe { core::ptr::copy_nonoverlapping(out.bytes.as_ptr(), dst, n) };
            }
            if !used_default.is_null() {
                // SAFETY: the caller's out-pointer.
                unsafe { used_default.write(out.used_default as i32) };
            }
            n as i32
        }
        Err(e) => fail(e, 0),
    }
}

/// A string argument of a comparison: `len` -1 is NUL-terminated (the NUL
/// not compared).
///
/// # Safety
/// As [`input`].
unsafe fn compared<'a>(p: *const u16, len: i32) -> Result<&'a [u16], u32> {
    if len == -1 {
        // SAFETY: the caller's NUL-terminated string.
        return unsafe { wide_arg(p) }.ok_or(ERROR_INVALID_PARAMETER);
    }
    if len < 0 || (p.is_null() && len > 0) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if len == 0 {
        return Ok(&[]);
    }
    // SAFETY: the caller's `len` units.
    Ok(unsafe { core::slice::from_raw_parts(p, len as usize) })
}

/// `CompareStringOrdinal`: `CSTR_*`, or 0 with `ERROR_INVALID_PARAMETER`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CompareStringOrdinal(
    a: *const u16,
    a_len: i32,
    b: *const u16,
    b_len: i32,
    ignore_case: i32,
) -> i32 {
    // SAFETY: the caller's strings.
    match unsafe { (compared(a, a_len), compared(b, b_len)) } {
        (Ok(a), Ok(b)) => nls::compare_ordinal(a, b, ignore_case != 0),
        (Err(e), _) | (_, Err(e)) => fail(e, 0),
    }
}
