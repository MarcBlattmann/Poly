//! SetSecurityInfo on a duplicate of the process token (the process token itself is not touched): replacing
//! its DACL and reading it back; what the token's own access check then allows. (ambient: the caller's token)
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
const READ_CONTROL: u32 = 0x0002_0000;
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
const SECURITY_IMPERSONATION: u32 = 2;
const ACL_REVISION: u32 = 2;
const DACL_SECURITY_INFORMATION: u32 = 0x4;
const SE_KERNEL_OBJECT: u32 = 6;
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

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
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

/// `say!` with a longer line (SDDL texts, hex dumps).
macro_rules! sdsay {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<2000>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const SD_SENTINEL: usize = 0x1234_5678_9ABC;
/// `s` with the machine-dependent SIDs masked: S-1-5-21-<big>-<big>-<big>-RID becomes
/// S-1-5-21-<machine>-RID and S-1-5-5-x-y becomes S-1-5-5-<logon>. Small made-up domain numbers
/// (S-1-5-21-1-2-3-...) are left alone.
fn sd_mask_text(s: &str) -> Buf<1500> {
    let b = s.as_bytes();
    let mut out = Buf::<1500>::new();
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"S-1-5-21-") {
            let mut j = i + 9;
            let mut groups = 0;
            let mut first_len = 0;
            let mut last = (j, j);
            loop {
                let st = j;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if j == st {
                    if groups > 0 && b[st - 1] == b'-' {
                        j = st - 1;
                    }
                    break;
                }
                groups += 1;
                if groups == 1 {
                    first_len = j - st;
                }
                last = (st, j);
                if j < b.len() && b[j] == b'-' {
                    j += 1;
                } else {
                    break;
                }
            }
            if first_len >= 5 && groups >= 3 {
                let _ = write!(out, "S-1-5-21-<machine>");
                if groups >= 4 {
                    let _ = write!(out, "-{}", core::str::from_utf8(&b[last.0..last.1]).unwrap_or("?"));
                }
                i = j;
                continue;
            }
        }
        if b[i..].starts_with(b"S-1-5-5-") {
            let mut j = i + 8;
            let mut groups = 0;
            loop {
                let st = j;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if j == st {
                    break;
                }
                groups += 1;
                if groups < 2 && j < b.len() && b[j] == b'-' {
                    j += 1;
                } else {
                    break;
                }
            }
            if groups == 2 {
                let _ = write!(out, "S-1-5-5-<logon>");
                i = j;
                continue;
            }
        }
        let _ = write!(out, "{}", b[i] as char);
        i += 1;
    }
    out
}

/// The result of ConvertSecurityDescriptorToStringSecurityDescriptorW: success, the text (machine
/// SIDs masked), the length it reported (characters including the NUL?), the real string length and the
/// error after a failure.
struct SdText {
    ok: i32,
    text: Buf<1500>,
    len: u32,
    units: usize,
    err: i64,
}

fn sd_sddl_rev(sd: *const c_void, rev: u32, info: u32) -> SdText {
    let mut out: *mut u16 = core::ptr::null_mut();
    let mut len = 0xAAAA_AAAAu32;
    clear_error();
    // SAFETY: `sd` is a descriptor or the call fails; both out pointers are valid.
    let ok = unsafe { ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, rev, info, &mut out, &mut len) };
    let err = gle_if(ok);
    let mut text = Buf::<1500>::new();
    let mut units = 0;
    if ok != 0 && !out.is_null() {
        let raw = wide_text::<1500>(out);
        text = sd_mask_text(raw.as_str());
        units = wide_len(out);
        // SAFETY: the string was allocated with LocalAlloc by the call.
        unsafe { LocalFree(out.cast()) };
    } else {
        let _ = write!(text, "-");
    }
    SdText { ok, text, len, units, err }
}

fn sd_sddl(sd: *const c_void, info: u32) -> SdText {
    sd_sddl_rev(sd, SDDL_REVISION_1, info)
}

/// One line: success, text, reported length and string length.
fn sd_sddl_say(t: &mut Test, label: &str, sd: *const c_void, info: u32) {
    let r = sd_sddl(sd, info);
    if r.ok != 0 {
        sdsay!(t, "{label}: ok=1 text={} len={} units={}", r.text.as_str(), r.len, r.units);
    } else {
        sdsay!(t, "{label}: ok=0 err={} len={:#x}", r.err, r.len);
    }
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

/// An impersonation token (level impersonation) duplicated from the process token with all its
/// privileges disabled, or null.
fn sd_imp_token() -> HANDLE {
    let t = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_IMPERSONATION);
    // SAFETY: disables every privilege of the copy (no new state).
    unsafe { AdjustTokenPrivileges(t, TRUE, core::ptr::null(), 0, core::ptr::null_mut(), core::ptr::null_mut()) };
    t
}

