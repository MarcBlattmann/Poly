//! UnregisterDeviceNotification and the lifetime of the registration: after
//! the window was destroyed, from another thread than the one that
//! registered, while a device message is being handled (WM_DEVICECHANGE sent
//! by hand to the window procedure that unregisters inside it), and at the
//! end of the thread that registered (the registration does not end with the
//! thread: the handle can still be unregistered).
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
const WM_DEVICECHANGE: u32 = 0x0219;
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

const GUID_DEVINTERFACE_USB_DEVICE: GUID = GUID { d1: 0xA5DCBF10, d2: 0x6530, d3: 0x11D2, d4: [0x90, 0x1F, 0x00, 0xC0, 0x4F, 0xB9, 0x51, 0xED] };
const GUID_DEVINTERFACE_HID: GUID = GUID { d1: 0x4D1E55B2, d2: 0xF16F, d3: 0x11CF, d4: [0x88, 0xCB, 0x00, 0x11, 0x11, 0x00, 0x00, 0x30] };

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
    fn SendMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn RegisterDeviceNotificationW(recipient: HANDLE, filter: *const c_void, flags: u32) -> HDEVNOTIFY;
    fn UnregisterDeviceNotification(handle: HDEVNOTIFY) -> BOOL;
}

poly_wtest::main!(test);

static INNER: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
static INNER_R: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(9);

unsafe extern "system" fn proc_unreg(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_DEVICECHANGE {
        let h = INNER.load(core::sync::atomic::Ordering::SeqCst);
        // SAFETY: the handle was registered by the test.
        let r = unsafe { UnregisterDeviceNotification(h as HDEVNOTIFY) };
        INNER_R.store(r as u32, core::sync::atomic::Ordering::SeqCst);
        return 1;
    }
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

struct Arg {
    hwnd: usize,
    handle: usize,
    result: u32,
}

unsafe extern "system" fn registers(param: *mut c_void) -> u32 {
    // SAFETY: the arg outlives the thread.
    unsafe {
        let a = &mut *(param as *mut Arg);
        let mut f: DEV_BROADCAST_DEVICEINTERFACE_W = zeroed();
        f.size = core::mem::size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32;
        f.device_type = DBT_DEVTYP_DEVICEINTERFACE;
        f.class_guid = GUID_DEVINTERFACE_HID;
        a.handle = RegisterDeviceNotificationW(a.hwnd as HWND, &f as *const _ as *const c_void, 0) as usize;
    }
    0
}

unsafe extern "system" fn unregisters(param: *mut c_void) -> u32 {
    // SAFETY: the arg outlives the thread.
    unsafe {
        let a = &mut *(param as *mut Arg);
        a.result = UnregisterDeviceNotification(a.handle as HDEVNOTIFY) as u32;
    }
    0
}

fn test(t: &mut Test) {
    register(wz!("UdlClass"), Some(proc_unreg), 0, 0, core::ptr::null_mut());
    let none: HWND = core::ptr::null_mut();
    // SAFETY: our own windows and registrations.
    unsafe {
        let mut f: DEV_BROADCAST_DEVICEINTERFACE_W = zeroed();
        f.size = core::mem::size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32;
        f.device_type = DBT_DEVTYP_DEVICEINTERFACE;
        f.class_guid = GUID_DEVINTERFACE_USB_DEVICE;
        // window destroyed first
        let w = create(0, wz!("UdlClass"), wz!("udl"), WS_OVERLAPPEDWINDOW, 0, 0, 100, 100, none, 0);
        let h = RegisterDeviceNotificationW(w, &f as *const _ as *const c_void, 0);
        DestroyWindow(w);
        set_error(0xDEAD);
        let r = UnregisterDeviceNotification(h);
        say!(t, "after the window was destroyed: returns {r}, last error set to 0 {}", last_error() == 0);
        set_error(0xDEAD);
        let r = UnregisterDeviceNotification(h);
        say!(t, "and again: returns {r}, last error {}", last_error());
        // inside the window procedure
        let w = create(0, wz!("UdlClass"), wz!("udl"), WS_OVERLAPPEDWINDOW, 0, 0, 100, 100, none, 0);
        let h = RegisterDeviceNotificationW(w, &f as *const _ as *const c_void, 0);
        INNER.store(h as usize, core::sync::atomic::Ordering::SeqCst);
        SendMessageW(w, WM_DEVICECHANGE, 0x0007, 0);
        say!(t, "unregistering inside the window procedure of a WM_DEVICECHANGE: returns {}", INNER_R.load(core::sync::atomic::Ordering::SeqCst));
        say!(t, "the handle is then gone: {}", UnregisterDeviceNotification(h) == 0);
        // registered by another thread, unregistered by this one, and by a third
        let mut arg = Arg { hwnd: w as usize, handle: 0, result: 9 };
        let th = CreateThread(core::ptr::null_mut(), 0, registers, &mut arg as *mut Arg as *mut c_void, 0, core::ptr::null_mut());
        WaitForSingleObject(th, 10_000);
        CloseHandle(th);
        say!(t, "the registering thread has ended; the handle it got is not NULL: {}", arg.handle != 0);
        set_error(0xDEAD);
        let th = CreateThread(core::ptr::null_mut(), 0, unregisters, &mut arg as *mut Arg as *mut c_void, 0, core::ptr::null_mut());
        WaitForSingleObject(th, 10_000);
        CloseHandle(th);
        say!(t, "unregistered from a third thread after the first one ended: returns {}", arg.result);
        say!(t, "and a second time from this one: {}", UnregisterDeviceNotification(arg.handle as HDEVNOTIFY));
        DestroyWindow(w);
    }
}
