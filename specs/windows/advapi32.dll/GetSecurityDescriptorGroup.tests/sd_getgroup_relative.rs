//! GetSecurityDescriptorGroup on self-relative descriptors: the SID pointer is the descriptor's address plus
//! the stored offset; an absent group gives NULL; the defaulted flag comes from the control word.
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

const SE_SELF_RELATIVE: u16 = 0x8000;
const SDDL_REVISION_1: u32 = 1;
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

/// The bytes of the little-endian u32 at `off`.
fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
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

/// The printable owner or group: masked SID text, or "(null)".
fn sd_sid_str(p: *const c_void) -> Buf<200> {
    if p.is_null() {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "(null)");
        return b;
    }
    mask_sid(p)
}

/// The pointer `p` against the descriptor at `base` (`len` bytes): null, the offset inside, the
/// untouched sentinel or elsewhere.
fn sd_where(p: *const c_void, base: *const c_void, len: usize) -> Buf<40> {
    let mut b = Buf::<40>::new();
    if p.is_null() {
        let _ = write!(b, "null");
    } else if p as usize == SD_SENTINEL {
        let _ = write!(b, "unchanged");
    } else {
        let d = (p as usize).wrapping_sub(base as usize);
        if d < len {
            let _ = write!(b, "inside+{d}");
        } else {
            let _ = write!(b, "elsewhere");
        }
    }
    b
}

/// GetSecurityDescriptorOwner / Group: success, the SID pointer, the defaulted flag, the error.
struct SdGetSid {
    ok: i32,
    sid: *mut c_void,
    defaulted: BOOL,
    err: i64,
}

fn sd_get_group(sd: *const c_void) -> SdGetSid {
    let mut sid = SD_SENTINEL as *mut c_void;
    let mut d: BOOL = 0x77;
    clear_error();
    // SAFETY: `sd` is a descriptor; both out pointers are valid.
    let ok = unsafe { GetSecurityDescriptorGroup(sd, &mut sid, &mut d) };
    SdGetSid { ok, sid, defaulted: d, err: gle_if(ok) }
}

/// Prints an SdGetSid result against the descriptor at `base`/`len`.
fn sd_say_sid(t: &mut Test, label: &str, r: &SdGetSid, base: *const c_void, len: usize) {
    if r.ok != 0 {
        let text = if (r.sid as usize) == SD_SENTINEL { sd_sid_str(core::ptr::null()) } else { sd_sid_str(r.sid) };
        sdsay!(
            t,
            "{label}: ok=1 where={} sid={} defaulted={}",
            sd_where(r.sid, base, len).as_str(),
            text.as_str(),
            r.defaulted
        );
    } else {
        sdsay!(
            t,
            "{label}: ok=0 err={} where={} defaulted={}",
            r.err,
            sd_where(r.sid, base, len).as_str(),
            r.defaulted
        );
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
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        s: *const u16,
        revision: u32,
        sd: *mut PSECURITY_DESCRIPTOR,
        size: *mut u32,
    ) -> BOOL;
    fn GetSecurityDescriptorOwner(sd: *const c_void, owner: *mut PSID, defaulted: *mut BOOL) -> BOOL;
    fn GetSecurityDescriptorGroup(sd: *const c_void, group: *mut PSID, defaulted: *mut BOOL) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for s in ["G:SY", "O:BAG:SY", "O:BA", "O:SYG:BAD:(A;;GA;;;WD)", "D:(A;;GA;;;WD)", "G:S-1-5-21-1-2-3-513"] {
        let sd = sd_from(s);
        let mut label = Buf::<80>::new();
        let _ = write!(label, "{s:?}");
        sd_head(t, label.as_str(), sd.ptr());
        sd_say_sid(t, "  group", &sd_get_group(sd.ptr()), sd.ptr(), sd.size as usize);
    }
    let sd = sd_from("O:BAG:SY");
    let mut buf = Raw::<512>::new();
    buf.b[..sd.bytes().len()].copy_from_slice(sd.bytes());
    buf.b[2] |= 0x02;
    sd_say_sid(t, "GROUP_DEFAULTED set by hand", &sd_get_group(buf.b.as_ptr().cast()), buf.b.as_ptr().cast(), 512);
    buf.b[2] = 0x01;
    sd_say_sid(t, "OWNER_DEFAULTED only", &sd_get_group(buf.b.as_ptr().cast()), buf.b.as_ptr().cast(), 512);
    buf.b[8..12].copy_from_slice(&0u32.to_le_bytes());
    sd_say_sid(t, "group offset 0", &sd_get_group(buf.b.as_ptr().cast()), buf.b.as_ptr().cast(), 512);
}
