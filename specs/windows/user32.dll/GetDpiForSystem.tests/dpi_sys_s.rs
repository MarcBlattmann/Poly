//! GetDpiForSystem, GetSystemDpiForProcess, GetDpiForWindow of the desktop, LOGPIXELSX and the
//! monitor DPI asked from a thread of every awareness context, on a display scaled to 200 percent;
//! the program is a system DPI aware process (SetProcessDpiAwarenessContext(SYSTEM_AWARE)).
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
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
}

const LOGPIXELSX: i32 = 88;
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
const CTX_UNAWARE: isize = -1;
const CTX_SYSTEM: isize = -2;
const CTX_PM: isize = -3;
const CTX_PMV2: isize = -4;
const CTX_GDI: isize = -5;
type HANDLE = *mut c_void;
/// The name of the pseudo context a context equals ("other" when none).
fn ctx_name(c: isize) -> &'static str {
    // SAFETY: plain comparisons of context values.
    unsafe {
        if AreDpiAwarenessContextsEqual(c, CTX_UNAWARE) != 0 {
            "unaware"
        } else if AreDpiAwarenessContextsEqual(c, CTX_SYSTEM) != 0 {
            "system"
        } else if AreDpiAwarenessContextsEqual(c, CTX_PM) != 0 {
            "per-monitor"
        } else if AreDpiAwarenessContextsEqual(c, CTX_PMV2) != 0 {
            "per-monitor-v2"
        } else if AreDpiAwarenessContextsEqual(c, CTX_GDI) != 0 {
            "unaware-gdi-scaled"
        } else {
            "other"
        }
    }
}

fn thread_ctx_name() -> &'static str {
    // SAFETY: plain query.
    ctx_name(unsafe { GetThreadDpiAwarenessContext() })
}

fn pseudo(name: &str) -> isize {
    match name {
        "unaware" => CTX_UNAWARE,
        "system" => CTX_SYSTEM,
        "pm" => CTX_PM,
        "pmv2" => CTX_PMV2,
        _ => CTX_GDI,
    }
}

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

/// GetDpiForSystem and its relatives from threads with every context.
fn sec_sys(t: &mut Test) {
    // SAFETY: plain calls; the thread's context is put back.
    unsafe {
        let desk = GetDesktopWindow();
        let dc = GetDC(core::ptr::null_mut());
        let start = GetThreadDpiAwarenessContext();
        for (n, c) in [("unaware", CTX_UNAWARE), ("system", CTX_SYSTEM), ("per-monitor", CTX_PM), ("per-monitor-v2", CTX_PMV2), ("gdi-scaled", CTX_GDI)] {
            let old = SetThreadDpiAwarenessContext(c);
            say!(
                t,
                "thread {n} (was {}): GetDpiForSystem {} GetSystemDpiForProcess {} GetDpiForWindow(desktop) {} LOGPIXELSX {} GetDpiForMonitor(primary) {}",
                ctx_name(old),
                GetDpiForSystem(),
                GetSystemDpiForProcess(GetCurrentProcess()),
                GetDpiForWindow(desk),
                GetDeviceCaps(dc, LOGPIXELSX),
                monitor_dpi_x()
            );
        }
        SetThreadDpiAwarenessContext(start);
        ReleaseDC(core::ptr::null_mut(), dc);
        say!(t, "back on the starting context: {}", thread_ctx_name());
    }
}

fn primary() -> HMONITOR {
    // SAFETY: plain query.
    unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
}

fn monitor_dpi_x() -> u32 {
    let (mut x, mut y) = (0u32, 0u32);
    // SAFETY: writable outputs.
    unsafe { GetDpiForMonitor(primary(), 0, &mut x, &mut y) };
    x
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcess() -> HANDLE;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDesktopWindow() -> HWND;
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    fn MonitorFromPoint(pt: POINT, flags: u32) -> HMONITOR;
    fn GetDpiForWindow(hwnd: HWND) -> u32;
    fn GetDpiForSystem() -> u32;
    fn SetProcessDpiAwarenessContext(ctx: isize) -> BOOL;
    fn GetThreadDpiAwarenessContext() -> isize;
    fn SetThreadDpiAwarenessContext(ctx: isize) -> isize;
    fn AreDpiAwarenessContextsEqual(a: isize, b: isize) -> BOOL;
    fn GetSystemDpiForProcess(process: HANDLE) -> u32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDeviceCaps(hdc: HDC, index: i32) -> i32;
}

#[link(name = "shcore", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDpiForMonitor(monitor: HMONITOR, kind: i32, x: *mut u32, y: *mut u32) -> i32;
}

const MODE: u8 = b's';

poly_wtest::main!(test);

fn test(t: &mut Test) {
    enter(t, MODE);
    sec_sys(t);
}
