//! PrivilegeCheck on an impersonation token: all-necessary and any-necessary over held/enabled, held/disabled and
//! not-held privileges; the result and the UsedForAccess attribute it sets.
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

const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
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
    fn AdjustTokenPrivileges(
        token: HANDLE,
        disable_all: BOOL,
        new_state: *const c_void,
        buf_len: u32,
        previous: *mut c_void,
        ret_len: *mut u32,
    ) -> BOOL;
    fn PrivilegeCheck(token: HANDLE, privileges: *mut c_void, result: *mut BOOL) -> BOOL;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut LUID) -> BOOL;
}

poly_wtest::main!(test);

const PRIVILEGE_SET_ALL_NECESSARY: u32 = 1;

fn set_for(names: &[&str], control: u32) -> PRIVILEGE_SET_N<3> {
    let mut p =
        PRIVILEGE_SET_N::<3> { count: names.len() as u32, control, privileges: [LUID_AND_ATTRIBUTES::default(); 3] };
    for (i, n) in names.iter().enumerate() {
        let w = wide(n);
        // SAFETY: valid name and out LUID.
        unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut p.privileges[i].luid) };
    }
    p
}

fn check(token: HANDLE, names: &[&str], control: u32) -> (i32, BOOL, u32, u32, u32) {
    let mut p = set_for(names, control);
    let mut result: BOOL = 0x77;
    clear_error();
    // SAFETY: a valid privilege set with the entries counted.
    let ok = unsafe { PrivilegeCheck(token, (&mut p as *mut PRIVILEGE_SET_N<3>).cast(), &mut result) };
    (ok, result, gle(), p.privileges[0].attributes, p.privileges[1].attributes)
}

fn test(t: &mut Test) {
    let token = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_IMPERSONATION);
    // Disable SeShutdownPrivilege (held, disabled); SeTcbPrivilege is not held; SeChangeNotifyPrivilege is enabled.
    let off = {
        let mut st = TOKEN_PRIVILEGES_N::<1> { count: 1, privileges: [LUID_AND_ATTRIBUTES::default()] };
        let w = wide("SeShutdownPrivilege");
        // SAFETY: valid name and out LUID.
        unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut st.privileges[0].luid) };
        st
    };
    // SAFETY: a valid privilege set of one entry with attributes 0.
    unsafe {
        AdjustTokenPrivileges(
            token,
            FALSE,
            (&off as *const TOKEN_PRIVILEGES_N<1>).cast(),
            0,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };
    for (label, names, control) in [
        ("enabled, all", &["SeChangeNotifyPrivilege"][..], PRIVILEGE_SET_ALL_NECESSARY),
        ("disabled, all", &["SeShutdownPrivilege"][..], PRIVILEGE_SET_ALL_NECESSARY),
        ("not held, all", &["SeTcbPrivilege"][..], PRIVILEGE_SET_ALL_NECESSARY),
        (
            "enabled + disabled, all",
            &["SeChangeNotifyPrivilege", "SeShutdownPrivilege"][..],
            PRIVILEGE_SET_ALL_NECESSARY,
        ),
        ("enabled + disabled, any", &["SeChangeNotifyPrivilege", "SeShutdownPrivilege"][..], 0),
        ("disabled + not held, any", &["SeShutdownPrivilege", "SeTcbPrivilege"][..], 0),
        ("not held + enabled, any", &["SeTcbPrivilege", "SeChangeNotifyPrivilege"][..], 0),
        ("no entries, all", &[][..], PRIVILEGE_SET_ALL_NECESSARY),
        ("no entries, any", &[][..], 0),
    ] {
        let (ok, result, err, a0, a1) = check(token, names, control);
        say!(
            t,
            "{label}: ok={ok} result={result} err={} attrs after: {a0:#x} {a1:#x}",
            if ok != 0 { -1 } else { i64::from(err) }
        );
    }
    // SAFETY: closing the handle.
    unsafe { CloseHandle(token) };
}
