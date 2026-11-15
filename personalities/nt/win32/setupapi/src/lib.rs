//! Our `setupapi.dll` (ADR 0155, step 12). Chrome's browser delay-loads it (device enumeration
//! for its hardware and HID code) once the page load has started; with the DLL missing the
//! delay load ended the browser (`0xC06D007E`). Every export fails: the functions that return a
//! device information set or a registry key return `INVALID_HANDLE_VALUE` (Learn,
//! "SetupDiGetClassDevsW", "SetupDiCreateDeviceInfoList", "SetupDiOpenDevRegKey": "returns
//! INVALID_HANDLE_VALUE ... call GetLastError"), the others `FALSE`, all with last error
//! `ERROR_CALL_NOT_IMPLEMENTED` (the rule of ADR 0155 for what the system does not provide).

#![no_std]

use poly_win32_rt as _;

poly_win32_rt::fail_exports!(
    "setupapi",
    win32,
    [
        SetupDiDestroyDeviceInfoList,
        SetupDiEnumDeviceInfo,
        SetupDiEnumDeviceInterfaces,
        SetupDiGetDeviceInterfaceDetailW,
        SetupDiGetDevicePropertyW,
        SetupDiOpenDeviceInfoW,
        SetupDiOpenDeviceInterfaceW,
    ]
);

/// `SetupDiCreateDeviceInfoList`: not implemented; fails with `INVALID_HANDLE_VALUE` and last error
/// `ERROR_CALL_NOT_IMPLEMENTED` (see the module docs).
#[unsafe(no_mangle)]
pub extern "system" fn SetupDiCreateDeviceInfoList() -> u64 {
    poly_win32_rt::unimplemented_export("stub: setupapi!SetupDiCreateDeviceInfoList", true, u64::MAX)
}

/// `SetupDiGetClassDevsW`: not implemented; fails with `INVALID_HANDLE_VALUE` and last error
/// `ERROR_CALL_NOT_IMPLEMENTED` (see the module docs).
#[unsafe(no_mangle)]
pub extern "system" fn SetupDiGetClassDevsW() -> u64 {
    poly_win32_rt::unimplemented_export("stub: setupapi!SetupDiGetClassDevsW", true, u64::MAX)
}

/// `SetupDiOpenDevRegKey`: not implemented; fails with `INVALID_HANDLE_VALUE` and last error
/// `ERROR_CALL_NOT_IMPLEMENTED` (see the module docs).
#[unsafe(no_mangle)]
pub extern "system" fn SetupDiOpenDevRegKey() -> u64 {
    poly_win32_rt::unimplemented_export("stub: setupapi!SetupDiOpenDevRegKey", true, u64::MAX)
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn setupapi_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
