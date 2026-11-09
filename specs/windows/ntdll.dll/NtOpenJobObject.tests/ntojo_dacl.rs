//! NtOpenJobObject against different DACLs given when the job is created: a NULL DACL, an empty DACL, a deny ACE
//! first, a deny ACE after the allow, an ACE for a SID the token lacks; the access each MAXIMUM_ALLOWED open gets and
//! whether specific rights are refused. (Ambient: administrator token, owner rights.)
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

const SECURITY_MAX_SID_SIZE: usize = 68;
const STANDARD_RIGHTS_REQUIRED: u32 = 0x000F_0000;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const ACL_REVISION: u32 = 2;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const JOB_OBJECT_QUERY: u32 = 0x4;
const JOB_OBJECT_TERMINATE: u32 = 0x8;
const JOB_OBJECT_ALL_ACCESS: u32 = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3F;
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

/// A counted string over a NUL-terminated wide buffer (from `wide`).
fn nt_us(w: &[u16; 256]) -> UnicodeString {
    let n = (wide_len(w.as_ptr()) * 2) as u16;
    UnicodeString { length: n, maximum_length: n + 2, buffer: w.as_ptr() }
}

/// An absolute security descriptor with a DACL of (sid, allow?, mask) entries and optional owner / group.
fn nt_build_sd(
    sd: &mut SdBuf,
    acl: &mut AclBuf,
    aces: &[(&SidBuf, bool, u32)],
    owner: *const c_void,
    group: *const c_void,
) {
    // SAFETY: the buffers are large enough for the entries given; the SIDs are valid and outlive the descriptor.
    unsafe {
        InitializeSecurityDescriptor(sd.p(), SECURITY_DESCRIPTOR_REVISION);
        InitializeAcl(acl.p(), 1024, ACL_REVISION);
        for (sid, allow, mask) in aces {
            if *allow {
                AddAccessAllowedAce(acl.p(), ACL_REVISION, *mask, sid.as_const());
            } else {
                AddAccessDeniedAce(acl.p(), ACL_REVISION, *mask, sid.as_const());
            }
        }
        SetSecurityDescriptorDacl(sd.p(), TRUE, acl.p(), FALSE);
        if !owner.is_null() {
            SetSecurityDescriptorOwner(sd.p(), owner, FALSE);
        }
        if !group.is_null() {
            SetSecurityDescriptorGroup(sd.p(), group, FALSE);
        }
    }
}

/// The information NtQueryObject gives for a handle's basic information: the granted access, or 0xFFFFFFFF.
fn nt_granted(h: HANDLE) -> u32 {
    granted_access(h)
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalFree(p: PVOID) -> PVOID;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn InitializeAcl(acl: PACL, len: u32, revision: u32) -> BOOL;
    fn AddAccessAllowedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAccessDeniedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn InitializeSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, revision: u32) -> BOOL;
    fn SetSecurityDescriptorOwner(sd: PSECURITY_DESCRIPTOR, owner: *const c_void, defaulted: BOOL) -> BOOL;
    fn SetSecurityDescriptorGroup(sd: PSECURITY_DESCRIPTOR, group: *const c_void, defaulted: BOOL) -> BOOL;
    fn SetSecurityDescriptorDacl(sd: PSECURITY_DESCRIPTOR, present: BOOL, dacl: *const c_void, defaulted: BOOL)
    -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryObject(h: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtCreateJobObject(job: *mut HANDLE, access: u32, attrs: *const NT_OA) -> i32;
    fn NtOpenJobObject(job: *mut HANDLE, access: u32, attrs: *const NT_OA) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let w = wide("\\BaseNamedObjects\\PolySecNtOpenB");
    let us = nt_us(&w);
    let everyone = sid_of("S-1-1-0");
    let system = sid_of("S-1-5-18");
    let q = JOB_OBJECT_QUERY;
    let cases: [(&str, &[(&SidBuf, bool, u32)]); 6] = [
        ("empty DACL", &[]),
        ("allow Everyone QUERY", &[(&everyone, true, q)]),
        (
            "deny Everyone TERMINATE, then allow Everyone QUERY|TERMINATE",
            &[(&everyone, false, JOB_OBJECT_TERMINATE), (&everyone, true, q | JOB_OBJECT_TERMINATE)],
        ),
        (
            "allow Everyone QUERY|TERMINATE, then deny Everyone TERMINATE",
            &[(&everyone, true, q | JOB_OBJECT_TERMINATE), (&everyone, false, JOB_OBJECT_TERMINATE)],
        ),
        ("allow SYSTEM only", &[(&system, true, 0x1F_003F)]),
        ("deny Everyone QUERY", &[(&everyone, false, q)]),
    ];
    for (label, aces) in cases {
        let mut sd = SdBuf::new();
        let mut acl = AclBuf::new();
        nt_build_sd(&mut sd, &mut acl, aces, core::ptr::null(), core::ptr::null());
        let mut oa = nt_oa(&us, 0x40);
        oa.descriptor = sd.p() as *const c_void;
        let mut job: HANDLE = core::ptr::null_mut();
        // SAFETY: valid attributes (name and descriptor live across the call) and out pointer.
        let st = unsafe { NtCreateJobObject(&mut job, JOB_OBJECT_ALL_ACCESS, &oa) };
        let plain = nt_oa(&us, 0x40);
        let mut line = Buf::<300>::new();
        for (n, a) in [
            ("QUERY", q),
            ("TERMINATE", JOB_OBJECT_TERMINATE),
            ("QUERY|TERMINATE", q | JOB_OBJECT_TERMINATE),
            ("MAX", MAXIMUM_ALLOWED),
        ] {
            let mut h: HANDLE = core::ptr::null_mut();
            // SAFETY: valid attributes and out pointer.
            let s = unsafe { NtOpenJobObject(&mut h, a, &plain) };
            if s >= 0 {
                let _ = write!(line, "{n}: ok({:#x}) ", nt_granted(h));
                // SAFETY: closing the handle just opened.
                unsafe { NtClose(h) };
            } else {
                let _ = write!(line, "{n}: {s:#x} ");
            }
        }
        say!(t, "{label} (create {st:#010x}): {}", line.as_str());
        // SAFETY: closing the job.
        unsafe { NtClose(job) };
    }
    // No DACL at all (a NULL DACL): everyone gets everything.
    let mut sd = SdBuf::new();
    // SAFETY: an absolute descriptor with a NULL DACL.
    unsafe {
        InitializeSecurityDescriptor(sd.p(), SECURITY_DESCRIPTOR_REVISION);
        SetSecurityDescriptorDacl(sd.p(), TRUE, core::ptr::null(), FALSE);
    }
    let mut oa = nt_oa(&us, 0x40);
    oa.descriptor = sd.p() as *const c_void;
    let mut job: HANDLE = core::ptr::null_mut();
    // SAFETY: valid attributes and out pointer.
    let st = unsafe { NtCreateJobObject(&mut job, JOB_OBJECT_ALL_ACCESS, &oa) };
    let plain = nt_oa(&us, 0x40);
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: valid attributes and out pointer.
    let s = unsafe { NtOpenJobObject(&mut h, MAXIMUM_ALLOWED, &plain) };
    say!(t, "NULL DACL (create {st:#010x}): MAX open {s:#010x} granted={:#x}", if s >= 0 { nt_granted(h) } else { 0 });
    // SAFETY: closing the handles.
    unsafe {
        if s >= 0 {
            NtClose(h);
        }
        NtClose(job);
    }
}
