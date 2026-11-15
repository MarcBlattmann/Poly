//! GetMessageTime inside a window procedure of a plain hidden window: the
//! time of a posted message is the time it was posted (not when it was
//! taken from the queue), checked against the tick count and against the
//! `time` member of the MSG the loop dispatches; three messages posted one
//! after the other have non-decreasing times; two messages posted 120 ms
//! apart and taken together are at least 50 ms apart; inside a message sent
//! while a posted one is handled the call returns the posted message's time;
//! a message sent from the loop's body and a call between messages return
//! the time of the last message taken; PeekMessageW with PM_NOREMOVE is
//! observed (does it count as retrieving the message?). Only relations are
//! printed, never a time.
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

const WM_APP: u32 = 0x8000;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const GWLP_USERDATA: i32 = -21;
const PM_NOREMOVE: u32 = 0x0000;
const PM_REMOVE: u32 = 0x0001;
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

/// Whether a time (a tick count of the message functions) is within ten
/// seconds of the current tick count, wrap-safe.
fn near_tick(t: u32) -> bool {
    // SAFETY: plain call.
    let now = unsafe { GetTickCount() };
    (now.wrapping_sub(t) as i32).unsigned_abs() < 10_000
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetCurrentProcessId() -> u32;
    fn Sleep(ms: u32);
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
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn SendMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn PostMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> BOOL;
    fn GetMessageTime() -> i32;
}

const M_POST: u32 = WM_APP + 1;
const M_OUTER: u32 = WM_APP + 3;
const M_INNER: u32 = WM_APP + 4;

static TIMES: Global<[i32; 8]> = Global::new([0; 8]);
static NTIMES: Global<usize> = Global::new(0);
static CUR: Global<i32> = Global::new(0);
static MATCHES: Global<u32> = Global::new(0);
static OUTER: Global<i32> = Global::new(0);
static INNER: Global<i32> = Global::new(0);

unsafe extern "system" fn proc_time(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: plain calls on the window being served.
    unsafe {
        match msg {
            M_POST => {
                let n = *NTIMES.get();
                let tm = GetMessageTime();
                if n < 8 {
                    TIMES.get()[n] = tm;
                }
                *NTIMES.get() = n + 1;
                if tm == *CUR.get() {
                    *MATCHES.get() += 1;
                }
                0
            }
            M_OUTER => {
                *OUTER.get() = GetMessageTime();
                SendMessageW(hwnd, M_INNER, 0, 0);
                0
            }
            M_INNER => {
                *INNER.get() = GetMessageTime();
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

/// Take every waiting message out of the queue and dispatch it, remembering
/// each message's own `time` for the procedure to compare.
fn run_queue() -> u32 {
    let mut msg: MSG = zeroed();
    let mut n = 0;
    // SAFETY: msg is writable; it is the message just removed.
    while unsafe { PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
        *CUR.get() = msg.time as i32;
        // SAFETY: dispatching the message just removed.
        unsafe { DispatchMessageW(&msg) };
        n += 1;
    }
    n
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain query.
    let first = unsafe { GetMessageTime() };
    let class = if first == 0 {
        "zero"
    } else if near_tick(first as u32) {
        "near the tick count"
    } else {
        "something else"
    };
    say!(t, "before any message was taken: {class}");

    t.check("registered", register(wz!("MsgTime"), Some(proc_time), 0, 0, core::ptr::null_mut()) != 0, 0);
    let h = create(0, wz!("MsgTime"), &[], WS_OVERLAPPED, 0, 0, 10, 10, core::ptr::null_mut(), 0);
    say!(t, "window created: {}", (!h.is_null()) as i32);

    say!(t, "== three messages posted one after the other");
    // SAFETY: our window.
    unsafe {
        for i in 0..3 {
            PostMessageW(h, M_POST, i, 0);
        }
    }
    let n = run_queue();
    let ts = *TIMES.get();
    say!(t, "  taken {n}, handled {}", *NTIMES.get());
    say!(t, "  each time is near the tick count: {} {} {}", near_tick(ts[0] as u32) as i32, near_tick(ts[1] as u32) as i32, near_tick(ts[2] as u32) as i32);
    say!(t, "  non-decreasing: {}", (ts[0] as u32 <= ts[1] as u32 && ts[1] as u32 <= ts[2] as u32) as i32);
    say!(t, "  GetMessageTime inside the procedure equals MSG.time in {} of 3", *MATCHES.get());
    // SAFETY: plain query.
    let after = unsafe { GetMessageTime() };
    say!(t, "  after the loop: equals the last message's time: {}", (after == ts[2]) as i32);

    say!(t, "== two messages posted 120 ms apart, taken together");
    *NTIMES.get() = 0;
    // SAFETY: our window; plain calls.
    unsafe {
        PostMessageW(h, M_POST, 10, 0);
        Sleep(120);
        PostMessageW(h, M_POST, 11, 0);
    }
    run_queue();
    let ts = *TIMES.get();
    let d = (ts[1] as u32).wrapping_sub(ts[0] as u32);
    say!(t, "  handled {}", *NTIMES.get());
    say!(t, "  the second is at least 50 ms after the first: {}", (d >= 50 && d < 5000) as i32);
    // SAFETY: plain query.
    let now = unsafe { GetTickCount() };
    say!(t, "  the first message is at least 100 ms old although it was taken just now: {}", (now.wrapping_sub(ts[0] as u32) >= 100) as i32);

    say!(t, "== a message sent while a posted one is handled");
    // SAFETY: our window.
    unsafe { PostMessageW(h, M_OUTER, 0, 0) };
    run_queue();
    say!(t, "  the sent message sees the posted message's time: {}", (*INNER.get() == *OUTER.get()) as i32);
    say!(t, "  the posted message's time is near the tick count: {}", near_tick(*OUTER.get() as u32) as i32);

    say!(t, "== a message sent from the loop's body, a call after a pause");
    let last = *OUTER.get();
    // SAFETY: our window; plain calls.
    unsafe {
        Sleep(80);
        SendMessageW(h, M_INNER, 0, 0);
        say!(t, "  the sent message sees the last taken message's time: {}", (*INNER.get() == last) as i32);
        say!(t, "  a direct call 80 ms later returns the same: {}", (GetMessageTime() == last) as i32);
    }

    say!(t, "== PeekMessageW with PM_NOREMOVE of a message posted later");
    // SAFETY: our window; plain calls; msg is writable.
    unsafe {
        Sleep(60);
        PostMessageW(h, M_POST, 20, 0);
        let mut msg: MSG = zeroed();
        let got = PeekMessageW(&mut msg, core::ptr::null_mut(), M_POST, M_POST, PM_NOREMOVE);
        let seen = GetMessageTime();
        say!(t, "  peeked: {got}");
        say!(t, "  the message is at least 50 ms newer than the last taken one: {}", (msg.time.wrapping_sub(last as u32) >= 50) as i32);
        say!(t, "  GetMessageTime afterwards is still the last taken message's: {}", (seen == last) as i32);
        say!(t, "  GetMessageTime afterwards is the peeked message's: {}", (seen == msg.time as i32) as i32);
    }
    *NTIMES.get() = 0;
    run_queue();
    say!(t, "  taken and handled afterwards: {}", *NTIMES.get());

    // SAFETY: our window.
    unsafe { DestroyWindow(h) };
}
