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
use core::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use poly_win32k::props::{Key, Props};
use poly_win32k::spi;

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

/// `DISPLAY_DEVICE_*` (`wingdi.h`).
const DISPLAY_DEVICE_ATTACHED_TO_DESKTOP: u32 = 1;
const DISPLAY_DEVICE_PRIMARY_DEVICE: u32 = 4;
const DISPLAY_DEVICE_ACTIVE: u32 = 1;
const DISPLAY_DEVICE_ATTACHED: u32 = 2;

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

/// `EnumDisplayDevicesW`: one adapter, `\\.\DISPLAY1`, with one monitor.
/// `DISPLAY_DEVICEW` is 840 bytes (`cb`, name 32, string 128, flags, id
/// 128, key 128 characters).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumDisplayDevicesW(device: *const u16, index: u32, dd: *mut u8, _flags: u32) -> i32 {
    if dd.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's structure starts with cb.
    if unsafe { core::ptr::read_unaligned(dd as *const u32) } != 840 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: a NUL-terminated name when not NULL.
    let name = if device.is_null() { Vec::new() } else { unsafe { wide(device, -1) } };
    let adapter = name.is_empty();
    let display1 = !adapter && name.iter().copied().eq("\\\\.\\DISPLAY1".encode_utf16());
    if index != 0 || !(adapter || display1) {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let (dev_name, dev_string, state, id) = if adapter {
        (
            "\\\\.\\DISPLAY1",
            "Poly Display Adapter",
            DISPLAY_DEVICE_ATTACHED_TO_DESKTOP | DISPLAY_DEVICE_PRIMARY_DEVICE,
            "PCI\\VEN_1234&DEV_1111",
        )
    } else {
        ("\\\\.\\DISPLAY1\\Monitor0", "Generic PnP Monitor", DISPLAY_DEVICE_ACTIVE | DISPLAY_DEVICE_ATTACHED, "MONITOR\\POL0001")
    };
    // SAFETY: the caller's 840-byte structure.
    unsafe {
        core::ptr::write_bytes(dd.add(4), 0, 836);
        put_wide(dd.add(4), 32, dev_name);
        put_wide(dd.add(68), 128, dev_string);
        core::ptr::write_unaligned(dd.add(324) as *mut u32, state);
        put_wide(dd.add(328), 128, id);
    }
    1
}

/// `ENUM_CURRENT_SETTINGS`, `ENUM_REGISTRY_SETTINGS` (`wingdi.h`).
const ENUM_CURRENT_SETTINGS: u32 = 0xFFFF_FFFF;
const ENUM_REGISTRY_SETTINGS: u32 = 0xFFFF_FFFE;

/// `EnumDisplaySettingsW`: the one mode, as the current and the registry
/// settings and as mode 0. `DEVMODEW` is 220 bytes; the fields set are
/// `DM_POSITION | DM_BITSPERPEL | DM_PELSWIDTH | DM_PELSHEIGHT |
/// DM_DISPLAYFLAGS | DM_DISPLAYFREQUENCY`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumDisplaySettingsW(device: *const u16, mode: u32, dm: *mut u8) -> i32 {
    if dm.is_null() || !(mode == 0 || mode == ENUM_CURRENT_SETTINGS || mode == ENUM_REGISTRY_SETTINGS) {
        return 0;
    }
    if !device.is_null() {
        // SAFETY: a NUL-terminated name.
        let name = unsafe { wide(device, -1) };
        if !name.iter().copied().eq("\\\\.\\DISPLAY1".encode_utf16()) {
            return 0;
        }
    }
    let d = u::screen(&SYS);
    // SAFETY: the caller's 220-byte DEVMODEW.
    unsafe {
        core::ptr::write_bytes(dm, 0, 220);
        put_wide(dm, 32, "\\\\.\\DISPLAY1");
        core::ptr::write_unaligned(dm.add(64) as *mut u16, 0x0401); // dmSpecVersion
        core::ptr::write_unaligned(dm.add(68) as *mut u16, 220); // dmSize
        core::ptr::write_unaligned(dm.add(72) as *mut u32, 0x20 | 0x4_0000 | 0x8_0000 | 0x10_0000 | 0x20_0000 | 0x40_0000);
        core::ptr::write_unaligned(dm.add(168) as *mut u32, 32); // dmBitsPerPel
        core::ptr::write_unaligned(dm.add(172) as *mut u32, d.width as u32);
        core::ptr::write_unaligned(dm.add(176) as *mut u32, d.height as u32);
        core::ptr::write_unaligned(dm.add(184) as *mut u32, 60); // dmDisplayFrequency
    }
    1
}

