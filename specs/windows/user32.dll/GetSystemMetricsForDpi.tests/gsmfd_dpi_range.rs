//! GetSystemMetricsForDpi with DPI values that are not usual: which values
//! are accepted for the indices that scale and for those that do not, and
//! what a DPI above 65535 means. Each line is one index; a number is the
//! result, "!" after it means the call set the last error to 87
//! (ERROR_INVALID_PARAMETER), "?n" some other last error n.
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
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // the first call of the process to some functions clears the last error
    // (SystemParametersInfoW sheet): a call that is not about the DPI goes first
    // SAFETY: plain query.
    unsafe { GetSystemMetrics(0) };
    let dpis: [u32; 12] = [1, 0xFFFE, 0xFFFF, 0x10000, 0x10001, 0x1FFFF, 0x20000, 0xFFFFF, 0x100000, 0x7FFF_FFFE, 0x7FFF_FFFF, 0x1234_5678];
    let mut b = Buf::<300>::new();
    let _ = write!(b, "dpis:");
    for d in dpis {
        let _ = write!(b, " {d:#x}");
    }
    t.line(b.as_str());
    // indices whose values do not depend on the machine's screen or taskbar
    let idx: [i32; 31] = [2, 3, 4, 7, 9, 10, 11, 13, 15, 20, 28, 29, 30, 31, 32, 36, 38, 45, 47, 49, 51, 52, 54, 55, 56, 57, 71, 85, 87, 92, 5];
    for i in idx {
        let mut b = Buf::<300>::new();
        let _ = write!(b, "index {i:3}:");
        for d in dpis {
            set_error(0x1234);
            // SAFETY: plain query.
            let v = unsafe { GetSystemMetricsForDpi(i, d) };
            let e = last_error();
            if e == 0x1234 {
                let _ = write!(b, " {v}");
            } else if e == 87 {
                let _ = write!(b, " {v}!");
            } else {
                let _ = write!(b, " {v}?{e}");
            }
        }
        t.line(b.as_str());
    }
}
