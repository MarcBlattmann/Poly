//! NtOpenThreadTokenEx: HandleAttributes (0, OBJ_INHERIT, OBJ_KERNEL_HANDLE, ... undefined bits) while the thread
//! impersonates, and the status with no thread token.
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

const TOKEN_QUERY: u32 = 0x0008;
const SECURITY_IMPERSONATION: u32 = 2;
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

const OBJ_INHERIT: u32 = 0x2;
const OBJ_PERMANENT: u32 = 0x10;
const OBJ_OPENIF: u32 = 0x80;
const OBJ_KERNEL_HANDLE: u32 = 0x200;
const OBJ_FORCE_ACCESS_CHECK: u32 = 0x400;
/// The handle's flags (GetHandleInformation): bit 0 inherit, bit 1 protect-from-close; -1 when it fails.
fn nt_hflags(h: HANDLE) -> i64 {
    let mut f = 0u32;
    // SAFETY: valid out pointer.
    let ok = unsafe { GetHandleInformation(h, &mut f) };
    if ok != 0 { i64::from(f) } else { -1 }
}

/// The information NtQueryObject gives for a handle's basic information: the granted access, or 0xFFFFFFFF.
fn nt_granted(h: HANDLE) -> u32 {
    granted_access(h)
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetHandleInformation(h: HANDLE, flags: *mut u32) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ImpersonateSelf(level: u32) -> BOOL;
    fn RevertToSelf() -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryObject(h: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtOpenThreadTokenEx(thread: HANDLE, access: u32, open_as_self: u8, attributes: u32, token: *mut HANDLE) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let attrs: [(&str, u32); 9] = [
        ("0", 0),
        ("OBJ_INHERIT", OBJ_INHERIT),
        ("OBJ_CASE_INSENSITIVE", 0x40),
        ("OBJ_KERNEL_HANDLE", OBJ_KERNEL_HANDLE),
        ("OBJ_PERMANENT", OBJ_PERMANENT),
        ("OBJ_OPENIF", OBJ_OPENIF),
        ("OBJ_FORCE_ACCESS_CHECK", OBJ_FORCE_ACCESS_CHECK),
        ("0x4", 0x4),
        ("0x80000000", 0x8000_0000),
    ];
    for phase in 0..2 {
        if phase == 1 {
            // SAFETY: impersonates our own token on this thread.
            unsafe { ImpersonateSelf(SECURITY_IMPERSONATION) };
        }
        for (name, a) in attrs {
            let mut h: HANDLE = core::ptr::null_mut();
            // SAFETY: the current-thread pseudo-handle and a valid out pointer.
            let st = unsafe { NtOpenThreadTokenEx(-2isize as HANDLE, TOKEN_QUERY, 0, a, &mut h) };
            let ph = if phase == 0 { "no thread token" } else { "impersonating" };
            if st >= 0 {
                let fl = nt_hflags(h);
                say!(t, "{ph}, {name}: status={st:#010x} handle_flags={fl} granted={:#x}", nt_granted(h));
                if fl & 2 == 0 {
                    // SAFETY: closing the handle just opened (not close-protected).
                    unsafe { NtClose(h) };
                }
            } else {
                say!(t, "{ph}, {name}: status={st:#010x}");
            }
        }
    }
    // SAFETY: ends the impersonation.
    unsafe { RevertToSelf() };
}
