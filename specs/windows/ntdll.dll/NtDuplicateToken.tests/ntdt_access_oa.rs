//! NtDuplicateToken desired access masks, the rights the source handle needs, and the OBJECT_ATTRIBUTES forms: a
//! NULL pointer, handle attributes (OBJ_INHERIT, OBJ_KERNEL_HANDLE ...), a wrong structure length, an object name, a
//! bad root directory.
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
const STANDARD_RIGHTS_READ: u32 = 0x0002_0000;
const STANDARD_RIGHTS_WRITE: u32 = 0x0002_0000;
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
const TOKEN_TYPE_CLASS: u32 = 8;
const TOKEN_IMPERSONATION_LEVEL_CLASS: u32 = 9;
const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_IMPERSONATION: u32 = 2;
const GENERIC_ALL_RIGHTS: u32 = 0x1000_0000;
const GENERIC_READ_RIGHT: u32 = 0x8000_0000;
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

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 255 units).
fn wide(s: &str) -> [u16; 256] {
    let mut w = [0u16; 256];
    for (i, c) in s.bytes().enumerate().take(255) {
        w[i] = u16::from(c);
    }
    w
}

/// The units of a NUL-terminated UTF-16 string.
fn wide_len(p: *const u16) -> usize {
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
    }
    n
}

/// The process token with `access`, or null.
fn process_token(access: u32) -> HANDLE {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: the current process pseudo-handle and a valid out pointer.
    unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut h) };
    h
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

const OBJ_INHERIT: u32 = 0x2;
const OBJ_PERMANENT: u32 = 0x10;
const OBJ_EXCLUSIVE: u32 = 0x20;
const OBJ_OPENIF: u32 = 0x80;
const OBJ_KERNEL_HANDLE: u32 = 0x200;
const SECURITY_DYNAMIC_TRACKING: u8 = 1;
/// ntdef.h: `OBJECT_ATTRIBUTES` (48 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct NT_OA {
    length: u32,
    root: HANDLE,
    name: *const UnicodeString,
    attributes: u32,
    descriptor: *const c_void,
    qos: *const c_void,
}

/// `InitializeObjectAttributes(&oa, name, attributes, NULL, NULL)`.
fn nt_oa(name: *const UnicodeString, attributes: u32) -> NT_OA {
    NT_OA {
        length: core::mem::size_of::<NT_OA>() as u32,
        root: core::ptr::null_mut(),
        name,
        attributes,
        descriptor: core::ptr::null(),
        qos: core::ptr::null(),
    }
}

/// winnt.h: `SECURITY_QUALITY_OF_SERVICE` (12 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
struct NT_QOS {
    length: u32,
    level: u32,
    tracking: u8,
    effective_only: u8,
}

fn nt_qos(level: u32, tracking: u8, effective_only: u8) -> NT_QOS {
    NT_QOS { length: core::mem::size_of::<NT_QOS>() as u32, level, tracking, effective_only }
}

/// A counted string over a NUL-terminated wide buffer (from `wide`).
fn nt_us(w: &[u16; 256]) -> UnicodeString {
    let n = (wide_len(w.as_ptr()) * 2) as u16;
    UnicodeString { length: n, maximum_length: n + 2, buffer: w.as_ptr() }
}

/// NtQueryInformationToken into `buf` with length `len`; (status, returned length).
fn nt_tq<const N: usize>(token: HANDLE, class: u32, buf: &mut Raw<N>, len: u32) -> (i32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    // SAFETY: `buf` has N writable bytes; callers pass len <= N.
    let st = unsafe { NtQueryInformationToken(token, class, buf.p(), len, &mut ret) };
    (st, ret)
}

