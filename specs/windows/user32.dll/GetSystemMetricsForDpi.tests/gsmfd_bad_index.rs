//! GetSystemMetricsForDpi with indices that are not defined: negative,
//! 0x1000 (SM_REMOTESESSION is 0x1000: 0 and last error unchanged), 0x2000,
//! 1000, 0x7FFFFFFF: 0, with ERROR_INVALID_PARAMETER for negative and large
//! indices.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

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
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain queries.
    unsafe {
        for i in [-1, -100, 100, 1000, 0x0FFF, 0x1000, 0x1001, 0x2000, 0x2001, 0x7FFF_FFFF, i32::MIN] {
            set_error(0x1234);
            let v = GetSystemMetricsForDpi(i, 96);
            say!(t, "index {:#x} -> {} last error {:#x}", i, v, last_error());
        }
    }
}
