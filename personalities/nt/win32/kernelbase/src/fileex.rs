//! Files, second part (Chrome batch 2, ADR 0155): Learn's `fileapi.h`,
//! `winbase.h` and `minwinbase.h` pages for `FindFirstFileExW`,
//! `GetFileAttributesExW`, `GetFileInformationByHandleEx`,
//! `SetFileInformationByHandle`, `LockFileEx`, `UnlockFileEx`,
//! `ReplaceFileW`, `GetFinalPathNameByHandleW`, `GetLongPathNameW`,
//! `GetSystemDirectoryW` and `GetWindowsDirectoryW`. The rules and the
//! structure conversions are `poly_kernelbase::fileex`'s.

use crate::base::*;
use crate::file::{find_first, open_for, query, set_info};
use crate::nt::{self, Handle, IoStatusBlock, ObjectAttributes};
use alloc::vec::Vec;
use poly_kernelbase::error::*;
use poly_kernelbase::file::*;
use poly_kernelbase::fileex::{self, QueryPlan, ReplaceStep, SetPlan};
use poly_ntobj::NtStatus;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryFullAttributesFile(attributes: *const ObjectAttributes, info: *mut u8) -> NtStatus;
    fn NtLockFile(
        handle: Handle,
        event: Handle,
        apc: *const u8,
        apc_context: *const u8,
        iosb: *mut IoStatusBlock,
        offset: *const i64,
        length: *const i64,
        key: u32,
        fail_immediately: u8,
        exclusive: u8,
    ) -> NtStatus;
    fn NtUnlockFile(
        handle: Handle,
        iosb: *mut IoStatusBlock,
        offset: *const i64,
        length: *const i64,
        key: u32,
    ) -> NtStatus;
}

// ----- FindFirstFileExW ---------------------------------------------------------------------------

/// `FindFirstFileExW`: `FindFirstFileW` with the level and options of
/// `poly_kernelbase::fileex::find_ex_args`. `lpSearchFilter` is only checked
/// (it must be NULL for `FindExSearchNameMatch`; `FindExSearchLimitToDirectories`
/// is a hint Poly ignores).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindFirstFileExW(
    pattern: *const u16,
    info_level: u32,
    data: *mut u8,
    search_op: u32,
    filter: *const u8,
    flags: u32,
) -> Handle {
    match fileex::find_ex_args(info_level, search_op, filter.is_null(), flags) {
        // SAFETY: the caller's arguments.
        Ok(args) => unsafe { find_first(pattern, data, args.basic) },
        Err(e) => fail(e, INVALID_HANDLE_VALUE),
    }
}

// ----- GetFileAttributesExW -----------------------------------------------------------------------

/// `GetFileAttributesExW`: `WIN32_FILE_ATTRIBUTE_DATA` of a name, from
/// `NtQueryFullAttributesFile` (Learn: the only level is
/// `GetFileExInfoStandard`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileAttributesExW(name: *const u16, level: u32, out: *mut u8) -> i32 {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    if level != fileex::GET_FILE_EX_INFO_STANDARD || out.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let nt_name = match nt_name(name) {
        Ok(n) => n,
        Err(e) => return fail(e, FALSE),
    };
    let oa = nt_name.attributes();
    let mut net = [0u8; 56];
    // SAFETY: valid pointers; FILE_NETWORK_OPEN_INFORMATION is 56 bytes.
    let st = unsafe { NtQueryFullAttributesFile(&oa, net.as_mut_ptr()) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    let data = fileex::attribute_data(&net);
    // SAFETY: the caller's WIN32_FILE_ATTRIBUTE_DATA (36 bytes).
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), out, data.len()) };
    TRUE
}

// ----- information by handle ---------------------------------------------------------------------

/// `FileFsVolumeInformation` (`wdm.h`): `VolumeSerialNumber` at 8.
const FILE_FS_VOLUME_INFORMATION: u32 = 1;

