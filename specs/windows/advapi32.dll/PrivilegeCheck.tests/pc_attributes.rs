//! PrivilegeCheck and the Attributes field of each entry: entries that come in with attribute bits already set,
//! for an enabled, a disabled and a not-held privilege, with "all necessary" and "any"; and LUIDs whose number
//! is a privilege's number plus a multiple of 64, or that carry a high part.
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

fn luid_of(name: &str) -> LUID {
    let mut l = LUID::default();
    let w = wide(name);
    // SAFETY: valid name and out LUID.
    unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut l) };
    l
}

fn test(t: &mut Test) {
    let token = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_IMPERSONATION);
    // SeShutdownPrivilege is held and gets disabled; SeTcbPrivilege is not held; SeChangeNotifyPrivilege is enabled.
    let off = TOKEN_PRIVILEGES_N::<1> {
        count: 1,
        privileges: [LUID_AND_ATTRIBUTES { luid: luid_of("SeShutdownPrivilege"), attributes: 0 }],
    };
    // SAFETY: a valid privilege list of one entry with attributes 0.
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
    let luids = [luid_of("SeChangeNotifyPrivilege"), luid_of("SeShutdownPrivilege"), luid_of("SeTcbPrivilege")];
    for control in [1u32, 0] {
        for init in [0u32, 1, 2, 3, 4, 0x8000_0000, 0x8000_0002, 0x7FFF_FFFF, 0xFFFF_FFFF] {
            let mut p = PRIVILEGE_SET_N::<3> { count: 3, control, privileges: [LUID_AND_ATTRIBUTES::default(); 3] };
            for (i, l) in luids.iter().enumerate() {
                p.privileges[i] = LUID_AND_ATTRIBUTES { luid: *l, attributes: init };
            }
            let mut result: BOOL = 0x55;
            // SAFETY: a valid privilege set of three entries.
            let ok = unsafe { PrivilegeCheck(token, (&mut p as *mut PRIVILEGE_SET_N<3>).cast(), &mut result) };
            say!(
                t,
                "control {control}, attributes {init:#x} on [enabled, disabled, not held]: ok={ok} err={} result={result:#x} after: {:#x} {:#x} {:#x}",
                gle_if(ok),
                p.privileges[0].attributes,
                p.privileges[1].attributes,
                p.privileges[2].attributes
            );
        }
    }
    // The count and control fields are not changed.
    let mut p = PRIVILEGE_SET_N::<1> {
        count: 1,
        control: 1,
        privileges: [LUID_AND_ATTRIBUTES { luid: luids[0], attributes: 0x10 }],
    };
    let mut result: BOOL = 0x55;
    // SAFETY: a valid privilege set of one entry.
    let ok = unsafe { PrivilegeCheck(token, (&mut p as *mut PRIVILEGE_SET_N<1>).cast(), &mut result) };
    say!(
        t,
        "one enabled entry with attributes 0x10: ok={ok} result={result:#x} count={} control={} after: {:#x}",
        p.count,
        p.control,
        p.privileges[0].attributes
    );
    // LUIDs that are not the plain number of a privilege.
    for (what, base) in [("enabled", luids[0]), ("disabled", luids[1]), ("not held", luids[2])] {
        let mut line = Buf::<300>::new();
        for (add, high) in [(0u32, 0i32), (64, 0), (128, 0), (0x1_0000, 0), (0x8000_0000, 0), (0, 7), (0, -1), (64, 3)]
        {
            let mut p = PRIVILEGE_SET_N::<1> {
                count: 1,
                control: 1,
                privileges: [LUID_AND_ATTRIBUTES {
                    luid: LUID { low: base.low.wrapping_add(add), high },
                    attributes: 0,
                }],
            };
            let mut result: BOOL = 0x55;
            // SAFETY: a valid privilege set of one entry.
            let ok = unsafe { PrivilegeCheck(token, (&mut p as *mut PRIVILEGE_SET_N<1>).cast(), &mut result) };
            let _ = write!(line, "+{add:#x}/high {high}: {ok}/{result:#x}/{:#x} ", p.privileges[0].attributes);
        }
        say!(t, "the {what} privilege's number plus n / with a high part: ok/result/attributes: {}", line.as_str());
    }
    // SAFETY: closing the handle.
    unsafe { CloseHandle(token) };
}
