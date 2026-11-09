//! The user32 calls of a headless browser (ADR 0155, batch 4): what Chrome
//! makes of `user32.dll` when it only has hidden and message-only windows
//! and no screen to speak of — window properties, registered window
//! messages, the DPI functions, `SystemParametersInfoW`, the keyboard
//! layout list, device and power notifications, icons, display
//! enumeration and the few queries that answer "nothing".
//!
//! The decisions that can be tested on the host are in `poly_win32k`
//! (`spi`, `props`, the `RegisterMessage` request); this is the export glue.
//! Everything else here follows the Learn page of the function.

use super::*;
use core::sync::atomic::{AtomicUsize, Ordering};
use poly_win32k::props::{Key, Props};

// ----- window properties

static PROPS: SpinLock<Props> = SpinLock::new(Props::new());
/// Sets since the table was last cleaned of destroyed windows.
static PROP_SETS: AtomicUsize = AtomicUsize::new(0);

/// The key `lpString` names: an atom below 0x10000, else the string.
unsafe fn prop_key(raw: *const u16) -> Option<Key> {
    // SAFETY: a NUL-terminated name when not an atom.
    Key::from_raw(raw as usize, || unsafe { wide(raw, -1) })
}

/// Drop the properties of windows that no longer exist.
fn purge_dead_props() {
    let owners: Vec<u64> = PROPS.with(|p| {
        let mut v = Vec::new();
        p.retain(|h| {
            v.push(h);
            true
        });
        v
    });
    let dead: Vec<u64> = owners.into_iter().filter(|&h| info(h as H).is_none()).collect();
    if !dead.is_empty() {
        PROPS.with(|p| dead.iter().for_each(|&h| p.remove_window(h)));
    }
}

/// `SetPropW`: FALSE with `ERROR_INVALID_WINDOW_HANDLE` for a bad window.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetPropW(hwnd: H, name: *const u16, data: usize) -> i32 {
    // SAFETY: the caller's name.
    let Some(key) = (unsafe { prop_key(name) }) else {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    };
    if info(hwnd).is_none() {
        return bad_window();
    }
    PROPS.with(|p| p.set(hwnd as u64, key, data));
    if PROP_SETS.fetch_add(1, Ordering::Relaxed) % 64 == 63 {
        purge_dead_props();
    }
    1
}

/// `GetPropW`: the value, or NULL when the window has no such property.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetPropW(hwnd: H, name: *const u16) -> usize {
    // SAFETY: the caller's name.
    let Some(key) = (unsafe { prop_key(name) }) else { return 0 };
    PROPS.with(|p| p.get(hwnd as u64, &key))
}

/// `RemovePropW`: the value it had, or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RemovePropW(hwnd: H, name: *const u16) -> usize {
    // SAFETY: the caller's name.
    let Some(key) = (unsafe { prop_key(name) }) else { return 0 };
    PROPS.with(|p| p.remove(hwnd as u64, &key))
}

// ----- registered window messages

/// `RegisterWindowMessageW`: a number in 0xC000..=0xFFFF, the same for the
/// same name in every process; 0 with `ERROR_INVALID_PARAMETER` for NULL,
/// the empty string or one over 255 characters.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegisterWindowMessageW(name: *const u16) -> u32 {
    if name.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: a NUL-terminated name.
    let n = unsafe { wide(name, -1) };
    match call(Request::RegisterMessage { name: n }) {
        Reply::Atom { atom } => u32::from(atom),
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

// ----- enumeration

/// `EnumThreadWindows`: the thread's top-level and message-only windows,
/// top first; stops when the callback returns FALSE. TRUE even when there
/// are none.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumThreadWindows(tid: u32, f: Option<EnumProc>, l: isize) -> i32 {
    let Some(f) = f else {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    };
    let message_parent = match call(Request::Desktop {}) {
        Reply::Desktop { message, .. } => message as H,
        _ => 0,
    };
    let mut mine = Vec::new();
    for parent in [GetDesktopWindow(), message_parent] {
        if parent == 0 {
            continue;
        }
        let mut h = GetWindow(parent, 5);
        while h != 0 {
            if info(h).is_some_and(|i| i.tid == tid) {
                mine.push(h);
            }
            h = GetWindow(h, 2);
        }
    }
    for h in mine {
        // SAFETY: the caller's callback.
        if unsafe { f(h, l) } == 0 {
            break;
        }
    }
    1
}

unsafe fn put_wide(dst: *mut u8, capacity: usize, s: &str) {
    let mut n = 0;
    for c in s.encode_utf16().take(capacity - 1) {
        // SAFETY: inside the caller's `capacity` characters.
        unsafe { core::ptr::write_unaligned(dst.add(n * 2) as *mut u16, c) };
        n += 1;
    }
    // SAFETY: the terminating NUL is inside the capacity.
    unsafe { core::ptr::write_unaligned(dst.add(n * 2) as *mut u16, 0) };
}

