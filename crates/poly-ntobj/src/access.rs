//! Access masks, generic mappings and the access check.
//!
//! Constants are from the public SDK header `winnt.h` (standard, generic and
//! per-type rights) and the WDK header `wdm.h` / `ntddk.h` (directory and
//! symbolic link rights, `SEMAPHORE_QUERY_STATE`). The layout of an
//! `ACCESS_MASK` (specific rights in bits 0–15, standard rights in 16–20,
//! `ACCESS_SYSTEM_SECURITY`, `MAXIMUM_ALLOWED`, generic rights in 28–31) is
//! described on Microsoft Learn under "ACCESS_MASK".
//!
//! There are no security descriptors yet. Each object carries an *allowed*
//! mask standing in for what its DACL would grant the (single) user; it
//! defaults to every valid right for the type, and the kernel or a test can
//! narrow it (see [`crate::ObjectManager::set_allowed_access`]).

use crate::object::ObjectType;
use crate::status::{NtStatus, Result, STATUS_ACCESS_DENIED, STATUS_PRIVILEGE_NOT_HELD};

/// `ACCESS_MASK` (`winnt.h`).
pub type AccessMask = u32;

pub const DELETE: AccessMask = 0x0001_0000;
pub const READ_CONTROL: AccessMask = 0x0002_0000;
pub const WRITE_DAC: AccessMask = 0x0004_0000;
pub const WRITE_OWNER: AccessMask = 0x0008_0000;
pub const SYNCHRONIZE: AccessMask = 0x0010_0000;
pub const STANDARD_RIGHTS_REQUIRED: AccessMask = 0x000F_0000;
pub const STANDARD_RIGHTS_READ: AccessMask = READ_CONTROL;
pub const STANDARD_RIGHTS_WRITE: AccessMask = READ_CONTROL;
pub const STANDARD_RIGHTS_EXECUTE: AccessMask = READ_CONTROL;
pub const STANDARD_RIGHTS_ALL: AccessMask = 0x001F_0000;
pub const ACCESS_SYSTEM_SECURITY: AccessMask = 0x0100_0000;
pub const MAXIMUM_ALLOWED: AccessMask = 0x0200_0000;
pub const GENERIC_ALL: AccessMask = 0x1000_0000;
pub const GENERIC_EXECUTE: AccessMask = 0x2000_0000;
pub const GENERIC_WRITE: AccessMask = 0x4000_0000;
pub const GENERIC_READ: AccessMask = 0x8000_0000;

pub const EVENT_QUERY_STATE: AccessMask = 0x0001;
pub const EVENT_MODIFY_STATE: AccessMask = 0x0002;
pub const EVENT_ALL_ACCESS: AccessMask = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3;

pub const MUTANT_QUERY_STATE: AccessMask = 0x0001;
pub const MUTANT_ALL_ACCESS: AccessMask = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | MUTANT_QUERY_STATE;

pub const SEMAPHORE_QUERY_STATE: AccessMask = 0x0001;
pub const SEMAPHORE_MODIFY_STATE: AccessMask = 0x0002;
pub const SEMAPHORE_ALL_ACCESS: AccessMask = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3;

pub const TIMER_QUERY_STATE: AccessMask = 0x0001;
pub const TIMER_MODIFY_STATE: AccessMask = 0x0002;
pub const TIMER_ALL_ACCESS: AccessMask = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3;

pub const DIRECTORY_QUERY: AccessMask = 0x0001;
pub const DIRECTORY_TRAVERSE: AccessMask = 0x0002;
pub const DIRECTORY_CREATE_OBJECT: AccessMask = 0x0004;
pub const DIRECTORY_CREATE_SUBDIRECTORY: AccessMask = 0x0008;
pub const DIRECTORY_ALL_ACCESS: AccessMask = STANDARD_RIGHTS_REQUIRED | 0xF;

pub const SYMBOLIC_LINK_QUERY: AccessMask = 0x0001;
pub const SYMBOLIC_LINK_ALL_ACCESS: AccessMask = STANDARD_RIGHTS_REQUIRED | 0x1;

pub const SECTION_QUERY: AccessMask = 0x0001;
pub const SECTION_MAP_WRITE: AccessMask = 0x0002;
pub const SECTION_MAP_READ: AccessMask = 0x0004;
pub const SECTION_MAP_EXECUTE: AccessMask = 0x0008;
pub const SECTION_EXTEND_SIZE: AccessMask = 0x0010;
pub const SECTION_ALL_ACCESS: AccessMask = STANDARD_RIGHTS_REQUIRED | 0x1F;

