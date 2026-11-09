//! FindWindowW with several matches: it returns the window highest in the
//! z order (the most recently created of our hidden popups), and the next
//! after a z-order change; FindWindowW does not send WM_GETTEXT to our
//! windows (the title is the stored text, so a procedure that answers
//! WM_GETTEXT differently does not change what matches).
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

const WM_GETTEXT: u32 = 0x000D;
const WS_POPUP: u32 = 0x8000_0000;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOACTIVATE: u32 = 0x0010;
const HWND_TOP: HWND = core::ptr::null_mut();
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
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
    fn SetWindowPos(hwnd: HWND, after: HWND, x: i32, y: i32, w: i32, h: i32, flags: u32) -> BOOL;
    fn FindWindowW(class: *const u16, name: *const u16) -> HWND;
    fn GetWindowTextW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

static GETTEXT: Global<u32> = Global::new(0);

unsafe extern "system" fn proc_t(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_GETTEXT {
        *GETTEXT.get() += 1;
        if w >= 6 && l != 0 {
            let s = wz!("Other");
            // SAFETY: lParam is a buffer of wParam characters.
            unsafe { core::ptr::copy_nonoverlapping(s.as_ptr(), l as *mut u16, 6) };
            return 5;
        }
    }
    // SAFETY: the default handling.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    t.check("registered", register(wz!("FwOrder"), Some(proc_t), 0, 0, core::ptr::null_mut()) != 0, 0);
    let none = core::ptr::null_mut();
    let a = create(0, wz!("FwOrder"), wz!("Same"), WS_POPUP, 0, 0, 10, 10, none, 0);
    let b = create(0, wz!("FwOrder"), wz!("Same"), WS_POPUP, 0, 0, 10, 10, none, 0);
    let c = create(0, wz!("FwOrder"), wz!("Same"), WS_POPUP, 0, 0, 10, 10, none, 0);
    let name = |h: HWND| {
        if h == a {
            "A"
        } else if h == b {
            "B"
        } else if h == c {
            "C"
        } else if h.is_null() {
            "null"
        } else {
            "other"
        }
    };
    *GETTEXT.get() = 0;
    // SAFETY: NUL-terminated strings; our windows.
    unsafe {
        let h = FindWindowW(wz!("FwOrder").as_ptr(), core::ptr::null());
        say!(t, "first match by class: {}", name(h));
        SetWindowPos(a, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        let h = FindWindowW(wz!("FwOrder").as_ptr(), core::ptr::null());
        say!(t, "after A to top: {}", name(h));
        let h = FindWindowW(core::ptr::null(), wz!("Same").as_ptr());
        say!(t, "by stored title: {}", name(h));
        let h = FindWindowW(core::ptr::null(), wz!("Other").as_ptr());
        say!(t, "by the title WM_GETTEXT would give: {}", name(h));
        say!(t, "WM_GETTEXT messages during FindWindowW: {}", *GETTEXT.get());
        let mut buf = [0u16; 16];
        let n = GetWindowTextW(a, buf.as_mut_ptr(), 16);
        say!(t, "GetWindowTextW does ask: n={} gettext={}", n, *GETTEXT.get());
        DestroyWindow(a);
        DestroyWindow(b);
        DestroyWindow(c);
    }
}
