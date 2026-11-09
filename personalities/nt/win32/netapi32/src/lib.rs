//! Our `netapi32.dll` (ADR 0155): what Chrome asks of the Net API and the
//! directory-service role API to decide whether the computer belongs to a
//! domain: `NetUserGetInfo`, `NetGetJoinInformation`,
//! `DsRoleGetPrimaryDomainInformation`, `NetGetAadJoinInformation`, and the
//! calls that free their buffers. Poly is a workgroup computer whose one
//! account is `poly`; the answers are `poly-netapi`'s (host-tested). Every
//! buffer comes from the process heap and is released by
//! `NetApiBufferFree` (`DsRoleFreeMemory`, `NetFreeAadJoinInformation`).

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use poly_netapi as api;
use poly_win32_netrt as rt;

/// `NERR_Success`.
const NERR_SUCCESS: u32 = 0;

/// `NetApiBufferFree(Buffer)`: `NERR_Success`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn NetApiBufferFree(buffer: *mut u8) -> u32 {
    // SAFETY: a buffer of this module, or null.
    unsafe { rt::heap_free(buffer) };
    NERR_SUCCESS
}

/// A buffer made for the address it will have: build once to size it, then again at its address.
fn alloc_result(build: impl Fn(u64) -> Result<alloc::vec::Vec<u8>, u32>) -> Result<*mut u8, u32> {
    let size = build(0)?.len();
    let p = rt::heap_alloc(size.max(1));
    if p.is_null() {
        return Err(api::ERROR_NOT_ENOUGH_MEMORY);
    }
    match build(p as u64) {
        Ok(bytes) if bytes.len() == size => {
            // SAFETY: `p` holds `size` bytes.
            unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, size) };
            Ok(p)
        }
        Ok(_) | Err(_) => {
            // SAFETY: our block.
            unsafe { rt::heap_free(p) };
            Err(api::ERROR_NOT_ENOUGH_MEMORY)
        }
    }
}

/// `NetUserGetInfo(servername, username, level, bufptr**)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn NetUserGetInfo(
    server: *const u16,
    user: *const u16,
    level: u32,
    buf: *mut *mut u8,
) -> u32 {
    if buf.is_null() || user.is_null() {
        return api::ERROR_INVALID_PARAMETER;
    }
    // SAFETY: the caller's strings.
    let (server, user) = unsafe { (rt::wstr(server), rt::wstr(user).unwrap_or_default()) };
    let host = rt::host_name();
    match alloc_result(|base| api::user_info(server.as_deref(), &user, level, &host, base)) {
        Ok(p) => {
            // SAFETY: checked non-null.
            unsafe { buf.write(p) };
            NERR_SUCCESS
        }
        Err(e) => e,
    }
}

/// `NetGetJoinInformation(lpServer, lpNameBuffer**, BufferType*)`: a workgroup.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn NetGetJoinInformation(server: *const u16, name: *mut *mut u16, kind: *mut u32) -> u32 {
    if name.is_null() || kind.is_null() {
        return api::ERROR_INVALID_PARAMETER;
    }
    // SAFETY: the caller's string.
    let server = unsafe { rt::wstr(server) };
    match api::join_information(server.as_deref(), &rt::host_name()) {
        Ok((n, k)) => {
            let w = rt::wide(n);
            let bytes: alloc::vec::Vec<u8> = w.iter().flat_map(|c| c.to_le_bytes()).collect();
            let p = rt::heap_copy(&bytes);
            if p.is_null() {
                return api::ERROR_NOT_ENOUGH_MEMORY;
            }
            // SAFETY: checked non-null.
            unsafe {
                name.write(p.cast());
                kind.write(k);
            }
            NERR_SUCCESS
        }
        Err(e) => e,
    }
}

/// `DsRoleGetPrimaryDomainInformation(lpServer, InfoLevel, Buffer**)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DsRoleGetPrimaryDomainInformation(
    server: *const u16,
    level: u32,
    buf: *mut *mut u8,
) -> u32 {
    if buf.is_null() {
        return api::ERROR_INVALID_PARAMETER;
    }
    // SAFETY: the caller's string.
    let server = unsafe { rt::wstr(server) };
    let host = rt::host_name();
    match alloc_result(|base| api::dsrole_basic(server.as_deref(), level, &host, base)) {
        Ok(p) => {
            // SAFETY: checked non-null.
            unsafe { buf.write(p) };
            0
        }
        Err(e) => e,
    }
}

/// `DsRoleFreeMemory(Buffer)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DsRoleFreeMemory(buffer: *mut u8) {
    // SAFETY: a buffer of this module, or null.
    unsafe { rt::heap_free(buffer) };
}

/// `NetGetAadJoinInformation(pcszTenantId, ppJoinInfo**)`: `HRESULT`. A computer that is not
/// joined to Azure AD gets `S_OK` and no structure (Learn: "ppJoinInfo ... NULL if the device is
/// not joined").
#[unsafe(no_mangle)]
pub unsafe extern "system" fn NetGetAadJoinInformation(_tenant: *const u16, info: *mut *mut u8) -> i32 {
    if info.is_null() {
        return 0x8007_0057_u32 as i32; // E_INVALIDARG
    }
    // SAFETY: checked non-null.
    unsafe { info.write(core::ptr::null_mut()) };
    0
}

/// `NetFreeAadJoinInformation(pJoinInfo)`: nothing was allocated.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn NetFreeAadJoinInformation(_info: *mut u8) {}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn netapi32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
