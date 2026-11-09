//! RtlFreeSid: the value it returns after freeing, over 200 allocate/free cycles.
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
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut first = 0usize;
    let mut all_same = true;
    let mut nonzero = 0u32;
    for i in 0..200 {
        let mut sid: PSID = core::ptr::null_mut();
        // SAFETY: valid authority and out pointer; the SID is freed once.
        let r = unsafe {
            RtlAllocateAndInitializeSid(&SECURITY_NT_AUTHORITY, 2, 32, 544, 0, 0, 0, 0, 0, 0, &mut sid);
            RtlFreeSid(sid)
        };
        if i == 0 {
            first = r;
        }
        all_same &= r == first;
        nonzero += u32::from(r != 0);
    }
    say!(t, "first return value: {first:#x}");
    say!(t, "all 200 returns identical: {all_same}; nonzero: {nonzero}");
}
