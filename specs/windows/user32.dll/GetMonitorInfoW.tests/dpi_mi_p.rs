//! GetMonitorInfoW, EnumDisplayMonitors, the virtual screen metrics and the work area, on a display
//! scaled to 200 percent; the program is a per-monitor aware (v1) process
//! (SetProcessDpiAwarenessContext(PER_MONITOR_AWARE)).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HDC = *mut c_void;
type LPARAM = isize;
type BOOL = i32;
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

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
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
/// winuser.h
#[repr(C)]
struct MONITORINFOEXW {
    cb_size: u32,
    monitor: RECT,
    work: RECT,
    flags: u32,
    device: [u16; 32],
}

const MONITOR_DEFAULTTOPRIMARY: u32 = 1;
const SM_XVIRTUALSCREEN: i32 = 76;
const SM_YVIRTUALSCREEN: i32 = 77;
const SM_CXVIRTUALSCREEN: i32 = 78;
const SM_CYVIRTUALSCREEN: i32 = 79;
const SM_CMONITORS: i32 = 80;
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
const SPI_GETWORKAREA: u32 = 0x30;
type HANDLE = *mut c_void;
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

unsafe extern "system" fn count_monitor(m: HANDLE, _dc: HDC, r: *mut RECT, data: LPARAM) -> BOOL {
    // SAFETY: data points to our counter; r to the monitor rectangle.
    unsafe {
        let n = &mut *(data as *mut [i32; 5]);
        n[0] += 1;
        let r = *r;
        n[1] = r.left;
        n[2] = r.top;
        n[3] = r.right;
        n[4] = r.bottom;
        let _ = m;
    }
    1
}

/// GetMonitorInfoW, EnumDisplayMonitors and the virtual screen.
fn sec_mi(t: &mut Test) {
    // SAFETY: plain calls with writable outputs.
    unsafe {
        let m = primary();
        let mut mi: MONITORINFOEXW = zeroed();
        mi.cb_size = core::mem::size_of::<MONITORINFOEXW>() as u32;
        set_error(0);
        let r = GetMonitorInfoW(m, &mut mi);
        say!(t, "GetMonitorInfoW -> {} error {} monitor {} flags {}", r != 0, last_error(), R(mi.monitor), mi.flags);
        let wa = mi.work;
        let mo = mi.monitor;
        say!(t, "work area {}: left {} top {} right_gap {} bottom_gap {}", R(wa), wa.left - mo.left, wa.top - mo.top, mo.right - wa.right, mo.bottom - wa.bottom);
        let mut sp = RECT::default();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut sp as *mut RECT as *mut c_void, 0);
        say!(t, "SPI_GETWORKAREA equals the work area {}", R(sp).to_string_eq(&R(wa)));
        let mut n = [0i32; 5];
        let r = EnumDisplayMonitors(core::ptr::null_mut(), core::ptr::null(), Some(count_monitor), &mut n as *mut [i32; 5] as LPARAM);
        say!(t, "EnumDisplayMonitors -> {} count {} rect {},{},{},{}", r != 0, n[0], n[1], n[2], n[3], n[4]);
        say!(
            t,
            "virtual screen {},{} {}x{} monitors {}",
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
            GetSystemMetrics(SM_CMONITORS)
        );
        say!(t, "SM_CXSCREEN x SM_CYSCREEN {}x{}", GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
    }
}

impl R {
    fn to_string_eq(&self, o: &R) -> bool {
        self.0.left == o.0.left && self.0.top == o.0.top && self.0.right == o.0.right && self.0.bottom == o.0.bottom
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
    fn GetMonitorInfoW(monitor: HMONITOR, info: *mut MONITORINFOEXW) -> BOOL;
    fn SetProcessDpiAwarenessContext(ctx: isize) -> BOOL;
    fn SystemParametersInfoW(action: u32, param: u32, pv: *mut c_void, ini: u32) -> BOOL;
    fn EnumDisplayMonitors(
        hdc: HDC,
        clip: *const RECT,
        cb: Option<unsafe extern "system" fn(HANDLE, HDC, *mut RECT, LPARAM) -> BOOL>,
        data: LPARAM,
    ) -> BOOL;
}

const MODE: u8 = b'p';

poly_wtest::main!(test);

fn test(t: &mut Test) {
    enter(t, MODE);
    sec_mi(t);
}
