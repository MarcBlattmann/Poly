//! GetSysColor with indices outside 0..30 (-1, 31, 32, 100, 0x1000,
//! 0x7FFFFFFF): 0 and ERROR_INVALID_PARAMETER (87); GetSysColorBrush of them
//! is NULL; the same brush handle comes back for the same index (stock-like).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HBRUSH = *mut c_void;
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
    fn GetSysColor(index: i32) -> u32;
    fn GetSysColorBrush(index: i32) -> HBRUSH;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain queries.
    unsafe {
        for i in [-1, 31, 32, 33, 100, 0x1000, 0x7FFF_FFFF, i32::MIN] {
            set_error(0x1234);
            let c = GetSysColor(i);
            let e = last_error();
            set_error(0x1234);
            let b = GetSysColorBrush(i);
            say!(
                t,
                "index {:#x}: colour {:#x} last error {} ; brush NULL {} last error {}",
                i,
                c,
                e,
                b.is_null(),
                last_error()
            );
        }
        for i in [0, 5, 15, 30] {
            say!(t, "brush {} the same twice: {}", i, GetSysColorBrush(i) == GetSysColorBrush(i));
        }
    }
}
