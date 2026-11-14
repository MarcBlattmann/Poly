//! Our `tbs.dll` (ADR 0155, step 12). Chrome's browser delay-loads it to ask about the TPM; with
//! the DLL missing the delay load ended the browser. Poly has no TPM: both functions return the
//! `TBS_RESULT` `TBS_E_TPM_NOT_FOUND` (`tbs.h`, 0x8028400F; Learn, "Tbsi_GetDeviceInfo": "A
//! compatible Trusted Platform Module (TPM) Security Device cannot be found on this computer").

#![no_std]

use poly_win32_rt as _;

poly_win32_rt::fail_exports!(
    "tbs",
    value(0x8028_400F),
    [
        Tbsi_GetDeviceInfo,
        Tbsip_Submit_Command,
    ]
);

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn tbs_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
