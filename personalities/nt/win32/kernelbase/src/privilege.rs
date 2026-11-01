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

#[link(name = "ntdll", kind = "raw-dylib")]
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

/// `GetFileSecurityW`: Poly's NT file I/O keeps no security descriptors
/// yet (no `NtQuerySecurityObject`): `ERROR_NOT_SUPPORTED` once the file
/// is known to exist.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileSecurityW(
    name: *const u16,
    _requested: u32,
    _descriptor: *mut u8,
    _length: u32,
    _needed: *mut u32,
) -> i32 {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    match crate::file::attributes_of(name) {
        Ok(_) => fail(ERROR_NOT_SUPPORTED, FALSE),
        Err(e) => fail(e, FALSE),
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
