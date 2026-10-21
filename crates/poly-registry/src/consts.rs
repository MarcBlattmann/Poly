//! Registry constants, each from a public Windows SDK or WDK header and
//! named as there: `winnt.h` (`REG_*` value types, `REG_OPTION_*`,
//! `REG_CREATED_NEW_KEY`, `REG_NOTIFY_*`, `KEY_*` access rights), `wdm.h`
//! (`KEY_INFORMATION_CLASS`, `KEY_VALUE_INFORMATION_CLASS`,
//! `KEY_SET_INFORMATION_CLASS`) and `ntstatus.h` (the statuses). Meanings
//! are from Microsoft Learn: "ZwCreateKey", "ZwOpenKeyEx", "ZwSetValueKey",
//! "ZwNotifyChangeKey", "KEY_INFORMATION_CLASS",
//! "KEY_VALUE_INFORMATION_CLASS", "Registry Value Types",
//! "RegNotifyChangeKeyValue".

use poly_ntobj::NtStatus;

// ----- value types (`winnt.h`, Learn "Registry Value Types")

pub const REG_NONE: u32 = 0;
pub const REG_SZ: u32 = 1;
pub const REG_EXPAND_SZ: u32 = 2;
pub const REG_BINARY: u32 = 3;
pub const REG_DWORD: u32 = 4;
pub const REG_DWORD_LITTLE_ENDIAN: u32 = 4;
pub const REG_DWORD_BIG_ENDIAN: u32 = 5;
pub const REG_LINK: u32 = 6;
pub const REG_MULTI_SZ: u32 = 7;
pub const REG_RESOURCE_LIST: u32 = 8;
pub const REG_FULL_RESOURCE_DESCRIPTOR: u32 = 9;
pub const REG_RESOURCE_REQUIREMENTS_LIST: u32 = 10;
pub const REG_QWORD: u32 = 11;
pub const REG_QWORD_LITTLE_ENDIAN: u32 = 11;

// ----- create and open options (`winnt.h`, Learn "ZwCreateKey", "ZwOpenKeyEx")

pub const REG_OPTION_NON_VOLATILE: u32 = 0x0000_0000;
pub const REG_OPTION_VOLATILE: u32 = 0x0000_0001;
pub const REG_OPTION_CREATE_LINK: u32 = 0x0000_0002;
pub const REG_OPTION_BACKUP_RESTORE: u32 = 0x0000_0004;
pub const REG_OPTION_OPEN_LINK: u32 = 0x0000_0008;
/// `REG_OPTION_DONT_VIRTUALIZE` (`winnt.h`): accepted and ignored (no
/// registry virtualization in Poly).
pub const REG_OPTION_DONT_VIRTUALIZE: u32 = 0x0000_0010;

/// Every `CreateOptions` bit `NtCreateKey` accepts.
pub const REG_LEGAL_CREATE_OPTIONS: u32 = REG_OPTION_VOLATILE
    | REG_OPTION_CREATE_LINK
    | REG_OPTION_BACKUP_RESTORE
    | REG_OPTION_OPEN_LINK
    | REG_OPTION_DONT_VIRTUALIZE;
/// Every `OpenOptions` bit `NtOpenKeyEx` accepts (Learn: "ZwOpenKeyEx":
/// `REG_OPTION_OPEN_LINK`, `REG_OPTION_BACKUP_RESTORE`).
pub const REG_LEGAL_OPEN_OPTIONS: u32 = REG_OPTION_OPEN_LINK | REG_OPTION_BACKUP_RESTORE;

// ----- dispositions (`winnt.h`)

pub const REG_CREATED_NEW_KEY: u32 = 1;
pub const REG_OPENED_EXISTING_KEY: u32 = 2;

// ----- notification filters (`winnt.h`, Learn "ZwNotifyChangeKey")

pub const REG_NOTIFY_CHANGE_NAME: u32 = 0x0000_0001;
pub const REG_NOTIFY_CHANGE_ATTRIBUTES: u32 = 0x0000_0002;
pub const REG_NOTIFY_CHANGE_LAST_SET: u32 = 0x0000_0004;
pub const REG_NOTIFY_CHANGE_SECURITY: u32 = 0x0000_0008;
/// Learn: "RegNotifyChangeKeyValue": the registration is not tied to the
/// calling thread. Poly never ties it to a thread, so the flag changes
/// nothing.
pub const REG_NOTIFY_THREAD_AGNOSTIC: u32 = 0x1000_0000;
pub const REG_LEGAL_CHANGE_FILTER: u32 = REG_NOTIFY_CHANGE_NAME
    | REG_NOTIFY_CHANGE_ATTRIBUTES
    | REG_NOTIFY_CHANGE_LAST_SET
    | REG_NOTIFY_CHANGE_SECURITY
    | REG_NOTIFY_THREAD_AGNOSTIC;

