//! Constants of the file I/O services (ADR 0067).
//!
//! Every value is from a public Windows SDK or WDK header, named as there:
//! `winnt.h` (`FILE_*` access rights, `FILE_SHARE_*`, `FILE_ATTRIBUTE_*`),
//! `wdm.h` / `ntifs.h` (`FILE_SUPERSEDE` ... `FILE_OVERWRITE_IF`, the
//! `FILE_*` create options, `FILE_INFORMATION_CLASS`,
//! `FS_INFORMATION_CLASS`, the `IO_STATUS_BLOCK.Information` values,
//! `FILE_DEVICE_*`) and `ntstatus.h` (the `STATUS_*` values below). Their
//! meanings are from Microsoft Learn: "NtCreateFile", "ZwReadFile",
//! "ZwWriteFile", "ZwQueryInformationFile", "ZwSetInformationFile",
//! "FILE_INFORMATION_CLASS", "ZwQueryDirectoryFile", "ZwLockFile",
//! "ZwUnlockFile", "ZwQueryVolumeInformationFile", "File Attribute
//! Constants", "Creating and Opening Files".

use poly_ntobj::NtStatus;

// ----- access rights (`winnt.h`; Learn: "File Access Rights Constants")

pub const FILE_READ_DATA: u32 = 0x0001;
pub const FILE_LIST_DIRECTORY: u32 = 0x0001;
pub const FILE_WRITE_DATA: u32 = 0x0002;
pub const FILE_ADD_FILE: u32 = 0x0002;
pub const FILE_APPEND_DATA: u32 = 0x0004;
pub const FILE_ADD_SUBDIRECTORY: u32 = 0x0004;
pub const FILE_READ_EA: u32 = 0x0008;
pub const FILE_WRITE_EA: u32 = 0x0010;
pub const FILE_EXECUTE: u32 = 0x0020;
pub const FILE_TRAVERSE: u32 = 0x0020;
pub const FILE_DELETE_CHILD: u32 = 0x0040;
pub const FILE_READ_ATTRIBUTES: u32 = 0x0080;
pub const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
pub const DELETE: u32 = 0x0001_0000;
pub const READ_CONTROL: u32 = 0x0002_0000;
pub const SYNCHRONIZE: u32 = 0x0010_0000;
pub const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
pub const GENERIC_ALL: u32 = 0x1000_0000;
pub const GENERIC_EXECUTE: u32 = 0x2000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const GENERIC_READ: u32 = 0x8000_0000;
pub use poly_ntobj::access::{FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE};

// ----- share access (`winnt.h`)

pub const FILE_SHARE_READ: u32 = 0x1;
pub const FILE_SHARE_WRITE: u32 = 0x2;
pub const FILE_SHARE_DELETE: u32 = 0x4;
pub const FILE_SHARE_VALID_FLAGS: u32 = 0x7;

// ----- create dispositions (`wdm.h`)

pub const FILE_SUPERSEDE: u32 = 0;
pub const FILE_OPEN: u32 = 1;
pub const FILE_CREATE: u32 = 2;
pub const FILE_OPEN_IF: u32 = 3;
pub const FILE_OVERWRITE: u32 = 4;
pub const FILE_OVERWRITE_IF: u32 = 5;
pub const FILE_MAXIMUM_DISPOSITION: u32 = 5;

// ----- create options (`wdm.h`)

