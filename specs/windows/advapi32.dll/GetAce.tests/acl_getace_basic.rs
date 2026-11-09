//! GetAce: the pointer returned for each index (an offset inside the ACL), the ACE it points at,
//! that it points into the ACL itself, and ACLs of revision 3 and 4 with other ACE types.
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
const TRUE: BOOL = 1;
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
const OBJECT_INHERIT_ACE: u8 = 0x1;
const CONTAINER_INHERIT_ACE: u8 = 0x2;
const SUCCESSFUL_ACCESS_ACE_FLAG: u8 = 0x40;
const FAILED_ACCESS_ACE_FLAG: u8 = 0x80;
const SYSTEM_MANDATORY_LABEL_NO_WRITE_UP: u32 = 0x1;
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

/// A revision-2 ACL of `size` bytes with three ACEs: allowed S-1-1-0 mask 1 (20 bytes), denied
/// S-1-5-18 mask 2 (20 bytes), allowed S-1-5-32-544 mask 4 (24 bytes); 72 bytes are in use.
fn acl_three<const N: usize>(buf: &mut Raw<N>, size: u32) {
    acl_init_fill(buf, size, ACL_REVISION);
    let a = sid_of("S-1-1-0");
    let s = sid_of("S-1-5-18");
    let adm = sid_of("S-1-5-32-544");
    // SAFETY: `buf` holds an initialised ACL with room for the three ACEs; the SIDs are valid.
    unsafe {
        AddAccessAllowedAce(buf.p(), 2, 1, a.as_const());
        AddAccessDeniedAce(buf.p(), 2, 2, s.as_const());
        AddAccessAllowedAce(buf.p(), 2, 4, adm.as_const());
    }
}

/// GetAce: (result, last error, offset of the returned ACE in the buffer: -1 NULL, -2 elsewhere,
/// -3 the out pointer was left as it was).
fn acl_get_ace<const N: usize>(b: &mut Raw<N>, index: u32) -> (i32, i64, i64) {
    let marker = 0x1234usize as PVOID;
    let mut p: PVOID = marker;
    clear_error();
    // SAFETY: valid ACL buffer and a valid out pointer.
    let ok = unsafe { GetAce(b.p(), index, &mut p) };
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
    fn GetAce(acl: *const c_void, index: u32, ace: *mut PVOID) -> BOOL;
    fn AddAccessAllowedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAccessAllowedAceEx(acl: PACL, revision: u32, flags: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAccessDeniedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAuditAccessAceEx(
        acl: PACL,
        revision: u32,
        flags: u32,
        mask: u32,
        sid: *const c_void,
        success: BOOL,
        failure: BOOL,
    ) -> BOOL;
    fn AddMandatoryAce(acl: PACL, revision: u32, flags: u32, policy: u32, sid: *const c_void) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut b = Raw::<1100>::new();
    acl_three(&mut b, 128);
    for index in 0u32..3 {
        let (ok, err, off) = acl_get_ace(&mut b, index);
        let bytes = if off >= 0 {
            let o = off as usize;
            let n = usize::from(b.u16_at(o + 2));
            let mut h = Buf::<200>::new();
            let _ = write!(h, "{}", Hex(&b.b[o..o + n]));
            h
        } else {
            Buf::<200>::new()
        };
        asay!(t, "index {index}: ok={ok} err={err} offset={off} ace={}", bytes.as_str());
    }
    // Writing through the returned pointer changes the ACL.
    let mut p: PVOID = core::ptr::null_mut();
    // SAFETY: valid ACL, valid out pointer; the ACE has at least 8 bytes.
    unsafe {
        GetAce(b.p(), 1, &mut p);
        *(p.cast::<u8>().add(4)) = 0x7F;
    }
    asay!(t, "mask byte of ACE 1 after writing through the pointer: {:#x}", b.b[28 + 4]);
    // ACEs with flags, a mandatory label and an object-less audit ACE in a revision 4 ACL.
    let mut c = Raw::<1100>::new();
    acl_init_fill(&mut c, 256, 4);
    let s = sid_of("S-1-5-32-545");
    let label = sid_of("S-1-16-8192");
    // SAFETY: valid ACL and SIDs, room for the ACEs.
    unsafe {
        AddAccessAllowedAceEx(c.p(), 4, u32::from(CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE), 0x1200A9, s.as_const());
        AddAuditAccessAceEx(
            c.p(),
            4,
            u32::from(SUCCESSFUL_ACCESS_ACE_FLAG | FAILED_ACCESS_ACE_FLAG),
            0x2,
            s.as_const(),
            TRUE,
            TRUE,
        );
        AddMandatoryAce(c.p(), 4, 0, SYSTEM_MANDATORY_LABEL_NO_WRITE_UP, label.as_const());
    }
    acl_list(t, "rev 4 ACL", &c);
    for index in 0u32..4 {
        let (ok, err, off) = acl_get_ace(&mut c, index);
        asay!(t, "rev 4 ACL index {index}: ok={ok} err={err} offset={off}");
    }
}
