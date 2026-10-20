//! `NTSTATUS` values.
//!
//! Every value below is a constant from the public Windows SDK header
//! `ntstatus.h` (`shared/ntstatus.h`), except [`STATUS_SUCCESS`], the
//! `STATUS_WAIT_*` and `STATUS_ABANDONED_WAIT_*` values, which `ntstatus.h`
//! and `winnt.h` both define. Only the values this crate returns are here.

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
            STATUS_SEMAPHORE_LIMIT_EXCEEDED => "STATUS_SEMAPHORE_LIMIT_EXCEEDED",
            STATUS_PRIVILEGE_NOT_HELD => "STATUS_PRIVILEGE_NOT_HELD",
            STATUS_INSUFFICIENT_RESOURCES => "STATUS_INSUFFICIENT_RESOURCES",
            STATUS_MUTANT_LIMIT_EXCEEDED => "STATUS_MUTANT_LIMIT_EXCEEDED",
            STATUS_HANDLE_NOT_CLOSABLE => "STATUS_HANDLE_NOT_CLOSABLE",
            STATUS_REPARSE_POINT_NOT_RESOLVED => "STATUS_REPARSE_POINT_NOT_RESOLVED",
            STATUS_REPARSE_POINT_ENCOUNTERED => "STATUS_REPARSE_POINT_ENCOUNTERED",
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
pub const STATUS_MUTANT_NOT_OWNED: NtStatus = NtStatus(0xC000_0046);
pub const STATUS_SEMAPHORE_LIMIT_EXCEEDED: NtStatus = NtStatus(0xC000_0047);
pub const STATUS_PRIVILEGE_NOT_HELD: NtStatus = NtStatus(0xC000_0061);
pub const STATUS_INSUFFICIENT_RESOURCES: NtStatus = NtStatus(0xC000_009A);
pub const STATUS_MUTANT_LIMIT_EXCEEDED: NtStatus = NtStatus(0xC000_0191);
pub const STATUS_HANDLE_NOT_CLOSABLE: NtStatus = NtStatus(0xC000_0235);
pub const STATUS_REPARSE_POINT_NOT_RESOLVED: NtStatus = NtStatus(0xC000_0280);
pub const STATUS_REPARSE_POINT_ENCOUNTERED: NtStatus = NtStatus(0xC000_050B);

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
        assert_eq!(std::format!("{:?}", NtStatus(0xC000_0001)), "NTSTATUS(0xc0000001)");
    }
}
