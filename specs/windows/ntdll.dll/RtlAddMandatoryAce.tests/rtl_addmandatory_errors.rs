//! RtlAddMandatoryAce errors: ACL and ACE revision, the ACE type argument, a SID that is not a label SID, an
//! invalid SID, no room.
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

fn test(t: &mut Test) {
    let label = rtl_sid(16, &[0x1000]);
    for acl_rev in [2u32, 3, 4] {
        let mut line = Buf::<300>::new();
        for rev in [0u32, 1, 2, 3, 4, 5] {
            let mut acl = rtl_acl(64, acl_rev);
            // SAFETY: valid ACL buffer and SID.
            let st = unsafe { RtlAddMandatoryAce(acl.p(), rev, 0, label.as_const(), 0x11, 1) };
            let _ = write!(line, "{rev}:{:#x}/rev{}/n{} ", st as u32, acl.b[0], rtl_le16(&acl.b, 4));
        }
        say!(t, "ACL revision {acl_rev}, revision argument: {}", line.as_str());
    }
    let mut line = Buf::<300>::new();
    for ty in [0u32, 1, 2, 0x10, 0x11, 0x12, 0x13, 0xFF, 0x111, 0x1_0011] {
        let mut acl = rtl_acl(64, 4);
        // SAFETY: valid ACL buffer and SID.
        let st = unsafe { RtlAddMandatoryAce(acl.p(), 4, 0, label.as_const(), ty, 1) };
        let _ = write!(line, "{ty:#x}:{:#x}/type{} ", st as u32, acl.b[8]);
    }
    say!(t, "ACE type argument: {}", line.as_str());
    let others = [
        ("system S-1-5-18", rtl_sid(5, &[18])),
        ("everyone", rtl_sid(1, &[0])),
        ("label with two sub-authorities", rtl_sid(16, &[1, 2])),
        ("label authority, no sub-authority", rtl_sid(16, &[])),
    ];
    for (name, sid) in &others {
        let mut acl = rtl_acl(64, 4);
        // SAFETY: valid ACL buffer and SID.
        let st = unsafe { RtlAddMandatoryAce(acl.p(), 4, 0, sid.as_const(), 0x11, 1) };
        say!(t, "{name}: status={:#010x}", st as u32);
    }
    let mut big = Raw::<1100>::new();
    big.b[0] = 1;
    big.b[1] = 16;
    big.b[7] = 16;
    let mut acl = rtl_acl(64, 4);
    // SAFETY: valid ACL buffer; the SID buffer is larger than any SID.
    let st = unsafe { RtlAddMandatoryAce(acl.p(), 4, 0, big.p(), 0x11, 1) };
    say!(t, "SID count 16: status={:#010x}", st as u32);
    for len in [27u32, 28] {
        let mut acl = rtl_acl(len, 4);
        // SAFETY: valid ACL buffer and SID.
        let st = unsafe { RtlAddMandatoryAce(acl.p(), 4, 0, label.as_const(), 0x11, 1) };
        say!(t, "ACL length {len}: status={:#010x}", st as u32);
    }
}