// ----- key access rights (`winnt.h`, Learn "ZwCreateKey")

pub const KEY_QUERY_VALUE: u32 = 0x0001;
pub const KEY_SET_VALUE: u32 = 0x0002;
pub const KEY_CREATE_SUB_KEY: u32 = 0x0004;
pub const KEY_ENUMERATE_SUB_KEYS: u32 = 0x0008;
pub const KEY_NOTIFY: u32 = 0x0010;
pub const KEY_CREATE_LINK: u32 = 0x0020;
pub const KEY_WOW64_64KEY: u32 = 0x0100;
pub const KEY_WOW64_32KEY: u32 = 0x0200;
/// `KEY_WOW64_RES`: the two view-selection bits, which are not access
/// rights and are taken off before the access check.
pub const KEY_WOW64_RES: u32 = 0x0300;
pub use poly_ntobj::access::{KEY_ALL_ACCESS, KEY_EXECUTE, KEY_READ, KEY_WRITE};
/// `DELETE` (`winnt.h`), needed by `NtDeleteKey`.
pub const DELETE: u32 = 0x0001_0000;

// ----- information classes (`wdm.h`)

/// `KEY_INFORMATION_CLASS`.
pub const KEY_BASIC_INFORMATION: u32 = 0;
pub const KEY_NODE_INFORMATION: u32 = 1;
pub const KEY_FULL_INFORMATION: u32 = 2;
pub const KEY_NAME_INFORMATION: u32 = 3;
pub const KEY_CACHED_INFORMATION: u32 = 4;
pub const KEY_FLAGS_INFORMATION: u32 = 5;
pub const KEY_VIRTUALIZATION_INFORMATION: u32 = 6;
pub const KEY_HANDLE_TAGS_INFORMATION: u32 = 7;
pub const KEY_TRUST_INFORMATION: u32 = 8;
pub const KEY_LAYER_INFORMATION: u32 = 9;

/// `KEY_VALUE_INFORMATION_CLASS`.
pub const KEY_VALUE_BASIC_INFORMATION: u32 = 0;
pub const KEY_VALUE_FULL_INFORMATION: u32 = 1;
pub const KEY_VALUE_PARTIAL_INFORMATION: u32 = 2;
pub const KEY_VALUE_FULL_INFORMATION_ALIGN64: u32 = 3;
pub const KEY_VALUE_PARTIAL_INFORMATION_ALIGN64: u32 = 4;
pub const KEY_VALUE_LAYER_INFORMATION: u32 = 5;

/// `KEY_SET_INFORMATION_CLASS`: `KeyWriteTimeInformation`, the only class
/// Learn documents (the others are "Reserved for system use").
pub const KEY_WRITE_TIME_INFORMATION: u32 = 0;

// ----- statuses (`ntstatus.h`) the object manager crate does not define

pub const STATUS_DATATYPE_MISALIGNMENT: NtStatus = NtStatus(0x8000_0002);
pub const STATUS_NOTIFY_CLEANUP: NtStatus = NtStatus(0x0000_010B);
pub const STATUS_NOTIFY_ENUM_DIR: NtStatus = NtStatus(0x0000_010C);
pub const STATUS_CANNOT_DELETE: NtStatus = NtStatus(0xC000_0121);
pub const STATUS_REGISTRY_CORRUPT: NtStatus = NtStatus(0xC000_014C);
pub const STATUS_REGISTRY_IO_FAILED: NtStatus = NtStatus(0xC000_014D);
pub const STATUS_KEY_DELETED: NtStatus = NtStatus(0xC000_017C);
pub const STATUS_KEY_HAS_CHILDREN: NtStatus = NtStatus(0xC000_0180);
pub const STATUS_CHILD_MUST_BE_VOLATILE: NtStatus = NtStatus(0xC000_0181);

// ----- limits (Learn: "Registry Element Size Limits")

/// Longest key name component, in characters ("Key name: 255
/// characters"; Poly applies it per component, to be confirmed by the spec
/// agent against real Windows).
pub const MAX_KEY_NAME_CHARS: usize = 255;
/// Longest value name, in characters ("Value name: 16,383 characters").
pub const MAX_VALUE_NAME_CHARS: usize = 16_383;
/// Deepest key below `\REGISTRY` ("A registry tree can be 512 levels
/// deep").
pub const MAX_DEPTH: usize = 512;

/// The value that holds a symbolic-link key's target (Learn:
/// "RegCreateKeyExW", `REG_OPTION_CREATE_LINK`: "The target path is
/// assigned to the L"SymbolicLinkValue" value of the key").
pub const SYMBOLIC_LINK_VALUE: &str = "SymbolicLinkValue";

/// FILETIME of the Unix epoch: 100-nanosecond intervals from 1601-01-01
/// to 1970-01-01 (Learn: "FILETIME" counts from 1601).
pub const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;
