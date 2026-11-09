//! NtSetInformationToken(TokenOwner / TokenPrimaryGroup / TokenDefaultDacl): the SID must be the user or a group the
//! token has (owner: one allowed as owner), else STATUS_INVALID_OWNER / STATUS_INVALID_PRIMARY_GROUP; lengths; a new
//! default DACL reads back byte for byte; a NULL ACL; an invalid ACL.
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
const TOKEN_ASSIGN_PRIMARY: u32 = 0x0001;
const TOKEN_DUPLICATE: u32 = 0x0002;
const TOKEN_IMPERSONATE: u32 = 0x0004;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_QUERY_SOURCE: u32 = 0x0010;
const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
const TOKEN_ADJUST_GROUPS: u32 = 0x0040;
const TOKEN_ADJUST_DEFAULT: u32 = 0x0080;
const TOKEN_ADJUST_SESSIONID: u32 = 0x0100;
const STANDARD_RIGHTS_REQUIRED: u32 = 0x000F_0000;
const TOKEN_ALL_ACCESS: u32 = STANDARD_RIGHTS_REQUIRED
    | TOKEN_ASSIGN_PRIMARY
    | TOKEN_DUPLICATE
    | TOKEN_IMPERSONATE
    | TOKEN_QUERY
    | TOKEN_QUERY_SOURCE
    | TOKEN_ADJUST_PRIVILEGES
    | TOKEN_ADJUST_GROUPS
    | TOKEN_ADJUST_DEFAULT
    | TOKEN_ADJUST_SESSIONID;

const TOKEN_OWNER_CLASS: u32 = 4;
const TOKEN_PRIMARY_GROUP_CLASS: u32 = 5;
const TOKEN_DEFAULT_DACL_CLASS: u32 = 6;
const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_IMPERSONATION: u32 = 2;
const ACL_REVISION: u32 = 2;
const GENERIC_ALL_RIGHTS: u32 = 0x1000_0000;
const GENERIC_READ_RIGHT: u32 = 0x8000_0000;
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

/// winnt.h: `SID_AND_ATTRIBUTES`.
#[repr(C)]
#[derive(Clone, Copy)]
struct SID_AND_ATTRIBUTES {
    sid: PSID,
    attributes: u32,
}

/// winbase.h: `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: PVOID,
    inherit: BOOL,
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

/// The process token with `access`, or null.
fn process_token(access: u32) -> HANDLE {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: the current process pseudo-handle and a valid out pointer.
    unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut h) };
    h
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

