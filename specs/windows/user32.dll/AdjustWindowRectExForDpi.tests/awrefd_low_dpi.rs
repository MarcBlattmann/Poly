//! AdjustWindowRectExForDpi for DPIs at the very low end (1, 2, 24, 47, 48, 72)
//! and very high (1000, 4000) for a WS_OVERLAPPEDWINDOW with a menu
//! and the metrics at those DPIs: the frame, caption and menu measures follow
//! GetSystemMetricsForDpi of the same DPI (including its rounding at the low
//! end); "last error -1" in the output means the call succeeded and set no error code, which the second line of each pair prints.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const SM_CYCAPTION: i32 = 4;
const SM_CXFRAME: i32 = 32;
const SM_CXPADDEDBORDER: i32 = 92;
const SM_CYMENU: i32 = 15;
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

/// "l,t,r,b" of a rectangle.
struct R(RECT);

impl core::fmt::Display for R {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{},{},{},{}", self.0.left, self.0.top, self.0.right, self.0.bottom)
    }
}

/// The last error of a call for printing: -1 when the call succeeded and
/// left it as it was (0x1234, set before) or cleared (0): a success does
/// not promise either.
fn dp_e(ok: i32, e: u32) -> i64 {
    if ok != 0 && (e == 0 || e == 0x1234) { -1 } else { e as i64 }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
    fn AdjustWindowRectExForDpi(rect: *mut RECT, style: u32, menu: BOOL, ex_style: u32, dpi: u32) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for d in [1u32, 2, 24, 47, 48, 72, 1000, 4000] {
        let mut r = RECT { left: 0, top: 0, right: 100, bottom: 100 };
        set_error(0x1234);
        // SAFETY: r is writable.
        let ok = unsafe { AdjustWindowRectExForDpi(&mut r, WS_OVERLAPPEDWINDOW, 1, 0, d) };
        say!(t, "DPI {d}: {} {} last error {}", ok, R(r), dp_e(ok, last_error()));
        // SAFETY: plain queries.
        unsafe {
            say!(
                t,
                "   metrics: frame {} padded {} caption {} menu {}",
                GetSystemMetricsForDpi(SM_CXFRAME, d),
                GetSystemMetricsForDpi(SM_CXPADDEDBORDER, d),
                GetSystemMetricsForDpi(SM_CYCAPTION, d),
                GetSystemMetricsForDpi(SM_CYMENU, d)
            );
        }
    }
}
