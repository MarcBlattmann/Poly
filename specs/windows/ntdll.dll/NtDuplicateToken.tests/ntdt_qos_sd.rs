//! NtDuplicateToken: the SECURITY_QUALITY_OF_SERVICE argument (length, levels, EffectiveOnly) and a security
//! descriptor in the object attributes: the DACL and owner of the new token object compared with the default.
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
type DWORD = u32;
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

const TOKEN_IMPERSONATION_LEVEL_CLASS: u32 = 9;
const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
const SECURITY_IMPERSONATION: u32 = 2;
const ACL_REVISION: u32 = 2;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const OWNER_SECURITY_INFORMATION: u32 = 0x1;
const GROUP_SECURITY_INFORMATION: u32 = 0x2;
const DACL_SECURITY_INFORMATION: u32 = 0x4;
const SACL_SECURITY_INFORMATION: u32 = 0x8;
const LABEL_SECURITY_INFORMATION: u32 = 0x10;
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

const SECURITY_DYNAMIC_TRACKING: u8 = 1;
const SECURITY_STATIC_TRACKING: u8 = 0;
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

/// A one-DWORD token class through NtQueryInformationToken, or -1 when it fails.
fn nt_tok_u32(token: HANDLE, class: u32) -> i64 {
    let mut raw = Raw::<64>::new();
    let (st, _) = nt_tq(token, class, &mut raw, 4);
    if st >= 0 { i64::from(raw.u32_at(0)) } else { -1 }
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
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
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
    fn NtQueryInformationToken(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtDuplicateToken(
        token: HANDLE,
        access: u32,
        attrs: *const NT_OA,
        effective_only: u8,
        kind: u32,
        out: *mut HANDLE,
    ) -> i32;
    fn NtQuerySecurityObject(h: HANDLE, info: u32, sd: PVOID, len: u32, ret_len: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn dup_with(src: HANDLE, oa: *const NT_OA, effective: u8, kind: u32) -> (i32, HANDLE) {
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: `oa` is a valid attributes structure; valid out pointer.
    let st = unsafe { NtDuplicateToken(src, TOKEN_ALL_ACCESS, oa, effective, kind, &mut out) };
    (st, out)
}

fn test(t: &mut Test) {
    let src = dup_token(TOKEN_ALL_ACCESS, SECURITY_IMPERSONATION, TOKEN_PRIMARY);
    for len in [0u32, 4, 8, 11, 12, 13, 16, 64] {
        let mut qos = nt_qos(SECURITY_IMPERSONATION, SECURITY_DYNAMIC_TRACKING, 0);
        qos.length = len;
        let mut oa = nt_oa(core::ptr::null(), 0);
        oa.qos = (&qos as *const NT_QOS).cast();
        let (st, out) = dup_with(src, &oa, 0, TOKEN_IMPERSONATION);
        if st >= 0 {
            say!(
                t,
                "quality-of-service length {len}: status={st:#010x} level={}",
                nt_tok_u32(out, TOKEN_IMPERSONATION_LEVEL_CLASS)
            );
            // SAFETY: closing the duplicate.
            unsafe { NtClose(out) };
        } else {
            say!(t, "quality-of-service length {len}: status={st:#010x}");
        }
    }
    for (name, tracking, eff) in [
        ("static tracking", SECURITY_STATIC_TRACKING, 0u8),
        ("dynamic tracking", SECURITY_DYNAMIC_TRACKING, 0),
        ("tracking 2", 2, 0),
        ("EffectiveOnly 1", SECURITY_DYNAMIC_TRACKING, 1),
        ("EffectiveOnly 2", SECURITY_DYNAMIC_TRACKING, 2),
    ] {
        let qos = nt_qos(SECURITY_IMPERSONATION, tracking, eff);
        let mut oa = nt_oa(core::ptr::null(), 0);
        oa.qos = (&qos as *const NT_QOS).cast();
        let (st, out) = dup_with(src, &oa, 0, TOKEN_IMPERSONATION);
        if st >= 0 {
            say!(t, "{name}: status={st:#010x} level={}", nt_tok_u32(out, TOKEN_IMPERSONATION_LEVEL_CLASS));
            // SAFETY: closing the duplicate.
            unsafe { NtClose(out) };
        } else {
            say!(t, "{name}: status={st:#010x}");
        }
    }
    // The default security of a duplicate.
    let qos = nt_qos(SECURITY_IMPERSONATION, SECURITY_DYNAMIC_TRACKING, 0);
    let mut oa = nt_oa(core::ptr::null(), 0);
    oa.qos = (&qos as *const NT_QOS).cast();
    let (_, plain) = dup_with(src, &oa, 0, TOKEN_IMPERSONATION);
    nt_print_sd(
        t,
        "default security",
        plain,
        OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
    );
    // SAFETY: closing the duplicate.
    unsafe { NtClose(plain) };
    // A descriptor with a DACL (Everyone: TOKEN_QUERY) and no owner.
    let everyone = sid_of("S-1-1-0");
    let mut sd = SdBuf::new();
    let mut acl = AclBuf::new();
    nt_build_sd(&mut sd, &mut acl, &[(&everyone, true, TOKEN_QUERY)], core::ptr::null(), core::ptr::null());
    let mut oa2 = nt_oa(core::ptr::null(), 0);
    oa2.qos = (&qos as *const NT_QOS).cast();
    oa2.descriptor = sd.p() as *const c_void;
    let (st, d1) = dup_with(src, &oa2, 0, TOKEN_IMPERSONATION);
    say!(t, "descriptor with a DACL only: status={st:#010x}");
    if st >= 0 {
        nt_print_sd(
            t,
            "  new security",
            d1,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
        );
        // SAFETY: closing the duplicate.
        unsafe { NtClose(d1) };
    }
    // A descriptor with an owner (Everyone) too.
    let mut sd2 = SdBuf::new();
    let mut acl2 = AclBuf::new();
    nt_build_sd(&mut sd2, &mut acl2, &[(&everyone, true, TOKEN_QUERY)], everyone.as_const(), core::ptr::null());
    oa2.descriptor = sd2.p() as *const c_void;
    let (st, d2) = dup_with(src, &oa2, 0, TOKEN_IMPERSONATION);
    say!(t, "descriptor with a DACL and owner Everyone: status={st:#010x}");
    if st >= 0 {
        nt_print_sd(
            t,
            "  new security",
            d2,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
        );
        // SAFETY: closing the duplicate.
        unsafe { NtClose(d2) };
    }
    // An empty descriptor (no DACL present).
    let mut sd3 = SdBuf::new();
    // SAFETY: an absolute descriptor in a large buffer.
    unsafe { InitializeSecurityDescriptor(sd3.p(), SECURITY_DESCRIPTOR_REVISION) };
    oa2.descriptor = sd3.p() as *const c_void;
    let (st, d3) = dup_with(src, &oa2, 0, TOKEN_IMPERSONATION);
    say!(t, "descriptor without DACL: status={st:#010x}");
    if st >= 0 {
        nt_print_sd(t, "  new security", d3, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION);
        // SAFETY: closing the duplicate.
        unsafe { NtClose(d3) };
    }
    // SAFETY: closing the source.
    unsafe { CloseHandle(src) };
}
