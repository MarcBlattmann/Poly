//! RegisterClassW as Chrome's crash handler uses it (S3, the crashpad-handler
//! process): a class with only a name, an instance and a window procedure, a
//! hidden window of it, messages posted to the window and handled by a message
//! loop (WM_USER range, a registered message), then the window is destroyed
//! and the class removed. The same sequence is repeated with the same name
//! (the class is registered again after UnregisterClassW) and with a
//! message-only parent. Prints what the procedure saw (in order, messages of
//! the system left out), not values that depend on the machine.
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

const WM_CREATE: u32 = 0x0001;
const WM_DESTROY: u32 = 0x0002;
const WM_USER: u32 = 0x0400;
const WS_POPUP: u32 = 0x8000_0000;
const HWND_MESSAGE: HWND = -3isize as HWND;
const GWLP_USERDATA: i32 = -21;
const PM_REMOVE: u32 = 0x0001;
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

fn wide_str(buf: &[u16], out: &mut Buf<300>) {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    for &c in &buf[..n] {
        let _ = write!(out, "{}", if (32..127).contains(&c) { c as u8 as char } else { '?' });
    }
}

fn w_ascii(buf: &[u16]) -> Buf<300> {
    let mut b = Buf::<300>::new();
    wide_str(buf, &mut b);
    b
}

/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct WNDCLASSW {
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
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn UnregisterClassW(class: *const u16, instance: HINSTANCE) -> BOOL;
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
    fn IsWindowVisible(hwnd: HWND) -> BOOL;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn SendMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn PostMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> BOOL;
    fn RegisterWindowMessageW(name: *const u16) -> u32;
    fn RegisterClassW(class: *const WNDCLASSW) -> u16;
}

poly_wtest::main!(test);

static SEEN: Cell3 = Cell3::new();

struct Cell3(core::cell::UnsafeCell<[u32; 16]>, core::cell::UnsafeCell<usize>);
// SAFETY: single threaded test.
unsafe impl Sync for Cell3 {}
impl Cell3 {
    const fn new() -> Self {
        Cell3(core::cell::UnsafeCell::new([0; 16]), core::cell::UnsafeCell::new(0))
    }
    fn push(&self, v: u32) {
        // SAFETY: single threaded.
        unsafe {
            let n = &mut *self.1.get();
            if *n < 16 {
                (*self.0.get())[*n] = v;
                *n += 1;
            }
        }
    }
    fn take(&self, out: &mut Buf<200>) {
        // SAFETY: single threaded.
        unsafe {
            let n = &mut *self.1.get();
            for i in 0..*n {
                let _ = write!(out, "{:#x} ", (*self.0.get())[i]);
            }
            *n = 0;
        }
    }
}

unsafe extern "system" fn proc_crash(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_CREATE || msg == WM_DESTROY || msg >= WM_USER {
        SEEN.push(msg);
    }
    // SAFETY: the default handling of the message just received.
    unsafe {
        if msg == WM_USER + 5 {
            return 0x77;
        }
        DefWindowProcW(hwnd, msg, w, l)
    }
}

fn run(t: &mut Test, round: &str, parent: HWND) {
    let wc = WNDCLASSW {
        style: 0,
        wnd_proc: Some(proc_crash),
        cls_extra: 0,
        wnd_extra: 0,
        instance: instance(),
        icon: core::ptr::null_mut(),
        cursor: core::ptr::null_mut(),
        background: core::ptr::null_mut(),
        menu_name: core::ptr::null(),
        class_name: wz!("CrashpadLikeWindow").as_ptr(),
    };
    // SAFETY: our class and window.
    unsafe {
        let atom = RegisterClassW(&wc);
        say!(t, "[{round}] RegisterClassW: atom in range {}", atom >= 0xC000);
        let w = create(0, wz!("CrashpadLikeWindow"), wz!("crash"), if parent.is_null() { 0 } else { WS_POPUP }, 0, 0, 0, 0, parent, 0);
        say!(t, "[{round}] hidden window created {}, visible {}", !w.is_null(), IsWindowVisible(w));
        PostMessageW(w, WM_USER + 1, 1, 2);
        PostMessageW(w, WM_USER + 2, 3, 4);
        let r = SendMessageW(w, WM_USER + 5, 0, 0);
        say!(t, "[{round}] SendMessageW returns the procedure's value 0x77: {}", r == 0x77);
        let mut buf = Buf::<200>::new();
        SEEN.take(&mut buf);
        say!(t, "[{round}] before the loop (WM_CREATE and the sent message only): {}", buf.as_str().trim_end());
        let mut msg: MSG = zeroed();
        let mut n = 0;
        while PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            if msg.hwnd == w && msg.message >= WM_USER {
                DispatchMessageW(&msg);
                n += 1;
            }
        }
        let mut buf = Buf::<200>::new();
        SEEN.take(&mut buf);
        say!(t, "[{round}] after the loop: {n} messages dispatched, seen in order {}", buf.as_str().trim_end());
        // a registered message
        let reg = RegisterWindowMessageW(wz!("CrashpadLikeMessage").as_ptr());
        say!(t, "[{round}] a registered message is in 0xC000..=0xFFFF: {}", reg >= 0xC000);
        let mut name = [0u16; 40];
        GetClassNameW(w, name.as_mut_ptr(), 40);
        say!(t, "[{round}] class name of the window: {}", w_ascii(&name).as_str());
        DestroyWindow(w);
        let mut buf = Buf::<200>::new();
        SEEN.take(&mut buf);
        say!(t, "[{round}] after DestroyWindow: {}", buf.as_str().trim_end());
        say!(t, "[{round}] UnregisterClassW returns {}", UnregisterClassW(wz!("CrashpadLikeWindow").as_ptr(), instance()));
    }
}

fn test(t: &mut Test) {
    run(t, "top-level", core::ptr::null_mut());
    run(t, "again", core::ptr::null_mut());
    run(t, "message-only", HWND_MESSAGE);
    // the same class registered by two RegisterClassW calls with different names gives different atoms; the old name is reusable
    let mk = |name: &'static [u16]| WNDCLASSW {
        style: 0,
        wnd_proc: Some(proc_crash),
        cls_extra: 0,
        wnd_extra: 0,
        instance: instance(),
        icon: core::ptr::null_mut(),
        cursor: core::ptr::null_mut(),
        background: core::ptr::null_mut(),
        menu_name: core::ptr::null(),
        class_name: name.as_ptr(),
    };
    // SAFETY: our classes.
    unsafe {
        let a = RegisterClassW(&mk(wz!("CrashpadA")));
        let b = RegisterClassW(&mk(wz!("CrashpadB")));
        say!(t, "two names, two atoms: {}", a != b && a != 0 && b != 0);
        UnregisterClassW(wz!("CrashpadA").as_ptr(), instance());
        let a2 = RegisterClassW(&mk(wz!("CrashpadA")));
        say!(t, "the removed name registers again: {}", a2 != 0);
        UnregisterClassW(wz!("CrashpadA").as_ptr(), instance());
        UnregisterClassW(wz!("CrashpadB").as_ptr(), instance());
    }
}
