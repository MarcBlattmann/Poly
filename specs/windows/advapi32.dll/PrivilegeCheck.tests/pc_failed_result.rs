//! PrivilegeCheck: what a call leaves in *result and in the entry's attributes when it fails (the result starts
//! as a marker value), for failures of several kinds, next to calls that succeed with the same markers.
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

const TOKEN_DUPLICATE: u32 = 0x0002;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_ANONYMOUS: u32 = 0;
const SECURITY_IMPERSONATION: u32 = 2;
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

/// winbase.h: `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: PVOID,
    inherit: BOOL,
}

/// winnt.h: `PRIVILEGE_SET` with room for `N` privileges.
#[repr(C)]
#[derive(Clone, Copy)]
struct PRIVILEGE_SET_N<const N: usize> {
    count: u32,
    control: u32,
    privileges: [LUID_AND_ATTRIBUTES; N],
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn CreateEventW(attrs: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DuplicateTokenEx(
        token: HANDLE,
        access: u32,
        attrs: *const SECURITY_ATTRIBUTES,
        level: u32,
        kind: u32,
        out: *mut HANDLE,
    ) -> BOOL;
    fn PrivilegeCheck(token: HANDLE, privileges: *mut c_void, result: *mut BOOL) -> BOOL;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut LUID) -> BOOL;
}

poly_wtest::main!(test);

fn run(t: &mut Test, label: &str, token: HANDLE, name: &str, marker: BOOL) {
    let mut p = PRIVILEGE_SET_N::<1> { count: 1, control: 1, privileges: [LUID_AND_ATTRIBUTES::default()] };
    let w = wide(name);
    // SAFETY: valid name and out LUID.
    unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut p.privileges[0].luid) };
    let mut result: BOOL = marker;
    clear_error();
    // SAFETY: a valid privilege set of one entry and a valid out pointer; the handle is what the case passes.
    let ok = unsafe { PrivilegeCheck(token, (&mut p as *mut PRIVILEGE_SET_N<1>).cast(), &mut result) };
    say!(
        t,
        "{label}: ok={ok} err={} result {marker:#x} -> {result:#x} attributes={:#x}",
        gle_if(ok),
        p.privileges[0].attributes
    );
}

fn test(t: &mut Test) {
    const ENABLED: &str = "SeChangeNotifyPrivilege";
    const NOT_HELD: &str = "SeTcbPrivilege";
    for marker in [0x1234_5678, -1, 1] {
        let query = dup_token(TOKEN_QUERY, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
        run(t, "primary token with TOKEN_QUERY, enabled privilege", query, ENABLED, marker);
        run(t, "primary token with TOKEN_QUERY, privilege not held", query, NOT_HELD, marker);
        // SAFETY: closing the handle.
        unsafe { CloseHandle(query) };
        run(t, "the same handle after CloseHandle", query, ENABLED, marker);
        let adjust = dup_token(TOKEN_ADJUST_PRIVILEGES, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
        run(t, "primary token with only TOKEN_ADJUST_PRIVILEGES", adjust, ENABLED, marker);
        // SAFETY: closing the handle.
        unsafe { CloseHandle(adjust) };
        let anon = dup_token(TOKEN_QUERY, SECURITY_ANONYMOUS, TOKEN_IMPERSONATION);
        run(t, "anonymous-level token with TOKEN_QUERY", anon, ENABLED, marker);
        // SAFETY: closing the handle.
        unsafe { CloseHandle(anon) };
        // SAFETY: an unnamed event, closed below.
        let event = unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) };
        run(t, "an event handle", event, ENABLED, marker);
        // SAFETY: closing the handle.
        unsafe { CloseHandle(event) };
        // SAFETY: the pseudo-handle of the current process.
        run(t, "the process pseudo-handle", unsafe { GetCurrentProcess() }, ENABLED, marker);
        run(t, "a handle value that is not open", 0x7FFF_FFF4usize as HANDLE, ENABLED, marker);
    }
}
