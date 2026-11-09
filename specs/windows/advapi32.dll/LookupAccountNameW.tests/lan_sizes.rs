//! LookupAccountNameW buffer protocol: SID buffer or domain buffer too small (required sizes),
//! zero sizes, null buffers, and what an unknown name reports.
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

type PSID = *mut c_void;
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

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 255 units).
fn wide(s: &str) -> [u16; 256] {
    let mut w = [0u16; 256];
    for (i, c) in s.bytes().enumerate().take(255) {
        w[i] = u16::from(c);
    }
    w
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LookupAccountNameW(
        system: *const u16,
        name: *const u16,
        sid: PSID,
        cb_sid: *mut u32,
        domain: *mut u16,
        cch_domain: *mut u32,
        kind: *mut u32,
    ) -> BOOL;
}

poly_wtest::main!(test);

fn run(t: &mut Test, label: &str, name: &str, cs_in: u32, cd_in: u32, null_sid: bool, null_dom: bool) {
    let w = wide(name);
    let mut sid = Raw::<128>::new();
    let mut dom = [0xAAAAu16; 64];
    let (mut cs, mut cd, mut kind) = (cs_in, cd_in, 0xDEADu32);
    clear_error();
    // SAFETY: the buffers are large enough for the sizes passed.
    let ok = unsafe {
        LookupAccountNameW(
            core::ptr::null(),
            w.as_ptr(),
            if null_sid { core::ptr::null_mut() } else { sid.p() },
            &mut cs,
            if null_dom { core::ptr::null_mut() } else { dom.as_mut_ptr() },
            &mut cd,
            &mut kind,
        )
    };
    let err = if ok == 0 { i64::from(gle()) } else { -1 };
    say!(
        t,
        "{label}: ok={ok} err={err} cb_sid={cs} cch_domain={cd} use={kind:#x} sid0={:#x} dom0={:#x}",
        sid.b[0],
        dom[0]
    );
}

fn test(t: &mut Test) {
    run(t, "SYSTEM exact (12 + 12 + NUL)", "NT AUTHORITY\\SYSTEM", 12, 13, false, false);
    run(t, "SID buffer one too small", "NT AUTHORITY\\SYSTEM", 11, 13, false, false);
    run(t, "domain buffer one too small", "NT AUTHORITY\\SYSTEM", 12, 12, false, false);
    run(t, "both zero", "NT AUTHORITY\\SYSTEM", 0, 0, false, false);
    run(t, "both zero, null buffers", "NT AUTHORITY\\SYSTEM", 0, 0, true, true);
    run(t, "Everyone: empty domain", "Everyone", 64, 64, false, false);
    run(t, "Everyone: domain size zero", "Everyone", 64, 0, false, false);
    run(t, "unknown name", "Nobody At All Poly", 64, 64, false, false);
    run(t, "unknown name, zero sizes", "Nobody At All Poly", 0, 0, false, false);
    run(t, "unknown domain prefix", "NoSuchDomainPoly\\SYSTEM", 64, 64, false, false);
}
