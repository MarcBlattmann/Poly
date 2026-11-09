//! RtlAllocateAndInitializeSid: sub-authority counts 0 to 8 give valid SIDs of 8 + 4 * count bytes
//! (arguments past the count are ignored); larger counts fail with an NTSTATUS and leave the out pointer alone.
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

/// The bytes as lowercase hex, no separators.
struct Hex<'a>(&'a [u8]);

impl core::fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

const SECURITY_NT_AUTHORITY: [u8; 6] = [0, 0, 0, 0, 0, 5];

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlAllocateAndInitializeSid(
        authority: *const [u8; 6],
        count: u8,
        s0: u32,
        s1: u32,
        s2: u32,
        s3: u32,
        s4: u32,
        s5: u32,
        s6: u32,
        s7: u32,
        sid: *mut PSID,
    ) -> i32;
    fn RtlFreeSid(sid: PSID) -> usize;
    fn RtlLengthSid(sid: *const c_void) -> u32;
    fn RtlValidSid(sid: *const c_void) -> u8;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let auth = SECURITY_NT_AUTHORITY;
    for count in [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 15, 16, 255] {
        let mut sid: PSID = core::ptr::null_mut();
        // SAFETY: valid authority and out pointer; a returned SID is released below.
        let st = unsafe { RtlAllocateAndInitializeSid(&auth, count, 11, 22, 33, 44, 55, 66, 77, 88, &mut sid) };
        say!(t, "count {count}: status={:#010x} sid_null={}", st as u32, sid.is_null());
        if !sid.is_null() {
            // SAFETY: `sid` was allocated by the call above.
            unsafe {
                let n = RtlLengthSid(sid);
                let bytes = core::slice::from_raw_parts(sid.cast::<u8>(), n as usize);
                say!(t, "count {count}: valid={} length={n} bytes={}", RtlValidSid(sid), Hex(bytes));
                RtlFreeSid(sid);
            }
        }
    }
}
