//! GetWindowLongPtrW of the predefined indexes: GWLP_WNDPROC is the class
//! procedure, GWLP_HINSTANCE the instance given, GWLP_ID a child's id,
//! GWLP_HWNDPARENT the parent of a child and the owner of a popup (NULL
//! for an unowned one), GWLP_USERDATA starts at 0, GWL_STYLE/GWL_EXSTYLE;
//! GetWindowLongW returns the same 32-bit fields; index -2 returns 0
//! without an error, index -30 fails with ERROR_INVALID_INDEX; a bad or
//! NULL window fails with ERROR_INVALID_WINDOW_HANDLE; a successful call
//! leaves the last error as it was.
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
const WS_CHILD: u32 = 0x4000_0000;
const WS_DISABLED: u32 = 0x0800_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const GWLP_WNDPROC: i32 = -4;
const GWLP_HINSTANCE: i32 = -6;
const GWLP_HWNDPARENT: i32 = -8;
const GWLP_ID: i32 = -12;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
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
    fn GetWindowLongW(hwnd: HWND, index: i32) -> i32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

unsafe extern "system" fn proc_a(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: the default handling.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

fn get(t: &mut Test, label: &str, h: HWND, index: i32) -> isize {
    set_error(0xDEAD);
    // SAFETY: GetWindowLongPtrW validates the handle and the index.
    let v = unsafe { GetWindowLongPtrW(h, index) };
    let e = last_error();
    say!(t, "{label}: zero={} error={}", (v == 0) as i32, if e == 0xDEAD { -1 } else { e as i64 });
    v
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let cls = wz!("GwlFields");
    t.check("registered", register(cls, Some(proc_a), 0, 8, core::ptr::null_mut()) != 0, 0);
    let p = create(WS_EX_TOOLWINDOW, cls, wz!(""), WS_POPUP | WS_BORDER, 0, 0, 100, 100, core::ptr::null_mut(), 0);
    let c = create(0, cls, wz!(""), WS_CHILD | WS_DISABLED, 0, 0, 10, 10, p, 0x77);
    let o = create(0, cls, wz!(""), WS_POPUP, 0, 0, 10, 10, p, 0);
    let v = get(t, "proc", p, GWLP_WNDPROC);
    t.check("proc_is_class_proc", v as usize == proc_a as *const () as usize, 0);
    let v = get(t, "hinstance", p, GWLP_HINSTANCE);
    t.check("hinstance_is_exe", v == instance() as isize, 0);
    let v = get(t, "child_id", c, GWLP_ID);
    t.expect("child_id_value", v as i64, 0x77);
    let v = get(t, "popup_id", p, GWLP_ID);
    t.expect("popup_id_value", v as i64, 0);
    let v = get(t, "child_hwndparent", c, GWLP_HWNDPARENT);
    t.check("child_hwndparent_is_parent", v == p as isize, 0);
    let v = get(t, "owned_hwndparent", o, GWLP_HWNDPARENT);
    t.check("owned_hwndparent_is_owner", v == p as isize, 0);
    get(t, "unowned_hwndparent", p, GWLP_HWNDPARENT);
    get(t, "userdata", p, GWLP_USERDATA);
    let s = get(t, "style", p, GWL_STYLE);
    say!(t, "popup style {:#x}", s as u32);
    let s = get(t, "child_style", c, GWL_STYLE);
    say!(t, "child style {:#x}", s as u32);
    let e = get(t, "exstyle", p, GWL_EXSTYLE);
    say!(t, "popup exstyle {:#x}", e as u32);
    get(t, "index_minus_2", p, -2);
    get(t, "index_minus_30", p, -30);
    get(t, "bad_window", 0x0bad_0bad_usize as HWND, GWL_STYLE);
    get(t, "null_window", core::ptr::null_mut(), GWL_STYLE);
    // SAFETY: plain queries of our windows.
    unsafe {
        t.expect(
            "long_style_matches",
            GetWindowLongW(c, GWL_STYLE) as u32 as i64,
            GetWindowLongPtrW(c, GWL_STYLE) as u32 as i64,
        );
        t.expect("long_id", GetWindowLongW(c, GWLP_ID) as i64, 0x77);
        DestroyWindow(p);
    }
}
