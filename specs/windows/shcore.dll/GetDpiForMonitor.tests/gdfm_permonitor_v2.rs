//! GetDpiForMonitor in a process set to PER_MONITOR_AWARE_V2: on the single 96 DPI monitor
//! every type behaves as in an unaware process (effective and angular 96,
//! raw not supported); the same monitor handle is returned for the window's
//! monitor and the primary monitor; GetDpiForWindow and GetDpiForSystem agree.
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

const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const SW_SHOWNOACTIVATE: i32 = 4;
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

type HMONITOR = *mut c_void;
const MONITOR_DEFAULTTONULL: u32 = 0;
const MONITOR_DEFAULTTOPRIMARY: u32 = 1;
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

const DPI_PER_MONITOR_AWARE_V2: isize = -4;
const MDT_EFFECTIVE_DPI: i32 = 0;
const MDT_ANGULAR_DPI: i32 = 1;
const MDT_RAW_DPI: i32 = 2;
/// The primary monitor.
fn dp_primary() -> HMONITOR {
    // SAFETY: plain query.
    unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
}

/// GetDpiForMonitor of the primary monitor for every type.
fn dp_monitor_report(t: &mut Test) {
    let m = dp_primary();
    for k in [MDT_EFFECTIVE_DPI, MDT_ANGULAR_DPI, MDT_RAW_DPI, 3, -1] {
        let (mut x, mut y) = (0xAAAAu32, 0xBBBBu32);
        // SAFETY: valid out pointers.
        let hr = unsafe { GetDpiForMonitor(m, k, &mut x, &mut y) };
        say!(t, "GetDpiForMonitor(type {}) {:#x} x {:#x} y {:#x}", k, hr, x, y);
    }
}

/// A window procedure that does nothing but the default handling (nothing
/// is logged).
unsafe extern "system" fn dp_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// Register `name` with `dp_proc` and create a hidden popup window.
fn dp_popup(name: &[u16]) -> HWND {
    register(name, Some(dp_proc), 0, 0, core::ptr::null_mut());
    create(WS_EX_NOACTIVATE, name, wz!(""), WS_POPUP, 10, 10, 100, 50, core::ptr::null_mut(), 0)
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetCurrentProcessId() -> u32;
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
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn MonitorFromWindow(hwnd: HWND, flags: u32) -> HMONITOR;
    fn MonitorFromPoint(pt: POINT, flags: u32) -> HMONITOR;
    fn GetDpiForWindow(hwnd: HWND) -> u32;
    fn GetDpiForSystem() -> u32;
    fn SetProcessDpiAwarenessContext(ctx: isize) -> BOOL;
}

#[link(name = "shcore", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDpiForMonitor(mon: HMONITOR, kind: i32, x: *mut u32, y: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls; the window is ours.
    unsafe {
        let r = SetProcessDpiAwarenessContext(DPI_PER_MONITOR_AWARE_V2);
        say!(t, "SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2) -> {}", r);
        dp_monitor_report(t);
        let w = dp_popup(wz!("Gdfmpermonitor_v2"));
        ShowWindow(w, SW_SHOWNOACTIVATE);
        let mw = MonitorFromWindow(w, MONITOR_DEFAULTTONULL);
        let (mut x, mut y) = (0u32, 0u32);
        let hr = GetDpiForMonitor(mw, MDT_EFFECTIVE_DPI, &mut x, &mut y);
        say!(
            t,
            "window's monitor is the primary {} hr {:#x} {}x{} ; GetDpiForWindow {} GetDpiForSystem {}",
            mw == dp_primary(),
            hr,
            x,
            y,
            GetDpiForWindow(w),
            GetDpiForSystem()
        );
        DestroyWindow(w);
    }
}
