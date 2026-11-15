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
use poly_win32k::notify::{self, Kind, Pending, Registry};
use poly_win32k::props::{Key, Props};

// ----- window properties

static PROPS: SpinLock<Props> = SpinLock::new(Props::new());
/// Sets since the table was last cleaned of destroyed windows.
static PROP_SETS: AtomicUsize = AtomicUsize::new(0);

/// The key `lpString` names: an atom below 0x10000, else the string.
unsafe fn prop_key(raw: *const u16) -> Option<Key> {
    // SAFETY: a NUL-terminated name when not an atom.
    Key::from_raw_atoms(raw as usize, || unsafe { wide(raw, -1) }, atom_name)
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GlobalGetAtomNameW(atom: u16, buffer: *mut u16, size: i32) -> u32;
    fn OpenThread(access: u32, inherit: i32, tid: u32) -> usize;
    fn CloseHandle(handle: usize) -> i32;
}

/// The string of a global atom made from one (`GlobalAddAtomW`).
fn atom_name(atom: u16) -> Option<Vec<u16>> {
    let mut buf = [0u16; 256];
    let err = poly_win32_gui::last_error();
    // SAFETY: a buffer of 256 characters.
    let n = unsafe { GlobalGetAtomNameW(atom, buf.as_mut_ptr(), 256) } as usize;
    set_last_error(err);
    (n != 0).then(|| buf[..n].to_vec())
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
    if info(hwnd).is_none() {
        // A destroyed window has no properties any more (sprop_chrome).
        return bad_window();
    }
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

/// `EnumThreadWindows`: the thread's top-level windows (no message-only
/// windows), top first, the IME windows last; stops when the callback
/// returns FALSE. TRUE even when there are none.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumThreadWindows(tid: u32, f: Option<EnumProc>, l: isize) -> i32 {
    let Some(f) = f else {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    };
    let mut mine = Vec::new();
    let mut h = GetWindow(GetDesktopWindow(), 5);
    while h != 0 {
        if info(h).is_some_and(|i| i.tid == tid) {
            mine.push(h);
        }
        h = GetWindow(h, 2);
    }
    if mine.is_empty() {
        // A thread id that names no thread (EnumThreadWindows.yaml `no_windows`): the call
        // finds nothing and says why.
        let before = poly_win32_gui::last_error();
        // SAFETY: a query for the thread's existence; the handle is closed at once.
        let t = unsafe { OpenThread(0x0800, 0, tid) };
        if t != 0 {
            // SAFETY: the handle just opened.
            unsafe { CloseHandle(t) };
        }
        let missing = t == 0 && poly_win32_gui::last_error() == ERROR_INVALID_PARAMETER;
        set_last_error(if missing { ERROR_INVALID_PARAMETER } else { before });
        return 1;
    }
    for h in crate::enumeration_order(mine) {
        // A window destroyed by an earlier callback is no longer reported; the error stays.
        if info(h).is_none() {
            set_last_error(ERROR_INVALID_WINDOW_HANDLE);
            continue;
        }
        // SAFETY: the caller's callback.
        if unsafe { f(h, l) } == 0 {
            return 0;
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
        // ERROR_NOACCESS (GetKeyboardLayoutNameW.yaml `name`).
        set_last_error(998);
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

/// `GetPointerDevices`: there are no pen or touch devices. A query with no buffer says so
/// (`*count` = 0); a call with a buffer fails and leaves both alone (GetPointerDevices.yaml).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetPointerDevices(count: *mut u32, devices: *mut u8) -> i32 {
    if count.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if !devices.is_null() {
        return 0;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *count = 0 };
    1
}

/// `GetPointerDevice`: no handle names a device; NULL is a bad parameter, anything else a bad handle.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetPointerDevice(device: usize, _info: *mut u8) -> i32 {
    set_last_error(if device == 0 { ERROR_INVALID_PARAMETER } else { ERROR_INVALID_HANDLE });
    0
}

/// `RegisterPointerDeviceNotifications`: once per window; no message ever comes (no devices).
#[unsafe(no_mangle)]
pub extern "system" fn RegisterPointerDeviceNotifications(hwnd: H, _notify_range: i32) -> i32 {
    if info(hwnd).is_none() {
        return bad_window();
    }
    i32::from(REGISTRY.with(|r| r.add_pointer(hwnd as u64, |w| info(w as H).is_some())))
}

/// `GetSystemMenu`: a window of ours has no system menu object (NULL).
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemMenu(hwnd: H, _revert: i32) -> H {
    if info(hwnd).is_none() {
        return bad_window();
    }
    0
}

/// The display affinity of the top-level windows that have one other than `WDA_NONE`.
static AFFINITY: SpinLock<Vec<(u64, u32)>> = SpinLock::new(Vec::new());

/// `WDA_NONE`, `WDA_MONITOR` and `WDA_EXCLUDEFROMCAPTURE` (winuser.h): the values `SetWindowDisplayAffinity` takes.
const WDA_VALUES: [u32; 3] = [0, 1, 0x11];

/// `GetWindowDisplayAffinity` (GetWindowDisplayAffinity.yaml): FALSE with 87 for a child window,
/// 998 for a NULL output; the desktop window and bad handles fail with the output left alone.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowDisplayAffinity(hwnd: H, affinity: *mut u32) -> i32 {
    let Some(i) = info(hwnd) else { return bad_window() };
    if affinity.is_null() {
        set_last_error(ERROR_NOACCESS);
        return 0;
    }
    if i.style & poly_win32k::metrics::WS_CHILD != 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if i.desktop {
        return 0;
    }
    // A window that is not shown (hidden, or minimized and then hidden) has no affinity to report
    // (GetWindowDisplayAffinity.yaml, test gwda_basic: 0 after minimize and hide).
    let v = if i.visible {
        AFFINITY.with(|a| {
            a.retain(|&(h, _)| info(h as H).is_some());
            a.iter().find(|&&(h, _)| h == hwnd as u64).map_or(0, |&(_, v)| v)
        })
    } else {
        0
    };
    // SAFETY: the caller's out parameter, checked for NULL.
    unsafe { *affinity = v };
    1
}

