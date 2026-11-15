//! `CopyFileW` (`winbase.h`): copies a file's bytes and attributes. 15 of the 19 programs of the
//! app survey import it, and the spec tests of the resource entries copy their own image with it.
//! Learn: the destination is overwritten unless `bFailIfExists` is TRUE (then `ERROR_FILE_EXISTS`);
//! a source that cannot be opened or a destination that cannot be created fails with the error of
//! the open.

use crate::base::*;
use crate::file::{CreateFileW, DeleteFileW, GetFileAttributesW, ReadFile, SetFileAttributesW, WriteFile};
use poly_kernelbase::error::*;

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_WRITE: u32 = 2;
const CREATE_NEW: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
/// The attributes a copy takes from its source (`FILE_ATTRIBUTE_READONLY`, `..._HIDDEN`, `..._SYSTEM`,
/// `..._ARCHIVE`).
const COPIED_ATTRIBUTES: u32 = 0x1 | 0x2 | 0x4 | 0x20;

/// `CopyFileW(lpExistingFileName, lpNewFileName, bFailIfExists)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CopyFileW(from: *const u16, to: *const u16, fail_if_exists: i32) -> i32 {
    if from.is_null() || to.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's NUL-terminated names.
    let src = unsafe {
        CreateFileW(from, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, core::ptr::null(), OPEN_EXISTING, 0, 0)
    };
    if src == INVALID_HANDLE_VALUE {
        return FALSE;
    }
    let disposition = if fail_if_exists != 0 { CREATE_NEW } else { CREATE_ALWAYS };
    // SAFETY: as above.
    let dst = unsafe { CreateFileW(to, GENERIC_WRITE, 0, core::ptr::null(), disposition, FILE_ATTRIBUTE_NORMAL, 0) };
    if dst == INVALID_HANDLE_VALUE {
        let e = last_error();
        close(src);
        return fail(e, FALSE);
    }
    let mut buf = alloc::vec![0u8; 0x10000];
    let mut ok = true;
    loop {
        let mut got = 0u32;
        // SAFETY: valid handle and buffer.
        if unsafe { ReadFile(src, buf.as_mut_ptr(), buf.len() as u32, &mut got, core::ptr::null_mut()) } == 0 {
            ok = false;
            break;
        }
        if got == 0 {
            break;
        }
        let mut put = 0u32;
        // SAFETY: `got` bytes were read into `buf`.
        if unsafe { WriteFile(dst, buf.as_ptr(), got, &mut put, core::ptr::null_mut()) } == 0 || put != got {
            ok = false;
            break;
        }
    }
    let error = last_error();
    close(src);
    close(dst);
    if !ok {
        // SAFETY: the destination this call made.
        unsafe { DeleteFileW(to) };
        return fail(error, FALSE);
    }
    // SAFETY: NUL-terminated names.
    unsafe {
        let attributes = GetFileAttributesW(from);
        if attributes != INVALID_FILE_ATTRIBUTES {
            SetFileAttributesW(to, attributes & COPIED_ATTRIBUTES);
        }
    }
    TRUE
}

/// `INVALID_FILE_ATTRIBUTES` (`fileapi.h`).
const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;
