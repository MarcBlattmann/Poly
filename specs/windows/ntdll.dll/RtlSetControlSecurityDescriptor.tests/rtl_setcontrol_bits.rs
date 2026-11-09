//! RtlSetControlSecurityDescriptor(sd, BitsOfInterest, BitsToSet), one control bit at a time: which bits
//! can be set or cleared, and the status for the others.
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
type PSECURITY_DESCRIPTOR = *mut c_void;
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

/// A new absolute security descriptor (RtlCreateSecurityDescriptor, revision 1) in a zeroed buffer.
fn rtl_new_sd() -> Raw<256> {
    let mut sd = Raw::<256>::new();
    // SAFETY: the buffer has room for the 40-byte descriptor.
    unsafe { RtlCreateSecurityDescriptor(sd.p(), 1) };
    sd
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlCreateSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, revision: u32) -> i32;
    fn RtlSetControlSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, interest: u16, set: u16) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for bit in 0..16u32 {
        let m = 1u16 << bit;
        let mut a = rtl_new_sd();
        // SAFETY: valid descriptor.
        let sa = unsafe { RtlSetControlSecurityDescriptor(a.p(), m, m) };
        let mut b = rtl_new_sd();
        b.b[2..4].copy_from_slice(&0xFFFFu16.to_le_bytes());
        // SAFETY: valid descriptor (all pointers NULL).
        let sb = unsafe { RtlSetControlSecurityDescriptor(b.p(), m, 0) };
        say!(
            t,
            "bit {m:#06x}: set on empty: status={:#010x} control={:#06x}; clear on 0xffff: status={:#010x} control={:#06x}",
            sa as u32,
            a.u16_at(2),
            sb as u32,
            b.u16_at(2)
        );
    }
}
