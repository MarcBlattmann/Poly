//! RegisterSuspendResumeNotification and UnregisterSuspendResumeNotification
//! together: the handles are shared with the other notification kinds only in
//! type, not in meaning: a suspend/resume handle given to
//! UnregisterDeviceNotification or UnregisterPowerSettingNotification, a
//! power handle given to UnregisterSuspendResumeNotification; NULL;
//! calls from another thread; and the callback flag is not used.
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

const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const GWLP_USERDATA: i32 = -21;
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
const WM_DEVICECHANGE: u32 = 0x0219;
const PBT_POWERSETTINGCHANGE: usize = 0x8013;
const WM_POINTERDEVICECHANGE: u32 = 0x238;
const WM_POINTERDEVICEOUTOFRANGE: u32 = 0x23A;
/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct POWERBROADCAST_SETTING {
    power_setting: GUID,
    data_length: u32,
    data: [u8; 4],
}

const GUID_MONITOR_POWER_ON: GUID = GUID { d1: 0x02731015, d2: 0x4510, d3: 0x4526, d4: [0x99, 0xE6, 0xE5, 0xA1, 0x7E, 0xBD, 0x1A, 0xEA] };
static NOTE_POWER: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static NOTE_DEVICE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static NOTE_POINTER: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static NOTE_GUID: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static NOTE_DATA: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static NOTE_LEN: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static NOTE_SENDER_THREAD: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
unsafe extern "system" fn proc_note(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    use core::sync::atomic::Ordering::SeqCst;
    if msg == WM_POWERBROADCAST {
        NOTE_POWER.fetch_add(1, SeqCst);
        if w == PBT_POWERSETTINGCHANGE {
            // SAFETY: for this message lparam points to a POWERBROADCAST_SETTING.
            let ps = unsafe { &*(l as *const POWERBROADCAST_SETTING) };
            NOTE_GUID.store(ps.power_setting.d1, SeqCst);
            NOTE_LEN.store(ps.data_length, SeqCst);
            NOTE_DATA.store(u32::from_le_bytes(ps.data), SeqCst);
        }
        // SAFETY: plain query.
        NOTE_SENDER_THREAD.store(unsafe { GetCurrentThreadId() }, SeqCst);
        return 1;
    }
    if msg == WM_DEVICECHANGE {
        NOTE_DEVICE.fetch_add(1, SeqCst);
    }
    if (WM_POINTERDEVICECHANGE..=WM_POINTERDEVICEOUTOFRANGE).contains(&msg) {
        NOTE_POINTER.fetch_add(1, SeqCst);
    }
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentThreadId() -> u32;
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
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn UnregisterDeviceNotification(handle: HDEVNOTIFY) -> BOOL;
    fn RegisterPowerSettingNotification(recipient: HANDLE, guid: *const GUID, flags: u32) -> HDEVNOTIFY;
    fn UnregisterPowerSettingNotification(handle: HDEVNOTIFY) -> BOOL;
    fn RegisterSuspendResumeNotification(recipient: HANDLE, flags: u32) -> HDEVNOTIFY;
    fn UnregisterSuspendResumeNotification(handle: HDEVNOTIFY) -> u32;
}

poly_wtest::main!(test);

unsafe extern "system" fn other(param: *mut c_void) -> u32 {
    // SAFETY: the handle is owned by the test.
    unsafe {
        set_error(0xDEAD);
        let r = UnregisterSuspendResumeNotification(param as HDEVNOTIFY);
        (r != 0) as u32 | ((last_error() == 0xDEAD) as u32) << 1
    }
}

fn test(t: &mut Test) {
    register(wz!("RseClass"), Some(proc_note), 0, 0, core::ptr::null_mut());
    let none: HWND = core::ptr::null_mut();
    // SAFETY: our own window.
    unsafe {
        let w = create(0, wz!("RseClass"), wz!("rse"), WS_OVERLAPPEDWINDOW, 0, 0, 100, 100, none, 0);
        let s = RegisterSuspendResumeNotification(w, 0);
        set_error(0xDEAD);
        let r = UnregisterDeviceNotification(s);
        let e = last_error();
        say!(t, "a suspend/resume handle given to UnregisterDeviceNotification: returns {r}, error {}", if r == 0 { e } else { 0 });
        let still = UnregisterSuspendResumeNotification(s);
        say!(t, "then released with its own function: {still}");
        let s = RegisterSuspendResumeNotification(w, 0);
        set_error(0xDEAD);
        let r = UnregisterPowerSettingNotification(s);
        let e = last_error();
        say!(t, "a suspend/resume handle given to UnregisterPowerSettingNotification: returns {r}, error {}", if r == 0 { e } else { 0 });
        say!(t, "then released with its own function: {}", UnregisterSuspendResumeNotification(s));
        let p = RegisterPowerSettingNotification(w, &GUID_MONITOR_POWER_ON, 0);
        set_error(0xDEAD);
        let r = UnregisterSuspendResumeNotification(p);
        let e = last_error();
        say!(t, "a power-setting handle given to UnregisterSuspendResumeNotification: returns {r}, error {}", if r == 0 { e } else { 0 });
        UnregisterPowerSettingNotification(p);
        set_error(0xDEAD);
        let r = UnregisterSuspendResumeNotification(none);
        say!(t, "NULL: returns {r}, last error {}", last_error());
        // another thread
        let s = RegisterSuspendResumeNotification(w, 0);
        let th = CreateThread(core::ptr::null_mut(), 0, other, s, 0, core::ptr::null_mut());
        WaitForSingleObject(th, 10_000);
        let mut code = 0u32;
        GetExitCodeThread(th, &mut code);
        CloseHandle(th);
        say!(t, "released from another thread: returns {}, last error left alone {}", code & 1, (code >> 1) & 1);
        say!(t, "and again from this thread: {}", UnregisterSuspendResumeNotification(s));
        DestroyWindow(w);
    }
}
