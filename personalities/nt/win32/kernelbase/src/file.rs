//! Files and directories (Learn's `fileapi.h` and `handleapi.h` pages)
//! over `NtCreateFile` and friends (ADR 0067). The argument conversions are
//! `poly_kernelbase::file`'s and `::path`'s.

use crate::base::*;
use crate::nt::{self, Handle, IoStatusBlock};
use alloc::boxed::Box;
use alloc::vec::Vec;
use poly_kernelbase::error::*;
use poly_kernelbase::file::{self, *};
use poly_ntobj::NtStatus;

/// `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`.
const SHARE_ALL: u32 = 7;

/// `CloseHandle`.
#[unsafe(no_mangle)]
pub extern "system" fn CloseHandle(handle: Handle) -> i32 {
    // SAFETY: closing a handle value is memory-safe; the kernel checks it.
    let st = unsafe { nt::NtClose(handle) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `CreateFileW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateFileW(
    name: *const u16,
    access: u32,
    share: u32,
    security: *const u8,
    disposition: u32,
    flags: u32,
    template: Handle,
) -> Handle {
    let _ = template;
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, INVALID_HANDLE_VALUE) };
    // CONIN$, CONOUT$, CON: the console's buffers (ADR 0080).
    if let Some(h) = crate::console::create_file_console(name, access) {
        return h;
    }
    let c = match file::create(access, disposition, flags) {
        Ok(c) => c,
        Err(e) => return fail(e, INVALID_HANDLE_VALUE),
    };
    let nt_name = match nt_name(name) {
        Ok(n) => n,
        Err(e) => return fail(e, INVALID_HANDLE_VALUE),
    };
    let mut oa = nt_name.attributes();
    // SECURITY_ATTRIBUTES.bInheritHandle (minwinbase.h: nLength, then the
    // descriptor pointer at 8, the BOOL at 16).
    // SAFETY: the caller's structure when non-null.
    if !security.is_null() && unsafe { (security.add(16) as *const i32).read() } != 0 {
        oa.attributes |= nt::OBJ_INHERIT;
    }
    let mut h: Handle = 0;
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtCreateFile(
            &mut h,
            c.access,
            &oa,
            &mut iosb,
            core::ptr::null(),
            c.attributes,
            share,
            c.disposition,
            c.options,
            core::ptr::null(),
            0,
        )
    };
    if !st.is_success() {
        return fail(for_create_file(st), INVALID_HANDLE_VALUE);
    }
    set_last_error(if file::existed(disposition, iosb.information) { ERROR_ALREADY_EXISTS } else { ERROR_SUCCESS });
    h
}

/// `CreateFileA`: the name in the ANSI code page.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateFileA(
    name: *const u8,
    access: u32,
    share: u32,
    security: *const u8,
    disposition: u32,
    flags: u32,
    template: Handle,
) -> Handle {
    // SAFETY: the caller's string.
    let Some(w) = (unsafe { ansi_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, INVALID_HANDLE_VALUE) };
    let w = with_nul(&w);
    // SAFETY: a NUL-terminated name; the rest as given.
    unsafe { CreateFileW(w.as_ptr(), access, share, security, disposition, flags, template) }
}

/// `OVERLAPPED` (`minwinbase.h`): `Internal` (the status), `InternalHigh`
/// (the byte count), `Offset`, `OffsetHigh`, `hEvent`.
#[repr(C)]
pub struct Overlapped {
    pub internal: u64,
    pub internal_high: u64,
    pub offset: u32,
    pub offset_high: u32,
    pub event: Handle,
}

/// One `ReadFile` / `WriteFile`. Learn: with no `OVERLAPPED` the transfer
/// is at the file pointer; with one, at its offset, signalling its event;
/// on a synchronous handle the call waits either way. A synchronous read
/// at the end of the file succeeds with 0 bytes; an overlapped one fails
/// with `ERROR_HANDLE_EOF`. An asynchronous transfer that is still running
/// fails with `ERROR_IO_PENDING`.
unsafe fn transfer(handle: Handle, buf: *mut u8, len: u32, done: *mut u32, ov: *mut Overlapped, write: bool) -> i32 {
    if !done.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { done.write(0) };
    }
    let mut local = IoStatusBlock::default();
    let (iosb, offset, event): (*mut IoStatusBlock, i64, Handle) = if ov.is_null() {
        (&mut local, 0, 0)
    } else {
        // SAFETY: the caller's OVERLAPPED; its first two fields are the
        // IO_STATUS_BLOCK the kernel completes (minwinbase.h).
        unsafe {
            (*ov).internal = nt::STATUS_PENDING.0 as u64;
            (ov as *mut IoStatusBlock, file::overlapped_offset((*ov).offset, (*ov).offset_high), (*ov).event)
        }
    };
    let offset_ptr = if ov.is_null() { core::ptr::null() } else { &offset as *const i64 };
    // SAFETY: the caller's buffer of `len` bytes; valid pointers.
    let mut st = unsafe {
        if write {
            nt::NtWriteFile(
                handle,
                event,
                core::ptr::null(),
                core::ptr::null(),
                iosb,
                buf,
                len,
                offset_ptr,
                core::ptr::null(),
            )
        } else {
            nt::NtReadFile(
                handle,
                event,
                core::ptr::null(),
                core::ptr::null(),
                iosb,
                buf,
                len,
                offset_ptr,
                core::ptr::null(),
            )
        }
    };
    if st == nt::STATUS_PENDING {
        if !ov.is_null() {
            return fail(ERROR_IO_PENDING, FALSE);
        }
        // A synchronous call on a handle opened for overlapped I/O: wait.
        // SAFETY: a handle value.
        unsafe { nt::NtWaitForSingleObject(handle, 0, core::ptr::null()) };
        // SAFETY: the kernel completed the status block.
        st = NtStatus(unsafe { (*iosb).status } as u32);
    }
    // SAFETY: the status block is complete.
    let information = unsafe { (*iosb).information } as u32;
    if !done.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { done.write(information) };
    }
    if st == nt::STATUS_END_OF_FILE && !write {
        if ov.is_null() {
            return TRUE;
        }
        return fail(ERROR_HANDLE_EOF, FALSE);
    }
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `ReadFile`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ReadFile(
    handle: Handle,
    buf: *mut u8,
    len: u32,
    read: *mut u32,
    overlapped: *mut Overlapped,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { transfer(handle, buf, len, read, overlapped, false) }
}

/// `WriteFile`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WriteFile(
    handle: Handle,
    buf: *const u8,
    len: u32,
    written: *mut u32,
    overlapped: *mut Overlapped,
) -> i32 {
    // SAFETY: the caller's arguments; a write only reads `buf`.
    unsafe { transfer(handle, buf as *mut u8, len, written, overlapped, true) }
}

/// Query `class` of an open file into `out`.
fn query(handle: Handle, class: u32, out: &mut [u8]) -> Result<(), NtStatus> {
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers; `out` is the class's size.
    let st = unsafe { nt::NtQueryInformationFile(handle, &mut iosb, out.as_mut_ptr(), out.len() as u32, class) };
    if st.is_success() { Ok(()) } else { Err(st) }
}

fn set_info(handle: Handle, class: u32, info: &[u8]) -> Result<(), NtStatus> {
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtSetInformationFile(handle, &mut iosb, info.as_ptr(), info.len() as u32, class) };
    if st.is_success() { Ok(()) } else { Err(st) }
}

/// `FILE_STANDARD_INFORMATION.EndOfFile` (at 8, `wdm.h`).
fn file_size(handle: Handle) -> Result<u64, NtStatus> {
    let mut info = [0u8; 24];
    query(handle, FILE_STANDARD_INFORMATION, &mut info)?;
    Ok(u64::from_le_bytes(info[8..16].try_into().unwrap()))
}

fn position(handle: Handle) -> Result<u64, NtStatus> {
    let mut info = [0u8; 8];
    query(handle, FILE_POSITION_INFORMATION, &mut info)?;
    Ok(u64::from_le_bytes(info))
}

/// `GetFileSize`: Learn: the low DWORD, the high one in
/// `*lpFileSizeHigh`; a failure is `INVALID_FILE_SIZE` with an error, so a
/// real size whose low DWORD is `0xFFFFFFFF` clears the last error.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileSize(handle: Handle, high: *mut u32) -> u32 {
    match file_size(handle) {
        Ok(size) => {
            if !high.is_null() {
                // SAFETY: the caller's out-pointer.
                unsafe { high.write((size >> 32) as u32) };
            }
            if size as u32 == u32::MAX {
                set_last_error(ERROR_SUCCESS);
            }
            size as u32
        }
        Err(st) => fail_status(st, u32::MAX),
    }
}

/// `GetFileSizeEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileSizeEx(handle: Handle, size: *mut i64) -> i32 {
    match file_size(handle) {
        Ok(s) => {
            // SAFETY: the caller's out-pointer.
            unsafe { size.write(s as i64) };
            TRUE
        }
        Err(st) => fail_status(st, FALSE),
    }
}

/// Move the file pointer; the new position.
fn seek_to(handle: Handle, distance: i64, method: u32) -> Result<u64, u32> {
    let current = if method == FILE_CURRENT { position(handle).map_err(from_status)? } else { 0 };
    let size = if method == FILE_END { file_size(handle).map_err(from_status)? } else { 0 };
    let new = file::seek(method, distance, current, size)?;
    set_info(handle, FILE_POSITION_INFORMATION, &new.to_le_bytes()).map_err(from_status)?;
    Ok(new)
}

/// `SetFilePointer`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetFilePointer(handle: Handle, low: i32, high: *mut i32, method: u32) -> u32 {
    // SAFETY: the caller's in/out pointer.
    let h = if high.is_null() { None } else { Some(unsafe { high.read() }) };
    let distance = file::distance(low, h);
    if h.is_none() {
        // Learn: without lpDistanceToMoveHigh the result must fit 32 bits;
        // check before moving.
        let current = if method == FILE_CURRENT { position(handle).ok() } else { Some(0) };
        let size = if method == FILE_END { file_size(handle).ok() } else { Some(0) };
        if let (Some(c), Some(s)) = (current, size)
            && let Ok(n) = file::seek(method, distance, c, s)
            && !file::fits_low(n)
        {
            return fail(ERROR_INVALID_PARAMETER, u32::MAX);
        }
    }
    match seek_to(handle, distance, method) {
        Ok(n) => {
            if !high.is_null() {
                // SAFETY: as above.
                unsafe { high.write((n >> 32) as i32) };
            }
            set_last_error(ERROR_SUCCESS);
            n as u32
        }
        Err(e) => fail(e, u32::MAX),
    }
}

/// `SetFilePointerEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetFilePointerEx(handle: Handle, distance: i64, new: *mut i64, method: u32) -> i32 {
    match seek_to(handle, distance, method) {
        Ok(n) => {
            if !new.is_null() {
                // SAFETY: the caller's out-pointer.
                unsafe { new.write(n as i64) };
            }
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}

/// `SetEndOfFile`: the end of the file moves to the file pointer.
#[unsafe(no_mangle)]
pub extern "system" fn SetEndOfFile(handle: Handle) -> i32 {
    let r = position(handle).and_then(|p| set_info(handle, FILE_END_OF_FILE_INFORMATION, &p.to_le_bytes()));
    match r {
        Ok(()) => TRUE,
        Err(st) => fail_status(st, FALSE),
    }
}

/// `FlushFileBuffers`.
#[unsafe(no_mangle)]
pub extern "system" fn FlushFileBuffers(handle: Handle) -> i32 {
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtFlushBuffersFile(handle, &mut iosb) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `GetFileType`: `FILE_TYPE_DISK` (1) for a file on a disk volume,
/// `FILE_TYPE_CHAR` (2) for a character device such as the console
/// (`FileFsDeviceInformation`'s `DeviceType`: `FILE_DEVICE_DISK` 7 /
/// `FILE_DEVICE_CONSOLE` 0x50 / others, `wdm.h`), `FILE_TYPE_PIPE` (3) for a
/// named pipe (`FILE_DEVICE_NAMED_PIPE` 0x11), `FILE_TYPE_UNKNOWN` (0) and
/// an error otherwise.
#[unsafe(no_mangle)]
pub extern "system" fn GetFileType(handle: Handle) -> u32 {
    let mut iosb = IoStatusBlock::default();
    let mut info = [0u8; 8];
    // SAFETY: valid pointers; FileFsDeviceInformation (4) is 8 bytes.
    let st = unsafe { nt::NtQueryVolumeInformationFile(handle, &mut iosb, info.as_mut_ptr(), 8, 4) };
    if !st.is_success() {
        return fail_status(st, 0);
    }
    match u32::from_le_bytes(info[0..4].try_into().unwrap()) {
        0x07 | 0x08 | 0x14 | 0x24 => 1,
        0x11 => 3,
        _ => {
            set_last_error(ERROR_SUCCESS);
            2
        }
    }
}

/// Open `win32` for a delete or rename: `DELETE | SYNCHRONIZE`.
fn open_for(win32: &[u16], access: u32, options: u32) -> Result<Handle, u32> {
    let name = nt_name(win32)?;
    let oa = name.attributes();
    let mut h: Handle = 0;
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtOpenFile(&mut h, access | SYNCHRONIZE, &oa, &mut iosb, SHARE_ALL, options | FILE_SYNCHRONOUS_IO_NONALERT)
    };
    if st.is_success() { Ok(h) } else { Err(from_status(st)) }
}

/// Mark an open file for deletion (`FileDispositionInformation`) and close
/// it.
fn delete_open(h: Handle) -> i32 {
    let r = set_info(h, FILE_DISPOSITION_INFORMATION, &[1]);
    close(h);
    match r {
        Ok(()) => TRUE,
        Err(st) => fail_status(st, FALSE),
    }
}

/// `DeleteFileW`: a directory is `ERROR_ACCESS_DENIED` (Learn), as is a
/// read-only file.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DeleteFileW(name: *const u16) -> i32 {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    match open_for(name, DELETE, FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT) {
        Ok(h) => delete_open(h),
        Err(e) => fail(e, FALSE),
    }
}

