//! NtSetInformationToken(TokenIntegrityLevel) on a fresh copy of the token for each case: lowering works, the
//! label's attributes must be SE_GROUP_INTEGRITY, the SID must be an integrity SID, raising back fails, the length
//! rules, and the rights of the handle.
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
type DWORD = u32;
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

const TOKEN_INTEGRITY_LEVEL_CLASS: u32 = 25;
const TOKEN_MANDATORY_POLICY_CLASS: u32 = 27;
const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_IMPERSONATION: u32 = 2;
const SE_GROUP_ENABLED: u32 = 0x4;
const SE_GROUP_INTEGRITY: u32 = 0x20;
const SE_GROUP_INTEGRITY_ENABLED: u32 = 0x40;
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

/// The process token with `access`, or null.
fn process_token(access: u32) -> HANDLE {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: the current process pseudo-handle and a valid out pointer.
    unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut h) };
    h
}

/// `GetTokenInformation` into `buf`; (success, needed length, last error).
fn token_info<const N: usize>(token: HANDLE, class: u32, buf: &mut Raw<N>) -> (bool, u32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    clear_error();
    // SAFETY: `buf` is N writable bytes, `ret` a valid out pointer.
    let ok = unsafe { GetTokenInformation(token, class, buf.p(), N as u32, &mut ret) } != 0;
    (ok, ret, gle())
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

/// The SID that the first `SID_AND_ATTRIBUTES` of a TOKEN_USER-like result points at.
fn first_sid(raw: &Raw<2048>) -> *const c_void {
    raw.u64_at(0) as usize as *const c_void
}

/// The token's integrity RID (TokenIntegrityLevel), or -1.
fn integrity_rid(token: HANDLE) -> i64 {
    let mut raw = Raw::<2048>::new();
    let (ok, _, _) = token_info(token, TOKEN_INTEGRITY_LEVEL_CLASS, &mut raw);
    if !ok {
        return -1;
    }
    let sid = first_sid(&raw);
    // SAFETY: the SID is inside `raw`.
    unsafe {
        let n = *GetSidSubAuthorityCount(sid);
        i64::from(*GetSidSubAuthority(sid, u32::from(n) - 1))
    }
}

/// NtQueryInformationToken into `buf` with length `len`; (status, returned length).
fn nt_tq<const N: usize>(token: HANDLE, class: u32, buf: &mut Raw<N>, len: u32) -> (i32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    // SAFETY: `buf` has N writable bytes; callers pass len <= N.
    let st = unsafe { NtQueryInformationToken(token, class, buf.p(), len, &mut ret) };
    (st, ret)
}

/// A one-DWORD token class through NtQueryInformationToken, or -1 when it fails.
fn nt_tok_u32(token: HANDLE, class: u32) -> i64 {
    let mut raw = Raw::<64>::new();
    let (st, _) = nt_tq(token, class, &mut raw, 4);
    if st >= 0 { i64::from(raw.u32_at(0)) } else { -1 }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn LocalFree(p: PVOID) -> PVOID;
    fn GetCurrentProcess() -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSidSubAuthority(sid: *const c_void, n: u32) -> *mut u32;
    fn GetSidSubAuthorityCount(sid: *const c_void) -> *mut u8;
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> BOOL;
    fn DuplicateTokenEx(
        token: HANDLE,
        access: u32,
        attrs: *const SECURITY_ATTRIBUTES,
        level: u32,
        kind: u32,
        out: *mut HANDLE,
    ) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryInformationToken(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtSetInformationToken(token: HANDLE, class: u32, info: *const c_void, len: u32) -> i32;
}

poly_wtest::main!(test);

#[repr(C)]
struct Label {
    sid: PSID,
    attrs: u32,
}

fn set_len(token: HANDLE, sid: &SidBuf, attrs: u32, len: u32) -> i32 {
    let l = Label { sid: sid.as_const() as PSID, attrs };
    // SAFETY: a valid TOKEN_MANDATORY_LABEL pointing at a valid SID; the length is the caller's choice.
    unsafe { NtSetInformationToken(token, TOKEN_INTEGRITY_LEVEL_CLASS, (&l as *const Label).cast(), len) }
}

fn set(token: HANDLE, sid: &SidBuf, attrs: u32) -> i32 {
    set_len(token, sid, attrs, core::mem::size_of::<Label>() as u32 + sid.bytes().len() as u32)
}

fn test(t: &mut Test) {
    for (name, text, attrs) in [
        ("High (same)", "S-1-16-12288", SE_GROUP_INTEGRITY),
        ("Medium", "S-1-16-8192", SE_GROUP_INTEGRITY),
        ("Low", "S-1-16-4096", SE_GROUP_INTEGRITY),
        ("Untrusted", "S-1-16-0", SE_GROUP_INTEGRITY),
        ("System (raising)", "S-1-16-16384", SE_GROUP_INTEGRITY),
        ("Low with attrs 0", "S-1-16-4096", 0),
        ("Low with attrs INTEGRITY|INTEGRITY_ENABLED", "S-1-16-4096", SE_GROUP_INTEGRITY | SE_GROUP_INTEGRITY_ENABLED),
        ("Low with attrs ENABLED", "S-1-16-4096", SE_GROUP_ENABLED),
        ("not an integrity SID (S-1-5-18)", "S-1-5-18", SE_GROUP_INTEGRITY),
        ("odd RID 5000", "S-1-16-5000", SE_GROUP_INTEGRITY),
    ] {
        let token = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
        let s = sid_of(text);
        let st = set(token, &s, attrs);
        say!(
            t,
            "{name}: status={st:#010x} integrity now={:#x} policy now={}",
            integrity_rid(token),
            nt_tok_u32(token, TOKEN_MANDATORY_POLICY_CLASS)
        );
        // SAFETY: closing the handle.
        unsafe { CloseHandle(token) };
    }
    let token = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    say!(t, "chain start integrity={:#x}", integrity_rid(token));
    for text in ["S-1-16-8192", "S-1-16-12288", "S-1-16-4096", "S-1-16-8192", "S-1-16-0", "S-1-16-4096"] {
        let st = set(token, &sid_of(text), SE_GROUP_INTEGRITY);
        say!(t, "set {text}: status={st:#010x} integrity now={:#x}", integrity_rid(token));
    }
    // SAFETY: closing the handle.
    unsafe { CloseHandle(token) };
    let low = sid_of("S-1-16-4096");
    let sid_len = low.bytes().len() as u32;
    for len in [0u32, 8, 15, 16, 16 + sid_len - 1, 16 + sid_len, 16 + sid_len + 8] {
        let token = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
        let st = set_len(token, &low, SE_GROUP_INTEGRITY, len);
        say!(
            t,
            "Low with length {len} (SID is {sid_len} bytes): status={st:#010x} integrity now={:#x}",
            integrity_rid(token)
        );
        // SAFETY: closing the handle.
        unsafe { CloseHandle(token) };
    }
    for (label, access) in [
        ("TOKEN_QUERY", TOKEN_QUERY),
        ("TOKEN_ADJUST_DEFAULT", TOKEN_ADJUST_DEFAULT),
        ("TOKEN_ADJUST_PRIVILEGES", TOKEN_ADJUST_PRIVILEGES),
    ] {
        let q = dup_token(access, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
        let st = set(q, &low, SE_GROUP_INTEGRITY);
        say!(t, "with only {label}: status={st:#010x}");
        // SAFETY: closing the handle.
        unsafe { CloseHandle(q) };
    }
    let imp = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_IMPERSONATION);
    let st = set(imp, &low, SE_GROUP_INTEGRITY);
    say!(t, "impersonation token lowered to Low: status={st:#010x} integrity now={:#x}", integrity_rid(imp));
    // SAFETY: closing the handle.
    unsafe { CloseHandle(imp) };
}
