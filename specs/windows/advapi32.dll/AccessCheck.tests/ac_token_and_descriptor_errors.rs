//! AccessCheck failures: a primary token, impersonation levels, a token without TOKEN_QUERY, a NULL/invalid
//! handle, descriptors without owner or group, a damaged revision, a self-relative descriptor from SDDL.
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
const INVALID_HANDLE: HANDLE = -1isize as HANDLE;
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

const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_ANONYMOUS: u32 = 0;
const SECURITY_IDENTIFICATION: u32 = 1;
const SECURITY_IMPERSONATION: u32 = 2;
const ACL_REVISION: u32 = 2;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const SDDL_REVISION_1: u32 = 1;
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

/// A pointer stored in a result buffer, as an offset from the buffer start
/// (so printed values do not depend on addresses); -1 for null, -2 outside.
fn rel(base: *const u8, len: usize, p: *const c_void) -> i64 {
    if p.is_null() {
        return -1;
    }
    let d = (p as usize).wrapping_sub(base as usize);
    if d < len { d as i64 } else { -2 }
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

/// `say!` with a longer line (SDDL texts, hex dumps).
macro_rules! sdsay {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<2000>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// A made-up mapping: read 0x20001, write 0x20002, execute 0x20004, all 0xF0007.
fn sd_simple_mapping() -> GENERIC_MAPPING {
    GENERIC_MAPPING { read: 0x0002_0001, write: 0x0002_0002, execute: 0x0002_0004, all: 0x000F_0007 }
}

/// A self-relative descriptor made by ConvertStringSecurityDescriptorToSecurityDescriptorW.
struct SdLocal {
    p: PSECURITY_DESCRIPTOR,
    size: u32,
    ok: i32,
    err: i64,
}

impl SdLocal {
    fn ptr(&self) -> *const c_void {
        self.p as *const c_void
    }
    fn bytes(&self) -> &[u8] {
        if self.p.is_null() {
            return &[];
        }
        // SAFETY: the allocation is `size` bytes as reported by the conversion.
        unsafe { core::slice::from_raw_parts(self.p as *const u8, self.size as usize) }
    }
}

impl Drop for SdLocal {
    fn drop(&mut self) {
        if !self.p.is_null() {
            // SAFETY: allocated with LocalAlloc by the conversion.
            unsafe { LocalFree(self.p) };
        }
    }
}

fn sd_from_rev(s: &str, rev: u32) -> SdLocal {
    let w = wide(s);
    let mut p: PSECURITY_DESCRIPTOR = core::ptr::null_mut();
    let mut size = 0xAAAA_AAAAu32;
    clear_error();
    // SAFETY: `w` is NUL-terminated; both out pointers are valid.
    let ok = unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(w.as_ptr(), rev, &mut p, &mut size) };
    let err = gle_if(ok);
    if ok == 0 {
        p = core::ptr::null_mut();
    }
    SdLocal { p, size, ok, err }
}

fn sd_from(s: &str) -> SdLocal {
    sd_from_rev(s, SDDL_REVISION_1)
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

/// An impersonation token (level impersonation) duplicated from the process token with all its
/// privileges disabled, or null.
fn sd_imp_token() -> HANDLE {
    let t = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_IMPERSONATION);
    // SAFETY: disables every privilege of the copy (no new state).
    unsafe { AdjustTokenPrivileges(t, TRUE, core::ptr::null(), 0, core::ptr::null_mut(), core::ptr::null_mut()) };
    t
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

/// One line for an AccessCheck result; after a denial it adds the last error.
fn sd_ac_say(t: &mut Test, label: &str, r: &SdAc) {
    if r.ok != 0 && r.status != 0 {
        sdsay!(
            t,
            "{label}: ok=1 granted={:#x} status={} privlen={} privcount={}",
            r.granted,
            r.status,
            r.priv_len,
            r.priv_count
        );
    } else if r.ok != 0 {
        sdsay!(
            t,
            "{label}: ok=1 granted={:#x} status={} lasterr={} privlen={} privcount={}",
            r.granted,
            r.status,
            r.last,
            r.priv_len,
            r.priv_count
        );
    } else {
        sdsay!(t, "{label}: ok=0 err={} granted={:#x} status={:#x} privlen={}", r.err, r.granted, r.status, r.priv_len);
    }
}

/// A unique object name "PolySecSd<pid>_<n>" (UTF-16, NUL-terminated).
fn sd_unique_name() -> [u16; 256] {
    static COUNTER: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let mut text = Buf::<80>::new();
    // SAFETY: reads the process id.
    let _ = write!(text, "PolySecSd{}_{}", unsafe { GetCurrentProcessId() }, n);
    wide(text.as_str())
}

/// A new named manual-reset event (unique name) with the default security of the caller's token.
fn sd_event() -> HANDLE {
    let name = sd_unique_name();
    // SAFETY: no attributes, NUL-terminated name.
    unsafe { CreateEventW(core::ptr::null(), TRUE, FALSE, name.as_ptr()) }
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
    fn GetCurrentProcessId() -> u32;
    fn CreateEventW(attrs: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        s: *const u16,
        revision: u32,
        sd: *mut PSECURITY_DESCRIPTOR,
        size: *mut u32,
    ) -> BOOL;
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

fn token_at(level: u32) -> HANDLE {
    let src = process_token(TOKEN_DUPLICATE | TOKEN_QUERY);
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: valid source token and out pointer.
    unsafe {
        DuplicateTokenEx(src, TOKEN_ALL_ACCESS, core::ptr::null(), level, TOKEN_IMPERSONATION, &mut out);
        CloseHandle(src);
    }
    out
}

fn test(t: &mut Test) {
    let map = sd_simple_mapping();
    let mut a = SdAbs::new();
    a.set_owner("S-1-5-18", false);
    a.set_group("S-1-5-18", false);
    a.set_dacl(&[(0, 0, 0x7, "S-1-1-0")]);
    let primary = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    sd_ac_say(t, "primary token", &sd_access(a.cp(), primary, 0x1, &map));
    let real_primary = process_token(TOKEN_ALL_ACCESS);
    sd_ac_say(t, "the process token itself", &sd_access(a.cp(), real_primary, 0x1, &map));
    for (name, level) in [
        ("anonymous", SECURITY_ANONYMOUS),
        ("identification", SECURITY_IDENTIFICATION),
        ("impersonation", SECURITY_IMPERSONATION),
    ] {
        let tok = token_at(level);
        let mut label = Buf::<60>::new();
        let _ = write!(label, "{name} level token (handle {})", if tok.is_null() { "not created" } else { "created" });
        if tok.is_null() {
            sdsay!(t, "{}", label.as_str());
        } else {
            sd_ac_say(t, label.as_str(), &sd_access(a.cp(), tok, 0x1, &map));
            // SAFETY: closing the handle.
            unsafe { CloseHandle(tok) };
        }
    }
    // A token handle without TOKEN_QUERY.
    let src = process_token(TOKEN_DUPLICATE | TOKEN_QUERY);
    let mut noquery: HANDLE = core::ptr::null_mut();
    // SAFETY: valid source token and out pointer.
    unsafe {
        DuplicateTokenEx(
            src,
            TOKEN_IMPERSONATE,
            core::ptr::null(),
            SECURITY_IMPERSONATION,
            TOKEN_IMPERSONATION,
            &mut noquery,
        )
    };
    sd_ac_say(t, "token with TOKEN_IMPERSONATE only", &sd_access(a.cp(), noquery, 0x1, &map));
    sd_ac_say(t, "NULL handle", &sd_access(a.cp(), core::ptr::null_mut(), 0x1, &map));
    sd_ac_say(t, "handle -1", &sd_access(a.cp(), INVALID_HANDLE, 0x1, &map));
    sd_ac_say(t, "handle 0x1234 (not a handle)", &sd_access(a.cp(), 0x1234 as HANDLE, 0x1, &map));
    let ev = sd_event();
    sd_ac_say(t, "an event handle", &sd_access(a.cp(), ev, 0x1, &map));
    let tok = sd_imp_token();
    // Descriptor shapes
    let mut no_owner = SdAbs::new();
    no_owner.set_group("S-1-5-18", false);
    no_owner.set_dacl(&[(0, 0, 0x7, "S-1-1-0")]);
    sd_ac_say(t, "descriptor without owner", &sd_access(no_owner.cp(), tok, 0x1, &map));
    let mut no_group = SdAbs::new();
    no_group.set_owner("S-1-5-18", false);
    no_group.set_dacl(&[(0, 0, 0x7, "S-1-1-0")]);
    sd_ac_say(t, "descriptor without group", &sd_access(no_group.cp(), tok, 0x1, &map));
    let mut neither = SdAbs::new();
    neither.set_dacl(&[(0, 0, 0x7, "S-1-1-0")]);
    sd_ac_say(t, "descriptor with neither", &sd_access(neither.cp(), tok, 0x1, &map));
    let mut bad_rev = SdAbs::new();
    bad_rev.set_owner("S-1-5-18", false);
    bad_rev.set_group("S-1-5-18", false);
    bad_rev.set_dacl(&[(0, 0, 0x7, "S-1-1-0")]);
    bad_rev.sd.b[0] = 2;
    sd_ac_say(t, "descriptor revision 2", &sd_access(bad_rev.cp(), tok, 0x1, &map));
    let rel = sd_from("O:SYG:SYD:(A;;0x7;;;WD)");
    sd_ac_say(t, "self-relative descriptor from SDDL, desired 0x1", &sd_access(rel.ptr(), tok, 0x1, &map));
    sd_ac_say(t, "self-relative descriptor from SDDL, desired 0x8", &sd_access(rel.ptr(), tok, 0x8, &map));
    let rel_noowner = sd_from("G:SYD:(A;;0x7;;;WD)");
    sd_ac_say(t, "self-relative descriptor without owner", &sd_access(rel_noowner.ptr(), tok, 0x1, &map));
    let empty = sd_from("");
    sd_ac_say(t, "self-relative empty descriptor", &sd_access(empty.ptr(), tok, 0x1, &map));
    // SAFETY: closing the handles.
    unsafe {
        CloseHandle(tok);
        CloseHandle(ev);
        CloseHandle(noquery);
        CloseHandle(src);
        CloseHandle(real_primary);
        CloseHandle(primary);
    }
}
