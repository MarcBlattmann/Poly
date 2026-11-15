//! UnregisterPowerSettingNotification: TRUE for a handle of
//! RegisterPowerSettingNotification (last error not touched); FALSE with
//! ERROR_INVALID_PARAMETER 87 when released twice; NULL handles;
//! the handles of device and suspend/resume registrations given to it; after
//! the release no more messages arrive; releasing from another thread;
//! releasing inside the handler of the message the registration produced.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HMENU = *mut c_void;
type HBRUSH = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
type WNDPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>;
type THREADPROC = unsafe extern "system" fn(*mut c_void) -> u32;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
}

/// winuser.h
#[repr(C)]
struct WNDCLASSEXW {
    cb_size: u32,
    style: u32,
    wnd_proc: WNDPROC,
    cls_extra: i32,
    wnd_extra: i32,
    instance: HINSTANCE,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: HBRUSH,
    menu_name: *const u16,
    class_name: *const u16,
    icon_sm: *mut c_void,
}

/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct MSG {
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    time: u32,
    pt: POINT,
    private: u32,
}

const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const GWLP_USERDATA: i32 = -21;
const PM_REMOVE: u32 = 0x0001;
const QS_KEY: u32 = 0x0001;
const QS_MOUSEMOVE: u32 = 0x0002;
const QS_MOUSEBUTTON: u32 = 0x0004;
const QS_POSTMESSAGE: u32 = 0x0008;
const QS_TIMER: u32 = 0x0010;
const QS_PAINT: u32 = 0x0020;
const QS_SENDMESSAGE: u32 = 0x0040;
const QS_HOTKEY: u32 = 0x0080;
const QS_RAWINPUT: u32 = 0x0400;
const QS_INPUT: u32 = QS_MOUSEMOVE | QS_MOUSEBUTTON | QS_KEY | QS_RAWINPUT;
const QS_ALLEVENTS: u32 = QS_INPUT | QS_POSTMESSAGE | QS_TIMER | QS_PAINT | QS_HOTKEY;
const QS_ALLINPUT: u32 = QS_ALLEVENTS | QS_SENDMESSAGE;
/// A fixed-size text buffer for formatting lines.
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<300>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// This executable's instance handle.
fn instance() -> HINSTANCE {
    // SAFETY: NULL names the executable.
    unsafe { GetModuleHandleW(core::ptr::null()) }
}

/// Register a class with `proc_` (and no menu, icon or cursor).
fn register(name: &[u16], proc_: WNDPROC, style: u32, wnd_extra: i32, brush: HBRUSH) -> u16 {
    let wc = WNDCLASSEXW {
        cb_size: core::mem::size_of::<WNDCLASSEXW>() as u32,
        style,
        wnd_proc: proc_,
        cls_extra: 0,
        wnd_extra,
        instance: instance(),
        icon: core::ptr::null_mut(),
        cursor: core::ptr::null_mut(),
        background: brush,
        menu_name: core::ptr::null(),
        class_name: name.as_ptr(),
        icon_sm: core::ptr::null_mut(),
    };
    // SAFETY: a filled WNDCLASSEXW; the name is NUL-terminated and static.
    unsafe { RegisterClassExW(&wc) }
}

/// CreateWindowExW with this executable's instance.
fn create(
    ex: u32,
    class: &[u16],
    name: &[u16],
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: HWND,
    menu: usize,
) -> HWND {
    // SAFETY: NUL-terminated names; the parent is a window or NULL.
    unsafe {
        CreateWindowExW(
            ex,
            class.as_ptr(),
            if name.is_empty() { core::ptr::null() } else { name.as_ptr() },
            style,
            x,
            y,
            w,
            h,
            parent,
            menu as HMENU,
            instance(),
            core::ptr::null_mut(),
        )
    }
}

/// Dispatch every waiting message.
fn pump() -> u32 {
    let mut msg: MSG = zeroed();
    let mut n = 0;
    // SAFETY: msg is writable; it is the message just removed.
    while unsafe { PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        n += 1;
    }
    n
}

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

/// SetLastError.
fn set_error(code: u32) {
    // SAFETY: sets this thread's last-error value.
    unsafe { SetLastError(code) }
}

/// GetLastError.
fn last_error() -> u32 {
    // SAFETY: reads this thread's last-error value.
    unsafe { GetLastError() }
}

