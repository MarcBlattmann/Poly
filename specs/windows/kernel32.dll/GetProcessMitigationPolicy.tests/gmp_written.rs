//! GetProcessMitigationPolicy: how much of the buffer each policy writes. The buffer starts filled with 0x5A;
//! after a call with the policy's own length the bytes past that length are compared with the fill, and after
//! a failing call the whole buffer. The policy values themselves are not printed.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winnt.h, winbase.h, sddl.h, aclapi.h, jobapi2.h,
// processthreadsapi.h, winternl.h, ntstatus.h and winerror.h (constants,
// layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type PVOID = *mut c_void;
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

/// An 8-aligned scratch buffer for token information and other results.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct Raw<const N: usize> {
    b: [u8; N],
}

impl<const N: usize> Raw<N> {
    const fn new() -> Raw<N> {
        Raw { b: [0; N] }
    }
    fn p(&mut self) -> PVOID {
        self.b.as_mut_ptr().cast()
    }
    fn u32_at(&self, off: usize) -> u32 {
        u32::from_le_bytes([self.b[off], self.b[off + 1], self.b[off + 2], self.b[off + 3]])
    }
    fn u16_at(&self, off: usize) -> u16 {
        u16::from_le_bytes([self.b[off], self.b[off + 1]])
    }
    fn u64_at(&self, off: usize) -> u64 {
        let mut a = [0u8; 8];
        a.copy_from_slice(&self.b[off..off + 8]);
        u64::from_le_bytes(a)
    }
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
    fn GetCurrentProcess() -> HANDLE;
    fn GetProcessMitigationPolicy(process: HANDLE, policy: u32, buffer: PVOID, len: usize) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: the pseudo-handle of the current process.
    let me = unsafe { GetCurrentProcess() };
    for class in 0..=20u32 {
        let own = if class == 0 { 8usize } else { 4 };
        if class == 5 {
            continue;
        }
        let mut raw = Raw::<64>::new();
        raw.b = [0x5A; 64];
        clear_error();
        // SAFETY: a 64-byte buffer.
        let ok = unsafe { GetProcessMitigationPolicy(me, class, raw.p(), own) };
        let good = (ok, gle_if(ok), raw.b[own..].iter().all(|&b| b == 0x5A));
        let mut raw = Raw::<64>::new();
        raw.b = [0x5A; 64];
        clear_error();
        // SAFETY: a 64-byte buffer.
        let ok = unsafe { GetProcessMitigationPolicy(me, class, raw.p(), own + 4) };
        say!(
            t,
            "class {class}: length {own}: ok={} err={} bytes past the length untouched={}; length {}: ok={ok} err={} buffer untouched={}",
            good.0,
            good.1,
            good.2,
            own + 4,
            gle_if(ok),
            raw.b.iter().all(|&b| b == 0x5A)
        );
    }
    // The options mask: every length from 0 to 40, and which of them are accepted.
    let mut line = Buf::<300>::new();
    for len in 0..=40usize {
        let mut raw = Raw::<64>::new();
        // SAFETY: a 64-byte buffer.
        let ok = unsafe { GetProcessMitigationPolicy(me, 5, raw.p(), len) };
        if ok != 0 {
            let _ = write!(line, "{len} ");
        }
    }
    say!(t, "class 5: lengths 0..40 accepted: {}", line.as_str());
}
