//! GetDisplayConfigBufferSizes with every flag combination that is valid
//! (QDC_ALL_PATHS, QDC_ONLY_ACTIVE_PATHS, QDC_DATABASE_CURRENT, each with
//! QDC_VIRTUAL_MODE_AWARE, QDC_INCLUDE_HMD and QDC_VIRTUAL_REFRESH_RATE_AWARE
//! added) and with invalid ones (0, several of the three base flags, unknown
//! bits), and null pointers. Prints the return value (the error code itself,
//! the last error is not set), whether the counts are plausible, how many
//! modes there are per path, and what the outputs hold after a failure.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

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

const SM_CMONITORS: i32 = 80;
const QDC_ALL_PATHS: u32 = 0x1;
const QDC_ONLY_ACTIVE_PATHS: u32 = 0x2;
const QDC_DATABASE_CURRENT: u32 = 0x4;
const QDC_VIRTUAL_MODE_AWARE: u32 = 0x10;
const QDC_INCLUDE_HMD: u32 = 0x20;
const QDC_VIRTUAL_REFRESH_RATE_AWARE: u32 = 0x40;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetDisplayConfigBufferSizes(flags: u32, num_paths: *mut u32, num_modes: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: the out parameters are locals; the pointers passed as null are
    // part of what is observed.
    unsafe {
        let mut counts = [(0u32, 0u32); 4];
        let flags: [(&str, u32); 9] = [
            ("ONLY_ACTIVE_PATHS", QDC_ONLY_ACTIVE_PATHS),
            ("ALL_PATHS", QDC_ALL_PATHS),
            ("DATABASE_CURRENT", QDC_DATABASE_CURRENT),
            ("ONLY_ACTIVE_PATHS|VIRTUAL_MODE_AWARE", QDC_ONLY_ACTIVE_PATHS | QDC_VIRTUAL_MODE_AWARE),
            ("ALL_PATHS|VIRTUAL_MODE_AWARE", QDC_ALL_PATHS | QDC_VIRTUAL_MODE_AWARE),
            ("DATABASE_CURRENT|VIRTUAL_MODE_AWARE", QDC_DATABASE_CURRENT | QDC_VIRTUAL_MODE_AWARE),
            ("ONLY_ACTIVE_PATHS|VIRTUAL_MODE_AWARE|INCLUDE_HMD", QDC_ONLY_ACTIVE_PATHS | QDC_VIRTUAL_MODE_AWARE | QDC_INCLUDE_HMD),
            ("ONLY_ACTIVE_PATHS|VIRTUAL_MODE_AWARE|VIRTUAL_REFRESH_RATE_AWARE", QDC_ONLY_ACTIVE_PATHS | QDC_VIRTUAL_MODE_AWARE | QDC_VIRTUAL_REFRESH_RATE_AWARE),
            ("ONLY_ACTIVE_PATHS|INCLUDE_HMD", QDC_ONLY_ACTIVE_PATHS | QDC_INCLUDE_HMD),
        ];
        for (name, f) in flags {
            let mut np = 0xAAAAu32;
            let mut nm = 0xBBBBu32;
            set_error(0xDEAD);
            let r = GetDisplayConfigBufferSizes(f, &mut np, &mut nm);
            let per = if np > 0 && nm % np == 0 { nm / np } else { 0 };
            say!(t, "{name}: returns {r}, paths>=1 {}, modes per path {per}, last error {}", np >= 1, last_error());
            match f {
                QDC_ONLY_ACTIVE_PATHS => counts[0] = (np, nm),
                QDC_ALL_PATHS => counts[1] = (np, nm),
                QDC_DATABASE_CURRENT => counts[2] = (np, nm),
                _ => {}
            }
        }
        say!(t, "ALL_PATHS has at least the active paths: {}", counts[1].0 >= counts[0].0);
        say!(t, "DATABASE_CURRENT has the counts of ONLY_ACTIVE_PATHS: {}", counts[2] == counts[0]);
        say!(t, "ONLY_ACTIVE_PATHS equals the monitors of the desktop (SM_CMONITORS): {}", counts[0].0 as i32 == GetSystemMetrics(SM_CMONITORS));
        // the same answer twice
        let (mut a, mut b) = (0u32, 0u32);
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut a, &mut b);
        say!(t, "a second call gives the same counts: {}", (a, b) == counts[0]);

        // invalid flags: the error 87; the path count is set to 0, the mode count is left
        let bad: [(&str, u32); 8] = [
            ("0", 0),
            ("8", 8),
            ("0x80", 0x80),
            ("ALL_PATHS|ONLY_ACTIVE_PATHS", 3),
            ("ALL_PATHS|DATABASE_CURRENT", 5),
            ("ONLY_ACTIVE_PATHS|DATABASE_CURRENT", 6),
            ("all three", 7),
            ("VIRTUAL_MODE_AWARE alone", QDC_VIRTUAL_MODE_AWARE),
        ];
        for (name, f) in bad {
            let mut np = 0xAAAAu32;
            let mut nm = 0xBBBBu32;
            set_error(0xDEAD);
            let r = GetDisplayConfigBufferSizes(f, &mut np, &mut nm);
            say!(t, "flags {name}: returns {r}, paths set to 0: {}, modes left alone: {}, last error {}", np == 0, nm == 0xBBBB, last_error());
        }
        // null pointers are refused, not a crash
        let mut v = 5u32;
        set_error(0xDEAD);
        let r = GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, core::ptr::null_mut(), &mut v);
        say!(t, "null paths pointer: returns {r}, modes left alone: {}", v == 5);
        let r = GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut v, core::ptr::null_mut());
        say!(t, "null modes pointer: returns {r}");
        let r = GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, core::ptr::null_mut(), core::ptr::null_mut());
        say!(t, "both null: returns {r}, last error {}", last_error());
    }
}
