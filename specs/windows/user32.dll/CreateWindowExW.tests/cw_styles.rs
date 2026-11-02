//! CreateWindowExW and styles: the style and extended style the procedure
//! sees in WM_NCCREATE/WM_CREATE and what GWL_STYLE / GWL_EXSTYLE hold
//! afterwards, for overlapped, captioned, popup, child, dialog-frame,
//! tool-window and client-edge windows (the bits the system adds or
//! drops, e.g. WS_CLIPSIBLINGS and WS_CAPTION for an overlapped window,
//! WS_EX_WINDOWEDGE where there is a caption or thick frame).
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
struct CREATESTRUCTW {
    create_params: *mut c_void,
    instance: HINSTANCE,
    menu: HMENU,
    parent: HWND,
    cy: i32,
    cx: i32,
    y: i32,
    x: i32,
    style: i32,
    name: *const u16,
    class: *const u16,
    ex_style: u32,
}

const WM_CREATE: u32 = 0x0001;
const WM_NCCREATE: u32 = 0x0081;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_DISABLED: u32 = 0x0800_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const WS_CLIPCHILDREN: u32 = 0x0200_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_DLGFRAME: u32 = 0x0040_0000;
const WS_VSCROLL: u32 = 0x0020_0000;
const WS_HSCROLL: u32 = 0x0010_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const WS_EX_DLGMODALFRAME: u32 = 0x0000_0001;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_WINDOWEDGE: u32 = 0x0000_0100;
const WS_EX_CLIENTEDGE: u32 = 0x0000_0200;
const WS_EX_STATICEDGE: u32 = 0x0002_0000;
const WS_EX_APPWINDOW: u32 = 0x0004_0000;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
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

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

/// GetLastError.
fn last_error() -> u32 {
    // SAFETY: reads this thread's last-error value.
    unsafe { GetLastError() }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
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
    fn GetWindowLongW(hwnd: HWND, index: i32) -> i32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

static CS: Global<[u32; 4]> = Global::new([0; 4]);

unsafe extern "system" fn proc_s(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE || msg == WM_CREATE {
        // SAFETY: lParam is a CREATESTRUCTW.
        let cs = unsafe { &*(l as *const CREATESTRUCTW) };
        let i = if msg == WM_NCCREATE { 0 } else { 2 };
        CS.get()[i] = cs.style as u32;
        CS.get()[i + 1] = cs.ex_style;
    }
    // SAFETY: the default handling.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

fn case(t: &mut Test, label: &str, ex: u32, style: u32, parent: HWND) {
    *CS.get() = [0; 4];
    let h = create(ex, wz!("CwStyles"), wz!(""), style, 0, 0, 100, 80, parent, if parent.is_null() { 0 } else { 1 });
    if h.is_null() {
        say!(t, "{label}: failed error={}", last_error());
        return;
    }
    let c = *CS.get();
    // SAFETY: plain queries of our window.
    let (s, e) = unsafe { (GetWindowLongW(h, GWL_STYLE) as u32, GetWindowLongW(h, GWL_EXSTYLE) as u32) };
    say!(
        t,
        "{label}: asked {:#x}/{:#x} nccreate {:#x}/{:#x} create {:#x}/{:#x} after {:#x}/{:#x}",
        style,
        ex,
        c[0],
        c[1],
        c[2],
        c[3],
        s,
        e
    );
    // SAFETY: our window.
    unsafe { DestroyWindow(h) };
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    t.check("registered", register(wz!("CwStyles"), Some(proc_s), 0, 0, core::ptr::null_mut()) != 0, 0);
    let p = create(0, wz!("CwStyles"), wz!(""), WS_POPUP, 0, 0, 300, 300, core::ptr::null_mut(), 0);
    let none = core::ptr::null_mut();
    case(t, "overlapped", 0, WS_OVERLAPPED, none);
    case(t, "overlappedwindow", 0, WS_OVERLAPPEDWINDOW, none);
    case(t, "overlapped_visible_disabled", 0, WS_OVERLAPPED | WS_DISABLED, none);
    case(t, "caption", 0, WS_CAPTION, none);
    case(t, "popup", 0, WS_POPUP, none);
    case(t, "popup_caption", 0, WS_POPUP | WS_CAPTION, none);
    case(t, "popup_border", 0, WS_POPUP | WS_BORDER, none);
    case(t, "popup_thickframe", 0, WS_POPUP | WS_THICKFRAME, none);
    case(t, "popup_dlgframe", 0, WS_POPUP | WS_DLGFRAME, none);
    case(t, "popup_dlgmodalframe", WS_EX_DLGMODALFRAME, WS_POPUP, none);
    case(t, "popup_toolwindow", WS_EX_TOOLWINDOW, WS_POPUP | WS_CAPTION, none);
    case(t, "popup_clientedge", WS_EX_CLIENTEDGE, WS_POPUP, none);
    case(t, "popup_staticedge", WS_EX_STATICEDGE, WS_POPUP, none);
    case(t, "popup_windowedge_only", WS_EX_WINDOWEDGE, WS_POPUP, none);
    case(t, "popup_topmost", WS_EX_TOPMOST, WS_POPUP, none);
    case(t, "popup_scrollbars", 0, WS_POPUP | WS_VSCROLL | WS_HSCROLL, none);
    case(t, "child", 0, WS_CHILD, p);
    case(t, "child_caption", 0, WS_CHILD | WS_CAPTION, p);
    case(t, "child_border_clip", 0, WS_CHILD | WS_BORDER | WS_CLIPSIBLINGS | WS_CLIPCHILDREN, p);
    case(t, "child_topmost", WS_EX_TOPMOST, WS_CHILD, p);
    case(t, "child_appwindow", WS_EX_APPWINDOW, WS_CHILD, p);
    case(t, "owned_overlapped", 0, WS_OVERLAPPED, p);
    // SAFETY: our window.
    unsafe { DestroyWindow(p) };
}
