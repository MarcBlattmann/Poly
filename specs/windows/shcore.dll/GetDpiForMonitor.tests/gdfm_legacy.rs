//! GetProcessDpiAwareness (the shcore call that reports the same awareness as
//! GetDpiForMonitor depends on): NULL process handle or the current process
//! handle: S_OK and PROCESS_DPI_UNAWARE (0) for a manifest-less process; an
//! invalid process handle: 0x80070057 (E_INVALIDARG) and -1; IsProcessDPIAware 0.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
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

/// GetDpiForMonitor of the primary monitor for every type.
fn dp_monitor_report(t: &mut Test) {
    let m = dp_primary();
    for k in [MDT_EFFECTIVE_DPI, MDT_ANGULAR_DPI, MDT_RAW_DPI, 3, -1] {
        let (mut x, mut y) = (0xAAAAu32, 0xBBBBu32);
        // SAFETY: valid out pointers.
        let hr = unsafe { GetDpiForMonitor(m, k, &mut x, &mut y) };
        say!(t, "GetDpiForMonitor(type {}) {:#x} x {:#x} y {:#x}", k, hr, x, y);
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcess() -> HANDLE;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MonitorFromPoint(pt: POINT, flags: u32) -> HMONITOR;
    fn IsProcessDPIAware() -> BOOL;
}

#[link(name = "shcore", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDpiForMonitor(mon: HMONITOR, kind: i32, x: *mut u32, y: *mut u32) -> i32;
    fn GetProcessDpiAwareness(process: HANDLE, out: *mut i32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls with valid out pointers.
    unsafe {
        let mut a = -99;
        set_error(0x1234);
        let hr = GetProcessDpiAwareness(core::ptr::null_mut(), &mut a);
        say!(t, "GetProcessDpiAwareness(NULL) {:#x} {} last error {:#x}", hr, a, last_error());
        let hr = GetProcessDpiAwareness(GetCurrentProcess(), &mut a);
        say!(t, "GetProcessDpiAwareness(current) {:#x} {}", hr, a);
        a = -99;
        let hr = GetProcessDpiAwareness(0x1234 as HANDLE, &mut a);
        say!(t, "GetProcessDpiAwareness(made up) {:#x} {}", hr, a);
        say!(t, "IsProcessDPIAware {}", IsProcessDPIAware());
        dp_monitor_report(t);
    }
}