/// `DeleteFileA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DeleteFileA(name: *const u8) -> i32 {
    // SAFETY: the caller's string.
    let Some(w) = (unsafe { ansi_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let w = with_nul(&w);
    // SAFETY: NUL-terminated.
    unsafe { DeleteFileW(w.as_ptr()) }
}

/// `RemoveDirectoryW`: a directory that is not empty is
/// `ERROR_DIR_NOT_EMPTY`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RemoveDirectoryW(name: *const u16) -> i32 {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    match open_for(name, DELETE, FILE_DIRECTORY_FILE | FILE_OPEN_FOR_BACKUP_INTENT | FILE_OPEN_REPARSE_POINT) {
        Ok(h) => delete_open(h),
        Err(e) => fail(e, FALSE),
    }
}

/// `RemoveDirectoryA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RemoveDirectoryA(name: *const u8) -> i32 {
    // SAFETY: the caller's string.
    let Some(w) = (unsafe { ansi_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let w = with_nul(&w);
    // SAFETY: NUL-terminated.
    unsafe { RemoveDirectoryW(w.as_ptr()) }
}

/// `CreateDirectoryW`: Learn: an existing name is `ERROR_ALREADY_EXISTS`, a
/// missing parent `ERROR_PATH_NOT_FOUND`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateDirectoryW(name: *const u16, security: *const u8) -> i32 {
    let _ = security;
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let nt_name = match nt_name(name) {
        Ok(n) => n,
        Err(e) => return fail(e, FALSE),
    };
    let oa = nt_name.attributes();
    let mut h: Handle = 0;
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtCreateFile(
            &mut h,
            FILE_LIST_DIRECTORY | SYNCHRONIZE,
            &oa,
            &mut iosb,
            core::ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            3,
            FILE_CREATE,
            FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_FOR_BACKUP_INTENT,
            core::ptr::null(),
            0,
        )
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    close(h);
    TRUE
}