/// `SetWindowDisplayAffinity`: 0, 1 and 0x11 for a top-level window, from any thread of the process.
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowDisplayAffinity(hwnd: H, affinity: u32) -> i32 {
    let Some(i) = info(hwnd) else { return bad_window() };
    if i.desktop {
        return 0;
    }
    if i.style & poly_win32k::metrics::WS_CHILD != 0 || !WDA_VALUES.contains(&affinity) {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    AFFINITY.with(|a| {
        a.retain(|&(h, _)| h != hwnd as u64);
        if affinity != 0 {
            a.push((hwnd as u64, affinity));
        }
    });
    1
}

// ----- notification registrations (device, power, suspend and resume; `poly_win32k::notify`)

static REGISTRY: SpinLock<Registry> = SpinLock::new(Registry::new());

const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NOACCESS: u32 = 998;
const WM_POWERBROADCAST: u32 = 0x0218;
/// `PBT_POWERSETTINGCHANGE` (winuser.h).
const PBT_POWERSETTINGCHANGE: usize = 0x8013;

/// The initial messages of power-setting registrations that wait for the calling thread: it is
/// looking into its queue now.
pub fn pump_power_settings() {
    let tid = SYS.tid();
    loop {
        let Some(p) = REGISTRY.with(|r| if r.waiting_for(tid) { r.take_for(tid) } else { None }) else { return };
        // POWERBROADCAST_SETTING: the GUID, the data length (4) and the value.
        let mut data = [0u8; 24];
        data[..16].copy_from_slice(&p.guid);
        data[16..20].copy_from_slice(&4u32.to_le_bytes());
        data[20..24].copy_from_slice(&p.value.to_le_bytes());
        SendMessageW(p.hwnd as H, WM_POWERBROADCAST, PBT_POWERSETTINGCHANGE, data.as_ptr() as isize);
    }
}

/// `RegisterDeviceNotificationW`: a handle of its own for every call; no device ever arrives. The
/// recipient is not checked (RegisterDeviceNotificationW.yaml).
#[unsafe(no_mangle)]
pub extern "system" fn RegisterDeviceNotificationW(recipient: H, filter: *const u8, flags: u32) -> usize {
    if filter.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: a DEV_BROADCAST_HDR is 12 bytes (size, type, reserved) and the caller's filter has
    // at least that (sizes 12 and 31 are accepted by Windows).
    let device_type = unsafe { core::ptr::read_unaligned(filter.add(4) as *const u32) };
    if let Err(e) = notify::check_device_filter(device_type, flags) {
        set_last_error(e);
        return 0;
    }
    set_last_error(0);
    REGISTRY.with(|r| r.add(Kind::Device, recipient as u64))
}

/// Release a registration of `kind`: the error (set) or TRUE (`clear`: also sets the last error
/// to 0, which only the device kind does).
fn release(kind: Kind, handle: usize, clear: bool) -> i32 {
    match REGISTRY.with(|r| r.release(kind, handle)) {
        Ok(_) => {
            if clear {
                set_last_error(0);
            }
            1
        }
        Err(e) => {
            set_last_error(e);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn UnregisterDeviceNotification(handle: usize) -> i32 {
    release(Kind::Device, handle, true)
}

/// `RegisterPowerSettingNotification`: a handle; a setting that has a value now sends it once to
/// its window's thread when that next looks into its queue.
#[unsafe(no_mangle)]
pub extern "system" fn RegisterPowerSettingNotification(recipient: H, guid: *const u8, flags: u32) -> usize {
    if guid.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if flags & notify::DEVICE_NOTIFY_SERVICE_HANDLE != 0 {
        set_last_error(notify::ERROR_SERVICE_NOT_ACTIVE);
        return 0;
    }
    let mut g = [0u8; 16];
    // SAFETY: the caller's GUID is 16 bytes.
    unsafe { core::ptr::copy_nonoverlapping(guid, g.as_mut_ptr(), 16) };
    let tid = info(recipient).map(|i| i.tid);
    set_last_error(0);
    REGISTRY.with(|r| {
        let h = r.add(Kind::Power, recipient as u64);
        if let (Some(value), Some(tid)) = (notify::power_value(&g), tid) {
            r.defer(Pending { handle: h, hwnd: recipient as u64, tid, guid: g, value });
        }
        h
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn UnregisterPowerSettingNotification(handle: usize) -> i32 {
    release(Kind::Power, handle, false)
}

/// `RegisterSuspendResumeNotification`: a handle; the machine never suspends. The last error is
/// left alone.
#[unsafe(no_mangle)]
pub extern "system" fn RegisterSuspendResumeNotification(recipient: H, flags: u32) -> usize {
    if flags & notify::DEVICE_NOTIFY_SERVICE_HANDLE != 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    REGISTRY.with(|r| r.add(Kind::Suspend, recipient as u64))
}

/// `UnregisterSuspendResumeNotification` (`powrprof.dll`'s on Windows 8 and later, user32's name
/// on some builds).
#[unsafe(no_mangle)]
pub extern "system" fn UnregisterSuspendResumeNotification(handle: usize) -> i32 {
    release(Kind::Suspend, handle, false)
}

// ----- the per-monitor DPI functions (`api-ms-win-shcore-scaling-*`)
