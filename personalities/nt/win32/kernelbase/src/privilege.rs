//! The security functions 7-Zip's command line imports beyond ADR 0083's:
//! privilege adjustment (ADR 0114: the kernel's `NtAdjustPrivilegesToken`,
//! the logic `poly_security::token::adjust_privileges`) and file security
//! descriptors (ADR 0087: stubs in the coverage map, each reporting what
//! Learn documents for that outcome).

use crate::base::*;
use crate::nt::Handle;
use poly_kernelbase::error::*;
use poly_ntobj::NtStatus;
use poly_security::token::STATUS_NOT_ALL_ASSIGNED;

/// `ERROR_NOT_ALL_ASSIGNED` (`winerror.h`).
const ERROR_NOT_ALL_ASSIGNED: u32 = 1300;

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtAdjustPrivilegesToken(
        token: Handle,
        disable_all: u8,
        new_state: *const u8,
        buffer_length: u32,
        previous: *mut u8,
        return_length: *mut u32,
    ) -> NtStatus;
}

/// `AdjustTokenPrivileges` (Learn): enables, disables or removes the
/// privileges `NewState` names that the token holds, or with
/// `DisableAllPrivileges` disables them all. A success sets the last error
/// to `ERROR_SUCCESS`, or to `ERROR_NOT_ALL_ASSIGNED` when the token lacks
/// one of them ("The function may succeed with this error value even if no
/// privileges were adjusted"); `PreviousState` gets the privileges that
/// changed with their former attributes, and a buffer too small for them
/// is `ERROR_INSUFFICIENT_BUFFER` with nothing adjusted.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AdjustTokenPrivileges(
    token: Handle,
    disable_all: i32,
    new_state: *const u8,
    buffer_length: u32,
    previous: *mut u8,
    return_length: *mut u32,
) -> i32 {
    // Learn: ReturnLength "can be NULL if PreviousState is NULL".
    if !previous.is_null() && return_length.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's structures, checked by the kernel.
    let st = unsafe {
        NtAdjustPrivilegesToken(token, u8::from(disable_all != 0), new_state, buffer_length, previous, return_length)
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    set_last_error(if st == STATUS_NOT_ALL_ASSIGNED { ERROR_NOT_ALL_ASSIGNED } else { ERROR_SUCCESS });
    TRUE
}

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtQuerySecurityObject(handle: Handle, info: u32, sd: *mut u8, length: u32, needed: *mut u32) -> NtStatus;
}

/// `GetFileSecurityW(lpFileName, RequestedInformation, pSecurityDescriptor, nLength,
/// lpnLengthNeeded)`: the file's or directory's descriptor, as `NtQuerySecurityObject` gives it on
/// a handle opened for it (Learn: `READ_CONTROL`, and `ACCESS_SYSTEM_SECURITY` for the SACL; the
/// open asks for backup intent so that a directory opens too). A file the caller cannot open is
/// that open's error (the sheet of `SystemTemp`: `ERROR_ACCESS_DENIED` for the filtered token); a
/// buffer that is too small is `ERROR_INSUFFICIENT_BUFFER` with the size in `*lpnLengthNeeded`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileSecurityW(
    name: *const u16,
    requested: u32,
    descriptor: *mut u8,
    length: u32,
    needed: *mut u32,
) -> i32 {
    /// `SACL_SECURITY_INFORMATION`, `ACCESS_SYSTEM_SECURITY`, `READ_CONTROL`,
    /// `FILE_OPEN_FOR_BACKUP_INTENT` (`winnt.h`, `winternl.h`).
    const SACL_SECURITY_INFORMATION: u32 = 8;
    const ACCESS_SYSTEM_SECURITY: u32 = 0x0100_0000;
    const READ_CONTROL: u32 = 0x0002_0000;
    const FILE_OPEN_FOR_BACKUP_INTENT: u32 = 0x4000;
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    if needed.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let access = READ_CONTROL | if requested & SACL_SECURITY_INFORMATION != 0 { ACCESS_SYSTEM_SECURITY } else { 0 };
    let h = match crate::file::open_for(name, access, FILE_OPEN_FOR_BACKUP_INTENT) {
        Ok(h) => h,
        Err(e) => return fail(e, FALSE),
    };
    let mut need = 0u32;
    // SAFETY: the caller's buffer of `length` bytes (or NULL with 0); a local count.
    let st = unsafe { NtQuerySecurityObject(h, requested, descriptor, length, &mut need) };
    close(h);
    // SAFETY: the caller's out-pointer.
    unsafe { needed.write_unaligned(need) };
    if st.is_success() {
        TRUE
    } else if st.0 == 0xC000_0023 {
        // STATUS_BUFFER_TOO_SMALL: ERROR_INSUFFICIENT_BUFFER (122).
        fail(122, FALSE)
    } else {
        fail_status(st, FALSE)
    }
}

/// `SetFileSecurityW`: as `GetFileSecurityW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetFileSecurityW(name: *const u16, _information: u32, _descriptor: *const u8) -> i32 {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    match crate::file::attributes_of(name) {
        Ok(_) => fail(ERROR_NOT_SUPPORTED, FALSE),
        Err(e) => fail(e, FALSE),
    }
}
