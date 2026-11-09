//! RtlCopySid copies exactly RtlLengthSid bytes; the copy equals the source and nothing past it is touched.
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
    fn RtlLengthSid(sid: *const c_void) -> u32;
    fn RtlValidSid(sid: *const c_void) -> u8;
    fn RtlEqualSid(a: *const c_void, b: *const c_void) -> u8;
    fn RtlCopySid(len: u32, dst: PSID, src: *const c_void) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let sids = [
        rtl_sid(5, &[18]),
        rtl_sid(5, &[32, 544]),
        rtl_sid(5, &[21, 1, 2, 3, 4]),
        rtl_sid(5, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]),
        rtl_sid(1, &[0]),
        rtl_sid(5, &[]),
    ];
    for src in &sids {
        let mut dst = SidBuf::new();
        dst.b.fill(0xEE);
        // SAFETY: `src` is a valid SID; `dst` has room for the largest SID.
        unsafe {
            let n = RtlLengthSid(src.as_const());
            let st = RtlCopySid(SECURITY_MAX_SID_SIZE as u32, dst.as_psid(), src.as_const());
            say!(
                t,
                "length={n} status={:#010x} equal={} valid={} beyond_untouched={}",
                st as u32,
                RtlEqualSid(src.as_const(), dst.as_const()),
                RtlValidSid(dst.as_const()),
                dst.b[n as usize..].iter().all(|&b| b == 0xEE)
            );
        }
    }
}