/// `CreateDirectoryA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateDirectoryA(name: *const u8, security: *const u8) -> i32 {
    // SAFETY: the caller's string.
    let Some(w) = (unsafe { ansi_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let w = with_nul(&w);
    // SAFETY: NUL-terminated.
    unsafe { CreateDirectoryW(w.as_ptr(), security) }
}

/// `MoveFileExW`: a rename (`FileRenameInformation`) on the same volume.
/// `MOVEFILE_REPLACE_EXISTING` replaces a file (Learn: not a directory);
/// without it an existing target is `ERROR_ALREADY_EXISTS`. Moving to
/// another volume (`MOVEFILE_COPY_ALLOWED`) and `MOVEFILE_DELAY_UNTIL_REBOOT`
/// are not supported yet: `ERROR_NOT_SAME_DEVICE` / `ERROR_NOT_SUPPORTED`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32 {
    // SAFETY: the caller's strings.
    let (Some(from), to) = (unsafe { wide_arg(from) }, unsafe { wide_arg(to) }) else {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    };
    if flags & MOVEFILE_DELAY_UNTIL_REBOOT != 0 {
        return fail(ERROR_NOT_SUPPORTED, FALSE);
    }
    let Some(to) = to else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let target = match nt_path(to) {
        Ok(t) => t,
        Err(e) => return fail(e, FALSE),
    };
    let h = match open_for(from, DELETE, FILE_OPEN_FOR_BACKUP_INTENT | FILE_OPEN_REPARSE_POINT) {
        Ok(h) => h,
        Err(e) => return fail(e, FALSE),
    };
    let info = file::rename_information(flags & MOVEFILE_REPLACE_EXISTING != 0, &target);
    let r = set_info(h, FILE_RENAME_INFORMATION, &info);
    close(h);
    match r {
        Ok(()) => TRUE,
        Err(st) => fail_status(st, FALSE),
    }
}

