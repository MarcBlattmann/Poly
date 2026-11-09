//! NtSetSecurityObject owner and group rules for different callers: the thread impersonates a copy of the token that
//! lost its privileges (DISABLE_MAX_PRIVILEGE), one whose Administrators group is deny-only, and both; each tries to
//! make a job (opened with all access before) owned by the user, Administrators, Users, Everyone and SYSTEM and to
//! set its group. (Ambient: the elevated administrator token.)
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

const TOKEN_USER_CLASS: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_IMPERSONATION: u32 = 2;
const DISABLE_MAX_PRIVILEGE: u32 = 0x1;
const ACL_REVISION: u32 = 2;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const OWNER_SECURITY_INFORMATION: u32 = 0x1;
const GROUP_SECURITY_INFORMATION: u32 = 0x2;
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

/// winnt.h: `SID_AND_ATTRIBUTES`.
#[repr(C)]
#[derive(Clone, Copy)]
struct SID_AND_ATTRIBUTES {
    sid: PSID,
    attributes: u32,
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

/// The SID that the first `SID_AND_ATTRIBUTES` of a TOKEN_USER-like result points at.
fn first_sid(raw: &Raw<2048>) -> *const c_void {
    raw.u64_at(0) as usize as *const c_void
}

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

/// NtQueryInformationToken into `buf` with length `len`; (status, returned length).
fn nt_tq<const N: usize>(token: HANDLE, class: u32, buf: &mut Raw<N>, len: u32) -> (i32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    // SAFETY: `buf` has N writable bytes; callers pass len <= N.
    let st = unsafe { NtQueryInformationToken(token, class, buf.p(), len, &mut ret) };
    (st, ret)
}

/// An unnamed job object with all access, or null.
fn nt_job() -> HANDLE {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: valid out pointer; no object attributes.
    unsafe { NtCreateJobObject(&mut h, JOB_OBJECT_ALL_ACCESS, core::ptr::null()) };
    h
}

/// A one-entry TOKEN_GROUPS.
#[repr(C)]
#[derive(Clone, Copy)]
struct NT_GROUPS1 {
    count: u32,
    sa: SID_AND_ATTRIBUTES,
}

fn nt_group1(sid: &SidBuf, attrs: u32) -> NT_GROUPS1 {
    NT_GROUPS1 { count: 1, sa: SID_AND_ATTRIBUTES { sid: sid.as_const() as PSID, attributes: attrs } }
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn LocalFree(p: PVOID) -> PVOID;
    fn GetCurrentProcess() -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn SetThreadToken(thread: *const HANDLE, token: HANDLE) -> BOOL;
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
    fn NtQueryInformationToken(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtDuplicateToken(
        token: HANDLE,
        access: u32,
        attrs: *const NT_OA,
        effective_only: u8,
        kind: u32,
        out: *mut HANDLE,
    ) -> i32;
    fn NtFilterToken(
        token: HANDLE,
        flags: u32,
        disable: *const c_void,
        delete: *const c_void,
        restrict: *const c_void,
        out: *mut HANDLE,
    ) -> i32;
    fn NtSetSecurityObject(h: HANDLE, info: u32, sd: *const c_void) -> i32;
    fn NtCreateJobObject(job: *mut HANDLE, access: u32, attrs: *const NT_OA) -> i32;
}

poly_wtest::main!(test);

fn make(src: HANDLE, flags: u32, disable_admins: bool) -> HANDLE {
    let admins = sid_of("S-1-5-32-544");
    let dis = nt_group1(&admins, 0);
    let mut f: HANDLE = core::ptr::null_mut();
    // SAFETY: valid group set (when given) and out pointer.
    unsafe {
        NtFilterToken(
            src,
            flags,
            if disable_admins { (&dis as *const NT_GROUPS1).cast() } else { core::ptr::null() },
            core::ptr::null(),
            core::ptr::null(),
            &mut f,
        )
    };
    let qos = nt_qos(SECURITY_IMPERSONATION, SECURITY_DYNAMIC_TRACKING, 0);
    let mut oa = nt_oa(core::ptr::null(), 0);
    oa.qos = (&qos as *const NT_QOS).cast();
    let mut d: HANDLE = core::ptr::null_mut();
    // SAFETY: valid attributes and out pointer.
    unsafe { NtDuplicateToken(f, TOKEN_ALL_ACCESS, &oa, 0, TOKEN_IMPERSONATION, &mut d) };
    // SAFETY: closing the intermediate token.
    unsafe { NtClose(f) };
    d
}

fn test(t: &mut Test) {
    let src = process_token(TOKEN_DUPLICATE | TOKEN_QUERY);
    let mut user = Raw::<2048>::new();
    nt_tq(src, TOKEN_USER_CLASS, &mut user, 2048);
    let everyone = sid_of("S-1-1-0");
    let system = sid_of("S-1-5-18");
    let admins = sid_of("S-1-5-32-544");
    let users = sid_of("S-1-5-32-545");
    let targets: [(&str, *const c_void); 5] = [
        ("the user", first_sid(&user)),
        ("Administrators", admins.as_const()),
        ("Users", users.as_const()),
        ("Everyone", everyone.as_const()),
        ("SYSTEM", system.as_const()),
    ];
    let callers: [(&str, HANDLE); 4] = [
        ("unchanged token", make(src, 0, false)),
        ("DISABLE_MAX_PRIVILEGE", make(src, DISABLE_MAX_PRIVILEGE, false)),
        ("Administrators deny-only", make(src, 0, true)),
        ("both", make(src, DISABLE_MAX_PRIVILEGE, true)),
    ];
    for (cname, ct) in callers {
        let job = nt_job();
        // SAFETY: NULL thread is the calling thread; `ct` is an impersonation token.
        let ok = unsafe { SetThreadToken(core::ptr::null(), ct) };
        let mut line = Buf::<300>::new();
        for (n, sid) in targets {
            let mut s = SdBuf::new();
            let mut a = AclBuf::new();
            nt_build_sd(&mut s, &mut a, &[], sid, sid);
            // SAFETY: valid absolute descriptors with an owner / a group.
            let (so, sg) = unsafe {
                (
                    NtSetSecurityObject(job, OWNER_SECURITY_INFORMATION, s.p() as *const c_void),
                    NtSetSecurityObject(job, GROUP_SECURITY_INFORMATION, s.p() as *const c_void),
                )
            };
            let _ = write!(line, "{n}: owner {so:#x} group {sg:#x}; ");
        }
        // SAFETY: ends the impersonation.
        unsafe { SetThreadToken(core::ptr::null(), core::ptr::null_mut()) };
        say!(t, "{cname} (SetThreadToken {ok}): {}", line.as_str());
        // SAFETY: closing the job and the token.
        unsafe {
            NtClose(job);
            NtClose(ct);
        }
    }
    // SAFETY: closing the source.
    unsafe { CloseHandle(src) };
}
