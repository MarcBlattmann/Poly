//! Our `secur32.dll` (ADR 0155): `GetUserNameExW`, and the SSPI and LSA
//! entry points Chrome imports for Negotiate/NTLM authentication, answered
//! as a system without those security packages answers them (Learn lists
//! `SEC_E_SECPKG_NOT_FOUND` for an unknown package, `SEC_E_INVALID_HANDLE`
//! for a handle nobody issued). Chrome then offers no integrated
//! authentication.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use poly_netapi as api;
use poly_win32_netrt as rt;

/// `SECURITY_STATUS` values (`winerror.h`, `sspi.h`).
const SEC_E_OK: i32 = 0;
const SEC_E_INVALID_HANDLE: i32 = 0x8009_0301_u32 as i32;
const SEC_E_SECPKG_NOT_FOUND: i32 = 0x8009_0305_u32 as i32;
/// `STATUS_NOT_IMPLEMENTED`, `STATUS_ACCESS_DENIED` (`ntstatus.h`).
const STATUS_NOT_IMPLEMENTED: i32 = 0xC000_0002_u32 as i32;

/// `GetUserNameExW(NameFormat, lpNameBuffer, nSize*)`: `BOOLEAN`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetUserNameExW(format: u32, buffer: *mut u16, size: *mut u32) -> u8 {
    if size.is_null() {
        rt::set_error(api::ERROR_INVALID_PARAMETER);
        return 0;
    }
    let name = match api::user_name_ex(format, &rt::host_name()) {
        Ok(n) => n,
        Err(e) => {
            rt::set_error(e);
            return 0;
        }
    };
    // SAFETY: checked non-null.
    let capacity = if buffer.is_null() { 0 } else { unsafe { size.read() } };
    match api::fit(&name, capacity) {
        Ok(count) => {
            // SAFETY: the caller's buffer holds `count + 1` characters.
            unsafe {
                for (i, c) in name.encode_utf16().chain([0]).enumerate() {
                    buffer.add(i).write(c);
                }
                size.write(count);
            }
            1
        }
        Err((e, need)) => {
            // SAFETY: checked non-null.
            unsafe { size.write(need) };
            rt::set_error(e);
            0
        }
    }
}

// ----- SSPI (sspi.h): no security packages ---------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "system" fn AcquireCredentialsHandleW(
    _principal: *const u16,
    _package: *const u16,
    _use: u32,
    _logon_id: *const u8,
    _auth_data: *const u8,
    _get_key_fn: usize,
    _get_key_arg: *const u8,
    _credential: *mut u8,
    _expiry: *mut u8,
) -> i32 {
    SEC_E_SECPKG_NOT_FOUND
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn QuerySecurityPackageInfoW(_package: *const u16, info: *mut *mut u8) -> i32 {
    if !info.is_null() {
        // SAFETY: the caller's variable.
        unsafe { info.write(core::ptr::null_mut()) };
    }
    SEC_E_SECPKG_NOT_FOUND
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitializeSecurityContextW(
    _credential: *const u8,
    _context: *const u8,
    _target: *const u16,
    _flags: u32,
    _reserved1: u32,
    _data_rep: u32,
    _input: *const u8,
    _reserved2: u32,
    _new_context: *mut u8,
    _output: *mut u8,
    _attrs: *mut u32,
    _expiry: *mut u8,
) -> i32 {
    SEC_E_INVALID_HANDLE
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryContextAttributesW(_context: *const u8, _attribute: u32, _buffer: *mut u8) -> i32 {
    SEC_E_INVALID_HANDLE
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DeleteSecurityContext(_context: *const u8) -> i32 {
    SEC_E_INVALID_HANDLE
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn FreeCredentialsHandle(_credential: *const u8) -> i32 {
    SEC_E_INVALID_HANDLE
}

/// `FreeContextBuffer(pvContextBuffer)`: nothing of ours is ever handed out.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FreeContextBuffer(_buffer: *mut u8) -> i32 {
    SEC_E_OK
}

// ----- LSA (ntsecapi.h) ------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "system" fn LsaConnectUntrusted(handle: *mut isize) -> i32 {
    if !handle.is_null() {
        // SAFETY: the caller's variable.
        unsafe { handle.write(0) };
    }
    STATUS_NOT_IMPLEMENTED
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn LsaDeregisterLogonProcess(_handle: isize) -> i32 {
    SEC_E_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn LsaFreeReturnBuffer(_buffer: *mut u8) -> i32 {
    SEC_E_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn LsaLogonUser(
    _handle: isize,
    _origin: *const u8,
    _logon_type: u32,
    _package: u32,
    _auth_info: *const u8,
    _auth_len: u32,
    _groups: *const u8,
    _source: *const u8,
    _profile: *mut *mut u8,
    _profile_len: *mut u32,
    _logon_id: *mut u8,
    _token: *mut isize,
    _quotas: *mut u8,
    _sub_status: *mut i32,
) -> i32 {
    STATUS_NOT_IMPLEMENTED
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn secur32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
