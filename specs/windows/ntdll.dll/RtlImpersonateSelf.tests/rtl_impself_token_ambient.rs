//! The token RtlImpersonateSelf puts on the thread is a copy of the process token (ambient: the oracle's
//! elevated administrator token): same user, groups and privileges, an impersonation token.
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

const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_USER_CLASS: u32 = 1;
const TOKEN_GROUPS_CLASS: u32 = 2;
const TOKEN_PRIVILEGES_CLASS: u32 = 3;
const TOKEN_TYPE_CLASS: u32 = 8;
const TOKEN_IMPERSONATION_LEVEL_CLASS: u32 = 9;
const TOKEN_SESSION_ID_CLASS: u32 = 12;
const TOKEN_INTEGRITY_LEVEL_CLASS: u32 = 25;
const SECURITY_IMPERSONATION: u32 = 2;
/// winnt.h: `LUID`.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct LUID {
    low: u32,
    high: i32,
}

/// winnt.h: `SID_AND_ATTRIBUTES`.
#[repr(C)]
#[derive(Clone, Copy)]
struct SID_AND_ATTRIBUTES {
    sid: PSID,
    attributes: u32,
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

/// A 32-bit token information value (class with a single DWORD), or -1 when the query fails.
fn tok_u32(token: HANDLE, class: u32) -> i64 {
    let mut raw = Raw::<64>::new();
    let mut ret = 0u32;
    // SAFETY: `raw` is 64 writable bytes; the length given is the 4 bytes the classes need.
    let ok = unsafe { GetTokenInformation(token, class, raw.p(), 4, &mut ret) };
    if ok != 0 { i64::from(raw.u32_at(0)) } else { -1 }
}

/// The SID that the first `SID_AND_ATTRIBUTES` of a TOKEN_USER-like result points at.
fn first_sid(raw: &Raw<2048>) -> *const c_void {
    raw.u64_at(0) as usize as *const c_void
}

/// Whether the token has the privilege (by name) and its attributes, or -1.
fn privilege_attrs(token: HANDLE, name: &str) -> i64 {
    let w = wide(name);
    let mut luid = LUID::default();
    // SAFETY: NUL-terminated name, valid out pointer.
    unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut luid) };
    let mut raw = Raw::<4096>::new();
    let (ok, _, _) = token_info(token, TOKEN_PRIVILEGES_CLASS, &mut raw);
    if !ok {
        return -2;
    }
    let n = raw.u32_at(0) as usize;
    for i in 0..n.min(60) {
        let off = 4 + 12 * i;
        if raw.u32_at(off) == luid.low && raw.u32_at(off + 4) as i32 == luid.high {
            return i64::from(raw.u32_at(off + 8));
        }
    }
    -1
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn GetCurrentThread() -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn OpenThreadToken(thread: HANDLE, access: u32, as_self: BOOL, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSidSubAuthority(sid: *const c_void, n: u32) -> *mut u32;
    fn GetSidSubAuthorityCount(sid: *const c_void) -> *mut u8;
    fn EqualSid(a: *const c_void, b: *const c_void) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> BOOL;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut LUID) -> BOOL;
    fn RevertToSelf() -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlImpersonateSelf(level: u32) -> i32;
}

poly_wtest::main!(test);

fn same_info(a: HANDLE, b: HANDLE, class: u32) -> bool {
    let mut ra = Raw::<4096>::new();
    let mut rb = Raw::<4096>::new();
    let (oka, na, _) = token_info(a, class, &mut ra);
    let (okb, nb, _) = token_info(b, class, &mut rb);
    oka && okb && na == nb && ra.b[..4] == rb.b[..4]
}

fn test(t: &mut Test) {
    let process = process_token(TOKEN_QUERY);
    // SAFETY: impersonates the process token on this thread.
    let st = unsafe { RtlImpersonateSelf(SECURITY_IMPERSONATION) };
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: the thread pseudo-handle and a valid out pointer.
    let ok = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &mut h) };
    say!(t, "status={:#010x} thread token opened: {}", st as u32, ok != 0);
    if ok != 0 {
        say!(t, "type: process {} thread {}", tok_u32(process, TOKEN_TYPE_CLASS), tok_u32(h, TOKEN_TYPE_CLASS));
        say!(t, "impersonation level of the thread token: {}", tok_u32(h, TOKEN_IMPERSONATION_LEVEL_CLASS));
        say!(t, "group count equal: {}", same_info(process, h, TOKEN_GROUPS_CLASS));
        say!(t, "privilege count equal: {}", same_info(process, h, TOKEN_PRIVILEGES_CLASS));
        say!(t, "integrity RID: process {} thread {}", integrity_rid(process), integrity_rid(h));
        say!(
            t,
            "session id: process {} thread {}",
            tok_u32(process, TOKEN_SESSION_ID_CLASS),
            tok_u32(h, TOKEN_SESSION_ID_CLASS)
        );
        for name in ["SeChangeNotifyPrivilege", "SeShutdownPrivilege", "SeDebugPrivilege", "SeTcbPrivilege"] {
            say!(t, "{name}: process {} thread {}", privilege_attrs(process, name), privilege_attrs(h, name));
        }
        let mut ra = Raw::<2048>::new();
        let mut rb = Raw::<2048>::new();
        let (oka, _, _) = token_info(process, TOKEN_USER_CLASS, &mut ra);
        let (okb, _, _) = token_info(h, TOKEN_USER_CLASS, &mut rb);
        if oka && okb {
            // SAFETY: both SIDs are inside their result buffers.
            say!(t, "same user SID: {}", unsafe { EqualSid(first_sid(&ra), first_sid(&rb)) });
        }
        // SAFETY: closing the handle.
        unsafe { CloseHandle(h) };
    }
    // SAFETY: ends the impersonation; closing the handle.
    unsafe {
        RevertToSelf();
        CloseHandle(process);
    }
}
