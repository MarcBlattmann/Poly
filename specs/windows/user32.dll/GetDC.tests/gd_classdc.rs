//! GetDC and the class DC styles: without CS_OWNDC/CS_CLASSDC two GetDC
//! calls on one window (without ReleaseDC) give two different DCs; with
//! CS_OWNDC each window has its own DC, returned every time and keeping
//! its state (also across ReleaseDC, which returns 1); with CS_CLASSDC two
//! windows of the class share one DC; GetDC of a window of another thread
//! works (observed).
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
type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
type COLORREF = u32;
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

const WS_POPUP: u32 = 0x8000_0000;
const CS_OWNDC: u32 = 0x0020;
const CS_CLASSDC: u32 = 0x0040;
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

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
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

/// "window" or "null" (a handle is never printed).
fn hwnd_kind(h: HWND) -> &'static str {
    if h.is_null() { "null" } else { "window" }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn CreateThread(
        attributes: *mut c_void,
        stack: usize,
        start: THREADPROC,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn CreateEventW(attributes: *mut c_void, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn SetEvent(event: HANDLE) -> BOOL;
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
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn SetBkColor(hdc: HDC, color: COLORREF) -> COLORREF;
    fn GetBkColor(hdc: HDC) -> COLORREF;
}

static OTHER: Global<usize> = Global::new(0);
static READY: Global<usize> = Global::new(0);
static DONE: Global<usize> = Global::new(0);

unsafe extern "system" fn proc_a(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: the default handling.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

unsafe extern "system" fn thread_main(_: *mut c_void) -> u32 {
    let h = create(0, wz!("GdcPlain"), wz!(""), WS_POPUP, 0, 0, 10, 10, core::ptr::null_mut(), 0);
    *OTHER.get() = h as usize;
    // SAFETY: events made by the main thread.
    unsafe {
        SetEvent(*READY.get() as HANDLE);
        WaitForSingleObject(*DONE.get() as HANDLE, 10000);
        DestroyWindow(h);
    }
    0
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    t.check("plain", register(wz!("GdcPlain"), Some(proc_a), 0, 0, core::ptr::null_mut()) != 0, 0);
    t.check("own", register(wz!("GdcOwn"), Some(proc_a), CS_OWNDC, 0, core::ptr::null_mut()) != 0, 0);
    t.check("class", register(wz!("GdcClass"), Some(proc_a), CS_CLASSDC, 0, core::ptr::null_mut()) != 0, 0);
    let none = core::ptr::null_mut();
    let p = create(0, wz!("GdcPlain"), wz!(""), WS_POPUP, 0, 0, 10, 10, none, 0);
    let o1 = create(0, wz!("GdcOwn"), wz!(""), WS_POPUP, 0, 0, 10, 10, none, 0);
    let o2 = create(0, wz!("GdcOwn"), wz!(""), WS_POPUP, 0, 0, 10, 10, none, 0);
    let c1 = create(0, wz!("GdcClass"), wz!(""), WS_POPUP, 0, 0, 10, 10, none, 0);
    let c2 = create(0, wz!("GdcClass"), wz!(""), WS_POPUP, 0, 0, 10, 10, none, 0);
    // SAFETY: GetDC/ReleaseDC on our windows.
    unsafe {
        let a = GetDC(p);
        let b = GetDC(p);
        say!(t, "plain: two GetDC without release differ {}", (a != b) as i32);
        say!(t, "plain: releases {} {}", ReleaseDC(p, a), ReleaseDC(p, b));
        let x = GetDC(o1);
        let y = GetDC(o2);
        SetBkColor(x, rgb(5, 6, 7));
        let x2 = GetDC(o1);
        say!(
            t,
            "CS_OWNDC: same DC again {} windows differ {} bk kept {:#08x}",
            (x == x2) as i32,
            (x != y) as i32,
            GetBkColor(x2)
        );
        say!(t, "CS_OWNDC: releases {} {} {}", ReleaseDC(o1, x), ReleaseDC(o1, x2), ReleaseDC(o2, y));
        let x3 = GetDC(o1);
        say!(t, "CS_OWNDC: after release same DC {} bk {:#08x}", (x3 == x) as i32, GetBkColor(x3));
        ReleaseDC(o1, x3);
        let k1 = GetDC(c1);
        let k2 = GetDC(c2);
        say!(t, "CS_CLASSDC: two windows share the DC {}", (k1 == k2) as i32);
        ReleaseDC(c1, k1);
        ReleaseDC(c2, k2);
        *READY.get() = CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null()) as usize;
        *DONE.get() = CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null()) as usize;
        let th = CreateThread(core::ptr::null_mut(), 0, thread_main, core::ptr::null_mut(), 0, core::ptr::null_mut());
        WaitForSingleObject(*READY.get() as HANDLE, 10000);
        let other = *OTHER.get() as HWND;
        let od = GetDC(other);
        say!(t, "other thread's window: dc {} type {}", hwnd_kind(od), GetObjectType(od));
        if !od.is_null() {
            say!(t, "  release {}", ReleaseDC(other, od));
        }
        SetEvent(*DONE.get() as HANDLE);
        WaitForSingleObject(th, 10000);
        CloseHandle(th);
        for h in [p, o1, o2, c1, c2] {
            DestroyWindow(h);
        }
    }
}
