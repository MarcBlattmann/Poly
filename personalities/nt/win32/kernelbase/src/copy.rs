//! `CopyFileW` (`winbase.h`): copies a file's bytes, its named streams, its attributes and its last
//! write time (sheet `kernel32.dll/CopyFileW`). 15 of the 19 programs of the app survey import it,
//! and Chrome's setup copies its files with it.
//!
//! What the sheet fixes and how this follows it:
//! - the source is opened for reading with read and write sharing (a source another handle holds
//!   for writing with full sharing copies; one held without sharing is a sharing violation);
//! - the destination is opened for writing with read sharing (a destination another handle holds
//!   for reading with full sharing is replaced; one held without sharing is 32);
//! - the same file under another name (case, a hard link) is a sharing violation (32), found by
//!   comparing the two files' volume serial and index before the destination is touched;
//! - the copy gets the source's read-only, hidden and system bits and always the archive bit, and
//!   the source's last write time; its creation and last access times are its own.

use crate::base::*;
use crate::file::{
    CreateFileW, DeleteFileW, GetFileInformationByHandle, ReadFile, SetFileAttributesW, SetFileTime, WriteFile,
};
use crate::nt::Handle;
use alloc::vec::Vec;
use poly_kernelbase::error::*;

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
/// `FILE_READ_ATTRIBUTES` (`winnt.h`).
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_WRITE: u32 = 2;
const FILE_SHARE_DELETE: u32 = 4;
const CREATE_NEW: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
/// `FILE_FLAG_BACKUP_SEMANTICS` (`winbase.h`): the probe of the destination also opens a directory.
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
/// The attributes a copy takes from its source (`FILE_ATTRIBUTE_READONLY`, `..._HIDDEN`, `..._SYSTEM`).
const COPIED_ATTRIBUTES: u32 = 0x1 | 0x2 | 0x4;
/// `FILE_ATTRIBUTE_ARCHIVE`: every copy has it.
const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;

/// `BY_HANDLE_FILE_INFORMATION` (`fileapi.h`): 52 bytes.
const BY_HANDLE_SIZE: usize = 52;

/// What `GetFileInformationByHandle` says of an open file: its attributes, last write time and
/// identity (volume serial, file index).
struct Facts {
    attributes: u32,
    last_write: u64,
    identity: (u32, u32, u32),
}

fn facts(h: Handle) -> Option<Facts> {
    let mut b = [0u8; BY_HANDLE_SIZE];
    // SAFETY: a BY_HANDLE_FILE_INFORMATION-sized buffer.
    if unsafe { GetFileInformationByHandle(h, b.as_mut_ptr()) } == 0 {
        return None;
    }
    let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    Some(Facts {
        attributes: u(0),
        last_write: u64::from(u(20)) | (u64::from(u(24)) << 32),
        identity: (u(28), u(44), u(48)),
    })
}

/// Copy the bytes of the open `src` into the open `dst`.
fn copy_data(src: Handle, dst: Handle) -> Result<(), u32> {
    let mut buf = alloc::vec![0u8; 0x10000];
    loop {
        let mut got = 0u32;
        // SAFETY: valid handle and buffer.
        if unsafe { ReadFile(src, buf.as_mut_ptr(), buf.len() as u32, &mut got, core::ptr::null_mut()) } == 0 {
            return Err(last_error());
        }
        if got == 0 {
            return Ok(());
        }
        let mut put = 0u32;
        // SAFETY: `got` bytes were read into `buf`.
        if unsafe { WriteFile(dst, buf.as_ptr(), got, &mut put, core::ptr::null_mut()) } == 0 {
            return Err(last_error());
        }
        if put != got {
            return Err(ERROR_DISK_FULL);
        }
    }
}

