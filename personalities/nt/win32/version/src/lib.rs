//! Our `version.dll` (ADR 0155): `GetFileVersionInfoSizeW`,
//! `GetFileVersionInfoW` (and their `Ex` and ANSI-name forms) and
//! `VerQueryValueW`, the functions Chrome's `chrome_elf.dll` and browser
//! process call to read the version of their own files. Learn's pages for
//! each function give the behaviour; the file is read by
//! `poly_kernelbase::verinfo` (the headers and the resource section of the
//! PE file, not the whole file) and the block it returns is the
//! `VS_VERSIONINFO` resource as stored.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::vec::Vec;
use poly_kernelbase::error::*;
use poly_kernelbase::verinfo;
use poly_win32_rt as _;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetProcessHeap() -> *mut u8;
    fn HeapAlloc(heap: *mut u8, flags: u32, bytes: usize) -> *mut u8;
    fn HeapFree(heap: *mut u8, flags: u32, p: *mut u8) -> i32;
    fn SetLastError(error: u32);
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const u8,
        disposition: u32,
        flags: u32,
        template: isize,
    ) -> isize;
    fn ReadFile(h: isize, buf: *mut u8, len: u32, read: *mut u32, overlapped: *mut u8) -> i32;
    fn SetFilePointerEx(h: isize, distance: i64, new: *mut i64, method: u32) -> i32;
    fn CloseHandle(h: isize) -> i32;
    fn GetLastError() -> u32;
    fn GetSystemDirectoryW(buf: *mut u16, size: u32) -> u32;
}

/// The process heap (16-byte aligned) for `alloc`.
struct ProcessHeap;

// SAFETY: HeapAlloc returns 16-byte aligned blocks; stricter alignments
// are refused.
unsafe impl core::alloc::GlobalAlloc for ProcessHeap {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        // SAFETY: the process heap.
        unsafe { HeapAlloc(GetProcessHeap(), 0, layout.size().max(1)) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _: core::alloc::Layout) {
        // SAFETY: a block from `alloc`.
        unsafe { HeapFree(GetProcessHeap(), 0, ptr) };
    }
}

#[global_allocator]
static ALLOC: ProcessHeap = ProcessHeap;

const GENERIC_READ: u32 = 0x8000_0000;
const SHARE_ALL: u32 = 7;
const OPEN_EXISTING: u32 = 3;
const INVALID_HANDLE_VALUE: isize = -1;
const FILE_BEGIN: u32 = 0;

fn set_error(e: u32) {
    // SAFETY: a plain call.
    unsafe { SetLastError(e) };
}

/// The `RT_VERSION` resource of the file named by the NUL-terminated
/// `name`; a name with no directory is also looked for in the system
/// directory (Learn: the functions find a file the way `LoadLibrary`
/// would).
unsafe fn version_of(name: *const u16) -> Result<Vec<u8>, u32> {
    if name.is_null() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: the caller's string.
    let name = unsafe { poly_kernelbase::text::wide_at(name) };
    if name.is_empty() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let mut path: Vec<u16> = name.to_vec();
    path.push(0);
    // SAFETY: a NUL-terminated name.
    let mut h = unsafe { CreateFileW(path.as_ptr(), GENERIC_READ, SHARE_ALL, core::ptr::null(), OPEN_EXISTING, 0, 0) };
    let bare = !name.iter().any(|&c| c == u16::from(b'\\') || c == u16::from(b'/') || c == u16::from(b':'));
    // SAFETY: plain calls.
    if h == INVALID_HANDLE_VALUE && bare && unsafe { GetLastError() } == ERROR_FILE_NOT_FOUND {
        let mut dir = [0u16; 260];
        // SAFETY: a buffer of 260 units.
        let n = unsafe { GetSystemDirectoryW(dir.as_mut_ptr(), 260) } as usize;
        if n > 0 && n < 255 {
            let mut p: Vec<u16> = dir[..n].to_vec();
            p.push(u16::from(b'\\'));
            p.extend_from_slice(name);
            p.push(0);
            // SAFETY: a NUL-terminated name.
            h = unsafe { CreateFileW(p.as_ptr(), GENERIC_READ, SHARE_ALL, core::ptr::null(), OPEN_EXISTING, 0, 0) };
        }
    }
    if h == INVALID_HANDLE_VALUE {
        // SAFETY: a plain call.
        return Err(unsafe { GetLastError() });
    }
    let mut read = |at: u64, buf: &mut [u8]| -> Result<usize, u32> {
        let mut n = 0u32;
        // SAFETY: valid pointers; `buf` is writable for its length.
        unsafe {
            if SetFilePointerEx(h, at as i64, core::ptr::null_mut(), FILE_BEGIN) == 0 {
                return Err(GetLastError());
            }
            if ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut n, core::ptr::null_mut()) == 0 {
                return Err(GetLastError());
            }
        }
        Ok(n as usize)
    };
    let r = verinfo::read_version_resource(&mut read);
    // SAFETY: the handle opened above.
    unsafe { CloseHandle(h) };
    r
}

/// Set `*handle` to 0 (Learn: `lpdwHandle` is set to zero; it is ignored
/// on input).
unsafe fn zero_handle(handle: *mut u32) {
    if !handle.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { handle.write_unaligned(0) };
    }
}

