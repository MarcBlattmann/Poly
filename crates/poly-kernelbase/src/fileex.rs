//! Files, second part (Chrome batch 2, ADR 0155): the argument rules and
//! structure conversions behind `FindFirstFileExW`, `GetFileAttributesExW`,
//! `GetFileInformationByHandleEx`, `SetFileInformationByHandle`,
//! `LockFileEx`/`UnlockFileEx`, `ReplaceFileW`, `GetFinalPathNameByHandleW`
//! and `GetLongPathNameW`.
//!
//! Sources (clean room): Learn's pages for each function, "FILE_INFO_BY_HANDLE_CLASS"
//! (`minwinbase.h`), "FINDEX_INFO_LEVELS", "FINDEX_SEARCH_OPS",
//! `FILE_BASIC_INFO`, `FILE_STANDARD_INFO`, `FILE_NAME_INFO`,
//! `FILE_RENAME_INFO`, `FILE_DISPOSITION_INFO(_EX)`, `FILE_COMPRESSION_INFO`,
//! `FILE_ID_INFO`, `WIN32_FILE_ATTRIBUTE_DATA` and the NT structures of
//! `ntifs.h` that have the same layout.

use crate::error::*;
use alloc::vec::Vec;

/// `ERROR_UNABLE_TO_REMOVE_REPLACED`, `ERROR_UNABLE_TO_MOVE_REPLACEMENT`,
/// `ERROR_UNABLE_TO_MOVE_REPLACEMENT_2` (`winerror.h`).
pub const ERROR_UNABLE_TO_REMOVE_REPLACED: u32 = 1175;
pub const ERROR_UNABLE_TO_MOVE_REPLACEMENT: u32 = 1176;
pub const ERROR_UNABLE_TO_MOVE_REPLACEMENT_2: u32 = 1177;

// ----- FindFirstFileExW ---------------------------------------------------------------------

/// `FindExInfoStandard`, `FindExInfoBasic` (`minwinbase.h`).
pub const FIND_EX_INFO_STANDARD: u32 = 0;
pub const FIND_EX_INFO_BASIC: u32 = 1;
/// `FindExSearchNameMatch`, `FindExSearchLimitToDirectories`,
/// `FindExSearchLimitToDevices`.
pub const FIND_EX_SEARCH_NAME_MATCH: u32 = 0;
pub const FIND_EX_SEARCH_LIMIT_TO_DIRECTORIES: u32 = 1;
pub const FIND_EX_SEARCH_LIMIT_TO_DEVICES: u32 = 2;
/// `FIND_FIRST_EX_CASE_SENSITIVE`, `FIND_FIRST_EX_LARGE_FETCH`,
/// `FIND_FIRST_EX_ON_DISK_ENTRIES_ONLY`.
pub const FIND_FIRST_EX_CASE_SENSITIVE: u32 = 1;
pub const FIND_FIRST_EX_LARGE_FETCH: u32 = 2;
pub const FIND_FIRST_EX_ON_DISK_ENTRIES_ONLY: u32 = 4;

/// What `FindFirstFileExW`'s arguments ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindEx {
    /// `FindExInfoBasic`: `cAlternateFileName` stays empty.
    pub basic: bool,
    /// `FIND_FIRST_EX_CASE_SENSITIVE`.
    pub case_sensitive: bool,
}

/// Check `FindFirstFileExW`'s arguments. Learn: `fInfoLevelId` is one of
/// the two levels; `fSearchOp` `FindExSearchNameMatch` (the filter must be
/// NULL) or `FindExSearchLimitToDirectories` (a hint a file system may
/// ignore, which Poly does: the caller checks the attributes); devices are
/// not supported (`ERROR_NOT_SUPPORTED`); other flags are invalid.
pub fn find_ex_args(info_level: u32, search_op: u32, filter_null: bool, flags: u32) -> Result<FindEx, u32> {
    if info_level > FIND_EX_INFO_BASIC || flags & !7 != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    match search_op {
        FIND_EX_SEARCH_NAME_MATCH => {
            if !filter_null {
                return Err(ERROR_INVALID_PARAMETER);
            }
        }
        FIND_EX_SEARCH_LIMIT_TO_DIRECTORIES => {}
        FIND_EX_SEARCH_LIMIT_TO_DEVICES => return Err(ERROR_NOT_SUPPORTED),
        _ => return Err(ERROR_INVALID_PARAMETER),
    }
    Ok(FindEx { basic: info_level == FIND_EX_INFO_BASIC, case_sensitive: flags & FIND_FIRST_EX_CASE_SENSITIVE != 0 })
}

/// `FindExInfoBasic`: no short name in a `WIN32_FIND_DATAW`.
pub fn drop_alternate_name(data: &mut [u8; crate::file::find_data::SIZE]) {
    data[crate::file::find_data::ALTERNATE_FILE_NAME..].fill(0);
}

// ----- GetFileAttributesExW -----------------------------------------------------------------

/// `GetFileExInfoStandard` (`minwinbase.h`), the only level.
pub const GET_FILE_EX_INFO_STANDARD: u32 = 0;
/// `WIN32_FILE_ATTRIBUTE_DATA` (`fileapi.h`): `dwFileAttributes` 0, the
/// three times at 4, 12, 20, `nFileSizeHigh` 28, `nFileSizeLow` 32.
pub const FILE_ATTRIBUTE_DATA_SIZE: usize = 36;

