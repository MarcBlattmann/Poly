//! File functions 7-Zip's command line imports (ADR 0087): the older
//! moves, hard links, alternate streams, and MS-DOS dates and times.

use crate::base::*;
use crate::nt::{self, Handle, IoStatusBlock};
use poly_kernelbase::error::*;
use poly_kernelbase::file::{
    FILE_OPEN_FOR_BACKUP_INTENT, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, MOVEFILE_COPY_ALLOWED,
    SYNCHRONIZE, rename_information,
};
use poly_kernelbase::time;

/// `MoveFileW`: Learn: moves a file or directory, a file also to another
/// volume, so `MoveFileExW` with `MOVEFILE_COPY_ALLOWED` (which Poly's
/// `MoveFileExW` does not do across volumes yet, ADR 0079).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MoveFileW(from: *const u16, to: *const u16) -> i32 {
    // SAFETY: the caller's strings.
    unsafe { crate::file::MoveFileExW(from, to, MOVEFILE_COPY_ALLOWED) }
}

/// `MoveFileWithProgressW`: `MoveFileExW`; a rename moves no data, so the
/// progress routine is not called (Poly's reading, to be confirmed).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MoveFileWithProgressW(
    from: *const u16,
    to: *const u16,
    _progress: *const u8,
    _data: *mut u8,
    flags: u32,
) -> i32 {
    // SAFETY: the caller's strings.
    unsafe { crate::file::MoveFileExW(from, to, flags) }
}

/// `FileLinkInformation` (`wdm.h`): `FILE_LINK_INFORMATION` has
/// `FILE_RENAME_INFORMATION`'s layout.
const FILE_LINK_INFORMATION: u32 = 11;
/// `STATUS_INVALID_INFO_CLASS` (`ntstatus.h`).
const STATUS_INVALID_INFO_CLASS: u32 = 0xC000_0003;

/// `CreateHardLinkW`: `NtSetInformationFile(FileLinkInformation)` on the
/// existing file. Poly's NT file I/O has no hard links yet: the class is
/// unknown there, which is reported as a volume without hard links,
/// `ERROR_INVALID_FUNCTION` (a stub until then, ADR 0087).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateHardLinkW(new: *const u16, existing: *const u16, _security: *const u8) -> i32 {
    // SAFETY: the caller's strings.
    let (Some(new), Some(existing)) = (unsafe { wide_arg(new) }, unsafe { wide_arg(existing) }) else {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    };
    let target = match nt_path(new) {
        Ok(t) => t,
        Err(e) => return fail(e, FALSE),
    };
    let name = match nt_name(existing) {
        Ok(n) => n,
        Err(e) => return fail(e, FALSE),
    };
    let oa = name.attributes();
    let mut h: Handle = 0;
    let mut iosb = IoStatusBlock::default();
    // FILE_WRITE_ATTRIBUTES (`winnt.h`).
    let access = 0x100 | SYNCHRONIZE;
    let options = FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_FOR_BACKUP_INTENT | FILE_OPEN_REPARSE_POINT;
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtOpenFile(&mut h, access, &oa, &mut iosb, 7, options) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    let info = rename_information(false, &target);
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtSetInformationFile(h, &mut iosb, info.as_ptr(), info.len() as u32, FILE_LINK_INFORMATION) };
    close(h);
    if st.is_success() {
        TRUE
    } else if st.0 == STATUS_INVALID_INFO_CLASS {
        fail(ERROR_INVALID_FUNCTION, FALSE)
    } else {
        fail_status(st, FALSE)
    }
}

/// `FindStreamInfoStandard` (`minwinbase.h`).
const FIND_STREAM_INFO_STANDARD: u32 = 0;

/// `FindFirstStreamW`: Learn: "If no streams can be found, the function
/// fails and GetLastError returns ERROR_HANDLE_EOF". Poly's file systems
/// keep no NTFS streams, so after checking the arguments and that the file
/// exists, every file reports none (a stub, ADR 0087).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindFirstStreamW(name: *const u16, level: u32, _data: *mut u8, flags: u32) -> Handle {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, INVALID_HANDLE_VALUE) };
    if level != FIND_STREAM_INFO_STANDARD || flags != 0 {
        return fail(ERROR_INVALID_PARAMETER, INVALID_HANDLE_VALUE);
    }
    match crate::file::attributes_of(name) {
        Ok(_) => fail(ERROR_HANDLE_EOF, INVALID_HANDLE_VALUE),
        Err(e) => fail(e, INVALID_HANDLE_VALUE),
    }
}

/// `FindNextStreamW`: no handle of `FindFirstStreamW` exists (above).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindNextStreamW(_find: Handle, _data: *mut u8) -> i32 {
    fail(ERROR_INVALID_HANDLE, FALSE)
}

/// `FileTimeToDosDateTime`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FileTimeToDosDateTime(ft: *const u8, date: *mut u16, time_out: *mut u16) -> i32 {
    // SAFETY: the caller's FILETIME (unaligned).
    match time::to_dos_date_time(unsafe { ft.cast::<u64>().read_unaligned() }) {
        Some((d, t)) => {
            // SAFETY: the caller's out-pointers.
            unsafe {
                date.write_unaligned(d);
                time_out.write_unaligned(t);
            }
            TRUE
        }
        None => fail(ERROR_INVALID_PARAMETER, FALSE),
    }
}

/// `DosDateTimeToFileTime`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DosDateTimeToFileTime(date: u16, time_in: u16, ft: *mut u8) -> i32 {
    match time::from_dos_date_time(date, time_in) {
        Some(v) => {
            // SAFETY: the caller's FILETIME (unaligned).
            unsafe { ft.cast::<u64>().write_unaligned(v) };
            TRUE
        }
        None => fail(ERROR_INVALID_PARAMETER, FALSE),
    }
}