/// `FILE_BASIC_INFORMATION.FileAttributes` (at 32, `wdm.h`) of a path.
pub(crate) fn attributes_of(win32: &[u16]) -> Result<u32, u32> {
    let name = nt_name(win32)?;
    let oa = name.attributes();
    let mut info = [0u8; 40];
    // SAFETY: valid pointers; FILE_BASIC_INFORMATION is 40 bytes.
    let st = unsafe { nt::NtQueryAttributesFile(&oa, info.as_mut_ptr()) };
    if st.is_success() { Ok(u32::from_le_bytes(info[32..36].try_into().unwrap())) } else { Err(from_status(st)) }
}

/// `GetFileAttributesW`: `INVALID_FILE_ATTRIBUTES` on failure.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileAttributesW(name: *const u16) -> u32 {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, INVALID_FILE_ATTRIBUTES) };
    attributes_of(name).unwrap_or_else(|e| fail(e, INVALID_FILE_ATTRIBUTES))
}

/// `GetFileAttributesA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileAttributesA(name: *const u8) -> u32 {
    // SAFETY: the caller's string.
    let Some(w) = (unsafe { ansi_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, INVALID_FILE_ATTRIBUTES) };
    attributes_of(&w).unwrap_or_else(|e| fail(e, INVALID_FILE_ATTRIBUTES))
}

