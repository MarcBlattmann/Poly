//! AddAccessDeniedAce: the bytes of the ACE (type 1), masks stored verbatim, SIDs of several
//! lengths, appending after other ACEs.
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
const ACL_REVISION: u32 = 2;
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

/// Fills `buf` with 0xCC, then InitializeAcl(buf, size, rev); returns its result.
fn acl_init_fill<const N: usize>(buf: &mut Raw<N>, size: u32, rev: u32) -> i32 {
    buf.b.fill(0xCC);
    clear_error();
    // SAFETY: `buf` is at least as large as the ACL size given in every use.
    unsafe { InitializeAcl(buf.p(), size, rev) }
}

/// The ACL header, and all of the ACL's bytes (its size field, at most the buffer) as hex.
fn acl_show<const N: usize>(t: &mut Test, label: &str, buf: &Raw<N>) {
    let size = usize::from(buf.u16_at(2)).min(N);
    asay!(
        t,
        "{label}: rev={} sbz1={} size={} count={} sbz2={} bytes={}",
        buf.b[0],
        buf.b[1],
        buf.u16_at(2),
        buf.u16_at(4),
        buf.u16_at(6),
        Hex(&buf.b[..size])
    );
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

/// One of the AddAccess*Ace functions as `(acl, revision, flags, mask, sid) -> BOOL`.
type AclAddFn = fn(PACL, u32, u32, u32, *const c_void) -> i32;

fn acl_add_denied(acl: PACL, rev: u32, _flags: u32, mask: u32, sid: *const c_void) -> i32 {
    // SAFETY: the pointers are an ACL buffer and a SID buffer of the caller.
    unsafe { AddAccessDeniedAce(acl, rev, mask, sid) }
}

/// Scenario: masks stored verbatim, SIDs of several lengths, several ACEs in a row.
fn acl_scn_masks(t: &mut Test, add: AclAddFn, flags: u32) {
    let sid = sid_of("S-1-1-0");
    for mask in [
        0u32,
        1,
        0x1F01FF,
        0x0010_0000,
        0x1000_0000,
        0x2000_0000,
        0x4000_0000,
        0x8000_0000,
        0x0200_0000,
        0x0100_0000,
        0xFFFF_FFFF,
    ] {
        let mut b = Raw::<1100>::new();
        acl_init_fill(&mut b, 64, ACL_REVISION);
        clear_error();
        let ok = add(b.p(), ACL_REVISION, flags, mask, sid.as_const());
        let mut l = Buf::<100>::new();
        let _ = write!(l, "mask {mask:#x}: ok={} err={}", ok, acl_err(ok));
        acl_show(t, l.as_str(), &b);
    }
    for text in [
        "S-1-1-0",
        "S-1-5-18",
        "S-1-5-32-544",
        "S-1-5-5-0-12345",
        "S-1-5-21-111-222-333-1001",
        "S-1-5-1-2-3-4-5-6-7-8-9-10-11-12-13-14-15",
    ] {
        let s = sid_of(text);
        let mut b = Raw::<1100>::new();
        acl_init_fill(&mut b, 200, ACL_REVISION);
        clear_error();
        let ok = add(b.p(), ACL_REVISION, flags, 0x1F01FF, s.as_const());
        let mut l = Buf::<100>::new();
        let _ = write!(l, "sid {text}: ok={} err={}", ok, acl_err(ok));
        acl_show(t, l.as_str(), &b);
    }
    // Several in a row: appended in call order.
    let mut b = Raw::<1100>::new();
    acl_init_fill(&mut b, 200, ACL_REVISION);
    for (i, (text, mask)) in
        [("S-1-1-0", 1u32), ("S-1-5-18", 2), ("S-1-5-32-544", 4), ("S-1-1-0", 8)].into_iter().enumerate()
    {
        let s = sid_of(text);
        let ok = add(b.p(), ACL_REVISION, flags, mask, s.as_const());
        asay!(t, "append {i}: ok={ok}");
    }
    acl_show(t, "after four", &b);
    acl_list(t, "after four", &b);
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
    fn AddAccessAllowedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAccessDeniedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    acl_scn_masks(t, acl_add_denied, 0);
    // Appended after an allowed ACE: no reordering.
    let mut b = Raw::<1100>::new();
    acl_init_fill(&mut b, 128, ACL_REVISION);
    let s = sid_of("S-1-5-32-544");
    let e = sid_of("S-1-1-0");
    // SAFETY: valid ACL and SIDs, room for the ACEs.
    unsafe {
        AddAccessAllowedAce(b.p(), 2, 0x1, e.as_const());
        AddAccessDeniedAce(b.p(), 2, 0x2, s.as_const());
        AddAccessAllowedAce(b.p(), 2, 0x4, s.as_const());
        AddAccessDeniedAce(b.p(), 2, 0x8, e.as_const());
    }
    acl_list(t, "allowed, denied, allowed, denied", &b);
}
