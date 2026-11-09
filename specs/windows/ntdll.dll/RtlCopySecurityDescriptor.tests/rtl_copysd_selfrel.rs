//! RtlCopySecurityDescriptor of a self-relative descriptor, and of invalid ones: the status and what is
//! written to the out pointer.
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
type PSECURITY_DESCRIPTOR = *mut c_void;
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

/// A pointer stored in a result buffer, as an offset from the buffer start
/// (so printed values do not depend on addresses); -1 for null, -2 outside.
fn rel(base: *const u8, len: usize, p: *const c_void) -> i64 {
    if p.is_null() {
        return -1;
    }
    let d = (p as usize).wrapping_sub(base as usize);
    if d < len { d as i64 } else { -2 }
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

/// An absolute descriptor over the standard parts, set through the Rtl setters (not defaulted):
/// owner and group when asked; `d` and `s` for the DACL and SACL: 0 not set, 1 the ACL, 2 present
/// but NULL. The descriptor keeps pointers to the parts, which must stay where they are.
fn rtl_build_sd(p: &(SidBuf, SidBuf, AclBuf, AclBuf), o: bool, g: bool, d: u8, s: u8) -> Raw<256> {
    let mut sd = rtl_new_sd();
    let null: *const c_void = core::ptr::null();
    // SAFETY: the descriptor is initialized and the parts are valid for the calls.
    unsafe {
        if o {
            RtlSetOwnerSecurityDescriptor(sd.p(), p.0.as_const(), 0);
        }
        if g {
            RtlSetGroupSecurityDescriptor(sd.p(), p.1.as_const(), 0);
        }
        match d {
            1 => RtlSetDaclSecurityDescriptor(sd.p(), 1, p.2.b.as_ptr().cast(), 0),
            2 => RtlSetDaclSecurityDescriptor(sd.p(), 1, null, 0),
            _ => 0,
        };
        match s {
            1 => RtlSetSaclSecurityDescriptor(sd.p(), 1, p.3.b.as_ptr().cast(), 0),
            2 => RtlSetSaclSecurityDescriptor(sd.p(), 1, null, 0),
            _ => 0,
        };
    }
    sd
}

/// A self-relative copy of `abs` through RtlMakeSelfRelativeSD in a zeroed 512-byte buffer: the
/// required length is asked first, then the descriptor is made with exactly that length; (status, length).
fn rtl_make_rel(abs: &Raw<256>, out: &mut Raw<512>) -> (i32, u32) {
    let mut need = 0u32;
    // SAFETY: valid descriptor; a zero length asks for the size.
    unsafe { RtlMakeSelfRelativeSD(abs.b.as_ptr().cast(), out.p(), &mut need) };
    let mut len = need.min(512);
    // SAFETY: valid descriptor and a 512-byte buffer with its length.
    let st = unsafe { RtlMakeSelfRelativeSD(abs.b.as_ptr().cast(), out.p(), &mut len) };
    (st, len)
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlAddAce(acl: PACL, revision: u32, start: u32, list: *const c_void, len: u32) -> i32;
    fn RtlCreateSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, revision: u32) -> i32;
    fn RtlLengthSecurityDescriptor(sd: *const c_void) -> u32;
    fn RtlSetOwnerSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, owner: *const c_void, defaulted: u8) -> i32;
    fn RtlSetGroupSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, group: *const c_void, defaulted: u8) -> i32;
    fn RtlSetDaclSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, present: u8, dacl: *const c_void, defaulted: u8) -> i32;
    fn RtlSetSaclSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, present: u8, sacl: *const c_void, defaulted: u8) -> i32;
    fn RtlMakeSelfRelativeSD(abs: *const c_void, rel: PSECURITY_DESCRIPTOR, len: *mut u32) -> i32;
    fn RtlCopySecurityDescriptor(src: *const c_void, dst: *mut PSECURITY_DESCRIPTOR) -> i32;
}

poly_wtest::main!(test);

fn run(t: &mut Test, label: &str, input: *const c_void) {
    let mut out = 1usize as PSECURITY_DESCRIPTOR;
    // SAFETY: valid descriptor (possibly with a bad revision) and out pointer. The copy is not released.
    let st = unsafe { RtlCopySecurityDescriptor(input, &mut out) };
    let written = out as usize != 1;
    say!(t, "{label}: status={:#010x} written={written} out_null={}", st as u32, out.is_null());
    if written && !out.is_null() {
        // SAFETY: the copy is a descriptor of at least 20 bytes.
        let (head, len) =
            unsafe { (core::slice::from_raw_parts(out.cast::<u8>(), 20), RtlLengthSecurityDescriptor(out)) };
        say!(t, "{label}: length={len} header={}", Hex(&head[..4]));
        if head[3] & 0x80 != 0 {
            let n = (len as usize).min(512);
            // SAFETY: a self-relative copy and a self-relative input both hold `len` bytes.
            let (copy, orig) = unsafe {
                (core::slice::from_raw_parts(out.cast::<u8>(), n), core::slice::from_raw_parts(input.cast::<u8>(), n))
            };
            say!(t, "{label}: bytes equal to the input: {}", copy == orig);
        }
    }
}

fn test(t: &mut Test) {
    let parts = rtl_std_parts();
    let sd = rtl_build_sd(&parts, true, true, 1, 1);
    let mut rel = Raw::<512>::new();
    rtl_make_rel(&sd, &mut rel);
    run(t, "self-relative", rel.b.as_ptr().cast());
    let mut c = rel;
    c.b[0] = 2;
    run(t, "self-relative, revision 2", c.b.as_ptr().cast());
    let mut c = sd;
    c.b[0] = 0;
    run(t, "absolute, revision 0", c.b.as_ptr().cast());
    let mut e = Raw::<512>::new();
    let ed = rtl_new_sd();
    rtl_make_rel(&ed, &mut e);
    run(t, "empty self-relative", e.b.as_ptr().cast());
}
