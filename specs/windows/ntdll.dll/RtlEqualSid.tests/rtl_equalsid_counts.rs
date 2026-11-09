//! RtlEqualSid: SIDs of different counts differ even when one is a prefix of the other; bytes after
//! the last sub-authority are not looked at.
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

const SECURITY_MAX_SID_SIZE: usize = 68;
/// A `SID_IDENTIFIER_AUTHORITY` plus up to 15 sub-authorities in a buffer.
#[repr(C)]
#[derive(Clone, Copy)]
struct SidBuf {
    b: [u8; SECURITY_MAX_SID_SIZE + 4],
}

impl SidBuf {
    const fn new() -> SidBuf {
        SidBuf { b: [0; SECURITY_MAX_SID_SIZE + 4] }
    }
    fn as_psid(&mut self) -> PSID {
        self.b.as_mut_ptr().cast()
    }
    fn as_const(&self) -> *const c_void {
        self.b.as_ptr().cast()
    }
    /// The bytes of the SID (its length from the header).
    fn bytes(&self) -> &[u8] {
        let n = 8 + 4 * usize::from(self.b[1]);
        &self.b[..n.min(self.b.len())]
    }
}

/// A SID with authority {0,0,0,0,0,auth} and the given sub-authorities, built byte by byte.
fn rtl_sid(auth: u8, subs: &[u32]) -> SidBuf {
    let mut s = SidBuf::new();
    s.b[0] = 1;
    s.b[1] = subs.len().min(15) as u8;
    s.b[7] = auth;
    for (i, v) in subs.iter().enumerate().take(15) {
        s.b[8 + 4 * i..12 + 4 * i].copy_from_slice(&v.to_le_bytes());
    }
    s
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlEqualSid(a: *const c_void, b: *const c_void) -> u8;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let a = rtl_sid(5, &[21, 22]);
    let mut b = rtl_sid(5, &[21, 22, 9]);
    // SAFETY: valid SIDs in 72-byte buffers.
    unsafe {
        say!(t, "prefix: {} {}", RtlEqualSid(a.as_const(), b.as_const()), RtlEqualSid(b.as_const(), a.as_const()));
        b.b[1] = 2;
        say!(t, "same count, junk after: {}", RtlEqualSid(a.as_const(), b.as_const()));
        let mut a2 = a;
        a2.b[40..44].copy_from_slice(&[1, 2, 3, 4]);
        say!(t, "different junk far after: {}", RtlEqualSid(a2.as_const(), b.as_const()));
        let d = rtl_sid(5, &[]);
        let e = rtl_sid(5, &[]);
        say!(t, "two empty: {}", RtlEqualSid(d.as_const(), e.as_const()));
        let f = rtl_sid(1, &[]);
        say!(t, "empty, other authority: {}", RtlEqualSid(d.as_const(), f.as_const()));
    }
}
