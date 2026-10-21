//! `NTSTATUS` values.
//!
//! Every value below is a constant from the public Windows SDK header
//! `ntstatus.h` (`shared/ntstatus.h`), except [`STATUS_SUCCESS`], the
//! `STATUS_WAIT_*` and `STATUS_ABANDONED_WAIT_*` values, which `ntstatus.h`
//! and `winnt.h` both define. The values this crate returns are here, and
//! those the NT personality's other services return (memory, sections,
//! information queries), so there is one table of names.

use core::fmt;

/// An `NTSTATUS`: severity in bits 30–31, then facility and code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct NtStatus(pub u32);

/// What every fallible call in this crate returns: success values travel in
/// `Ok` (some calls also report an informational status, see
/// [`crate::Created`]), failures as the `NTSTATUS` the service returns.
pub type Result<T> = core::result::Result<T, NtStatus>;

impl NtStatus {
    /// `NT_SUCCESS` from `ntdef.h`: success and informational values.
    pub const fn is_success(self) -> bool {
        (self.0 as i32) >= 0
    }

    /// Severity "error" (bits 30–31 both set).
    pub const fn is_error(self) -> bool {
        self.0 >> 30 == 3
    }

    /// The `ntstatus.h` name, for the values this crate knows.
    pub const fn name(self) -> Option<&'static str> {
        Some(match self {
            STATUS_SUCCESS => "STATUS_SUCCESS",
            STATUS_TIMEOUT => "STATUS_TIMEOUT",
            STATUS_ALERTED => "STATUS_ALERTED",
            STATUS_USER_APC => "STATUS_USER_APC",
            STATUS_PENDING => "STATUS_PENDING",
            STATUS_OBJECT_NAME_EXISTS => "STATUS_OBJECT_NAME_EXISTS",
            STATUS_INVALID_HANDLE => "STATUS_INVALID_HANDLE",
            STATUS_INVALID_PARAMETER => "STATUS_INVALID_PARAMETER",
            STATUS_ACCESS_DENIED => "STATUS_ACCESS_DENIED",
            STATUS_OBJECT_TYPE_MISMATCH => "STATUS_OBJECT_TYPE_MISMATCH",
            STATUS_INVALID_PARAMETER_MIX => "STATUS_INVALID_PARAMETER_MIX",
            STATUS_OBJECT_NAME_INVALID => "STATUS_OBJECT_NAME_INVALID",
            STATUS_OBJECT_NAME_NOT_FOUND => "STATUS_OBJECT_NAME_NOT_FOUND",
            STATUS_OBJECT_NAME_COLLISION => "STATUS_OBJECT_NAME_COLLISION",
            STATUS_OBJECT_PATH_NOT_FOUND => "STATUS_OBJECT_PATH_NOT_FOUND",
            STATUS_OBJECT_PATH_SYNTAX_BAD => "STATUS_OBJECT_PATH_SYNTAX_BAD",
            STATUS_MUTANT_NOT_OWNED => "STATUS_MUTANT_NOT_OWNED",
            STATUS_THREAD_IS_TERMINATING => "STATUS_THREAD_IS_TERMINATING",
            STATUS_PROCESS_IS_TERMINATING => "STATUS_PROCESS_IS_TERMINATING",
            STATUS_SUSPEND_COUNT_EXCEEDED => "STATUS_SUSPEND_COUNT_EXCEEDED",
            STATUS_SEMAPHORE_LIMIT_EXCEEDED => "STATUS_SEMAPHORE_LIMIT_EXCEEDED",
            STATUS_PRIVILEGE_NOT_HELD => "STATUS_PRIVILEGE_NOT_HELD",
            STATUS_INSUFFICIENT_RESOURCES => "STATUS_INSUFFICIENT_RESOURCES",
            STATUS_MUTANT_LIMIT_EXCEEDED => "STATUS_MUTANT_LIMIT_EXCEEDED",
            STATUS_HANDLE_NOT_CLOSABLE => "STATUS_HANDLE_NOT_CLOSABLE",
            STATUS_REPARSE_POINT_NOT_RESOLVED => "STATUS_REPARSE_POINT_NOT_RESOLVED",
            STATUS_REPARSE_POINT_ENCOUNTERED => "STATUS_REPARSE_POINT_ENCOUNTERED",
            STATUS_IMAGE_NOT_AT_BASE => "STATUS_IMAGE_NOT_AT_BASE",
            STATUS_MORE_ENTRIES => "STATUS_MORE_ENTRIES",
            STATUS_GUARD_PAGE_VIOLATION => "STATUS_GUARD_PAGE_VIOLATION",
            STATUS_BUFFER_OVERFLOW => "STATUS_BUFFER_OVERFLOW",
            STATUS_NO_MORE_ENTRIES => "STATUS_NO_MORE_ENTRIES",
            STATUS_UNSUCCESSFUL => "STATUS_UNSUCCESSFUL",
            STATUS_NOT_IMPLEMENTED => "STATUS_NOT_IMPLEMENTED",
            STATUS_INVALID_INFO_CLASS => "STATUS_INVALID_INFO_CLASS",
            STATUS_INFO_LENGTH_MISMATCH => "STATUS_INFO_LENGTH_MISMATCH",
            STATUS_ACCESS_VIOLATION => "STATUS_ACCESS_VIOLATION",
            STATUS_NO_MEMORY => "STATUS_NO_MEMORY",
            STATUS_CONFLICTING_ADDRESSES => "STATUS_CONFLICTING_ADDRESSES",
            STATUS_NOT_MAPPED_VIEW => "STATUS_NOT_MAPPED_VIEW",
            STATUS_UNABLE_TO_FREE_VM => "STATUS_UNABLE_TO_FREE_VM",
            STATUS_UNABLE_TO_DELETE_SECTION => "STATUS_UNABLE_TO_DELETE_SECTION",
            STATUS_INVALID_SYSTEM_SERVICE => "STATUS_INVALID_SYSTEM_SERVICE",
            STATUS_INVALID_VIEW_SIZE => "STATUS_INVALID_VIEW_SIZE",
            STATUS_INVALID_FILE_FOR_SECTION => "STATUS_INVALID_FILE_FOR_SECTION",
            STATUS_ALREADY_COMMITTED => "STATUS_ALREADY_COMMITTED",
            STATUS_BUFFER_TOO_SMALL => "STATUS_BUFFER_TOO_SMALL",
            STATUS_NOT_COMMITTED => "STATUS_NOT_COMMITTED",
            STATUS_SECTION_TOO_BIG => "STATUS_SECTION_TOO_BIG",
            STATUS_INVALID_PAGE_PROTECTION => "STATUS_INVALID_PAGE_PROTECTION",
            STATUS_SECTION_NOT_IMAGE => "STATUS_SECTION_NOT_IMAGE",
            STATUS_SECTION_PROTECTION => "STATUS_SECTION_PROTECTION",
            STATUS_INVALID_IMAGE_FORMAT => "STATUS_INVALID_IMAGE_FORMAT",
            STATUS_FREE_VM_NOT_AT_BASE => "STATUS_FREE_VM_NOT_AT_BASE",
            STATUS_MEMORY_NOT_ALLOCATED => "STATUS_MEMORY_NOT_ALLOCATED",
            STATUS_NOT_SUPPORTED => "STATUS_NOT_SUPPORTED",
            STATUS_INVALID_PARAMETER_1 => "STATUS_INVALID_PARAMETER_1",
            STATUS_INVALID_PARAMETER_2 => "STATUS_INVALID_PARAMETER_2",
            STATUS_INVALID_PARAMETER_3 => "STATUS_INVALID_PARAMETER_3",
            STATUS_INVALID_PARAMETER_4 => "STATUS_INVALID_PARAMETER_4",
            STATUS_INVALID_PARAMETER_5 => "STATUS_INVALID_PARAMETER_5",
            STATUS_INVALID_PARAMETER_6 => "STATUS_INVALID_PARAMETER_6",
            STATUS_INVALID_PARAMETER_7 => "STATUS_INVALID_PARAMETER_7",
            STATUS_INVALID_PARAMETER_8 => "STATUS_INVALID_PARAMETER_8",
            STATUS_INVALID_PARAMETER_9 => "STATUS_INVALID_PARAMETER_9",
            STATUS_INVALID_PARAMETER_10 => "STATUS_INVALID_PARAMETER_10",
            STATUS_MAPPED_FILE_SIZE_ZERO => "STATUS_MAPPED_FILE_SIZE_ZERO",
            STATUS_MAPPED_ALIGNMENT => "STATUS_MAPPED_ALIGNMENT",
            NtStatus(v) if v <= STATUS_WAIT_63.0 => "STATUS_WAIT_n",
            NtStatus(v) if v >= STATUS_ABANDONED_WAIT_0.0 && v <= STATUS_ABANDONED_WAIT_63.0 => {
                "STATUS_ABANDONED_WAIT_n"
            }
            _ => return None,
        })
    }

    /// `STATUS_WAIT_0 + index`: wait-any satisfied by the object at `index`.
    pub const fn wait(index: usize) -> NtStatus {
        NtStatus(STATUS_WAIT_0.0 + index as u32)
    }

    /// `STATUS_ABANDONED_WAIT_0 + index`.
    pub const fn abandoned(index: usize) -> NtStatus {
        NtStatus(STATUS_ABANDONED_WAIT_0.0 + index as u32)
    }
}

