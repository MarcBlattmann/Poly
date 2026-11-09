//! After SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2) as the first
//! call, threads created by the process start with that awareness, also those
//! created by a thread whose own context was changed; a thread may change its
//! own context afterwards; the process-level call from another thread than the
//! first fails with ERROR_ACCESS_DENIED.
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

type BOOL = i32;
type THREADPROC = unsafe extern "system" fn(*mut c_void) -> u32;
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

const DPI_UNAWARE: isize = -1;
const DPI_SYSTEM_AWARE: isize = -2;
const DPI_PER_MONITOR_AWARE: isize = -3;
const DPI_PER_MONITOR_AWARE_V2: isize = -4;
const DPI_UNAWARE_GDISCALED: isize = -5;
/// A tiny helper: the name of an awareness context handle as the tests print
/// it (the pseudo handles by name, anything else as "other").
fn dp_ctx(c: isize) -> &'static str {
    match c {
        0 => "NULL",
        -1 => "UNAWARE",
        -2 => "SYSTEM_AWARE",
        -3 => "PER_MONITOR_AWARE",
        -4 => "PER_MONITOR_AWARE_V2",
        -5 => "UNAWARE_GDISCALED",
        _ => "other",
    }
}

/// Which pseudo handle a context equals (AreDpiAwarenessContextsEqual), or
/// "none".
fn dp_which(c: isize) -> &'static str {
    // SAFETY: plain queries on handle values.
    unsafe {
        for p in [DPI_UNAWARE, DPI_SYSTEM_AWARE, DPI_PER_MONITOR_AWARE, DPI_PER_MONITOR_AWARE_V2, DPI_UNAWARE_GDISCALED]
        {
            if AreDpiAwarenessContextsEqual(c, p) != 0 {
                return dp_ctx(p);
            }
        }
    }
    "none"
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CreateThread(
        attributes: *mut c_void,
        stack: usize,
        start: THREADPROC,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn WaitForSingleObject(handle: HANDLE, ms: u32) -> u32;
    fn CloseHandle(handle: HANDLE) -> BOOL;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetProcessDpiAwarenessContext(ctx: isize) -> BOOL;
    fn GetThreadDpiAwarenessContext() -> isize;
    fn SetThreadDpiAwarenessContext(ctx: isize) -> isize;
    fn AreDpiAwarenessContextsEqual(a: isize, b: isize) -> BOOL;
}

poly_wtest::main!(test);

static P_LINES: Global<[u8; 2000]> = Global::new([0; 2000]);

unsafe extern "system" fn p_thread(_p: *mut c_void) -> u32 {
    // SAFETY: plain calls.
    unsafe {
        logln!("thread starts with {}", dp_which(GetThreadDpiAwarenessContext()));
        let prev = SetThreadDpiAwarenessContext(DPI_UNAWARE);
        logln!("  SetThreadDpiAwarenessContext(UNAWARE) returns {}", dp_which(prev));
        set_error(0x1234);
        let r = SetProcessDpiAwarenessContext(DPI_SYSTEM_AWARE);
        logln!("  SetProcessDpiAwarenessContext(SYSTEM_AWARE) from the thread -> {} error {}", r, last_error());
        logln!("  thread context now {}", dp_which(GetThreadDpiAwarenessContext()));
    }
    0
}

fn test(t: &mut Test) {
    // SAFETY: plain calls; the thread is joined.
    unsafe {
        let r = SetProcessDpiAwarenessContext(DPI_PER_MONITOR_AWARE_V2);
        say!(t, "SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2) -> {}", r);
        SetThreadDpiAwarenessContext(DPI_SYSTEM_AWARE);
        say!(t, "main thread context after changing it: {}", dp_which(GetThreadDpiAwarenessContext()));
        let th = CreateThread(core::ptr::null_mut(), 0, p_thread, core::ptr::null_mut(), 0, core::ptr::null_mut());
        WaitForSingleObject(th, 5000);
        CloseHandle(th);
        flush_log(t, "  ");
        say!(t, "main thread context after the thread: {}", dp_which(GetThreadDpiAwarenessContext()));
    }
}
