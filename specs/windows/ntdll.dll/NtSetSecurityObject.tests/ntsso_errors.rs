//! NtSetSecurityObject errors: handles without WRITE_DAC / WRITE_OWNER, descriptors lacking the part being set, a bad
//! revision, the empty information mask, undefined bits, the SACL without the privilege, a self-relative descriptor,
//! NULL and closed handles.
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
const TOKEN_QUERY: u32 = 0x0008;
const STANDARD_RIGHTS_REQUIRED: u32 = 0x000F_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
const TOKEN_USER_CLASS: u32 = 1;
const ACL_REVISION: u32 = 2;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const OWNER_SECURITY_INFORMATION: u32 = 0x1;
const GROUP_SECURITY_INFORMATION: u32 = 0x2;
const DACL_SECURITY_INFORMATION: u32 = 0x4;
const SACL_SECURITY_INFORMATION: u32 = 0x8;
const LABEL_SECURITY_INFORMATION: u32 = 0x10;
const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
const UNPROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x2000_0000;
const JOB_OBJECT_QUERY: u32 = 0x4;
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

/// winbase.h: `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: PVOID,
    inherit: BOOL,
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

/// A pointer stored in a result buffer, as an offset from the buffer start
/// (so printed values do not depend on addresses); -1 for null, -2 outside.
fn rel(base: *const u8, len: usize, p: *const c_void) -> i64 {
    if p.is_null() {
        return -1;
    }
    let d = (p as usize).wrapping_sub(base as usize);
    if d < len { d as i64 } else { -2 }
}

/// The text of a SID with the machine-dependent parts masked: S-1-5-21-a-b-c-RID prints as
/// S-1-5-21-<machine>-RID and a logon-session SID S-1-5-5-x-y as S-1-5-5-<logon>.
fn mask_sid(sid: *const c_void) -> Buf<200> {
    let full = sid_text(sid);
    let s = full.as_str();
    let mut b = Buf::<200>::new();
    if s.starts_with("S-1-5-21-") && s.matches('-').count() >= 7 {
        let rid = s.rsplit('-').next().unwrap_or("?");
        if rid.parse::<u32>().map_or(false, |r| r >= 1000) {
            let _ = write!(b, "S-1-5-21-<machine>-<account>");
        } else {
            let _ = write!(b, "S-1-5-21-<machine>-{rid}");
        }
    } else if s.starts_with("S-1-5-5-") {
        let _ = write!(b, "S-1-5-5-<logon>");
    } else {
        let _ = write!(b, "{s}");
    }
    b
}

