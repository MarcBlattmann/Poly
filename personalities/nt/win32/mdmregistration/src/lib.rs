//! Our `mdmregistration.dll` (ADR 0196, the sheet of `IsDeviceRegisteredWithManagement`): Poly is
//! not enrolled with a mobile-device-management service. Chrome's browser loads the library and
//! resolves two functions by name to find out whether the machine is managed.
//!
//! `IsDeviceRegisteredWithManagement(BOOL *registered, DWORD cchUpn, LPWSTR upn)` (Learn,
//! `mdmregistration.h`) on a machine that is not enrolled: `S_OK`, `*registered` FALSE, the buffer
//! untouched, NULL arguments tolerated. `IsManagementRegistrationAllowed` has no sheet; it
//! answers like the first (`S_OK`, FALSE: Poly offers no enrolment).

#![no_std]

use poly_win32_rt as _;

/// `IsDeviceRegisteredWithManagement`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsDeviceRegisteredWithManagement(registered: *mut i32, _cch: u32, _upn: *mut u16) -> i32 {
    if !registered.is_null() {
        // SAFETY: the caller's out variable.
        unsafe { registered.write(0) };
    }
    0
}

/// `IsManagementRegistrationAllowed`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsManagementRegistrationAllowed(allowed: *mut i32) -> i32 {
    if !allowed.is_null() {
        // SAFETY: the caller's out variable.
        unsafe { allowed.write(0) };
    }
    0
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn mdmregistration_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
