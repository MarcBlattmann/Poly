//! The S3 delta of the Chrome gap map (ADR 0172, `specs/windows/apps/chrome/gap-map.md`
//! section 8): the user32 calls Chrome's window code makes that no earlier
//! slice had. Where Poly has nothing to say (one layout, no hooks that
//! fire, no display-config adapter) the answers are the plainest ones a
//! browser accepts.

use super::{H, SYS, info, set_last_error};
use poly_win32k::kbd;
use poly_win32k::user::Env;

const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
const ERROR_NOT_SUPPORTED: u32 = 50;

/// `WINDOWPLACEMENT` (44 bytes): flags, showCmd, min and max positions
/// (-1, -1), the normal rectangle.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowPlacement(hwnd: H, wp: *mut u8) -> i32 {
    let Some(i) = info(hwnd) else {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    };
    if wp.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let show = if i.minimized {
        2
    } else if i.maximized {
        3
    } else {
        1
    };
    let vals: [i32; 10] = [44, 0, show, -1, -1, -1, -1, i.window.left, i.window.top, i.window.right];
    // SAFETY: the caller's WINDOWPLACEMENT (44 bytes).
    unsafe {
        for (k, v) in vals.iter().enumerate() {
            core::ptr::write_unaligned(wp.add(4 * k) as *mut i32, *v);
        }
        core::ptr::write_unaligned(wp.add(40) as *mut i32, i.window.bottom);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn IsWindowArranged(_hwnd: H) -> i32 {
    0
}

// ----- keyboard layouts: the oracle's three installed ones

const LAYOUTS: [u32; 3] = [kbd::HKL, 0x040C_040C, 0x1007_0807];

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetKeyboardLayoutList(n: i32, list: *mut usize) -> i32 {
    if n <= 0 || list.is_null() {
        return LAYOUTS.len() as i32;
    }
    let k = (n as usize).min(LAYOUTS.len());
    for (i, l) in LAYOUTS.iter().take(k).enumerate() {
        // SAFETY: `n` HKLs of the caller.
        unsafe { *list.add(i) = *l as usize };
    }
    k as i32
}

/// Whether an `HKL` is one of the installed layouts (for `imm32`).
#[unsafe(no_mangle)]
pub extern "system" fn PrivateLayoutInstalled(hkl: usize) -> i32 {
    i32::from(LAYOUTS.iter().any(|&l| l as usize == hkl))
}

// ----- small queries

// ----- notifications that never fire

static NEXT_NOTIFY: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0x0009_0000);

fn notify_handle() -> usize {
    NEXT_NOTIFY.fetch_add(8, core::sync::atomic::Ordering::Relaxed)
}

// ----- WinEvents: hooks are accepted, nothing is raised

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn SetWinEventHook(
    _min: u32,
    _max: u32,
    _module: usize,
    _proc: usize,
    _pid: u32,
    _tid: u32,
    _flags: u32,
) -> usize {
    notify_handle()
}

#[unsafe(no_mangle)]
pub extern "system" fn UnhookWinEvent(_hook: usize) -> i32 {
    1
}

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

#[unsafe(no_mangle)]
pub extern "system" fn GetThreadDesktop(_tid: u32) -> usize {
    0x0000_0030
}

#[allow(dead_code)]
fn keep() -> u32 {
    SYS.tid()
}
