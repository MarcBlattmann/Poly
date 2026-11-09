//! GetSystemMetricsForDpi gives the same numbers for a process that is set to
//! PER_MONITOR_AWARE_V2 as for an unaware one (the values depend on the DPI
//! argument only), and GetSystemMetrics (no DPI) equals GetSystemMetricsForDpi
//! at 96 for every index 0..99 there.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
const SM_CYCAPTION: i32 = 4;
const SM_CXFRAME: i32 = 32;
const SM_CXPADDEDBORDER: i32 = 92;
const SM_CYMENU: i32 = 15;
const SM_CXVSCROLL: i32 = 2;
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

const SM_CXICON: i32 = 11;
const SM_CXSIZE: i32 = 30;
const DPI_PER_MONITOR_AWARE_V2: isize = -4;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
    fn SetProcessDpiAwarenessContext(ctx: isize) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe {
        say!(
            t,
            "SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2) -> {}",
            SetProcessDpiAwarenessContext(DPI_PER_MONITOR_AWARE_V2)
        );
        let mut same = 0;
        let mut diff = Buf::<300>::new();
        for i in 0..100 {
            if GetSystemMetrics(i) == GetSystemMetricsForDpi(i, 96) {
                same += 1;
            } else {
                let _ = write!(diff, " {}", i);
            }
        }
        say!(t, "indices where GetSystemMetrics == ForDpi(96): {} of 100 ; different:{}", same, diff.as_str());
        for i in [SM_CYCAPTION, SM_CXVSCROLL, SM_CXFRAME, SM_CXPADDEDBORDER, SM_CXICON, SM_CYMENU, SM_CXSIZE] {
            say!(
                t,
                "index {:3}: 96 {} 120 {} 144 {} 192 {} 288 {}",
                i,
                GetSystemMetricsForDpi(i, 96),
                GetSystemMetricsForDpi(i, 120),
                GetSystemMetricsForDpi(i, 144),
                GetSystemMetricsForDpi(i, 192),
                GetSystemMetricsForDpi(i, 288)
            );
        }
    }
}
