//! Control bits in the output: P (protected), AR (auto-inherit required), AI (auto-inherited) after D: and S:,
//! NO_ACCESS_CONTROL for a NULL ACL, the empty ACL, an absent ACL; defaulted bits and the others leave no trace.
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
const ACL_REVISION: u32 = 2;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const SE_DACL_AUTO_INHERIT_REQ: u16 = 0x0100;
const SE_SACL_AUTO_INHERIT_REQ: u16 = 0x0200;
const SE_DACL_AUTO_INHERITED: u16 = 0x0400;
const SE_SACL_AUTO_INHERITED: u16 = 0x0800;
const SE_DACL_PROTECTED: u16 = 0x1000;
const SE_SACL_PROTECTED: u16 = 0x2000;
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

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
}

/// `say!` with a longer line (SDDL texts, hex dumps).
macro_rules! sdsay {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<2000>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const SE_DACL_UNTRUSTED: u16 = 0x40;
const SE_SERVER_SECURITY: u16 = 0x80;
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
    fn LocalFree(p: PVOID) -> PVOID;
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
    fn SetSecurityDescriptorControl(sd: PSECURITY_DESCRIPTOR, mask: u16, set: u16) -> BOOL;
}

poly_wtest::main!(test);

fn text(a: &SdAbs) -> Buf<1500> {
    sd_sddl(a.cp(), 0xF).text
}

fn test(t: &mut Test) {
    let bits: [(&str, u16); 12] = [
        ("DACL_PROTECTED", SE_DACL_PROTECTED),
        ("DACL_AUTO_INHERIT_REQ", SE_DACL_AUTO_INHERIT_REQ),
        ("DACL_AUTO_INHERITED", SE_DACL_AUTO_INHERITED),
        ("SACL_PROTECTED", SE_SACL_PROTECTED),
        ("SACL_AUTO_INHERIT_REQ", SE_SACL_AUTO_INHERIT_REQ),
        ("SACL_AUTO_INHERITED", SE_SACL_AUTO_INHERITED),
        ("DACL_UNTRUSTED", SE_DACL_UNTRUSTED),
        ("SERVER_SECURITY", SE_SERVER_SECURITY),
        ("RM_CONTROL_VALID", 0x4000),
        (
            "DACL_PROTECTED|DACL_AUTO_INHERIT_REQ|DACL_AUTO_INHERITED",
            SE_DACL_PROTECTED | SE_DACL_AUTO_INHERIT_REQ | SE_DACL_AUTO_INHERITED,
        ),
        (
            "SACL_PROTECTED|SACL_AUTO_INHERIT_REQ|SACL_AUTO_INHERITED",
            SE_SACL_PROTECTED | SE_SACL_AUTO_INHERIT_REQ | SE_SACL_AUTO_INHERITED,
        ),
        ("all of them", 0x7FC0 | 0x1F00),
    ];
    for (name, bit) in bits {
        let mut a = SdAbs::new();
        a.set_dacl(&[(0, 0, 0x1, "S-1-1-0")]);
        a.set_sacl(&[(2, 0xC0, 0x1, "S-1-1-0")]);
        a.sd.b[2] |= bit as u8;
        a.sd.b[3] |= (bit >> 8) as u8;
        sdsay!(t, "{name}: {}", text(&a).as_str());
    }
    let mut n = SdAbs::new();
    n.null_dacl();
    sdsay!(t, "NULL DACL: {}", text(&n).as_str());
    let mut e = SdAbs::new();
    e.set_dacl(&[]);
    sdsay!(t, "empty DACL: {}", text(&e).as_str());
    let a = SdAbs::new();
    sdsay!(t, "no DACL, no SACL: {:?}", text(&a).as_str());
    let mut p = SdAbs::new();
    p.null_dacl();
    // SAFETY: valid descriptor.
    unsafe { SetSecurityDescriptorControl(p.p(), SE_DACL_PROTECTED, SE_DACL_PROTECTED) };
    sdsay!(t, "NULL DACL, protected: {}", text(&p).as_str());
    let mut q = SdAbs::new();
    q.set_dacl(&[]);
    // SAFETY: valid descriptor.
    unsafe {
        SetSecurityDescriptorControl(
            q.p(),
            SE_DACL_PROTECTED | SE_DACL_AUTO_INHERITED,
            SE_DACL_PROTECTED | SE_DACL_AUTO_INHERITED,
        )
    };
    sdsay!(t, "empty DACL, protected and auto-inherited: {}", text(&q).as_str());
    // SAFETY: valid descriptor.
    unsafe { SetSecurityDescriptorSacl(q.p(), TRUE, core::ptr::null(), FALSE) };
    sdsay!(t, "plus NULL SACL: {}", text(&q).as_str());
    let mut d = SdAbs::new();
    d.set_owner("S-1-5-18", true);
    d.set_group("S-1-5-18", true);
    d.set_dacl(&[(0, 0, 0x1, "S-1-1-0")]);
    // SAFETY: valid descriptor and ACL.
    unsafe { SetSecurityDescriptorDacl(d.p(), TRUE, d.dacl.p(), TRUE) };
    sdsay!(t, "owner, group and DACL defaulted: {}", text(&d).as_str());
}
