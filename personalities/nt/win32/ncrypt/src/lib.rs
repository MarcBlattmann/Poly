//! Our `ncrypt.dll` (ADR 0155, step 12). Chrome's browser delay-loads it (key storage); a delay
//! load of a missing DLL ends the browser. Every function returns its `SECURITY_STATUS`
//! directly: `NTE_NOT_SUPPORTED` (`winerror.h`, 0x80090029; Learn, "NCryptOpenStorageProvider":
//! the function returns a status code).

#![no_std]

use poly_win32_rt as _;

poly_win32_rt::fail_exports!(
    "ncrypt",
    value(0x8009_0029),
    [
        NCryptCreateClaim,
        NCryptCreatePersistedKey,
        NCryptDeleteKey,
        NCryptExportKey,
        NCryptFinalizeKey,
        NCryptFreeObject,
        NCryptGetProperty,
        NCryptImportKey,
        NCryptIsAlgSupported,
        NCryptOpenKey,
        NCryptOpenStorageProvider,
        NCryptSetProperty,
        NCryptSignHash,
    ]
);

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn ncrypt_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

mod apps;
