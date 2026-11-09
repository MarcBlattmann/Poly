//! LookupPrivilegeNameW buffer protocol: on success the length excludes the NUL; when the buffer
//! is too small it fails with ERROR_INSUFFICIENT_BUFFER and the length includes the NUL.
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
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// winnt.h: `LUID`.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct LUID {
    low: u32,
    high: i32,
}

/// The last-error code after a failed call, as a plain number.
fn gle() -> u32 {
    // SAFETY: reads the calling thread's last error.
    unsafe { GetLastError() }
}

fn clear_error() {
    // SAFETY: sets the calling thread's last error.
    unsafe { SetLastError(0xDEAD) };
}

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LookupPrivilegeNameW(system: *const u16, luid: *const LUID, name: *mut u16, len: *mut u32) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let luid = LUID { low: 23, high: 0 };
    for size in [0u32, 1, 10, 21, 22, 23, 24, 25, 100] {
        let mut name = [0xFFFFu16; 128];
        let mut len = size;
        clear_error();
        // SAFETY: valid LUID; the buffer is 128 units, `size` at most 100.
        let ok = unsafe { LookupPrivilegeNameW(core::ptr::null(), &luid, name.as_mut_ptr(), &mut len) };
        let first = name[0];
        say!(t, "size {size}: ok={ok} err={} len={len} first_unit={first:#x}", gle_if(ok));
    }
}
