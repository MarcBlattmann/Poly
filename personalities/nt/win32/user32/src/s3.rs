//! The S3 delta of the Chrome gap map (ADR 0172, `specs/windows/apps/chrome/gap-map.md`
//! section 8): the user32 calls Chrome's window code makes that no earlier
//! slice had. Where Poly has nothing to say (one layout, no hooks that
//! fire, no display-config adapter) the answers are the plainest ones a
//! browser accepts.

use super::{EnumProc, H, SYS, call, info, set_last_error, wide};
use alloc::vec::Vec;
use poly_win32k::kbd;
use poly_win32k::proto::{Reply, Request};
use poly_win32k::user::Env;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetTickCount() -> u32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn PolyGdiDisplayWindow(hdc: usize) -> usize;
}

const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
const ERROR_NOT_SUPPORTED: u32 = 50;

/// `RegisterWindowMessageW`: the string atom (0xC000..0xFFFF) of the name.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegisterWindowMessageW(name: *const u16) -> u32 {
    if name.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: a NUL-terminated name.
    let name = unsafe { wide(name, -1) };
    if name.is_empty() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    match call(Request::RegisterMessage { name }) {
        Reply::Atom { atom } => u32::from(atom),
        _ => 0,
    }
}

/// `EnumThreadWindows`: the top-level windows of a thread, top first;
/// TRUE when the callback never stopped.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumThreadWindows(tid: u32, f: Option<EnumProc>, l: isize) -> i32 {
    let Some(f) = f else { return 0 };
    let mut mine = Vec::new();
    let mut h = super::GetWindow(super::GetDesktopWindow(), 5);
    while h != 0 {
        if info(h).is_some_and(|i| i.tid == tid) {
            mine.push(h);
        }
        h = super::GetWindow(h, 2);
    }
    for w in mine {
        // SAFETY: the caller's callback.
        if unsafe { f(w, l) } == 0 {
            return 0;
        }
    }
    1
}

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

/// `WDA_NONE` for a window.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowDisplayAffinity(hwnd: H, affinity: *mut u32) -> i32 {
    if info(hwnd).is_none() || affinity.is_null() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    // SAFETY: the caller's DWORD.
    unsafe { *affinity = 0 };
    1
}

/// No system menu objects exist yet: NULL.
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemMenu(_hwnd: H, _revert: i32) -> usize {
    0
}

/// `WindowFromDC`: the window of a window DC, NULL for the screen's.
#[unsafe(no_mangle)]
pub extern "system" fn WindowFromDC(hdc: usize) -> H {
    // SAFETY: our gdi32.
    let w = unsafe { PolyGdiDisplayWindow(hdc) };
    if w == usize::MAX { 0 } else { w }
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

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetKeyboardLayoutNameW(buf: *mut u16) -> i32 {
    if buf.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    for (i, c) in kbd::KLID.encode_utf16().enumerate() {
        // SAFETY: KL_NAMELENGTH (9) units.
        unsafe { *buf.add(i) = c };
    }
    // SAFETY: as above.
    unsafe { *buf.add(kbd::KLID.len()) = 0 };
    1
}

// ----- small queries

#[unsafe(no_mangle)]
pub extern "system" fn GetCaretBlinkTime() -> u32 {
    530
}

/// `LASTINPUTINFO` (cbSize 8, dwTime): now.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetLastInputInfo(info: *mut u32) -> i32 {
    // SAFETY: the caller's LASTINPUTINFO.
    if info.is_null() || unsafe { *info } != 8 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: as above.
    unsafe { *info.add(1) = GetTickCount() };
    1
}

/// No pointer devices: a count of 0.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetPointerDevices(count: *mut u32, _devices: *mut u8) -> i32 {
    if count.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's count.
    unsafe { *count = 0 };
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn RegisterPointerDeviceNotifications(_hwnd: H, _range: i32) -> i32 {
    1
}

// ----- notifications that never fire

static NEXT_NOTIFY: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0x0009_0000);

fn notify_handle() -> usize {
    NEXT_NOTIFY.fetch_add(8, core::sync::atomic::Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub extern "system" fn RegisterDeviceNotificationW(_recipient: usize, _filter: *const u8, _flags: u32) -> usize {
    notify_handle()
}

#[unsafe(no_mangle)]
pub extern "system" fn UnregisterDeviceNotification(_h: usize) -> i32 {
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn RegisterPowerSettingNotification(_recipient: usize, _guid: *const u8, _flags: u32) -> usize {
    notify_handle()
}

#[unsafe(no_mangle)]
pub extern "system" fn UnregisterPowerSettingNotification(_h: usize) -> i32 {
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn RegisterSuspendResumeNotification(_recipient: usize, _flags: u32) -> usize {
    notify_handle()
}

#[unsafe(no_mangle)]
pub extern "system" fn UnregisterSuspendResumeNotification(_h: usize) -> u32 {
    0
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

#[unsafe(no_mangle)]
pub extern "system" fn NotifyWinEvent(_event: u32, _hwnd: H, _object: i32, _child: i32) {}

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
