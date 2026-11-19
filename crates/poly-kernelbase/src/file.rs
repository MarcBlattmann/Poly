//! Files: `CreateFileW`'s arguments as `NtCreateFile`'s, the file pointer,
//! and `WIN32_FIND_DATAW` from a directory entry.
//!
//! Constants are `winnt.h` / `fileapi.h` / `winbase.h` (Win32 side) and
//! `wdm.h` / `ntifs.h` (NT side); the rules are Learn's `CreateFileW`,
//! `SetFilePointer(Ex)`, `ReadFile` and `FindFirstFileW` pages.

use crate::error::{ERROR_INVALID_PARAMETER, ERROR_NEGATIVE_SEEK};
use alloc::vec::Vec;

// ----- Win32 constants -------------------------------------------------------

pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const GENERIC_EXECUTE: u32 = 0x2000_0000;
pub const GENERIC_ALL: u32 = 0x1000_0000;
pub const DELETE: u32 = 0x0001_0000;
pub const SYNCHRONIZE: u32 = 0x0010_0000;
pub const FILE_READ_ATTRIBUTES: u32 = 0x0080;
pub const FILE_WRITE_DATA: u32 = 0x0002;
pub const FILE_APPEND_DATA: u32 = 0x0004;
pub const FILE_LIST_DIRECTORY: u32 = 0x0001;

pub const CREATE_NEW: u32 = 1;
pub const CREATE_ALWAYS: u32 = 2;
pub const OPEN_EXISTING: u32 = 3;
pub const OPEN_ALWAYS: u32 = 4;
pub const TRUNCATE_EXISTING: u32 = 5;

pub const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
pub const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
pub const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
pub const FILE_FLAG_RANDOM_ACCESS: u32 = 0x1000_0000;
pub const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
pub const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
pub const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
pub const FILE_FLAG_POSIX_SEMANTICS: u32 = 0x0100_0000;
pub const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
pub const FILE_FLAG_OPEN_NO_RECALL: u32 = 0x0010_0000;

pub const FILE_ATTRIBUTE_READONLY: u32 = 0x0001;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0010;
pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x0080;
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
/// The attributes `CreateFileW` passes on (`FILE_ATTRIBUTE_VALID_FLAGS`
/// less the ones only the system sets).
pub const FILE_ATTRIBUTE_VALID_SET_FLAGS: u32 = 0x31A7;
pub const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;

pub const FILE_BEGIN: u32 = 0;
pub const FILE_CURRENT: u32 = 1;
pub const FILE_END: u32 = 2;

pub const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
pub const MOVEFILE_COPY_ALLOWED: u32 = 0x2;
pub const MOVEFILE_DELAY_UNTIL_REBOOT: u32 = 0x4;
pub const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

// ----- NT constants (wdm.h / ntifs.h) ---------------------------------------

pub const FILE_SUPERSEDE: u32 = 0;
pub const FILE_OPEN: u32 = 1;
pub const FILE_CREATE: u32 = 2;
pub const FILE_OPEN_IF: u32 = 3;
pub const FILE_OVERWRITE: u32 = 4;
pub const FILE_OVERWRITE_IF: u32 = 5;

pub const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
pub const FILE_WRITE_THROUGH: u32 = 0x0000_0002;
pub const FILE_SEQUENTIAL_ONLY: u32 = 0x0000_0004;
pub const FILE_NO_INTERMEDIATE_BUFFERING: u32 = 0x0000_0008;
pub const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
pub const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
pub const FILE_RANDOM_ACCESS: u32 = 0x0000_0800;
pub const FILE_DELETE_ON_CLOSE: u32 = 0x0000_1000;
pub const FILE_OPEN_FOR_BACKUP_INTENT: u32 = 0x0000_4000;
pub const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

/// `IO_STATUS_BLOCK.Information` after a create.
pub const FILE_SUPERSEDED: u64 = 0;
pub const FILE_OPENED: u64 = 1;
pub const FILE_CREATED: u64 = 2;
pub const FILE_OVERWRITTEN: u64 = 3;
pub const FILE_EXISTS: u64 = 4;

/// The information classes `kernelbase` uses (`wdm.h` / `ntifs.h`).
pub const FILE_BOTH_DIRECTORY_INFORMATION: u32 = 3;
pub const FILE_BASIC_INFORMATION: u32 = 4;
pub const FILE_STANDARD_INFORMATION: u32 = 5;
pub const FILE_RENAME_INFORMATION: u32 = 10;
pub const FILE_DISPOSITION_INFORMATION: u32 = 13;
pub const FILE_POSITION_INFORMATION: u32 = 14;
pub const FILE_END_OF_FILE_INFORMATION: u32 = 20;

