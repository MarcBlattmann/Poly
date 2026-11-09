//! GetSystemMetricsForDpi for the indices 0..99 at the DPIs 96, 120, 144, 168,
//! 192, 240, 288 and 384 (one row per index), next to GetSystemMetrics.
//! Sizes scale; counts, flags, screen and work-area values do not; borders,
//! edges and double-click rectangles stay the same.
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

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let dpis = [96u32, 120, 144, 168, 192, 240, 288, 384];
    // SAFETY: plain queries.
    unsafe {
        for i in 0..100 {
            let mut row = Buf::<300>::new();
            for d in dpis {
                let _ = write!(row, " {}", GetSystemMetricsForDpi(i, d));
            }
            say!(t, "index {:3}:{} | GetSystemMetrics {}", i, row.as_str(), GetSystemMetrics(i));
        }
    }
}