/// `WIN32_FILE_ATTRIBUTE_DATA` from `FILE_NETWORK_OPEN_INFORMATION`
/// (`ntifs.h`: `CreationTime` 0, `LastAccessTime` 8, `LastWriteTime` 16,
/// `ChangeTime` 24, `AllocationSize` 32, `EndOfFile` 40, `FileAttributes`
/// 48).
pub fn attribute_data(net: &[u8; 56]) -> [u8; FILE_ATTRIBUTE_DATA_SIZE] {
    let mut out = [0u8; FILE_ATTRIBUTE_DATA_SIZE];
    out[0..4].copy_from_slice(&net[48..52]);
    out[4..12].copy_from_slice(&net[0..8]);
    out[12..20].copy_from_slice(&net[8..16]);
    out[20..28].copy_from_slice(&net[16..24]);
    out[28..32].copy_from_slice(&net[44..48]);
    out[32..36].copy_from_slice(&net[40..44]);
    out
}

/// The same from `FILE_BASIC_INFORMATION` (times at 0, 8, 16, attributes
/// at 32) and `FILE_STANDARD_INFORMATION` (`EndOfFile` at 8): for a file
/// opened rather than queried by name.
pub fn attribute_data_from_handle(basic: &[u8; 40], standard: &[u8; 24]) -> [u8; FILE_ATTRIBUTE_DATA_SIZE] {
    let mut net = [0u8; 56];
    net[0..24].copy_from_slice(&basic[0..24]);
    net[24..32].copy_from_slice(&basic[24..32]);
    net[32..40].copy_from_slice(&standard[0..8]);
    net[40..48].copy_from_slice(&standard[8..16]);
    net[48..52].copy_from_slice(&basic[32..36]);
    attribute_data(&net)
}

// ----- FILE_INFO_BY_HANDLE_CLASS ---------------------------------------------------------------

/// `FILE_INFO_BY_HANDLE_CLASS` (`minwinbase.h`).
pub mod class {
    pub const FILE_BASIC_INFO: u32 = 0;
    pub const FILE_STANDARD_INFO: u32 = 1;
    pub const FILE_NAME_INFO: u32 = 2;
    pub const FILE_RENAME_INFO: u32 = 3;
    pub const FILE_DISPOSITION_INFO: u32 = 4;
    pub const FILE_ALLOCATION_INFO: u32 = 5;
    pub const FILE_END_OF_FILE_INFO: u32 = 6;
    pub const FILE_STREAM_INFO: u32 = 7;
    pub const FILE_COMPRESSION_INFO: u32 = 8;
    pub const FILE_ATTRIBUTE_TAG_INFO: u32 = 9;
    pub const FILE_ID_BOTH_DIRECTORY_INFO: u32 = 10;
    pub const FILE_ID_BOTH_DIRECTORY_RESTART_INFO: u32 = 11;
    pub const FILE_IO_PRIORITY_HINT_INFO: u32 = 12;
    pub const FILE_REMOTE_PROTOCOL_INFO: u32 = 13;
    pub const FILE_FULL_DIRECTORY_INFO: u32 = 14;
    pub const FILE_FULL_DIRECTORY_RESTART_INFO: u32 = 15;
    pub const FILE_STORAGE_INFO: u32 = 16;
    pub const FILE_ALIGNMENT_INFO: u32 = 17;
    pub const FILE_ID_INFO: u32 = 18;
    pub const FILE_ID_EXTD_DIRECTORY_INFO: u32 = 19;
    pub const FILE_ID_EXTD_DIRECTORY_RESTART_INFO: u32 = 20;
    pub const FILE_DISPOSITION_INFO_EX: u32 = 21;
    pub const FILE_RENAME_INFO_EX: u32 = 22;
    pub const FILE_CASE_SENSITIVE_INFO: u32 = 23;
    pub const FILE_NORMALIZED_NAME_INFO: u32 = 24;
    /// `MaximumFileInfoByHandleClass`.
    pub const MAXIMUM: u32 = 25;
}

/// The NT information classes the answers come from (`ntifs.h`).
pub mod nt {
    pub const FILE_DIRECTORY_INFORMATION_FULL: u32 = 2;
    pub const FILE_BASIC_INFORMATION: u32 = 4;
    pub const FILE_STANDARD_INFORMATION: u32 = 5;
    pub const FILE_INTERNAL_INFORMATION: u32 = 6;
    pub const FILE_NAME_INFORMATION: u32 = 9;
    pub const FILE_RENAME_INFORMATION: u32 = 10;
    pub const FILE_DISPOSITION_INFORMATION: u32 = 13;
    pub const FILE_ALIGNMENT_INFORMATION: u32 = 17;
    pub const FILE_ALLOCATION_INFORMATION: u32 = 19;
    pub const FILE_END_OF_FILE_INFORMATION: u32 = 20;
    pub const FILE_STREAM_INFORMATION: u32 = 22;
    pub const FILE_ATTRIBUTE_TAG_INFORMATION: u32 = 35;
    pub const FILE_ID_BOTH_DIRECTORY_INFORMATION: u32 = 37;
    pub const FILE_DISPOSITION_INFORMATION_EX: u32 = 64;
}

