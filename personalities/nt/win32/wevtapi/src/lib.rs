//! Our `wevtapi.dll` (ADR 0155, step 12). Chrome delay-loads it to read the event log; a delay
//! load of a missing DLL ends the browser. Every function fails with `NULL` / `FALSE` and last
//! error `ERROR_CALL_NOT_IMPLEMENTED` (Learn, "EvtQuery": "If NULL, call GetLastError").

#![no_std]

use poly_win32_rt as _;

poly_win32_rt::fail_exports!("wevtapi", win32, [EvtClose, EvtCreateRenderContext, EvtNext, EvtQuery, EvtRender,]);

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn wevtapi_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

mod apps;
