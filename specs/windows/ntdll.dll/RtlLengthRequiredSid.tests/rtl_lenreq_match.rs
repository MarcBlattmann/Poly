//! RtlLengthRequiredSid agrees with RtlLengthSid of a SID initialized with that count, and with
//! the Win32 GetSidLengthRequired for every byte count.
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

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSidLengthRequired(count: u8) -> u32;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlInitializeSid(sid: PSID, authority: *const [u8; 6], count: u8) -> i32;
    fn RtlLengthRequiredSid(count: u32) -> u32;
    fn RtlLengthSid(sid: *const c_void) -> u32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut agree = 0;
    for count in 0..=15u8 {
        let mut sid = SidBuf::new();
        // SAFETY: `sid` can hold 15 sub-authorities.
        unsafe {
            RtlInitializeSid(sid.as_psid(), &SECURITY_NT_AUTHORITY, count);
            if RtlLengthRequiredSid(u32::from(count)) == RtlLengthSid(sid.as_const()) {
                agree += 1;
            }
        }
    }
    say!(t, "agree with RtlLengthSid for {agree} of 16 counts");
    let mut win32 = 0;
    for count in 0..=255u8 {
        // SAFETY: pure computations.
        unsafe {
            if RtlLengthRequiredSid(u32::from(count)) == GetSidLengthRequired(count) {
                win32 += 1;
            }
        }
    }
    say!(t, "agree with GetSidLengthRequired for {win32} of 256 counts");
}
