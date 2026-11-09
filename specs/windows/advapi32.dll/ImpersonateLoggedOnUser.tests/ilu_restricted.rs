//! ImpersonateLoggedOnUser with a restricted token (Chrome's pattern: impersonate the lockdown token for
//! a while): the thread token is a restricted token; its group states are the lockdown token's; opening the
//! thread token for TOKEN_QUERY may be refused to the restricted thread.
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
const TRUE: BOOL = 1;
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

const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_IMPERSONATION: u32 = 2;
const DISABLE_MAX_PRIVILEGE: u32 = 0x1;
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

/// winnt.h: `LUID_AND_ATTRIBUTES`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct LUID_AND_ATTRIBUTES {
    luid: LUID,
    attributes: u32,
}

/// winbase.h: `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: PVOID,
    inherit: BOOL,
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

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn LocalFree(p: PVOID) -> PVOID;
    fn GetCurrentProcess() -> HANDLE;
    fn GetCurrentThread() -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn OpenThreadToken(thread: HANDLE, access: u32, as_self: BOOL, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn DuplicateTokenEx(
        token: HANDLE,
        access: u32,
        attrs: *const SECURITY_ATTRIBUTES,
        level: u32,
        kind: u32,
        out: *mut HANDLE,
    ) -> BOOL;
    fn CreateRestrictedToken(
        token: HANDLE,
        flags: u32,
        disable_count: u32,
        disable: *const SID_AND_ATTRIBUTES,
        delete_count: u32,
        delete: *const LUID_AND_ATTRIBUTES,
        restrict_count: u32,
        restrict: *const SID_AND_ATTRIBUTES,
        out: *mut HANDLE,
    ) -> BOOL;
    fn ImpersonateLoggedOnUser(token: HANDLE) -> BOOL;
    fn RevertToSelf() -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let src = process_token(TOKEN_ALL_ACCESS);
    let admins = sid_of("S-1-5-32-544");
    let everyone = sid_of("S-1-1-0");
    let dis = [SID_AND_ATTRIBUTES { sid: admins.as_const() as PSID, attributes: 0 }];
    let res_sids = [SID_AND_ATTRIBUTES { sid: everyone.as_const() as PSID, attributes: 0 }];
    for (label, flags, nd, nr) in [
        ("deny-only administrators", 0u32, 1u32, 0u32),
        ("DISABLE_MAX_PRIVILEGE", DISABLE_MAX_PRIVILEGE, 0, 0),
        ("restricting everyone", 0, 0, 1),
    ] {
        let mut res: HANDLE = core::ptr::null_mut();
        // SAFETY: valid pointers; arrays have one entry when the counts say so.
        let ok = unsafe {
            CreateRestrictedToken(
                src,
                flags,
                nd,
                if nd > 0 { dis.as_ptr() } else { core::ptr::null() },
                0,
                core::ptr::null(),
                nr,
                if nr > 0 { res_sids.as_ptr() } else { core::ptr::null() },
                &mut res,
            )
        };
        let mut imp: HANDLE = core::ptr::null_mut();
        // SAFETY: valid pointers.
        let dok = unsafe {
            DuplicateTokenEx(
                res,
                TOKEN_ALL_ACCESS,
                core::ptr::null(),
                SECURITY_IMPERSONATION,
                TOKEN_IMPERSONATION,
                &mut imp,
            )
        };
        clear_error();
        // SAFETY: valid token handle.
        let iok = unsafe { ImpersonateLoggedOnUser(imp) };
        let mut h: HANDLE = core::ptr::null_mut();
        clear_error();
        // SAFETY: thread pseudo-handle and a valid out pointer.
        let oas = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &mut h) };
        let eas = gle_if(oas);
        if oas != 0 {
            // SAFETY: closing the handle.
            unsafe { CloseHandle(h) };
        }
        clear_error();
        // SAFETY: thread pseudo-handle and a valid out pointer.
        let oth = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, FALSE, &mut h) };
        let eth = gle_if(oth);
        if oth != 0 {
            // SAFETY: closing the handle.
            unsafe { CloseHandle(h) };
        }
        say!(
            t,
            "{label}: created={ok} duplicated={dok} impersonating={iok}; OpenThreadToken as self: ok={oas} err={eas}; as the restricted thread: ok={oth} err={eth}"
        );
        // SAFETY: ends the impersonation and closes the handles.
        unsafe {
            RevertToSelf();
            CloseHandle(imp);
            CloseHandle(res);
        }
    }
    // SAFETY: closing the handle.
    unsafe { CloseHandle(src) };
}