pub const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
pub const FILE_WRITE_THROUGH: u32 = 0x0000_0002;
pub const FILE_SEQUENTIAL_ONLY: u32 = 0x0000_0004;
pub const FILE_NO_INTERMEDIATE_BUFFERING: u32 = 0x0000_0008;
pub const FILE_SYNCHRONOUS_IO_ALERT: u32 = 0x0000_0010;
pub const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
pub const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
pub const FILE_CREATE_TREE_CONNECTION: u32 = 0x0000_0080;
pub const FILE_COMPLETE_IF_OPLOCKED: u32 = 0x0000_0100;
pub const FILE_NO_EA_KNOWLEDGE: u32 = 0x0000_0200;
pub const FILE_OPEN_REMOTE_INSTANCE: u32 = 0x0000_0400;
pub const FILE_RANDOM_ACCESS: u32 = 0x0000_0800;
pub const FILE_DELETE_ON_CLOSE: u32 = 0x0000_1000;
pub const FILE_OPEN_BY_FILE_ID: u32 = 0x0000_2000;
pub const FILE_OPEN_FOR_BACKUP_INTENT: u32 = 0x0000_4000;
pub const FILE_NO_COMPRESSION: u32 = 0x0000_8000;
pub const FILE_OPEN_REQUIRING_OPLOCK: u32 = 0x0001_0000;
pub const FILE_RESERVE_OPFILTER: u32 = 0x0010_0000;
pub const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
pub const FILE_OPEN_NO_RECALL: u32 = 0x0040_0000;
pub const FILE_OPEN_FOR_FREE_SPACE_QUERY: u32 = 0x0080_0000;
/// Every option bit `wdm.h` defines (`FILE_VALID_OPTION_FLAGS`).
pub const FILE_VALID_OPTION_FLAGS: u32 = 0x00FF_FFFF;

// ----- IO_STATUS_BLOCK.Information after a create (`wdm.h`)

pub const FILE_SUPERSEDED: u64 = 0;
pub const FILE_OPENED: u64 = 1;
pub const FILE_CREATED: u64 = 2;
pub const FILE_OVERWRITTEN: u64 = 3;
pub const FILE_EXISTS: u64 = 4;
pub const FILE_DOES_NOT_EXIST: u64 = 5;

// ----- special ByteOffset values (`wdm.h`: `FILE_WRITE_TO_END_OF_FILE`,
// `FILE_USE_FILE_POINTER_POSITION`, a LARGE_INTEGER with HighPart -1)

pub const FILE_WRITE_TO_END_OF_FILE: i64 = -1;
pub const FILE_USE_FILE_POINTER_POSITION: i64 = -2;

// ----- file attributes (`winnt.h`; Learn: "File Attribute Constants")

pub const FILE_ATTRIBUTE_READONLY: u32 = 0x0000_0001;
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
pub const FILE_ATTRIBUTE_SYSTEM: u32 = 0x0000_0004;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
pub const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x0000_0020;
pub const FILE_ATTRIBUTE_DEVICE: u32 = 0x0000_0040;
pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
pub const FILE_ATTRIBUTE_TEMPORARY: u32 = 0x0000_0100;
pub const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x0000_0200;
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
pub const FILE_ATTRIBUTE_COMPRESSED: u32 = 0x0000_0800;
pub const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
pub const FILE_ATTRIBUTE_NOT_CONTENT_INDEXED: u32 = 0x0000_2000;
pub const FILE_ATTRIBUTE_ENCRYPTED: u32 = 0x0000_4000;
/// `FILE_ATTRIBUTE_VALID_FLAGS` (`wdm.h`).
pub const FILE_ATTRIBUTE_VALID_FLAGS: u32 = 0x0000_7FB7;
/// `FILE_ATTRIBUTE_VALID_SET_FLAGS` (`wdm.h`): what `FileBasicInformation`
/// may set.
pub const FILE_ATTRIBUTE_VALID_SET_FLAGS: u32 = 0x0000_31A7;

/// `IO_REPARSE_TAG_SYMLINK` (`winnt.h`): the tag a symbolic link reports.
pub const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;

// ----- FILE_INFORMATION_CLASS (`wdm.h`, `ntifs.h`)

