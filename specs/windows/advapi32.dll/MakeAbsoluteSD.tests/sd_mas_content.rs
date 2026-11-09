//! MakeAbsoluteSD result: the absolute header (control without SE_SELF_RELATIVE, pointers to the caller's
//! buffers), the copied SIDs and ACLs, the SDDL text of the result, the descriptor round trip.
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

/// The bytes as lowercase hex, no separators.
struct Hex<'a>(&'a [u8]);

impl core::fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

const SECURITY_MAX_SID_SIZE: usize = 68;
const SE_SELF_RELATIVE: u16 = 0x8000;
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

/// The bytes of the little-endian u32 at `off`.
fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
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

/// `say!` with a longer line (SDDL texts, hex dumps).
macro_rules! sdsay {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<2000>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const SD_SENTINEL: usize = 0x1234_5678_9ABC;
/// Names of the SECURITY_DESCRIPTOR_CONTROL bits that are set, joined with `|`.
fn sd_ctrl_names(c: u16) -> Buf<200> {
    let mut b = Buf::<200>::new();
    let names: [(u16, &str); 16] = [
        (0x0001, "OWNER_DEFAULTED"),
        (0x0002, "GROUP_DEFAULTED"),
        (0x0004, "DACL_PRESENT"),
        (0x0008, "DACL_DEFAULTED"),
        (0x0010, "SACL_PRESENT"),
        (0x0020, "SACL_DEFAULTED"),
        (0x0040, "DACL_UNTRUSTED"),
        (0x0080, "SERVER_SECURITY"),
        (0x0100, "DACL_AUTO_INHERIT_REQ"),
        (0x0200, "SACL_AUTO_INHERIT_REQ"),
        (0x0400, "DACL_AUTO_INHERITED"),
        (0x0800, "SACL_AUTO_INHERITED"),
        (0x1000, "DACL_PROTECTED"),
        (0x2000, "SACL_PROTECTED"),
        (0x4000, "RM_CONTROL_VALID"),
        (0x8000, "SELF_RELATIVE"),
    ];
    let mut first = true;
    for (bit, name) in names {
        if c & bit != 0 {
            let _ = write!(b, "{}{name}", if first { "" } else { "|" });
            first = false;
        }
    }
    if first {
        let _ = write!(b, "-");
    }
    b
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

/// Prints the header of a descriptor: absolute (control, padding, the four pointer slots as
/// null/set) or self-relative (control and the four offsets).
fn sd_head(t: &mut Test, label: &str, sd: *const c_void) {
    // SAFETY: the caller passes a descriptor of at least 20 bytes (self-relative) or 40 bytes (absolute).
    let first = unsafe { core::slice::from_raw_parts(sd as *const u8, 4) };
    let control = u16::from_le_bytes([first[2], first[3]]);
    if control & SE_SELF_RELATIVE != 0 {
        // SAFETY: a self-relative descriptor has a 20-byte header.
        let h = unsafe { core::slice::from_raw_parts(sd as *const u8, 20) };
        sdsay!(
            t,
            "{label}: self-relative rev={} sbz1={} control={control:#06x} [{}] owner={} group={} sacl={} dacl={}",
            h[0],
            h[1],
            sd_ctrl_names(control).as_str(),
            le32(h, 4),
            le32(h, 8),
            le32(h, 12),
            le32(h, 16)
        );
    } else {
        // SAFETY: an absolute descriptor is 40 bytes on x64.
        let h = unsafe { core::slice::from_raw_parts(sd as *const u8, 40) };
        let slot = |o: usize| {
            let mut a = [0u8; 8];
            a.copy_from_slice(&h[o..o + 8]);
            if u64::from_le_bytes(a) == 0 { "null" } else { "set" }
        };
        sdsay!(
            t,
            "{label}: absolute rev={} sbz1={} control={control:#06x} [{}] pad={} owner={} group={} sacl={} dacl={}",
            h[0],
            h[1],
            sd_ctrl_names(control).as_str(),
            Hex(&h[4..8]),
            slot(8),
            slot(16),
            slot(24),
            slot(32)
        );
    }
}

/// GetSecurityDescriptorOwner / Group: success, the SID pointer, the defaulted flag, the error.
struct SdGetSid {
    ok: i32,
    sid: *mut c_void,
    defaulted: BOOL,
    err: i64,
}

fn sd_get_owner(sd: *const c_void) -> SdGetSid {
    let mut sid = SD_SENTINEL as *mut c_void;
    let mut d: BOOL = 0x77;
    clear_error();
    // SAFETY: `sd` is a descriptor; both out pointers are valid.
    let ok = unsafe { GetSecurityDescriptorOwner(sd, &mut sid, &mut d) };
    SdGetSid { ok, sid, defaulted: d, err: gle_if(ok) }
}

fn sd_get_group(sd: *const c_void) -> SdGetSid {
    let mut sid = SD_SENTINEL as *mut c_void;
    let mut d: BOOL = 0x77;
    clear_error();
    // SAFETY: `sd` is a descriptor; both out pointers are valid.
    let ok = unsafe { GetSecurityDescriptorGroup(sd, &mut sid, &mut d) };
    SdGetSid { ok, sid, defaulted: d, err: gle_if(ok) }
}

/// GetSecurityDescriptorDacl / Sacl.
struct SdGetAcl {
    ok: i32,
    present: BOOL,
    acl: *mut c_void,
    defaulted: BOOL,
    err: i64,
}

fn sd_get_dacl(sd: *const c_void) -> SdGetAcl {
    let mut acl = SD_SENTINEL as *mut c_void;
    let mut present: BOOL = 0x66;
    let mut d: BOOL = 0x77;
    clear_error();
    // SAFETY: `sd` is a descriptor; the out pointers are valid.
    let ok = unsafe { GetSecurityDescriptorDacl(sd, &mut present, &mut acl, &mut d) };
    SdGetAcl { ok, present, acl, defaulted: d, err: gle_if(ok) }
}

fn sd_get_sacl(sd: *const c_void) -> SdGetAcl {
    let mut acl = SD_SENTINEL as *mut c_void;
    let mut present: BOOL = 0x66;
    let mut d: BOOL = 0x77;
    clear_error();
    // SAFETY: `sd` is a descriptor; the out pointers are valid.
    let ok = unsafe { GetSecurityDescriptorSacl(sd, &mut present, &mut acl, &mut d) };
    SdGetAcl { ok, present, acl, defaulted: d, err: gle_if(ok) }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn LocalFree(p: PVOID) -> PVOID;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
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
    fn IsValidSecurityDescriptor(sd: *const c_void) -> BOOL;
    fn GetSecurityDescriptorOwner(sd: *const c_void, owner: *mut PSID, defaulted: *mut BOOL) -> BOOL;
    fn GetSecurityDescriptorGroup(sd: *const c_void, group: *mut PSID, defaulted: *mut BOOL) -> BOOL;
    fn GetSecurityDescriptorDacl(sd: *const c_void, present: *mut BOOL, dacl: *mut PACL, defaulted: *mut BOOL) -> BOOL;
    fn GetSecurityDescriptorSacl(sd: *const c_void, present: *mut BOOL, sacl: *mut PACL, defaulted: *mut BOOL) -> BOOL;
    fn MakeAbsoluteSD(
        rel: *const c_void,
        abs: PSECURITY_DESCRIPTOR,
        abs_len: *mut u32,
        dacl: PACL,
        dacl_len: *mut u32,
        sacl: PACL,
        sacl_len: *mut u32,
        owner: PSID,
        owner_len: *mut u32,
        group: PSID,
        group_len: *mut u32,
    ) -> BOOL;
}

poly_wtest::main!(test);

fn run(t: &mut Test, s: &str) {
    let rel = sd_from(s);
    let mut abs = Raw::<64>::new();
    let mut dacl = Raw::<512>::new();
    let mut sacl = Raw::<512>::new();
    let mut owner = SidBuf::new();
    let mut group = SidBuf::new();
    abs.b.fill(0xCC);
    dacl.b.fill(0xCC);
    sacl.b.fill(0xCC);
    owner.b.fill(0xCC);
    group.b.fill(0xCC);
    let (mut la, mut ld, mut ls, mut lo, mut lg) = (64u32, 512u32, 512u32, 68u32, 68u32);
    clear_error();
    // SAFETY: all buffers are as large as the lengths given.
    let ok = unsafe {
        MakeAbsoluteSD(
            rel.ptr(),
            abs.p(),
            &mut la,
            dacl.p(),
            &mut ld,
            sacl.p(),
            &mut ls,
            owner.as_psid(),
            &mut lo,
            group.as_psid(),
            &mut lg,
        )
    };
    sdsay!(t, "{s:?}: ok={ok} err={} lengths abs={la} dacl={ld} sacl={ls} owner={lo} group={lg}", gle_if(ok));
    if ok == 0 {
        return;
    }
    sd_head(t, "  header", abs.b.as_ptr().cast());
    sdsay!(t, "  abs bytes 0..4: {}", Hex(&abs.b[..4]));
    let o = sd_get_owner(abs.b.as_ptr().cast());
    let g = sd_get_group(abs.b.as_ptr().cast());
    let d = sd_get_dacl(abs.b.as_ptr().cast());
    let sc = sd_get_sacl(abs.b.as_ptr().cast());
    sdsay!(t, "  owner is the owner buffer: {}", o.sid as usize == owner.b.as_ptr() as usize);
    sdsay!(t, "  group is the group buffer: {}", g.sid as usize == group.b.as_ptr() as usize);
    sdsay!(t, "  dacl is the dacl buffer: {} (present={})", d.acl as usize == dacl.b.as_ptr() as usize, d.present);
    sdsay!(t, "  sacl is the sacl buffer: {} (present={})", sc.acl as usize == sacl.b.as_ptr() as usize, sc.present);
    let n = ld as usize;
    sdsay!(t, "  dacl buffer first {} bytes: {} then {:02x}", n, Hex(&dacl.b[..n.min(512)]), dacl.b[n.min(511)]);
    let n = ls as usize;
    sdsay!(t, "  sacl buffer first {} bytes: {} then {:02x}", n, Hex(&sacl.b[..n.min(512)]), sacl.b[n.min(511)]);
    sdsay!(t, "  owner bytes: {} next {:02x}", Hex(&owner.b[..lo as usize]), owner.b[lo as usize]);
    sd_sddl_say(t, "  SDDL of the absolute descriptor", abs.b.as_ptr().cast(), 0xF);
    sdsay!(t, "  IsValidSecurityDescriptor: {}", unsafe_valid(abs.b.as_ptr().cast()));
}

fn unsafe_valid(sd: *const c_void) -> i32 {
    // SAFETY: a descriptor in a live buffer.
    unsafe { IsValidSecurityDescriptor(sd) }
}

fn test(t: &mut Test) {
    run(t, "O:BAG:SYD:(A;;GA;;;WD)S:(AU;SAFA;GA;;;WD)");
    run(t, "O:BA");
    run(t, "D:(A;;GA;;;WD)(D;;GW;;;BA)");
    run(t, "D:NO_ACCESS_CONTROL");
    run(t, "D:");
    run(t, "S:");
    run(t, "O:BAG:SYD:PAI(A;;GA;;;WD)S:AI(AU;SAFA;GA;;;WD)");
    run(t, "");
}
