//! GetSystemMetricsForDpi: for every index 0..99, whether the value follows
//! the DPI and which DPI values the call refuses. "S" an index that scales
//! (DPI 0x10000 is refused with 87, 0x10001 answers as DPI 1), "U" an index
//! that does not (every DPI from 1 to 0x7FFFFFFF is accepted), "X" an index
//! that is not defined (87 at every DPI); "B" is printed when DPI 0 and
//! DPI 0x80000000..0xFFFFFFFF are not refused for the index.
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
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain query (the first such call of the process clears the last error).
    unsafe { GetSystemMetrics(0) };
    let mut line = Buf::<300>::new();
    let mut count_s = 0;
    let mut count_u = 0;
    let mut count_x = 0;
    for i in 0..100 {
        set_error(0x1234);
        // SAFETY: plain query.
        let r96 = unsafe { GetSystemMetricsForDpi(i, 96) };
        let e96 = last_error();
        set_error(0x1234);
        // SAFETY: plain query.
        let big = unsafe { GetSystemMetricsForDpi(i, 0x10000) };
        let ebig = last_error();
        // SAFETY: plain queries.
        let (b1, d1) = unsafe { (GetSystemMetricsForDpi(i, 0x10001), GetSystemMetricsForDpi(i, 1)) };
        let mut bad_ok = false;
        for d in [0u32, 0x8000_0000, 0x8000_0001, 0xFFFF_FFFE, 0xFFFF_FFFF] {
            set_error(0x1234);
            // SAFETY: plain query.
            let v = unsafe { GetSystemMetricsForDpi(i, d) };
            if last_error() != 87 || v != 0 {
                bad_ok = true;
            }
        }
        let class = if e96 == 87 && r96 == 0 {
            count_x += 1;
            'X'
        } else if ebig == 87 && big == 0 && b1 == d1 {
            count_s += 1;
            'S'
        } else if ebig == 0x1234 {
            count_u += 1;
            'U'
        } else {
            '?'
        };
        let _ = write!(line, "{i}:{class}{} ", if bad_ok { "B" } else { "" });
        if line.n > 240 {
            t.line(line.as_str());
            line = Buf::<300>::new();
        }
    }
    t.line(line.as_str());
    say!(t, "S {count_s} U {count_u} X {count_x}");
}