// ----- keyboard layout

/// `GetKeyboardLayoutNameW`: the layout's identifier, 8 hex digits and a
/// NUL (the German layout of ADR 0136: `00000407`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetKeyboardLayoutNameW(buf: *mut u16) -> i32 {
    if buf.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's buffer of KL_NAMELENGTH (9) characters.
    unsafe { put_wide(buf as *mut u8, 9, "00000407") };
    1
}

// ----- small queries

/// `GetCaretBlinkTime`: Windows' default, 530 ms.
#[unsafe(no_mangle)]
pub extern "system" fn GetCaretBlinkTime() -> u32 {
    530
}

/// `GetLastInputInfo`: `LASTINPUTINFO` (8 bytes); the time is now (the
/// session has no idle clock yet).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetLastInputInfo(plii: *mut u8) -> i32 {
    // SAFETY: the caller's structure starts with cbSize.
    if plii.is_null() || unsafe { core::ptr::read_unaligned(plii as *const u32) } != 8 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's 8-byte structure; no preconditions for the clock.
    unsafe { core::ptr::write_unaligned(plii.add(4) as *mut u32, GetTickCount()) };
    1
}

/// `GetPointerDevices`: no pointer devices are reported (`*count` = 0).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetPointerDevices(count: *mut u32, _devices: *mut u8) -> i32 {
    if count.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *count = 0 };
    1
}

/// `RegisterPointerDeviceNotifications`: accepted.
#[unsafe(no_mangle)]
pub extern "system" fn RegisterPointerDeviceNotifications(_hwnd: H, _notify_range: i32) -> i32 {
    1
}

/// `GetSystemMenu`: a window of ours has no system menu object (NULL).
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemMenu(hwnd: H, _revert: i32) -> H {
    if info(hwnd).is_none() {
        return bad_window();
    }
    0
}

/// `GetWindowDisplayAffinity`: `WDA_NONE`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowDisplayAffinity(hwnd: H, affinity: *mut u32) -> i32 {
    if affinity.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if info(hwnd).is_none() {
        return bad_window();
    }
    // SAFETY: the caller's out parameter.
    unsafe { *affinity = 0 };
    1
}

/// `SetWindowDisplayAffinity`: only `WDA_NONE` (0) is possible here.
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowDisplayAffinity(hwnd: H, affinity: u32) -> i32 {
    if info(hwnd).is_none() {
        return bad_window();
    }
    if affinity == 0 {
        1
    } else {
        set_last_error(ERROR_INVALID_PARAMETER);
        0
    }
}

/// `NotifyWinEvent`: there are no event hooks to tell.
#[unsafe(no_mangle)]
pub extern "system" fn NotifyWinEvent(_event: u32, _hwnd: H, _object: i32, _child: i32) {}

// ----- notification registrations (device, power, suspend and resume)

static NEXT_REGISTRATION: AtomicUsize = AtomicUsize::new(0x000A_0000);

fn registration() -> usize {
    NEXT_REGISTRATION.fetch_add(0x10, Ordering::Relaxed)
}

/// `RegisterDeviceNotificationW`: a handle; no device ever arrives.
#[unsafe(no_mangle)]
pub extern "system" fn RegisterDeviceNotificationW(recipient: H, filter: *const u8, _flags: u32) -> usize {
    if recipient == 0 || filter.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    registration()
}

#[unsafe(no_mangle)]
pub extern "system" fn UnregisterDeviceNotification(handle: usize) -> i32 {
    i32::from(handle != 0)
}

/// `RegisterPowerSettingNotification`: a handle; no setting ever changes.
#[unsafe(no_mangle)]
pub extern "system" fn RegisterPowerSettingNotification(recipient: H, guid: *const u8, _flags: u32) -> usize {
    if recipient == 0 || guid.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    registration()
}

#[unsafe(no_mangle)]
pub extern "system" fn UnregisterPowerSettingNotification(handle: usize) -> i32 {
    i32::from(handle != 0)
}

/// `RegisterSuspendResumeNotification`: a handle; the machine never suspends.
#[unsafe(no_mangle)]
pub extern "system" fn RegisterSuspendResumeNotification(recipient: H, _flags: u32) -> usize {
    if recipient == 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    registration()
}

/// `UnregisterSuspendResumeNotification` (`powrprof.dll`'s on Windows 8
/// and later, user32's name on some builds): FALSE for NULL.
#[unsafe(no_mangle)]
pub extern "system" fn UnregisterSuspendResumeNotification(handle: usize) -> i32 {
    i32::from(handle != 0)
}

// ----- the per-monitor DPI functions (`api-ms-win-shcore-scaling-*`)
