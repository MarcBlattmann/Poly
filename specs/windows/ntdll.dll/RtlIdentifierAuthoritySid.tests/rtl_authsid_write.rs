//! Writing through the pointer of RtlIdentifierAuthoritySid changes the SID itself.
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

/// ntdef.h: `UNICODE_STRING` (writable buffer).
#[repr(C)]
#[derive(Clone, Copy)]
struct RTL_UNICODE_STRING {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
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

/// The text of a counted UTF-16 string (ASCII only; others as `?`), by its Length field.
fn rtl_us_text(us: &RTL_UNICODE_STRING) -> Buf<200> {
    let mut b = Buf::<200>::new();
    if us.buffer.is_null() {
        let _ = write!(b, "(null)");
        return b;
    }
    for i in 0..usize::from(us.length / 2) {
        // SAFETY: the caller's buffer holds at least Length bytes.
        let c = unsafe { *us.buffer.add(i) };
        let _ = write!(b, "{}", if c < 0x80 { c as u8 as char } else { '?' });
    }
    b
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlLengthSid(sid: *const c_void) -> u32;
    fn RtlValidSid(sid: *const c_void) -> u8;
    fn RtlIdentifierAuthoritySid(sid: *const c_void) -> *const [u8; 6];
    fn RtlConvertSidToUnicodeString(dst: *mut RTL_UNICODE_STRING, sid: *const c_void, allocate: u8) -> i32;
    fn RtlFreeUnicodeString(s: *mut RTL_UNICODE_STRING);
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut sid = rtl_sid(5, &[]);
    // SAFETY: the pointer addresses the six authority bytes inside `sid`.
    unsafe {
        say!(t, "before: {}", Hex(&sid.b[..8]));
        let p = RtlIdentifierAuthoritySid(sid.as_psid()) as *mut [u8; 6];
        *p = [0, 0, 0, 0, 0, 16];
        say!(t, "after writing 00 00 00 00 00 10: {}", Hex(&sid.b[..8]));
        say!(t, "valid={} length={}", RtlValidSid(sid.as_const()), RtlLengthSid(sid.as_const()));
        let mut us = RTL_UNICODE_STRING { length: 0, maximum_length: 0, buffer: core::ptr::null_mut() };
        let st = RtlConvertSidToUnicodeString(&mut us, sid.as_const(), 1);
        say!(t, "text: status={:#010x} {}", st as u32, rtl_us_text(&us).as_str());
        RtlFreeUnicodeString(&mut us);
    }
}
