//! RtlAddAccessAllowedAceEx with flags 0 gives the same ACE as RtlAddAccessAllowedAce; ACEs of both appended in order.
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
    fn RtlAddAccessAllowedAceEx(acl: PACL, revision: u32, flags: u32, mask: u32, sid: *const c_void) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let admins = rtl_sid(5, &[32, 544]);
    let mut a = rtl_acl(64, 2);
    let mut b = rtl_acl(64, 2);
    // SAFETY: valid ACL buffers and SID.
    unsafe {
        RtlAddAccessAllowedAce(a.p(), 2, 0x20019, admins.as_const());
        RtlAddAccessAllowedAceEx(b.p(), 2, 0, 0x20019, admins.as_const());
    }
    say!(t, "plain: {}", Hex(a.bytes()));
    say!(t, "Ex:    {}", Hex(b.bytes()));
    say!(t, "identical: {}", a.bytes() == b.bytes());
    // SAFETY: valid ACL buffers and SID.
    unsafe {
        RtlAddAccessAllowedAceEx(a.p(), 2, 0x3, 5, admins.as_const());
        RtlAddAccessAllowedAce(a.p(), 2, 6, admins.as_const());
    }
    rtl_acl_dump(t, "three", &a);
}
