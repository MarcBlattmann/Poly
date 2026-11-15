//! Window classes of the common controls (comctl32 version 5.82, the one a
//! program without a manifest gets): which classes exist before and after
//! InitCommonControlsEx(ICC_*), whether CreateWindowExW makes a control of
//! each, its class name, default styles, the window text and size; the classes
//! of user32 for comparison.
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

const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const GWL_STYLE: i32 = -16;
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

fn rect_of(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    // SAFETY: r is writable.
    unsafe { GetWindowRect(hwnd, &mut r) };
    r
}

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

/// The UTF-16 text in `buf` up to its first NUL, as ASCII (others as '?').
fn ascii_of(buf: &[u16], out: &mut Buf<300>) {
    for &c in buf {
        if c == 0 {
            break;
        }
        let _ = out.write_char(if (0x20..0x7f).contains(&c) { c as u8 as char } else { '?' });
    }
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

/// commctrl.h
#[repr(C)]
#[derive(Clone, Copy)]
struct INITCOMMONCONTROLSEX {
    dw_size: u32,
    dw_icc: u32,
}

const ICC_WIN95_CLASSES: u32 = 0xFF;
const ICC_DATE_CLASSES: u32 = 0x100;
const ICC_USEREX_CLASSES: u32 = 0x200;
const ICC_COOL_CLASSES: u32 = 0x400;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassExW(class: *const WNDCLASSEXW) -> u16;
    fn GetClassNameW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
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
    fn GetWindowLongW(hwnd: HWND, index: i32) -> i32;
    fn GetWindowRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn GetWindowTextW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetSystemMetrics(index: i32) -> i32;
    fn IsWindowUnicode(hwnd: HWND) -> BOOL;
}

#[link(name = "comctl32", kind = "raw-dylib")]
unsafe extern "system" {
    fn InitCommonControls();
    fn InitCommonControlsEx(icc: *const INITCOMMONCONTROLSEX) -> BOOL;
}

/// Call `$e` with the last error set to a marker; print "label: result err=N"
/// (err -1: the call did not touch the last error). Gives the result.
macro_rules! call {
    ($t:expr, $label:expr, $e:expr) => {{
        set_error(0xDEAD);
        let r = ($e) as i64;
        let e = last_error();
        if r == 0 || r == -1 || stringify!($e).contains("is_null") {
            say!($t, "{}: {} err={}", $label, r, if e == 0xDEAD { -1 } else { e as i64 });
        } else {
            say!($t, "{}: {}", $label, r);
        }
        r
    }};
}

/// MAKEINTRESOURCE
fn mir(id: usize) -> *const u16 {
    id as *const u16
}

/// A menu handle made up by the test (never a real one).
const BOGUS_MENU: usize = 0x0bad_0bad;

fn make(t: &mut Test, label: &str, class: &[u16], style: u32, parent: HWND) -> HWND {
    set_error(0xDEAD);
    let h = create(0, class, wz!("text"), style | WS_CHILD, 0, 0, 100, 40, parent, 1);
    let e = last_error();
    if h.is_null() {
        say!(t, "{label}: not created, err={}", if e == 0xDEAD { -1 } else { e as i64 });
    } else {
        let r = rect_of(h);
        let mut b = Buf::<300>::new();
        let mut v = [0u16; 60];
        // SAFETY: v has 60 units.
        unsafe { GetClassNameW(h, v.as_mut_ptr(), 60) };
        ascii_of(&v, &mut b);
        let mut tb = Buf::<300>::new();
        let mut tv = [0u16; 60];
        unsafe { GetWindowTextW(h, tv.as_mut_ptr(), 60) };
        ascii_of(&tv, &mut tb);
        let text_shown = if label == "datetime" || label == "monthcal" { "(a date: not printed)" } else { tb.as_str() };
        say!(t, "{label}: created, class '{}' width {}: height {} style {:#x} text '{}' unicode {}", b.as_str(), if r.right - r.left == 100 { "100" } else { "other" }, if r.bottom - r.top == 40 { "40" } else { "other" }, unsafe { GetWindowLongW(h, GWL_STYLE) } as u32 & !(WS_CHILD | 0), text_shown, unsafe { IsWindowUnicode(h) });
    }
    h
}

