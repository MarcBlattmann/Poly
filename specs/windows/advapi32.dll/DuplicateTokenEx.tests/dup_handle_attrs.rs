//! DuplicateTokenEx security attributes: bInheritHandle makes the new handle inheritable; a
//! NULL attributes pointer does not; with an explicit security descriptor the call still works.
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
type PACL = *mut c_void;
type PSECURITY_DESCRIPTOR = *mut c_void;
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

const TOKEN_DUPLICATE: u32 = 0x0002;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_TYPE_CLASS: u32 = 8;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_IMPERSONATION: u32 = 2;
const ACL_REVISION: u32 = 2;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const HANDLE_FLAG_INHERIT: u32 = 1;
/// winbase.h: `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: PVOID,
    inherit: BOOL,
}

/// A `SECURITY_DESCRIPTOR` (absolute, 40 bytes on x64) in a buffer.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct SdBuf {
    b: [u8; 4096],
}

impl SdBuf {
    const fn new() -> SdBuf {
        SdBuf { b: [0; 4096] }
    }
    fn p(&mut self) -> PSECURITY_DESCRIPTOR {
        self.b.as_mut_ptr().cast()
    }
}

/// An ACL in an 8-aligned buffer.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct AclBuf {
    b: [u8; 1024],
}

impl AclBuf {
    const fn new() -> AclBuf {
        AclBuf { b: [0; 1024] }
    }
    fn p(&mut self) -> PACL {
        self.b.as_mut_ptr().cast()
    }
    /// The ACL's bytes by its own size field.
    fn bytes(&self) -> &[u8] {
        let n = usize::from(u16::from_le_bytes([self.b[2], self.b[3]]));
        &self.b[..n.min(self.b.len())]
    }
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

/// `GetTokenInformation` into `buf`; (success, needed length, last error).
fn token_info<const N: usize>(token: HANDLE, class: u32, buf: &mut Raw<N>) -> (bool, u32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    clear_error();
    // SAFETY: `buf` is N writable bytes, `ret` a valid out pointer.
    let ok = unsafe { GetTokenInformation(token, class, buf.p(), N as u32, &mut ret) } != 0;
    (ok, ret, gle())
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
    fn GetHandleInformation(h: HANDLE, flags: *mut u32) -> BOOL;
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
    fn InitializeAcl(acl: PACL, len: u32, revision: u32) -> BOOL;
    fn InitializeSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, revision: u32) -> BOOL;
    fn SetSecurityDescriptorDacl(sd: PSECURITY_DESCRIPTOR, present: BOOL, dacl: *const c_void, defaulted: BOOL)
    -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryObject(h: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let src = process_token(TOKEN_DUPLICATE | TOKEN_QUERY);
    for (label, inherit) in [("null attributes", None), ("inherit FALSE", Some(FALSE)), ("inherit TRUE", Some(TRUE))] {
        let sa = SECURITY_ATTRIBUTES {
            length: core::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            descriptor: core::ptr::null_mut(),
            inherit: inherit.unwrap_or(0),
        };
        let mut out: HANDLE = core::ptr::null_mut();
        // SAFETY: valid source token and out pointer; the attributes live across the call.
        let ok = unsafe {
            DuplicateTokenEx(
                src,
                0,
                if inherit.is_some() { &sa as *const SECURITY_ATTRIBUTES } else { core::ptr::null() },
                SECURITY_IMPERSONATION,
                TOKEN_IMPERSONATION,
                &mut out,
            )
        };
        let mut flags = 0u32;
        // SAFETY: valid handle and out pointer.
        unsafe { GetHandleInformation(out, &mut flags) };
        say!(t, "{label}: ok={ok} inheritable={}", flags & HANDLE_FLAG_INHERIT);
        // SAFETY: closing the duplicate.
        unsafe { CloseHandle(out) };
    }
    // An absolute security descriptor with an empty DACL on the new token object.
    let mut sd = SdBuf::new();
    let mut acl = AclBuf::new();
    // SAFETY: the buffers are large enough and live across the calls.
    unsafe {
        InitializeSecurityDescriptor(sd.p(), SECURITY_DESCRIPTOR_REVISION);
        InitializeAcl(acl.p(), 1024, ACL_REVISION);
        SetSecurityDescriptorDacl(sd.p(), TRUE, acl.p(), FALSE);
    }
    let sa = SECURITY_ATTRIBUTES {
        length: core::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        descriptor: sd.p(),
        inherit: FALSE,
    };
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: valid pointers.
    let ok = unsafe { DuplicateTokenEx(src, 0, &sa, SECURITY_IMPERSONATION, TOKEN_IMPERSONATION, &mut out) };
    say!(t, "empty DACL on the new token: ok={ok} granted={:#x}", granted_access(out));
    let mut raw = Raw::<2048>::new();
    let (qok, _, qerr) = token_info(out, TOKEN_TYPE_CLASS, &mut raw);
    say!(t, "query through the handle: ok={qok} type={} err={}", raw.u32_at(0), qerr);
    // SAFETY: closing the handles.
    unsafe {
        CloseHandle(out);
        CloseHandle(src);
    }
}
