//! SetTokenInformation classes and their rules: virtualization flags work; session id and UI access need a
//! privilege; read-only classes are refused; a wrong length; access rights.
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

const TOKEN_TYPE_CLASS: u32 = 8;
const TOKEN_SESSION_ID_CLASS: u32 = 12;
const TOKEN_SANDBOX_INERT_CLASS: u32 = 15;
const TOKEN_ELEVATION_TYPE_CLASS: u32 = 18;
const TOKEN_HAS_RESTRICTIONS_CLASS: u32 = 21;
const TOKEN_VIRTUALIZATION_ALLOWED_CLASS: u32 = 23;
const TOKEN_VIRTUALIZATION_ENABLED_CLASS: u32 = 24;
const TOKEN_UIACCESS_CLASS: u32 = 26;
const TOKEN_MANDATORY_POLICY_CLASS: u32 = 27;
const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_IMPERSONATION: u32 = 2;
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

/// The process token with `access`, or null.
fn process_token(access: u32) -> HANDLE {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: the current process pseudo-handle and a valid out pointer.
    unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut h) };
    h
}

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
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

/// A 32-bit token information value (class with a single DWORD), or -1 when the query fails.
fn tok_u32(token: HANDLE, class: u32) -> i64 {
    let mut raw = Raw::<64>::new();
    let mut ret = 0u32;
    // SAFETY: `raw` is 64 writable bytes; the length given is the 4 bytes the classes need.
    let ok = unsafe { GetTokenInformation(token, class, raw.p(), 4, &mut ret) };
    if ok != 0 { i64::from(raw.u32_at(0)) } else { -1 }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetTokenInformation(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> BOOL;
    fn SetTokenInformation(token: HANDLE, class: u32, info: *const c_void, len: u32) -> BOOL;
    fn DuplicateTokenEx(
        token: HANDLE,
        access: u32,
        attrs: *const SECURITY_ATTRIBUTES,
        level: u32,
        kind: u32,
        out: *mut HANDLE,
    ) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let token = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    for (name, class, value) in [
        ("TokenVirtualizationEnabled 1", TOKEN_VIRTUALIZATION_ENABLED_CLASS, 1u32),
        ("TokenVirtualizationEnabled 0", TOKEN_VIRTUALIZATION_ENABLED_CLASS, 0),
        ("TokenVirtualizationAllowed 0", TOKEN_VIRTUALIZATION_ALLOWED_CLASS, 0),
        ("TokenSessionId 1", TOKEN_SESSION_ID_CLASS, 1),
        ("TokenUIAccess 1", TOKEN_UIACCESS_CLASS, 1),
        ("TokenMandatoryPolicy 0", TOKEN_MANDATORY_POLICY_CLASS, 0),
        ("TokenMandatoryPolicy 1", TOKEN_MANDATORY_POLICY_CLASS, 1),
        ("TokenSandBoxInert 1", TOKEN_SANDBOX_INERT_CLASS, 1),
        ("TokenType 2", TOKEN_TYPE_CLASS, 2),
        ("TokenElevationType 1", TOKEN_ELEVATION_TYPE_CLASS, 1),
        ("TokenHasRestrictions 1", TOKEN_HAS_RESTRICTIONS_CLASS, 1),
        ("class 0", 0, 1),
        ("class 99", 99, 1),
    ] {
        clear_error();
        // SAFETY: a 4-byte value.
        let ok = unsafe { SetTokenInformation(token, class, (&value as *const u32).cast(), 4) };
        say!(t, "{name}: ok={ok} err={}", gle_if(ok));
    }
    say!(
        t,
        "virtualization enabled now {}, allowed now {}, policy now {}",
        tok_u32(token, TOKEN_VIRTUALIZATION_ENABLED_CLASS),
        tok_u32(token, TOKEN_VIRTUALIZATION_ALLOWED_CLASS),
        tok_u32(token, TOKEN_MANDATORY_POLICY_CLASS)
    );
    for len in [0u32, 1, 3, 8] {
        let value = [1u32, 0];
        clear_error();
        // SAFETY: a buffer of at least `len` bytes (at most 8).
        let ok = unsafe { SetTokenInformation(token, TOKEN_VIRTUALIZATION_ENABLED_CLASS, value.as_ptr().cast(), len) };
        say!(t, "TokenVirtualizationEnabled with length {len}: ok={ok} err={}", gle_if(ok));
    }
    // SAFETY: closing the handle.
    unsafe { CloseHandle(token) };
    for (label, access) in [
        ("QUERY", TOKEN_QUERY),
        ("DUPLICATE", TOKEN_DUPLICATE),
        ("ADJUST_DEFAULT", TOKEN_ADJUST_DEFAULT),
        ("ADJUST_SESSIONID", TOKEN_ADJUST_SESSIONID),
        ("ADJUST_PRIVILEGES", TOKEN_ADJUST_PRIVILEGES),
    ] {
        let tk = dup_token(access, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
        let value = 1u32;
        clear_error();
        // SAFETY: a 4-byte value.
        let ok =
            unsafe { SetTokenInformation(tk, TOKEN_VIRTUALIZATION_ENABLED_CLASS, (&value as *const u32).cast(), 4) };
        say!(t, "TokenVirtualizationEnabled with only {label}: ok={ok} err={}", gle_if(ok));
        // SAFETY: closing the handle.
        unsafe { CloseHandle(tk) };
    }
}