/// `SetFileAttributesW`: `FILE_BASIC_INFORMATION` with zero times (left
/// unchanged) and the attributes (`FILE_ATTRIBUTE_NORMAL` when 0).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetFileAttributesW(name: *const u16, attributes: u32) -> i32 {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    let h = match open_for(name, FILE_WRITE_ATTRIBUTES, FILE_OPEN_FOR_BACKUP_INTENT) {
        Ok(h) => h,
        Err(e) => return fail(e, FALSE),
    };
    let mut info = [0u8; 40];
    let a = if attributes == 0 { FILE_ATTRIBUTE_NORMAL } else { attributes & FILE_ATTRIBUTE_VALID_SET_FLAGS };
    info[32..36].copy_from_slice(&a.to_le_bytes());
    let r = set_info(h, FILE_BASIC_INFORMATION, &info);
    close(h);
    match r {
        Ok(()) => TRUE,
        Err(st) => fail_status(st, FALSE),
    }
}

/// `FILE_WRITE_ATTRIBUTES` (`winnt.h`).
const FILE_WRITE_ATTRIBUTES: u32 = 0x100;

// ----- paths -----------------------------------------------------------------------------

/// `GetFullPathNameW`: the "size" convention; `*lpFilePart` points at the
/// last component in the caller's buffer (NULL if the path ends with a
/// separator).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFullPathNameW(
    name: *const u16,
    size: u32,
    buf: *mut u16,
    file_part: *mut *mut u16,
) -> u32 {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, 0) };
    let (full, part) = match full_path(name) {
        Ok(f) => f,
        Err(e) => return fail(e, 0),
    };
    // SAFETY: the caller's buffer.
    let n = unsafe { return_sized(&full, buf, size) };
    if n as usize == full.len() && !file_part.is_null() {
        // SAFETY: the caller's out-pointer; `part` is inside `buf`.
        unsafe { file_part.write(part.map_or(core::ptr::null_mut(), |i| buf.add(i))) };
    }
    n
}

