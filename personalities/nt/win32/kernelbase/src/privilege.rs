//! The security functions 7-Zip's command line imports that Poly cannot do
//! yet (ADR 0087): privilege adjustment and file security descriptors.
//! Each reports what Learn documents for that outcome; they are stubs in
//! the coverage map.

use crate::base::*;
use crate::nt::Handle;
use alloc::vec::Vec;
use poly_kernelbase::error::*;

/// `ERROR_NOT_ALL_ASSIGNED` (`winerror.h`).
const ERROR_NOT_ALL_ASSIGNED: u32 = 1300;
/// `TokenPrivileges` (`winnt.h`'s `TOKEN_INFORMATION_CLASS`).
const TOKEN_PRIVILEGES: u32 = 3;

/// The LUIDs of the privileges `token` holds (`TOKEN_PRIVILEGES`: a count,
/// then 12-byte `LUID_AND_ATTRIBUTES`).
fn held(token: Handle) -> Option<Vec<u64>> {
    let mut buf = [0u8; 1024];
    let mut len = 0u32;
    // SAFETY: valid pointers.
    let ok = unsafe {
        crate::security::GetTokenInformation(token, TOKEN_PRIVILEGES, buf.as_mut_ptr(), buf.len() as u32, &mut len)
    };
    if ok == FALSE {
        return None;
    }
    let n = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
    Some((0..n.min(84)).map(|i| u64::from_le_bytes(buf[4 + i * 12..12 + i * 12].try_into().unwrap())).collect())
}

/// `AdjustTokenPrivileges`: Poly's tokens have no service to change their
/// privileges' state yet, so nothing is adjusted. Learn: the function
/// "may succeed with" `ERROR_NOT_ALL_ASSIGNED` "even if no privileges were
/// adjusted"; Poly reports that unless every privilege asked for is one
/// the token holds (then `ERROR_SUCCESS`, its state left as it is).
/// `PreviousState`, if asked for, lists no privileges.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AdjustTokenPrivileges(
    token: Handle,
    disable_all: i32,
    new_state: *const u8,
    buffer_length: u32,
    previous: *mut u8,
    return_length: *mut u32,
) -> i32 {
    let Some(have) = held(token) else { return FALSE };
    if !previous.is_null() {
        if buffer_length < 4 {
            if !return_length.is_null() {
                // SAFETY: the caller's out-pointer.
                unsafe { return_length.write_unaligned(4) };
            }
            return fail(ERROR_INSUFFICIENT_BUFFER, FALSE);
        }
        // SAFETY: the caller's TOKEN_PRIVILEGES of `buffer_length` bytes.
        unsafe { previous.cast::<u32>().write_unaligned(0) };
    }
    if !return_length.is_null() {
        // SAFETY: as above.
        unsafe { return_length.write_unaligned(4) };
    }
    if disable_all != 0 {
        return fail(ERROR_SUCCESS, TRUE);
    }
    if new_state.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's TOKEN_PRIVILEGES.
    let n = unsafe { new_state.cast::<u32>().read_unaligned() } as usize;
    let all = (0..n).all(|i| {
        // SAFETY: as above: `n` entries of 12 bytes after the count.
        let luid = unsafe { new_state.add(4 + i * 12).cast::<u64>().read_unaligned() };
        have.contains(&luid)
    });
    fail(if all { ERROR_SUCCESS } else { ERROR_NOT_ALL_ASSIGNED }, TRUE)
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
