//! MonitorFromPoint with flag values other than the three defined ones
//! (3, 4, 0x10, 0x100, 0x80000000, 0xFFFFFFFF), for a point on the screen
//! and one far off it: the monitor returned ("primary", "other", "null") and
//! the last error (set to 0 first); and the last error after successful
//! calls with the defined flags.
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MonitorFromPoint(pt: POINT, flags: u32) -> HMONITOR;
}

fn mon_kind(m: HMONITOR, primary: HMONITOR) -> &'static str {
    if m.is_null() {
        "null"
    } else if m == primary {
        "primary"
    } else {
        "other"
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain query.
    let primary = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    for (label, p) in [("on the screen", POINT { x: 10, y: 10 }), ("far off", POINT { x: -5000, y: -5000 })] {
        for flags in [0u32, 1, 2, 3, 4, 0x10, 0x100, 0x8000_0000, 0xFFFF_FFFF] {
            set_error(0x1234);
            // SAFETY: plain query; bad flags are what is observed.
            let m = unsafe { MonitorFromPoint(p, flags) };
            say!(t, "{label} flags {flags:#x}: {} error {}", mon_kind(m, primary), last_error());
        }
    }
}
