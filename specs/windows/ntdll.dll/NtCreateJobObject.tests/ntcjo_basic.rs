//! NtCreateJobObject without a name: desired access masks and the granted access, handle attributes, a NULL
//! attributes pointer, a wrong attributes length, the object's type name.
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

const STANDARD_RIGHTS_REQUIRED: u32 = 0x000F_0000;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const ACCESS_SYSTEM_SECURITY: u32 = 0x0100_0000;
const GENERIC_ALL_RIGHTS: u32 = 0x1000_0000;
const GENERIC_READ_RIGHT: u32 = 0x8000_0000;
const GENERIC_WRITE_RIGHT: u32 = 0x4000_0000;
const GENERIC_EXECUTE_RIGHT: u32 = 0x2000_0000;
const JOB_OBJECT_ASSIGN_PROCESS: u32 = 0x1;
const JOB_OBJECT_QUERY: u32 = 0x4;
const JOB_OBJECT_ALL_ACCESS: u32 = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3F;
const JobObjectBasicAccountingInformation: u32 = 1;
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
const OBJ_EXCLUSIVE: u32 = 0x20;
const OBJ_OPENIF: u32 = 0x80;
const OBJ_KERNEL_HANDLE: u32 = 0x200;
const OBJ_FORCE_ACCESS_CHECK: u32 = 0x400;
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

/// The handle's type name (NtQueryObject, ObjectTypeInformation, whose first field is the name).
fn nt_type_name(h: HANDLE) -> Buf<64> {
    let mut raw = Raw::<512>::new();
    let mut ret = 0u32;
    let mut out = Buf::<64>::new();
    // SAFETY: 512 writable bytes, as the class needs.
    let st = unsafe { NtQueryObject(h, 2, raw.p(), 512, &mut ret) };
    if st < 0 {
        let _ = write!(out, "<error {:#010x}>", st);
        return out;
    }
    let len = usize::from(raw.u16_at(0)) / 2;
    let p = raw.u64_at(8) as usize as *const u16;
    for i in 0..len.min(30) {
        // SAFETY: the name lies inside `raw` (len units from the pointer in the structure).
        let c = unsafe { *p.add(i) };
        let _ = write!(out, "{}", if c < 0x80 { c as u8 as char } else { '?' });
    }
    out
}

/// The handle's flags (GetHandleInformation): bit 0 inherit, bit 1 protect-from-close; -1 when it fails.
fn nt_hflags(h: HANDLE) -> i64 {
    let mut f = 0u32;
    // SAFETY: valid out pointer.
    let ok = unsafe { GetHandleInformation(h, &mut f) };
    if ok != 0 { i64::from(f) } else { -1 }
}

/// An unnamed job object with all access, or null.
fn nt_job() -> HANDLE {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: valid out pointer; no object attributes.
    unsafe { NtCreateJobObject(&mut h, JOB_OBJECT_ALL_ACCESS, core::ptr::null()) };
    h
}

/// The information NtQueryObject gives for a handle's basic information: the granted access, or 0xFFFFFFFF.
fn nt_granted(h: HANDLE) -> u32 {
    granted_access(h)
}