/// `GetFullPathNameA`: converted through UTF-16; the sizes are the ANSI
/// string's (one byte per character in code page 1252).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFullPathNameA(
    name: *const u8,
    size: u32,
    buf: *mut u8,
    file_part: *mut *mut u8,
) -> u32 {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { ansi_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, 0) };
    let (full, part) = match full_path(&name) {
        Ok(f) => f,
        Err(e) => return fail(e, 0),
    };
    let a = poly_kernelbase::text::to_ansi(&full);
    match poly_kernelbase::text::sized(a.len(), size) {
        poly_kernelbase::text::Sized::Copy(n) => {
            // SAFETY: the caller's buffer holds more than `a.len()` bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(a.as_ptr(), buf, a.len());
                buf.add(a.len()).write(0);
                if !file_part.is_null() {
                    file_part.write(part.map_or(core::ptr::null_mut(), |i| buf.add(i)));
                }
            }
            n
        }
        poly_kernelbase::text::Sized::Needed(n) => n,
    }
}

/// `GetTempPathW`: `TMP`, `TEMP`, `USERPROFILE`, else the Windows
/// directory, with a trailing separator.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTempPathW(size: u32, buf: *mut u16) -> u32 {
    use crate::process::env_var;
    let w = poly_kernelbase::wide;
    let (tmp, temp, profile) = (env_var(&w("TMP")), env_var(&w("TEMP")), env_var(&w("USERPROFILE")));
    let windows = env_var(&w("SystemRoot")).unwrap_or_else(|| w("C:\\Windows"));
    let cur = current_dir();
    let none = |_: u16| None;
    let base = poly_kernelbase::path::Base { current: &cur, drive_dir: &none };
    match poly_kernelbase::path::temp_path([tmp.as_deref(), temp.as_deref(), profile.as_deref()], &windows, &base) {
        // SAFETY: the caller's buffer.
        Ok(p) => unsafe { return_sized(&p, buf, size) },
        Err(e) => fail(e, 0),
    }
}

// ----- finding files ------------------------------------------------------------------------

/// What a find handle holds: the open directory, the pattern, and the
/// entries of the last query not yet returned.
struct Find {
    dir: Handle,
    mask: Vec<u16>,
    buf: Vec<u8>,
    entries: Vec<usize>,
    next: usize,
}

/// A buffer for several `FILE_BOTH_DIR_INFORMATION` entries.
const FIND_BUFFER: usize = 4096;