/// How `GetFileInformationByHandleEx` answers a class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryPlan {
    /// The NT class has the same layout: query it into the caller's buffer
    /// (a short buffer gives the NT status's error, `ERROR_BAD_LENGTH` or
    /// `ERROR_MORE_DATA`).
    Direct(u32),
    /// `FILE_COMPRESSION_INFO` from the standard information.
    Compression,
    /// `FILE_ID_INFO` from the internal information and the volume.
    Id,
    /// `FILE_IO_PRIORITY_HINT_INFO`: `IoPriorityHintNormal`.
    IoPriorityHint,
    /// A directory enumeration; `restart` for the `...RestartInfo` classes.
    Directory { nt_class: u32, restart: bool },
}

/// The plan for `class`: `ERROR_INVALID_PARAMETER` for a value outside the
/// enumeration or one that cannot be queried (rename, disposition,
/// allocation, end of file are set-only), `ERROR_NOT_SUPPORTED` for the
/// classes Poly does not answer.
pub fn query_plan(class: u32) -> Result<QueryPlan, u32> {
    use class::*;
    Ok(match class {
        FILE_BASIC_INFO => QueryPlan::Direct(nt::FILE_BASIC_INFORMATION),
        FILE_STANDARD_INFO => QueryPlan::Direct(nt::FILE_STANDARD_INFORMATION),
        FILE_NAME_INFO | FILE_NORMALIZED_NAME_INFO => QueryPlan::Direct(nt::FILE_NAME_INFORMATION),
        FILE_STREAM_INFO => QueryPlan::Direct(nt::FILE_STREAM_INFORMATION),
        FILE_ATTRIBUTE_TAG_INFO => QueryPlan::Direct(nt::FILE_ATTRIBUTE_TAG_INFORMATION),
        FILE_ALIGNMENT_INFO => QueryPlan::Direct(nt::FILE_ALIGNMENT_INFORMATION),
        FILE_COMPRESSION_INFO => QueryPlan::Compression,
        FILE_ID_INFO => QueryPlan::Id,
        FILE_IO_PRIORITY_HINT_INFO => QueryPlan::IoPriorityHint,
        FILE_ID_BOTH_DIRECTORY_INFO => {
            QueryPlan::Directory { nt_class: nt::FILE_ID_BOTH_DIRECTORY_INFORMATION, restart: false }
        }
        FILE_ID_BOTH_DIRECTORY_RESTART_INFO => {
            QueryPlan::Directory { nt_class: nt::FILE_ID_BOTH_DIRECTORY_INFORMATION, restart: true }
        }
        FILE_FULL_DIRECTORY_INFO => {
            QueryPlan::Directory { nt_class: nt::FILE_DIRECTORY_INFORMATION_FULL, restart: false }
        }
        FILE_FULL_DIRECTORY_RESTART_INFO => {
            QueryPlan::Directory { nt_class: nt::FILE_DIRECTORY_INFORMATION_FULL, restart: true }
        }
        FILE_RENAME_INFO
        | FILE_DISPOSITION_INFO
        | FILE_ALLOCATION_INFO
        | FILE_END_OF_FILE_INFO
        | FILE_DISPOSITION_INFO_EX
        | FILE_RENAME_INFO_EX => return Err(ERROR_INVALID_PARAMETER),
        FILE_REMOTE_PROTOCOL_INFO
        | FILE_STORAGE_INFO
        | FILE_ID_EXTD_DIRECTORY_INFO
        | FILE_ID_EXTD_DIRECTORY_RESTART_INFO
        | FILE_CASE_SENSITIVE_INFO => return Err(ERROR_NOT_SUPPORTED),
        _ => return Err(ERROR_INVALID_PARAMETER),
    })
}

/// `FILE_COMPRESSION_INFO` (16 bytes): `CompressedFileSize` (a
/// `LARGE_INTEGER`), `CompressionFormat` (`COMPRESSION_FORMAT_NONE` 0),
/// the unit, chunk and cluster shifts and 3 reserved bytes. Poly's files are
/// never compressed, so the compressed size is the end of file
/// (`FILE_STANDARD_INFORMATION.EndOfFile` at 8).
pub fn compression_info(standard: &[u8; 24]) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(&standard[8..16]);
    out
}

/// `FILE_ID_INFO` (24 bytes): `VolumeSerialNumber` (a `ULONGLONG`) and the
/// `FILE_ID_128` (16 bytes; the 64-bit index in the low half).
pub fn id_info(volume_serial: u64, index: u64) -> [u8; 24] {
    let mut out = [0u8; 24];
    out[0..8].copy_from_slice(&volume_serial.to_le_bytes());
    out[8..16].copy_from_slice(&index.to_le_bytes());
    out
}

/// `FILE_IO_PRIORITY_HINT_INFO` (4 bytes): `PriorityHint`. The hints are
/// `IoPriorityHintVeryLow` 0, `IoPriorityHintLow` 1, `IoPriorityHintNormal`
/// 2 (`MaximumIoPriorityHintType` 3 is not a hint).
pub const IO_PRIORITY_HINT_NORMAL: u32 = 2;