impl fmt::Debug for NtStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) if n.ends_with("_n") => write!(f, "{}({:#x})", n, self.0),
            Some(n) => f.write_str(n),
            None => write!(f, "NTSTATUS({:#010x})", self.0),
        }
    }
}

impl fmt::Display for NtStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

pub const STATUS_SUCCESS: NtStatus = NtStatus(0x0000_0000);
pub const STATUS_WAIT_0: NtStatus = NtStatus(0x0000_0000);
pub const STATUS_WAIT_63: NtStatus = NtStatus(0x0000_003F);
pub const STATUS_ABANDONED_WAIT_0: NtStatus = NtStatus(0x0000_0080);
pub const STATUS_ABANDONED_WAIT_63: NtStatus = NtStatus(0x0000_00BF);
pub const STATUS_USER_APC: NtStatus = NtStatus(0x0000_00C0);
pub const STATUS_ALERTED: NtStatus = NtStatus(0x0000_0101);
pub const STATUS_TIMEOUT: NtStatus = NtStatus(0x0000_0102);
pub const STATUS_PENDING: NtStatus = NtStatus(0x0000_0103);
pub const STATUS_OBJECT_NAME_EXISTS: NtStatus = NtStatus(0x4000_0000);
pub const STATUS_INVALID_HANDLE: NtStatus = NtStatus(0xC000_0008);
pub const STATUS_INVALID_PARAMETER: NtStatus = NtStatus(0xC000_000D);
pub const STATUS_ACCESS_DENIED: NtStatus = NtStatus(0xC000_0022);
pub const STATUS_OBJECT_TYPE_MISMATCH: NtStatus = NtStatus(0xC000_0024);
pub const STATUS_INVALID_PARAMETER_MIX: NtStatus = NtStatus(0xC000_0030);
pub const STATUS_OBJECT_NAME_INVALID: NtStatus = NtStatus(0xC000_0033);
pub const STATUS_OBJECT_NAME_NOT_FOUND: NtStatus = NtStatus(0xC000_0034);
pub const STATUS_OBJECT_NAME_COLLISION: NtStatus = NtStatus(0xC000_0035);
pub const STATUS_OBJECT_PATH_NOT_FOUND: NtStatus = NtStatus(0xC000_003A);
pub const STATUS_OBJECT_PATH_SYNTAX_BAD: NtStatus = NtStatus(0xC000_003B);
pub const STATUS_THREAD_IS_TERMINATING: NtStatus = NtStatus(0xC000_004B);
pub const STATUS_PROCESS_IS_TERMINATING: NtStatus = NtStatus(0xC000_010A);
pub const STATUS_SUSPEND_COUNT_EXCEEDED: NtStatus = NtStatus(0xC000_004A);
pub const STATUS_MUTANT_NOT_OWNED: NtStatus = NtStatus(0xC000_0046);
pub const STATUS_SEMAPHORE_LIMIT_EXCEEDED: NtStatus = NtStatus(0xC000_0047);
pub const STATUS_PRIVILEGE_NOT_HELD: NtStatus = NtStatus(0xC000_0061);
pub const STATUS_INSUFFICIENT_RESOURCES: NtStatus = NtStatus(0xC000_009A);
pub const STATUS_MUTANT_LIMIT_EXCEEDED: NtStatus = NtStatus(0xC000_0191);
pub const STATUS_HANDLE_NOT_CLOSABLE: NtStatus = NtStatus(0xC000_0235);
pub const STATUS_REPARSE_POINT_NOT_RESOLVED: NtStatus = NtStatus(0xC000_0280);
pub const STATUS_REPARSE_POINT_ENCOUNTERED: NtStatus = NtStatus(0xC000_050B);

