//! RegisterPointerDeviceNotifications(hwnd, notifyRange): the window is
//! registered for WM_POINTERDEVICECHANGE (and, with notifyRange TRUE, for
//! WM_POINTERDEVICEINRANGE and WM_POINTERDEVICEOUTOFRANGE). Chrome's browser
//! calls it once for its top-level window (S3). First registration of a
//! window: TRUE and the last error is not touched; a window that is registered
//! already: FALSE, also with the other value of notifyRange, and the last
//! error is still not touched; different windows are separate; child and
//! message-only windows are accepted; NULL and invalid handles fail with
//! ERROR_INVALID_WINDOW_HANDLE (1400); destroyed windows fail; the
//! registration of a window handle that was destroyed does not carry over to
//! a new window. On a machine without pointer devices no message arrives.
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
const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const HWND_MESSAGE: HWND = -3isize as HWND;
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

fn note_reset() {
    use core::sync::atomic::Ordering::SeqCst;
    for a in [&NOTE_POWER, &NOTE_DEVICE, &NOTE_POINTER, &NOTE_GUID, &NOTE_DATA, &NOTE_LEN, &NOTE_SENDER_THREAD] {
        a.store(0, SeqCst);
    }
}

fn note_get(a: &core::sync::atomic::AtomicU32) -> u32 {
    a.load(core::sync::atomic::Ordering::SeqCst)
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
    fn GetTickCount() -> u32;
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
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn RegisterPointerDeviceNotifications(hwnd: HWND, notify_range: BOOL) -> BOOL;
}

poly_wtest::main!(test);

struct Arg {
    hwnd: usize,
    result: u32,
    untouched: bool,
}

unsafe extern "system" fn other(param: *mut c_void) -> u32 {
    // SAFETY: the window belongs to the main thread.
    unsafe {
        let a = &mut *(param as *mut Arg);
        set_error(0xDEAD);
        a.result = RegisterPointerDeviceNotifications(a.hwnd as HWND, 0) as u32;
        a.untouched = last_error() == 0xDEAD;
    }
    0
}

fn test(t: &mut Test) {
    register(wz!("RpdClass"), Some(proc_note), 0, 0, core::ptr::null_mut());
    let none: HWND = core::ptr::null_mut();
    // SAFETY: our own windows.
    unsafe {
        let w = create(0, wz!("RpdClass"), wz!("rpd"), WS_OVERLAPPEDWINDOW, 0, 0, 100, 100, none, 0);
        let w2 = create(0, wz!("RpdClass"), wz!("rpd2"), WS_OVERLAPPEDWINDOW, 0, 0, 100, 100, none, 0);
        let w3 = create(0, wz!("RpdClass"), wz!("rpd3"), WS_OVERLAPPEDWINDOW, 0, 0, 100, 100, none, 0);
        note_reset();
        for (what, h, flag) in [("w, notifyRange FALSE", w, 0), ("w again, FALSE", w, 0), ("w again, TRUE", w, 1), ("w2, TRUE", w2, 1), ("w2 again, FALSE", w2, 0), ("w2 again, TRUE", w2, 1)] {
            set_error(0xDEAD);
            let r = RegisterPointerDeviceNotifications(h, flag);
            say!(t, "{what}: returns {r}, last error left alone {}", last_error() == 0xDEAD);
        }
        // another thread, for a window that is registered
        let mut arg = Arg { hwnd: w as usize, result: 9, untouched: false };
        let th = CreateThread(core::ptr::null_mut(), 0, other, &mut arg as *mut Arg as *mut c_void, 0, core::ptr::null_mut());
        WaitForSingleObject(th, 10_000);
        CloseHandle(th);
        say!(t, "from another thread, window already registered: returns {}, last error left alone {}", arg.result, arg.untouched);
        // and for a fresh window
        let mut arg = Arg { hwnd: w3 as usize, result: 9, untouched: false };
        let th = CreateThread(core::ptr::null_mut(), 0, other, &mut arg as *mut Arg as *mut c_void, 0, core::ptr::null_mut());
        WaitForSingleObject(th, 10_000);
        CloseHandle(th);
        say!(t, "from another thread, fresh window of this thread: returns {}", arg.result);
        // destroyed windows
        DestroyWindow(w);
        set_error(0xDEAD);
        let r = RegisterPointerDeviceNotifications(w, 0);
        say!(t, "a destroyed window: returns {r}, last error {}", last_error());
        let n = create(0, wz!("RpdClass"), wz!("rpdn"), WS_OVERLAPPEDWINDOW, 0, 0, 100, 100, none, 0);
        let r = RegisterPointerDeviceNotifications(n, 1);
        say!(t, "a new window (possibly with the handle of the destroyed one): returns {r}");
        // kinds of windows
        let k = create(0, wz!("RpdClass"), wz!("kid"), WS_CHILD, 0, 0, 10, 10, w2, 1);
        let m = create(0, wz!("RpdClass"), wz!("msg"), 0, 0, 0, 0, 0, HWND_MESSAGE, 0);
        let p = create(0, wz!("RpdClass"), wz!("pop"), WS_POPUP, 0, 0, 10, 10, none, 0);
        say!(t, "a child window: {}, a message-only window: {}, a popup: {}", RegisterPointerDeviceNotifications(k, 0), RegisterPointerDeviceNotifications(m, 0), RegisterPointerDeviceNotifications(p, 0));
        for (what, h) in [("NULL", none), ("an invalid handle", 0x1234usize as HWND)] {
            set_error(0xDEAD);
            let r = RegisterPointerDeviceNotifications(h, 0);
            say!(t, "{what}: returns {r}, last error {}", last_error());
        }
        settle(200);
        pump();
        say!(t, "pointer-device messages that arrived: {} (none without pointer devices)", note_get(&NOTE_POINTER));
        for h in [p, m, k, n, w3, w2] {
            DestroyWindow(h);
        }
    }
}
