//! RtlSetSaclSecurityDescriptor: the Present argument sets or clears SE_SACL_PRESENT, the ACL pointer is stored
//! (no copy), a NULL ACL with Present TRUE is a NULL SACL, and the Defaulted argument sets SE_SACL_DEFAULTED.
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
type PSECURITY_DESCRIPTOR = *mut c_void;
const FALSE: BOOL = 0;
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

/// An ACE (header, mask, SID) for `RtlAddAce` lists and raw ACLs; its size.
fn rtl_ace(ty: u8, flags: u8, mask: u32, sid: &SidBuf) -> (Raw<128>, u32) {
    let mut r = Raw::<128>::new();
    let sl = 8 + 4 * usize::from(sid.b[1].min(15));
    let n = 8 + sl;
    r.b[0] = ty;
    r.b[1] = flags;
    r.b[2..4].copy_from_slice(&(n as u16).to_le_bytes());
    r.b[4..8].copy_from_slice(&mask.to_le_bytes());
    r.b[8..8 + sl].copy_from_slice(&sid.b[..sl]);
    (r, n as u32)
}

/// An ACL written byte by byte: header (revision, size field, count) followed by the ACEs.
fn rtl_acl_raw(rev: u8, size: usize, aces: &[(Raw<128>, u32)]) -> AclBuf {
    let mut a = AclBuf::new();
    a.b[0] = rev;
    a.b[2..4].copy_from_slice(&(size as u16).to_le_bytes());
    a.b[4..6].copy_from_slice(&(aces.len() as u16).to_le_bytes());
    let mut off = 8;
    for (r, n) in aces {
        let n = *n as usize;
        a.b[off..off + n].copy_from_slice(&r.b[..n]);
        off += n;
    }
    a
}

/// Where a pointer points: "null", "name+offset" inside one of the named buffers, or "other".
fn rtl_where(p: *const c_void, bases: &[(&str, *const u8, usize)]) -> Buf<48> {
    let mut b = Buf::<48>::new();
    if p.is_null() {
        let _ = write!(b, "null");
        return b;
    }
    for (name, base, len) in bases {
        let d = (p as usize).wrapping_sub(*base as usize);
        if d < *len {
            let _ = write!(b, "{name}+{d}");
            return b;
        }
    }
    let _ = write!(b, "other");
    b
}

/// A new absolute security descriptor (RtlCreateSecurityDescriptor, revision 1) in a zeroed buffer.
fn rtl_new_sd() -> Raw<256> {
    let mut sd = Raw::<256>::new();
    // SAFETY: the buffer has room for the 40-byte descriptor.
    unsafe { RtlCreateSecurityDescriptor(sd.p(), 1) };
    sd
}

/// The standard parts of the SD tests: owner S-1-5-32-544, group S-1-5-32-545, a DACL of
/// revision 2 and size 64 with one access-allowed ACE for Everyone (20 bytes), a SACL of
/// revision 2 and size 28 with one audit ACE for Everyone (20 bytes).
fn rtl_std_parts() -> (SidBuf, SidBuf, AclBuf, AclBuf) {
    let owner = rtl_sid(5, &[32, 544]);
    let group = rtl_sid(5, &[32, 545]);
    let everyone = rtl_sid(1, &[0]);
    let dacl = rtl_acl_raw(2, 64, &[rtl_ace(0, 0, 0x001F_01FF, &everyone)]);
    let sacl = rtl_acl_raw(2, 28, &[rtl_ace(2, 0xC0, 0x1, &everyone)]);
    (owner, group, dacl, sacl)
}

/// The pointer stored at `off` of an absolute descriptor.
fn rtl_sd_ptr(sd: &Raw<256>, off: usize) -> *const c_void {
    sd.u64_at(off) as usize as *const c_void
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlAddAce(acl: PACL, revision: u32, start: u32, list: *const c_void, len: u32) -> i32;
    fn RtlCreateSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, revision: u32) -> i32;
    fn RtlSetSaclSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, present: u8, sacl: *const c_void, defaulted: u8) -> i32;
}

poly_wtest::main!(test);

fn show(t: &mut Test, label: &str, st: i32, sd: &Raw<256>, bases: &[(&str, *const u8, usize)]) {
    say!(
        t,
        "{label}: status={:#010x} control={:#06x} sacl={}",
        st as u32,
        sd.u16_at(2),
        rtl_where(rtl_sd_ptr(sd, 24), bases).as_str()
    );
}

fn test(t: &mut Test) {
    let parts = rtl_std_parts();
    let acl = parts.3;
    let bases = [("acl", acl.b.as_ptr(), acl.b.len())];
    let null: *const c_void = core::ptr::null();
    let cases: [(u8, bool, u8); 12] = [
        (1, true, 0),
        (1, true, 1),
        (0, true, 0),
        (0, true, 1),
        (1, false, 0),
        (0, false, 0),
        (1, false, 1),
        (0, false, 1),
        (2, true, 0),
        (0xFF, true, 0),
        (1, true, 2),
        (1, true, 0xFF),
    ];
    for (present, with_acl, dflt) in cases {
        let mut sd = rtl_new_sd();
        let p = if with_acl { acl.b.as_ptr().cast::<c_void>() } else { null };
        // SAFETY: valid descriptor and ACL.
        let st = unsafe { RtlSetSaclSecurityDescriptor(sd.p(), present, p, dflt) };
        let mut l = Buf::<100>::new();
        let _ = write!(l, "present {present:#x} acl {with_acl} defaulted {dflt:#x}");
        show(t, l.as_str(), st, &sd, &bases);
    }
    let mut sd = rtl_new_sd();
    // SAFETY: valid descriptor and ACL.
    unsafe {
        let st = RtlSetSaclSecurityDescriptor(sd.p(), 1, acl.b.as_ptr().cast(), 1);
        show(t, "sequence: set, defaulted", st, &sd, &bases);
        let st = RtlSetSaclSecurityDescriptor(sd.p(), 0, acl.b.as_ptr().cast(), 0);
        show(t, "sequence: Present FALSE with an ACL (clears bits, keeps pointer?)", st, &sd, &bases);
        let st = RtlSetSaclSecurityDescriptor(sd.p(), 1, null, 0);
        show(t, "sequence: NULL ACL present", st, &sd, &bases);
    }
    let mut sd = rtl_new_sd();
    sd.b[2..4].copy_from_slice(&0x7FF3u16.to_le_bytes());
    // SAFETY: valid descriptor and ACL.
    let st = unsafe { RtlSetSaclSecurityDescriptor(sd.p(), 1, acl.b.as_ptr().cast(), 0) };
    show(t, "other control bits 0x7ff3 kept", st, &sd, &bases);
}
