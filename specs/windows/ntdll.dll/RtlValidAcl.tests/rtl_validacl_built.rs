//! RtlValidAcl on ACLs built with the Rtl functions: valid after each step; the size field alone
//! (not the ACEs' sum) bounds the ACL; a full ACL is valid.
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
    fn RtlValidAcl(acl: *const c_void) -> u8;
    fn RtlAddAccessAllowedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> i32;
    fn RtlAddAccessDeniedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let everyone = rtl_sid(1, &[0]);
    let admins = rtl_sid(5, &[32, 544]);
    let mut acl = rtl_acl(8 + 20 + 24, 2);
    // SAFETY: valid ACL buffer and SIDs.
    unsafe {
        say!(t, "empty: {}", RtlValidAcl(acl.p()));
        let st = RtlAddAccessAllowedAce(acl.p(), 2, 1, everyone.as_const());
        say!(t, "after allowed ACE: status={:#010x} valid={}", st as u32, RtlValidAcl(acl.p()));
        let st = RtlAddAccessDeniedAce(acl.p(), 2, 2, admins.as_const());
        say!(t, "after denied ACE (ACL now full): status={:#010x} valid={}", st as u32, RtlValidAcl(acl.p()));
        let size = rtl_le16(&acl.b, 2);
        say!(t, "size field {size}");
        acl.b[2..4].copy_from_slice(&(size - 4).to_le_bytes());
        say!(t, "size field cut by 4 (second ACE crosses the end): {}", RtlValidAcl(acl.p()));
        acl.b[2..4].copy_from_slice(&(size - 24).to_le_bytes());
        say!(t, "size field cut by 24 (second ACE outside, count 2): {}", RtlValidAcl(acl.p()));
        acl.b[2..4].copy_from_slice(&(size + 100).to_le_bytes());
        say!(t, "size field larger than the ACEs: {}", RtlValidAcl(acl.p()));
    }
}
