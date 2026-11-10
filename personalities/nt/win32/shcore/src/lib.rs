//! Our `shcore.dll` (ADR 0172): `GetDpiForMonitor` and the legacy process
//! DPI awareness calls over `user32`. One monitor at 96 DPI in every
//! awareness mode (`GetDpiForMonitor.yaml`).

#![no_std]
#![allow(clippy::missing_safety_doc)]

use poly_win32_gui::set_last_error;
use poly_win32_rt as _;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetMonitorInfoW(monitor: usize, info: *mut u8) -> i32;
    fn GetThreadDpiAwarenessContext() -> isize;
    fn GetAwarenessFromDpiAwarenessContext(ctx: isize) -> i32;
    fn SetProcessDpiAwarenessContext(ctx: isize) -> i32;
}

const S_OK: i32 = 0;
const E_ACCESSDENIED: i32 = 0x8007_0005u32 as i32;
const E_HANDLE: i32 = 0x8007_0006u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const HR_NOT_SUPPORTED: i32 = 0x8007_0032u32 as i32;
const HR_BAD_ARGUMENTS: i32 = 0x8007_00A0u32 as i32;
const HR_INVALID_ADDRESS: i32 = 0x8007_01E7u32 as i32;

/// `MDT_EFFECTIVE_DPI`, `MDT_ANGULAR_DPI`, `MDT_RAW_DPI`.
const MDT_RAW_DPI: i32 = 2;

fn fail(win32: u32, hr: i32) -> i32 {
    set_last_error(win32);
    hr
}

/// `GetDpiForMonitor`: 96 x 96 for the effective and angular types;
/// the raw type is not supported; the type is checked before the monitor.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetDpiForMonitor(monitor: usize, kind: i32, dpi_x: *mut u32, dpi_y: *mut u32) -> i32 {
    if !(0..=2).contains(&kind) {
        return fail(0xA0, HR_BAD_ARGUMENTS);
    }
    let mut mi = [0u8; 40];
    mi[0] = 40;
    // SAFETY: a MONITORINFO of the right size.
    if unsafe { GetMonitorInfoW(monitor, mi.as_mut_ptr()) } == 0 {
        return fail(6, E_HANDLE);
    }
    if dpi_x.is_null() || dpi_y.is_null() {
        return fail(0x1E7, HR_INVALID_ADDRESS);
    }
    let v = if kind == MDT_RAW_DPI { 0 } else { 96 };
    // SAFETY: the caller's outputs.
    unsafe {
        *dpi_x = v;
        *dpi_y = v;
    }
    if kind == MDT_RAW_DPI { fail(0x32, HR_NOT_SUPPORTED) } else { S_OK }
}

/// `GetProcessDpiAwareness`: the calling thread's awareness for NULL or
/// the current process, E_INVALIDARG and -1 for another handle.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetProcessDpiAwareness(process: usize, value: *mut i32) -> i32 {
    if value.is_null() {
        return E_INVALIDARG;
    }
    if process != 0 && process != usize::MAX {
        // SAFETY: the caller's output.
        unsafe { *value = -1 };
        return E_INVALIDARG;
    }
    // SAFETY: user32 calls without preconditions.
    let a = unsafe { GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()) };
    // SAFETY: the caller's output.
    unsafe { *value = a };
    S_OK
}

/// `SetProcessDpiAwareness` (`PROCESS_DPI_AWARENESS` 0..=2): once.
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessDpiAwareness(value: i32) -> i32 {
    let ctx = match value {
        0 => -1,
        1 => -2,
        2 => -3,
        _ => return E_INVALIDARG,
    };
    // SAFETY: a user32 call without preconditions.
    if unsafe { SetProcessDpiAwarenessContext(ctx) } != 0 { S_OK } else { E_ACCESSDENIED }
}

/// `GetScaleFactorForMonitor`: 100 percent.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetScaleFactorForMonitor(monitor: usize, scale: *mut i32) -> i32 {
    let mut mi = [0u8; 40];
    mi[0] = 40;
    // SAFETY: a MONITORINFO of the right size.
    if scale.is_null() || unsafe { GetMonitorInfoW(monitor, mi.as_mut_ptr()) } == 0 {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's output.
    unsafe { *scale = 100 };
    S_OK
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn shcore_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
