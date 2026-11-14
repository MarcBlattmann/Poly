//! Our `cfgmgr32.dll` (ADR 0155, step 12). Chrome delay-loads it next to `setupapi.dll` for device
//! properties; a delay load of a missing DLL ends the browser. Every function returns its
//! `CONFIGRET` directly: `CR_CALL_NOT_IMPLEMENTED` (`cfgmgr32.h`, 0x34).

#![no_std]

use poly_win32_rt as _;

poly_win32_rt::fail_exports!(
    "cfgmgr32",
    value(0x34),
    [
        CM_Get_DevNode_PropertyW,
        CM_Get_Device_IDW,
        CM_Get_Parent,
        CM_Locate_DevNodeW,
    ]
);

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn cfgmgr32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
