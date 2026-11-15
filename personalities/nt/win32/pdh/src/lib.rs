//! Our `pdh.dll` (ADR 0155, step 12). Chrome's browser delay-loads it for performance counters;
//! with the DLL missing the delay load ended the browser. Every function returns its
//! `PDH_STATUS` directly (Learn: "If the function succeeds, the return value is ERROR_SUCCESS.
//! If the function fails, the return value is a system error code or a PDH error code"):
//! `PDH_NOT_IMPLEMENTED` (`pdhmsg.h`, 0xC0000BD3).

#![no_std]

use poly_win32_rt as _;

poly_win32_rt::fail_exports!(
    "pdh",
    value(0xC000_0BD3),
    [
        PdhAddEnglishCounterW,
        PdhCloseQuery,
        PdhCollectQueryData,
        PdhGetFormattedCounterValue,
        PdhOpenQueryW,
        PdhRemoveCounter,
    ]
);

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn pdh_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

mod apps;