pub const FILE_DIRECTORY_INFORMATION: u32 = 1;
pub const FILE_FULL_DIRECTORY_INFORMATION: u32 = 2;
pub const FILE_BOTH_DIRECTORY_INFORMATION: u32 = 3;
pub const FILE_BASIC_INFORMATION: u32 = 4;
pub const FILE_STANDARD_INFORMATION: u32 = 5;
pub const FILE_INTERNAL_INFORMATION: u32 = 6;
pub const FILE_EA_INFORMATION: u32 = 7;
pub const FILE_ACCESS_INFORMATION: u32 = 8;
pub const FILE_NAME_INFORMATION: u32 = 9;
pub const FILE_RENAME_INFORMATION: u32 = 10;
pub const FILE_LINK_INFORMATION: u32 = 11;
pub const FILE_NAMES_INFORMATION: u32 = 12;
pub const FILE_DISPOSITION_INFORMATION: u32 = 13;
pub const FILE_POSITION_INFORMATION: u32 = 14;
pub const FILE_MODE_INFORMATION: u32 = 16;
pub const FILE_ALIGNMENT_INFORMATION: u32 = 17;
pub const FILE_ALL_INFORMATION: u32 = 18;
pub const FILE_ALLOCATION_INFORMATION: u32 = 19;
pub const FILE_END_OF_FILE_INFORMATION: u32 = 20;
pub const FILE_STREAM_INFORMATION: u32 = 22;
pub const FILE_NETWORK_OPEN_INFORMATION: u32 = 34;
pub const FILE_ATTRIBUTE_TAG_INFORMATION: u32 = 35;
pub const FILE_ID_BOTH_DIRECTORY_INFORMATION: u32 = 37;
pub const FILE_ID_FULL_DIRECTORY_INFORMATION: u32 = 38;
pub const FILE_DISPOSITION_INFORMATION_EX: u32 = 64;

/// `FILE_DISPOSITION_INFORMATION_EX.Flags` (`ntddk.h`).
pub const FILE_DISPOSITION_DELETE: u32 = 0x1;
pub const FILE_DISPOSITION_POSIX_SEMANTICS: u32 = 0x2;
pub const FILE_DISPOSITION_IGNORE_READONLY_ATTRIBUTE: u32 = 0x10;

// ----- structure sizes on x64 (the layouts are in `info.rs`)

pub const IO_STATUS_BLOCK_SIZE: usize = 16;
pub const BASIC_INFO_SIZE: usize = 40;
pub const STANDARD_INFO_SIZE: usize = 24;
pub const NETWORK_OPEN_INFO_SIZE: usize = 56;
/// `FILE_ALL_INFORMATION` up to `NameInformation.FileName`.
pub const ALL_INFO_FIXED: usize = 100;
/// `FILE_NAME_INFORMATION` up to `FileName`.
pub const NAME_INFO_FIXED: usize = 4;
/// `FILE_RENAME_INFORMATION` up to `FileName`.
pub const RENAME_INFO_FIXED: usize = 20;
/// `FILE_STREAM_INFORMATION` (`ntifs.h`) up to `StreamName`.
pub const STREAM_INFO_FIXED: usize = 24;

// ----- FS_INFORMATION_CLASS (`wdm.h`)

pub const FILE_FS_VOLUME_INFORMATION: u32 = 1;
pub const FILE_FS_SIZE_INFORMATION: u32 = 3;
pub const FILE_FS_DEVICE_INFORMATION: u32 = 4;
pub const FILE_FS_ATTRIBUTE_INFORMATION: u32 = 5;
pub const FILE_FS_FULL_SIZE_INFORMATION: u32 = 7;

/// `FILE_DEVICE_DISK` and `FILE_DEVICE_NAMED_PIPE` (`DEVICE_TYPE`, `wdm.h`).
pub const FILE_DEVICE_DISK: u32 = 0x0000_0007;
pub const FILE_DEVICE_NAMED_PIPE: u32 = 0x0000_0011;
/// `FILE_DEVICE_UNKNOWN` (`wdm.h`): an inherited character device.
pub const FILE_DEVICE_UNKNOWN: u32 = 0x0000_0022;

/// `FileSystemAttributes` bits (`winnt.h`): what our volumes report.
pub const FILE_CASE_SENSITIVE_SEARCH: u32 = 0x0000_0001;
pub const FILE_CASE_PRESERVED_NAMES: u32 = 0x0000_0002;
pub const FILE_UNICODE_ON_DISK: u32 = 0x0000_0004;
pub const FILE_SUPPORTS_REPARSE_POINTS: u32 = 0x0000_0080;
pub const FILE_SUPPORTS_HARD_LINKS: u32 = 0x0040_0000;
pub const FILE_READ_ONLY_VOLUME: u32 = 0x0008_0000;