/// The serial number of the volume of `h`.
fn volume_serial(h: Handle) -> u32 {
    let mut volume = [0u8; 128];
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st =
        unsafe { nt::NtQueryVolumeInformationFile(h, &mut iosb, volume.as_mut_ptr(), 128, FILE_FS_VOLUME_INFORMATION) };
    if st.is_success() || st.0 == 0x8000_0005 { u32::from_le_bytes(volume[8..12].try_into().unwrap()) } else { 0 }
}

/// `GetFileInformationByHandleEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileInformationByHandleEx(handle: Handle, class: u32, buf: *mut u8, size: u32) -> i32 {
    let plan = match fileex::query_plan(class) {
        Ok(p) => p,
        Err(e) => return fail(e, FALSE),
    };
    if buf.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    if let Err(e) = fileex::check_out_size(plan, size as usize) {
        return fail(e, FALSE);
    }
    let mut iosb = IoStatusBlock::default();
    match plan {
        QueryPlan::Direct(nt_class) => {
            // SAFETY: the caller's buffer of `size` bytes.
            let st = unsafe { nt::NtQueryInformationFile(handle, &mut iosb, buf, size, nt_class) };
            if st.is_success() { TRUE } else { fail_status(st, FALSE) }
        }
        QueryPlan::Compression => {
            let mut standard = [0u8; 24];
            if let Err(st) = query(handle, fileex::nt::FILE_STANDARD_INFORMATION, &mut standard) {
                return fail_status(st, FALSE);
            }
            let info = fileex::compression_info(&standard);
            // SAFETY: the caller's buffer holds 16 bytes (checked).
            unsafe { core::ptr::copy_nonoverlapping(info.as_ptr(), buf, info.len()) };
            TRUE
        }
        QueryPlan::Id => {
            let mut index = [0u8; 8];
            if let Err(st) = query(handle, fileex::nt::FILE_INTERNAL_INFORMATION, &mut index) {
                return fail_status(st, FALSE);
            }
            let info = fileex::id_info(u64::from(volume_serial(handle)), u64::from_le_bytes(index));
            // SAFETY: the caller's buffer holds 24 bytes (checked).
            unsafe { core::ptr::copy_nonoverlapping(info.as_ptr(), buf, info.len()) };
            TRUE
        }
        QueryPlan::IoPriorityHint => {
            // SAFETY: the caller's buffer holds 4 bytes (checked).
            unsafe { buf.cast::<u32>().write_unaligned(fileex::IO_PRIORITY_HINT_NORMAL) };
            TRUE
        }
        QueryPlan::Directory { nt_class, restart } => {
            // SAFETY: the caller's buffer of `size` bytes.
            let st = unsafe {
                nt::NtQueryDirectoryFile(
                    handle,
                    0,
                    core::ptr::null(),
                    core::ptr::null(),
                    &mut iosb,
                    buf,
                    size,
                    nt_class,
                    0,
                    core::ptr::null(),
                    restart as u8,
                )
            };
            if st.is_success() { TRUE } else { fail_status(st, FALSE) }
        }
    }
}

/// `SetFileInformationByHandle`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetFileInformationByHandle(handle: Handle, class: u32, buf: *const u8, size: u32) -> i32 {
    if buf.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's buffer of `size` bytes.
    let bytes = unsafe { core::slice::from_raw_parts(buf, size as usize) };
    match fileex::set_plan(class, bytes, &|win32| nt_path(win32)) {
        Ok(SetPlan::Nt { nt_class, bytes }) => match set_info(handle, nt_class, &bytes) {
            Ok(()) => TRUE,
            Err(st) => fail_status(st, FALSE),
        },
        Ok(SetPlan::Nothing) => TRUE,
        Err(e) => fail(e, FALSE),
    }
}

// ----- byte-range locks ----------------------------------------------------------------------------

/// The offset an `OVERLAPPED` carries (`Offset`, `OffsetHigh`).
///
/// # Safety
/// `ov` is null or a valid `OVERLAPPED`.
unsafe fn overlapped_range(ov: *const crate::file::Overlapped) -> Option<(u32, u32)> {
    // SAFETY: the caller's contract.
    if ov.is_null() { None } else { Some(unsafe { ((*ov).offset, (*ov).offset_high) }) }
}

