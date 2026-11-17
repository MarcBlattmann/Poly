//! MonitorFromPoint at the edges of the screen in the caller's coordinates, on a display scaled to
//! 200 percent; the program is a process that never sets its awareness (it starts DPI unaware).
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

const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;
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
const CTX_UNAWARE: isize = -1;
const CTX_SYSTEM: isize = -2;
const CTX_PM: isize = -3;
const CTX_PMV2: isize = -4;
const CTX_GDI: isize = -5;
/// The label of a probe's mode letter: u none, U unaware, s system, p per-monitor v1, v per-monitor v2,
/// g unaware GDI scaled (a letter in capitals M: the awareness comes from the manifest).
fn mode_label(m: u8) -> &'static str {
    match m {
        b'u' => "no call",
        b'U' => "unaware",
        b's' => "system",
        b'p' => "per-monitor",
        b'v' => "per-monitor-v2",
        b'g' => "unaware-gdi-scaled",
        _ => "manifest",
    }
}

/// Puts the process into the probe's mode with SetProcessDpiAwarenessContext (modes u and M: nothing).
fn enter(t: &mut Test, m: u8) {
    let ctx = match m {
        b'U' => CTX_UNAWARE,
        b's' => CTX_SYSTEM,
        b'p' => CTX_PM,
        b'v' => CTX_PMV2,
        b'g' => CTX_GDI,
        _ => 0,
    };
    if ctx == 0 {
        say!(t, "mode {}: no SetProcessDpiAwarenessContext call", mode_label(m));
        return;
    }
    set_error(0);
    // SAFETY: plain call.
    let r = unsafe { SetProcessDpiAwarenessContext(ctx) };
    say!(t, "mode {}: SetProcessDpiAwarenessContext -> {} error {}", mode_label(m), r != 0, last_error());
}

fn primary() -> HMONITOR {
    // SAFETY: plain query.
    unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
}

/// MonitorFromPoint at the corners and edges of the screen in the caller's coordinates.
fn sec_mfp(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe {
        let prim = primary();
        let (cx, cy) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        say!(t, "screen {}x{}", cx, cy);
        let pts = [(0, 0), (cx / 2, cy / 2), (cx - 1, cy - 1), (cx, cy), (cx + 100, cy + 100), (-1, -1), (cx * 2 - 1, cy * 2 - 1), (cx * 2, cy * 2)];
        for (x, y) in pts {
            let a = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL);
            let b = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
            say!(t, "({x},{y}): null-flag {} nearest-flag primary {}", if a.is_null() { "null" } else if a == prim { "primary" } else { "other" }, b == prim);
        }
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn MonitorFromPoint(pt: POINT, flags: u32) -> HMONITOR;
    fn SetProcessDpiAwarenessContext(ctx: isize) -> BOOL;
}

const MODE: u8 = b'u';

poly_wtest::main!(test);

fn test(t: &mut Test) {
    enter(t, MODE);
    sec_mfp(t);
}
