//! EqualSid: SIDs of different counts are different even when one is a prefix of the other;
//! bytes after the last sub-authority are not looked at.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type PSID = *mut c_void;
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
    fn InitializeSid(sid: PSID, authority: *const [u8; 6], count: u8) -> BOOL;
    fn GetSidSubAuthority(sid: *const c_void, n: u32) -> *mut u32;
    fn EqualSid(a: *const c_void, b: *const c_void) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut a = SidBuf::new();
    let mut b = SidBuf::new();
    // SAFETY: both buffers can hold 15 sub-authorities.
    unsafe {
        InitializeSid(a.as_psid(), &SECURITY_NT_AUTHORITY, 2);
        InitializeSid(b.as_psid(), &SECURITY_NT_AUTHORITY, 3);
        for i in 0..2 {
            *GetSidSubAuthority(a.as_const(), i) = 21 + i;
            *GetSidSubAuthority(b.as_const(), i) = 21 + i;
        }
        *GetSidSubAuthority(b.as_const(), 2) = 9;
        say!(t, "prefix: {} {}", EqualSid(a.as_const(), b.as_const()), EqualSid(b.as_const(), a.as_const()));
        b.b[1] = 2;
        say!(t, "same count, junk after: {}", EqualSid(a.as_const(), b.as_const()));
        a.b[40..44].copy_from_slice(&[1, 2, 3, 4]);
        say!(t, "different junk far after: {}", EqualSid(a.as_const(), b.as_const()));
        let c = SidBuf::new();
        let mut d = SidBuf::new();
        InitializeSid(d.as_psid(), &SECURITY_NT_AUTHORITY, 0);
        let mut e = SidBuf::new();
        InitializeSid(e.as_psid(), &SECURITY_NT_AUTHORITY, 0);
        say!(t, "two empty: {}", EqualSid(d.as_const(), e.as_const()));
        let _ = c;
    }
}
