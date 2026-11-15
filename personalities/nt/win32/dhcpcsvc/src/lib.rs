//! Our `dhcpcsvc.dll` (ADR 0155): the DHCP client API Chrome's network
//! service asks for the WPAD option (252) of an adapter. Poly's adapters
//! are configured by the boot setting (`poly_net::StaticIpv4`), so no
//! option was ever received: `DhcpRequestParams` succeeds for an adapter
//! Poly has and every requested parameter comes back with no data
//! (`nBytesData` 0); the argument checks are `poly_iphlp::dhcp`'s.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use core::sync::atomic::{AtomicBool, Ordering};
use poly_win32_netrt as netrt;

/// `ERROR_SUCCESS`, `ERROR_INVALID_PARAMETER` (`winerror.h`).
const ERROR_SUCCESS: u32 = 0;
const ERROR_INVALID_PARAMETER: u32 = 87;

/// The API version `DhcpCApiInitialize` reports (`dhcpcsdk.h`: `DHCPCAPI_VERSION` is 2).
const DHCP_API_VERSION: u32 = 2;

/// Between `DhcpCApiInitialize` and `DhcpCApiCleanup`: the calls fail with 87 outside it (observed on
/// Windows; one cleanup undoes any number of initialisations).
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// `DhcpCApiInitialize(LPDWORD Version)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DhcpCApiInitialize(version: *mut u32) -> u32 {
    if version.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    // SAFETY: checked non-null.
    unsafe { version.write(DHCP_API_VERSION) };
    INITIALIZED.store(true, Ordering::Relaxed);
    ERROR_SUCCESS
}

/// `DhcpCApiCleanup()`.
#[unsafe(no_mangle)]
pub extern "system" fn DhcpCApiCleanup() {
    INITIALIZED.store(false, Ordering::Relaxed);
}

/// `DHCPCAPI_PARAMS` (x64, 32 bytes): `Flags` 0, `OptionId` 4, `IsVendor` 8, `Data` 16, `nBytesData` 24.
const PARAMS_SIZE: usize = 32;

/// `DhcpRequestParams(Flags, Reserved, AdapterName, ClassId, SendParams, RecdParams, Buffer,
/// pSize, RequestIdStr)`. `SendParams` and `RecdParams` are `DHCPCAPI_PARAMS_ARRAY` structures
/// passed by value, so the x64 calling convention hands over pointers to copies: `nParams` at 0 and
/// `Params` at 8. The checks are `poly_iphlp::dhcp`'s; a request for an adapter of Poly's succeeds
/// and every requested parameter comes back with no data.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DhcpRequestParams(
    flags: u32,
    _reserved: *const u8,
    adapter: *const u16,
    _class_id: *const u8,
    _send: *const u8,
    recd: *const u8,
    _buffer: *mut u8,
    size: *mut u32,
    _request_id: *const u16,
) -> u32 {
    let (count, params) = if recd.is_null() {
        (0, core::ptr::null_mut())
    } else {
        // SAFETY: the caller's DHCPCAPI_PARAMS_ARRAY.
        unsafe {
            (core::ptr::read_unaligned(recd.cast::<u32>()), core::ptr::read_unaligned(recd.add(8).cast::<*mut u8>()))
        }
    };
    // SAFETY: the caller's NUL-terminated adapter name.
    let name = unsafe { netrt::wstr(adapter) };
    if let Err(e) = poly_iphlp::dhcp::request_params(
        INITIALIZED.load(Ordering::Relaxed),
        flags,
        name.as_deref(),
        count,
        &netrt::interfaces(),
    ) {
        return e;
    }
    if !params.is_null() {
        // SAFETY: the caller's `count` DHCPCAPI_PARAMS entries.
        unsafe {
            for i in 0..count as usize {
                let p = params.add(i * PARAMS_SIZE);
                core::ptr::write_unaligned(p.add(16).cast::<usize>(), 0);
                core::ptr::write_unaligned(p.add(24).cast::<u32>(), 0);
            }
        }
    }
    if !size.is_null() {
        // SAFETY: the caller's variable.
        unsafe { size.write(0) };
    }
    ERROR_SUCCESS
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn dhcpcsvc_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