// Informational and warning values.
pub const STATUS_IMAGE_NOT_AT_BASE: NtStatus = NtStatus(0x4000_0003);
pub const STATUS_MORE_ENTRIES: NtStatus = NtStatus(0x0000_0105);
pub const STATUS_GUARD_PAGE_VIOLATION: NtStatus = NtStatus(0x8000_0001);
pub const STATUS_BUFFER_OVERFLOW: NtStatus = NtStatus(0x8000_0005);
pub const STATUS_NO_MORE_ENTRIES: NtStatus = NtStatus(0x8000_001A);

// Errors of the information, memory and section services.
pub const STATUS_UNSUCCESSFUL: NtStatus = NtStatus(0xC000_0001);
pub const STATUS_NOT_IMPLEMENTED: NtStatus = NtStatus(0xC000_0002);
pub const STATUS_INVALID_INFO_CLASS: NtStatus = NtStatus(0xC000_0003);
pub const STATUS_INFO_LENGTH_MISMATCH: NtStatus = NtStatus(0xC000_0004);
pub const STATUS_ACCESS_VIOLATION: NtStatus = NtStatus(0xC000_0005);
pub const STATUS_NO_MEMORY: NtStatus = NtStatus(0xC000_0017);
pub const STATUS_CONFLICTING_ADDRESSES: NtStatus = NtStatus(0xC000_0018);
pub const STATUS_NOT_MAPPED_VIEW: NtStatus = NtStatus(0xC000_0019);
pub const STATUS_UNABLE_TO_FREE_VM: NtStatus = NtStatus(0xC000_001A);
pub const STATUS_UNABLE_TO_DELETE_SECTION: NtStatus = NtStatus(0xC000_001B);
pub const STATUS_INVALID_SYSTEM_SERVICE: NtStatus = NtStatus(0xC000_001C);
pub const STATUS_INVALID_VIEW_SIZE: NtStatus = NtStatus(0xC000_001F);
pub const STATUS_INVALID_FILE_FOR_SECTION: NtStatus = NtStatus(0xC000_0020);
pub const STATUS_ALREADY_COMMITTED: NtStatus = NtStatus(0xC000_0021);
pub const STATUS_BUFFER_TOO_SMALL: NtStatus = NtStatus(0xC000_0023);
pub const STATUS_NOT_COMMITTED: NtStatus = NtStatus(0xC000_002D);
pub const STATUS_SECTION_TOO_BIG: NtStatus = NtStatus(0xC000_0040);
pub const STATUS_INVALID_PAGE_PROTECTION: NtStatus = NtStatus(0xC000_0045);
pub const STATUS_SECTION_NOT_IMAGE: NtStatus = NtStatus(0xC000_0049);
pub const STATUS_SECTION_PROTECTION: NtStatus = NtStatus(0xC000_004E);
pub const STATUS_INVALID_IMAGE_FORMAT: NtStatus = NtStatus(0xC000_007B);
pub const STATUS_FREE_VM_NOT_AT_BASE: NtStatus = NtStatus(0xC000_009F);
pub const STATUS_MEMORY_NOT_ALLOCATED: NtStatus = NtStatus(0xC000_00A0);
pub const STATUS_NOT_SUPPORTED: NtStatus = NtStatus(0xC000_00BB);
pub const STATUS_INVALID_PARAMETER_1: NtStatus = NtStatus(0xC000_00EF);
pub const STATUS_INVALID_PARAMETER_2: NtStatus = NtStatus(0xC000_00F0);
pub const STATUS_INVALID_PARAMETER_3: NtStatus = NtStatus(0xC000_00F1);
pub const STATUS_INVALID_PARAMETER_4: NtStatus = NtStatus(0xC000_00F2);
pub const STATUS_INVALID_PARAMETER_5: NtStatus = NtStatus(0xC000_00F3);
pub const STATUS_INVALID_PARAMETER_6: NtStatus = NtStatus(0xC000_00F4);
pub const STATUS_INVALID_PARAMETER_7: NtStatus = NtStatus(0xC000_00F5);
pub const STATUS_INVALID_PARAMETER_8: NtStatus = NtStatus(0xC000_00F6);
pub const STATUS_INVALID_PARAMETER_9: NtStatus = NtStatus(0xC000_00F7);
pub const STATUS_INVALID_PARAMETER_10: NtStatus = NtStatus(0xC000_00F8);
pub const STATUS_MAPPED_FILE_SIZE_ZERO: NtStatus = NtStatus(0xC000_011E);
pub const STATUS_MAPPED_ALIGNMENT: NtStatus = NtStatus(0xC000_0220);

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn severity() {
        assert!(STATUS_SUCCESS.is_success());
        assert!(STATUS_TIMEOUT.is_success());
        assert!(STATUS_OBJECT_NAME_EXISTS.is_success());
        assert!(!STATUS_OBJECT_NAME_EXISTS.is_error());
        assert!(STATUS_ACCESS_DENIED.is_error());
        assert!(!STATUS_ACCESS_DENIED.is_success());
    }

    #[test]
    fn wait_values() {
        assert_eq!(NtStatus::wait(0), STATUS_WAIT_0);
        assert_eq!(NtStatus::wait(63), STATUS_WAIT_63);
        assert_eq!(NtStatus::abandoned(63), STATUS_ABANDONED_WAIT_63);
        assert_eq!(std::format!("{:?}", NtStatus::wait(3)), "STATUS_WAIT_n(0x3)");
        assert_eq!(std::format!("{}", STATUS_ACCESS_DENIED), "STATUS_ACCESS_DENIED");
        assert_eq!(std::format!("{:?}", NtStatus(0xC000_0001)), "STATUS_UNSUCCESSFUL");
        assert_eq!(std::format!("{:?}", NtStatus(0xC000_7777)), "NTSTATUS(0xc0007777)");
    }
}
