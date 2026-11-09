//! SetKernelObjectSecurity failures: invalid handles, a damaged or uninitialized descriptor, information 0
//! and unknown information bits.
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

/// GetKernelObjectSecurity into a 2048-byte buffer: (success, needed length, error, the descriptor bytes).
struct SdKernelGet {
    ok: i32,
    needed: u32,
    err: i64,
    buf: Raw<2048>,
}

fn sd_get_kernel_len(h: HANDLE, info: u32, len: u32) -> SdKernelGet {
    let mut r = SdKernelGet { ok: 0, needed: 0xAAAA_AAAA, err: 0, buf: Raw::new() };
    clear_error();
    // SAFETY: the buffer is 2048 bytes and `len` never exceeds that; valid out pointer.
    r.ok = unsafe { GetKernelObjectSecurity(h, info, r.buf.p(), len, &mut r.needed) };
    r.err = gle_if(r.ok);
    r
}

fn sd_get_kernel(h: HANDLE, info: u32) -> SdKernelGet {
    sd_get_kernel_len(h, info, 2048)
}

/// The SDDL text of an object's security (kernel object), with `info` as request: "ok=1 text=..." or the error.
fn sd_kernel_text(t: &mut Test, label: &str, h: HANDLE, info: u32) {
    let r = sd_get_kernel(h, info);
    if r.ok != 0 {
        sd_sddl_say(t, label, r.buf.b.as_ptr().cast(), info & 0x1F);
    } else {
        sdsay!(t, "{label}: get failed err={} needed={}", r.err, r.needed);
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

/// A copy of a handle with exactly `access`.
fn sd_dup_handle(h: HANDLE, access: u32) -> HANDLE {
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: duplicates within the current process.
    unsafe { DuplicateHandle(GetCurrentProcess(), h, GetCurrentProcess(), &mut out, access, FALSE, 0) };
    out
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

/// SetKernelObjectSecurity: (success, last error after a failure).
fn sd_sko(h: HANDLE, info: u32, sd: *const c_void) -> (i32, i64) {
    clear_error();
    // SAFETY: `sd` is a descriptor in a live buffer.
    let ok = unsafe { SetKernelObjectSecurity(h, info, sd) };
    (ok, gle_if(ok))
}

fn sd_sko_say(t: &mut Test, label: &str, h: HANDLE, info: u32, sd: *const c_void) {
    let (ok, err) = sd_sko(h, info, sd);
    sdsay!(t, "{label}: ok={ok} err={err}");
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
    fn DuplicateHandle(
        src_process: HANDLE,
        src: HANDLE,
        dst_process: HANDLE,
        dst: *mut HANDLE,
        access: u32,
        inherit: BOOL,
        options: u32,
    ) -> BOOL;
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
    fn InitializeSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, revision: u32) -> BOOL;
    fn SetSecurityDescriptorOwner(sd: PSECURITY_DESCRIPTOR, owner: *const c_void, defaulted: BOOL) -> BOOL;
    fn SetSecurityDescriptorGroup(sd: PSECURITY_DESCRIPTOR, group: *const c_void, defaulted: BOOL) -> BOOL;
    fn SetSecurityDescriptorDacl(sd: PSECURITY_DESCRIPTOR, present: BOOL, dacl: *const c_void, defaulted: BOOL)
    -> BOOL;
    fn SetSecurityDescriptorSacl(sd: PSECURITY_DESCRIPTOR, present: BOOL, sacl: *const c_void, defaulted: BOOL)
    -> BOOL;
    fn GetKernelObjectSecurity(h: HANDLE, info: u32, sd: PSECURITY_DESCRIPTOR, len: u32, needed: *mut u32) -> BOOL;
    fn SetKernelObjectSecurity(h: HANDLE, info: u32, sd: *const c_void) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let ev = sd_event();
    let sd = sd_from("D:(A;;0x1f0003;;;WD)");
    sd_sko_say(t, "NULL handle", core::ptr::null_mut(), 4, sd.ptr());
    sd_sko_say(t, "handle 0x1234", 0x1234 as HANDLE, 4, sd.ptr());
    sd_sko_say(t, "information 0", ev, 0, sd.ptr());
    for info in [0x20u32, 0x40, 0x80, 0x100, 0x8000_0000, 0x4000_0000, 0x2000_0000, 0xFFFF_FFFF] {
        let mut l = Buf::<60>::new();
        let _ = write!(l, "information {info:#x} alone");
        sd_sko_say(t, l.as_str(), ev, info, sd.ptr());
    }
    sd_sko_say(t, "information DACL|0x8000_0000 (protected)", ev, 0x8000_0004, sd.ptr());
    sd_sko_say(t, "information DACL|0x2000_0000 (unprotected)", ev, 0x2000_0004, sd.ptr());
    // Damaged descriptors (copies of a self-relative one).
    let base = sd_from("O:BAG:SYD:(A;;0x1f0003;;;WD)");
    for (title, f) in [
        ("revision 0", (|r: &mut Raw<512>| r.b[0] = 0) as fn(&mut Raw<512>)),
        ("revision 2", |r| r.b[0] = 2),
        ("DACL revision 9", |r| {
            let o = r.u32_at(16) as usize;
            r.b[o] = 9
        }),
        ("owner SID count 16", |r| {
            let o = r.u32_at(4) as usize;
            r.b[o + 1] = 16
        }),
        ("DACL size 4", |r| {
            let o = r.u32_at(16) as usize;
            r.b[o + 2] = 4;
            r.b[o + 3] = 0
        }),
    ] {
        let mut r = Raw::<512>::new();
        r.b[..base.bytes().len()].copy_from_slice(base.bytes());
        f(&mut r);
        sd_sko_say(t, title, ev, 7, r.b.as_ptr().cast());
    }
    let zero = Raw::<64>::new();
    sd_sko_say(t, "all-zero descriptor", ev, 4, zero.b.as_ptr().cast());
    let mut empty = SdAbs::new();
    sd_sko_say(t, "initialized empty absolute descriptor, information DACL", ev, 4, empty.cp());
    sd_sko_say(t, "initialized empty absolute descriptor, information OWNER", ev, 1, empty.cp());
    sd_sko_say(t, "initialized empty absolute descriptor, information OWNER|GROUP|DACL", ev, 7, empty.cp());
    empty.null_dacl();
    sd_sko_say(t, "absolute descriptor with NULL DACL", ev, 4, empty.cp());
    sd_kernel_text(t, "  dacl now", ev, 4);
    // A thread/process pseudo handle, a token handle without WRITE_DAC.
    let tok = sd_imp_token();
    let tok_rc = sd_dup_handle(tok, TOKEN_QUERY | READ_CONTROL);
    sd_sko_say(t, "token handle without WRITE_DAC", tok_rc, 4, sd.ptr());
    // SAFETY: closing the handles.
    unsafe {
        CloseHandle(tok_rc);
        CloseHandle(tok);
        CloseHandle(ev);
    }
}