/// Copy one named stream `from` -> `to` (NUL-terminated `file:stream` names); the destination
/// stream is made or replaced.
///
/// # Safety
/// `from` and `to` are NUL-terminated.
unsafe fn copy_stream(from: *const u16, to: *const u16) -> Result<(), u32> {
    // SAFETY: the caller's NUL-terminated names.
    let src = unsafe {
        CreateFileW(from, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, core::ptr::null(), OPEN_EXISTING, 0, 0)
    };
    if src == INVALID_HANDLE_VALUE {
        return Err(last_error());
    }
    // SAFETY: as above.
    let dst = unsafe {
        CreateFileW(to, GENERIC_WRITE, FILE_SHARE_READ, core::ptr::null(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0)
    };
    if dst == INVALID_HANDLE_VALUE {
        let e = last_error();
        close(src);
        return Err(e);
    }
    let r = copy_data(src, dst);
    close(src);
    close(dst);
    r
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

/// Whether `to` names an existing file that is the same file as `src` (another name of it).
///
/// # Safety
/// `to` is NUL-terminated.
unsafe fn same_file(src: &Facts, to: *const u16) -> bool {
    // SAFETY: the caller's NUL-terminated name.
    let probe = unsafe {
        CreateFileW(
            to,
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            core::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            0,
        )
    };
    if probe == INVALID_HANDLE_VALUE {
        return false;
    }
    let same = facts(probe).is_some_and(|d| d.identity == src.identity);
    close(probe);
    same
}

/// `CopyFileW(lpExistingFileName, lpNewFileName, bFailIfExists)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CopyFileW(from: *const u16, to: *const u16, fail_if_exists: i32) -> i32 {
    // A NULL source is an empty name (the sheet: 3); a NULL destination crashed the oracle's caller.
    if from.is_null() {
        return fail(ERROR_PATH_NOT_FOUND, FALSE);
    }
    if to.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's NUL-terminated name.
    let src = unsafe {
        CreateFileW(from, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, core::ptr::null(), OPEN_EXISTING, 0, 0)
    };
    if src == INVALID_HANDLE_VALUE {
        return FALSE;
    }
    let Some(source) = facts(src) else {
        let e = last_error();
        close(src);
        return fail(e, FALSE);
    };
    // The same file under another name: refused before the destination is touched.
    // SAFETY: the caller's NUL-terminated name.
    if fail_if_exists == 0 && unsafe { same_file(&source, to) } {
        close(src);
        return fail(ERROR_SHARING_VIOLATION, FALSE);
    }
    let disposition = if fail_if_exists != 0 { CREATE_NEW } else { CREATE_ALWAYS };
    // SAFETY: the caller's NUL-terminated name.
    let dst = unsafe {
        CreateFileW(to, GENERIC_WRITE, FILE_SHARE_READ, core::ptr::null(), disposition, FILE_ATTRIBUTE_NORMAL, 0)
    };
    if dst == INVALID_HANDLE_VALUE {
        let e = last_error();
        close(src);
        return fail(e, FALSE);
    }
    let mut result = copy_data(src, dst);
    close(src);
    // The named streams (a file system without them has none to copy).
    if result.is_ok() {
        // SAFETY: the caller's NUL-terminated name.
        let streams = crate::legacy::streams_of(unsafe { wide_arg(from) }.unwrap_or_default()).unwrap_or_default();
        let records: Vec<_> = streams.into_iter().collect();
        for suffix in poly_kernelbase::file::named_stream_suffixes(&records) {
            // SAFETY: NUL-terminated names.
            let (s, d) = unsafe { (with_suffix(from, &suffix), with_suffix(to, &suffix)) };
            // SAFETY: NUL-terminated names.
            if let Err(e) = unsafe { copy_stream(s.as_ptr(), d.as_ptr()) } {
                result = Err(e);
                break;
            }
        }
    }
    // The source's last write time, set last so that no write of the copy moves it.
    if result.is_ok() {
        let t = source.last_write.to_le_bytes();
        // SAFETY: a FILETIME; the other times are left alone.
        unsafe { SetFileTime(dst, core::ptr::null(), core::ptr::null(), t.as_ptr()) };
    }
    close(dst);
    if let Err(e) = result {
        // SAFETY: the destination this call wrote.
        unsafe { DeleteFileW(to) };
        return fail(e, FALSE);
    }
    // SAFETY: NUL-terminated name.
    unsafe { SetFileAttributesW(to, (source.attributes & COPIED_ATTRIBUTES) | FILE_ATTRIBUTE_ARCHIVE) };
    TRUE
}
