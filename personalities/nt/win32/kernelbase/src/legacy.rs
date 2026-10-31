//! File functions 7-Zip's command line imports (ADR 0087): the older
//! moves, hard links, alternate streams, and MS-DOS dates and times.

use crate::base::*;
use crate::nt::{self, Handle, IoStatusBlock};
use alloc::collections::VecDeque;
use poly_kernelbase::error::*;
use poly_kernelbase::file::{
    FILE_OPEN_FOR_BACKUP_INTENT, FILE_OPEN_REPARSE_POINT, FILE_STREAM_INFORMATION, FILE_SYNCHRONOUS_IO_NONALERT,
    FIND_STREAM_DATA_SIZE, MOVEFILE_COPY_ALLOWED, SYNCHRONIZE, find_stream_data, rename_information,
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
/// existing file (ADR 0091). A file system that does not know the class
/// is reported as one without hard links, `ERROR_INVALID_FUNCTION`; a new
/// name that exists, `ERROR_ALREADY_EXISTS`.
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
/// `FILE_READ_ATTRIBUTES` (`winnt.h`).
const FILE_READ_ATTRIBUTES: u32 = 0x80;

/// The streams of the file or directory `name`, as
/// `WIN32_FIND_STREAM_DATA` records: `NtQueryInformationFile`'s
/// `FileStreamInformation`, in a buffer that grows while it is too small.
fn streams_of(name: &[u16]) -> Result<VecDeque<[u8; FIND_STREAM_DATA_SIZE]>, u32> {
    let name = nt_name(name)?;
    let oa = name.attributes();
    let mut h: Handle = 0;
    let mut iosb = IoStatusBlock::default();
    let options = FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_FOR_BACKUP_INTENT;
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtOpenFile(&mut h, FILE_READ_ATTRIBUTES | SYNCHRONIZE, &oa, &mut iosb, 7, options) };
    if !st.is_success() {
        return Err(from_status(st));
    }
    let mut buf = alloc::vec![0u8; 1024];
    let answer = loop {
        let mut iosb = IoStatusBlock::default();
        // SAFETY: valid pointers; `buf` holds `buf.len()` bytes.
        let st = unsafe {
            nt::NtQueryInformationFile(h, &mut iosb, buf.as_mut_ptr(), buf.len() as u32, FILE_STREAM_INFORMATION)
        };
        if st.0 == STATUS_BUFFER_OVERFLOW && buf.len() < 1 << 20 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        break if st.is_success() { Ok(iosb.information as usize) } else { Err(from_status(st)) };
    };
    close(h);
    Ok(find_stream_data(&buf[..answer?.min(buf.len())]).into())
}

/// `STATUS_BUFFER_OVERFLOW` (`ntstatus.h`).
const STATUS_BUFFER_OVERFLOW: u32 = 0x8000_0005;

/// `FindFirstStreamW`: the first stream of a file or directory
/// (`FileStreamInformation`). Poly's volumes keep no named streams, so a
/// file has its default data stream `::$DATA` only and a directory none,
/// which Learn says fails with `ERROR_HANDLE_EOF` ("If no streams can be
/// found").
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindFirstStreamW(name: *const u16, level: u32, data: *mut u8, flags: u32) -> Handle {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, INVALID_HANDLE_VALUE) };
    if level != FIND_STREAM_INFO_STANDARD || flags != 0 || data.is_null() {
        return fail(ERROR_INVALID_PARAMETER, INVALID_HANDLE_VALUE);
    }
    let mut streams = match streams_of(name) {
        Ok(s) => s,
        Err(e) => return fail(e, INVALID_HANDLE_VALUE),
    };
    let Some(first) = streams.pop_front() else { return fail(ERROR_HANDLE_EOF, INVALID_HANDLE_VALUE) };
    // SAFETY: the caller's WIN32_FIND_STREAM_DATA.
    unsafe { core::ptr::copy_nonoverlapping(first.as_ptr(), data, first.len()) };
    set_last_error(ERROR_SUCCESS);
    crate::file::stream_find_handle(streams)
}

/// `FindNextStreamW`: the next stream; `ERROR_HANDLE_EOF` after the last
/// (Learn).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindNextStreamW(find: Handle, data: *mut u8) -> i32 {
    match crate::file::next_stream(find) {
        None => fail(ERROR_INVALID_HANDLE, FALSE),
        Some(None) => fail(ERROR_HANDLE_EOF, FALSE),
        Some(Some(d)) => {
            // SAFETY: the caller's WIN32_FIND_STREAM_DATA.
            unsafe { core::ptr::copy_nonoverlapping(d.as_ptr(), data, d.len()) };
            TRUE
        }
    }
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
