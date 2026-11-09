//! AddAce: ACLs that are exactly full or one byte short, a list of three ACEs that does not fit as
//! a whole, and the ACE types that need a newer ACL revision.
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

const SECURITY_MAX_SID_SIZE: usize = 68;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
const SYSTEM_AUDIT_ACE_TYPE: u8 = 2;
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

/// The last-error code after a failed call, as a plain number.
fn gle() -> u32 {
    // SAFETY: reads the calling thread's last error.
    unsafe { GetLastError() }
}

fn clear_error() {
    // SAFETY: sets the calling thread's last error.
    unsafe { SetLastError(0xDEAD) };
}

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 255 units).
fn wide(s: &str) -> [u16; 256] {
    let mut w = [0u16; 256];
    for (i, c) in s.bytes().enumerate().take(255) {
        w[i] = u16::from(c);
    }
    w
}

/// A SID from its string form (up to 68 bytes), through ConvertStringSidToSidW.
fn sid_of(s: &str) -> SidBuf {
    let w = wide(s);
    let mut p: PSID = core::ptr::null_mut();
    let mut out = SidBuf::new();
    // SAFETY: `w` is NUL-terminated; `p` receives a LocalAlloc'ed SID, copied and freed below.
    unsafe {
        if ConvertStringSidToSidW(w.as_ptr(), &mut p) != 0 {
            let n = GetLengthSid(p) as usize;
            core::ptr::copy_nonoverlapping(p.cast::<u8>(), out.b.as_mut_ptr(), n.min(out.b.len()));
            LocalFree(p);
        }
    }
    out
}

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
}

/// `say!` with a longer line (hex dumps of whole ACLs).
macro_rules! asay {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<2400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// A hand-built ACE (or several, one after the other) for AddAce: `len` bytes of `b`.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct AclAce {
    b: [u8; 512],
    len: usize,
}

impl AclAce {
    const fn empty() -> AclAce {
        AclAce { b: [0; 512], len: 0 }
    }
    fn p(&self) -> *const c_void {
        self.b.as_ptr().cast()
    }
    fn bytes(&self) -> &[u8] {
        &self.b[..self.len]
    }
    /// Appends another ACE (or list) after the bytes already here.
    fn push(&mut self, o: &AclAce) {
        let n = self.len;
        self.b[n..n + o.len].copy_from_slice(&o.b[..o.len]);
        self.len = n + o.len;
    }
}

/// An ACE of type `ty` with `flags`, `mask` and the SID given as text; its size field is
/// 8 + the SID's length.
fn acl_ace(ty: u8, flags: u8, mask: u32, sid: &str) -> AclAce {
    let s = sid_of(sid);
    let sb = s.bytes();
    let size = 8 + sb.len();
    let mut a = AclAce::empty();
    a.b[0] = ty;
    a.b[1] = flags;
    a.b[2..4].copy_from_slice(&(size as u16).to_le_bytes());
    a.b[4..8].copy_from_slice(&mask.to_le_bytes());
    a.b[8..size].copy_from_slice(sb);
    a.len = size;
    a
}

/// Fills `buf` with 0xCC, then InitializeAcl(buf, size, rev); returns its result.
fn acl_init_fill<const N: usize>(buf: &mut Raw<N>, size: u32, rev: u32) -> i32 {
    buf.b.fill(0xCC);
    clear_error();
    // SAFETY: `buf` is at least as large as the ACL size given in every use.
    unsafe { InitializeAcl(buf.p(), size, rev) }
}

/// A short name for an error value printed after a failure: the number, or "-" after success.
fn acl_err(ok: i32) -> i64 {
    gle_if(ok)
}

