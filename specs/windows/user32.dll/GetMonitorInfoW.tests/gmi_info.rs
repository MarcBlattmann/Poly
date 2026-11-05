//! GetMonitorInfoW of the primary monitor with a MONITORINFOEXW (cbSize 104)
//! and a MONITORINFO (cbSize 40, the first 40 bytes; winuser.h): the monitor
//! rectangle, the work area (the screen less the taskbar), the flags
//! (MONITORINFOF_PRIMARY 1) and the device name; the bytes past cbSize are
//! left alone; cbSize is not changed. The work area agrees with
//! SystemParametersInfoW(SPI_GETWORKAREA).
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

/// The UTF-16 text in `buf` up to its first NUL, as ASCII (others as '?').
fn ascii_of(buf: &[u16], out: &mut Buf<300>) {
    for &c in buf {
        if c == 0 {
            break;
        }
        let _ = out.write_char(if (0x20..0x7f).contains(&c) { c as u8 as char } else { '?' });
    }
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
/// "l,t,r,b" of a rectangle.
struct R(RECT);

impl core::fmt::Display for R {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{},{},{},{}", self.0.left, self.0.top, self.0.right, self.0.bottom)
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
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SystemParametersInfoW(action: u32, param: u32, pv: *mut c_void, ini: u32) -> BOOL;
}
/// winuser.h
const GMI_SPI_GETWORKAREA: u32 = 0x0030;

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain query.
    let m = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let mut ex: MONITORINFOEXW = zeroed();
    ex.cb_size = 104;
    for c in ex.device.iter_mut() {
        *c = 0xAAAA;
    }
    set_error(0);
    // SAFETY: ex is a MONITORINFOEXW of the size given.
    let ok = unsafe { GetMonitorInfoW(m, &mut ex) };
    say!(t, "MONITORINFOEXW: ok={} error {} cbSize {}", ok, last_error(), ex.cb_size);
    say!(t, "  monitor {} work {} flags {:#x}", R(ex.monitor), R(ex.work), ex.flags);
    let mut name = Buf::<300>::new();
    ascii_of(&ex.device, &mut name);
    let len = ex.device.iter().position(|&c| c == 0).unwrap_or(32);
    say!(t, "  device \"{}\" length {} after the NUL: {:#x}", name.as_str(), len, ex.device[(len + 1).min(31)]);
    let mut b: MONITORINFOEXW = zeroed();
    b.cb_size = 40;
    for c in b.device.iter_mut() {
        *c = 0xAAAA;
    }
    // SAFETY: b is larger than the 40 bytes given.
    let ok = unsafe { GetMonitorInfoW(m, &mut b) };
    say!(
        t,
        "MONITORINFO: ok={} cbSize {} monitor {} work {} flags {:#x}",
        ok,
        b.cb_size,
        R(b.monitor),
        R(b.work),
        b.flags
    );
    t.check("bytes past cbSize 40 untouched", b.device.iter().all(|&c| c == 0xAAAA), b.device[0] as i64);
    let mut wa = RECT::default();
    // SAFETY: wa is a writable RECT.
    unsafe { SystemParametersInfoW(GMI_SPI_GETWORKAREA, 0, &mut wa as *mut RECT as *mut c_void, 0) };
    t.check("work area == SPI_GETWORKAREA", wa == ex.work, wa.bottom as i64);
    // SAFETY: plain queries.
    unsafe {
        t.check(
            "monitor == the screen",
            ex.monitor
                == RECT {
                    left: 0,
                    top: 0,
                    right: GetSystemMetrics(SM_CXSCREEN),
                    bottom: GetSystemMetrics(SM_CYSCREEN),
                },
            0,
        );
    }
}
