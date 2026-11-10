//! RtlAbsoluteToSelfRelativeSD gives the same self-relative descriptor as RtlMakeSelfRelativeSD; the
//! offsets and length for several combinations of parts.
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

/// Print `data` as hex lines of 48 bytes: "label[offset]: hex".
fn rtl_hex_lines(t: &mut Test, label: &str, data: &[u8]) {
    let mut off = 0;
    while off < data.len() {
        let end = (off + 48).min(data.len());
        say!(t, "{label}[{off}]: {}", Hex(&data[off..end]));
        off = end;
    }
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
    fn RtlSetOwnerSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, owner: *const c_void, defaulted: u8) -> i32;
    fn RtlSetGroupSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, group: *const c_void, defaulted: u8) -> i32;
    fn RtlSetDaclSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, present: u8, dacl: *const c_void, defaulted: u8) -> i32;
    fn RtlSetSaclSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, present: u8, sacl: *const c_void, defaulted: u8) -> i32;
    fn RtlMakeSelfRelativeSD(abs: *const c_void, rel: PSECURITY_DESCRIPTOR, len: *mut u32) -> i32;
    fn RtlAbsoluteToSelfRelativeSD(abs: *const c_void, rel: PSECURITY_DESCRIPTOR, len: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let parts = rtl_std_parts();
    let configs: [(&str, bool, bool, u8, u8); 6] = [
        ("everything", true, true, 1, 1),
        ("owner and group", true, true, 0, 0),
        ("DACL only", false, false, 1, 0),
        ("SACL only", false, false, 0, 1),
        ("NULL DACL and NULL SACL", false, false, 2, 2),
        ("nothing", false, false, 0, 0),
    ];
    for (name, o, g, d, s) in configs {
        let sd = rtl_build_sd(&parts, o, g, d, s);
        let mut rel = Raw::<512>::new();
        let mut len = 0u32;
        // SAFETY: valid descriptor; a zero length asks for the size, then the buffer has 512 bytes.
        let st0 = unsafe { RtlAbsoluteToSelfRelativeSD(sd.b.as_ptr().cast(), rel.p(), &mut len) };
        // SAFETY: valid descriptor and a 512-byte buffer with its length.
        let st = unsafe { RtlAbsoluteToSelfRelativeSD(sd.b.as_ptr().cast(), rel.p(), &mut len) };
        let _ = st0;
        let mut rel2 = Raw::<512>::new();
        let (st2, len2) = rtl_make_rel(&sd, &mut rel2);
        say!(
            t,
            "{name}: status={:#010x} length={len} control={:#06x} offsets {} {} {} {} same as RtlMakeSelfRelativeSD: {}",
            st as u32,
            rel.u16_at(2),
            rel.u32_at(4),
            rel.u32_at(8),
            rel.u32_at(12),
            rel.u32_at(16),
            st == st2 && len == len2 && rel.b == rel2.b
        );
        rtl_hex_lines(t, name, &rel.b[..(len as usize).min(512)]);
    }
}