/// The SID that the first `SID_AND_ATTRIBUTES` of a TOKEN_USER-like result points at.
fn first_sid(raw: &Raw<2048>) -> *const c_void {
    raw.u64_at(0) as usize as *const c_void
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

/// NtQueryInformationToken into `buf` with length `len`; (status, returned length).
fn nt_tq<const N: usize>(token: HANDLE, class: u32, buf: &mut Raw<N>, len: u32) -> (i32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    // SAFETY: `buf` has N writable bytes; callers pass len <= N.
    let st = unsafe { NtQueryInformationToken(token, class, buf.p(), len, &mut ret) };
    (st, ret)
}

/// A duplicate of `h` (in this process) with exactly `access`, or null.
fn nt_dup_access(h: HANDLE, access: u32) -> HANDLE {
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: DuplicateHandle with an explicit access mask (no DUPLICATE_SAME_ACCESS).
    unsafe { DuplicateHandle(GetCurrentProcess(), h, GetCurrentProcess(), &mut out, access, FALSE, 0) };
    out
}

/// An unnamed job object with all access, or null.
fn nt_job() -> HANDLE {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: valid out pointer; no object attributes.
    unsafe { NtCreateJobObject(&mut h, JOB_OBJECT_ALL_ACCESS, core::ptr::null()) };
    h
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

/// The self-relative copy of an absolute descriptor into `out`; its length (0 on failure).
fn nt_selfrel(abs: &mut SdBuf, out: &mut SdBuf) -> u32 {
    let mut len = 4096u32;
    // SAFETY: both buffers are 4096 bytes.
    let ok = unsafe { MakeSelfRelativeSD(abs.p(), out.p(), &mut len) };
    if ok != 0 { len } else { 0 }
}

/// Print the ACL at `off` inside the self-relative descriptor at `base`: revision, size, count and each ACE
/// (type, flags, mask, masked SID).
fn nt_print_acl(t: &mut Test, label: &str, base: *const u8, off: usize) {
    if off == 0 {
        say!(t, "{label}: none");
        return;
    }
    // SAFETY: `off` lies inside the 4096-byte result buffer the caller read the descriptor into.
    let acl = unsafe { base.add(off) };
    // SAFETY: the ACL header is 8 bytes inside the buffer.
    let (rev, size, count) = unsafe {
        (*acl, u16::from_le_bytes([*acl.add(2), *acl.add(3)]), u16::from_le_bytes([*acl.add(4), *acl.add(5)]))
    };
    say!(t, "{label}: revision={rev} size={size} count={count}");
    let mut pos = 8usize;
    for i in 0..usize::from(count).min(16) {
        // SAFETY: the ACEs follow the header inside the ACL (bounded by the loop count and the buffer).
        let (ty, fl, sz, mask) = unsafe {
            let a = acl.add(pos);
            (
                *a,
                *a.add(1),
                u16::from_le_bytes([*a.add(2), *a.add(3)]),
                u32::from_le_bytes([*a.add(4), *a.add(5), *a.add(6), *a.add(7)]),
            )
        };
        // SAFETY: the SID of an access ACE starts at byte 8 of the ACE.
        let sid = unsafe { acl.add(pos + 8) } as *const c_void;
        say!(t, "{label}[{i}]: type={ty} flags={fl:#x} size={sz} mask={mask:#x} sid={}", mask_sid(sid).as_str());
        pos += usize::from(sz).max(8);
    }
}

/// NtQuerySecurityObject(info) of a handle printed: status, length, the self-relative layout (control, offsets),
/// the masked owner and group SIDs and the ACLs.
fn nt_print_sd(t: &mut Test, label: &str, h: HANDLE, info: u32) {
    let mut raw = Raw::<4096>::new();
    let mut ret = 0xAAAA_AAAAu32;
    // SAFETY: 4096 writable bytes.
    let st = unsafe { NtQuerySecurityObject(h, info, raw.p(), 4096, &mut ret) };
    if st < 0 {
        say!(t, "{label}: status={st:#010x} needed={}", if ret == 0xAAAA_AAAA { -1 } else { i64::from(ret) });
        return;
    }
    let base = raw.b.as_ptr();
    let (o, g, s, d) =
        (raw.u32_at(4) as usize, raw.u32_at(8) as usize, raw.u32_at(12) as usize, raw.u32_at(16) as usize);
    say!(
        t,
        "{label}: status={st:#010x} length={ret} revision={} control={:#06x} offsets owner={o} group={g} sacl={s} dacl={d}",
        raw.b[0],
        raw.u16_at(2)
    );
    if o != 0 {
        // SAFETY: the owner SID lies inside the buffer.
        say!(t, "{label}: owner={}", mask_sid(unsafe { base.add(o) } as *const c_void).as_str());
    }
    if g != 0 {
        // SAFETY: the group SID lies inside the buffer.
        say!(t, "{label}: group={}", mask_sid(unsafe { base.add(g) } as *const c_void).as_str());
    }
    if info & (SACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION) != 0 {
        nt_print_acl(t, "  sacl", base, s);
    }
    if info & DACL_SECURITY_INFORMATION != 0 {
        nt_print_acl(t, "  dacl", base, d);
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn LocalFree(p: PVOID) -> PVOID;
    fn GetCurrentProcess() -> HANDLE;
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
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn InitializeAcl(acl: PACL, len: u32, revision: u32) -> BOOL;
    fn AddAccessAllowedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAccessDeniedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAuditAccessAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void, success: BOOL, failure: BOOL)
    -> BOOL;
    fn InitializeSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, revision: u32) -> BOOL;
    fn SetSecurityDescriptorOwner(sd: PSECURITY_DESCRIPTOR, owner: *const c_void, defaulted: BOOL) -> BOOL;
    fn SetSecurityDescriptorGroup(sd: PSECURITY_DESCRIPTOR, group: *const c_void, defaulted: BOOL) -> BOOL;
    fn SetSecurityDescriptorDacl(sd: PSECURITY_DESCRIPTOR, present: BOOL, dacl: *const c_void, defaulted: BOOL)
    -> BOOL;
    fn SetSecurityDescriptorSacl(sd: PSECURITY_DESCRIPTOR, present: BOOL, sacl: *const c_void, defaulted: BOOL)
    -> BOOL;
    fn MakeSelfRelativeSD(abs: *const c_void, rel: PSECURITY_DESCRIPTOR, len: *mut u32) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryInformationToken(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtQuerySecurityObject(h: HANDLE, info: u32, sd: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtSetSecurityObject(h: HANDLE, info: u32, sd: *const c_void) -> i32;
    fn NtCreateJobObject(job: *mut HANDLE, access: u32, attrs: *const NT_OA) -> i32;
}

poly_wtest::main!(test);

fn set(h: HANDLE, info: u32, sd: *const c_void) -> i32 {
    // SAFETY: `sd` points at a valid descriptor (possibly missing parts) in a large buffer.
    unsafe { NtSetSecurityObject(h, info, sd) }
}

fn test(t: &mut Test) {
    let job = nt_job();
    let everyone = sid_of("S-1-1-0");
    let mut user = Raw::<2048>::new();
    let tk = process_token(TOKEN_QUERY);
    nt_tq(tk, TOKEN_USER_CLASS, &mut user, 2048);
    let mut full = SdBuf::new();
    let mut acl = AclBuf::new();
    nt_build_sd(&mut full, &mut acl, &[(&everyone, true, JOB_OBJECT_QUERY)], first_sid(&user), first_sid(&user));
    let f = full.p() as *const c_void;
    for (name, access) in [
        ("only JOB_OBJECT_QUERY", JOB_OBJECT_QUERY),
        ("only WRITE_DAC", WRITE_DAC),
        ("only WRITE_OWNER", WRITE_OWNER),
        ("only READ_CONTROL", READ_CONTROL),
    ] {
        let h = nt_dup_access(job, access);
        let mut line = Buf::<200>::new();
        for (n, i) in [
            ("dacl", DACL_SECURITY_INFORMATION),
            ("owner", OWNER_SECURITY_INFORMATION),
            ("group", GROUP_SECURITY_INFORMATION),
        ] {
            let _ = write!(line, "{n}:{:#x} ", set(h, i, f));
        }
        say!(t, "handle with {name}: {}", line.as_str());
        // SAFETY: closing the duplicate.
        unsafe { CloseHandle(h) };
    }
    // Descriptors missing the part.
    let mut bare = SdBuf::new();
    // SAFETY: an absolute descriptor in a large buffer.
    unsafe { InitializeSecurityDescriptor(bare.p(), SECURITY_DESCRIPTOR_REVISION) };
    let b = bare.p() as *const c_void;
    say!(t, "owner info, descriptor without owner: {:#010x}", set(job, OWNER_SECURITY_INFORMATION, b));
    say!(t, "group info, descriptor without group: {:#010x}", set(job, GROUP_SECURITY_INFORMATION, b));
    say!(t, "DACL info, descriptor without DACL: {:#010x}", set(job, DACL_SECURITY_INFORMATION, b));
    say!(t, "SACL info, descriptor without SACL: {:#010x}", set(job, SACL_SECURITY_INFORMATION, b));
    say!(t, "label info, descriptor without SACL: {:#010x}", set(job, LABEL_SECURITY_INFORMATION, b));
    let mut bad = SdBuf::new();
    let mut a2 = AclBuf::new();
    nt_build_sd(&mut bad, &mut a2, &[(&everyone, true, JOB_OBJECT_QUERY)], first_sid(&user), first_sid(&user));
    bad.b[0] = 9;
    say!(t, "descriptor revision 9: {:#010x}", set(job, DACL_SECURITY_INFORMATION, bad.p() as *const c_void));
    say!(t, "information mask 0: {:#010x}", set(job, 0, f));
    say!(t, "information mask 0x20: {:#010x}", set(job, 0x20, f));
    say!(t, "information mask 0x100: {:#010x}", set(job, 0x100, f));
    say!(
        t,
        "information mask DACL|PROTECTED_DACL: {:#010x}",
        set(job, DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, f)
    );
    say!(
        t,
        "information mask DACL|UNPROTECTED_DACL: {:#010x}",
        set(job, DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION, f)
    );
    nt_print_sd(t, "DACL control after PROTECTED_DACL", job, DACL_SECURITY_INFORMATION);
    // SACL with a plain audit ACE needs SeSecurityPrivilege.
    let mut s = SdBuf::new();
    let mut sacl = AclBuf::new();
    // SAFETY: valid buffers; the SID outlives the descriptor.
    unsafe {
        InitializeSecurityDescriptor(s.p(), SECURITY_DESCRIPTOR_REVISION);
        InitializeAcl(sacl.p(), 1024, ACL_REVISION);
        AddAuditAccessAce(sacl.p(), ACL_REVISION, JOB_OBJECT_QUERY, everyone.as_const(), TRUE, TRUE);
        SetSecurityDescriptorSacl(s.p(), TRUE, sacl.p(), FALSE);
    }
    say!(
        t,
        "SACL with an audit ACE, privilege disabled: {:#010x}",
        set(job, SACL_SECURITY_INFORMATION, s.p() as *const c_void)
    );
    // A self-relative descriptor is accepted too.
    let mut rel_sd = SdBuf::new();
    let n = nt_selfrel(&mut full, &mut rel_sd);
    say!(
        t,
        "self-relative descriptor (length_nonzero={}): {:#010x}",
        n > 0,
        set(job, DACL_SECURITY_INFORMATION, rel_sd.p() as *const c_void)
    );
    say!(t, "NULL handle: {:#010x}", set(core::ptr::null_mut(), DACL_SECURITY_INFORMATION, f));
    // SAFETY: an unnamed event, closed afterwards.
    let ev = unsafe { CreateEventW(core::ptr::null(), TRUE, FALSE, core::ptr::null()) };
    say!(t, "unnamed event handle: {:#010x}", set(ev, DACL_SECURITY_INFORMATION, f));
    // SAFETY: closing the event.
    unsafe { CloseHandle(ev) };
    // SAFETY: closing the handles.
    unsafe {
        NtClose(job);
        CloseHandle(tk);
    }
    say!(t, "closed handle: {:#010x}", set(job, DACL_SECURITY_INFORMATION, f));
}