// ----- NTSTATUS values the file services return (`ntstatus.h`) that
// `poly_ntobj::status` does not already define

pub const STATUS_NO_MORE_FILES: NtStatus = NtStatus(0x8000_0006);
pub const STATUS_NO_SUCH_FILE: NtStatus = NtStatus(0xC000_000F);
pub const STATUS_INVALID_DEVICE_REQUEST: NtStatus = NtStatus(0xC000_0010);
pub const STATUS_END_OF_FILE: NtStatus = NtStatus(0xC000_0011);
pub const STATUS_OBJECT_PATH_INVALID: NtStatus = NtStatus(0xC000_0039);
pub const STATUS_SHARING_VIOLATION: NtStatus = NtStatus(0xC000_0043);
pub const STATUS_FILE_LOCK_CONFLICT: NtStatus = NtStatus(0xC000_0054);
pub const STATUS_LOCK_NOT_GRANTED: NtStatus = NtStatus(0xC000_0055);
pub const STATUS_DELETE_PENDING: NtStatus = NtStatus(0xC000_0056);
pub const STATUS_RANGE_NOT_LOCKED: NtStatus = NtStatus(0xC000_007E);
pub const STATUS_DISK_FULL: NtStatus = NtStatus(0xC000_007F);
pub const STATUS_MEDIA_WRITE_PROTECTED: NtStatus = NtStatus(0xC000_00A2);
pub const STATUS_FILE_IS_A_DIRECTORY: NtStatus = NtStatus(0xC000_00BA);
pub const STATUS_NOT_SAME_DEVICE: NtStatus = NtStatus(0xC000_00D4);
pub const STATUS_DIRECTORY_NOT_EMPTY: NtStatus = NtStatus(0xC000_0101);
pub const STATUS_NOT_A_DIRECTORY: NtStatus = NtStatus(0xC000_0103);
pub const STATUS_NAME_TOO_LONG: NtStatus = NtStatus(0xC000_0106);
pub const STATUS_TOO_MANY_OPENED_FILES: NtStatus = NtStatus(0xC000_011F);
pub const STATUS_CANCELLED: NtStatus = NtStatus(0xC000_0120);
pub const STATUS_CANNOT_DELETE: NtStatus = NtStatus(0xC000_0121);
pub const STATUS_PIPE_BROKEN: NtStatus = NtStatus(0xC000_014B);
pub const STATUS_IO_DEVICE_ERROR: NtStatus = NtStatus(0xC000_0185);
pub const STATUS_INVALID_LOCK_RANGE: NtStatus = NtStatus(0xC000_01A1);
pub const STATUS_NOT_FOUND: NtStatus = NtStatus(0xC000_0225);
pub const STATUS_EAS_NOT_SUPPORTED: NtStatus = NtStatus(0xC000_004F);
pub const STATUS_NOT_A_REPARSE_POINT: NtStatus = NtStatus(0xC000_0275);

/// `FSCTL_GET_REPARSE_POINT` (`winioctl.h`: `CTL_CODE(FILE_DEVICE_FILE_SYSTEM,
/// 42, METHOD_BUFFERED, FILE_ANY_ACCESS)`).
pub const FSCTL_GET_REPARSE_POINT: u32 = 0x0009_00A8;
/// `FSCTL_SET_REPARSE_POINT` (`winioctl.h`: `CTL_CODE(FILE_DEVICE_FILE_SYSTEM,
/// 41, METHOD_BUFFERED, FILE_SPECIAL_ACCESS)`, `FILE_SPECIAL_ACCESS` being
/// `FILE_ANY_ACCESS`).
pub const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
/// `REPARSE_DATA_BUFFER` (`ntifs.h`) up to its type-specific part.
pub const REPARSE_HEADER_SIZE: usize = 8;
/// `SYMLINK_FLAG_RELATIVE` (`ntifs.h`).
pub const SYMLINK_FLAG_RELATIVE: u32 = 1;
