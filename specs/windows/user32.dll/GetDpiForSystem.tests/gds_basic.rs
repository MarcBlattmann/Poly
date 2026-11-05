//! GetDpiForSystem in a DPI-unaware process: the value, that it does not
//! change the last error, that it equals LOGPIXELSX and LOGPIXELSY of the
//! screen DC and of a memory DC, GetDpiForWindow of the desktop window, and
//! that GetSystemMetricsForDpi at that DPI equals GetSystemMetrics for a few
//! indices.
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
const SM_CXSCREEN: i32 = 0;
const SM_CYCAPTION: i32 = 4;
const SM_CXFRAME: i32 = 32;
const SM_CXPADDEDBORDER: i32 = 92;
const SM_CYMENU: i32 = 15;
const SM_CXVSCROLL: i32 = 2;
const LOGPIXELSX: i32 = 88;
const LOGPIXELSY: i32 = 90;
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

const SM_CXICON: i32 = 11;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDesktopWindow() -> HWND;
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetDpiForWindow(hwnd: HWND) -> u32;
    fn GetDpiForSystem() -> u32;
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn GetDeviceCaps(hdc: HDC, index: i32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    set_error(0x1234);
    // SAFETY: plain query.
    let d = unsafe { GetDpiForSystem() };
    say!(t, "GetDpiForSystem {d} error {:#x}", last_error());
    // SAFETY: plain queries; DCs are released/deleted.
    unsafe {
        let dc = GetDC(core::ptr::null_mut());
        say!(t, "screen DC LOGPIXELSX {} LOGPIXELSY {}", GetDeviceCaps(dc, LOGPIXELSX), GetDeviceCaps(dc, LOGPIXELSY));
        ReleaseDC(core::ptr::null_mut(), dc);
        let mdc = CreateCompatibleDC(core::ptr::null_mut());
        say!(
            t,
            "memory DC LOGPIXELSX {} LOGPIXELSY {}",
            GetDeviceCaps(mdc, LOGPIXELSX),
            GetDeviceCaps(mdc, LOGPIXELSY)
        );
        DeleteDC(mdc);
        say!(t, "GetDpiForWindow(desktop) {}", GetDpiForWindow(GetDesktopWindow()));
        let mut same = 0;
        for i in [SM_CXSCREEN, SM_CYCAPTION, SM_CXVSCROLL, SM_CXICON, SM_CXFRAME, SM_CXPADDEDBORDER, SM_CYMENU] {
            if GetSystemMetricsForDpi(i, d) == GetSystemMetrics(i) {
                same += 1;
            }
        }
        t.expect("GetSystemMetricsForDpi(i, system DPI) == GetSystemMetrics(i) for 7 indices", same, 7);
    }
}
