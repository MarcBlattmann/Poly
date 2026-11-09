//! RtlAddMandatoryAce with different label SIDs (RIDs, sub-authority counts, other authorities), and the same
//! SID as built by CreateWellKnownSid and RtlAllocateAndInitializeSid.
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
type BOOL = i32;
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

const SECURITY_MANDATORY_LABEL_AUTHORITY: [u8; 6] = [0, 0, 0, 0, 0, 16];
const SECURITY_MAX_SID_SIZE: usize = 68;
const WinLowLabelSid: u32 = 66;
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

/// `ConvertSidToStringSidW` of a well-known SID built by `CreateWellKnownSid`.
fn well_known(kind: u32) -> SidBuf {
    let mut sid = SidBuf::new();
    let mut len = SECURITY_MAX_SID_SIZE as u32;
    // SAFETY: `sid` has room for the largest SID; `len` is its size.
    unsafe { CreateWellKnownSid(kind, core::ptr::null(), sid.as_psid(), &mut len) };
    sid
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

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateWellKnownSid(kind: u32, domain: *const c_void, sid: PSID, len: *mut u32) -> BOOL;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
}

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
    fn RtlCopySid(len: u32, dst: PSID, src: *const c_void) -> i32;
    fn RtlCreateAcl(acl: PACL, len: u32, revision: u32) -> i32;
    fn RtlAddMandatoryAce(acl: PACL, revision: u32, flags: u32, sid: *const c_void, ace_type: u32, mask: u32) -> i32;
}

poly_wtest::main!(test);

fn add(t: &mut Test, label: &str, sid: *const c_void) {
    let mut acl = rtl_acl(64, 4);
    // SAFETY: valid ACL buffer; the SID is a buffer of at least 72 bytes.
    let st = unsafe { RtlAddMandatoryAce(acl.p(), 4, 0, sid, 0x11, 1) };
    say!(t, "{label}: status={:#010x} count={} ace={}", st as u32, rtl_le16(&acl.b, 4), Hex(&acl.b[8..24]));
}

fn test(t: &mut Test) {
    for rid in [0u32, 0x1000, 0x2000, 0x2100, 0x3000, 0x4000, 0x5000, 0x1234, 0xFFFF_FFFF] {
        let s = rtl_sid(16, &[rid]);
        let mut l = Buf::<60>::new();
        let _ = write!(l, "label RID {rid:#x}");
        add(t, l.as_str(), s.as_const());
    }
    let wk = well_known(WinLowLabelSid);
    add(t, "CreateWellKnownSid(WinLowLabelSid)", wk.as_const());
    let mut alloc: PSID = core::ptr::null_mut();
    // SAFETY: valid authority and out pointer; the SID is released below.
    unsafe {
        RtlAllocateAndInitializeSid(&SECURITY_MANDATORY_LABEL_AUTHORITY, 1, 0x2000, 0, 0, 0, 0, 0, 0, 0, &mut alloc);
    }
    let mut copy = SidBuf::new();
    // SAFETY: `alloc` is a valid SID of 12 bytes; the copy is a 72-byte buffer.
    unsafe {
        RtlCopySid(72, copy.as_psid(), alloc);
        RtlFreeSid(alloc);
    }
    add(t, "RtlAllocateAndInitializeSid label 0x2000", copy.as_const());
    let mut aligned = Raw::<72>::new();
    aligned.b[..12].copy_from_slice(&copy.b[..12]);
    add(t, "the same SID in an 8-aligned buffer", aligned.b.as_ptr().cast());
    let two = rtl_sid(16, &[0x1000, 0x2000]);
    add(t, "label with two sub-authorities", two.as_const());
    let none = rtl_sid(16, &[]);
    add(t, "label authority, no sub-authority", none.as_const());
    for auth in [0u8, 1, 5, 15, 17] {
        let s = rtl_sid(auth, &[0x2000]);
        let mut l = Buf::<60>::new();
        let _ = write!(l, "authority {auth}, RID 0x2000");
        add(t, l.as_str(), s.as_const());
    }
}
