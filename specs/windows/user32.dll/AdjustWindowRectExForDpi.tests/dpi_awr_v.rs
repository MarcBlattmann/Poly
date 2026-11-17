//! AdjustWindowRectEx and AdjustWindowRectExForDpi for a 400x300 client of several styles, on a
//! display scaled to 200 percent; the program is a per-monitor aware v2 process
//! (SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)).
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
const WS_POPUP: u32 = 0x8000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
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

impl R {
    fn to_string_eq(&self, o: &R) -> bool {
        self.0.left == o.0.left && self.0.top == o.0.top && self.0.right == o.0.right && self.0.bottom == o.0.bottom
    }
}

/// AdjustWindowRectEx and AdjustWindowRectExForDpi for a client of 400x300.
fn sec_awr(t: &mut Test) {
    // SAFETY: plain calls with writable rectangles.
    unsafe {
        let styles = [
            ("overlapped window", WS_OVERLAPPEDWINDOW, 0u32, 0),
            ("overlapped window with a menu", WS_OVERLAPPEDWINDOW, 0, 1),
            ("caption only", WS_CAPTION, 0, 0),
            ("thick frame popup", WS_POPUP | WS_THICKFRAME, 0, 0),
            ("border popup", WS_POPUP | WS_BORDER, 0, 0),
            ("tool window", WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU, WS_EX_TOOLWINDOW, 0),
        ];
        for (n, s, ex, menu) in styles {
            let mut r = RECT { left: 0, top: 0, right: 400, bottom: 300 };
            let ok = AdjustWindowRectEx(&mut r, s, menu, ex);
            say!(t, "AdjustWindowRectEx({n}) -> {} {}", ok != 0, R(r));
            let mut b = Buf::<300>::new();
            let _ = write!(b, "  ForDpi:");
            for dpi in [96u32, 120, 144, 168, 192, 288] {
                let mut r = RECT { left: 0, top: 0, right: 400, bottom: 300 };
                AdjustWindowRectExForDpi(&mut r, s, menu, ex, dpi);
                let _ = write!(b, " {dpi}: {}", R(r));
            }
            t.line(b.as_str());
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
    fn AdjustWindowRectEx(rect: *mut RECT, style: u32, menu: BOOL, ex_style: u32) -> BOOL;
    fn SetProcessDpiAwarenessContext(ctx: isize) -> BOOL;
    fn AdjustWindowRectExForDpi(rect: *mut RECT, style: u32, menu: BOOL, ex: u32, dpi: u32) -> BOOL;
}

const MODE: u8 = b'v';

poly_wtest::main!(test);

fn test(t: &mut Test) {
    enter(t, MODE);
    sec_awr(t);
}
