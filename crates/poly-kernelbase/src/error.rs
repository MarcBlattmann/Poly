//! Win32 error codes and where they come from.
//!
//! Almost every failure of a `kernelbase` function is an `NTSTATUS` from
//! `ntdll`, and the error `GetLastError` reports is
//! `RtlNtStatusToDosError` of it (Learn, "RtlNtStatusToDosError"; the table
//! is [`poly_rtl::status`], shared with our `ntdll`). A few functions
//! document a different code for one case; those are [`for_create_file`]
//! and friends here.

use poly_ntobj::NtStatus;
pub use poly_rtl::status::win32::*;

/// `ERROR_INVALID_ACCESS` (`winerror.h`).
pub const ERROR_INVALID_ACCESS: u32 = 12;
/// `ERROR_OUTOFMEMORY` (`winerror.h`).
pub const ERROR_OUTOFMEMORY: u32 = 14;
/// `ERROR_FILE_EXISTS` (`winerror.h`).
pub const ERROR_FILE_EXISTS: u32 = 80;
/// `ERROR_NEGATIVE_SEEK` (`winerror.h`).
pub const ERROR_NEGATIVE_SEEK: u32 = 131;
/// `ERROR_BAD_ARGUMENTS` (`winerror.h`).
pub const ERROR_BAD_ARGUMENTS: u32 = 160;
/// `ERROR_IO_INCOMPLETE` (`winerror.h`).
pub const ERROR_IO_INCOMPLETE: u32 = 996;
/// `ERROR_CANNOT_IMPERSONATE` (`winerror.h`).
pub const ERROR_CANNOT_IMPERSONATE: u32 = 1368;
/// `ERROR_ABANDONED_WAIT_0` (`winerror.h`).
pub const ERROR_ABANDONED_WAIT_0: u32 = 735;
/// `ERROR_OBJECT_NAME_EXISTS` (`winerror.h`).
pub const ERROR_OBJECT_NAME_EXISTS: u32 = 698;
/// `WAIT_TIMEOUT`, `WAIT_IO_COMPLETION` (`winbase.h`) as errors.
pub const ERROR_WAIT_TIMEOUT: u32 = 258;
pub const ERROR_WAIT_IO_COMPLETION: u32 = 192;
/// `ERROR_OLD_WIN_VERSION` (`winerror.h`).
pub const ERROR_OLD_WIN_VERSION: u32 = 1150;
/// `ERROR_INVALID_FLAGS` (`winerror.h`).
pub const ERROR_INVALID_FLAGS: u32 = 1004;
/// `ERROR_RESOURCE_TYPE_NOT_FOUND` (`winerror.h`).
pub const ERROR_RESOURCE_TYPE_NOT_FOUND: u32 = 1813;
/// `ERROR_RESOURCE_LANG_NOT_FOUND` (`winerror.h`).
pub const ERROR_RESOURCE_LANG_NOT_FOUND: u32 = 1815;

/// `RtlNtStatusToDosError`.
pub fn from_status(status: NtStatus) -> u32 {
    poly_rtl::status::nt_status_to_dos_error(status)
}

/// `STATUS_OBJECT_NAME_COLLISION` (`ntstatus.h`).
pub const STATUS_OBJECT_NAME_COLLISION: NtStatus = NtStatus(0xC000_0035);

/// The error `CreateFileW` reports for a failed `NtCreateFile`: Learn's
/// `CreateFileW` page gives `ERROR_FILE_EXISTS` (80) for `CREATE_NEW` on an
/// existing file, where the general mapping gives `ERROR_ALREADY_EXISTS`.
pub fn for_create_file(status: NtStatus) -> u32 {
    if status == STATUS_OBJECT_NAME_COLLISION { ERROR_FILE_EXISTS } else { from_status(status) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_file_reports_file_exists() {
        assert_eq!(for_create_file(STATUS_OBJECT_NAME_COLLISION), ERROR_FILE_EXISTS);
        assert_eq!(from_status(STATUS_OBJECT_NAME_COLLISION), ERROR_ALREADY_EXISTS);
        assert_eq!(for_create_file(NtStatus(0xC000_0034)), ERROR_FILE_NOT_FOUND);
        assert_eq!(for_create_file(NtStatus(0xC000_003A)), ERROR_PATH_NOT_FOUND);
        assert_eq!(for_create_file(NtStatus(0xC000_0043)), ERROR_SHARING_VIOLATION);
    }
}