impl Find {
    /// Query the next batch; `Err` with the status at the end.
    fn fill(&mut self, restart: bool) -> Result<(), NtStatus> {
        let mask = nt::UnicodeString::new(&self.mask);
        let mut iosb = IoStatusBlock::default();
        self.buf.resize(FIND_BUFFER, 0);
        // SAFETY: valid pointers; `buf` holds FIND_BUFFER bytes.
        let st = unsafe {
            nt::NtQueryDirectoryFile(
                self.dir,
                0,
                core::ptr::null(),
                core::ptr::null(),
                &mut iosb,
                self.buf.as_mut_ptr(),
                FIND_BUFFER as u32,
                FILE_BOTH_DIRECTORY_INFORMATION,
                0,
                &mask,
                restart as u8,
            )
        };
        if !st.is_success() {
            return Err(st);
        }
        let n = (iosb.information as usize).min(self.buf.len());
        self.buf.truncate(n);
        self.entries = file::dir_entries(&self.buf);
        self.next = 0;
        if self.entries.is_empty() { Err(nt::STATUS_NO_MORE_FILES) } else { Ok(()) }
    }

    /// The next entry as `WIN32_FIND_DATAW`.
    fn next(&mut self) -> Result<[u8; find_data::SIZE], NtStatus> {
        if self.next >= self.entries.len() {
            self.fill(false)?;
        }
        let at = self.entries[self.next];
        self.next += 1;
        file::to_find_data(&self.buf, at).ok_or(nt::STATUS_NO_MORE_FILES)
    }
}

/// `FindFirstFileW`: Learn: no match is `ERROR_FILE_NOT_FOUND`; a missing
/// directory `ERROR_PATH_NOT_FOUND`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindFirstFileW(pattern: *const u16, data: *mut u8) -> Handle {
    // SAFETY: the caller's string.
    let Some(pattern) = (unsafe { wide_arg(pattern) }) else {
        return fail(ERROR_INVALID_PARAMETER, INVALID_HANDLE_VALUE);
    };
    let cur = current_dir();
    let none = |_: u16| None;
    let base = poly_kernelbase::path::Base { current: &cur, drive_dir: &none };
    let (dir, mask) = match poly_kernelbase::path::split_find(pattern, &base) {
        Ok(x) => x,
        Err(e) => return fail(e, INVALID_HANDLE_VALUE),
    };
    let dir = match open_for(&dir, FILE_LIST_DIRECTORY, FILE_DIRECTORY_FILE | FILE_OPEN_FOR_BACKUP_INTENT) {
        Ok(h) => h,
        Err(e) => return fail(e, INVALID_HANDLE_VALUE),
    };
    let mut find = Box::new(Find { dir, mask, buf: Vec::new(), entries: Vec::new(), next: 0 });
    let first = find.fill(true).and_then(|()| find.next());
    match first {
        Ok(d) => {
            // SAFETY: the caller's WIN32_FIND_DATAW.
            unsafe { core::ptr::copy_nonoverlapping(d.as_ptr(), data, d.len()) };
            set_last_error(ERROR_SUCCESS);
            Box::into_raw(find) as Handle
        }
        Err(st) => {
            close(find.dir);
            let e = if st == nt::STATUS_NO_MORE_FILES || st == nt::STATUS_NO_SUCH_FILE {
                ERROR_FILE_NOT_FOUND
            } else {
                from_status(st)
            };
            fail(e, INVALID_HANDLE_VALUE)
        }
    }
}

/// A find handle back as the `Find` it is, if it plausibly is one.
fn find_of(handle: Handle) -> Option<&'static mut Find> {
    if handle == 0 || handle == INVALID_HANDLE_VALUE || handle & 0xF != 0 {
        return None;
    }
    // SAFETY: a find handle is the address of a boxed `Find` (Poly's own
    // representation, ADR 0079); a program that passes anything else gets
    // undefined behaviour as on Windows.
    Some(unsafe { &mut *(handle as *mut Find) })
}

/// `FindNextFileW`: `ERROR_NO_MORE_FILES` at the end.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindNextFileW(handle: Handle, data: *mut u8) -> i32 {
    let Some(find) = find_of(handle) else { return fail(ERROR_INVALID_HANDLE, FALSE) };
    match find.next() {
        Ok(d) => {
            // SAFETY: the caller's WIN32_FIND_DATAW.
            unsafe { core::ptr::copy_nonoverlapping(d.as_ptr(), data, d.len()) };
            TRUE
        }
        Err(st) if st == nt::STATUS_NO_MORE_FILES || st == nt::STATUS_NO_SUCH_FILE => fail(ERROR_NO_MORE_FILES, FALSE),
        Err(st) => fail_status(st, FALSE),
    }
}

