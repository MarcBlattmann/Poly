//! Our `audioses.dll` (audio plan W2): the audio session classes are served from here. **A
//! scaffold:** W2's sheets give them their behaviour method by method. Until then
//! `DllGetClassObject` answers as a class that is not there (`REGDB_E_CLASSNOTREG`).

#![no_std]
#![allow(clippy::missing_safety_doc)]

use core::ffi::c_void;
use poly_win32_rt as _;

/// `REGDB_E_CLASSNOTREG` (`winerror.h`).
const REGDB_E_CLASSNOTREG: i32 = 0x8004_0154_u32 as i32;
/// `E_POINTER` (`winerror.h`).
const E_POINTER: i32 = 0x8000_4003_u32 as i32;
/// `S_FALSE` (`winerror.h`).
const S_FALSE: i32 = 1;

/// `DllGetClassObject` (Learn): no class is served yet; the out pointer is cleared.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    _clsid: *const c_void,
    _iid: *const c_void,
    out: *mut *mut c_void,
) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out pointer.
    unsafe { *out = core::ptr::null_mut() };
    REGDB_E_CLASSNOTREG
}

/// `DllCanUnloadNow` (Learn): a system DLL stays loaded.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> i32 {
    S_FALSE
}

/// The entry point (`DllMain`).
#[unsafe(no_mangle)]
pub extern "system" fn audioses_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
