//! `_findfirst`, `_findnext`, `_findclose` (Learn: each function's page;
//! ADR 0093): `FindFirstFileW` / `FindNextFileW` / `FindClose`, each entry
//! laid out as the caller's `_finddata*_t` (`poly_crt::find`). The search
//! handle is `FindFirstFileW`'s. Narrow names go through the ANSI code
//! page, as every narrow file name of the runtime does.
//!
//! The functions both DLLs export are `#[no_mangle]` here; the others
//! (`_findfirst32i64` …, `ucrtbase` only; `_findfirst`, `_findfirsti64` …,
//! `msvcrt` only, whose `time_t` is 64-bit) are [`first`] / [`next`] with
//! their layout, wrapped by each DLL.

use crate::global;
use crate::sys::{self, Handle};
use crate::text;
use alloc::vec::Vec;
use poly_crt::errno::{EINVAL, ENOENT};
use poly_crt::find::{self as f, FIND_DATA_SIZE, Layout};

/// A search's failure: `ENOENT` when nothing (more) matched (Learn:
/// "ENOENT: File specification that couldn't be matched"), else `EINVAL`
/// ("the operating system returned an unexpected error").
fn fail() {
    // SAFETY: no preconditions.
    let e = unsafe { sys::GetLastError() };
    let errno = match e {
        sys::ERROR_FILE_NOT_FOUND | sys::ERROR_PATH_NOT_FOUND | sys::ERROR_NO_MORE_FILES => ENOENT,
        _ => EINVAL,
    };
    global::set_errno(errno);
}

/// Write one entry as `layout`, with narrow (`wide` false) or wide names.
///
/// # Safety
/// `out` holds the structure.
unsafe fn give(raw: &[u8; FIND_DATA_SIZE], layout: Layout, wide: bool, out: *mut u8) {
    let found = f::from_find_data(raw);
    if wide {
        let size = layout.size::<u16>();
        // SAFETY: the caller's structure.
        let buf = unsafe { core::slice::from_raw_parts_mut(out, size) };
        f::store(&found, layout, buf);
    } else {
        let narrow = f::Found {
            attrib: found.attrib,
            create: found.create,
            access: found.access,
            write: found.write,
            size: found.size,
            name: text::wide_to_ansi(&found.name),
        };
        let size = layout.size::<u8>();
        // SAFETY: as above.
        let buf = unsafe { core::slice::from_raw_parts_mut(out, size) };
        f::store(&narrow, layout, buf);
    }
}

/// `_findfirst*`'s work for a wide pattern with its NUL: the search
/// handle, or −1.
///
/// # Safety
/// `out` is NULL or holds the structure.
pub unsafe fn first_wide(pattern: &[u16], layout: Layout, wide: bool, out: *mut u8) -> isize {
    if out.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    let mut raw = [0u8; FIND_DATA_SIZE];
    // SAFETY: a NUL-terminated pattern; `raw` holds a WIN32_FIND_DATAW.
    let h = unsafe { sys::FindFirstFileW(pattern.as_ptr(), raw.as_mut_ptr()) };
    if h == sys::INVALID_HANDLE_VALUE {
        fail();
        return -1;
    }
    // SAFETY: the caller's structure.
    unsafe { give(&raw, layout, wide, out) };
    h
}

/// `_findfirst*` with a narrow (`wide` false) or wide pattern.
///
/// # Safety
/// `pattern` is NULL or a C string of its width; `out` NULL or the
/// structure.
pub unsafe fn first(pattern: *const u8, layout: Layout, wide: bool, out: *mut u8) -> isize {
    if pattern.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's string.
    let p = unsafe { if wide { text::wide_path_w(pattern as *const u16) } else { text::wide_path(pattern) } };
    // SAFETY: as documented.
    unsafe { first_wide(&p, layout, wide, out) }
}

/// `_findnext*`: 0, or −1 (`ENOENT` when no more match).
///
/// # Safety
/// `out` is NULL or the structure.
pub unsafe fn next(handle: isize, layout: Layout, wide: bool, out: *mut u8) -> i32 {
    if out.is_null() || handle == -1 {
        global::invalid(EINVAL);
        return -1;
    }
    let mut raw = [0u8; FIND_DATA_SIZE];
    // SAFETY: a search handle from `_findfirst`; `raw` writable.
    if unsafe { sys::FindNextFileW(handle as Handle, raw.as_mut_ptr()) } == 0 {
        fail();
        return -1;
    }
    // SAFETY: the caller's structure.
    unsafe { give(&raw, layout, wide, out) };
    0
}

/// `_findclose`: 0, or −1 with `ENOENT` (Learn).
#[unsafe(no_mangle)]
pub extern "C" fn _findclose(handle: isize) -> i32 {
    // SAFETY: closing a search handle; the kernel checks it.
    if handle == -1 || unsafe { sys::FindClose(handle as Handle) } == 0 {
        global::set_errno(ENOENT);
        return -1;
    }
    0
}

/// The names (last components) a wide pattern with its NUL matches, in
/// the order the directory lists them (wildcard `argv`).
pub fn names_matching(pattern: &[u16]) -> Vec<Vec<u16>> {
    let mut out = Vec::new();
    let mut raw = [0u8; FIND_DATA_SIZE];
    // SAFETY: a NUL-terminated pattern; `raw` writable.
    let h = unsafe { sys::FindFirstFileW(pattern.as_ptr(), raw.as_mut_ptr()) };
    if h == sys::INVALID_HANDLE_VALUE {
        return out;
    }
    loop {
        out.push(f::from_find_data(&raw).name);
        // SAFETY: the search handle; `raw` writable.
        if unsafe { sys::FindNextFileW(h, raw.as_mut_ptr()) } == 0 {
            break;
        }
    }
    // SAFETY: the search handle.
    unsafe { sys::FindClose(h) };
    out
}

macro_rules! find_exports {
    ($($first:ident, $next:ident, $layout:ident, $wide:literal, $c:ty;)*) => {$(
        /// # Safety
        /// `pattern` is a C string; `out` the `_finddata` structure.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $first(pattern: *const $c, out: *mut u8) -> isize {
            // SAFETY: the caller's arguments.
            unsafe { first(pattern as *const u8, Layout::$layout, $wide, out) }
        }

        /// # Safety
        /// `out` is the `_finddata` structure.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $next(handle: isize, out: *mut u8) -> i32 {
            // SAFETY: the caller's arguments.
            unsafe { next(handle, Layout::$layout, $wide, out) }
        }
    )*};
}

// The functions both DLLs export (`_wfindnext32` is `ucrtbase`'s only).
find_exports! {
    _findfirst32, _findnext32, T32S32, false, u8;
    _findfirst64, _findnext64, T64S64, false, u8;
    _findfirst64i32, _findnext64i32, T64S32, false, u8;
    _wfindfirst64, _wfindnext64, T64S64, true, u16;
    _wfindfirst64i32, _wfindnext64i32, T64S32, true, u16;
}

/// `_wfindfirst32` (both DLLs); its `_wfindnext32` is `ucrtbase`'s only.
///
/// # Safety
/// As `_findfirst32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wfindfirst32(pattern: *const u16, out: *mut u8) -> isize {
    // SAFETY: the caller's arguments.
    unsafe { first(pattern as *const u8, Layout::T32S32, true, out) }
}