/// `EnumDisplayMonitors`: the one monitor, with its rectangle clipped to
/// `rect` when given. `hdc` is only checked for being non-NULL when a
/// clip is wanted from it, which is not modelled.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumDisplayMonitors(
    _hdc: H,
    clip: *const i32,
    f: Option<unsafe extern "system" fn(H, H, *mut i32, isize) -> i32>,
    l: isize,
) -> i32 {
    let Some(f) = f else {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    };
    let d = u::screen(&SYS);
    let mut r = [0, 0, d.width, d.height];
    if !clip.is_null() {
        // SAFETY: the caller's RECT.
        let c = unsafe { [*clip, *clip.add(1), *clip.add(2), *clip.add(3)] };
        r = [r[0].max(c[0]), r[1].max(c[1]), r[2].min(c[2]), r[3].min(c[3])];
        if r[2] <= r[0] || r[3] <= r[1] {
            return 1;
        }
    }
    // SAFETY: the caller's callback gets a RECT of ours.
    unsafe { f(PRIMARY_MONITOR, 0, r.as_mut_ptr(), l) };
    1
}

// ----- keyboard layout

/// `GetKeyboardLayoutList`: the one layout; with `n` 0 the count.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetKeyboardLayoutList(n: i32, list: *mut usize) -> i32 {
    if n <= 0 {
        return 1;
    }
    if list.is_null() {
        return 0;
    }
    // SAFETY: the caller's array of `n >= 1` handles.
    unsafe { *list = kbd::HKL as usize };
    1
}

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

// ----- DPI

/// `DPI_AWARENESS_CONTEXT` values (`windef.h`): the pseudo handles
/// programs pass, and the values `GetThreadDpiAwarenessContext` returns.
const CTX_UNAWARE: isize = -1;
const CTX_SYSTEM_AWARE: isize = -2;
const CTX_PER_MONITOR: isize = -3;
const CTX_PER_MONITOR_V2: isize = -4;
const CTX_UNAWARE_GDISCALED: isize = -5;

/// The process's awareness (as the `CTX_*` pseudo handle) once set; 0 = never set.
static PROCESS_DPI: AtomicIsize = AtomicIsize::new(0);
/// 0: default (the process's); else the thread override's handle. One slot:
/// threads here share a single DPI of 96, so only the value round-trips.
static THREAD_DPI: AtomicIsize = AtomicIsize::new(0);
/// `DPI_AWARENESS_INVALID`.
const DPI_AWARENESS_INVALID: i32 = -1;

fn valid_context(ctx: isize) -> bool {
    matches!(ctx, CTX_UNAWARE | CTX_SYSTEM_AWARE | CTX_PER_MONITOR | CTX_PER_MONITOR_V2 | CTX_UNAWARE_GDISCALED)
        // The values Get...DpiAwarenessContext hands out.
        || matches!(ctx, 0x10 | 0x11 | 0x12 | 0x22 | 0x6010)
}

/// Normalise a handle or returned value to the `CTX_*` pseudo handle.
fn normal(ctx: isize) -> isize {
    match ctx {
        0x10 => CTX_UNAWARE,
        0x11 => CTX_SYSTEM_AWARE,
        0x12 => CTX_PER_MONITOR,
        0x22 => CTX_PER_MONITOR_V2,
        0x6010 => CTX_UNAWARE_GDISCALED,
        c => c,
    }
}

/// The value the Get functions return for a context.
fn exposed(ctx: isize) -> isize {
    match ctx {
        CTX_UNAWARE => 0x10,
        CTX_SYSTEM_AWARE => 0x11,
        CTX_PER_MONITOR => 0x12,
        CTX_PER_MONITOR_V2 => 0x22,
        _ => 0x6010,
    }
}

fn current_context() -> isize {
    let t = THREAD_DPI.load(Ordering::Relaxed);
    if t != 0 {
        return t;
    }
    match PROCESS_DPI.load(Ordering::Relaxed) {
        0 => CTX_UNAWARE,
        p => p,
    }
}

