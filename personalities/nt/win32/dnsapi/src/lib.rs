//! Our `dnsapi.dll` (ADR 0155). Chrome loads it by name and looks up
//! `DnsIsZtEnabled` (zero trust DNS: off) with `GetProcAddress`; its own
//! resolver does not use `DnsQuery`, so only the few functions programs
//! probe are here. Names are resolved by `ws2_32!getaddrinfo`.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use poly_win32_netrt as _;

/// `DnsIsZtEnabled()`: `BOOL`, zero trust DNS is not enabled.
#[unsafe(no_mangle)]
pub extern "system" fn DnsIsZtEnabled() -> i32 {
    0
}

/// `DnsFree(pData, FreeType)`: Poly's `dnsapi` hands out no records.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DnsFree(_data: *mut u8, _free_type: u32) {}

/// `DnsFlushResolverCache()`: `BOOL`; Poly's resolver keeps no cache.
#[unsafe(no_mangle)]
pub extern "system" fn DnsFlushResolverCache() -> i32 {
    1
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn dnsapi_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