/// AddAce(acl, revision, start, list, len): (result, last error).
fn acl_add_ace_list<const N: usize>(b: &mut Raw<N>, rev: u32, start: u32, list: &AclAce, len: u32) -> (i32, i64) {
    clear_error();
    // SAFETY: the list buffer holds at least `len` bytes in every use; the ACL buffer is valid.
    let ok = unsafe { AddAce(b.p(), rev, start, list.p(), len) };
    (ok, acl_err(ok))
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn LocalFree(p: PVOID) -> PVOID;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn InitializeAcl(acl: PACL, len: u32, revision: u32) -> BOOL;
    fn AddAce(acl: PACL, revision: u32, start: u32, list: *const c_void, len: u32) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let a = acl_ace(ACCESS_ALLOWED_ACE_TYPE, 0, 0x11, "S-1-1-0");
    let mut list = AclAce::empty();
    list.push(&a);
    list.push(&acl_ace(ACCESS_DENIED_ACE_TYPE, 0, 0x22, "S-1-5-18"));
    list.push(&acl_ace(SYSTEM_AUDIT_ACE_TYPE, 0xC0, 0x33, "S-1-5-32-544"));
    for size in [8u32, 16, 27, 28, 29, 30, 31, 32, 47, 48, 49, 52, 71, 72, 73, 75, 76, 80] {
        let mut b = Raw::<1100>::new();
        let init = acl_init_fill(&mut b, size, 2);
        let (ok1, e1) = acl_add_ace_list(&mut b, 2, 0, &a, 20);
        let c1 = b.u16_at(4);
        let (ok2, e2) = acl_add_ace_list(&mut b, 2, 0xFFFF_FFFF, &a, 20);
        let c2 = b.u16_at(4);
        let mut c = Raw::<1100>::new();
        acl_init_fill(&mut c, size, 2);
        let (ok3, e3) = acl_add_ace_list(&mut c, 2, 0, &list, list.len as u32);
        let tail = b.b[usize::from(b.u16_at(2))..].iter().all(|&x| x == 0xCC);
        asay!(
            t,
            "acl size {size} (init={init}, field {}): one: ok={ok1} err={e1} count={c1}; second: ok={ok2} err={e2} count={c2}; list of three (64 bytes): ok={ok3} err={e3} count={}; tail untouched={tail}",
            b.u16_at(2),
            c.u16_at(4)
        );
    }
    // ACE types against the ACL revision (the ACE has the layout of an access ACE).
    for ty in 0u8..=0x20 {
        let mut x = acl_ace(ty, 0, 0x1F01FF, "S-1-1-0");
        x.b[0] = ty;
        let mut line = Buf::<300>::new();
        for (acl_rev, arg) in [(2u32, 2u32), (3, 3), (4, 4), (2, 4), (4, 2)] {
            let mut b = Raw::<1100>::new();
            acl_init_fill(&mut b, 64, acl_rev);
            let (ok, err) = acl_add_ace_list(&mut b, arg, 0, &x, 20);
            let _ = write!(line, " [acl {acl_rev} arg {arg}: ok={ok} err={err} rev={}]", b.b[0]);
        }
        asay!(t, "type {ty:#x}:{}", line.as_str());
    }
    // A damaged ACL header.
    let cases: [(&str, fn(&mut Raw<1100>)); 11] = [
        ("revision 0", |b| b.b[0] = 0),
        ("revision 1", |b| b.b[0] = 1),
        ("revision 5", |b| b.b[0] = 5),
        ("revision 255", |b| b.b[0] = 255),
        ("sbz1 = 1", |b| b.b[1] = 1),
        ("size 0", |b| b.b[2] = 0),
        ("size 7", |b| b.b[2] = 7),
        ("size 8", |b| b.b[2] = 8),
        ("size 1000", |b| {
            b.b[2] = 0xE8;
            b.b[3] = 0x03;
        }),
        ("count 1 without an ACE", |b| b.b[4] = 1),
        ("count 200 without ACEs", |b| b.b[4] = 200),
    ];
    for (label, f) in cases {
        let mut b = Raw::<1100>::new();
        acl_init_fill(&mut b, 64, 2);
        b.b[8..].fill(0);
        f(&mut b);
        let (ok, err) = acl_add_ace_list(&mut b, 2, 0, &a, 20);
        asay!(t, "header {label}: ok={ok} err={err} count after={} first bytes={}", b.u16_at(4), Hex(&b.b[8..16]));
    }
}
