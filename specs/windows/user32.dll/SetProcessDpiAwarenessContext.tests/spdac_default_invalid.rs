//! SetProcessDpiAwarenessContext in a process without a manifest: the
//! awareness is unaware at the start; NULL, 1, -6 and unknown handle values
//! fail with ERROR_INVALID_PARAMETER and change nothing; changes of the
//! thread's context do not count as setting the process's; the first valid
//! call after the failures still succeeds, a second one fails with
//! ERROR_ACCESS_DENIED.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
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
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDpiForSystem() -> u32;
    fn SetProcessDpiAwarenessContext(ctx: isize) -> BOOL;
    fn GetThreadDpiAwarenessContext() -> isize;
    fn SetThreadDpiAwarenessContext(ctx: isize) -> isize;
    fn AreDpiAwarenessContextsEqual(a: isize, b: isize) -> BOOL;
    fn IsProcessDPIAware() -> BOOL;
}

#[link(name = "shcore", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetProcessDpiAwareness(process: HANDLE, out: *mut i32) -> i32;
}

poly_wtest::main!(test);

fn state(t: &mut Test, label: &str) {
    // SAFETY: plain queries.
    unsafe {
        let th = GetThreadDpiAwarenessContext();
        let mut a = -99;
        let hr = GetProcessDpiAwareness(core::ptr::null_mut(), &mut a);
        say!(
            t,
            "[{label}] thread {} GetProcessDpiAwareness {:#x} {} IsProcessDPIAware {} GetDpiForSystem {}",
            dp_which(th),
            hr,
            a,
            IsProcessDPIAware(),
            GetDpiForSystem()
        );
    }
}

fn test(t: &mut Test) {
    state(t, "start");
    // SAFETY: plain calls.
    unsafe {
        for c in [0isize, 1, -6, -7, -100, 16, 0x1234, 0x6012] {
            set_error(0x1234);
            let r = SetProcessDpiAwarenessContext(c);
            say!(t, "SetProcessDpiAwarenessContext({:#x}) -> {} error {}", c, r, last_error());
        }
        state(t, "after the failures");
        SetThreadDpiAwarenessContext(DPI_PER_MONITOR_AWARE_V2);
        SetThreadDpiAwarenessContext(DPI_UNAWARE);
        state(t, "after thread changes");
        set_error(0x1234);
        let r = SetProcessDpiAwarenessContext(DPI_PER_MONITOR_AWARE_V2);
        say!(t, "first valid call (PER_MONITOR_AWARE_V2) -> {} error {}", r, last_error());
        state(t, "after it");
        set_error(0x1234);
        let r = SetProcessDpiAwarenessContext(DPI_PER_MONITOR_AWARE_V2);
        say!(t, "same value again -> {} error {}", r, last_error());
    }
}
