//! Strings at the runtime's edges: C strings from raw pointers, and the
//! ANSI code page (`CP_ACP`) the runtime uses for narrow file names, the
//! narrow command line and environment (Learn, `fopen`: "the file name is
//! interpreted using the ANSI code page").

use crate::sys;
use alloc::vec::Vec;

/// A NUL-terminated narrow string.
///
/// # Safety
/// `p` is a valid NUL-terminated string living for `'a`.
pub unsafe fn cstr<'a>(p: *const u8) -> &'a [u8] {
    // SAFETY: the caller's contract.
    unsafe { poly_crt::unit::slice(p) }
}

/// A NUL-terminated wide string.
///
/// # Safety
/// As [`cstr`].
pub unsafe fn wstr<'a>(p: *const u16) -> &'a [u16] {
    // SAFETY: the caller's contract.
    unsafe { poly_crt::unit::slice(p) }
}

/// ANSI → UTF-16 (no NUL added).
pub fn ansi_to_wide(s: &[u8]) -> Vec<u16> {
    if s.is_empty() {
        return Vec::new();
    }
    let n = s.len().min(i32::MAX as usize) as i32;
    // SAFETY: a size query.
    let need = unsafe { sys::MultiByteToWideChar(sys::CP_ACP, 0, s.as_ptr(), n, core::ptr::null_mut(), 0) };
    let mut out = alloc::vec![0u16; need.max(0) as usize];
    // SAFETY: `out` holds `need` units.
    let got = unsafe { sys::MultiByteToWideChar(sys::CP_ACP, 0, s.as_ptr(), n, out.as_mut_ptr(), need) };
    out.truncate(got.max(0) as usize);
    out
}

/// UTF-16 → ANSI (no NUL added); unmappable characters become the code
/// page's default character.
pub fn wide_to_ansi(s: &[u16]) -> Vec<u8> {
    if s.is_empty() {
        return Vec::new();
    }
    let n = s.len().min(i32::MAX as usize) as i32;
    // SAFETY: a size query.
    let need = unsafe {
        sys::WideCharToMultiByte(
            sys::CP_ACP,
            0,
            s.as_ptr(),
            n,
            core::ptr::null_mut(),
            0,
            core::ptr::null(),
            core::ptr::null_mut(),
        )
    };
    let mut out = alloc::vec![0u8; need.max(0) as usize];
    // SAFETY: `out` holds `need` bytes.
    let got = unsafe {
        sys::WideCharToMultiByte(
            sys::CP_ACP,
            0,
            s.as_ptr(),
            n,
            out.as_mut_ptr(),
            need,
            core::ptr::null(),
            core::ptr::null_mut(),
        )
    };
    out.truncate(got.max(0) as usize);
    out
}

/// A wide path with its NUL from a narrow C string.
///
/// # Safety
/// As [`cstr`].
pub unsafe fn wide_path(p: *const u8) -> Vec<u16> {
    // SAFETY: the caller's contract.
    let mut w = ansi_to_wide(unsafe { cstr(p) });
    w.push(0);
    w
}

/// A wide path with its NUL from a wide C string.
///
/// # Safety
/// As [`cstr`].
pub unsafe fn wide_path_w(p: *const u16) -> Vec<u16> {
    // SAFETY: the caller's contract.
    let mut w = unsafe { wstr(p) }.to_vec();
    w.push(0);
    w
}