/// Dispatch everything waiting, then keep waiting up to `ms` for anything
/// further (input arrives asynchronously), dispatching as it comes.
fn settle(ms: u32) -> u32 {
    let mut n = pump();
    // SAFETY: plain timing call.
    let start = unsafe { GetTickCount() };
    loop {
        // SAFETY: plain timing call.
        let elapsed = unsafe { GetTickCount() }.wrapping_sub(start);
        if elapsed >= ms {
            break;
        }
        // SAFETY: no handles; wakes on any input or message.
        unsafe { MsgWaitForMultipleObjects(0, core::ptr::null(), 0, ms - elapsed, QS_ALLINPUT) };
        n += pump();
    }
    n
}

/// How a window handle in a message is printed: "null", the tag letter of
/// a window of this process (GWLP_USERDATA), "ours" (this process, no tag),
/// "other" (another process: the desktop's windows, the shell) or
/// "invalid".
struct W(HWND);

impl core::fmt::Display for W {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0.is_null() {
            return f.write_str("null");
        }
        let mut pid = 0u32;
        // SAFETY: plain queries; pid is writable.
        let (tid, me) = unsafe { (GetWindowThreadProcessId(self.0, &mut pid), GetCurrentProcessId()) };
        if tid == 0 {
            return f.write_str("invalid");
        }
        if pid != me {
            return f.write_str("other");
        }
        // SAFETY: a window of this process.
        let tag = unsafe { GetWindowLongPtrW(self.0, GWLP_USERDATA) };
        if (0x41..=0x5A).contains(&tag) || (0x61..=0x7A).contains(&tag) {
            f.write_char(tag as u8 as char)
        } else {
            f.write_str("ours")
        }
    }
}

const WM_POWERBROADCAST: u32 = 0x0218;
/// guiddef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct GUID {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

type HDEVNOTIFY = *mut c_void;
const DBT_DEVTYP_DEVICEINTERFACE: u32 = 5;
const PBT_POWERSETTINGCHANGE: usize = 0x8013;
/// dbt.h (DEV_BROADCAST_DEVICEINTERFACE_W with room for the NUL of the name)
#[repr(C)]
#[derive(Clone, Copy)]
struct DEV_BROADCAST_DEVICEINTERFACE_W {
    size: u32,
    device_type: u32,
    reserved: u32,
    class_guid: GUID,
    name: [u16; 1],
}

/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct POWERBROADCAST_SETTING {
    power_setting: GUID,
    data_length: u32,
    data: [u8; 4],
}

const GUID_DEVINTERFACE_USB_DEVICE: GUID = GUID { d1: 0xA5DCBF10, d2: 0x6530, d3: 0x11D2, d4: [0x90, 0x1F, 0x00, 0xC0, 0x4F, 0xB9, 0x51, 0xED] };
const GUID_CONSOLE_DISPLAY_STATE: GUID = GUID { d1: 0x6FE69556, d2: 0x704A, d3: 0x47A0, d4: [0x8F, 0x24, 0xC2, 0x8D, 0x93, 0x6F, 0xDA, 0x47] };
const GUID_MONITOR_POWER_ON: GUID = GUID { d1: 0x02731015, d2: 0x4510, d3: 0x4526, d4: [0x99, 0xE6, 0xE5, 0xA1, 0x7E, 0xBD, 0x1A, 0xEA] };

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
    fn CreateThread(
        attributes: *mut c_void,
        stack: usize,
        start: THREADPROC,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn WaitForSingleObject(handle: HANDLE, ms: u32) -> u32;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn GetTickCount() -> u32;
    fn GetExitCodeThread(thread: HANDLE, code: *mut u32) -> BOOL;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassExW(class: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: HWND,
        menu: HMENU,
        instance: HINSTANCE,
        param: *mut c_void,
    ) -> HWND;
    fn DestroyWindow(hwnd: HWND) -> BOOL;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn SendMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn RegisterDeviceNotificationW(recipient: HANDLE, filter: *const c_void, flags: u32) -> HDEVNOTIFY;
    fn UnregisterDeviceNotification(handle: HDEVNOTIFY) -> BOOL;
    fn RegisterPowerSettingNotification(recipient: HANDLE, guid: *const GUID, flags: u32) -> HDEVNOTIFY;
    fn UnregisterPowerSettingNotification(handle: HDEVNOTIFY) -> BOOL;
    fn RegisterSuspendResumeNotification(recipient: HANDLE, flags: u32) -> HDEVNOTIFY;
    fn UnregisterSuspendResumeNotification(handle: HDEVNOTIFY) -> u32;
}

