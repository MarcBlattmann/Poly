//! What WM_GETMINMAXINFO carries and where a maximized window lands, by
//! window style: for top-level windows without WS_POPUP (which always get a
//! caption) and for popups, with no frame, a border, a dialog frame, a
//! caption, a sizing frame, both, the overlapped-window set and a system
//! menu only, each plain and as a tool window, with WS_EX_DLGMODALFRAME,
//! WS_EX_STATICEDGE and WS_EX_CLIENTEDGE. Per window: the style and
//! extended style the window really has, how many WM_GETMINMAXINFO it got at
//! creation, at SW_SHOWMAXIMIZED and at the SW_SHOWMINIMIZED after it, the
//! values of the last one, and the window and client rectangles.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::cell::UnsafeCell;
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
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
}

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
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

/// winuser.h
#[repr(C)]
struct MINMAXINFO {
    reserved: POINT,
    max_size: POINT,
    max_position: POINT,
    min_track_size: POINT,
    max_track_size: POINT,
}

const WM_GETMINMAXINFO: u32 = 0x0024;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_DLGFRAME: u32 = 0x0040_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const WS_EX_DLGMODALFRAME: u32 = 0x0000_0001;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_CLIENTEDGE: u32 = 0x0000_0200;
const WS_EX_STATICEDGE: u32 = 0x0002_0000;
const SW_SHOWMINIMIZED: i32 = 2;
const SW_SHOWMAXIMIZED: i32 = 3;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
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
/// A global the window procedures and the test share (one thread at a
/// time unless a test says otherwise).
struct Global<T>(UnsafeCell<T>);

unsafe impl<T> Sync for Global<T> {}
impl<T> Global<T> {
    const fn new(v: T) -> Self {
        Global(UnsafeCell::new(v))
    }
    #[allow(clippy::mut_from_ref)]
    fn get(&self) -> &mut T {
        // SAFETY: see the Sync impl: no two live references at once.
        unsafe { &mut *self.0.get() }
    }
}

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

fn rect_of(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    // SAFETY: r is writable.
    unsafe { GetWindowRect(hwnd, &mut r) };
    r
}

fn client_of(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    // SAFETY: r is writable.
    unsafe { GetClientRect(hwnd, &mut r) };
    r
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

/// "l,t,r,b" of a rectangle.
struct R(RECT);

impl core::fmt::Display for R {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{},{},{},{}", self.0.left, self.0.top, self.0.right, self.0.bottom)
    }
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetCurrentProcessId() -> u32;
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
    fn ShowWindow(hwnd: HWND, cmd: i32) -> BOOL;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowLongW(hwnd: HWND, index: i32) -> i32;
    fn GetWindowRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
}

static MM: Global<[i32; 9]> = Global::new([0; 9]);

unsafe extern "system" fn mm_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_GETMINMAXINFO {
        // SAFETY: lParam points to a MINMAXINFO for this message.
        let m = unsafe { &*(l as *const MINMAXINFO) };
        let g = MM.get();
        g[0] += 1;
        g[1] = m.max_size.x;
        g[2] = m.max_size.y;
        g[3] = m.max_position.x;
        g[4] = m.max_position.y;
        g[5] = m.min_track_size.x;
        g[6] = m.min_track_size.y;
        g[7] = m.max_track_size.x;
        g[8] = m.max_track_size.y;
    }
    // SAFETY: the default handling.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

fn mm() -> Buf<100> {
    let g = MM.get();
    let mut b = Buf::<100>::new();
    if g[0] == 0 {
        let _ = write!(b, "none");
    } else {
        let _ = write!(b, "x{} max {}x{} at {},{} mintrack {},{} maxtrack {},{}", g[0], g[1], g[2], g[3], g[4], g[5], g[6], g[7], g[8]);
    }
    *g = [0; 9];
    b
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let cls = wz!("SwMinMaxStyles");
    t.check("registered", register(cls, Some(mm_proc), 0, 0, core::ptr::null_mut()) != 0, 0);
    let null = core::ptr::null_mut();
    let styles: [u32; 8] = [
        0,
        WS_BORDER,
        WS_DLGFRAME,
        WS_CAPTION,
        WS_THICKFRAME,
        WS_CAPTION | WS_THICKFRAME,
        WS_OVERLAPPEDWINDOW,
        WS_SYSMENU,
    ];
    let exs: [u32; 5] = [0, WS_EX_TOOLWINDOW, WS_EX_DLGMODALFRAME, WS_EX_STATICEDGE, WS_EX_CLIENTEDGE];
    for popup in [false, true] {
        for ex in exs {
            for st in styles {
                let style = st | if popup { WS_POPUP } else { 0 };
                *MM.get() = [0; 9];
                let h = create(ex, cls, &[], style, 60, 70, 300, 200, null, 0);
                let at_create = mm();
                // SAFETY: our window.
                let (s2, e2) = unsafe { (GetWindowLongW(h, GWL_STYLE) as u32, GetWindowLongW(h, GWL_EXSTYLE) as u32) };
                // SAFETY: our window.
                unsafe { ShowWindow(h, SW_SHOWMAXIMIZED) };
                let at_max = mm();
                let r = rect_of(h);
                let c = client_of(h);
                // SAFETY: our window.
                unsafe { ShowWindow(h, SW_SHOWMINIMIZED) };
                let at_min = mm();
                let rmin = rect_of(h);
                // SAFETY: our window.
                unsafe { DestroyWindow(h) };
                settle(10);
                say!(t, "style {:#010x} ex {:#x}: window style {:#010x} ex {:#x}", style, ex, s2, e2);
                say!(t, "  created: {}", at_create.as_str());
                say!(t, "  maximized: {}; window {} client {}x{}", at_max.as_str(), R(r), c.right, c.bottom);
                say!(t, "  then minimized: {}; window {}", at_min.as_str(), R(rmin));
            }
        }
    }
}