/// `NtCreateFile`'s arguments for a `CreateFileW` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Create {
    pub access: u32,
    pub attributes: u32,
    pub disposition: u32,
    pub options: u32,
}

/// `CreateFileW`'s `dwDesiredAccess`, `dwCreationDisposition` and
/// `dwFlagsAndAttributes` as `NtCreateFile`'s. Learn: `TRUNCATE_EXISTING`
/// needs `GENERIC_WRITE`; an unknown disposition is
/// `ERROR_INVALID_PARAMETER`. The handle always gets `SYNCHRONIZE` and
/// `FILE_READ_ATTRIBUTES` (so it can be waited on and its size read);
/// without `FILE_FLAG_OVERLAPPED` its I/O is synchronous; without
/// `FILE_FLAG_BACKUP_SEMANTICS` it cannot be a directory.
pub fn create(access: u32, disposition: u32, flags_and_attributes: u32) -> Result<Create, u32> {
    let disposition = match disposition {
        CREATE_NEW => FILE_CREATE,
        CREATE_ALWAYS => FILE_OVERWRITE_IF,
        OPEN_EXISTING => FILE_OPEN,
        OPEN_ALWAYS => FILE_OPEN_IF,
        TRUNCATE_EXISTING => {
            if access & (GENERIC_WRITE | GENERIC_ALL | FILE_WRITE_DATA) == 0 {
                return Err(ERROR_INVALID_PARAMETER);
            }
            FILE_OVERWRITE
        }
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    let f = flags_and_attributes;
    let mut options = 0;
    let mut access = access | SYNCHRONIZE | FILE_READ_ATTRIBUTES;
    if f & FILE_FLAG_OVERLAPPED == 0 {
        options |= FILE_SYNCHRONOUS_IO_NONALERT;
    }
    if f & FILE_FLAG_BACKUP_SEMANTICS != 0 {
        options |= FILE_OPEN_FOR_BACKUP_INTENT;
    } else {
        options |= FILE_NON_DIRECTORY_FILE;
    }
    if f & FILE_FLAG_WRITE_THROUGH != 0 {
        options |= FILE_WRITE_THROUGH;
    }
    if f & FILE_FLAG_NO_BUFFERING != 0 {
        options |= FILE_NO_INTERMEDIATE_BUFFERING;
    }
    if f & FILE_FLAG_RANDOM_ACCESS != 0 {
        options |= FILE_RANDOM_ACCESS;
    }
    if f & FILE_FLAG_SEQUENTIAL_SCAN != 0 {
        options |= FILE_SEQUENTIAL_ONLY;
    }
    if f & FILE_FLAG_DELETE_ON_CLOSE != 0 {
        options |= FILE_DELETE_ON_CLOSE;
        access |= DELETE;
    }
    if f & FILE_FLAG_OPEN_REPARSE_POINT != 0 {
        options |= FILE_OPEN_REPARSE_POINT;
    }
    let mut attributes = f & FILE_ATTRIBUTE_VALID_SET_FLAGS;
    if attributes == 0 {
        attributes = FILE_ATTRIBUTE_NORMAL;
    }
    Ok(Create { access, attributes, disposition, options })
}

/// Whether a successful create of disposition `CREATE_ALWAYS` /
/// `OPEN_ALWAYS` found the file already there (Learn: the function
/// succeeds and the last error is `ERROR_ALREADY_EXISTS`).
pub fn existed(win32_disposition: u32, information: u64) -> bool {
    matches!(win32_disposition, CREATE_ALWAYS | OPEN_ALWAYS)
        && matches!(information, FILE_OPENED | FILE_OVERWRITTEN | FILE_SUPERSEDED | FILE_EXISTS)
}

/// `SetFilePointer(Ex)`: the new position for `method` and `distance`
/// from `current` in a file of `size` bytes. Learn: a negative result is
/// `ERROR_NEGATIVE_SEEK` (the pointer does not move); an unknown method is
/// `ERROR_INVALID_PARAMETER`; a position past the end is allowed.
pub fn seek(method: u32, distance: i64, current: u64, size: u64) -> Result<u64, u32> {
    let base = match method {
        FILE_BEGIN => 0i128,
        FILE_CURRENT => i128::from(current),
        FILE_END => i128::from(size),
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    let new = base + i128::from(distance);
    if new < 0 {
        return Err(ERROR_NEGATIVE_SEEK);
    }
    u64::try_from(new).ok().filter(|&n| n <= i64::MAX as u64).ok_or(ERROR_INVALID_PARAMETER)
}

/// `SetFilePointer`'s distance: `lDistanceToMove` alone is a signed 32-bit
/// value; with `lpDistanceToMoveHigh` the two form one signed 64-bit value.
pub fn distance(low: i32, high: Option<i32>) -> i64 {
    match high {
        Some(h) => (i64::from(h) << 32) | i64::from(low as u32),
        None => i64::from(low),
    }
}

/// `SetFilePointer` without `lpDistanceToMoveHigh` can only report a
/// position below 4 GiB - 1 (Learn: otherwise the function fails).
pub fn fits_low(position: u64) -> bool {
    position < 0xFFFF_FFFF
}

/// Whether `ReadFileEx` / `WriteFileEx` accept a request whose `NtReadFile` /
/// `NtWriteFile` answered `status` at once: the request is queued (pending), done, at the
/// end of the file, or a message pipe's read buffer was too short
/// (`STATUS_BUFFER_OVERFLOW`, 0x80000005: the routine still runs, with error 0 and the bytes
/// that fit; specs/windows/kernel32.dll/ReadFileEx.yaml, `message_mode_short_buffer`).
pub fn ex_request_accepted(status: u32) -> bool {
    const STATUS_PENDING: u32 = 0x103;
    const STATUS_END_OF_FILE: u32 = 0xC000_0011;
    const STATUS_BUFFER_OVERFLOW: u32 = 0x8000_0005;
    status < 0x8000_0000 || status == STATUS_PENDING || status == STATUS_END_OF_FILE || status == STATUS_BUFFER_OVERFLOW
}

/// The error the completion routine of `ReadFileEx` / `WriteFileEx` gets for the final
/// `status` of its request: 0 for a success and for the short-buffer warning of a message
/// pipe, else `None` (the caller converts the status).
pub fn ex_completion_error(status: u32) -> Option<u32> {
    const STATUS_BUFFER_OVERFLOW: u32 = 0x8000_0005;
    (status == 0 || status == STATUS_BUFFER_OVERFLOW).then_some(0)
}

/// The byte offset an `OVERLAPPED` structure gives (`Offset`, `OffsetHigh`).
pub fn overlapped_offset(low: u32, high: u32) -> i64 {
    ((u64::from(high) << 32) | u64::from(low)) as i64
}

// ----- directory entries -------------------------------------------------------

/// `FILE_BOTH_DIR_INFORMATION` (`ntifs.h`, x64): field offsets.
pub mod both_dir {
    pub const NEXT_ENTRY_OFFSET: usize = 0;
    pub const CREATION_TIME: usize = 8;
    pub const LAST_ACCESS_TIME: usize = 16;
    pub const LAST_WRITE_TIME: usize = 24;
    pub const END_OF_FILE: usize = 40;
    pub const FILE_ATTRIBUTES: usize = 56;
    pub const FILE_NAME_LENGTH: usize = 60;
    pub const EA_SIZE: usize = 64;
    pub const SHORT_NAME_LENGTH: usize = 68;
    pub const SHORT_NAME: usize = 70;
    pub const FILE_NAME: usize = 94;
}

/// `WIN32_FIND_DATAW` (`minwinbase.h`): field offsets and size.
pub mod find_data {
    pub const FILE_ATTRIBUTES: usize = 0;
    pub const CREATION_TIME: usize = 4;
    pub const LAST_ACCESS_TIME: usize = 12;
    pub const LAST_WRITE_TIME: usize = 20;
    pub const FILE_SIZE_HIGH: usize = 28;
    pub const FILE_SIZE_LOW: usize = 32;
    pub const RESERVED0: usize = 36;
    pub const RESERVED1: usize = 40;
    pub const FILE_NAME: usize = 44;
    pub const ALTERNATE_FILE_NAME: usize = 564;
    pub const SIZE: usize = 592;
    /// `MAX_PATH`.
    pub const MAX_PATH: usize = 260;
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// The entries of a `FILE_BOTH_DIR_INFORMATION` buffer: the byte offset
/// of each, following `NextEntryOffset` (bounds-checked).
pub fn dir_entries(buf: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = 0usize;
    loop {
        if at + both_dir::FILE_NAME > buf.len() {
            break;
        }
        out.push(at);
        let next = u32_at(buf, at + both_dir::NEXT_ENTRY_OFFSET).unwrap_or(0) as usize;
        if next == 0 {
            break;
        }
        at += next;
    }
    out
}

/// One `FILE_BOTH_DIR_INFORMATION` entry at `at` as a `WIN32_FIND_DATAW`.
/// Learn: `dwReserved0` holds the reparse tag when the entry is a reparse
/// point (the entry's `EaSize` carries it); a name longer than `MAX_PATH -
/// 1` is cut (Windows reports such files only through the `\\?\` APIs).
pub fn to_find_data(buf: &[u8], at: usize) -> Option<[u8; find_data::SIZE]> {
    use both_dir as b;
    use find_data as f;
    let mut out = [0u8; f::SIZE];
    let attrs = u32_at(buf, at + b::FILE_ATTRIBUTES)?;
    out[f::FILE_ATTRIBUTES..][..4].copy_from_slice(&attrs.to_le_bytes());
    for (src, dst) in [
        (b::CREATION_TIME, f::CREATION_TIME),
        (b::LAST_ACCESS_TIME, f::LAST_ACCESS_TIME),
        (b::LAST_WRITE_TIME, f::LAST_WRITE_TIME),
    ] {
        out[dst..][..8].copy_from_slice(&u64_at(buf, at + src)?.to_le_bytes());
    }
    let size = u64_at(buf, at + b::END_OF_FILE)?;
    out[f::FILE_SIZE_HIGH..][..4].copy_from_slice(&((size >> 32) as u32).to_le_bytes());
    out[f::FILE_SIZE_LOW..][..4].copy_from_slice(&(size as u32).to_le_bytes());
    if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        out[f::RESERVED0..][..4].copy_from_slice(&u32_at(buf, at + b::EA_SIZE)?.to_le_bytes());
    }
    let name_len = u32_at(buf, at + b::FILE_NAME_LENGTH)? as usize;
    let name = buf.get(at + b::FILE_NAME..at + b::FILE_NAME + name_len)?;
    let n = name.len().min((f::MAX_PATH - 1) * 2) & !1;
    out[f::FILE_NAME..][..n].copy_from_slice(&name[..n]);
    let short_len = (*buf.get(at + b::SHORT_NAME_LENGTH)? as usize).min(24) & !1;
    let short = buf.get(at + b::SHORT_NAME..at + b::SHORT_NAME + short_len)?;
    out[f::ALTERNATE_FILE_NAME..][..short_len].copy_from_slice(short);
    Some(out)
}

/// `FILE_RENAME_INFORMATION` (`ntifs.h`, x64): `ReplaceIfExists` (a
/// `BOOLEAN`, padded), `RootDirectory` at 8, `FileNameLength` at 16,
/// `FileName` at 20.
pub fn rename_information(replace: bool, nt_target: &[u16]) -> Vec<u8> {
    rename_information_in(8, replace, nt_target)
}

/// `FILE_RENAME_INFORMATION` / `FILE_LINK_INFORMATION` (`ntifs.h`: the same layout) for a
/// process whose pointers are `ptr_size` bytes (ADR 0186): `ReplaceIfExists` (a `BOOLEAN`,
/// padded to a pointer), `RootDirectory` (a `HANDLE`) at `ptr_size`, `FileNameLength` (a
/// `ULONG`) at `2 * ptr_size` and `FileName` after it: at 20 in a 64-bit process, 12 in a
/// 32-bit one.
pub fn rename_information_in(ptr_size: usize, replace: bool, nt_target: &[u16]) -> Vec<u8> {
    let length_at = 2 * ptr_size;
    let name_at = length_at + 4;
    let mut b = alloc::vec![0u8; name_at + nt_target.len() * 2 + 2];
    b[0] = replace as u8;
    b[length_at..name_at].copy_from_slice(&((nt_target.len() * 2) as u32).to_le_bytes());
    for (i, c) in nt_target.iter().enumerate() {
        b[name_at + 2 * i..name_at + 2 + 2 * i].copy_from_slice(&c.to_le_bytes());
    }
    b
}

/// `FILE_COMPLETION_INFORMATION` (`ntifs.h`) for a process whose pointers are `ptr_size` bytes
/// (ADR 0186): `Port` (a `HANDLE`) and `Key` (a `PVOID`), one pointer each (16 bytes in a
/// 64-bit process, 8 in a 32-bit one). The values are truncated to the pointer width.
pub fn completion_information(ptr_size: usize, port: u64, key: u64) -> Vec<u8> {
    let mut b = alloc::vec![0u8; 2 * ptr_size];
    b[..ptr_size].copy_from_slice(&port.to_le_bytes()[..ptr_size]);
    b[ptr_size..].copy_from_slice(&key.to_le_bytes()[..ptr_size]);
    b
}

/// `BY_HANDLE_FILE_INFORMATION` (`fileapi.h`, 52 bytes): `dwFileAttributes`
/// 0, `ftCreationTime` 4, `ftLastAccessTime` 12, `ftLastWriteTime` 20,
/// `dwVolumeSerialNumber` 28, `nFileSizeHigh` 32, `nFileSizeLow` 36,
/// `nNumberOfLinks` 40, `nFileIndexHigh` 44, `nFileIndexLow` 48.
pub const BY_HANDLE_FILE_INFORMATION_SIZE: usize = 52;

/// `GetFileInformationByHandle` from `FILE_BASIC_INFORMATION` (`wdm.h`:
/// the four times at 0, 8, 16, 24, `FileAttributes` at 32),
/// `FILE_STANDARD_INFORMATION` (`EndOfFile` at 8, `NumberOfLinks` at 16),
/// `FILE_INTERNAL_INFORMATION`'s `IndexNumber` and the volume's serial
/// number (`FILE_FS_VOLUME_INFORMATION`).
pub fn by_handle_information(
    basic: &[u8; 40],
    standard: &[u8; 24],
    index: u64,
    serial: u32,
) -> [u8; BY_HANDLE_FILE_INFORMATION_SIZE] {
    let mut out = [0u8; BY_HANDLE_FILE_INFORMATION_SIZE];
    out[0..4].copy_from_slice(&basic[32..36]);
    out[4..12].copy_from_slice(&basic[0..8]);
    out[12..20].copy_from_slice(&basic[8..16]);
    out[20..28].copy_from_slice(&basic[16..24]);
    out[28..32].copy_from_slice(&serial.to_le_bytes());
    out[32..36].copy_from_slice(&standard[12..16]);
    out[36..40].copy_from_slice(&standard[8..12]);
    out[40..44].copy_from_slice(&standard[16..20]);
    out[44..48].copy_from_slice(&((index >> 32) as u32).to_le_bytes());
    out[48..52].copy_from_slice(&(index as u32).to_le_bytes());
    out
}

/// `SetFileTime`'s `FILE_BASIC_INFORMATION`: Learn: a null time pointer
/// leaves that time alone, which the basic information says with 0; the
/// change time and attributes are left alone too (0).
pub fn set_times(creation: Option<u64>, access: Option<u64>, write: Option<u64>) -> [u8; 40] {
    let mut b = [0u8; 40];
    for (at, t) in [(0, creation), (8, access), (16, write)] {
        b[at..at + 8].copy_from_slice(&t.unwrap_or(0).to_le_bytes());
    }
    b
}

/// `FileStreamInformation` (`ntifs.h`).
pub const FILE_STREAM_INFORMATION: u32 = 22;

/// `WIN32_FIND_STREAM_DATA` (`minwinbase.h`): `StreamSize` (a
/// `LARGE_INTEGER`) at 0, `cStreamName[MAX_PATH + 36]` at 8.
pub const FIND_STREAM_DATA_SIZE: usize = 8 + 2 * (find_data::MAX_PATH + 36);

/// The entries of a `FILE_STREAM_INFORMATION` chain (`ntifs.h`:
/// `NextEntryOffset` 0, `StreamNameLength` 4, `StreamSize` 8,
/// `StreamAllocationSize` 16, `StreamName` 24), each as
/// `WIN32_FIND_STREAM_DATA` (a name too long for it is cut, terminated).
/// An empty buffer has none; a malformed entry ends the list.
pub fn find_stream_data(buf: &[u8]) -> Vec<[u8; FIND_STREAM_DATA_SIZE]> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let (Some(next), Some(len), Some(size)) = (u32_at(buf, at), u32_at(buf, at + 4), u64_at(buf, at + 8)) {
        let Some(name) = buf.get(at + 24..at + 24 + len as usize) else { break };
        let mut d = [0u8; FIND_STREAM_DATA_SIZE];
        d[0..8].copy_from_slice(&size.to_le_bytes());
        let n = name.len().min(FIND_STREAM_DATA_SIZE - 8 - 2) & !1;
        d[8..8 + n].copy_from_slice(&name[..n]);
        out.push(d);
        if next == 0 {
            break;
        }
        at += next as usize;
    }
    out
}

/// The full names of the named streams (`:name:$DATA`) among `WIN32_FIND_STREAM_DATA` records,
/// without the NUL: what `CopyFileW` appends to a file's name to copy each one. The default
/// stream (`::$DATA`, the file itself) is not one.
pub fn named_stream_suffixes(records: &[[u8; FIND_STREAM_DATA_SIZE]]) -> Vec<Vec<u16>> {
    records
        .iter()
        .map(|r| r[8..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect())
        .filter(|name: &Vec<u16>| name.len() > 1 && name[0] == u16::from(b':') && name[1] != u16::from(b':'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_streams_are_the_ones_after_the_default() {
        let record = |name: &str| {
            let mut d = [0u8; FIND_STREAM_DATA_SIZE];
            for (i, c) in name.encode_utf16().enumerate() {
                d[8 + 2 * i..10 + 2 * i].copy_from_slice(&c.to_le_bytes());
            }
            d
        };
        let records = [record("::$DATA"), record(":Zone.Identifier:$DATA"), record(":a<b:$DATA")];
        let names: Vec<alloc::string::String> =
            named_stream_suffixes(&records).iter().map(|n| alloc::string::String::from_utf16_lossy(n)).collect();
        assert_eq!(names, [":Zone.Identifier:$DATA", ":a<b:$DATA"]);
        assert!(named_stream_suffixes(&[]).is_empty());
    }

    #[test]
    fn stream_information_becomes_find_stream_data() {
        assert!(find_stream_data(&[]).is_empty());
        // Two entries: "::$DATA" (12 bytes) and ":x:$DATA" (3 bytes).
        let entry = |next: u32, name: &str, size: u64| {
            let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let mut e = alloc::vec![0u8; 24];
            e[0..4].copy_from_slice(&next.to_le_bytes());
            e[4..8].copy_from_slice(&(units.len() as u32).to_le_bytes());
            e[8..16].copy_from_slice(&size.to_le_bytes());
            e.extend_from_slice(&units);
            e
        };
        let mut buf = entry(40, "::$DATA", 12);
        buf.resize(40, 0);
        buf.extend(entry(0, ":x:$DATA", 3));
        let d = find_stream_data(&buf);
        assert_eq!(d.len(), 2);
        assert_eq!(u64::from_le_bytes(d[0][0..8].try_into().unwrap()), 12);
        let name = |r: &[u8; FIND_STREAM_DATA_SIZE]| -> alloc::string::String {
            let units: Vec<u16> =
                r[8..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
            alloc::string::String::from_utf16_lossy(&units)
        };
        assert_eq!(name(&d[0]), "::$DATA");
        assert_eq!((name(&d[1]).as_str(), d[1][0]), (":x:$DATA", 3));
        // A name that runs past the buffer ends the list.
        assert_eq!(find_stream_data(&buf[..30]).len(), 0);
        assert_eq!(FIND_STREAM_DATA_SIZE, 600);
    }

    #[test]
    fn by_handle_information_layout() {
        let mut basic = [0u8; 40];
        for (i, t) in [1u64, 2, 3, 4].iter().enumerate() {
            basic[i * 8..i * 8 + 8].copy_from_slice(&t.to_le_bytes());
        }
        basic[32..36].copy_from_slice(&0x20u32.to_le_bytes());
        let mut standard = [0u8; 24];
        standard[8..16].copy_from_slice(&0x1_0000_0005u64.to_le_bytes());
        standard[16..20].copy_from_slice(&1u32.to_le_bytes());
        let b = by_handle_information(&basic, &standard, 0x7_0000_0009, 0xABCD);
        let d = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let q = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        assert_eq!(d(0), 0x20);
        assert_eq!((q(4), q(12), q(20)), (1, 2, 3));
        assert_eq!(d(28), 0xABCD);
        assert_eq!((d(32), d(36)), (1, 5));
        assert_eq!(d(40), 1);
        assert_eq!((d(44), d(48)), (7, 9));
        let s = set_times(None, Some(5), None);
        assert_eq!(&s[0..8], &[0; 8]);
        assert_eq!(u64::from_le_bytes(s[8..16].try_into().unwrap()), 5);
        assert_eq!(&s[16..40], &[0; 24]);
    }

    #[test]
    fn dispositions_map() {
        let c = |d| create(GENERIC_READ, d, 0).map(|c| c.disposition);
        assert_eq!(c(CREATE_NEW), Ok(FILE_CREATE));
        assert_eq!(c(CREATE_ALWAYS), Ok(FILE_OVERWRITE_IF));
        assert_eq!(c(OPEN_EXISTING), Ok(FILE_OPEN));
        assert_eq!(c(OPEN_ALWAYS), Ok(FILE_OPEN_IF));
        assert_eq!(c(TRUNCATE_EXISTING), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(create(GENERIC_WRITE, TRUNCATE_EXISTING, 0).map(|c| c.disposition), Ok(FILE_OVERWRITE));
        assert_eq!(c(0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(c(6), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn flags_become_options() {
        let c = create(GENERIC_READ | GENERIC_WRITE, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL).unwrap();
        assert_eq!(c.options, FILE_SYNCHRONOUS_IO_NONALERT | FILE_NON_DIRECTORY_FILE);
        assert_eq!(c.access, GENERIC_READ | GENERIC_WRITE | SYNCHRONIZE | FILE_READ_ATTRIBUTES);
        assert_eq!(c.attributes, FILE_ATTRIBUTE_NORMAL);
        let c = create(0, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED).unwrap();
        assert_eq!(c.options, FILE_OPEN_FOR_BACKUP_INTENT);
        let c = create(GENERIC_WRITE, CREATE_ALWAYS, FILE_FLAG_DELETE_ON_CLOSE | FILE_ATTRIBUTE_READONLY).unwrap();
        assert_ne!(c.options & FILE_DELETE_ON_CLOSE, 0);
        assert_ne!(c.access & DELETE, 0);
        assert_eq!(c.attributes, FILE_ATTRIBUTE_READONLY);
        let c = create(
            GENERIC_READ,
            OPEN_EXISTING,
            FILE_FLAG_WRITE_THROUGH | FILE_FLAG_NO_BUFFERING | FILE_FLAG_RANDOM_ACCESS | FILE_FLAG_SEQUENTIAL_SCAN,
        )
        .unwrap();
        assert_eq!(
            c.options & !(FILE_SYNCHRONOUS_IO_NONALERT | FILE_NON_DIRECTORY_FILE),
            FILE_WRITE_THROUGH | FILE_NO_INTERMEDIATE_BUFFERING | FILE_RANDOM_ACCESS | FILE_SEQUENTIAL_ONLY
        );
    }

    #[test]
    fn already_exists_is_reported_for_always_dispositions() {
        assert!(existed(CREATE_ALWAYS, FILE_OVERWRITTEN));
        assert!(existed(OPEN_ALWAYS, FILE_OPENED));
        assert!(!existed(OPEN_ALWAYS, FILE_CREATED));
        assert!(!existed(OPEN_EXISTING, FILE_OPENED));
    }

    #[test]
    fn seeking() {
        assert_eq!(seek(FILE_BEGIN, 10, 5, 100), Ok(10));
        assert_eq!(seek(FILE_CURRENT, -3, 5, 100), Ok(2));
        assert_eq!(seek(FILE_END, 0, 5, 100), Ok(100));
        assert_eq!(seek(FILE_END, 50, 5, 100), Ok(150));
        assert_eq!(seek(FILE_CURRENT, -6, 5, 100), Err(ERROR_NEGATIVE_SEEK));
        assert_eq!(seek(3, 0, 0, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(distance(-1, None), -1);
        assert_eq!(distance(-1, Some(0)), 0xFFFF_FFFF);
        assert_eq!(distance(0, Some(1)), 1 << 32);
        assert_eq!(distance(-2, Some(-1)), -2);
        assert!(fits_low(0xFFFF_FFFE));
        assert!(!fits_low(0xFFFF_FFFF));
        assert_eq!(overlapped_offset(1, 1), (1 << 32) + 1);
        // A short buffer on a message pipe is accepted and completes with error 0.
        assert!(ex_request_accepted(0x8000_0005));
        assert!(ex_request_accepted(0x103) && ex_request_accepted(0) && ex_request_accepted(0xC000_0011));
        assert!(!ex_request_accepted(0xC000_0008));
        assert_eq!(ex_completion_error(0x8000_0005), Some(0));
        assert_eq!(ex_completion_error(0), Some(0));
        assert_eq!(ex_completion_error(0xC000_0011), None);
    }

    fn entry(name: &str, attrs: u32, size: u64, next: u32) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut b = alloc::vec![0u8; both_dir::FILE_NAME + units.len() * 2];
        b[0..4].copy_from_slice(&next.to_le_bytes());
        b[both_dir::CREATION_TIME..][..8].copy_from_slice(&1u64.to_le_bytes());
        b[both_dir::LAST_WRITE_TIME..][..8].copy_from_slice(&3u64.to_le_bytes());
        b[both_dir::END_OF_FILE..][..8].copy_from_slice(&size.to_le_bytes());
        b[both_dir::FILE_ATTRIBUTES..][..4].copy_from_slice(&attrs.to_le_bytes());
        b[both_dir::FILE_NAME_LENGTH..][..4].copy_from_slice(&((units.len() * 2) as u32).to_le_bytes());
        b[both_dir::EA_SIZE..][..4].copy_from_slice(&0xA000_000Cu32.to_le_bytes());
        for (i, c) in units.iter().enumerate() {
            b[both_dir::FILE_NAME + 2 * i..][..2].copy_from_slice(&c.to_le_bytes());
        }
        b
    }

    #[test]
    fn find_data_from_directory_entries() {
        let mut buf = entry("a.txt", FILE_ATTRIBUTE_NORMAL, 0x1_0000_0002, 0);
        let first_len = buf.len().next_multiple_of(8);
        buf.resize(first_len, 0);
        buf[0..4].copy_from_slice(&(first_len as u32).to_le_bytes());
        buf.extend(entry("Dir", FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT, 0, 0));
        let at = dir_entries(&buf);
        assert_eq!(at, [0, first_len]);
        let f = to_find_data(&buf, 0).unwrap();
        assert_eq!(u32::from_le_bytes(f[0..4].try_into().unwrap()), FILE_ATTRIBUTE_NORMAL);
        assert_eq!(u32::from_le_bytes(f[28..32].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(f[32..36].try_into().unwrap()), 2);
        assert_eq!(u64::from_le_bytes(f[4..12].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(f[20..28].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(f[36..40].try_into().unwrap()), 0);
        assert_eq!(&f[44..54], &[b'a', 0, b'.', 0, b't', 0, b'x', 0, b't', 0]);
        assert_eq!(&f[54..56], &[0, 0]);
        let d = to_find_data(&buf, first_len).unwrap();
        assert_eq!(u32::from_le_bytes(d[36..40].try_into().unwrap()), 0xA000_000C);
        // A truncated buffer yields no entry rather than a panic.
        assert_eq!(to_find_data(&buf[..50], 0), None);
        assert_eq!(dir_entries(&buf[..10]), Vec::<usize>::new());
    }

    #[test]
    fn rename_information_layout() {
        let b = rename_information(true, &[0x41, 0x42]);
        assert_eq!(b.len(), 26);
        assert_eq!(b[0], 1);
        assert_eq!(u32::from_le_bytes(b[16..20].try_into().unwrap()), 4);
        assert_eq!(&b[20..24], &[0x41, 0, 0x42, 0]);
    }

    #[test]
    fn rename_information_layout_32_bit() {
        // A 32-bit process: RootDirectory at 4, FileNameLength at 8, FileName at 12.
        let b = rename_information_in(4, true, &[0x41, 0x42]);
        assert_eq!(b.len(), 12 + 4 + 2);
        assert_eq!(b[0], 1);
        assert_eq!(&b[4..8], &[0; 4]);
        assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()), 4);
        assert_eq!(&b[12..16], &[0x41, 0, 0x42, 0]);
        assert_eq!(&b[16..18], &[0, 0]);
        // The x64 wrapper is the 8-byte form.
        assert_eq!(rename_information(false, &[1]), rename_information_in(8, false, &[1]));
    }

    #[test]
    fn completion_information_layout() {
        let b = completion_information(8, 0x44, 0x1_0000_0002);
        assert_eq!(b.len(), 16);
        assert_eq!(&b[0..8], &0x44u64.to_le_bytes());
        assert_eq!(&b[8..16], &0x1_0000_0002u64.to_le_bytes());
        let b = completion_information(4, 0x44, 0x1_0000_0002);
        assert_eq!(b.len(), 8);
        assert_eq!(&b[0..4], &0x44u32.to_le_bytes());
        assert_eq!(&b[4..8], &2u32.to_le_bytes());
    }
}
