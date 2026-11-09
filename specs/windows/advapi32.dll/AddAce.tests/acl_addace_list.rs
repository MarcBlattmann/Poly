//! AddAce with a list of several ACEs: the list length against the ACE sizes (shorter, in the
//! middle of an ACE), the start index with several ACEs, ACE size fields that are rejected, and ACE
//! types. (Lists whose length or ACE sizes do not add up the other way, such as trailing bytes
//! after the last ACE or an ACE size of 2 or 3, made the call not return; they are not tested.)
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

/// The units of a NUL-terminated UTF-16 string.
fn wide_len(p: *const u16) -> usize {
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
    }
    n
}

/// A UTF-16 string pointer as text (ASCII only; others as `?`).
fn wide_text<const N: usize>(p: *const u16) -> Buf<N> {
    let mut b = Buf::<N>::new();
    if p.is_null() {
        let _ = write!(b, "(null)");
        return b;
    }
    let n = wide_len(p);
    for i in 0..n {
        // SAFETY: i < n, inside the string.
        let c = unsafe { *p.add(i) };
        let _ = write!(b, "{}", if c < 0x80 { c as u8 as char } else { '?' });
    }
    b
}

/// The SID in the text form of `ConvertSidToStringSidW`, or an error marker.
fn sid_text(sid: *const c_void) -> Buf<200> {
    let mut out: *mut u16 = core::ptr::null_mut();
    // SAFETY: `sid` is a valid SID or the call fails; `out` is a valid out pointer.
    let ok = unsafe { ConvertSidToStringSidW(sid, &mut out) };
    if ok == 0 {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "<invalid sid>");
        return b;
    }
    let b = wide_text::<200>(out);
    // SAFETY: the string was allocated by the call with LocalAlloc.
    unsafe { LocalFree(out.cast()) };
    b
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

