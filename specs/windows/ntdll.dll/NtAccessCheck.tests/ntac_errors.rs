//! NtAccessCheck argument errors: a primary token, impersonation tokens of each level, token handles with too few
//! rights, descriptors without owner or group or with a bad revision, desired access with unmapped generic bits, NULL
//! and wrong-type token handles.
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
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
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
const SECURITY_ANONYMOUS: u32 = 0;
const SECURITY_IDENTIFICATION: u32 = 1;
const SECURITY_IMPERSONATION: u32 = 2;
const SECURITY_DELEGATION: u32 = 3;
const ACL_REVISION: u32 = 2;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const GENERIC_ALL_RIGHTS: u32 = 0x1000_0000;
const GENERIC_READ_RIGHT: u32 = 0x8000_0000;
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

/// winnt.h: `GENERIC_MAPPING`.
#[repr(C)]
#[derive(Clone, Copy)]
struct GENERIC_MAPPING {
    read: u32,
    write: u32,
    execute: u32,
    all: u32,
}

/// winnt.h: `PRIVILEGE_SET` with room for `N` privileges.
#[repr(C)]
#[derive(Clone, Copy)]
struct PRIVILEGE_SET_N<const N: usize> {
    count: u32,
    control: u32,
    privileges: [LUID_AND_ATTRIBUTES; N],
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
    fn CreateEventW(attrs: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn DuplicateTokenEx(
        token: HANDLE,
        access: u32,
        attrs: *const SECURITY_ATTRIBUTES,
        level: u32,
        kind: u32,
        out: *mut HANDLE,
    ) -> BOOL;
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
    fn NtAccessCheck(
        sd: *const c_void,
        token: HANDLE,
        desired: u32,
        mapping: *const GENERIC_MAPPING,
        privs: *mut c_void,
        priv_len: *mut u32,
        granted: *mut u32,
        access_status: *mut i32,
    ) -> i32;
}

poly_wtest::main!(test);

fn run(t: &mut Test, label: &str, sd: *const c_void, token: HANDLE, desired: u32) {
    let map = GENERIC_MAPPING { read: 1, write: 2, execute: 4, all: 7 };
    let mut privs = PRIVILEGE_SET_N::<4> { count: 0, control: 0, privileges: [LUID_AND_ATTRIBUTES::default(); 4] };
    let mut plen = core::mem::size_of::<PRIVILEGE_SET_N<4>>() as u32;
    let mut granted = 0xAAAA_AAAAu32;
    let mut astat = 0x7777_7777i32;
    // SAFETY: a valid descriptor, a token (possibly of the wrong kind), mapping and out pointers.
    let st = unsafe {
        NtAccessCheck(
            sd,
            token,
            desired,
            &map,
            (&mut privs as *mut PRIVILEGE_SET_N<4>).cast(),
            &mut plen,
            &mut granted,
            &mut astat,
        )
    };
    say!(t, "{label}: status={st:#010x} access_status={astat:#010x} granted={granted:#x}");
}

fn test(t: &mut Test) {
    let everyone = sid_of("S-1-1-0");
    let system = sid_of("S-1-5-18");
    let mut sd = SdBuf::new();
    let mut acl = AclBuf::new();
    nt_build_sd(&mut sd, &mut acl, &[(&everyone, true, 7)], system.as_const(), system.as_const());
    let good = sd.p() as *const c_void;
    let primary = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    run(t, "primary token", good, primary, 1);
    for (label, level) in [
        ("anonymous", SECURITY_ANONYMOUS),
        ("identification", SECURITY_IDENTIFICATION),
        ("impersonation", SECURITY_IMPERSONATION),
        ("delegation", SECURITY_DELEGATION),
    ] {
        let imp = dup_token(TOKEN_ALL_ACCESS, level, TOKEN_IMPERSONATION);
        run(t, label, good, imp, 1);
        // SAFETY: closing the handle.
        unsafe { CloseHandle(imp) };
    }
    for (label, access) in [
        ("only TOKEN_QUERY", TOKEN_QUERY),
        ("only TOKEN_DUPLICATE", TOKEN_DUPLICATE),
        ("only TOKEN_IMPERSONATE", TOKEN_IMPERSONATE),
        ("TOKEN_QUERY|TOKEN_IMPERSONATE", TOKEN_QUERY | TOKEN_IMPERSONATE),
    ] {
        let imp = dup_token(access, SECURITY_IMPERSONATION, TOKEN_IMPERSONATION);
        run(t, label, good, imp, 1);
        // SAFETY: closing the handle.
        unsafe { CloseHandle(imp) };
    }
    let token = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_IMPERSONATION);
    // Descriptors with parts missing.
    let mut no_owner = SdBuf::new();
    let mut a2 = AclBuf::new();
    nt_build_sd(&mut no_owner, &mut a2, &[(&everyone, true, 7)], core::ptr::null(), system.as_const());
    run(t, "no owner", no_owner.p() as *const c_void, token, 1);
    let mut no_group = SdBuf::new();
    let mut a3 = AclBuf::new();
    nt_build_sd(&mut no_group, &mut a3, &[(&everyone, true, 7)], system.as_const(), core::ptr::null());
    run(t, "no group", no_group.p() as *const c_void, token, 1);
    let mut bare = SdBuf::new();
    // SAFETY: an absolute descriptor in a large buffer.
    unsafe { InitializeSecurityDescriptor(bare.p(), SECURITY_DESCRIPTOR_REVISION) };
    run(t, "empty descriptor", bare.p() as *const c_void, token, 1);
    let mut bad_rev = SdBuf::new();
    let mut a4 = AclBuf::new();
    nt_build_sd(&mut bad_rev, &mut a4, &[(&everyone, true, 7)], system.as_const(), system.as_const());
    bad_rev.b[0] = 9;
    run(t, "revision 9", bad_rev.p() as *const c_void, token, 1);
    // Desired access with generic or undefined bits.
    for (label, d) in [
        ("GENERIC_READ", GENERIC_READ_RIGHT),
        ("GENERIC_ALL", GENERIC_ALL_RIGHTS),
        ("MAXIMUM_ALLOWED|1", MAXIMUM_ALLOWED | 1),
        ("0x8", 8),
        ("0xFFFF", 0xFFFF),
    ] {
        run(t, label, good, token, d);
    }
    // Token handles of the wrong kind.
    run(t, "NULL handle", good, core::ptr::null_mut(), 1);
    run(t, "process pseudo-handle", good, NT_CURRENT_PROCESS, 1);
    // SAFETY: an unnamed event, closed after use.
    let ev = unsafe { CreateEventW(core::ptr::null(), TRUE, FALSE, core::ptr::null()) };
    run(t, "event handle", good, ev, 1);
    // SAFETY: closing the event and the tokens.
    unsafe {
        CloseHandle(ev);
        CloseHandle(token);
        CloseHandle(primary);
    }
}
