//! AccessCheck and mandatory integrity labels: a Low or Medium impersonation token against a descriptor whose
//! SACL has a label ACE (NO_WRITE_UP / NO_READ_UP / NO_EXECUTE_UP at Low, Medium, High) or none, with a DACL
//! granting everything to Everyone. Write rights (the mapping's write bits, DELETE, WRITE_DAC, WRITE_OWNER) are
//! removed for a token below the label. (ambient: the token starts High)
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

const SECURITY_MANDATORY_LABEL_AUTHORITY: [u8; 6] = [0, 0, 0, 0, 0, 16];
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
const TOKEN_INTEGRITY_LEVEL_CLASS: u32 = 25;
const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_IMPERSONATION: u32 = 2;
const SE_GROUP_INTEGRITY: u32 = 0x20;
const ACL_REVISION: u32 = 2;
const SYSTEM_MANDATORY_LABEL_NO_WRITE_UP: u32 = 0x1;
const SYSTEM_MANDATORY_LABEL_NO_READ_UP: u32 = 0x2;
const SYSTEM_MANDATORY_LABEL_NO_EXECUTE_UP: u32 = 0x4;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const SECURITY_MANDATORY_UNTRUSTED_RID: u32 = 0x0000;
const SECURITY_MANDATORY_LOW_RID: u32 = 0x1000;
const SECURITY_MANDATORY_MEDIUM_RID: u32 = 0x2000;
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

/// A UTF-16 string pointer as text (ASCII only; others as `?`).
fn wide_text<const N: usize>(p: *const u16) -> Buf<N> {
    let mut b = Buf::<N>::new();
    if p.is_null() {
        let _ = write!(b, "(null)");
        return b;
    }
    let n = wide_len(p);
    for i in 0..n {
        // SAFETY: i < n, inside the string.
        let c = unsafe { *p.add(i) };
        let _ = write!(b, "{}", if c < 0x80 { c as u8 as char } else { '?' });
    }
    b
}

/// The SID in the text form of `ConvertSidToStringSidW`, or an error marker.
fn sid_text(sid: *const c_void) -> Buf<200> {
    let mut out: *mut u16 = core::ptr::null_mut();
    // SAFETY: `sid` is a valid SID or the call fails; `out` is a valid out pointer.
    let ok = unsafe { ConvertSidToStringSidW(sid, &mut out) };
    if ok == 0 {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "<invalid sid>");
        return b;
    }
    let b = wide_text::<200>(out);
    // SAFETY: the string was allocated by the call with LocalAlloc.
    unsafe { LocalFree(out.cast()) };
    b
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