/// A one-DWORD token class through NtQueryInformationToken, or -1 when it fails.
fn nt_tok_u32(token: HANDLE, class: u32) -> i64 {
    let mut raw = Raw::<64>::new();
    let (st, _) = nt_tq(token, class, &mut raw, 4);
    if st >= 0 { i64::from(raw.u32_at(0)) } else { -1 }
}

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
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn GetHandleInformation(h: HANDLE, flags: *mut u32) -> BOOL;
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
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryObject(h: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtQueryInformationToken(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtDuplicateToken(
        token: HANDLE,
        access: u32,
        attrs: *const NT_OA,
        effective_only: u8,
        kind: u32,
        out: *mut HANDLE,
    ) -> i32;
}

poly_wtest::main!(test);

fn dup_with(src: HANDLE, access: u32, oa: *const NT_OA, kind: u32) -> (i32, HANDLE) {
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: `oa` is NULL or a valid attributes structure; valid out pointer.
    let st = unsafe { NtDuplicateToken(src, access, oa, 0, kind, &mut out) };
    (st, out)
}

fn test(t: &mut Test) {
    let src = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    let qos = nt_qos(SECURITY_IMPERSONATION, SECURITY_DYNAMIC_TRACKING, 0);
    let mut oa = nt_oa(core::ptr::null(), 0);
    oa.qos = (&qos as *const NT_QOS).cast();
    for (name, access) in [
        ("0", 0u32),
        ("QUERY", TOKEN_QUERY),
        ("ALL_ACCESS", TOKEN_ALL_ACCESS),
        ("MAXIMUM_ALLOWED", MAXIMUM_ALLOWED),
        ("READ", TOKEN_READ),
        ("WRITE", TOKEN_WRITE),
        ("GENERIC_ALL", GENERIC_ALL_RIGHTS),
        ("GENERIC_READ", GENERIC_READ_RIGHT),
        ("ACCESS_SYSTEM_SECURITY", ACCESS_SYSTEM_SECURITY),
        ("undefined 0x200", 0x200),
    ] {
        let (st, out) = dup_with(src, access, &oa, TOKEN_IMPERSONATION);
        if st >= 0 {
            say!(t, "access {name}: status={st:#010x} granted={:#x}", nt_granted(out));
            // SAFETY: closing the duplicate.
            unsafe { NtClose(out) };
        } else {
            say!(t, "access {name}: status={st:#010x}");
        }
    }
    // SAFETY: closing the source.
    unsafe { CloseHandle(src) };
    for (name, access) in [
        ("QUERY only", TOKEN_QUERY),
        ("DUPLICATE only", TOKEN_DUPLICATE),
        ("IMPERSONATE only", TOKEN_IMPERSONATE),
        ("ASSIGN_PRIMARY only", TOKEN_ASSIGN_PRIMARY),
        ("QUERY|DUPLICATE", TOKEN_QUERY | TOKEN_DUPLICATE),
        ("READ_CONTROL", READ_CONTROL),
    ] {
        let s = dup_token(access, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
        let (st, out) = dup_with(s, TOKEN_QUERY, &oa, TOKEN_IMPERSONATION);
        say!(t, "source with {name}, asking TOKEN_QUERY: status={st:#010x}");
        if st >= 0 {
            // SAFETY: closing the duplicate.
            unsafe { NtClose(out) };
        }
        // SAFETY: closing the source.
        unsafe { CloseHandle(s) };
    }
    let src = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    for (name, a) in [
        ("0", 0u32),
        ("OBJ_INHERIT", OBJ_INHERIT),
        ("OBJ_KERNEL_HANDLE", OBJ_KERNEL_HANDLE),
        ("OBJ_PERMANENT", OBJ_PERMANENT),
        ("OBJ_EXCLUSIVE", OBJ_EXCLUSIVE),
        ("OBJ_OPENIF", OBJ_OPENIF),
        ("OBJ_CASE_INSENSITIVE", 0x40),
        ("0x80000000", 0x8000_0000),
    ] {
        let mut o = nt_oa(core::ptr::null(), a);
        o.qos = (&qos as *const NT_QOS).cast();
        let (st, out) = dup_with(src, TOKEN_ALL_ACCESS, &o, TOKEN_IMPERSONATION);
        if st >= 0 {
            let fl = nt_hflags(out);
            say!(t, "attributes {name}: status={st:#010x} handle_flags={fl}");
            if fl & 2 == 0 {
                // SAFETY: closing the duplicate (not close-protected).
                unsafe { NtClose(out) };
            }
        } else {
            say!(t, "attributes {name}: status={st:#010x}");
        }
    }
    for len in [0u32, 8, 40, 47, 48, 49, 56, 100] {
        let mut o = nt_oa(core::ptr::null(), 0);
        o.length = len;
        o.qos = (&qos as *const NT_QOS).cast();
        let (st, out) = dup_with(src, TOKEN_ALL_ACCESS, &o, TOKEN_IMPERSONATION);
        say!(t, "attributes length {len}: status={st:#010x}");
        if st >= 0 {
            // SAFETY: closing the duplicate.
            unsafe { NtClose(out) };
        }
    }
    let (st, out) = dup_with(src, TOKEN_ALL_ACCESS, core::ptr::null(), TOKEN_IMPERSONATION);
    if st >= 0 {
        say!(
            t,
            "NULL attributes, impersonation token: status={st:#010x} level={} type={}",
            nt_tok_u32(out, TOKEN_IMPERSONATION_LEVEL_CLASS),
            nt_tok_u32(out, TOKEN_TYPE_CLASS)
        );
        // SAFETY: closing the duplicate.
        unsafe { NtClose(out) };
    } else {
        say!(t, "NULL attributes, impersonation token: status={st:#010x}");
    }
    let (st, out) = dup_with(src, TOKEN_ALL_ACCESS, core::ptr::null(), TOKEN_PRIMARY);
    if st >= 0 {
        say!(t, "NULL attributes, primary token: status={st:#010x} type={}", nt_tok_u32(out, TOKEN_TYPE_CLASS));
        // SAFETY: closing the duplicate.
        unsafe { NtClose(out) };
    } else {
        say!(t, "NULL attributes, primary token: status={st:#010x}");
    }
    let w = wide("\\BaseNamedObjects\\PolySecNtTokenDup1");
    let us = nt_us(&w);
    for (name, flags) in [("object name", 0x40u32), ("object name, OBJ_OPENIF", 0x40 | OBJ_OPENIF)] {
        let mut o = nt_oa(&us, flags);
        o.qos = (&qos as *const NT_QOS).cast();
        let (st, out) = dup_with(src, TOKEN_ALL_ACCESS, &o, TOKEN_IMPERSONATION);
        say!(t, "{name}: status={st:#010x}");
        if st >= 0 {
            // SAFETY: closing the duplicate (this also removes its name).
            unsafe { NtClose(out) };
        }
    }
    let mut o = nt_oa(core::ptr::null(), 0);
    o.qos = (&qos as *const NT_QOS).cast();
    o.root = 0x7FF4 as HANDLE;
    let (st, out) = dup_with(src, TOKEN_ALL_ACCESS, &o, TOKEN_IMPERSONATION);
    say!(t, "garbage root directory, no name: status={st:#010x}");
    if st >= 0 {
        // SAFETY: closing the duplicate.
        unsafe { NtClose(out) };
    }
    // SAFETY: closing the source.
    unsafe { CloseHandle(src) };
}