/// `GetFileVersionInfoSizeW`: the size in bytes of the file's version
/// information, 0 with the error on failure.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileVersionInfoSizeW(name: *const u16, handle: *mut u32) -> u32 {
    // SAFETY: the caller's arguments.
    unsafe { zero_handle(handle) };
    // SAFETY: as above.
    match unsafe { version_of(name) } {
        Ok(v) => v.len() as u32,
        Err(e) => {
            set_error(e);
            0
        }
    }
}

/// `GetFileVersionInfoSizeExW`: `FILE_VER_GET_LOCALISED` (1), `..._NEUTRAL`
/// (2) and `..._PREFETCHED` (4) choose among the language-specific files
/// Windows keeps; Poly's files are neutral, so every flag reads the file
/// itself.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileVersionInfoSizeExW(flags: u32, name: *const u16, handle: *mut u32) -> u32 {
    if flags & !7 != 0 {
        set_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's arguments.
    unsafe { GetFileVersionInfoSizeW(name, handle) }
}

/// `GetFileVersionInfoW`: copy the version information into `data` (`len`
/// bytes; `ERROR_INSUFFICIENT_BUFFER` if that is less than the size
/// `GetFileVersionInfoSizeW` gave).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileVersionInfoW(name: *const u16, _handle: u32, len: u32, data: *mut u8) -> i32 {
    if data.is_null() {
        set_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's string.
    match unsafe { version_of(name) } {
        Ok(v) if (len as usize) >= v.len() => {
            // SAFETY: the caller's buffer of `len >= v.len()` bytes.
            unsafe { core::ptr::copy_nonoverlapping(v.as_ptr(), data, v.len()) };
            1
        }
        Ok(_) => {
            set_error(ERROR_INSUFFICIENT_BUFFER);
            0
        }
        Err(e) => {
            set_error(e);
            0
        }
    }
}

/// `GetFileVersionInfoExW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileVersionInfoExW(
    flags: u32,
    name: *const u16,
    handle: u32,
    len: u32,
    data: *mut u8,
) -> i32 {
    if flags & !7 != 0 {
        set_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's arguments.
    unsafe { GetFileVersionInfoW(name, handle, len, data) }
}

/// A NUL-terminated ANSI name as UTF-16.
unsafe fn wide_name(name: *const u8) -> Option<Vec<u16>> {
    if name.is_null() {
        return None;
    }
    // SAFETY: the caller's NUL-terminated string.
    let bytes = unsafe { core::slice::from_raw_parts(name, poly_kernelbase::text::strlen(name)) };
    let mut w = poly_kernelbase::text::from_ansi(bytes);
    w.push(0);
    Some(w)
}

/// `GetFileVersionInfoSizeA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileVersionInfoSizeA(name: *const u8, handle: *mut u32) -> u32 {
    // SAFETY: the caller's string.
    match unsafe { wide_name(name) } {
        // SAFETY: NUL-terminated.
        Some(w) => unsafe { GetFileVersionInfoSizeW(w.as_ptr(), handle) },
        None => {
            set_error(ERROR_INVALID_PARAMETER);
            0
        }
    }
}

/// `GetFileVersionInfoA`: the block is the same as `GetFileVersionInfoW`'s.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileVersionInfoA(name: *const u8, handle: u32, len: u32, data: *mut u8) -> i32 {
    // SAFETY: the caller's string.
    match unsafe { wide_name(name) } {
        // SAFETY: NUL-terminated.
        Some(w) => unsafe { GetFileVersionInfoW(w.as_ptr(), handle, len, data) },
        None => {
            set_error(ERROR_INVALID_PARAMETER);
            0
        }
    }
}

/// `VerQueryValueW`: a pointer into `block` and the length of the value
/// (see `poly_kernelbase::verinfo::query_value`). `block` is the buffer
/// `GetFileVersionInfoW` filled; its length is in its first word.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VerQueryValueW(
    block: *const u8,
    sub_block: *const u16,
    buffer: *mut *mut u8,
    len: *mut u32,
) -> i32 {
    if block.is_null() || sub_block.is_null() || buffer.is_null() {
        set_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: a version block starts with its own length (`wLength`).
    let total = usize::from(unsafe { (block as *const u16).read_unaligned() });
    // SAFETY: the block is `total` bytes; the string is the caller's.
    let (bytes, sub) =
        unsafe { (core::slice::from_raw_parts(block, total), poly_kernelbase::text::wide_at(sub_block)) };
    match verinfo::query_value(bytes, sub) {
        Ok((at, n)) => {
            // SAFETY: the caller's out-pointers; `at` is inside the block.
            unsafe {
                buffer.write_unaligned(block.add(at) as *mut u8);
                if !len.is_null() {
                    len.write_unaligned(n);
                }
            }
            1
        }
        Err(e) => {
            // SAFETY: the caller's out-pointers.
            unsafe {
                buffer.write_unaligned(core::ptr::null_mut());
                if !len.is_null() {
                    len.write_unaligned(0);
                }
            }
            set_error(e);
            0
        }
    }
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn version_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
