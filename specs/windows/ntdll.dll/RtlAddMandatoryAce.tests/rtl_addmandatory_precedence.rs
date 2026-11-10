//! RtlAddMandatoryAce with two things wrong at once: which status wins. Each fault alone, then every pair.
//! Faults: T a type other than 0x11, S an invalid SID (16 sub-authorities), A a SID that is not a label,
//! M a mask of 8, F flags 0x20, R revision argument 5, N an ACL with one byte too little room, L an ACL whose
//! revision byte is 9, K an ACL whose revision byte is 1.
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
type PACL = *mut c_void;
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

/// An ACL in an 8-aligned buffer.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct AclBuf {
    b: [u8; 1024],
}

impl AclBuf {
    const fn new() -> AclBuf {
        AclBuf { b: [0; 1024] }
    }
    fn p(&mut self) -> PACL {
        self.b.as_mut_ptr().cast()
    }
    /// The ACL's bytes by its own size field.
    fn bytes(&self) -> &[u8] {
        let n = usize::from(u16::from_le_bytes([self.b[2], self.b[3]]));
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

/// The little-endian u16 at `off`.
fn rtl_le16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

/// An ACL made by `RtlCreateAcl` in a zeroed buffer.
fn rtl_acl(len: u32, rev: u32) -> AclBuf {
    let mut a = AclBuf::new();
    // SAFETY: the buffer has 1024 bytes; callers pass lengths up to 1024.
    unsafe { RtlCreateAcl(a.p(), len, rev) };
    a
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlCreateAcl(acl: PACL, len: u32, revision: u32) -> i32;
    fn RtlAddMandatoryAce(acl: PACL, revision: u32, flags: u32, sid: *const c_void, ace_type: u32, mask: u32) -> i32;
}

poly_wtest::main!(test);

const NAMES: [&str; 9] = ["T", "S", "A", "M", "F", "R", "N", "L", "K"];

fn call(faults: u32) -> (u32, u16) {
    let has = |i: u32| faults & (1 << i) != 0;
    let mut sid = if has(2) { rtl_sid(5, &[18]) } else { rtl_sid(16, &[0x1000]) };
    if has(1) {
        sid.b[1] = 16;
    }
    let mut acl = rtl_acl(if has(6) { 27 } else { 64 }, 4);
    if has(7) {
        acl.b[0] = 9;
    }
    if has(8) {
        acl.b[0] = 1;
    }
    let ty = if has(0) { 0x12 } else { 0x11 };
    let mask = if has(3) { 8 } else { 1 };
    let flags = if has(4) { 0x20 } else { 0 };
    let rev = if has(5) { 5 } else { 4 };
    // SAFETY: a valid ACL buffer of 1024 bytes and a SID buffer that holds 16 sub-authorities.
    let st = unsafe { RtlAddMandatoryAce(acl.p(), rev, flags, sid.as_const(), ty, mask) };
    (st as u32, rtl_le16(&acl.b, 4))
}

fn test(t: &mut Test) {
    let (st, n) = call(0);
    say!(t, "nothing wrong: status={st:#010x} count={n}");
    for i in 0..9u32 {
        let (st, n) = call(1 << i);
        say!(t, "{} alone: status={st:#010x} count={n}", NAMES[i as usize]);
    }
    for i in 0..8u32 {
        let mut line = Buf::<300>::new();
        for j in i + 1..9u32 {
            let (st, _) = call(1 << i | 1 << j);
            let _ = write!(line, "{}={st:#010x} ", NAMES[j as usize]);
        }
        say!(t, "{} together with: {}", NAMES[i as usize], line.as_str());
    }
}