poly_wtest::main!(test);

static INNER: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
static INNER_R: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(9);

unsafe extern "system" fn proc_u(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_POWERBROADCAST && w == PBT_POWERSETTINGCHANGE {
        let h = INNER.load(core::sync::atomic::Ordering::SeqCst);
        if h != 0 {
            // SAFETY: the handle was registered by the test.
            let r = unsafe { UnregisterPowerSettingNotification(h as HDEVNOTIFY) };
            INNER_R.store(r as u32, core::sync::atomic::Ordering::SeqCst);
        }
        return 1;
    }
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

unsafe extern "system" fn releaser(param: *mut c_void) -> u32 {
    // SAFETY: the handle is owned by the test.
    unsafe { UnregisterPowerSettingNotification(param as HDEVNOTIFY) as u32 }
}

fn test(t: &mut Test) {
    register(wz!("UpsClass"), Some(proc_u), 0, 0, core::ptr::null_mut());
    let none: HWND = core::ptr::null_mut();
    // SAFETY: our own window.
    unsafe {
        let w = create(0, wz!("UpsClass"), wz!("ups"), WS_OVERLAPPEDWINDOW, 0, 0, 100, 100, none, 0);
        let h = RegisterPowerSettingNotification(w, &GUID_MONITOR_POWER_ON, 0);
        set_error(0xDEAD);
        let r = UnregisterPowerSettingNotification(h);
        say!(t, "release: returns {r}, last error left alone {}", last_error() == 0xDEAD);
        set_error(0xDEAD);
        let r = UnregisterPowerSettingNotification(h);
        say!(t, "release again: returns {r}, last error {}", last_error());
        set_error(0xDEAD);
        let r = UnregisterPowerSettingNotification(none);
        say!(t, "NULL: returns {r}, last error {}", last_error());
        // other kinds of handles
        let mut f: DEV_BROADCAST_DEVICEINTERFACE_W = zeroed();
        f.size = core::mem::size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32;
        f.device_type = DBT_DEVTYP_DEVICEINTERFACE;
        f.class_guid = GUID_DEVINTERFACE_USB_DEVICE;
        let d = RegisterDeviceNotificationW(w, &f as *const _ as *const c_void, 0);
        let s = RegisterSuspendResumeNotification(w, 0);
        set_error(0xDEAD);
        let rd = UnregisterPowerSettingNotification(d);
        let ed = last_error();
        let rs = UnregisterPowerSettingNotification(s);
        let es = last_error();
        say!(t, "a device-notification handle: {rd} (error {}), a suspend/resume handle: {rs} (error {})", if rd == 0 { ed } else { 0 }, if rs == 0 { es } else { 0 });
        UnregisterDeviceNotification(d);
        UnregisterSuspendResumeNotification(s);
        // from another thread
        let h = RegisterPowerSettingNotification(w, &GUID_MONITOR_POWER_ON, 0);
        let th = CreateThread(core::ptr::null_mut(), 0, releaser, h, 0, core::ptr::null_mut());
        WaitForSingleObject(th, 10_000);
        let mut code = 9u32;
        GetExitCodeThread(th, &mut code);
        CloseHandle(th);
        say!(t, "released from another thread: returns {code}");
        say!(t, "and then from this one: {}", UnregisterPowerSettingNotification(h));
        // inside the handler of a power message (one sent by hand here)
        let first = RegisterPowerSettingNotification(w, &GUID_CONSOLE_DISPLAY_STATE, 0);
        settle(100);
        INNER.store(first as usize, core::sync::atomic::Ordering::SeqCst);
        let ps = POWERBROADCAST_SETTING { power_setting: GUID_CONSOLE_DISPLAY_STATE, data_length: 4, data: [1, 0, 0, 0] };
        SendMessageW(w, WM_POWERBROADCAST, PBT_POWERSETTINGCHANGE, &ps as *const POWERBROADCAST_SETTING as LPARAM);
        say!(t, "released inside the handler of a power message: {}", INNER_R.load(core::sync::atomic::Ordering::SeqCst));
        INNER.store(0, core::sync::atomic::Ordering::SeqCst);
        say!(t, "the handle is gone: {}", UnregisterPowerSettingNotification(first) == 0);
        DestroyWindow(w);
    }
}
