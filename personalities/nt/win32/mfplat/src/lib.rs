//! Our `mfplat.dll` (ADR 0155, step 9): Media Foundation's platform DLL on a system that has
//! no Media Foundation. Chrome's GPU process delay-loads it and calls `MFStartup` once; the
//! call may fail (the fault-injection table of the gap map: tolerated), the DLL may not be
//! missing. The other `MF*` functions Chrome imports stay unbound: nothing calls them once
//! `MFStartup` has failed.
//!
//! Learn, "MFStartup function (mfapi.h)": "If this function succeeds, it returns S_OK.
//! Otherwise, it returns an HRESULT error code"; `MF_E_DISABLED_IN_SAFEMODE` is its example of
//! a platform that cannot start. Poly answers `E_NOTIMPL`. Not observed on Windows.

#![no_std]

use poly_win32_rt as _;

/// `E_NOTIMPL` (`winerror.h`).
const E_NOTIMPL: u32 = 0x8000_4001;

/// `MFStartup(Version, dwFlags)`: the platform cannot be started.
#[unsafe(no_mangle)]
pub extern "system" fn MFStartup(_version: u32, _flags: u32) -> u32 {
    E_NOTIMPL
}

/// `MFShutdown()`: there is nothing to shut down.
#[unsafe(no_mangle)]
pub extern "system" fn MFShutdown() -> u32 {
    E_NOTIMPL
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn mfplat_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
