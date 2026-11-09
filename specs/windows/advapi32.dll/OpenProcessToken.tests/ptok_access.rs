//! OpenProcessToken: the access mask requested and the access the handle is granted (the
//! requested rights; generic rights are mapped; MAXIMUM_ALLOWED gives all token rights).
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
const STANDARD_RIGHTS_READ: u32 = 0x0002_0000;
const STANDARD_RIGHTS_WRITE: u32 = 0x0002_0000;
const STANDARD_RIGHTS_EXECUTE: u32 = 0x0002_0000;
const READ_CONTROL: u32 = 0x0002_0000;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const ACCESS_SYSTEM_SECURITY: u32 = 0x0100_0000;
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

const TOKEN_READ: u32 = STANDARD_RIGHTS_READ | TOKEN_QUERY;
const TOKEN_WRITE: u32 = STANDARD_RIGHTS_WRITE | TOKEN_ADJUST_PRIVILEGES | TOKEN_ADJUST_GROUPS | TOKEN_ADJUST_DEFAULT;
const TOKEN_EXECUTE: u32 = STANDARD_RIGHTS_EXECUTE;
const GENERIC_ALL_RIGHTS: u32 = 0x1000_0000;
/// The last-error code after a failed call, as a plain number.
fn gle() -> u32 {
    // SAFETY: reads the calling thread's last error.
    unsafe { GetLastError() }
}

fn clear_error() {
    // SAFETY: sets the calling thread's last error.
    unsafe { SetLastError(0xDEAD) };
}

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
}

/// winternl.h: `PUBLIC_OBJECT_BASIC_INFORMATION` (class 0 of NtQueryObject).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PUBLIC_OBJECT_BASIC_INFORMATION {
    attributes: u32,
    granted_access: u32,
    handle_count: u32,
    pointer_count: u32,
    reserved: [u32; 10],
}

/// The access mask a handle was opened with (NtQueryObject, ObjectBasicInformation), or 0xFFFF_FFFF.
fn granted_access(h: HANDLE) -> u32 {
    let mut info = PUBLIC_OBJECT_BASIC_INFORMATION::default();
    let mut ret = 0u32;
    // SAFETY: `info` is 56 bytes, as the class requires.
    let st = unsafe { NtQueryObject(h, 0, (&mut info as *mut PUBLIC_OBJECT_BASIC_INFORMATION).cast(), 56, &mut ret) };
    if st < 0 { 0xFFFF_FFFF } else { info.granted_access }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryObject(h: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let wants: [(&str, u32); 14] = [
        ("TOKEN_QUERY", TOKEN_QUERY),
        ("QUERY|DUPLICATE", TOKEN_QUERY | TOKEN_DUPLICATE),
        ("ADJUST_PRIVILEGES|QUERY", TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY),
        ("ASSIGN_PRIMARY", TOKEN_ASSIGN_PRIMARY),
        ("IMPERSONATE", TOKEN_IMPERSONATE),
        ("QUERY_SOURCE", TOKEN_QUERY_SOURCE),
        ("TOKEN_ALL_ACCESS", TOKEN_ALL_ACCESS),
        ("TOKEN_READ", TOKEN_READ),
        ("TOKEN_WRITE", TOKEN_WRITE),
        ("TOKEN_EXECUTE", TOKEN_EXECUTE),
        ("MAXIMUM_ALLOWED", MAXIMUM_ALLOWED),
        ("READ_CONTROL", READ_CONTROL),
        ("GENERIC_ALL", GENERIC_ALL_RIGHTS),
        ("ACCESS_SYSTEM_SECURITY", ACCESS_SYSTEM_SECURITY),
    ];
    for (name, access) in wants {
        let mut h: HANDLE = core::ptr::null_mut();
        clear_error();
        // SAFETY: the current process pseudo-handle and a valid out pointer.
        let ok = unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut h) };
        if ok != 0 {
            say!(t, "{name} ({access:#x}): ok granted={:#x}", granted_access(h));
            // SAFETY: closing the handle just opened.
            unsafe { CloseHandle(h) };
        } else {
            say!(t, "{name} ({access:#x}): FAIL err={}", gle());
        }
    }
    let mut h: HANDLE = core::ptr::null_mut();
    clear_error();
    // SAFETY: valid out pointer; desired access 0.
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), 0, &mut h) };
    say!(t, "access 0: ok={ok} err={}", gle_if(ok));
    if ok != 0 {
        say!(t, "access 0: granted={:#x}", granted_access(h));
        // SAFETY: closing the handle just opened.
        unsafe { CloseHandle(h) };
    }
}
