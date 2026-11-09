//! AdjustTokenPrivileges argument rules: previous-state buffer too small, no previous state, a
//! count of 0, and handles without TOKEN_ADJUST_PRIVILEGES (and without TOKEN_QUERY when the previous state is asked for).
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
const FALSE: BOOL = 0;
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

const TOKEN_PRIVILEGES_CLASS: u32 = 3;
const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_IMPERSONATION: u32 = 2;
const SE_PRIVILEGE_ENABLED: u32 = 0x2;
/// winnt.h: `LUID`.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct LUID {
    low: u32,
    high: i32,
}

/// winnt.h: `LUID_AND_ATTRIBUTES`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct LUID_AND_ATTRIBUTES {
    luid: LUID,
    attributes: u32,
}

/// winnt.h: `TOKEN_PRIVILEGES` with room for `N` privileges.
#[repr(C)]
#[derive(Clone, Copy)]
struct TOKEN_PRIVILEGES_N<const N: usize> {
    count: u32,
    privileges: [LUID_AND_ATTRIBUTES; N],
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
    fn DuplicateTokenEx(
        token: HANDLE,
        access: u32,
        attrs: *const SECURITY_ATTRIBUTES,
        level: u32,
        kind: u32,
        out: *mut HANDLE,
    ) -> BOOL;
    fn AdjustTokenPrivileges(
        token: HANDLE,
        disable_all: BOOL,
        new_state: *const c_void,
        buf_len: u32,
        previous: *mut c_void,
        ret_len: *mut u32,
    ) -> BOOL;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut LUID) -> BOOL;
}

poly_wtest::main!(test);

fn one(name: &str, attrs: u32) -> TOKEN_PRIVILEGES_N<1> {
    let mut st = TOKEN_PRIVILEGES_N::<1> { count: 1, privileges: [LUID_AND_ATTRIBUTES::default()] };
    let w = wide(name);
    // SAFETY: valid name and out LUID.
    unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut st.privileges[0].luid) };
    st.privileges[0].attributes = attrs;
    st
}

fn test(t: &mut Test) {
    let token = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    let st = one("SeShutdownPrivilege", 0);
    for size in [0u32, 1, 15, 16, 17, 100] {
        let mut prev = Raw::<256>::new();
        prev.b.fill(0xEE);
        let mut ret = 0xAAAA_AAAAu32;
        clear_error();
        // SAFETY: valid privilege set; `size` is at most 100 of the 256 bytes.
        let ok = unsafe {
            AdjustTokenPrivileges(token, FALSE, (&st as *const TOKEN_PRIVILEGES_N<1>).cast(), size, prev.p(), &mut ret)
        };
        say!(
            t,
            "previous buffer {size}: ok={ok} err={} returned={ret} written={} attrs_now={}",
            gle(),
            prev.b.iter().filter(|&&b| b != 0xEE).count(),
            privilege_attrs(token, "SeShutdownPrivilege")
        );
        let en = one("SeShutdownPrivilege", SE_PRIVILEGE_ENABLED);
        // SAFETY: valid privilege set; no previous state asked for.
        unsafe {
            AdjustTokenPrivileges(
                token,
                FALSE,
                (&en as *const TOKEN_PRIVILEGES_N<1>).cast(),
                0,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };
    }
    let mut ret = 0xAAAA_AAAAu32;
    clear_error();
    // SAFETY: valid privilege set; no previous buffer but a length pointer.
    let ok = unsafe {
        AdjustTokenPrivileges(
            token,
            FALSE,
            (&st as *const TOKEN_PRIVILEGES_N<1>).cast(),
            0,
            core::ptr::null_mut(),
            &mut ret,
        )
    };
    say!(t, "no previous buffer, length pointer: ok={ok} err={} returned={ret}", gle_if(ok));
    let zero = TOKEN_PRIVILEGES_N::<1> { count: 0, privileges: [LUID_AND_ATTRIBUTES::default()] };
    clear_error();
    // SAFETY: valid privilege set of zero entries.
    let ok = unsafe {
        AdjustTokenPrivileges(
            token,
            FALSE,
            (&zero as *const TOKEN_PRIVILEGES_N<1>).cast(),
            0,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };
    say!(t, "count 0: ok={ok} err={}", gle_if(ok));
    // SAFETY: closing the handle.
    unsafe { CloseHandle(token) };
    for (label, access) in [
        ("ADJUST_PRIVILEGES only", TOKEN_ADJUST_PRIVILEGES),
        ("QUERY only", TOKEN_QUERY),
        ("ADJUST_PRIVILEGES|QUERY", TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY),
    ] {
        let tk = dup_token(access, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
        for (what, with_prev) in [("without previous state", false), ("with previous state", true)] {
            let mut prev = Raw::<256>::new();
            let mut ret = 0u32;
            clear_error();
            // SAFETY: valid privilege set and previous buffer.
            let ok = unsafe {
                AdjustTokenPrivileges(
                    tk,
                    FALSE,
                    (&st as *const TOKEN_PRIVILEGES_N<1>).cast(),
                    if with_prev { 256 } else { 0 },
                    if with_prev { prev.p() } else { core::ptr::null_mut() },
                    if with_prev { &mut ret } else { core::ptr::null_mut() },
                )
            };
            say!(t, "{label}, {what}: ok={ok} err={}", gle_if(ok));
        }
        // SAFETY: closing the handle.
        unsafe { CloseHandle(tk) };
    }
}