/// `GetTokenInformation` into `buf`; (success, needed length, last error).
fn token_info<const N: usize>(token: HANDLE, class: u32, buf: &mut Raw<N>) -> (bool, u32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    clear_error();
    // SAFETY: `buf` is N writable bytes, `ret` a valid out pointer.
    let ok = unsafe { GetTokenInformation(token, class, buf.p(), N as u32, &mut ret) } != 0;
    (ok, ret, gle())
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

/// The SID that the first `SID_AND_ATTRIBUTES` of a TOKEN_USER-like result points at.
fn first_sid(raw: &Raw<2048>) -> *const c_void {
    raw.u64_at(0) as usize as *const c_void
}

/// The token's integrity RID (TokenIntegrityLevel), or -1.
fn integrity_rid(token: HANDLE) -> i64 {
    let mut raw = Raw::<2048>::new();
    let (ok, _, _) = token_info(token, TOKEN_INTEGRITY_LEVEL_CLASS, &mut raw);
    if !ok {
        return -1;
    }
    let sid = first_sid(&raw);
    // SAFETY: the SID is inside `raw`.
    unsafe {
        let n = *GetSidSubAuthorityCount(sid);
        i64::from(*GetSidSubAuthority(sid, u32::from(n) - 1))
    }
}

/// `say!` with a longer line (SDDL texts, hex dumps).
macro_rules! sdsay {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<2000>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// Builds an ACL into `acl` from (ace type, ace flags, mask, sid text). Types: 0 allowed, 1 denied,
/// 2 audit (success and failure), 0x11 mandatory label (the mask is the label policy).
fn sd_build_acl(acl: &mut AclBuf, aces: &[(u8, u8, u32, &str)]) -> bool {
    acl.b.fill(0);
    let mut size = 8usize;
    for &(_, _, _, sid) in aces {
        size += 8 + sid_of(sid).bytes().len();
    }
    size = (size + 3) & !3;
    // SAFETY: `acl` is 1024 writable bytes and `size` is the room the ACEs need.
    if unsafe { InitializeAcl(acl.p(), size as u32, ACL_REVISION) } == 0 {
        return false;
    }
    for &(ty, flags, mask, sid) in aces {
        let s = sid_of(sid);
        // SAFETY: the ACL is initialized and has room; `s` is a valid SID.
        let ok = unsafe {
            match ty {
                0 => AddAccessAllowedAceEx(acl.p(), ACL_REVISION, u32::from(flags), mask, s.as_const()),
                1 => AddAccessDeniedAceEx(acl.p(), ACL_REVISION, u32::from(flags), mask, s.as_const()),
                2 => AddAuditAccessAceEx(acl.p(), ACL_REVISION, u32::from(flags), mask, s.as_const(), TRUE, TRUE),
                0x11 => AddMandatoryAce(acl.p(), ACL_REVISION, u32::from(flags), mask, s.as_const()),
                _ => 0,
            }
        };
        if ok == 0 {
            return false;
        }
    }
    true
}

/// An absolute security descriptor with its own SID and ACL storage (do not move it after the
/// setters ran: the descriptor holds pointers into it).
struct SdAbs {
    sd: Raw<64>,
    owner: SidBuf,
    group: SidBuf,
    dacl: AclBuf,
    sacl: AclBuf,
}

impl SdAbs {
    fn new() -> SdAbs {
        let mut a = SdAbs {
            sd: Raw::new(),
            owner: SidBuf::new(),
            group: SidBuf::new(),
            dacl: AclBuf::new(),
            sacl: AclBuf::new(),
        };
        // SAFETY: the buffer is 64 bytes, larger than the 40-byte descriptor.
        unsafe { InitializeSecurityDescriptor(a.sd.p(), SECURITY_DESCRIPTOR_REVISION) };
        a
    }
    fn p(&mut self) -> PSECURITY_DESCRIPTOR {
        self.sd.p()
    }
    fn cp(&self) -> *const c_void {
        self.sd.b.as_ptr().cast()
    }
    fn set_owner(&mut self, sid: &str, defaulted: bool) -> bool {
        self.owner = sid_of(sid);
        // SAFETY: the SID lives in `self` as long as the descriptor.
        unsafe { SetSecurityDescriptorOwner(self.sd.p(), self.owner.as_const(), defaulted as BOOL) != 0 }
    }
    fn set_group(&mut self, sid: &str, defaulted: bool) -> bool {
        self.group = sid_of(sid);
        // SAFETY: the SID lives in `self` as long as the descriptor.
        unsafe { SetSecurityDescriptorGroup(self.sd.p(), self.group.as_const(), defaulted as BOOL) != 0 }
    }
    fn set_dacl(&mut self, aces: &[(u8, u8, u32, &str)]) -> bool {
        if !sd_build_acl(&mut self.dacl, aces) {
            return false;
        }
        // SAFETY: the ACL lives in `self` as long as the descriptor.
        unsafe { SetSecurityDescriptorDacl(self.sd.p(), TRUE, self.dacl.p(), FALSE) != 0 }
    }
    fn null_dacl(&mut self) -> bool {
        // SAFETY: a NULL DACL is allowed.
        unsafe { SetSecurityDescriptorDacl(self.sd.p(), TRUE, core::ptr::null(), FALSE) != 0 }
    }
    fn set_sacl(&mut self, aces: &[(u8, u8, u32, &str)]) -> bool {
        if !sd_build_acl(&mut self.sacl, aces) {
            return false;
        }
        // SAFETY: the ACL lives in `self` as long as the descriptor.
        unsafe { SetSecurityDescriptorSacl(self.sd.p(), TRUE, self.sacl.p(), FALSE) != 0 }
    }
}

/// The user SID of a token (copied).
fn sd_token_user(token: HANDLE) -> SidBuf {
    let mut raw = Raw::<2048>::new();
    token_info(token, TOKEN_USER_CLASS, &mut raw);
    let mut out = SidBuf::new();
    // SAFETY: the SID is inside `raw`; the copy fits.
    unsafe { CopySid(SECURITY_MAX_SID_SIZE as u32, out.as_psid(), first_sid(&raw)) };
    out
}

/// An impersonation token (level impersonation) duplicated from the process token with all its
/// privileges disabled, or null.
fn sd_imp_token() -> HANDLE {
    let t = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_IMPERSONATION);
    // SAFETY: disables every privilege of the copy (no new state).
    unsafe { AdjustTokenPrivileges(t, TRUE, core::ptr::null(), 0, core::ptr::null_mut(), core::ptr::null_mut()) };
    t
}

/// Sets the integrity level of a token (TOKEN_MANDATORY_LABEL with the SID S-1-16-<rid>).
fn sd_set_integrity(token: HANDLE, rid: u32) -> bool {
    let mut sid = SidBuf::new();
    // SAFETY: the buffer is larger than a SID with one sub-authority.
    unsafe {
        InitializeSid(sid.as_psid(), &SECURITY_MANDATORY_LABEL_AUTHORITY, 1);
        *GetSidSubAuthority(sid.as_psid(), 0) = rid;
    }
    let tml = SID_AND_ATTRIBUTES { sid: sid.as_psid(), attributes: SE_GROUP_INTEGRITY };
    // SAFETY: TOKEN_MANDATORY_LABEL is one SID_AND_ATTRIBUTES.
    unsafe {
        SetTokenInformation(token, TOKEN_INTEGRITY_LEVEL_CLASS, (&tml as *const SID_AND_ATTRIBUTES).cast(), 16 + 8) != 0
    }
}

/// The result of an AccessCheck call: `last` is the last error right after the call (printed only when
/// the access was denied), `privs` the returned PRIVILEGE_SET.
struct SdAc {
    ok: i32,
    granted: u32,
    status: BOOL,
    err: i64,
    last: u32,
    priv_len: u32,
    priv_count: u32,
    privs: Raw<256>,
}

fn sd_access_len(sd: *const c_void, token: HANDLE, desired: u32, map: &GENERIC_MAPPING, priv_buf_len: u32) -> SdAc {
    let mut privs = Raw::<256>::new();
    let mut plen = priv_buf_len;
    let mut granted = 0xAAAA_AAAAu32;
    let mut status: BOOL = 0x77;
    clear_error();
    // SAFETY: the privilege buffer is 256 bytes and `plen` never exceeds that; valid out pointers.
    let ok = unsafe { AccessCheck(sd, token, desired, map, privs.p(), &mut plen, &mut granted, &mut status) };
    let last = gle();
    SdAc { ok, granted, status, err: gle_if(ok), last, priv_len: plen, priv_count: privs.u32_at(0), privs }
}

fn sd_access(sd: *const c_void, token: HANDLE, desired: u32, map: &GENERIC_MAPPING) -> SdAc {
    sd_access_len(sd, token, desired, map, 256)
}

/// A mapping without standard rights: read 0x1, write 0x2, execute 0x4, all 0x1F0007.
fn sd_plain_mapping() -> GENERIC_MAPPING {
    GENERIC_MAPPING { read: 0x1, write: 0x2, execute: 0x4, all: 0x001F_0007 }
}

/// A hand-written ACL (revision 2): the ACEs one after the other, each with its size field set to 8 + the
/// SID's length, without any validation (types and flags as given).
fn sd_raw_acl(acl: &mut AclBuf, aces: &[(u8, u8, u32, &str)]) {
    acl.b.fill(0);
    acl.b[0] = 2;
    let mut off = 8usize;
    for &(ty, flags, mask, sid) in aces {
        let s = sid_of(sid);
        let sb = s.bytes();
        let size = 8 + sb.len();
        acl.b[off] = ty;
        acl.b[off + 1] = flags;
        acl.b[off + 2..off + 4].copy_from_slice(&(size as u16).to_le_bytes());
        acl.b[off + 4..off + 8].copy_from_slice(&mask.to_le_bytes());
        acl.b[off + 8..off + size].copy_from_slice(sb);
        off += size;
    }
    acl.b[2..4].copy_from_slice(&(off as u16).to_le_bytes());
    acl.b[4..6].copy_from_slice(&(aces.len() as u16).to_le_bytes());
}

impl SdAbs {
    /// Sets a hand-written DACL (see `sd_raw_acl`).
    fn set_dacl_raw(&mut self, aces: &[(u8, u8, u32, &str)]) -> bool {
        sd_raw_acl(&mut self.dacl, aces);
        // SAFETY: the ACL lives in `self` as long as the descriptor.
        unsafe { SetSecurityDescriptorDacl(self.sd.p(), TRUE, self.dacl.p(), FALSE) != 0 }
    }
    /// Sets a hand-written SACL (see `sd_raw_acl`).
    fn set_sacl_raw(&mut self, aces: &[(u8, u8, u32, &str)]) -> bool {
        sd_raw_acl(&mut self.sacl, aces);
        // SAFETY: the ACL lives in `self` as long as the descriptor.
        unsafe { SetSecurityDescriptorSacl(self.sd.p(), TRUE, self.sacl.p(), FALSE) != 0 }
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn LocalFree(p: PVOID) -> PVOID;
    fn GetCurrentProcess() -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn InitializeSid(sid: PSID, authority: *const [u8; 6], count: u8) -> BOOL;
    fn GetSidSubAuthority(sid: *const c_void, n: u32) -> *mut u32;
    fn GetSidSubAuthorityCount(sid: *const c_void) -> *mut u8;
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn CopySid(len: u32, dst: PSID, src: *const c_void) -> BOOL;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> BOOL;
    fn SetTokenInformation(token: HANDLE, class: u32, info: *const c_void, len: u32) -> BOOL;
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
    fn InitializeAcl(acl: PACL, len: u32, revision: u32) -> BOOL;
    fn AddAccessAllowedAceEx(acl: PACL, revision: u32, flags: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAccessDeniedAceEx(acl: PACL, revision: u32, flags: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAuditAccessAceEx(
        acl: PACL,
        revision: u32,
        flags: u32,
        mask: u32,
        sid: *const c_void,
        success: BOOL,
        failure: BOOL,
    ) -> BOOL;
    fn AddMandatoryAce(acl: PACL, revision: u32, flags: u32, policy: u32, sid: *const c_void) -> BOOL;
    fn InitializeSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, revision: u32) -> BOOL;
    fn SetSecurityDescriptorOwner(sd: PSECURITY_DESCRIPTOR, owner: *const c_void, defaulted: BOOL) -> BOOL;
    fn SetSecurityDescriptorGroup(sd: PSECURITY_DESCRIPTOR, group: *const c_void, defaulted: BOOL) -> BOOL;
    fn SetSecurityDescriptorDacl(sd: PSECURITY_DESCRIPTOR, present: BOOL, dacl: *const c_void, defaulted: BOOL)
    -> BOOL;
    fn SetSecurityDescriptorSacl(sd: PSECURITY_DESCRIPTOR, present: BOOL, sacl: *const c_void, defaulted: BOOL)
    -> BOOL;
    fn AccessCheck(
        sd: *const c_void,
        token: HANDLE,
        desired: u32,
        mapping: *const GENERIC_MAPPING,
        privs: *mut c_void,
        priv_len: *mut u32,
        granted: *mut u32,
        status: *mut BOOL,
    ) -> BOOL;
}

poly_wtest::main!(test);

fn row(t: &mut Test, tok: HANDLE, label: &str, owner: &str, sacl: &[(u8, u8, u32, &str)]) {
    let mut a = SdAbs::new();
    a.set_owner(owner, false);
    a.set_group("S-1-5-18", false);
    a.set_dacl(&[(0, 0, 0x1F0007, "S-1-1-0")]);
    if !sacl.is_empty() {
        a.set_sacl(sacl);
    }
    let map = sd_plain_mapping();
    let mut line = Buf::<400>::new();
    let _ = write!(line, "{label}:");
    for (name, bit) in [
        ("r", 0x1u32),
        ("w", 0x2),
        ("x", 0x4),
        ("del", DELETE),
        ("rc", READ_CONTROL),
        ("wdac", WRITE_DAC),
        ("wown", WRITE_OWNER),
    ] {
        let r = sd_access(a.cp(), tok, bit, &map);
        let _ = write!(line, " {name}={}", if r.ok != 0 && r.status != 0 { 1 } else { 0 });
    }
    let r = sd_access(a.cp(), tok, MAXIMUM_ALLOWED, &map);
    let _ = write!(line, " max={:#x}/{}", r.granted, r.status);
    t.line(line.as_str());
}

fn test(t: &mut Test) {
    let labels: [(&str, &[(u8, u8, u32, &str)]); 11] = [
        ("no label", &[]),
        ("ML NW Low", &[(0x11, 0, SYSTEM_MANDATORY_LABEL_NO_WRITE_UP, "S-1-16-4096")]),
        ("ML NW Medium", &[(0x11, 0, SYSTEM_MANDATORY_LABEL_NO_WRITE_UP, "S-1-16-8192")]),
        ("ML NW High", &[(0x11, 0, SYSTEM_MANDATORY_LABEL_NO_WRITE_UP, "S-1-16-12288")]),
        ("ML NW System", &[(0x11, 0, SYSTEM_MANDATORY_LABEL_NO_WRITE_UP, "S-1-16-16384")]),
        ("ML NR High", &[(0x11, 0, SYSTEM_MANDATORY_LABEL_NO_READ_UP, "S-1-16-12288")]),
        ("ML NX High", &[(0x11, 0, SYSTEM_MANDATORY_LABEL_NO_EXECUTE_UP, "S-1-16-12288")]),
        ("ML NW|NR|NX High", &[(0x11, 0, 7, "S-1-16-12288")]),
        ("ML policy 0 High", &[(0x11, 0, 0, "S-1-16-12288")]),
        ("ML NW|NR|NX Medium", &[(0x11, 0, 7, "S-1-16-8192")]),
        ("ML NW Untrusted", &[(0x11, 0, SYSTEM_MANDATORY_LABEL_NO_WRITE_UP, "S-1-16-0")]),
    ];
    for (level_name, rid) in [
        ("High (unchanged)", 0u32),
        ("Medium", SECURITY_MANDATORY_MEDIUM_RID),
        ("Low", SECURITY_MANDATORY_LOW_RID),
        ("Untrusted", SECURITY_MANDATORY_UNTRUSTED_RID),
    ] {
        let tok = sd_imp_token();
        if rid != 0 || level_name.starts_with("Untrusted") {
            let ok = sd_set_integrity(tok, rid);
            sdsay!(t, "== token lowered to {level_name}: ok={ok} integrity RID now {:#x}", integrity_rid(tok));
        } else {
            sdsay!(t, "== token {level_name}: integrity RID {:#x}", integrity_rid(tok));
        }
        for (name, sacl) in labels {
            row(t, tok, name, "S-1-5-18", sacl);
        }
        let user = sd_token_user(tok);
        let user_text = sid_text(user.as_const());
        row(t, tok, "no label, owner = the user", user_text.as_str(), &[]);
        row(
            t,
            tok,
            "ML NW High, owner = the user",
            user_text.as_str(),
            &[(0x11, 0, SYSTEM_MANDATORY_LABEL_NO_WRITE_UP, "S-1-16-12288")],
        );
        // SAFETY: closing the handle.
        unsafe { CloseHandle(tok) };
    }
}