/// A pointer stored in a result buffer, as an offset from the buffer start
/// (so printed values do not depend on addresses); -1 for null, -2 outside.
fn rel(base: *const u8, len: usize, p: *const c_void) -> i64 {
    if p.is_null() {
        return -1;
    }
    let d = (p as usize).wrapping_sub(base as usize);
    if d < len { d as i64 } else { -2 }
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

/// Walks the ACEs by their size fields (as far as they stay inside the ACL) and prints each:
/// offset, type, flags, size, mask and the SID's text.
fn acl_list<const N: usize>(t: &mut Test, label: &str, buf: &Raw<N>) {
    let count = usize::from(buf.u16_at(4));
    let size = usize::from(buf.u16_at(2)).min(N);
    let mut off = 8usize;
    for i in 0..count.min(40) {
        if off + 8 > size {
            asay!(t, "{label} ace {i}: at {off}, outside the ACL");
            break;
        }
        let sz = usize::from(buf.u16_at(off + 2));
        let sid = if off + 8 + 68 <= N { sid_text(buf.b[off + 8..].as_ptr().cast()) } else { Buf::<200>::new() };
        asay!(
            t,
            "{label} ace {i}: off={off} type={:#x} flags={:#x} size={sz} mask={:#x} sid={}",
            buf.b[off],
            buf.b[off + 1],
            buf.u32_at(off + 4),
            sid.as_str()
        );
        if sz == 0 {
            break;
        }
        off += sz;
    }
}

/// A short name for an error value printed after a failure: the number, or "-" after success.
fn acl_err(ok: i32) -> i64 {
    gle_if(ok)
}

/// `rel()` of a pointer result inside the ACL buffer: the offset, -1 for NULL, -2 outside.
fn acl_off<const N: usize>(buf: &Raw<N>, p: *const c_void) -> i64 {
    rel(buf.b.as_ptr(), N, p)
}

/// AddAce(acl, revision, start, list, len): (result, last error).
fn acl_add_ace_list<const N: usize>(b: &mut Raw<N>, rev: u32, start: u32, list: &AclAce, len: u32) -> (i32, i64) {
    clear_error();
    // SAFETY: the list buffer holds at least `len` bytes in every use; the ACL buffer is valid.
    let ok = unsafe { AddAce(b.p(), rev, start, list.p(), len) };
    (ok, acl_err(ok))
}

/// FindFirstFreeAce: (result, last error, offset of the result in the buffer: -1 NULL, -2 elsewhere,
/// -3 the out pointer was left as it was).
fn acl_first_free<const N: usize>(b: &mut Raw<N>) -> (i32, i64, i64) {
    let marker = 0x1234usize as PVOID;
    let mut p: PVOID = marker;
    clear_error();
    // SAFETY: valid ACL buffer and out pointer.
    let ok = unsafe { FindFirstFreeAce(b.p(), &mut p) };
    let err = acl_err(ok);
    let off = if p == marker { -3 } else { acl_off(b, p) };
    (ok, err, off)
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
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn InitializeAcl(acl: PACL, len: u32, revision: u32) -> BOOL;
    fn AddAce(acl: PACL, revision: u32, start: u32, list: *const c_void, len: u32) -> BOOL;
    fn FindFirstFreeAce(acl: PACL, ace: *mut PVOID) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let a = acl_ace(ACCESS_ALLOWED_ACE_TYPE, 0, 0x11, "S-1-1-0");
    let d = acl_ace(ACCESS_DENIED_ACE_TYPE, 0, 0x22, "S-1-5-18");
    let au = acl_ace(SYSTEM_AUDIT_ACE_TYPE, 0xC0, 0x33, "S-1-5-32-544");
    let mut list = AclAce::empty();
    list.push(&a);
    list.push(&d);
    list.push(&au);
    asay!(t, "list: {} bytes (ACEs of 20, 20 and 24)", list.len);
    for len in [0u32, 1, 3, 4, 8, 19, 20, 21, 23, 24, 39, 40, 41, 43, 44, 45, 63, 64] {
        let mut b = Raw::<1100>::new();
        acl_init_fill(&mut b, 256, 2);
        let (ok, err) = acl_add_ace_list(&mut b, 2, 0, &list, len);
        let (_, _, free) = acl_first_free(&mut b);
        asay!(
            t,
            "length {len}: ok={ok} err={err} count={} first free={free} bytes in ACL={}",
            b.u16_at(4),
            Hex(&b.b[8..8 + 70])
        );
    }
    // Into the middle of an ACL that holds two ACEs.
    for start in [0u32, 1, 2, 3, 0xFFFF_FFFF] {
        let mut b = Raw::<1100>::new();
        acl_init_fill(&mut b, 256, 2);
        acl_add_ace_list(&mut b, 2, 0, &d, d.len as u32);
        acl_add_ace_list(&mut b, 2, 1, &d, d.len as u32);
        let (ok, err) = acl_add_ace_list(&mut b, 2, start, &list, list.len as u32);
        asay!(t, "three ACEs at {start:#x} into two: ok={ok} err={err} count={}", b.u16_at(4));
        acl_list(t, "  ", &b);
    }
    // ACE size fields that do not describe the ACE (a single ACE of 20 bytes, length 20).
    for size in [1u16, 4, 100, 0xFFFF] {
        let mut x = a;
        x.b[2..4].copy_from_slice(&size.to_le_bytes());
        let mut b = Raw::<1100>::new();
        acl_init_fill(&mut b, 256, 2);
        let (ok, err) = acl_add_ace_list(&mut b, 2, 0, &x, 20);
        asay!(
            t,
            "one ACE, length 20, size field {size}: ok={ok} err={err} count={} first bytes={}",
            b.u16_at(4),
            Hex(&b.b[8..20])
        );
    }
    // A two-ACE list where the second ACE's size is smaller than its header and mask.
    let mut x = AclAce::empty();
    x.push(&a);
    x.push(&d);
    x.b[22] = 4;
    let mut b = Raw::<1100>::new();
    acl_init_fill(&mut b, 256, 2);
    let (ok, err) = acl_add_ace_list(&mut b, 2, 0, &x, 40);
    asay!(t, "two ACEs (length 40), second size 4: ok={ok} err={err} count={}", b.u16_at(4));
    // ACE types are not checked beyond the revision rules (see acl_addace_space).
    for ty in [0x55u8, 0xFF, 0x11, 0x12] {
        let mut x = a;
        x.b[0] = ty;
        let mut b = Raw::<1100>::new();
        acl_init_fill(&mut b, 256, 2);
        let (ok, err) = acl_add_ace_list(&mut b, 2, 0, &x, 20);
        asay!(t, "one ACE of type {ty:#x}: ok={ok} err={err} count={} first bytes={}", b.u16_at(4), Hex(&b.b[8..20]));
    }
}
