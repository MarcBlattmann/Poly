//! MonitorFromRect for rectangles inside the monitor, partly outside, off the
//! screen (right, left, far), empty and inverted (inside and off), touching the
//! right edge, the last pixel and the whole coordinate range, with the flags
//! MONITOR_DEFAULTTONULL, DEFAULTTOPRIMARY, DEFAULTTONEAREST: a rectangle that
//! overlaps the monitor (also empty or inverted ones inside it) gives the
//! monitor for every flag; one that does not gives NULL for DEFAULTTONULL and
//! the primary monitor for the others.
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

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
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
const MONITOR_DEFAULTTONULL: u32 = 0;
const MONITOR_DEFAULTTOPRIMARY: u32 = 1;
const MONITOR_DEFAULTTONEAREST: u32 = 2;
/// The primary monitor.
fn dp_primary() -> HMONITOR {
    // SAFETY: plain query.
    unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MonitorFromPoint(pt: POINT, flags: u32) -> HMONITOR;
    fn MonitorFromRect(rect: *const RECT, flags: u32) -> HMONITOR;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let cases: [(&str, RECT); 12] = [
        ("inside", RECT { left: 100, top: 100, right: 200, bottom: 200 }),
        ("partly left of the screen", RECT { left: -50, top: 100, right: 50, bottom: 200 }),
        ("off to the right", RECT { left: 2000, top: 100, right: 2100, bottom: 200 }),
        ("off to the left", RECT { left: -500, top: 100, right: -400, bottom: 200 }),
        ("far off", RECT { left: 100000, top: 100000, right: 100100, bottom: 100100 }),
        ("empty inside", RECT { left: 100, top: 100, right: 100, bottom: 100 }),
        ("empty off", RECT { left: 5000, top: 100, right: 5000, bottom: 100 }),
        ("inverted inside", RECT { left: 200, top: 200, right: 100, bottom: 100 }),
        ("inverted off", RECT { left: 5100, top: 200, right: 5000, bottom: 100 }),
        ("touching the right edge", RECT { left: 1920, top: 0, right: 2000, bottom: 100 }),
        ("the last pixel", RECT { left: 1919, top: 1079, right: 1920, bottom: 1080 }),
        ("the whole coordinate range", RECT { left: i32::MIN, top: i32::MIN, right: i32::MAX, bottom: i32::MAX }),
    ];
    for (name, r) in cases {
        let mut row = Buf::<300>::new();
        for f in [MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY, MONITOR_DEFAULTTONEAREST] {
            set_error(0x1234);
            // SAFETY: r is a RECT.
            let m = unsafe { MonitorFromRect(&r, f) };
            let e = last_error();
            let _ = write!(
                row,
                " flag {}: {}{}",
                f,
                if m.is_null() {
                    "NULL"
                } else if m == dp_primary() {
                    "primary"
                } else {
                    "other"
                },
                if e == 0x1234 { "" } else { " (error changed)" }
            );
        }
        say!(t, "{name}:{}", row.as_str());
    }
}
