//! The S3 delta of the Chrome gap map (ADR 0172, `specs/windows/apps/chrome/gap-map.md`
//! section 8): the user32 calls Chrome's window code makes that no earlier
//! slice had. Where Poly has nothing to say (one layout, no hooks that
//! fire, no display-config adapter) the answers are the plainest ones a
//! browser accepts.

use super::{H, SYS};
use poly_win32k::kbd;
use poly_win32k::user::Env;

const ERROR_NOT_SUPPORTED: u32 = 50;

// `GetWindowPlacement` is slice B's now (`lib.rs`, MonitorFromWindow.yaml `state_rects`).

#[unsafe(no_mangle)]
pub extern "system" fn IsWindowArranged(_hwnd: H) -> i32 {
    0
}

// ----- keyboard layouts: the oracle's three installed ones

const LAYOUTS: [u32; 3] = [kbd::HKL, 0x040C_040C, 0x1007_0807];

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetKeyboardLayoutList(n: i32, list: *mut usize) -> i32 {
    if n < 0 {
        // ERROR_NOACCESS (GetKeyboardLayoutList.yaml `small_buffers`: a negative size).
        super::set_last_error(998);
        return 0;
    }
    if n == 0 || list.is_null() {
        return LAYOUTS.len() as i32;
    }
    let k = (n as usize).min(LAYOUTS.len());
    // The first is the layout keys are read with: the desktop's (ADR 0209).
    let mut layouts = LAYOUTS;
    layouts[0] = crate::refresh_layout().hkl();
    for (i, l) in layouts.iter().take(k).enumerate() {
        // SAFETY: `n` HKLs of the caller.
        unsafe { *list.add(i) = *l as usize };
    }
    k as i32
}

/// Whether an `HKL` is one of the installed layouts (for `imm32`).
#[unsafe(no_mangle)]
pub extern "system" fn PrivateLayoutInstalled(hkl: usize) -> i32 {
    // The desktop's layout is installed too (ADR 0209).
    i32::from(LAYOUTS.iter().any(|&l| l as usize == hkl) || kbd::active().hkl() as usize == hkl)
}

// ----- small queries

// ----- display configuration: no adapter to describe

#[unsafe(no_mangle)]
pub extern "system" fn GetDisplayConfigBufferSizes(_flags: u32, _paths: *mut u32, _modes: *mut u32) -> i32 {
    ERROR_NOT_SUPPORTED as i32
}

#[unsafe(no_mangle)]
pub extern "system" fn QueryDisplayConfig(
    _flags: u32,
    _np: *mut u32,
    _paths: *mut u8,
    _nm: *mut u32,
    _modes: *mut u8,
    _topology: *mut u32,
) -> i32 {
    ERROR_NOT_SUPPORTED as i32
}

#[unsafe(no_mangle)]
pub extern "system" fn DisplayConfigGetDeviceInfo(_packet: *mut u8) -> i32 {
    ERROR_NOT_SUPPORTED as i32
}

#[allow(dead_code)]
fn keep() -> u32 {
    SYS.tid()
}
