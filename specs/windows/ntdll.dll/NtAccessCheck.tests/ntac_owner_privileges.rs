//! NtAccessCheck implicit rights and privileges: the owner of the descriptor gets READ_CONTROL and WRITE_DAC; an
//! owner SID that is only a group of the token does too; SeTakeOwnershipPrivilege (WRITE_OWNER) and
//! SeSecurityPrivilege (ACCESS_SYSTEM_SECURITY) must be enabled; the privilege set returned and its length rules.
//! (Ambient: the elevated administrator token has those privileges.)
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
const WRITE_DAC: u32 = 0x0004_0000;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
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

const TOKEN_USER_CLASS: u32 = 1;
const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_IMPERSONATION: u32 = 2;
const SE_PRIVILEGE_ENABLED: u32 = 0x2;
const ACL_REVISION: u32 = 2;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
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

/// winnt.h: `SID_AND_ATTRIBUTES`.
#[repr(C)]
#[derive(Clone, Copy)]
struct SID_AND_ATTRIBUTES {
    sid: PSID,
    attributes: u32,
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

/// The SID that the first `SID_AND_ATTRIBUTES` of a TOKEN_USER-like result points at.
fn first_sid(raw: &Raw<2048>) -> *const c_void {
    raw.u64_at(0) as usize as *const c_void
}

/// NtQueryInformationToken into `buf` with length `len`; (status, returned length).
fn nt_tq<const N: usize>(token: HANDLE, class: u32, buf: &mut Raw<N>, len: u32) -> (i32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    // SAFETY: `buf` has N writable bytes; callers pass len <= N.
    let st = unsafe { NtQueryInformationToken(token, class, buf.p(), len, &mut ret) };
    (st, ret)
}

/// A one-entry TOKEN_PRIVILEGES for the named privilege with `attrs`.
fn nt_priv(name: &str, attrs: u32) -> TOKEN_PRIVILEGES_N<1> {
    let mut st = TOKEN_PRIVILEGES_N::<1> { count: 1, privileges: [LUID_AND_ATTRIBUTES::default()] };
    let w = wide(name);
    // SAFETY: valid name and out LUID.
    unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut st.privileges[0].luid) };
    st.privileges[0].attributes = attrs;
    st
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
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut LUID) -> BOOL;
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
    fn NtAdjustPrivilegesToken(
        token: HANDLE,
        disable_all: u8,
        new_state: *const c_void,
        buf_len: u32,
        previous: *mut c_void,
        ret_len: *mut u32,
    ) -> i32;
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

fn run(t: &mut Test, label: &str, sd: *const c_void, token: HANDLE, desired: u32, plen_in: u32) {
    let map = GENERIC_MAPPING { read: 1, write: 2, execute: 4, all: 7 };
    let mut privs =
        PRIVILEGE_SET_N::<4> { count: 0xEE, control: 0xEE, privileges: [LUID_AND_ATTRIBUTES::default(); 4] };
    let mut plen = plen_in;
    let mut granted = 0xAAAA_AAAAu32;
    let mut astat = 0x7777_7777i32;
    // SAFETY: a valid descriptor, token, mapping and out pointers; plen_in is at most the 56-byte buffer.
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
    say!(
        t,
        "{label}, desired {desired:#x}, privilege buffer {plen_in}: status={st:#010x} access_status={astat:#010x} granted={granted:#x} privilege_set: count={} control={:#x} first_attrs={:#x} length_after={plen}",
        privs.count,
        privs.control,
        privs.privileges[0].attributes
    );
}

fn test(t: &mut Test) {
    let token = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_IMPERSONATION);
    let mut user = Raw::<2048>::new();
    nt_tq(token, TOKEN_USER_CLASS, &mut user, 2048);
    let user_sid = first_sid(&user);
    let system = sid_of("S-1-5-18");
    let admins = sid_of("S-1-5-32-544");
    for (label, owner) in [
        ("owner is the user", user_sid),
        ("owner is SYSTEM", system.as_const()),
        ("owner is Administrators", admins.as_const()),
    ] {
        let mut sd = SdBuf::new();
        let mut acl = AclBuf::new();
        nt_build_sd(&mut sd, &mut acl, &[], owner, owner);
        for d in [READ_CONTROL, WRITE_DAC, READ_CONTROL | WRITE_DAC, 0x10000, MAXIMUM_ALLOWED] {
            run(t, label, sd.p() as *const c_void, token, d, 56);
        }
    }
    let mut sd = SdBuf::new();
    let mut acl = AclBuf::new();
    nt_build_sd(&mut sd, &mut acl, &[], system.as_const(), system.as_const());
    for (pname, desired) in [("SeTakeOwnershipPrivilege", WRITE_OWNER), ("SeSecurityPrivilege", ACCESS_SYSTEM_SECURITY)]
    {
        for (state_name, attrs) in [("disabled", 0u32), ("enabled", SE_PRIVILEGE_ENABLED)] {
            let ps = nt_priv(pname, attrs);
            // SAFETY: a valid one-entry privilege set.
            let st = unsafe {
                NtAdjustPrivilegesToken(
                    token,
                    0,
                    (&ps as *const TOKEN_PRIVILEGES_N<1>).cast(),
                    0,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                )
            };
            let mut line = Buf::<100>::new();
            let _ = write!(line, "{pname} {state_name} (adjust {st:#010x})");
            run(t, line.as_str(), sd.p() as *const c_void, token, desired, 56);
            run(t, line.as_str(), sd.p() as *const c_void, token, MAXIMUM_ALLOWED, 56);
        }
    }
    // The privilege-set buffer when a privilege is used (SeSecurityPrivilege is enabled now).
    for plen in [0u32, 4, 8, 20, 32, 55, 56] {
        run(t, "SeSecurityPrivilege enabled", sd.p() as *const c_void, token, ACCESS_SYSTEM_SECURITY, plen);
    }
    // SAFETY: closing the handle.
    unsafe { CloseHandle(token) };
}