/// A duplicate of the process token (`kind` TOKEN_PRIMARY / TOKEN_IMPERSONATION) or null.
fn dup_token(access: u32, level: u32, kind: u32) -> HANDLE {
    let src = process_token(TOKEN_DUPLICATE | TOKEN_QUERY);
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: `src` is the process token with TOKEN_DUPLICATE; valid out pointer.
    unsafe {
        DuplicateTokenEx(src, access, core::ptr::null(), level, kind, &mut out);
        CloseHandle(src);
    }
    out
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

/// The SID that the first `SID_AND_ATTRIBUTES` of a TOKEN_USER-like result points at.
fn first_sid(raw: &Raw<2048>) -> *const c_void {
    raw.u64_at(0) as usize as *const c_void
}

/// NtQueryInformationToken into `buf` with length `len`; (status, returned length).
fn nt_tq<const N: usize>(token: HANDLE, class: u32, buf: &mut Raw<N>, len: u32) -> (i32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    // SAFETY: `buf` has N writable bytes; callers pass len <= N.
    let st = unsafe { NtQueryInformationToken(token, class, buf.p(), len, &mut ret) };
    (st, ret)
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn LocalFree(p: PVOID) -> PVOID;
    fn GetCurrentProcess() -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn DuplicateTokenEx(
        token: HANDLE,
        access: u32,
        attrs: *const SECURITY_ATTRIBUTES,
        level: u32,
        kind: u32,
        out: *mut HANDLE,
    ) -> BOOL;
    fn InitializeAcl(acl: PACL, len: u32, revision: u32) -> BOOL;
    fn AddAccessAllowedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryInformationToken(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtSetInformationToken(token: HANDLE, class: u32, info: *const c_void, len: u32) -> i32;
}

poly_wtest::main!(test);

#[repr(C)]
struct One {
    sid: PSID,
}

fn set(token: HANDLE, class: u32, text: &str, len: u32) -> i32 {
    let s = sid_of(text);
    let one = One { sid: s.as_const() as PSID };
    // SAFETY: a valid TOKEN_OWNER / TOKEN_PRIMARY_GROUP (8 bytes) pointing at a valid SID.
    unsafe { NtSetInformationToken(token, class, (&one as *const One).cast(), len) }
}

fn current(token: HANDLE, class: u32) -> Buf<200> {
    let mut raw = Raw::<2048>::new();
    nt_tq(token, class, &mut raw, 2048);
    mask_sid(first_sid(&raw))
}

fn test(t: &mut Test) {
    let token = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    say!(
        t,
        "start owner={} primary_group={}",
        current(token, TOKEN_OWNER_CLASS).as_str(),
        current(token, TOKEN_PRIMARY_GROUP_CLASS).as_str()
    );
    for (class, name) in [(TOKEN_OWNER_CLASS, "owner"), (TOKEN_PRIMARY_GROUP_CLASS, "primary group")] {
        for text in ["S-1-5-32-544", "S-1-5-32-545", "S-1-1-0", "S-1-5-11", "S-1-5-18", "S-1-5-32-546", "S-1-16-4096"] {
            let st = set(token, class, text, 8);
            say!(t, "{name} <- {text}: status={st:#010x} now {}", current(token, class).as_str());
        }
        for len in [0u32, 4, 7, 9, 16] {
            let st = set(token, class, "S-1-5-32-544", len);
            say!(t, "{name} with length {len}: status={st:#010x}");
        }
    }
    let everyone = sid_of("S-1-1-0");
    let admins = sid_of("S-1-5-32-544");
    let mut acl = AclBuf::new();
    // SAFETY: the ACL buffer is 1024 bytes; the SIDs are valid.
    unsafe {
        InitializeAcl(acl.p(), 1024, ACL_REVISION);
        AddAccessAllowedAce(acl.p(), ACL_REVISION, GENERIC_ALL_RIGHTS, admins.as_const());
        AddAccessAllowedAce(acl.p(), ACL_REVISION, GENERIC_READ_RIGHT, everyone.as_const());
    }
    let dd = One { sid: acl.p() };
    // SAFETY: a valid TOKEN_DEFAULT_DACL pointing at a valid ACL.
    let st = unsafe { NtSetInformationToken(token, TOKEN_DEFAULT_DACL_CLASS, (&dd as *const One).cast(), 8) };
    say!(t, "default DACL, two-ACE ACL: status={st:#010x}");
    let mut back = Raw::<4096>::new();
    let (qs, len) = nt_tq(token, TOKEN_DEFAULT_DACL_CLASS, &mut back, 4096);
    let off = rel(back.b.as_ptr(), 4096, back.u64_at(0) as usize as *const c_void);
    let size = if off > 0 { usize::from(back.u16_at(off as usize + 2)) } else { 0 };
    say!(
        t,
        "read back: status={qs:#010x} length={len} acl_offset={off} acl_size={size} same bytes={} ace_count={}",
        off > 0 && back.b[off as usize..off as usize + size] == acl.bytes()[..size],
        if off > 0 { back.u16_at(off as usize + 4) } else { 0 }
    );
    for len in [0u32, 4, 7, 9] {
        // SAFETY: a valid TOKEN_DEFAULT_DACL; the length is deliberately wrong.
        let st = unsafe { NtSetInformationToken(token, TOKEN_DEFAULT_DACL_CLASS, (&dd as *const One).cast(), len) };
        say!(t, "default DACL with length {len}: status={st:#010x}");
    }
    let dn = One { sid: core::ptr::null_mut() };
    // SAFETY: a TOKEN_DEFAULT_DACL with a NULL ACL.
    let st = unsafe { NtSetInformationToken(token, TOKEN_DEFAULT_DACL_CLASS, (&dn as *const One).cast(), 8) };
    let (qs, qlen) = nt_tq(token, TOKEN_DEFAULT_DACL_CLASS, &mut back, 4096);
    say!(
        t,
        "default DACL, NULL ACL: status={st:#010x}; read back status={qs:#010x} length={qlen} acl_null={}",
        back.u64_at(0) == 0
    );
    let mut bad = AclBuf::new();
    bad.b[0] = 9;
    bad.b[2] = 8;
    let db = One { sid: bad.p() };
    // SAFETY: a TOKEN_DEFAULT_DACL pointing at a buffer that is not a valid ACL (revision 9).
    let st = unsafe { NtSetInformationToken(token, TOKEN_DEFAULT_DACL_CLASS, (&db as *const One).cast(), 8) };
    say!(t, "default DACL, ACL with revision 9: status={st:#010x}");
    // SAFETY: closing the handle.
    unsafe { CloseHandle(token) };
}
