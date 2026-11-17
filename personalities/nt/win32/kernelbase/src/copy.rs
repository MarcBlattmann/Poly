//! `CopyFileW` (`winbase.h`): copies a file's bytes, its named streams and its attributes. 15 of
//! the 19 programs of the app survey import it, and the spec tests of the resource entries copy
//! their own image with it. Learn: the destination is overwritten unless `bFailIfExists` is TRUE
//! (then `ERROR_FILE_EXISTS`); a source that cannot be opened or a destination that cannot be
//! created fails with the error of the open. Learn ("CopyFile"): "The security resource
//! properties ... and alternate data streams" of the source are copied too (sheet
//! `kernel32.dll/MoveFileExW`: a `Zone.Identifier` stream comes with the copy).

use crate::base::*;
use crate::file::{CreateFileW, DeleteFileW, GetFileAttributesW, ReadFile, SetFileAttributesW, WriteFile};
use alloc::vec::Vec;
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

/// Copy the bytes of `from` into `to` (both NUL-terminated), made with `disposition`; on a failure
/// after `to` was made, `to` is deleted. The error is the failing call's.
///
/// # Safety
/// `from` and `to` are NUL-terminated.
unsafe fn copy_bytes(from: *const u16, to: *const u16, disposition: u32) -> Result<(), u32> {
    // SAFETY: the caller's NUL-terminated names.
    let src = unsafe {
        CreateFileW(from, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, core::ptr::null(), OPEN_EXISTING, 0, 0)
    };
    if src == INVALID_HANDLE_VALUE {
        return Err(last_error());
    }
    // SAFETY: as above.
    let dst = unsafe { CreateFileW(to, GENERIC_WRITE, 0, core::ptr::null(), disposition, FILE_ATTRIBUTE_NORMAL, 0) };
    if dst == INVALID_HANDLE_VALUE {
        let e = last_error();
        close(src);
        return Err(e);
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
        return Err(error);
    }
    Ok(())
}

/// `name` (NUL-terminated) followed by `suffix`, NUL-terminated.
///
/// # Safety
/// `name` is NUL-terminated.
unsafe fn with_suffix(name: *const u16, suffix: &[u16]) -> Vec<u16> {
    // SAFETY: the caller's NUL-terminated name.
    let mut out: Vec<u16> = unsafe { wide_arg(name) }.unwrap_or_default().to_vec();
    out.extend_from_slice(suffix);
    out.push(0);
    out
}

/// `CopyFileW(lpExistingFileName, lpNewFileName, bFailIfExists)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CopyFileW(from: *const u16, to: *const u16, fail_if_exists: i32) -> i32 {
    if from.is_null() || to.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let disposition = if fail_if_exists != 0 { CREATE_NEW } else { CREATE_ALWAYS };
    // SAFETY: the caller's NUL-terminated names.
    if let Err(e) = unsafe { copy_bytes(from, to, disposition) } {
        return fail(e, FALSE);
    }
    // The named streams (a file system without them has none to copy).
    // SAFETY: the caller's NUL-terminated name.
    let streams = crate::legacy::streams_of(unsafe { wide_arg(from) }.unwrap_or_default()).unwrap_or_default();
    let records: Vec<_> = streams.into_iter().collect();
    for suffix in poly_kernelbase::file::named_stream_suffixes(&records) {
        // SAFETY: NUL-terminated names.
        let (src, dst) = unsafe { (with_suffix(from, &suffix), with_suffix(to, &suffix)) };
        // SAFETY: NUL-terminated names.
        if let Err(e) = unsafe { copy_bytes(src.as_ptr(), dst.as_ptr(), CREATE_ALWAYS) } {
            return fail(e, FALSE);
        }
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
