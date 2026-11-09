//! GetCursorInfo argument rules: cbSize must be exactly 24 (the size of
//! CURSORINFO: 0, 8, 16, 20, 23, 25, 28, 1000 and 0xFFFFFFFF give FALSE
//! with ERROR_INVALID_PARAMETER 87 and leave the structure alone); a NULL
//! pointer gives FALSE with ERROR_NOACCESS (998); with cbSize 24 the call
//! writes flags, hCursor and ptScreenPos (garbage values are replaced; the
//! point equals GetCursorPos and is clamped to the screen) and the cbSize
//! member comes back as 0; the last error is not changed on success.
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
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
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

fn lo(v: isize) -> i32 {
    (v as u32 & 0xFFFF) as u16 as i16 as i32
}

fn hi(v: isize) -> i32 {
    ((v as u32 >> 16) & 0xFFFF) as u16 as i16 as i32
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

type HCURSOR = *mut c_void;
/// The cursor's position.
fn cursor_pos() -> POINT {
    let mut p = POINT::default();
    // SAFETY: p is writable.
    unsafe { GetCursorPos(&mut p) };
    p
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

/// Puts the cursor back where it was when created (keep one alive for the
/// whole test in every test that moves the cursor).
struct CursorGuard(POINT);

impl CursorGuard {
    /// Remember the cursor's position and park it at (1500, 600), away from
    /// the test windows (which tests place in the top-left 1000x500 of the
    /// 1920x1080 screen), so no test window starts under it.
    fn new() -> Self {
        let g = CursorGuard(cursor_pos());
        // SAFETY: plain call.
        unsafe { SetCursorPos(1500, 600) };
        g
    }
}

impl Drop for CursorGuard {
    fn drop(&mut self) {
        // SAFETY: plain call.
        unsafe { SetCursorPos(self.0.x, self.0.y) };
    }
}

/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct CURSORINFO {
    cb_size: u32,
    flags: u32,
    cursor: HCURSOR,
    pt: POINT,
}

/// "(lo-hi)" style result text of a BOOL.
fn ly_b(v: BOOL) -> &'static str {
    if v != 0 { "TRUE" } else { "FALSE" }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetTickCount() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn GetCursorPos(pt: *mut POINT) -> BOOL;
    fn SetCursorPos(x: i32, y: i32) -> BOOL;
    fn GetCursorInfo(info: *mut CURSORINFO) -> BOOL;
}

fn show(t: &mut Test, label: &str, size: u32) -> CURSORINFO {
    let mut ci =
        CURSORINFO { cb_size: size, flags: 0xAAAA, cursor: 0x1234usize as HCURSOR, pt: POINT { x: -99, y: -99 } };
    set_error(0x1234);
    // SAFETY: ci is writable (24 bytes).
    let r = unsafe { GetCursorInfo(&mut ci) };
    say!(
        t,
        "{label} {size}: {} error {:#x} cbSize {} flags {:#x} cursor {} pt {},{}",
        ly_b(r),
        last_error(),
        ci.cb_size,
        ci.flags,
        if ci.cursor == 0x1234usize as HCURSOR {
            "untouched"
        } else if ci.cursor.is_null() {
            "NULL"
        } else {
            "set"
        },
        ci.pt.x,
        ci.pt.y
    );
    ci
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let _c = CursorGuard::new();
    // SAFETY: plain calls.
    unsafe {
        say!(t, "sizeof(CURSORINFO) = {}", core::mem::size_of::<CURSORINFO>());
        for sz in [0u32, 8, 16, 20, 23, 24, 25, 28, 1000, 0xFFFF_FFFF] {
            show(t, "cbSize", sz);
        }
        let ci = show(t, "parked", 24);
        say!(t, "  ptScreenPos is GetCursorPos {} ({},{})", ci.pt == cursor_pos(), ci.pt.x, ci.pt.y);
        set_error(0x1234);
        let r = GetCursorInfo(core::ptr::null_mut());
        say!(t, "NULL pointer: {} error {}", ly_b(r), last_error());
        // the cursor moved
        SetCursorPos(321, 123);
        settle(100);
        let ci = show(t, "moved", 24);
        say!(t, "  ptScreenPos {},{} is GetCursorPos {}", ci.pt.x, ci.pt.y, ci.pt == cursor_pos());
        SetCursorPos(0, 0);
        settle(50);
        let ci = show(t, "at the corner", 24);
        say!(t, "  ptScreenPos {},{}", ci.pt.x, ci.pt.y);
        SetCursorPos(5000, 5000);
        settle(50);
        let ci = show(t, "beyond the screen", 24);
        say!(t, "  ptScreenPos {},{} (clamped)", ci.pt.x, ci.pt.y);
    }
}