pub const FILE_ALL_ACCESS: AccessMask = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x1FF;
pub const FILE_GENERIC_READ: AccessMask = 0x0012_0089;
pub const FILE_GENERIC_WRITE: AccessMask = 0x0012_0116;
pub const FILE_GENERIC_EXECUTE: AccessMask = 0x0012_00A0;

pub const KEY_ALL_ACCESS: AccessMask = 0x000F_003F;
pub const KEY_READ: AccessMask = 0x0002_0019;
pub const KEY_WRITE: AccessMask = 0x0002_0006;
pub const KEY_EXECUTE: AccessMask = 0x0002_0019;

// Access-token rights (`winnt.h`; ADR 0083). `TOKEN_READ` is
// `STANDARD_RIGHTS_READ | TOKEN_QUERY`, `TOKEN_WRITE` is
// `STANDARD_RIGHTS_WRITE` with the three adjust rights, `TOKEN_EXECUTE` is
// `STANDARD_RIGHTS_EXECUTE`.
pub const TOKEN_QUERY: AccessMask = 0x0008;
pub const TOKEN_QUERY_SOURCE: AccessMask = 0x0010;
pub const TOKEN_ALL_ACCESS: AccessMask = STANDARD_RIGHTS_REQUIRED | 0x1FF;
pub const TOKEN_READ: AccessMask = 0x0002_0008;
pub const TOKEN_WRITE: AccessMask = 0x0002_00E0;
pub const TOKEN_EXECUTE: AccessMask = 0x0002_0000;

// Process-specific rights (`winnt.h`).
pub const PROCESS_TERMINATE: AccessMask = 0x0001;
pub const PROCESS_CREATE_THREAD: AccessMask = 0x0002;
pub const PROCESS_SET_SESSIONID: AccessMask = 0x0004;
pub const PROCESS_VM_OPERATION: AccessMask = 0x0008;
pub const PROCESS_VM_READ: AccessMask = 0x0010;
pub const PROCESS_VM_WRITE: AccessMask = 0x0020;
/// `PROCESS_DUP_HANDLE`: what the kernel checks on the process handles it
/// passes to [`crate::ObjectManager::nt_duplicate_object`] (DuplicateHandle on Learn).
pub const PROCESS_DUP_HANDLE: AccessMask = 0x0040;
pub const PROCESS_CREATE_PROCESS: AccessMask = 0x0080;
pub const PROCESS_SET_QUOTA: AccessMask = 0x0100;
pub const PROCESS_SET_INFORMATION: AccessMask = 0x0200;
pub const PROCESS_QUERY_INFORMATION: AccessMask = 0x0400;
pub const PROCESS_SUSPEND_RESUME: AccessMask = 0x0800;
pub const PROCESS_QUERY_LIMITED_INFORMATION: AccessMask = 0x1000;
/// `PROCESS_ALL_ACCESS` for Windows Vista and later.
pub const PROCESS_ALL_ACCESS: AccessMask = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0xFFFF;

// Thread-specific rights (`winnt.h`).
pub const THREAD_TERMINATE: AccessMask = 0x0001;
pub const THREAD_SUSPEND_RESUME: AccessMask = 0x0002;
pub const THREAD_GET_CONTEXT: AccessMask = 0x0008;
pub const THREAD_SET_CONTEXT: AccessMask = 0x0010;
pub const THREAD_SET_INFORMATION: AccessMask = 0x0020;
pub const THREAD_QUERY_INFORMATION: AccessMask = 0x0040;
pub const THREAD_SET_THREAD_TOKEN: AccessMask = 0x0080;
pub const THREAD_IMPERSONATE: AccessMask = 0x0100;
pub const THREAD_DIRECT_IMPERSONATION: AccessMask = 0x0200;
pub const THREAD_SET_LIMITED_INFORMATION: AccessMask = 0x0400;
pub const THREAD_QUERY_LIMITED_INFORMATION: AccessMask = 0x0800;
/// `THREAD_ALL_ACCESS` for Windows Vista and later.
pub const THREAD_ALL_ACCESS: AccessMask = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0xFFFF;

/// `GENERIC_MAPPING` (`winnt.h`): what each generic right means for a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenericMapping {
    pub read: AccessMask,
    pub write: AccessMask,
    pub execute: AccessMask,
    pub all: AccessMask,
}