/// Check a caller's output size for the structure-class `plan` answers
/// itself; `ERROR_BAD_LENGTH` when it is too small.
pub fn check_out_size(plan: QueryPlan, size: usize) -> Result<(), u32> {
    let need = match plan {
        QueryPlan::Compression => 16,
        QueryPlan::Id => 24,
        QueryPlan::IoPriorityHint => 4,
        _ => 0,
    };
    if size < need { Err(ERROR_BAD_LENGTH) } else { Ok(()) }
}

/// What `SetFileInformationByHandle` does for a class.
#[derive(Debug, PartialEq, Eq)]
pub enum SetPlan {
    /// `NtSetInformationFile(nt_class, bytes)`.
    Nt { nt_class: u32, bytes: Vec<u8> },
    /// Accepted, nothing to do (the I/O priority hint).
    Nothing,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// `FILE_RENAME_INFO` (`minwinbase.h`, x64): `ReplaceIfExists` (a `BOOLEAN`)
/// or `Flags` (a `DWORD`) at 0, `RootDirectory` at 8, `FileNameLength` (in
/// bytes) at 16, `FileName` at 20. `FILE_RENAME_FLAG_REPLACE_IF_EXISTS` is
/// bit 0 of the flags; the other flags (`POSIX_SEMANTICS` 2, ...) are
/// accepted and do not change a rename here. The result is the NT
/// `FILE_RENAME_INFORMATION` (same layout, the name an NT path): `convert`
/// makes one of the Win32 name unless a `RootDirectory` is given (then the
/// name is relative to it, as NT takes it).
pub fn rename_info(
    buf: &[u8],
    extended: bool,
    convert: &dyn Fn(&[u16]) -> Result<Vec<u16>, u32>,
) -> Result<Vec<u8>, u32> {
    let len = u32_at(buf, 16).ok_or(ERROR_BAD_LENGTH)? as usize;
    let name = buf.get(20..20 + len).ok_or(ERROR_BAD_LENGTH)?;
    if len == 0 || len % 2 != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let replace = if extended { buf[0] & 1 != 0 } else { buf[0] != 0 };
    let root = buf.get(8..16).ok_or(ERROR_BAD_LENGTH)?;
    let units: Vec<u16> = name.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let has_root = root.iter().any(|&b| b != 0);
    let target = if has_root { units } else { convert(&units)? };
    let mut out = crate::file::rename_information(replace, &target);
    out[8..16].copy_from_slice(root);
    Ok(out)
}

/// The plan for `SetFileInformationByHandle(class, buf)`. A buffer shorter
/// than the class's structure is `ERROR_BAD_LENGTH`; a class that cannot be
/// set (the query-only ones) `ERROR_INVALID_PARAMETER`.
pub fn set_plan(class: u32, buf: &[u8], convert: &dyn Fn(&[u16]) -> Result<Vec<u16>, u32>) -> Result<SetPlan, u32> {
    use class::*;
    let take = |n: usize| buf.get(..n).map(<[u8]>::to_vec).ok_or(ERROR_BAD_LENGTH);
    Ok(match class {
        FILE_BASIC_INFO => SetPlan::Nt { nt_class: nt::FILE_BASIC_INFORMATION, bytes: take(40)? },
        FILE_DISPOSITION_INFO => SetPlan::Nt { nt_class: nt::FILE_DISPOSITION_INFORMATION, bytes: take(1)? },
        FILE_DISPOSITION_INFO_EX => SetPlan::Nt { nt_class: nt::FILE_DISPOSITION_INFORMATION_EX, bytes: take(4)? },
        FILE_ALLOCATION_INFO => SetPlan::Nt { nt_class: nt::FILE_ALLOCATION_INFORMATION, bytes: take(8)? },
        FILE_END_OF_FILE_INFO => SetPlan::Nt { nt_class: nt::FILE_END_OF_FILE_INFORMATION, bytes: take(8)? },
        FILE_RENAME_INFO => {
            SetPlan::Nt { nt_class: nt::FILE_RENAME_INFORMATION, bytes: rename_info(buf, false, convert)? }
        }
        FILE_RENAME_INFO_EX => {
            SetPlan::Nt { nt_class: nt::FILE_RENAME_INFORMATION, bytes: rename_info(buf, true, convert)? }
        }
        FILE_IO_PRIORITY_HINT_INFO => {
            if u32_at(buf, 0).ok_or(ERROR_BAD_LENGTH)? >= 3 {
                return Err(ERROR_INVALID_PARAMETER);
            }
            SetPlan::Nothing
        }
        c if c < MAXIMUM => return Err(ERROR_INVALID_PARAMETER),
        _ => return Err(ERROR_INVALID_PARAMETER),
    })
}

// ----- LockFileEx / UnlockFileEx --------------------------------------------------------------

/// `LOCKFILE_FAIL_IMMEDIATELY`, `LOCKFILE_EXCLUSIVE_LOCK` (`fileapi.h`).
pub const LOCKFILE_FAIL_IMMEDIATELY: u32 = 1;
pub const LOCKFILE_EXCLUSIVE_LOCK: u32 = 2;

/// The range `LockFileEx` / `UnlockFileEx` take: `dwReserved` must be 0,
/// the flags only the two above (`UnlockFileEx` takes none), and the
/// `OVERLAPPED` (the offset) is required (`ERROR_INVALID_PARAMETER`).
pub fn lock_range(
    flags: u32,
    reserved: u32,
    len_low: u32,
    len_high: u32,
    offset: Option<(u32, u32)>,
    allowed_flags: u32,
) -> Result<(i64, i64), u32> {
    let (Some((lo, hi)), true) = (offset, reserved == 0 && flags & !allowed_flags == 0) else {
        return Err(ERROR_INVALID_PARAMETER);
    };
    Ok((((u64::from(hi) << 32) | u64::from(lo)) as i64, ((u64::from(len_high) << 32) | u64::from(len_low)) as i64))
}

// ----- ReplaceFileW ---------------------------------------------------------------------------

/// `REPLACEFILE_WRITE_THROUGH`, `REPLACEFILE_IGNORE_MERGE_ERRORS`,
/// `REPLACEFILE_IGNORE_ACL_ERRORS` (`winbase.h`).
pub const REPLACEFILE_WRITE_THROUGH: u32 = 1;
pub const REPLACEFILE_IGNORE_MERGE_ERRORS: u32 = 2;
pub const REPLACEFILE_IGNORE_ACL_ERRORS: u32 = 4;

/// Check `ReplaceFileW`'s flags and reserved arguments.
pub fn replace_args(flags: u32, exclude_null: bool, reserved_null: bool) -> Result<(), u32> {
    if flags & !7 != 0 || !exclude_null || !reserved_null { Err(ERROR_INVALID_PARAMETER) } else { Ok(()) }
}

/// The error each failing step of a replacement reports (Learn,
/// "ReplaceFileW", Return value): with a backup the replaced file is first
/// renamed to the backup name (`ERROR_UNABLE_TO_REMOVE_REPLACED` when that
/// fails: both files keep their names); the attributes and creation time of
/// the replaced file are merged into the replacement
/// (`ERROR_UNABLE_TO_MOVE_REPLACEMENT_2` when that fails, unless
/// `REPLACEFILE_IGNORE_MERGE_ERRORS`: the replacement still has its name);
/// then the replacement takes the name of the replaced file
/// (`ERROR_UNABLE_TO_MOVE_REPLACEMENT` when that fails; Poly renames over
/// the old file in one step and undoes the backup, so the names are as they
/// were).
pub fn replace_error(step: ReplaceStep) -> u32 {
    match step {
        ReplaceStep::RemoveReplaced => ERROR_UNABLE_TO_REMOVE_REPLACED,
        ReplaceStep::MergeAttributes => ERROR_UNABLE_TO_MOVE_REPLACEMENT_2,
        ReplaceStep::MoveReplacement => ERROR_UNABLE_TO_MOVE_REPLACEMENT,
    }
}

/// Where `ReplaceFileW` failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplaceStep {
    RemoveReplaced,
    MergeAttributes,
    MoveReplacement,
}