/// `LockFileEx`: Learn: the `OVERLAPPED` gives the offset (and the event
/// signalled when a lock that had to wait is granted); a lock that cannot
/// be granted with `LOCKFILE_FAIL_IMMEDIATELY` is `ERROR_LOCK_VIOLATION`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LockFileEx(
    handle: Handle,
    flags: u32,
    reserved: u32,
    len_low: u32,
    len_high: u32,
    overlapped: *mut crate::file::Overlapped,
) -> i32 {
    // SAFETY: the caller's OVERLAPPED.
    let offset = unsafe { overlapped_range(overlapped) };
    let allowed = fileex::LOCKFILE_FAIL_IMMEDIATELY | fileex::LOCKFILE_EXCLUSIVE_LOCK;
    let (offset, length) = match fileex::lock_range(flags, reserved, len_low, len_high, offset, allowed) {
        Ok(r) => r,
        Err(e) => return fail(e, FALSE),
    };
    // SAFETY: not null (the range was checked); the first fields of an
    // OVERLAPPED are the IO_STATUS_BLOCK the kernel completes.
    let (iosb, event) = unsafe {
        (*overlapped).internal = nt::STATUS_PENDING.0 as u64;
        (overlapped as *mut IoStatusBlock, (*overlapped).event)
    };
    // SAFETY: valid pointers.
    let st = unsafe {
        NtLockFile(
            handle,
            event,
            core::ptr::null(),
            core::ptr::null(),
            iosb,
            &offset,
            &length,
            0,
            u8::from(flags & fileex::LOCKFILE_FAIL_IMMEDIATELY != 0),
            u8::from(flags & fileex::LOCKFILE_EXCLUSIVE_LOCK != 0),
        )
    };
    if st == nt::STATUS_PENDING {
        return fail(ERROR_IO_PENDING, FALSE);
    }
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `UnlockFileEx`: a range that is not locked is `ERROR_NOT_LOCKED`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn UnlockFileEx(
    handle: Handle,
    reserved: u32,
    len_low: u32,
    len_high: u32,
    overlapped: *mut crate::file::Overlapped,
) -> i32 {
    // SAFETY: the caller's OVERLAPPED.
    let offset = unsafe { overlapped_range(overlapped) };
    let (offset, length) = match fileex::lock_range(0, reserved, len_low, len_high, offset, 0) {
        Ok(r) => r,
        Err(e) => return fail(e, FALSE),
    };
    let mut iosb = IoStatusBlock::default();
    // SAFETY: valid pointers.
    let st = unsafe { NtUnlockFile(handle, &mut iosb, &offset, &length, 0) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

// ----- ReplaceFileW ---------------------------------------------------------------------------------------

/// `FILE_BASIC_INFORMATION` of the file named `win32`.
fn basic_of(win32: &[u16]) -> Result<[u8; 40], u32> {
    let h = open_for(win32, FILE_READ_ATTRIBUTES, FILE_OPEN_FOR_BACKUP_INTENT | FILE_OPEN_REPARSE_POINT)?;
    let mut basic = [0u8; 40];
    let r = query(h, FILE_BASIC_INFORMATION, &mut basic);
    close(h);
    r.map(|()| basic).map_err(from_status)
}

/// Give `win32` (the replacement) the replaced file's creation time and
/// attributes.
fn merge_into(win32: &[u16], replaced: &[u8; 40]) -> Result<(), u32> {
    // FILE_WRITE_ATTRIBUTES.
    let h = open_for(win32, 0x100, FILE_OPEN_FOR_BACKUP_INTENT | FILE_OPEN_REPARSE_POINT)?;
    let r = set_info(h, FILE_BASIC_INFORMATION, &fileex::merge_basic(replaced));
    close(h);
    r.map_err(from_status)
}

/// `ReplaceFileW`: Learn: the replacement takes the name, creation time and
/// attributes of the replaced file, which goes to the backup name if one is
/// given. A replaced file that does not exist is `ERROR_FILE_NOT_FOUND`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ReplaceFileW(
    replaced: *const u16,
    replacement: *const u16,
    backup: *const u16,
    flags: u32,
    exclude: *mut u8,
    reserved: *mut u8,
) -> i32 {
    if let Err(e) = fileex::replace_args(flags, exclude.is_null(), reserved.is_null()) {
        return fail(e, FALSE);
    }
    // SAFETY: the caller's strings.
    let (Some(replaced), Some(replacement), backup) =
        (unsafe { wide_arg(replaced) }, unsafe { wide_arg(replacement) }, unsafe { wide_arg(backup) })
    else {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    };
    let old = match basic_of(replaced) {
        Ok(b) => b,
        Err(e) => return fail(e, FALSE),
    };
    if let Err(e) = basic_of(replacement) {
        return fail(e, FALSE);
    }
    if let Err(e) = merge_into(replacement, &old)
        && flags & fileex::REPLACEFILE_IGNORE_MERGE_ERRORS == 0
    {
        let _ = e;
        return fail(fileex::replace_error(ReplaceStep::MergeAttributes), FALSE);
    }
    let (r, rp) = (with_nul(replaced), with_nul(replacement));
    if let Some(backup) = backup {
        let b = with_nul(backup);
        // SAFETY: NUL-terminated names.
        if unsafe { crate::file::MoveFileExW(r.as_ptr(), b.as_ptr(), MOVEFILE_REPLACE_EXISTING) } == FALSE {
            return fail(fileex::replace_error(ReplaceStep::RemoveReplaced), FALSE);
        }
        // SAFETY: as above.
        if unsafe { crate::file::MoveFileExW(rp.as_ptr(), r.as_ptr(), MOVEFILE_REPLACE_EXISTING) } == FALSE {
            // Undo: the replaced file takes its name back.
            // SAFETY: as above.
            unsafe { crate::file::MoveFileExW(b.as_ptr(), r.as_ptr(), MOVEFILE_REPLACE_EXISTING) };
            return fail(fileex::replace_error(ReplaceStep::MoveReplacement), FALSE);
        }
        return TRUE;
    }
    // SAFETY: as above.
    if unsafe { crate::file::MoveFileExW(rp.as_ptr(), r.as_ptr(), MOVEFILE_REPLACE_EXISTING) } == FALSE {
        return fail(fileex::replace_error(ReplaceStep::MoveReplacement), FALSE);
    }
    TRUE
}

// ----- GetFinalPathNameByHandleW ---------------------------------------------------------------------

/// `FileNameInformation` (`wdm.h`): `FileNameLength` and `FileName`.
fn name_below_volume(h: Handle) -> Result<Vec<u16>, NtStatus> {
    let mut buf = alloc::vec![0u8; 4 + 2 * 4096];
    query(h, fileex::nt::FILE_NAME_INFORMATION, &mut buf)?;
    let len = (u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize).min(buf.len() - 4);
    Ok(buf[4..4 + len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
}

/// The drive letter of the volume with `serial`, by opening each drive
/// root.
fn drive_with_serial(serial: u32) -> Option<u16> {
    for letter in b'A'..=b'Z' {
        let root: Vec<u16> = [letter, b':', b'\\'].iter().map(|&c| u16::from(c)).collect();
        let Ok(h) = open_for(&root, FILE_READ_ATTRIBUTES, FILE_DIRECTORY_FILE | FILE_OPEN_FOR_BACKUP_INTENT) else {
            continue;
        };
        let s = volume_serial(h);
        close(h);
        if s == serial {
            return Some(u16::from(letter));
        }
    }
    None
}

/// `GetFinalPathNameByHandleW`: Learn: the length without the NUL, or the
/// size needed (with it) when the buffer is too small, 0 on failure. Poly
/// answers `VOLUME_NAME_DOS` (`\\?\C:\...`) and `VOLUME_NAME_NONE`; the GUID
/// and NT forms are `ERROR_NOT_SUPPORTED` (Poly's volumes have neither
/// name).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFinalPathNameByHandleW(handle: Handle, buf: *mut u16, size: u32, flags: u32) -> u32 {
    let volume = match fileex::final_path_volume(flags) {
        Ok(v) => v,
        Err(e) => return fail(e, 0),
    };
    if volume == fileex::VOLUME_NAME_GUID || volume == fileex::VOLUME_NAME_NT {
        return fail(ERROR_NOT_SUPPORTED, 0);
    }
    let below = match name_below_volume(handle) {
        Ok(n) => n,
        Err(st) => return fail_status(st, 0),
    };
    let drive = if volume == fileex::VOLUME_NAME_DOS {
        match drive_with_serial(volume_serial(handle)) {
            Some(d) => d,
            None => return fail(ERROR_PATH_NOT_FOUND, 0),
        }
    } else {
        0
    };
    let path = fileex::final_path(volume, drive, &below);
    // SAFETY: the caller's buffer.
    unsafe { return_sized(&path, buf, size) }
}

// ----- GetLongPathNameW ---------------------------------------------------------------------------------

/// The name `dir` (as given, with its trailing separator) holds for the
/// component `name`: a `FindFirstFileW` for exactly that name.
fn find_name(dir: &[u16], name: &[u16]) -> Result<Option<Vec<u16>>, u32> {
    let mut pattern = dir.to_vec();
    pattern.extend_from_slice(name);
    pattern.push(0);
    let mut data = [0u8; find_data::SIZE];
    // SAFETY: a NUL-terminated pattern; a WIN32_FIND_DATAW.
    let h = unsafe { crate::file::FindFirstFileW(pattern.as_ptr(), data.as_mut_ptr()) };
    if h == INVALID_HANDLE_VALUE {
        let e = last_error();
        return if e == ERROR_FILE_NOT_FOUND || e == ERROR_PATH_NOT_FOUND { Ok(None) } else { Err(e) };
    }
    // SAFETY: the handle FindFirstFileW returned.
    unsafe { crate::file::FindClose(h) };
    let raw = &data[find_data::FILE_NAME..find_data::FILE_NAME + 2 * find_data::MAX_PATH];
    let units: Vec<u16> =
        raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    Ok(Some(units))
}

/// `GetLongPathNameW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetLongPathNameW(short: *const u16, long: *mut u16, size: u32) -> u32 {
    // SAFETY: the caller's string.
    let Some(short) = (unsafe { wide_arg(short) }) else { return fail(ERROR_INVALID_PARAMETER, 0) };
    match fileex::long_path(short, &mut find_name) {
        // SAFETY: the caller's buffer.
        Ok(p) => unsafe { return_sized(&p, long, size) },
        Err(e) => fail(e, 0),
    }
}

// ----- the system directories ----------------------------------------------------------------------

/// `%SystemRoot%`, else `C:\Windows`.
fn windows_dir() -> Vec<u16> {
    let w = poly_kernelbase::wide;
    crate::process::env_var(&w("SystemRoot")).unwrap_or_else(|| w("C:\\Windows"))
}

/// `GetSystemDirectoryW`: `<windows>\System32`, without a trailing
/// separator.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSystemDirectoryW(buf: *mut u16, size: u32) -> u32 {
    let mut dir = windows_dir();
    dir.extend("\\System32".encode_utf16());
    // SAFETY: the caller's buffer.
    unsafe { return_sized(&dir, buf, size) }
}

/// `GetWindowsDirectoryW`, `GetSystemWindowsDirectoryW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowsDirectoryW(buf: *mut u16, size: u32) -> u32 {
    // SAFETY: the caller's buffer.
    unsafe { return_sized(&windows_dir(), buf, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSystemWindowsDirectoryW(buf: *mut u16, size: u32) -> u32 {
    // SAFETY: as above.
    unsafe { GetWindowsDirectoryW(buf, size) }
}
