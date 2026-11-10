//! RtlAddMandatoryAce (arguments: ACL, revision, AceFlags, SID, AceType, AccessMask; the mask is the mandatory
//! policy of AddMandatoryAce): the SYSTEM_MANDATORY_LABEL_ACE it appends and how mask and flags are stored.
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

/// Print an ACL: its header, then every ACE as hex (walking by the ACE sizes, bounded by the ACL size).
fn rtl_acl_dump(t: &mut Test, label: &str, acl: &AclBuf) {
    let b = &acl.b;
    let size = usize::from(rtl_le16(b, 2)).min(b.len());
    let count = usize::from(rtl_le16(b, 4));
    say!(t, "{label}: header={} size={size} count={count}", Hex(&b[..8]));
    let mut off = 8;
    for i in 0..count.min(20) {
        if off + 4 > size {
            say!(t, "{label}: ace {i} beyond the acl");
            break;
        }
        let sz = usize::from(rtl_le16(b, off + 2));
        if sz < 4 || off + sz > size {
            say!(t, "{label}: ace {i} at {off} has size {sz}");
            break;
        }
        say!(t, "{label}: ace {i} at {off}: {}", Hex(&b[off..off + sz]));
        off += sz;
    }
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlCreateAcl(acl: PACL, len: u32, revision: u32) -> i32;
    fn RtlAddAccessAllowedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> i32;
    fn RtlAddAccessDeniedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> i32;
    fn RtlAddMandatoryAce(acl: PACL, revision: u32, flags: u32, sid: *const c_void, ace_type: u32, mask: u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let label = rtl_sid(16, &[0x2000]);
    let mut acl = rtl_acl(64, 4);
    // SAFETY: valid ACL buffer and SID.
    let st = unsafe { RtlAddMandatoryAce(acl.p(), 4, 0, label.as_const(), 0x11, 1) };
    say!(t, "status={:#010x}", st as u32);
    rtl_acl_dump(t, "label", &acl);
    for mask in [0u32, 1, 2, 3, 4, 5, 7, 8, 0xF, 0x10, 0x100, 0xFFFF_FFFF] {
        let mut acl = rtl_acl(64, 4);
        // SAFETY: valid ACL buffer and SID.
        let st = unsafe { RtlAddMandatoryAce(acl.p(), 4, 0, label.as_const(), 0x11, mask) };
        say!(t, "mask {mask:#x}: status={:#010x} mask_field={}", st as u32, Hex(&acl.b[12..16]));
    }
    for flags in [0u32, 1, 2, 4, 8, 0x10, 0x20, 0x40, 0x80, 0xFF, 0x100] {
        let mut acl = rtl_acl(64, 4);
        // SAFETY: valid ACL buffer and SID.
        let st = unsafe { RtlAddMandatoryAce(acl.p(), 4, flags, label.as_const(), 0x11, 1) };
        say!(t, "flags {flags:#x}: status={:#010x} header={}", st as u32, Hex(&acl.b[8..12]));
    }
    let mut acl = rtl_acl(256, 4);
    let everyone = rtl_sid(1, &[0]);
    // SAFETY: valid ACL buffer and SIDs.
    unsafe {
        RtlAddAccessAllowedAce(acl.p(), 4, 0x1F01FF, everyone.as_const());
        RtlAddMandatoryAce(acl.p(), 4, 0, label.as_const(), 0x11, 1);
        RtlAddAccessDeniedAce(acl.p(), 4, 2, everyone.as_const());
    }
    rtl_acl_dump(t, "appended after and before other ACEs", &acl);
}