/// NtQueryInformationJobObject into a buffer; (status, returned length).
fn nt_jq<const N: usize>(job: HANDLE, class: u32, buf: &mut Raw<N>, len: u32) -> (i32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    // SAFETY: `buf` has N writable bytes; callers pass len <= N.
    let st = unsafe { NtQueryInformationJobObject(job, class, buf.p(), len, &mut ret) };
    (st, ret)
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetHandleInformation(h: HANDLE, flags: *mut u32) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryObject(h: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtCreateJobObject(job: *mut HANDLE, access: u32, attrs: *const NT_OA) -> i32;
    fn NtQueryInformationJobObject(job: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let masks: [(&str, u32); 14] = [
        ("0", 0),
        ("JOB_OBJECT_QUERY", JOB_OBJECT_QUERY),
        ("JOB_OBJECT_ASSIGN_PROCESS", JOB_OBJECT_ASSIGN_PROCESS),
        ("JOB_OBJECT_ALL_ACCESS", JOB_OBJECT_ALL_ACCESS),
        ("MAXIMUM_ALLOWED", MAXIMUM_ALLOWED),
        ("GENERIC_ALL", GENERIC_ALL_RIGHTS),
        ("GENERIC_READ", GENERIC_READ_RIGHT),
        ("GENERIC_WRITE", GENERIC_WRITE_RIGHT),
        ("GENERIC_EXECUTE", GENERIC_EXECUTE_RIGHT),
        ("SYNCHRONIZE", SYNCHRONIZE),
        ("DELETE", 0x10000),
        ("ACCESS_SYSTEM_SECURITY", ACCESS_SYSTEM_SECURITY),
        ("undefined 0x20", 0x20),
        ("undefined 0x8000", 0x8000),
    ];
    for (name, access) in masks {
        let mut h: HANDLE = 0x1234 as HANDLE;
        // SAFETY: valid out pointer, no attributes.
        let st = unsafe { NtCreateJobObject(&mut h, access, core::ptr::null()) };
        if st >= 0 {
            say!(
                t,
                "access {name}: status={st:#010x} granted={:#x} type={} handle_flags={}",
                nt_granted(h),
                nt_type_name(h).as_str(),
                nt_hflags(h)
            );
            // SAFETY: closing the new job.
            unsafe { NtClose(h) };
        } else {
            say!(t, "access {name}: status={st:#010x} handle_unchanged={}", h as usize == 0x1234);
        }
    }
    for (name, a) in [
        ("0", 0u32),
        ("OBJ_INHERIT", OBJ_INHERIT),
        ("OBJ_CASE_INSENSITIVE", 0x40),
        ("OBJ_KERNEL_HANDLE", OBJ_KERNEL_HANDLE),
        ("OBJ_PERMANENT", OBJ_PERMANENT),
        ("OBJ_EXCLUSIVE", OBJ_EXCLUSIVE),
        ("OBJ_OPENIF", OBJ_OPENIF),
        ("OBJ_FORCE_ACCESS_CHECK", OBJ_FORCE_ACCESS_CHECK),
        ("0x80000000", 0x8000_0000),
    ] {
        let oa = nt_oa(core::ptr::null(), a);
        let mut h: HANDLE = core::ptr::null_mut();
        // SAFETY: valid attributes and out pointer.
        let st = unsafe { NtCreateJobObject(&mut h, JOB_OBJECT_ALL_ACCESS, &oa) };
        if st >= 0 {
            let fl = nt_hflags(h);
            say!(t, "attributes {name}: status={st:#010x} handle_flags={fl}");
            if fl & 2 == 0 {
                // SAFETY: closing the new job.
                unsafe { NtClose(h) };
            }
        } else {
            say!(t, "attributes {name}: status={st:#010x}");
        }
    }
    for len in [0u32, 8, 40, 47, 48, 49, 56] {
        let mut oa = nt_oa(core::ptr::null(), 0);
        oa.length = len;
        let mut h: HANDLE = core::ptr::null_mut();
        // SAFETY: valid attributes and out pointer.
        let st = unsafe { NtCreateJobObject(&mut h, JOB_OBJECT_ALL_ACCESS, &oa) };
        say!(t, "attributes length {len}: status={st:#010x}");
        if st >= 0 {
            // SAFETY: closing the new job.
            unsafe { NtClose(h) };
        }
    }
    let a = nt_job();
    let b = nt_job();
    let mut ra = Raw::<64>::new();
    let mut rb = Raw::<64>::new();
    nt_jq(a, JobObjectBasicAccountingInformation, &mut ra, 48);
    nt_jq(b, JobObjectBasicAccountingInformation, &mut rb, 48);
    say!(
        t,
        "two unnamed jobs: handles differ={} fresh accounting total_processes={} active={}",
        a != b,
        ra.u32_at(36),
        rb.u32_at(40)
    );
    // SAFETY: closing the jobs.
    unsafe {
        NtClose(a);
        NtClose(b);
    }
}
