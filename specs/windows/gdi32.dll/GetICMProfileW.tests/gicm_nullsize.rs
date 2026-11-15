//! GetICMProfileW with a NULL size pointer (with a buffer and without one)
//! on a screen DC. This is the one input the documentation does not
//! describe, so it has its own test: the return value and the last error
//! are printed (the call may fail or the process may be ended by an access
//! violation on some systems; the earlier lines are then still recorded).
//! A call with a valid size pointer follows to show the DC is still usable.
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
type HDC = *mut c_void;
type BOOL = i32;
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetICMProfileW(hdc: HDC, size: *mut u32, name: *mut u16) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut buf = [0u16; 300];
    // SAFETY: one screen DC; the buffer holds 300 characters.
    unsafe {
        let dc = GetDC(core::ptr::null_mut());
        t.line("before the calls");
        set_error(0x5A5A);
        let r = GetICMProfileW(dc, core::ptr::null_mut(), buf.as_mut_ptr());
        let e = last_error();
        say!(t, "NULL size, buffer: returns {r}, error {} (unchanged {}), buffer touched {}", if e == 0x5A5A { 0 } else { e }, (e == 0x5A5A) as i32, (buf[0] != 0) as i32);
        set_error(0x5A5A);
        let r = GetICMProfileW(dc, core::ptr::null_mut(), core::ptr::null_mut());
        let e = last_error();
        say!(t, "NULL size, NULL buffer: returns {r}, error {} (unchanged {})", if e == 0x5A5A { 0 } else { e }, (e == 0x5A5A) as i32);
        let mut size: u32 = 300;
        let r = GetICMProfileW(dc, &mut size, buf.as_mut_ptr());
        say!(t, "valid call afterwards: returns {r}, size nonzero {}", (size != 0) as i32);
        ReleaseDC(core::ptr::null_mut(), dc);
    }
}