/// `SetProcessDpiAwarenessContext`: once per process (`ERROR_ACCESS_DENIED`
/// after that). The process keeps drawing at 96 DPI on the one monitor; the
/// context is only remembered.
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessDpiAwarenessContext(ctx: isize) -> i32 {
    if !valid_context(ctx) {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    match PROCESS_DPI.compare_exchange(0, normal(ctx), Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => 1,
        Err(_) => {
            set_last_error(5); // ERROR_ACCESS_DENIED
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn SetProcessDPIAware() -> i32 {
    SetProcessDpiAwarenessContext(CTX_SYSTEM_AWARE)
}

#[unsafe(no_mangle)]
pub extern "system" fn IsProcessDPIAware() -> i32 {
    i32::from(matches!(PROCESS_DPI.load(Ordering::Relaxed), CTX_SYSTEM_AWARE | CTX_PER_MONITOR | CTX_PER_MONITOR_V2))
}

/// `SetThreadDpiAwarenessContext`: the previous context (NULL for an invalid one).
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadDpiAwarenessContext(ctx: isize) -> isize {
    if !valid_context(ctx) {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let old = current_context();
    THREAD_DPI.store(normal(ctx), Ordering::Relaxed);
    exposed(old)
}

#[unsafe(no_mangle)]
pub extern "system" fn GetThreadDpiAwarenessContext() -> isize {
    exposed(current_context())
}

#[unsafe(no_mangle)]
pub extern "system" fn GetWindowDpiAwarenessContext(hwnd: H) -> isize {
    if info(hwnd).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    exposed(match PROCESS_DPI.load(Ordering::Relaxed) {
        0 => CTX_UNAWARE,
        p => p,
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn GetDpiAwarenessContextForProcess(_process: usize) -> isize {
    exposed(match PROCESS_DPI.load(Ordering::Relaxed) {
        0 => CTX_UNAWARE,
        p => p,
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn IsValidDpiAwarenessContext(ctx: isize) -> i32 {
    i32::from(valid_context(ctx))
}

#[unsafe(no_mangle)]
pub extern "system" fn AreDpiAwarenessContextsEqual(a: isize, b: isize) -> i32 {
    if !valid_context(a) || !valid_context(b) {
        return 0;
    }
    i32::from(normal(a) == normal(b))
}

/// `GetAwarenessFromDpiAwarenessContext`: `DPI_AWARENESS_*` (0 unaware, 1
/// system, 2 per monitor) or -1.
#[unsafe(no_mangle)]
pub extern "system" fn GetAwarenessFromDpiAwarenessContext(ctx: isize) -> i32 {
    if !valid_context(ctx) {
        return DPI_AWARENESS_INVALID;
    }
    match normal(ctx) {
        CTX_UNAWARE | CTX_UNAWARE_GDISCALED => 0,
        CTX_SYSTEM_AWARE => 1,
        _ => 2,
    }
}

/// `EnableNonClientDpiScaling`: accepted for a window (the frame is at 96 DPI anyway).
#[unsafe(no_mangle)]
pub extern "system" fn EnableNonClientDpiScaling(hwnd: H) -> i32 {
    if info(hwnd).is_none() {
        return bad_window();
    }
    1
}

/// `GetAutoRotationState`: a desktop machine without a sensor
/// (`AR_NOSENSOR | AR_NOT_SUPPORTED`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetAutoRotationState(state: *mut u32) -> i32 {
    if state.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *state = 0x10 | 0x20 };
    1
}

/// `AdjustWindowRectExForDpi`: the frame is Windows' at 96 DPI whatever the DPI.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AdjustWindowRectExForDpi(rect: *mut i32, style: u32, menu: i32, ex: u32, dpi: u32) -> i32 {
    if dpi == 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's RECT.
    unsafe { AdjustWindowRectEx(rect, style, menu, ex) }
}

// ----- SystemParametersInfoW

/// `SystemParametersInfoW`: the `SPI_GET*` queries of [`spi`] (and
/// `SPI_GETWORKAREA`); other actions fail with `ERROR_INVALID_PARAMETER`.
pub unsafe fn system_parameters_info(action: u32, ui_param: u32, pv: *mut u8) -> i32 {
    if pv.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let cb = if spi::has_cb_size(action) {
        // SAFETY: these structures begin with cbSize.
        unsafe { core::ptr::read_unaligned(pv as *const u32) }
    } else {
        0
    };
    let w = u::screen(&SYS).work;
    match spi::query(action, ui_param, cb, [w.left, w.top, w.right, w.bottom]) {
        spi::Outcome::Bytes(b) => {
            // SAFETY: the caller's buffer for this action's value.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), pv, b.len()) };
            1
        }
        spi::Outcome::Error(e) => {
            set_last_error(e);
            0
        }
        spi::Outcome::Unknown => {
            set_last_error(ERROR_INVALID_PARAMETER);
            0
        }
    }
}

/// `SystemParametersInfoForDpi`: as `SystemParametersInfoW`; every DPI is 96 here.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SystemParametersInfoForDpi(action: u32, ui_param: u32, pv: *mut u8, _ini: u32, dpi: u32) -> i32 {
    if dpi == 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: as documented.
    unsafe { system_parameters_info(action, ui_param, pv) }
}

// ----- system colour brushes

static SYS_BRUSHES: SpinLock<[usize; 31]> = SpinLock::new([0; 31]);

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: u32) -> usize;
}

/// `GetSysColorBrush`: a brush of the system colour, made once and shared
/// (programs must not delete it); NULL for an index past `COLOR_MENUBAR` (30).
#[unsafe(no_mangle)]
pub extern "system" fn GetSysColorBrush(index: i32) -> usize {
    if !(0..=30).contains(&index) {
        return 0;
    }
    let cached = SYS_BRUSHES.with(|b| b[index as usize]);
    if cached != 0 {
        return cached;
    }
    // SAFETY: our gdi32.
    let brush = unsafe { CreateSolidBrush(GetSysColor(index)) };
    SYS_BRUSHES.with(|b| {
        if b[index as usize] == 0 {
            b[index as usize] = brush;
        }
        b[index as usize]
    })
}

// ----- icons

/// A system icon's handle: one value per `IDI_*` id (`IDI_APPLICATION` 32512 ...).
const ICON_BASE: usize = 0x0007_0000;
const IDI_FIRST: usize = 32512;
const IDI_LAST: usize = 32518;
/// Icons made with `CreateIconIndirect`.
const CREATED_ICON_BASE: usize = 0x0090_0000;
static NEXT_ICON: AtomicUsize = AtomicUsize::new(CREATED_ICON_BASE);
static CREATED_ICONS: SpinLock<Vec<usize>> = SpinLock::new(Vec::new());

fn is_system_icon(h: usize) -> bool {
    (ICON_BASE..=ICON_BASE + (IDI_LAST - IDI_FIRST)).contains(&h)
}

/// `LoadIconW`: the predefined `IDI_*` icons for a NULL module; icons in a
/// module's resources are not read (`ERROR_RESOURCE_NAME_NOT_FOUND`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadIconW(instance: H, name: *const u16) -> H {
    if instance != 0 {
        set_last_error(ERROR_RESOURCE_NAME_NOT_FOUND);
        return 0;
    }
    let id = name as usize;
    if (IDI_FIRST..=IDI_LAST).contains(&id) {
        ICON_BASE + (id - IDI_FIRST)
    } else {
        set_last_error(ERROR_RESOURCE_NAME_NOT_FOUND);
        0
    }
}

/// `LoadImageW`: system icons and cursors by id for a NULL module, and a
/// module's icon or cursor as the application icon / the arrow (the
/// resource is not read); bitmaps and files are not loaded.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadImageW(instance: H, name: *const u16, kind: u32, _cx: i32, _cy: i32, flags: u32) -> H {
    const LR_LOADFROMFILE: u32 = 0x10;
    if flags & LR_LOADFROMFILE != 0 {
        set_last_error(2); // ERROR_FILE_NOT_FOUND
        return 0;
    }
    match kind {
        // IMAGE_ICON
        1 => {
            // SAFETY: the same contract.
            let h = unsafe { LoadIconW(0, if instance == 0 { name } else { IDI_FIRST as *const u16 }) };
            if h == 0 { ICON_BASE } else { h }
        }
        // IMAGE_CURSOR
        2 => {
            // SAFETY: the same contract.
            unsafe { LoadCursorW(0, if instance == 0 { name } else { 32512 as *const u16 }) }
        }
        _ => {
            set_last_error(ERROR_RESOURCE_DATA_NOT_FOUND);
            0
        }
    }
}

/// `CreateIconIndirect`: a handle for the `ICONINFO` (bitmaps are not copied).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateIconIndirect(info: *const u8) -> H {
    if info.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // ICONINFO: fIcon u32, xHotspot u32, yHotspot u32, hbmMask, hbmColor (x64: 32 bytes).
    // SAFETY: the caller's ICONINFO.
    let (mask, color) = unsafe {
        (core::ptr::read_unaligned(info.add(16) as *const u64), core::ptr::read_unaligned(info.add(24) as *const u64))
    };
    if mask == 0 && color == 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let h = NEXT_ICON.fetch_add(0x10, Ordering::Relaxed);
    CREATED_ICONS.with(|v| v.push(h));
    h
}

/// `DestroyIcon`: icons of `CreateIconIndirect` go; shared system icons and
/// cursors stay (TRUE, as for a shared resource).
#[unsafe(no_mangle)]
pub extern "system" fn DestroyIcon(icon: H) -> i32 {
    if is_system_icon(icon) || cursor_index(icon).is_some() {
        return 1;
    }
    let found = CREATED_ICONS.with(|v| v.iter().position(|&h| h == icon).map(|i| v.swap_remove(i)).is_some());
    if found {
        1
    } else {
        set_last_error(ERROR_INVALID_CURSOR_HANDLE);
        0
    }
}
