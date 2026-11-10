//! The errors the security functions share, as one enum that maps to a
//! Win32 error (`kernelbase`) and an `NTSTATUS` (`ntdll`'s `Rtl*`).
//!
//! The two layers report the same failures under different codes (the spec
//! sheets list both): `ERROR_INSUFFICIENT_BUFFER` is `STATUS_BUFFER_TOO_SMALL`,
//! `ERROR_INVALID_FLAGS` is a plain `STATUS_INVALID_PARAMETER` in the native
//! API, and so on. The mapping is here so the logic exists once.

use crate::error::*;
use crate::status::NtStatus;

/// What went wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecErr {
    InvalidParameter,
    /// `ERROR_INSUFFICIENT_BUFFER` / `STATUS_BUFFER_TOO_SMALL`.
    BufferTooSmall,
    InvalidSid,
    InvalidAcl,
    /// `ERROR_ALLOTTED_SPACE_EXCEEDED` / `STATUS_ALLOTTED_SPACE_EXCEEDED`.
    AllottedSpace,
    RevisionMismatch,
    UnknownRevision,
    /// `ERROR_INVALID_FLAGS`; the native API says `STATUS_INVALID_PARAMETER`.
    InvalidFlags,
    InvalidSd,
    BadDescriptorFormat,
    NonDomainSid,
    NonAccountSid,
    InvalidOwner,
    InvalidGroup,
    NoImpersonationToken,
    BadImpersonationLevel,
    GenericNotMapped,
    PrivilegeNotHeld,
    AccessDenied,
    NoSuchPrivilege,
    NoneMapped,
    InvalidDatatype,
    InvalidUuid,
    /// An access-allowed callback object ACE (type 0x0B) has no SDDL text:
    /// `ConvertSecurityDescriptorToStringSecurityDescriptorW` fails with
    /// last error 805 (the spec sheet).
    CallbackObjectAce,
}

impl SecErr {
    /// The Win32 error code.
    pub fn win32(self) -> u32 {
        use SecErr::*;
        match self {
            InvalidParameter => ERROR_INVALID_PARAMETER,
            BufferTooSmall => ERROR_INSUFFICIENT_BUFFER,
            InvalidSid => ERROR_INVALID_SID,
            InvalidAcl => ERROR_INVALID_ACL,
            AllottedSpace => ERROR_ALLOTTED_SPACE_EXCEEDED,
            RevisionMismatch => ERROR_REVISION_MISMATCH,
            UnknownRevision => ERROR_UNKNOWN_REVISION,
            InvalidFlags => ERROR_INVALID_FLAGS,
            InvalidSd => ERROR_INVALID_SECURITY_DESCR,
            BadDescriptorFormat => ERROR_BAD_DESCRIPTOR_FORMAT,
            NonDomainSid => 1258,
            NonAccountSid => 1257,
            InvalidOwner => ERROR_INVALID_OWNER,
            InvalidGroup => ERROR_INVALID_PRIMARY_GROUP,
            NoImpersonationToken => 1309,
            BadImpersonationLevel => 1346,
            GenericNotMapped => 1360,
            PrivilegeNotHeld => 1314,
            AccessDenied => 5,
            NoSuchPrivilege => 1313,
            NoneMapped => ERROR_NONE_MAPPED,
            InvalidDatatype => 1804,
            InvalidUuid => 1705,
            CallbackObjectAce => 805,
        }
    }

    /// The native status (`ntstatus.h`).
    pub fn nt(self) -> NtStatus {
        use SecErr::*;
        NtStatus(match self {
            InvalidParameter | InvalidFlags | InvalidDatatype | InvalidUuid | NonDomainSid | NonAccountSid
            | CallbackObjectAce => 0xC000_000D,
            BufferTooSmall => 0xC000_0023,
            InvalidSid => 0xC000_0078,
            InvalidAcl => 0xC000_0077,
            AllottedSpace => 0xC000_0099,
            RevisionMismatch => 0xC000_0059,
            UnknownRevision => 0xC000_0058,
            InvalidSd => 0xC000_0079,
            BadDescriptorFormat => 0xC000_00E7,
            InvalidOwner => 0xC000_005A,
            InvalidGroup => 0xC000_005B,
            NoImpersonationToken => 0xC000_005C,
            BadImpersonationLevel => 0xC000_00A5,
            GenericNotMapped => 0xC000_00E6,
            PrivilegeNotHeld => 0xC000_0061,
            AccessDenied => 0xC000_0022,
            NoSuchPrivilege => 0xC000_0060,
            NoneMapped => 0xC000_0073,
        })
    }
}

pub type SecResult<T> = Result<T, SecErr>;