/// The outputs of GetSecurityInfo (all pointers start as the sentinel); the descriptor is freed on drop.
struct SdGsi {
    rc: u32,
    owner: *mut c_void,
    group: *mut c_void,
    dacl: *mut c_void,
    sacl: *mut c_void,
    sd: *mut c_void,
}

impl Drop for SdGsi {
    fn drop(&mut self) {
        if !self.sd.is_null() && self.sd as usize != SD_SENTINEL {
            // SAFETY: allocated with LocalAlloc by GetSecurityInfo.
            unsafe { LocalFree(self.sd) };
        }
    }
}

/// GetSecurityInfo with the output pointers selected by `give`: 1 owner, 2 group, 4 DACL, 8 SACL, 0x10 descriptor.
fn sd_gsi(h: HANDLE, kind: u32, info: u32, give: u32) -> SdGsi {
    let s = SD_SENTINEL as *mut c_void;
    let mut r = SdGsi { rc: 0xAAAA, owner: s, group: s, dacl: s, sacl: s, sd: s };
    let np = core::ptr::null_mut::<*mut c_void>();
    let po = if give & 1 != 0 { &mut r.owner as *mut *mut c_void } else { np };
    let pg = if give & 2 != 0 { &mut r.group as *mut *mut c_void } else { np };
    let pd = if give & 4 != 0 { &mut r.dacl as *mut *mut c_void } else { np };
    let ps = if give & 8 != 0 { &mut r.sacl as *mut *mut c_void } else { np };
    let psd = if give & 0x10 != 0 { &mut r.sd as *mut *mut c_void } else { np };
    // SAFETY: the out pointers are NULL or point into `r`.
    r.rc = unsafe { GetSecurityInfo(h, kind, info, po, pg, pd, ps, psd) };
    r
}

/// SetSecurityInfo for a kernel object with pointers as given (NULL allowed): the return code.
fn sd_ssi(h: HANDLE, info: u32, owner: *mut c_void, group: *mut c_void, dacl: *mut c_void, sacl: *mut c_void) -> u32 {
    // SAFETY: the pointers are NULL or valid SIDs/ACLs.
    unsafe { SetSecurityInfo(h, SE_KERNEL_OBJECT, info, owner, group, dacl, sacl) }
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
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
        sd: *const c_void,
        revision: u32,
        info: u32,
        out: *mut *mut u16,
        len: *mut u32,
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
    fn GetSecurityInfo(
        h: HANDLE,
        kind: u32,
        info: u32,
        owner: *mut PSID,
        group: *mut PSID,
        dacl: *mut PACL,
        sacl: *mut PACL,
        sd: *mut PSECURITY_DESCRIPTOR,
    ) -> u32;
    fn SetSecurityInfo(h: HANDLE, kind: u32, info: u32, owner: PSID, group: PSID, dacl: PACL, sacl: PACL) -> u32;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryObject(h: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn back(t: &mut Test, label: &str, h: HANDLE) {
    let r = sd_gsi(h, SE_KERNEL_OBJECT, 5, 0x14);
    sdsay!(t, "{label}: rc={}", r.rc);
    if r.rc == 0 {
        sd_sddl_say(t, "  now", r.sd, 5);
    }
}

fn test(t: &mut Test) {
    let tok = sd_imp_token();
    let np = core::ptr::null_mut();
    back(t, "initial", tok);
    let mut acl = AclBuf::new();
    sd_build_acl(&mut acl, &[(0, 0, TOKEN_ALL_ACCESS, "S-1-5-32-544"), (0, 0, TOKEN_QUERY | READ_CONTROL, "S-1-1-0")]);
    sdsay!(t, "set DACL on the token copy: rc={}", sd_ssi(tok, DACL_SECURITY_INFORMATION, np, np, acl.p(), np));
    back(t, "after", tok);
    let mut acl2 = AclBuf::new();
    sd_build_acl(&mut acl2, &[(0, 0, TOKEN_QUERY, "S-1-1-0")]);
    sdsay!(
        t,
        "set a DACL granting only TOKEN_QUERY: rc={}",
        sd_ssi(tok, DACL_SECURITY_INFORMATION, np, np, acl2.p(), np)
    );
    back(t, "after", tok);
    // The existing handle keeps its access mask.
    sdsay!(t, "the handle's granted access is unchanged: {}", granted_access(tok) == TOKEN_ALL_ACCESS);
    // SAFETY: closing the handle.
    unsafe { CloseHandle(tok) };
}
