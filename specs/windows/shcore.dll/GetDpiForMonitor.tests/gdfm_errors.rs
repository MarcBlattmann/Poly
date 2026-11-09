//! GetDpiForMonitor with a monitor handle that is NULL, made up or -1: HRESULT
//! 0x80070006 (E_HANDLE, ERROR_INVALID_HANDLE) for a valid type, outputs
//! untouched, last error 6; an invalid type is reported before the invalid
//! monitor (0x800700A0); a NULL output pointer fails with 0x800701E7
//! (ERROR_INVALID_ADDRESS) without writing the other.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

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

type HMONITOR = *mut c_void;
const MONITOR_DEFAULTTOPRIMARY: u32 = 1;
const MDT_EFFECTIVE_DPI: i32 = 0;
const MDT_ANGULAR_DPI: i32 = 1;
const MDT_RAW_DPI: i32 = 2;
/// The primary monitor.
fn dp_primary() -> HMONITOR {
    // SAFETY: plain query.
    unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MonitorFromPoint(pt: POINT, flags: u32) -> HMONITOR;
}

#[link(name = "shcore", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDpiForMonitor(mon: HMONITOR, kind: i32, x: *mut u32, y: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let m = dp_primary();
    // SAFETY: out pointers are valid or NULL on purpose.
    unsafe {
        for (label, mon) in
            [("NULL", core::ptr::null_mut()), ("made up", 0x1234 as HMONITOR), ("-1", -1isize as HMONITOR)]
        {
            for k in [MDT_EFFECTIVE_DPI, MDT_ANGULAR_DPI, MDT_RAW_DPI, 3] {
                let (mut x, mut y) = (0xAAAAu32, 0xBBBBu32);
                set_error(0x1234);
                let hr = GetDpiForMonitor(mon, k, &mut x, &mut y);
                say!(t, "{label} monitor, type {k}: hr {:#x} x {:#x} y {:#x} last error {:#x}", hr, x, y, last_error());
            }
        }
        let mut x = 0xAAAAu32;
        let hr = GetDpiForMonitor(m, 0, &mut x, core::ptr::null_mut());
        say!(t, "NULL y: hr {:#x} x {:#x}", hr, x);
        let mut y = 0xBBBBu32;
        let hr = GetDpiForMonitor(m, 0, core::ptr::null_mut(), &mut y);
        say!(t, "NULL x: hr {:#x} y {:#x}", hr, y);
        let hr = GetDpiForMonitor(m, 0, core::ptr::null_mut(), core::ptr::null_mut());
        say!(t, "both NULL: hr {:#x}", hr);
        let hr = GetDpiForMonitor(m, 2, &mut x, core::ptr::null_mut());
        say!(t, "type 2 with NULL y: hr {:#x}", hr);
    }
}
