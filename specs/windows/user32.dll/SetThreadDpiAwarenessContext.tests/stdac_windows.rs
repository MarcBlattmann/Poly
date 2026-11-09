//! Windows created under each thread context: the window's awareness is the
//! context the thread had at creation (GetWindowDpiAwarenessContext), it does
//! not change when the thread's context changes later; GetDpiForWindow is 96 for
//! all (one 96 DPI monitor); a child window has its parent's awareness whatever
//! the thread's context is; an owned window has the thread's.
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
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
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

const DPI_UNAWARE: isize = -1;
const DPI_SYSTEM_AWARE: isize = -2;
const DPI_PER_MONITOR_AWARE: isize = -3;
const DPI_PER_MONITOR_AWARE_V2: isize = -4;
const DPI_UNAWARE_GDISCALED: isize = -5;
/// A tiny helper: the name of an awareness context handle as the tests print
/// it (the pseudo handles by name, anything else as "other").
fn dp_ctx(c: isize) -> &'static str {
    match c {
        0 => "NULL",
        -1 => "UNAWARE",
        -2 => "SYSTEM_AWARE",
        -3 => "PER_MONITOR_AWARE",
        -4 => "PER_MONITOR_AWARE_V2",
        -5 => "UNAWARE_GDISCALED",
        _ => "other",
    }
}

/// Which pseudo handle a context equals (AreDpiAwarenessContextsEqual), or
/// "none".
fn dp_which(c: isize) -> &'static str {
    // SAFETY: plain queries on handle values.
    unsafe {
        for p in [DPI_UNAWARE, DPI_SYSTEM_AWARE, DPI_PER_MONITOR_AWARE, DPI_PER_MONITOR_AWARE_V2, DPI_UNAWARE_GDISCALED] {
            if AreDpiAwarenessContextsEqual(c, p) != 0 {
                return dp_ctx(p);
            }
        }
    }
    "none"
}

/// A window procedure that does nothing but the default handling (nothing
/// is logged).
unsafe extern "system" fn dp_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// The awareness of a window, by pseudo handle name.
fn dp_wa(h: HWND) -> &'static str {
    // SAFETY: plain query.
    dp_which(unsafe { GetWindowDpiAwarenessContext(h) })
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
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetDpiForWindow(hwnd: HWND) -> u32;
    fn SetThreadDpiAwarenessContext(ctx: isize) -> isize;
    fn GetWindowDpiAwarenessContext(hwnd: HWND) -> isize;
    fn AreDpiAwarenessContextsEqual(a: isize, b: isize) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let cls = wz!("StdacWin");
    register(cls, Some(dp_proc), 0, 0, core::ptr::null_mut());
    // SAFETY: plain calls; the windows are ours.
    unsafe {
        for c in CTXS_ {
            SetThreadDpiAwarenessContext(c);
            let w = create(WS_EX_NOACTIVATE, cls, wz!(""), WS_POPUP, 10, 10, 100, 50, core::ptr::null_mut(), 0);
            say!(t, "created under {}: window {} GetDpiForWindow {}", dp_ctx(c), dp_wa(w), GetDpiForWindow(w));
            SetThreadDpiAwarenessContext(DPI_UNAWARE);
            say!(t, "  after the thread switched to UNAWARE: window {} GetDpiForWindow {}", dp_wa(w), GetDpiForWindow(w));
            let ch = create(0, cls, wz!(""), WS_CHILD, 1, 1, 20, 20, w, 1);
            say!(t, "  child created under UNAWARE: {}", dp_wa(ch));
            SetThreadDpiAwarenessContext(DPI_PER_MONITOR_AWARE_V2);
            let ow = CreateWindowExW(WS_EX_NOACTIVATE, cls.as_ptr(), core::ptr::null(), WS_POPUP, 0, 0, 10, 10, w, core::ptr::null_mut(), instance(), core::ptr::null_mut());
            say!(t, "  owned window created under PER_MONITOR_AWARE_V2: {}", dp_wa(ow));
            DestroyWindow(ow);
            DestroyWindow(w);
        }
    }
}
const CTXS_: [isize; 5] = [DPI_UNAWARE, DPI_SYSTEM_AWARE, DPI_PER_MONITOR_AWARE, DPI_PER_MONITOR_AWARE_V2, DPI_UNAWARE_GDISCALED];