/// The `FILE_BASIC_INFORMATION` that gives the replacement the replaced
/// file's creation time and attributes (the other times and the change
/// time stay: 0 leaves a field alone). The attributes are the ones a file
/// can be given (`FILE_ATTRIBUTE_VALID_SET_FLAGS`); 0 would be "no change".
pub fn merge_basic(replaced: &[u8; 40]) -> [u8; 40] {
    let mut out = [0u8; 40];
    out[0..8].copy_from_slice(&replaced[0..8]);
    let attrs = u32::from_le_bytes(replaced[32..36].try_into().unwrap()) & crate::file::FILE_ATTRIBUTE_VALID_SET_FLAGS;
    out[32..36].copy_from_slice(&attrs.to_le_bytes());
    out
}

// ----- GetFinalPathNameByHandleW ---------------------------------------------------------------

/// `VOLUME_NAME_DOS` 0, `VOLUME_NAME_GUID` 1, `VOLUME_NAME_NT` 2,
/// `VOLUME_NAME_NONE` 4; `FILE_NAME_NORMALIZED` 0, `FILE_NAME_OPENED` 8.
pub const VOLUME_NAME_DOS: u32 = 0;
pub const VOLUME_NAME_GUID: u32 = 1;
pub const VOLUME_NAME_NT: u32 = 2;
pub const VOLUME_NAME_NONE: u32 = 4;
pub const FILE_NAME_OPENED: u32 = 8;

/// The volume form `flags` ask for, or `ERROR_INVALID_PARAMETER`.
pub fn final_path_volume(flags: u32) -> Result<u32, u32> {
    let volume = flags & 7;
    if flags & !(7 | FILE_NAME_OPENED) != 0 || !matches!(volume, 0 | 1 | 2 | 4) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok(volume)
}

