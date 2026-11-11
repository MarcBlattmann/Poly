//! Our `comctl32.dll` (ADR 0155, step 9): `InitCommonControlsEx`, which Chrome's browser
//! delay-loads and calls once when it starts (a failure is fatal for it). The common control
//! window classes themselves are not implemented yet: a program that creates one gets no
//! window. Learn, "InitCommonControlsEx function (commctrl.h)"; not observed on Windows.

#![no_std]
#![allow(clippy::missing_safety_doc)]

poly_win32_rt::process_heap_allocator!();

/// `InitCommonControlsEx(picce)`: TRUE for an `INITCOMMONCONTROLSEX` of the right size.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitCommonControlsEx(init: *const u32) -> i32 {
    if init.is_null() {
        return 0;
    }
    // SAFETY: the caller's INITCOMMONCONTROLSEX; `dwSize` is its first member.
    i32::from(poly_kernelbase::session::init_common_controls(unsafe { init.read_unaligned() }))
}

/// `InitCommonControls()`: "Registers and initializes certain common control window classes"
/// (Learn); it returns nothing.
#[unsafe(no_mangle)]
pub extern "system" fn InitCommonControls() {}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn comctl32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