/// `FindClose`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindClose(handle: Handle) -> i32 {
    if find_of(handle).is_none() {
        return fail(ERROR_INVALID_HANDLE, FALSE);
    }
    // SAFETY: a handle `FindFirstFileW` made from a box.
    let find = unsafe { Box::from_raw(handle as *mut Find) };
    close(find.dir);
    TRUE
}

// ----- information and times by handle (ADR 0079, second slice) --------------------------

/// `FileInternalInformation` (`wdm.h`): the `IndexNumber`.
const FILE_INTERNAL_INFORMATION: u32 = 6;
/// `FileFsVolumeInformation` (`wdm.h`): `VolumeSerialNumber` at 8.
const FILE_FS_VOLUME_INFORMATION: u32 = 1;

/// `GetFileInformationByHandle`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileInformationByHandle(handle: Handle, out: *mut u8) -> i32 {
    let (mut basic, mut standard, mut index) = ([0u8; 40], [0u8; 24], [0u8; 8]);
    let r = query(handle, FILE_BASIC_INFORMATION, &mut basic)
        .and_then(|()| query(handle, FILE_STANDARD_INFORMATION, &mut standard))
        .and_then(|()| query(handle, FILE_INTERNAL_INFORMATION, &mut index));
    if let Err(st) = r {
        return fail_status(st, FALSE);
    }
    // The volume serial number: the label may follow, so ask with room.
    let mut volume = [0u8; 128];
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtQueryVolumeInformationFile(handle, &mut iosb, volume.as_mut_ptr(), 128, FILE_FS_VOLUME_INFORMATION)
    };
    let serial =
        if st.is_success() || st.0 == 0x8000_0005 { u32::from_le_bytes(volume[8..12].try_into().unwrap()) } else { 0 };
    let info = file::by_handle_information(&basic, &standard, u64::from_le_bytes(index), serial);
    // SAFETY: the caller's BY_HANDLE_FILE_INFORMATION.
    unsafe { core::ptr::copy_nonoverlapping(info.as_ptr(), out, info.len()) };
    TRUE
}

/// Write a `FILETIME` (two `DWORD`s, possibly unaligned) if `p` is set.
///
/// # Safety
/// `p` is null or writable for 8 bytes.
unsafe fn put_time(p: *mut u8, t: u64) {
    if !p.is_null() {
        // SAFETY: the caller's contract.
        unsafe { p.cast::<u64>().write_unaligned(t) };
    }
}

/// `GetFileTime`: creation, last access, last write; each pointer may be
/// null.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileTime(handle: Handle, creation: *mut u8, access: *mut u8, write: *mut u8) -> i32 {
    let mut basic = [0u8; 40];
    if let Err(st) = query(handle, FILE_BASIC_INFORMATION, &mut basic) {
        return fail_status(st, FALSE);
    }
    let at = |o: usize| u64::from_le_bytes(basic[o..o + 8].try_into().unwrap());
    // SAFETY: the caller's out-pointers.
    unsafe {
        put_time(creation, at(0));
        put_time(access, at(8));
        put_time(write, at(16));
    }
    TRUE
}

/// `SetFileTime`: a null pointer leaves that time alone.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetFileTime(
    handle: Handle,
    creation: *const u8,
    access: *const u8,
    write: *const u8,
) -> i32 {
    // SAFETY: the caller's FILETIMEs, read unaligned.
    let get = |p: *const u8| (!p.is_null()).then(|| unsafe { p.cast::<u64>().read_unaligned() });
    let info = file::set_times(get(creation), get(access), get(write));
    match set_info(handle, FILE_BASIC_INFORMATION, &info) {
        Ok(()) => TRUE,
        Err(st) => fail_status(st, FALSE),
    }
}
