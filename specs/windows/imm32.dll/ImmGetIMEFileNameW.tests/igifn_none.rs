//! ImmGetIMEFileNameW for the thread's current keyboard layout and for every
//! installed layout of the machine (here none is an IME): the length is 0,
//! with a NULL buffer and with a buffer; with a buffer of at least one unit
//! it writes an empty string (the first unit becomes 0, the rest is left
//! alone); for NULL, a layout that is not installed and a made-up handle the
//! length is 0 too and the buffer is not touched at all. The values of the
//! layout handles are not printed.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
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

type HKL = *mut c_void;
/// The cursor's position.
fn cursor_pos() -> POINT {
    let mut p = POINT::default();
    // SAFETY: p is writable.
    unsafe { GetCursorPos(&mut p) };
    p
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCursorPos(pt: *mut POINT) -> BOOL;
    fn SetCursorPos(x: i32, y: i32) -> BOOL;
    fn GetKeyboardLayout(thread: u32) -> HKL;
}

#[link(name = "imm32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ImmGetIMEFileNameW(hkl: HKL, buf: *mut u16, len: u32) -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetKeyboardLayoutList(n: i32, list: *mut HKL) -> i32;
}

poly_wtest::main!(test);

fn one(t: &mut Test, label: &str, h: HKL) {
    let mut buf = [0xAAAAu16; 300];
    set_error(0x1234);
    // SAFETY: the buffers are as long as the length given.
    let (len0, len, small) = unsafe {
        (
            ImmGetIMEFileNameW(h, core::ptr::null_mut(), 0),
            ImmGetIMEFileNameW(h, buf.as_mut_ptr(), 300),
            ImmGetIMEFileNameW(h, buf.as_mut_ptr(), 3),
        )
    };
    say!(
        t,
        "{label}: NULL buffer {len0}, 300-unit buffer {len}, 3-unit buffer {small}, first unit {:#x}, rest untouched {}, last error {:#x}",
        buf[0],
        buf[1..].iter().all(|&c| c == 0xAAAA),
        last_error()
    );
}

fn test(t: &mut Test) {
    let _cursor = CursorGuard::new();
    // SAFETY: plain queries.
    let (cur, n) = unsafe {
        let mut list = [core::ptr::null_mut::<c_void>(); 32];
        let n = GetKeyboardLayoutList(32, list.as_mut_ptr());
        (GetKeyboardLayout(0), (n, list))
    };
    one(t, "current layout of the thread", cur);
    let (count, list) = n;
    say!(t, "at least one installed layout: {}", count >= 1);
    let mut all_zero = true;
    for i in 0..(count.max(0) as usize).min(32) {
        let mut buf = [0xAAAAu16; 16];
        // SAFETY: the buffer is 16 units.
        let l = unsafe { ImmGetIMEFileNameW(list[i], buf.as_mut_ptr(), 16) };
        all_zero &= l == 0 && buf[0] == 0 && buf[1..].iter().all(|&c| c == 0xAAAA);
    }
    say!(t, "every installed layout: length 0, empty string written, rest untouched: {all_zero}");
    one(t, "NULL layout", core::ptr::null_mut());
    one(t, "layout not installed (0x04090409 or 0xE0010411)", 0xE001_0411usize as HKL);
    one(t, "made-up handle", 0x12345678usize as HKL);
}