impl GenericMapping {
    /// Replace generic rights by the type's specific and standard rights, as
    /// `RtlMapGenericMask` does (Learn: "RtlMapGenericMask"). Other bits,
    /// `MAXIMUM_ALLOWED` included, are kept.
    pub const fn map(&self, mask: AccessMask) -> AccessMask {
        let mut out = mask & !(GENERIC_READ | GENERIC_WRITE | GENERIC_EXECUTE | GENERIC_ALL);
        if mask & GENERIC_READ != 0 {
            out |= self.read;
        }
        if mask & GENERIC_WRITE != 0 {
            out |= self.write;
        }
        if mask & GENERIC_EXECUTE != 0 {
            out |= self.execute;
        }
        if mask & GENERIC_ALL != 0 {
            out |= self.all;
        }
        out
    }
}

/// The mode the request came from (`KPROCESSOR_MODE`, the previous mode of
/// a system service).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    User,
    Kernel,
}

/// The privileges this crate cares about. Without tokens yet, the kernel
/// says which ones the caller holds (enabled).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Privileges {
    /// `SeCreatePermanentPrivilege`: needed by user-mode callers for
    /// `OBJ_PERMANENT` and `NtMakePermanentObject` (Learn: "Privilege
    /// Constants", `SE_CREATE_PERMANENT_NAME`).
    pub create_permanent: bool,
    /// `SeSecurityPrivilege`: needed for `ACCESS_SYSTEM_SECURITY` (Learn:
    /// "ACCESS_SYSTEM_SECURITY access right").
    pub security: bool,
}

/// Decide the access a new handle gets.
///
/// - Generic rights are mapped with the type's mapping.
/// - `ACCESS_SYSTEM_SECURITY` needs [`Privileges::security`] from user
///   mode, else `STATUS_PRIVILEGE_NOT_HELD`. To be confirmed by the spec
///   agent against real Windows (it may be `STATUS_ACCESS_DENIED`).
/// - A kernel-mode request without `force_check` (`OBJ_FORCE_ACCESS_CHECK`)
///   skips the check, as Learn documents for kernel-mode callers; it is
///   granted the requested rights that are valid for the type, and
///   `MAXIMUM_ALLOWED` grants every valid right.
/// - Otherwise every requested right must be in `allowed`, else
///   `STATUS_ACCESS_DENIED`; `MAXIMUM_ALLOWED` adds all of `allowed`. A
///   requested right that is not valid for the type at all is denied the
///   same way. To be confirmed by the spec agent against real Windows (it
///   might be silently dropped instead).
pub(crate) fn grant(
    kind: ObjectType,
    desired: AccessMask,
    allowed: AccessMask,
    mode: Mode,
    force_check: bool,
    privileges: Privileges,
) -> Result<AccessMask> {
    let mut want = kind.generic_mapping().map(desired);
    let maximum = want & MAXIMUM_ALLOWED != 0;
    want &= !MAXIMUM_ALLOWED;
    let user_rules = mode == Mode::User || force_check;
    if want & ACCESS_SYSTEM_SECURITY != 0 && user_rules && !privileges.security {
        return Err(STATUS_PRIVILEGE_NOT_HELD);
    }
    if !user_rules {
        let valid = kind.valid_access() | ACCESS_SYSTEM_SECURITY;
        if maximum {
            want |= kind.valid_access();
        }
        return Ok(want & valid);
    }
    if want & !(allowed | ACCESS_SYSTEM_SECURITY) != 0 {
        return Err(STATUS_ACCESS_DENIED);
    }
    if maximum {
        want |= allowed;
    }
    Ok(want)
}

