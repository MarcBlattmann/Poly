//! Our `hid.dll` (ADR 0155, step 12). Chrome delay-loads it for game pads and other HID devices;
//! a delay load of a missing DLL ends the browser. The `HidD_*` functions return `FALSE` (with
//! last error `ERROR_CALL_NOT_IMPLEMENTED`), the `HidP_*` functions their `NTSTATUS`:
//! `HIDP_STATUS_NOT_IMPLEMENTED` (`hidpi.h`, 0xC0110020).

#![no_std]

use poly_win32_rt as _;

poly_win32_rt::fail_exports!(
    "hid",
    win32,
    [
        HidD_FreePreparsedData,
        HidD_GetAttributes,
        HidD_GetPreparsedData,
        HidD_GetProductString,
        HidD_GetSerialNumberString,
    ]
);

poly_win32_rt::fail_exports!(
    "hid",
    value(0xC011_0020),
    [
        HidP_GetButtonCaps,
        HidP_GetCaps,
        HidP_GetLinkCollectionNodes,
        HidP_GetScaledUsageValue,
        HidP_GetUsageValue,
        HidP_GetUsagesEx,
        HidP_GetValueCaps,
    ]
);

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn hid_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

#[cfg(target_arch = "x86_64")]
mod apps;