/// The final path in the DOS form: `\\?\C:` and the path below the volume
/// (Learn: the string "begins with `\\?\`"), or only that path for
/// `VOLUME_NAME_NONE`.
pub fn final_path(volume: u32, drive: u16, below_volume: &[u16]) -> Vec<u16> {
    let mut out = Vec::new();
    if volume == VOLUME_NAME_DOS {
        out.extend(r"\\?\".encode_utf16());
        out.push(drive);
        out.push(u16::from(b':'));
    }
    if below_volume.first() != Some(&u16::from(b'\\')) {
        out.push(u16::from(b'\\'));
    }
    out.extend_from_slice(below_volume);
    out
}

// ----- GetLongPathNameW ---------------------------------------------------------------------------

/// Expand each component of `path` to the name the directory holds
/// (Learn, "GetLongPathNameW": a short `8.3` name becomes its long form;
/// a name that does not exist fails with `ERROR_FILE_NOT_FOUND` for the last
/// component and `ERROR_PATH_NOT_FOUND` for one before it). `lookup(dir,
/// name)` returns the name `dir` holds for `name` (`None`: no such entry),
/// `dir` being the path up to (not including) the component, as given.
/// The root (`C:\`, `\\?\C:\`, `\\server\share\`), `.` and `..` are kept
/// as they are; a component with a wildcard is `ERROR_INVALID_NAME`.
pub fn long_path(
    path: &[u16],
    lookup: &mut dyn FnMut(&[u16], &[u16]) -> Result<Option<Vec<u16>>, u32>,
) -> Result<Vec<u16>, u32> {
    if path.is_empty() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let sep = |c: u16| c == u16::from(b'\\') || c == u16::from(b'/');
    let root = crate::path::root_len(path);
    let mut out: Vec<u16> = path[..root].to_vec();
    let rest = &path[root..];
    let mut parts: Vec<&[u16]> = Vec::new();
    let mut start = 0;
    for (i, &c) in rest.iter().enumerate() {
        if sep(c) {
            parts.push(&rest[start..i]);
            start = i + 1;
        }
    }
    let tail = &rest[start..];
    let last = parts.len();
    parts.push(tail);
    for (n, comp) in parts.iter().enumerate() {
        let is_last = n == last;
        if comp.is_empty() {
            // `a\\b`, or a trailing separator: kept.
            if !is_last {
                out.push(u16::from(b'\\'));
            }
            continue;
        }
        let dots = comp.iter().all(|&c| c == u16::from(b'.')) && comp.len() <= 2;
        if comp.iter().any(|&c| c == u16::from(b'*') || c == u16::from(b'?')) {
            return Err(ERROR_INVALID_NAME);
        }
        if dots {
            out.extend_from_slice(comp);
        } else {
            match lookup(&out, comp)? {
                Some(found) => out.extend_from_slice(&found),
                None => return Err(if is_last { ERROR_FILE_NOT_FOUND } else { ERROR_PATH_NOT_FOUND }),
            }
        }
        if !is_last {
            out.push(u16::from(b'\\'));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wide;
    extern crate std;

    #[test]
    fn find_ex_arguments() {
        assert_eq!(find_ex_args(1, 0, true, 2), Ok(FindEx { basic: true, case_sensitive: false }));
        assert_eq!(find_ex_args(0, 1, false, 1), Ok(FindEx { basic: false, case_sensitive: true }));
        assert_eq!(find_ex_args(2, 0, true, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(find_ex_args(0, 0, false, 0), Err(ERROR_INVALID_PARAMETER), "filter must be NULL");
        assert_eq!(find_ex_args(0, 2, true, 0), Err(ERROR_NOT_SUPPORTED));
        assert_eq!(find_ex_args(0, 3, true, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(find_ex_args(0, 0, true, 8), Err(ERROR_INVALID_PARAMETER));
        let mut d = [7u8; crate::file::find_data::SIZE];
        drop_alternate_name(&mut d);
        assert_eq!(d[crate::file::find_data::ALTERNATE_FILE_NAME - 1], 7);
        assert!(d[crate::file::find_data::ALTERNATE_FILE_NAME..].iter().all(|&b| b == 0));
    }

    #[test]
    fn attribute_data_layout() {
        let mut net = [0u8; 56];
        net[0..8].copy_from_slice(&1u64.to_le_bytes());
        net[8..16].copy_from_slice(&2u64.to_le_bytes());
        net[16..24].copy_from_slice(&3u64.to_le_bytes());
        net[40..48].copy_from_slice(&0x0000_0005_0000_0007u64.to_le_bytes());
        net[48..52].copy_from_slice(&0x20u32.to_le_bytes());
        let d = attribute_data(&net);
        assert_eq!(u32::from_le_bytes(d[0..4].try_into().unwrap()), 0x20);
        assert_eq!(u64::from_le_bytes(d[4..12].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(d[12..20].try_into().unwrap()), 2);
        assert_eq!(u64::from_le_bytes(d[20..28].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(d[28..32].try_into().unwrap()), 5, "size high");
        assert_eq!(u32::from_le_bytes(d[32..36].try_into().unwrap()), 7, "size low");
        let (mut basic, mut standard) = ([0u8; 40], [0u8; 24]);
        basic[0..8].copy_from_slice(&9u64.to_le_bytes());
        basic[32..36].copy_from_slice(&0x10u32.to_le_bytes());
        standard[8..16].copy_from_slice(&0x0000_0001_0000_0002u64.to_le_bytes());
        let h = attribute_data_from_handle(&basic, &standard);
        assert_eq!(u32::from_le_bytes(h[0..4].try_into().unwrap()), 0x10);
        assert_eq!(u64::from_le_bytes(h[4..12].try_into().unwrap()), 9);
        assert_eq!(u32::from_le_bytes(h[28..32].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(h[32..36].try_into().unwrap()), 2);
    }

    #[test]
    fn query_plans() {
        assert_eq!(query_plan(class::FILE_BASIC_INFO), Ok(QueryPlan::Direct(4)));
        assert_eq!(query_plan(class::FILE_NAME_INFO), Ok(QueryPlan::Direct(9)));
        assert_eq!(query_plan(class::FILE_ID_INFO), Ok(QueryPlan::Id));
        assert_eq!(
            query_plan(class::FILE_ID_BOTH_DIRECTORY_RESTART_INFO),
            Ok(QueryPlan::Directory { nt_class: 37, restart: true })
        );
        assert_eq!(
            query_plan(class::FILE_FULL_DIRECTORY_INFO),
            Ok(QueryPlan::Directory { nt_class: 2, restart: false })
        );
        assert_eq!(query_plan(class::FILE_RENAME_INFO), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(query_plan(class::FILE_STORAGE_INFO), Err(ERROR_NOT_SUPPORTED));
        assert_eq!(query_plan(class::MAXIMUM), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(query_plan(999), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(check_out_size(QueryPlan::Id, 23), Err(ERROR_BAD_LENGTH));
        assert_eq!(check_out_size(QueryPlan::Id, 24), Ok(()));
        let mut standard = [0u8; 24];
        standard[8..16].copy_from_slice(&1234u64.to_le_bytes());
        assert_eq!(&compression_info(&standard)[0..8], &1234u64.to_le_bytes());
        let id = id_info(0xAABB, 0x11);
        assert_eq!(&id[0..8], &0xAABBu64.to_le_bytes());
        assert_eq!(&id[8..16], &0x11u64.to_le_bytes());
        assert_eq!(&id[16..], &[0u8; 8]);
    }

    fn dos_to_nt(w: &[u16]) -> Result<Vec<u16>, u32> {
        let mut v = wide(r"\??\");
        v.extend_from_slice(w);
        Ok(v)
    }

    fn rename_buf(flag: u8, root: u64, name: &str) -> Vec<u8> {
        let n = wide(name);
        let mut b = alloc::vec![0u8; 20 + n.len() * 2];
        b[0] = flag;
        b[8..16].copy_from_slice(&root.to_le_bytes());
        b[16..20].copy_from_slice(&((n.len() * 2) as u32).to_le_bytes());
        for (i, c) in n.iter().enumerate() {
            b[20 + 2 * i..22 + 2 * i].copy_from_slice(&c.to_le_bytes());
        }
        b
    }

    #[test]
    fn set_plans() {
        let nothing = |w: &[u16]| Ok(w.to_vec());
        // Rename: the Win32 name is converted, the replace flag kept.
        let b = rename_buf(1, 0, r"C:\a\b.txt");
        let SetPlan::Nt { nt_class, bytes } = set_plan(class::FILE_RENAME_INFO, &b, &dos_to_nt).unwrap() else {
            panic!()
        };
        assert_eq!(nt_class, 10);
        assert_eq!(bytes[0], 1);
        let len = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
        assert_eq!(len, (4 + 10) * 2);
        let units: Vec<u16> = bytes[20..20 + len].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(units, wide(r"\??\C:\a\b.txt"));
        // A root directory: the name is used as given, the root kept.
        let b = rename_buf(0, 0x44, "x.txt");
        let SetPlan::Nt { bytes, .. } = set_plan(class::FILE_RENAME_INFO, &b, &dos_to_nt).unwrap() else { panic!() };
        assert_eq!(bytes[0], 0);
        assert_eq!(&bytes[8..16], &0x44u64.to_le_bytes());
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 10);
        // Extended rename: flag bit 0 replaces, POSIX semantics (2) alone does not.
        let SetPlan::Nt { bytes, .. } = set_plan(class::FILE_RENAME_INFO_EX, &rename_buf(3, 0, "a"), &nothing).unwrap()
        else {
            panic!()
        };
        assert_eq!(bytes[0], 1);
        let SetPlan::Nt { bytes, .. } = set_plan(class::FILE_RENAME_INFO_EX, &rename_buf(2, 0, "a"), &nothing).unwrap()
        else {
            panic!()
        };
        assert_eq!(bytes[0], 0);
        // Short or odd buffers.
        assert_eq!(set_plan(class::FILE_RENAME_INFO, &[0u8; 10], &nothing), Err(ERROR_BAD_LENGTH));
        let mut odd = rename_buf(0, 0, "ab");
        odd[16] = 3;
        assert_eq!(set_plan(class::FILE_RENAME_INFO, &odd, &nothing), Err(ERROR_INVALID_PARAMETER));
        // The simple classes.
        assert_eq!(
            set_plan(class::FILE_DISPOSITION_INFO, &[1], &nothing),
            Ok(SetPlan::Nt { nt_class: 13, bytes: alloc::vec![1] })
        );
        assert_eq!(
            set_plan(class::FILE_DISPOSITION_INFO_EX, &[3, 0, 0, 0], &nothing),
            Ok(SetPlan::Nt { nt_class: 64, bytes: alloc::vec![3, 0, 0, 0] })
        );
        assert_eq!(set_plan(class::FILE_END_OF_FILE_INFO, &[0; 4], &nothing), Err(ERROR_BAD_LENGTH));
        assert_eq!(set_plan(class::FILE_END_OF_FILE_INFO, &[5, 0, 0, 0, 0, 0, 0, 0], &nothing).is_ok(), true);
        assert_eq!(set_plan(class::FILE_BASIC_INFO, &[0; 40], &nothing).is_ok(), true);
        assert_eq!(set_plan(class::FILE_BASIC_INFO, &[0; 39], &nothing), Err(ERROR_BAD_LENGTH));
        assert_eq!(set_plan(class::FILE_IO_PRIORITY_HINT_INFO, &[1, 0, 0, 0], &nothing), Ok(SetPlan::Nothing));
        assert_eq!(set_plan(class::FILE_IO_PRIORITY_HINT_INFO, &[3, 0, 0, 0], &nothing), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(set_plan(class::FILE_STANDARD_INFO, &[0; 24], &nothing), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn lock_ranges() {
        assert_eq!(lock_range(2, 0, 10, 1, Some((5, 2)), 3), Ok((0x2_0000_0005, 0x1_0000_000A)));
        assert_eq!(lock_range(2, 0, 10, 0, None, 3), Err(ERROR_INVALID_PARAMETER), "OVERLAPPED required");
        assert_eq!(lock_range(2, 1, 10, 0, Some((0, 0)), 3), Err(ERROR_INVALID_PARAMETER), "reserved");
        assert_eq!(lock_range(4, 0, 10, 0, Some((0, 0)), 3), Err(ERROR_INVALID_PARAMETER), "flag");
        assert_eq!(lock_range(1, 0, 10, 0, Some((0, 0)), 0), Err(ERROR_INVALID_PARAMETER), "unlock takes no flags");
    }

    #[test]
    fn replace_rules() {
        assert_eq!(replace_args(7, true, true), Ok(()));
        assert_eq!(replace_args(8, true, true), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(replace_args(0, false, true), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(replace_error(ReplaceStep::RemoveReplaced), 1175);
        assert_eq!(replace_error(ReplaceStep::MoveReplacement), 1176);
        assert_eq!(replace_error(ReplaceStep::MergeAttributes), 1177);
        let mut basic = [0xEEu8; 40];
        basic[32..36].copy_from_slice(&0x0000_2027u32.to_le_bytes());
        let m = merge_basic(&basic);
        assert_eq!(&m[0..8], &[0xEE; 8], "creation time");
        assert_eq!(&m[8..32], &[0u8; 24], "other times unchanged");
        assert_eq!(u32::from_le_bytes(m[32..36].try_into().unwrap()), 0x0000_2027 & 0x31A7);
    }

    #[test]
    fn final_paths() {
        assert_eq!(final_path_volume(0), Ok(0));
        assert_eq!(final_path_volume(8 | 4), Ok(4));
        assert_eq!(final_path_volume(3), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(final_path_volume(16), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(final_path(0, u16::from(b'C'), &wide(r"\Users\a.txt")), wide(r"\\?\C:\Users\a.txt"));
        assert_eq!(final_path(4, u16::from(b'C'), &wide(r"\Users\a.txt")), wide(r"\Users\a.txt"));
        assert_eq!(final_path(0, u16::from(b'C'), &wide(r"\")), wide(r"\\?\C:\"));
    }

    /// A fake directory tree for `long_path`.
    fn fs_lookup(dir: &[u16], name: &[u16]) -> Result<Option<Vec<u16>>, u32> {
        let tree: &[(&str, &str, &str)] = &[
            (r"C:\", "PROGRA~1", "Program Files"),
            (r"C:\", "windows", "Windows"),
            (r"C:\Program Files\", "Chrome", "Chrome"),
            (r"C:\Program Files\Chrome\", "CHROME~1.EXE", "chrome.exe"),
            (r"C:\Program Files\Chrome\", "chrome.exe", "chrome.exe"),
        ];
        let d = std::string::String::from_utf16_lossy(dir);
        let n = std::string::String::from_utf16_lossy(name);
        Ok(tree
            .iter()
            .find(|(td, tn, _)| td.eq_ignore_ascii_case(&d) && tn.eq_ignore_ascii_case(&n))
            .map(|(_, _, long)| wide(long)))
    }

    #[test]
    fn long_paths() {
        let mut l = fs_lookup;
        assert_eq!(
            long_path(&wide(r"C:\PROGRA~1\Chrome\CHROME~1.EXE"), &mut l),
            Ok(wide(r"C:\Program Files\Chrome\chrome.exe"))
        );
        assert_eq!(long_path(&wide(r"c:\WINDOWS"), &mut l), Ok(wide(r"c:\Windows")));
        assert_eq!(long_path(&wide(r"C:\"), &mut l), Ok(wide(r"C:\")));
        assert_eq!(long_path(&wide(r"C:\nope\x"), &mut l), Err(ERROR_PATH_NOT_FOUND));
        assert_eq!(long_path(&wide(r"C:\PROGRA~1\nope"), &mut l), Err(ERROR_FILE_NOT_FOUND));
        assert_eq!(long_path(&wide(r"C:\PROGRA~1\*"), &mut l), Err(ERROR_INVALID_NAME));
        assert_eq!(long_path(&wide(r"C:\PROGRA~1\."), &mut l), Ok(wide(r"C:\Program Files\.")));
        assert_eq!(long_path(&wide(""), &mut l), Err(ERROR_INVALID_PARAMETER));
        // A lookup error is passed on.
        let mut deny = |_: &[u16], _: &[u16]| Err(ERROR_ACCESS_DENIED);
        assert_eq!(long_path(&wide(r"C:\a"), &mut deny), Err(ERROR_ACCESS_DENIED));
    }
}