/// Check that a handle's granted access covers `required` when it is used.
/// Kernel-mode callers skip the check (Learn: `ObReferenceObjectByHandle`,
/// "AccessMode").
pub(crate) fn check(granted: AccessMask, required: AccessMask, mode: Mode) -> core::result::Result<(), NtStatus> {
    if mode == Mode::Kernel || granted & required == required { Ok(()) } else { Err(STATUS_ACCESS_DENIED) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_rights_map_per_type() {
        let m = ObjectType::Event.generic_mapping();
        assert_eq!(m.map(GENERIC_READ), STANDARD_RIGHTS_READ | EVENT_QUERY_STATE);
        assert_eq!(m.map(GENERIC_WRITE), STANDARD_RIGHTS_WRITE | EVENT_MODIFY_STATE);
        assert_eq!(m.map(GENERIC_EXECUTE), STANDARD_RIGHTS_EXECUTE | SYNCHRONIZE);
        assert_eq!(m.map(GENERIC_ALL), EVENT_ALL_ACCESS);
        assert_eq!(
            m.map(GENERIC_READ | DELETE | MAXIMUM_ALLOWED),
            READ_CONTROL | EVENT_QUERY_STATE | DELETE | MAXIMUM_ALLOWED
        );
        assert_eq!(ObjectType::File.generic_mapping().map(GENERIC_READ), FILE_GENERIC_READ);
        assert_eq!(ObjectType::Key.generic_mapping().map(GENERIC_WRITE), KEY_WRITE);
    }

    #[test]
    fn all_access_values_match_winnt() {
        assert_eq!(EVENT_ALL_ACCESS, 0x1F_0003);
        assert_eq!(MUTANT_ALL_ACCESS, 0x1F_0001);
        assert_eq!(SEMAPHORE_ALL_ACCESS, 0x1F_0003);
        assert_eq!(DIRECTORY_ALL_ACCESS, 0xF_000F);
        assert_eq!(SYMBOLIC_LINK_ALL_ACCESS, 0xF_0001);
        assert_eq!(SECTION_ALL_ACCESS, 0xF_001F);
        assert_eq!(FILE_ALL_ACCESS, 0x1F_01FF);
        assert_eq!(PROCESS_ALL_ACCESS, 0x1F_FFFF);
    }

    #[test]
    fn user_mode_checks_against_allowed() {
        let p = Privileges::default();
        let ev = ObjectType::Event;
        let allowed = SYNCHRONIZE | EVENT_QUERY_STATE;
        assert_eq!(grant(ev, SYNCHRONIZE, allowed, Mode::User, false, p), Ok(SYNCHRONIZE));
        assert_eq!(grant(ev, EVENT_MODIFY_STATE, allowed, Mode::User, false, p), Err(STATUS_ACCESS_DENIED));
        assert_eq!(grant(ev, GENERIC_ALL, allowed, Mode::User, false, p), Err(STATUS_ACCESS_DENIED));
        assert_eq!(grant(ev, MAXIMUM_ALLOWED, allowed, Mode::User, false, p), Ok(allowed));
        assert_eq!(grant(ev, MAXIMUM_ALLOWED | DELETE, allowed, Mode::User, false, p), Err(STATUS_ACCESS_DENIED));
        assert_eq!(grant(ev, 0, allowed, Mode::User, false, p), Ok(0));
        // A right no event has.
        assert_eq!(grant(ev, 0x8000, EVENT_ALL_ACCESS, Mode::User, false, p), Err(STATUS_ACCESS_DENIED));
    }

    #[test]
    fn kernel_mode_skips_the_check_unless_forced() {
        let p = Privileges::default();
        let ev = ObjectType::Event;
        assert_eq!(grant(ev, EVENT_MODIFY_STATE, 0, Mode::Kernel, false, p), Ok(EVENT_MODIFY_STATE));
        assert_eq!(grant(ev, MAXIMUM_ALLOWED, 0, Mode::Kernel, false, p), Ok(EVENT_ALL_ACCESS));
        assert_eq!(grant(ev, EVENT_MODIFY_STATE, 0, Mode::Kernel, true, p), Err(STATUS_ACCESS_DENIED));
        assert_eq!(grant(ev, ACCESS_SYSTEM_SECURITY, 0, Mode::Kernel, false, p), Ok(ACCESS_SYSTEM_SECURITY));
    }

    #[test]
    fn system_security_needs_the_privilege() {
        let ev = ObjectType::Event;
        let none = Privileges::default();
        let sec = Privileges { security: true, ..Privileges::default() };
        assert_eq!(
            grant(ev, ACCESS_SYSTEM_SECURITY, EVENT_ALL_ACCESS, Mode::User, false, none),
            Err(STATUS_PRIVILEGE_NOT_HELD)
        );
        assert_eq!(
            grant(ev, ACCESS_SYSTEM_SECURITY, EVENT_ALL_ACCESS, Mode::User, false, sec),
            Ok(ACCESS_SYSTEM_SECURITY)
        );
    }

    #[test]
    fn use_checks() {
        assert_eq!(check(SYNCHRONIZE | EVENT_QUERY_STATE, SYNCHRONIZE, Mode::User), Ok(()));
        assert_eq!(check(EVENT_QUERY_STATE, SYNCHRONIZE, Mode::User), Err(STATUS_ACCESS_DENIED));
        assert_eq!(check(0, SYNCHRONIZE, Mode::Kernel), Ok(()));
    }
}
