//! The `PathCch*` functions (`pathcch.h`, API set `api-ms-win-core-path-l1-1-0`), which Python
//! (`PathCchSkipRoot`) and LibreOffice (`PathCchCanonicalizeEx`) call at start-up. The rules are
//! `poly_kernelbase::winpath`'s, host-tested against the recorded results of
//! `specs/windows/kernelbase.dll/PathCch*.tests`; this file reads and writes the callers' memory.

use crate::base::*;
use poly_kernelbase::winpath as wp;

/// `PathCchSkipRoot(pszPath, ppszRootEnd)`: `S_OK` with the pointer behind the root, or
/// `E_INVALIDARG` when the path has none.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathCchSkipRoot(path: *const u16, root_end: *mut *const u16) -> u32 {
    // SAFETY: the caller's string.
    let (Some(p), false) = (unsafe { wide_arg(path) }, root_end.is_null()) else { return wp::E_INVALIDARG };
    match wp::pathcch_skip_root(p) {
        Some(at) => {
            // SAFETY: the caller's out-pointer; `at` is at most the string's length.
            unsafe { root_end.write(path.add(at)) };
            wp::S_OK
        }
        None => wp::E_INVALIDARG,
    }
}

/// Writes `result` into `out` (`cch` units, with the NUL), or an empty string for an error.
///
/// # Safety
/// `out` holds `cch` writable units.
unsafe fn deliver(out: *mut u16, cch: usize, result: Result<alloc::vec::Vec<u16>, u32>) -> u32 {
    match result {
        Ok(text) => {
            // SAFETY: `text.len() + 1 <= cch` (checked by `pathcch_fit`).
            unsafe { put_wide(out, &text) };
            wp::S_OK
        }
        Err(hr) => {
            if cch > 0 {
                // SAFETY: at least one unit.
                unsafe { out.write(0) };
            }
            hr
        }
    }
}

/// `PathCchCanonicalizeEx(pszPathOut, cchPathOut, pszPathIn, dwFlags)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathCchCanonicalizeEx(out: *mut u16, cch: usize, path: *const u16, flags: u32) -> u32 {
    // SAFETY: the caller's string.
    let (Some(p), false) = (unsafe { wide_arg(path) }, out.is_null()) else { return wp::E_INVALIDARG };
    let text = wp::pathcch_canonical_text(p, flags);
    // SAFETY: the caller's buffer of `cch` units.
    unsafe { deliver(out, cch, wp::pathcch_fit(text, cch, flags)) }
}

/// `PathCchCanonicalize(pszPathOut, cchPathOut, pszPathIn)`: the same with no flags.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathCchCanonicalize(out: *mut u16, cch: usize, path: *const u16) -> u32 {
    // SAFETY: as `PathCchCanonicalizeEx`.
    unsafe { PathCchCanonicalizeEx(out, cch, path, 0) }
}

/// `PathCchCombineEx(pszPathOut, cchPathOut, pszPathIn, pszMore, dwFlags)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathCchCombineEx(
    out: *mut u16,
    cch: usize,
    dir: *const u16,
    more: *const u16,
    flags: u32,
) -> u32 {
    if out.is_null() {
        return wp::E_INVALIDARG;
    }
    // SAFETY: the caller's strings; either may be NULL (empty).
    let (dir, more) = unsafe { (wide_arg(dir).unwrap_or(&[]), wide_arg(more).unwrap_or(&[])) };
    let Some(text) = wp::pathcch_combine_text(dir, more, flags) else {
        // SAFETY: the caller's buffer of `cch` units.
        return unsafe { deliver(out, cch, Err(wp::E_INVALIDARG)) };
    };
    // SAFETY: the caller's buffer of `cch` units.
    unsafe { deliver(out, cch, wp::pathcch_fit(text, cch, flags)) }
}

/// `PathCchCombine(pszPathOut, cchPathOut, pszPathIn, pszMore)`: the same with no flags.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathCchCombine(out: *mut u16, cch: usize, dir: *const u16, more: *const u16) -> u32 {
    // SAFETY: as `PathCchCombineEx`.
    unsafe { PathCchCombineEx(out, cch, dir, more, 0) }
}
