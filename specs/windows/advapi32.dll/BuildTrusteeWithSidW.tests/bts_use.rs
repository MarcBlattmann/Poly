//! BuildTrusteeWithSidW: a trustee built from a SID used in an explicit-access entry and handed to
//! SetEntriesInAclW: the ACE gets that SID; an invalid SID (revision 2, all zero) and a trustee type
//! changed after the call.
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

/// The bytes of the little-endian u32 at `off`.
fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// The text of a SID with the machine-dependent parts masked: S-1-5-21-a-b-c-RID prints as
/// S-1-5-21-<machine>-RID and a logon-session SID S-1-5-5-x-y as S-1-5-5-<logon>.
fn mask_sid(sid: *const c_void) -> Buf<200> {
    let full = sid_text(sid);
    let s = full.as_str();
    let mut b = Buf::<200>::new();
    if s.starts_with("S-1-5-21-") && s.matches('-').count() >= 7 {
        let rid = s.rsplit('-').next().unwrap_or("?");
        if rid.parse::<u32>().map_or(false, |r| r >= 1000) {
            let _ = write!(b, "S-1-5-21-<machine>-<account>");
        } else {
            let _ = write!(b, "S-1-5-21-<machine>-{rid}");
        }
    } else if s.starts_with("S-1-5-5-") {
        let _ = write!(b, "S-1-5-5-<logon>");
    } else {
        let _ = write!(b, "{s}");
    }
    b
}

const GRANT_ACCESS: u32 = 1;
const TRUSTEE_IS_SID: u32 = 0;
const TRUSTEE_IS_UNKNOWN: u32 = 0;
const TRUSTEE_IS_USER: u32 = 1;
/// accctrl.h `TRUSTEE_W` (32 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct TRUSTEE_W {
    multiple: *mut c_void,
    multiple_op: u32,
    form: u32,
    kind: u32,
    name: *mut u16,
}

/// accctrl.h `EXPLICIT_ACCESS_W` (48 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct EXPLICIT_ACCESS_W {
    access: u32,
    mode: u32,
    inherit: u32,
    trustee: TRUSTEE_W,
}

impl EXPLICIT_ACCESS_W {
    fn filled(byte: u8) -> EXPLICIT_ACCESS_W {
        let mut e = EXPLICIT_ACCESS_W {
            access: 0,
            mode: 0,
            inherit: 0,
            trustee: TRUSTEE_W {
                multiple: core::ptr::null_mut(),
                multiple_op: 0,
                form: 0,
                kind: 0,
                name: core::ptr::null_mut(),
            },
        };
        // SAFETY: a plain-old-data structure; every byte is overwritten.
        unsafe {
            core::ptr::write_bytes(
                &mut e as *mut EXPLICIT_ACCESS_W as *mut u8,
                byte,
                core::mem::size_of::<EXPLICIT_ACCESS_W>(),
            )
        };
        e
    }
    /// An entry for a SID trustee, built by hand (no API call): form SID, type `kind`.
    fn for_sid(sid: PSID, access: u32, mode: u32, inherit: u32, kind: u32) -> EXPLICIT_ACCESS_W {
        EXPLICIT_ACCESS_W {
            access,
            mode,
            inherit,
            trustee: TRUSTEE_W {
                multiple: core::ptr::null_mut(),
                multiple_op: 0,
                form: TRUSTEE_IS_SID,
                kind,
                name: sid.cast(),
            },
        }
    }
}

/// The ACE list of an ACL as lines `<label>: ace i type flags mask sid` (via `sd_acl_show`).
fn c2_acl(t: &mut Test, label: &str, acl: *const c_void) {
    sd_acl_show(t, label, acl);
}

