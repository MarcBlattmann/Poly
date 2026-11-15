//! Our `wintrust.dll` (ADR 0155, step 12). Chrome delay-loads it to check the signatures of
//! modules; a delay load of a missing DLL ends the browser. `WinVerifyTrust` returns its status
//! directly: `TRUST_E_PROVIDER_UNKNOWN` (`winerror.h`, 0x800B0001; Learn, "WinVerifyTrust": "The
//! trust provider is not recognized on this system"). The catalog functions and the
//! `WTHelper*` functions fail with `FALSE` / `NULL` and last error `ERROR_CALL_NOT_IMPLEMENTED`.

#![no_std]

use poly_win32_rt as _;

poly_win32_rt::fail_exports!("wintrust", value(0x800B_0001), [WinVerifyTrust,]);

poly_win32_rt::fail_exports!(
    "wintrust",
    win32,
    [
        CryptCATAdminAcquireContext,
        CryptCATAdminCalcHashFromFileHandle,
        CryptCATAdminEnumCatalogFromHash,
        CryptCATAdminReleaseCatalogContext,
        CryptCATAdminReleaseContext,
        CryptCATCatalogInfoFromContext,
        WTHelperGetProvCertFromChain,
        WTHelperGetProvSignerFromChain,
        WTHelperProvDataFromStateData,
    ]
);

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn wintrust_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

#[cfg(target_arch = "x86_64")]
mod apps;
