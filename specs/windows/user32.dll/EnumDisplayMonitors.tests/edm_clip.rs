//! EnumDisplayMonitors with a NULL device context and no clip: one callback
//! with the primary monitor, a NULL DC and the monitor rectangle 0,0,1920,1080;
//! with a clip rectangle the callback rectangle is the intersection (inside:
//! unchanged; partly off: cut; spanning: the monitor); no callback when the clip
//! does not intersect the monitor (off the screen, empty, inverted, touching
//! the right edge); a clip on the last pixel gives 1919,1079,1920,1080; the
//! function returns TRUE in all these cases with the last error unchanged.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::cell::UnsafeCell;
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

const LOGPIXELSX: i32 = 88;
/// A global the window procedures and the test share (one thread at a
/// time unless a test says otherwise).
struct Global<T>(UnsafeCell<T>);

unsafe impl<T> Sync for Global<T> {}
impl<T> Global<T> {
    const fn new(v: T) -> Self {
        Global(UnsafeCell::new(v))
    }
    #[allow(clippy::mut_from_ref)]
    fn get(&self) -> &mut T {
        // SAFETY: see the Sync impl: no two live references at once.
        unsafe { &mut *self.0.get() }
    }
}

/// The message log: lines appended by window procedures, printed by the test.
struct Log {
    buf: [u8; 32768],
    len: usize,
    on: bool,
}

static LOG: Global<Log> = Global::new(Log { buf: [0; 32768], len: 0, on: true });
impl core::fmt::Write for Log {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            if self.len < self.buf.len() {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
        Ok(())
    }
}

/// Append one line to the log (when logging is on).
fn logf(args: core::fmt::Arguments) {
    let log = LOG.get();
    if log.on {
        let _ = log.write_fmt(args);
        let _ = log.write_str("\n");
    }
}

macro_rules! logln {
    ($($a:tt)*) => { logf(format_args!($($a)*)) };
}

/// Print every logged line (prefixed) and empty the log.
fn flush_log(t: &mut Test, prefix: &str) {
    let log = LOG.get();
    let mut start = 0;
    for i in 0..log.len {
        if log.buf[i] == b'\n' {
            let line = core::str::from_utf8(&log.buf[start..i]).unwrap_or("?");
            let mut b = Buf::<300>::new();
            let _ = write!(b, "{prefix}{line}");
            t.line(b.as_str());
            start = i + 1;
        }
    }
    log.len = 0;
}

fn log_clear() {
    LOG.get().len = 0;
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
/// "l,t,r,b" of a rectangle.
struct R(RECT);

impl core::fmt::Display for R {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{},{},{},{}", self.0.left, self.0.top, self.0.right, self.0.bottom)
    }
}

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
    fn GetMonitorInfoW(monitor: HMONITOR, info: *mut MONITORINFOEXW) -> BOOL;
    fn EnumDisplayMonitors(
        hdc: HDC,
        clip: *const RECT,
        proc_: unsafe extern "system" fn(HMONITOR, HDC, *mut RECT, LPARAM) -> BOOL,
        data: LPARAM,
    ) -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetClipBox(hdc: HDC, rect: *mut RECT) -> i32;
    fn GetDeviceCaps(hdc: HDC, index: i32) -> i32;
}

poly_wtest::main!(test);

static E_N: Global<u32> = Global::new(0);
static E_STOP: Global<u32> = Global::new(0);

unsafe extern "system" fn e_cb(mon: HMONITOR, dc: HDC, r: *mut RECT, data: LPARAM) -> BOOL {
    *E_N.get() += 1;
    let n = *E_N.get();
    // SAFETY: r is the RECT the system gives.
    let rect = unsafe { *r };
    let mut mi: MONITORINFOEXW = zeroed();
    mi.cb_size = 104;
    // SAFETY: mi is valid.
    let ok = unsafe { GetMonitorInfoW(mon, &mut mi) };
    logln!(
        "  callback {}: monitor is primary {} dc NULL {} rect {} data {:#x} GetMonitorInfoW {} monitor {}",
        n,
        mon == dp_primary(),
        dc.is_null(),
        R(rect),
        data,
        ok,
        R(mi.monitor)
    );
    if !dc.is_null() {
        let mut cb = RECT::default();
        // SAFETY: the DC is valid inside the callback.
        let kind = unsafe { GetClipBox(dc, &mut cb) };
        logln!("    dc clip box kind {} {} LOGPIXELSX {}", kind, R(cb), unsafe { GetDeviceCaps(dc, LOGPIXELSX) });
    }
    if *E_STOP.get() == n { 0 } else { 1 }
}

fn run(t: &mut Test, label: &str, dc: HDC, clip: Option<RECT>, stop_at: u32) {
    *E_N.get() = 0;
    *E_STOP.get() = stop_at;
    log_clear();
    let cp = match &clip {
        Some(r) => r as *const RECT,
        None => core::ptr::null(),
    };
    set_error(0x1234);
    // SAFETY: the callback is valid; clip is NULL or a RECT.
    let r = unsafe { EnumDisplayMonitors(dc, cp, e_cb, 0x5A5A) };
    say!(t, "{label}: returns {} last error {:#x} callbacks {}", r, last_error(), *E_N.get());
    flush_log(t, "");
}

fn test(t: &mut Test) {
    let n = core::ptr::null_mut();
    run(t, "no clip", n, None, 0);
    run(t, "clip inside", n, Some(RECT { left: 100, top: 100, right: 300, bottom: 300 }), 0);
    run(t, "clip partly off the screen", n, Some(RECT { left: -100, top: -100, right: 300, bottom: 300 }), 0);
    run(t, "clip spanning the screen", n, Some(RECT { left: -5000, top: -5000, right: 5000, bottom: 5000 }), 0);
    run(t, "clip off the screen", n, Some(RECT { left: 3000, top: 100, right: 3200, bottom: 300 }), 0);
    run(t, "clip empty", n, Some(RECT { left: 100, top: 100, right: 100, bottom: 100 }), 0);
    run(t, "clip inverted", n, Some(RECT { left: 300, top: 300, right: 100, bottom: 100 }), 0);
    run(t, "clip touching the right edge", n, Some(RECT { left: 1920, top: 0, right: 2000, bottom: 100 }), 0);
    run(t, "clip on the last pixel", n, Some(RECT { left: 1919, top: 1079, right: 2000, bottom: 2000 }), 0);
    run(t, "clip on the first pixel", n, Some(RECT { left: -10, top: -10, right: 1, bottom: 1 }), 0);
}