/// `say!` with a longer line (SDDL texts, hex dumps).
macro_rules! sdsay {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<2000>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const SD_SENTINEL: usize = 0x1234_5678_9ABC;
/// An ACL's header and ACEs from a pointer: "rev size count" then per ACE "type flags size mask sid".
fn sd_acl_show(t: &mut Test, label: &str, acl: *const c_void) {
    if acl.is_null() {
        sdsay!(t, "{label}: (null)");
        return;
    }
    if acl as usize == SD_SENTINEL {
        sdsay!(t, "{label}: (not returned)");
        return;
    }
    // SAFETY: the caller passes a pointer to an ACL header (8 bytes at least).
    let head = unsafe { core::slice::from_raw_parts(acl as *const u8, 8) };
    let size = usize::from(u16::from_le_bytes([head[2], head[3]])).min(1024);
    let count = usize::from(u16::from_le_bytes([head[4], head[5]]));
    sdsay!(
        t,
        "{label}: acl rev={} sbz1={} size={} count={count} sbz2={}",
        head[0],
        head[1],
        size,
        u16::from_le_bytes([head[6], head[7]])
    );
    // SAFETY: the ACL is `size` bytes (as its own header says, at most 1024 here).
    let b = unsafe { core::slice::from_raw_parts(acl as *const u8, size.max(8)) };
    let mut off = 8usize;
    for i in 0..count.min(30) {
        if off + 8 > b.len() {
            sdsay!(t, "{label}: ace {i} at {off} is outside the ACL");
            break;
        }
        let ty = b[off];
        let flags = b[off + 1];
        let sz = usize::from(u16::from_le_bytes([b[off + 2], b[off + 3]]));
        let mask = le32(b, off + 4);
        let sid_off = if (5..=8).contains(&ty) && off + 12 <= b.len() {
            let f = le32(b, off + 8);
            off + 12 + if f & 1 != 0 { 16 } else { 0 } + if f & 2 != 0 { 16 } else { 0 }
        } else {
            off + 8
        };
        let sid = if sid_off + 8 <= b.len() { mask_sid(b[sid_off..].as_ptr().cast()) } else { Buf::<200>::new() };
        sdsay!(
            t,
            "{label}: ace {i} off={off} type={ty:#x} flags={flags:#x} size={sz} mask={mask:#x} sid={}",
            sid.as_str()
        );
        if sz == 0 {
            break;
        }
        off += sz;
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalFree(p: PVOID) -> PVOID;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn BuildTrusteeWithSidW(t: *mut TRUSTEE_W, sid: PSID);
    fn SetEntriesInAclW(count: u32, entries: *mut EXPLICIT_ACCESS_W, old: PACL, new: *mut PACL) -> u32;
}

poly_wtest::main!(test);

fn make(t: &mut Test, label: &str, sid: PSID, kind: u32) {
    let mut e = EXPLICIT_ACCESS_W::filled(0);
    e.access = 0x120089;
    e.mode = GRANT_ACCESS;
    // SAFETY: a valid trustee structure and a SID-shaped buffer.
    unsafe { BuildTrusteeWithSidW(&mut e.trustee, sid) };
    e.trustee.kind = kind;
    let mut acl: PACL = core::ptr::null_mut();
    // SAFETY: a valid entry.
    let r = unsafe { SetEntriesInAclW(1, &mut e, core::ptr::null_mut(), &mut acl) };
    say!(t, "{label}: form={} result {r}", e.trustee.form);
    if r == 0 && !acl.is_null() {
        c2_acl(t, label, acl);
        // SAFETY: allocated by the call.
        unsafe { LocalFree(acl) };
    }
}

fn test(t: &mut Test) {
    let mut a = sid_of("S-1-5-32-544");
    make(t, "Administrators, type unknown", a.as_psid(), TRUSTEE_IS_UNKNOWN);
    make(t, "Administrators, type user", a.as_psid(), TRUSTEE_IS_USER);
    make(t, "Administrators, type 99", a.as_psid(), 99);
    let mut raw = Raw::<64>::new();
    raw.b[0] = 2;
    raw.b[1] = 1;
    raw.b[7] = 5;
    raw.b[8] = 18;
    make(t, "SID revision 2", raw.p(), TRUSTEE_IS_UNKNOWN);
    let mut zero = Raw::<64>::new();
    make(t, "all-zero SID", zero.p(), TRUSTEE_IS_UNKNOWN);
}