unsafe extern "system" fn wproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: default processing.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

macro_rules! cls {
    ($t:expr, $label:expr, $name:literal, $parent:expr) => {{
        let h = make($t, $label, wz!($name), 0, $parent);
        // SAFETY: our window or NULL.
        unsafe { DestroyWindow(h) };
    }};
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    register(wz!("CcParent"), Some(wproc), 0, 0, core::ptr::null_mut());
    let parent = create(0, wz!("CcParent"), wz!("p"), WS_OVERLAPPEDWINDOW, 0, 0, 400, 300, core::ptr::null_mut(), 0);
    // classes of user32 first
    cls!(t, "button", "Button", parent);
    cls!(t, "edit", "Edit", parent);
    cls!(t, "static", "Static", parent);
    cls!(t, "listbox", "ListBox", parent);
    cls!(t, "combobox", "ComboBox", parent);
    cls!(t, "scrollbar", "ScrollBar", parent);
    // after InitCommonControls (ICC_WIN95_CLASSES)
    unsafe { InitCommonControls() };
    cls!(t, "listview", "SysListView32", parent);
    cls!(t, "treeview", "SysTreeView32", parent);
    cls!(t, "toolbar", "ToolbarWindow32", parent);
    cls!(t, "statusbar", "msctls_statusbar32", parent);
    cls!(t, "tab", "SysTabControl32", parent);
    cls!(t, "progress", "msctls_progress32", parent);
    cls!(t, "trackbar", "msctls_trackbar32", parent);
    cls!(t, "updown", "msctls_updown32", parent);
    cls!(t, "rebar", "ReBarWindow32", parent);
    cls!(t, "header", "SysHeader32", parent);
    cls!(t, "tooltips", "tooltips_class32", parent);
    cls!(t, "animate", "SysAnimate32", parent);
    cls!(t, "hotkey", "msctls_hotkey32", parent);
    cls!(t, "datetime", "SysDateTimePick32", parent);
    cls!(t, "monthcal", "SysMonthCal32", parent);
    cls!(t, "comboboxex", "ComboBoxEx32", parent);
    cls!(t, "ipaddress", "SysIPAddress32", parent);
    cls!(t, "pager", "SysPager", parent);
    cls!(t, "link", "SysLink", parent);
    cls!(t, "nativefont", "NativeFontCtl", parent);
    // the classes with the later bits
    for (label, bits) in [("ICC_DATE_CLASSES", ICC_DATE_CLASSES), ("ICC_USEREX_CLASSES", ICC_USEREX_CLASSES), ("ICC_COOL_CLASSES", ICC_COOL_CLASSES), ("ICC_INTERNET_CLASSES", 0x800), ("ICC_PAGESCROLLER_CLASS", 0x1000)] {
        let icc = INITCOMMONCONTROLSEX { dw_size: 8, dw_icc: bits };
        call!(t, label, unsafe { InitCommonControlsEx(&icc) });
    }
    // again, after the later bits
    cls!(t, "rebar", "ReBarWindow32", parent);
    cls!(t, "header", "SysHeader32", parent);
    cls!(t, "tooltips", "tooltips_class32", parent);
    cls!(t, "animate", "SysAnimate32", parent);
    cls!(t, "hotkey", "msctls_hotkey32", parent);
    cls!(t, "datetime", "SysDateTimePick32", parent);
    cls!(t, "monthcal", "SysMonthCal32", parent);
    cls!(t, "comboboxex", "ComboBoxEx32", parent);
    cls!(t, "ipaddress", "SysIPAddress32", parent);
    cls!(t, "pager", "SysPager", parent);
    cls!(t, "link", "SysLink", parent);
    cls!(t, "nativefont", "NativeFontCtl", parent);
    unsafe { DestroyWindow(parent) };
}
