//! Our `dhcpcsvc.dll` (ADR 0155): the DHCP client API Chrome's network
//! service asks for the WPAD option (252) of an adapter. Poly's adapters
//! are configured by the boot setting (`poly_net::StaticIpv4`), so no
//! option was ever received: `DhcpRequestParams` succeeds and every
//! requested parameter comes back with no data (`nBytesData` 0).

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use poly_win32_netrt as _;

/// `ERROR_SUCCESS`, `ERROR_INVALID_PARAMETER` (`winerror.h`).
const ERROR_SUCCESS: u32 = 0;
const ERROR_INVALID_PARAMETER: u32 = 87;

/// The API version `DhcpCApiInitialize` reports (`dhcpcsdk.h`: `DHCPCAPI_VERSION` is 2).
const DHCP_API_VERSION: u32 = 2;

/// `DhcpCApiInitialize(LPDWORD Version)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DhcpCApiInitialize(version: *mut u32) -> u32 {
    if version.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    // SAFETY: checked non-null.
    unsafe { version.write(DHCP_API_VERSION) };
    ERROR_SUCCESS
}

/// `DhcpCApiCleanup()`.
#[unsafe(no_mangle)]
pub extern "system" fn DhcpCApiCleanup() {}

/// `DHCPCAPI_PARAMS` (x64, 32 bytes): `Flags` 0, `OptionId` 4, `IsVendor` 8, `Data` 16, `nBytesData` 24.
const PARAMS_SIZE: usize = 32;

/// `DhcpRequestParams(Flags, Reserved, AdapterName, ClassId, SendParams, RecdParams, Buffer,
/// pSize, RequestIdStr)`. `RecdParams` is a `DHCPCAPI_PARAMS_ARRAY` (`Flags` 0, `nParams` 4,
/// `Params` 8): each entry gets no data.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DhcpRequestParams(
    _flags: u32,
    _reserved: *const u8,
    _adapter: *const u16,
    _class_id: *const u8,
    _send: *const u8,
    recd: *const u8,
    _buffer: *mut u8,
    size: *mut u32,
    _request_id: *const u16,
) -> u32 {
    if !recd.is_null() {
        // SAFETY: the caller's DHCPCAPI_PARAMS_ARRAY and its `nParams` entries.
        unsafe {
            let n = core::ptr::read_unaligned(recd.add(4).cast::<u32>()) as usize;
            let params = core::ptr::read_unaligned(recd.add(8).cast::<*mut u8>());
            if !params.is_null() {
                for i in 0..n {
                    let p = params.add(i * PARAMS_SIZE);
                    core::ptr::write_unaligned(p.add(16).cast::<usize>(), 0);
                    core::ptr::write_unaligned(p.add(24).cast::<u32>(), 0);
                }
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
