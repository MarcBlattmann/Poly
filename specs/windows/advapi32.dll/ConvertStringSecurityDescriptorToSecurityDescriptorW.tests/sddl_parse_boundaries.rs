//! Where one part of an SDDL string ends and the next begins: a numeric SID directly followed by another part,
//! aliases followed by parts, and white space (space, tab, newline, carriage return) after a SID, after ACL
//! flags and after the last ACE.
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

const SDDL_REVISION_1: u32 = 1;
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

/// The printable owner or group: masked SID text, or "(null)".
fn sd_sid_str(p: *const c_void) -> Buf<200> {
    if p.is_null() {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "(null)");
        return b;
    }
    mask_sid(p)
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
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        s: *const u16,
        revision: u32,
        sd: *mut PSECURITY_DESCRIPTOR,
        size: *mut u32,
    ) -> BOOL;
    fn GetSecurityDescriptorOwner(sd: *const c_void, owner: *mut PSID, defaulted: *mut BOOL) -> BOOL;
    fn GetSecurityDescriptorGroup(sd: *const c_void, group: *mut PSID, defaulted: *mut BOOL) -> BOOL;
    fn GetSecurityDescriptorDacl(sd: *const c_void, present: *mut BOOL, dacl: *mut PACL, defaulted: *mut BOOL) -> BOOL;
    fn GetSecurityDescriptorSacl(sd: *const c_void, present: *mut BOOL, sacl: *mut PACL, defaulted: *mut BOOL) -> BOOL;
}

poly_wtest::main!(test);

fn show(t: &mut Test, s: &str) {
    let sd = sd_from(s);
    if sd.ok == 0 {
        sdsay!(t, "{s:?}: failed err={}", sd.err);
        return;
    }
    let o = sd_get_owner(sd.ptr());
    let g = sd_get_group(sd.ptr());
    let d = sd_get_dacl(sd.ptr());
    let sc = sd_get_sacl(sd.ptr());
    let count = |present: i32, acl: *const c_void| -> i64 {
        if present == 0 {
            -1
        } else if acl.is_null() {
            -2
        } else {
            // SAFETY: an ACL header of 8 bytes inside the descriptor.
            let h = unsafe { core::slice::from_raw_parts(acl as *const u8, 8) };
            i64::from(u16::from_le_bytes([h[4], h[5]]))
        }
    };
    sdsay!(
        t,
        "{s:?}: ok size={} control={:#06x} owner={} group={} dacl aces={} sacl aces={}",
        sd.size,
        u16::from_le_bytes([sd.bytes()[2], sd.bytes()[3]]),
        sd_sid_str(o.sid).as_str(),
        sd_sid_str(g.sid).as_str(),
        count(d.present, d.acl),
        count(sc.present, sc.acl)
    );
}

fn test(t: &mut Test) {
    sdsay!(t, "-- a numeric SID directly followed by the next part");
    for s in [
        "O:S-1-5-19G:S-1-5-20",
        "O:S-1-5-19D:(A;;FR;;;WD)",
        "O:S-1-5-19S:(ML;;NW;;;LW)",
        "G:S-1-5-32-545D:P",
        "G:S-1-5-32-545S:",
        "O:S-1-5-19D:",
        "O:S-1-5-0x13D:(A;;FR;;;WD)",
        "O:S-1-5-0xAD:(A;;FR;;;WD)",
        "O:S-1-5-19 D:(A;;FR;;;WD)",
        "O:S-1-5-19D",
        "O:S-1-5-19G",
        "O:S-1-5-19S",
        "O:S-1-5-19DG:SY",
        "O:S-1-5-19O:SY",
        "D:(A;;FR;;;S-1-5-19)(A;;FW;;;S-1-5-20)S:(ML;;NW;;;S-1-16-4096)",
        "D:(A;;FR;;;S-1-5-19)O:S-1-5-20G:S-1-5-19",
    ] {
        show(t, s);
    }
    sdsay!(t, "-- aliases followed by parts");
    for s in
        ["O:SYD:(A;;FR;;;WD)", "O:SYS:(ML;;NW;;;LW)", "O:LSG:NSD:S:", "G:BUO:BAS:D:", "O:SYG:BAD:PAI(A;;FR;;;WD)S:AI"]
    {
        show(t, s);
    }
    sdsay!(t, "-- white space after a part");
    for (name, ws) in [
        ("space", " "),
        ("tab", "\t"),
        ("newline", "\n"),
        ("carriage return", "\r"),
        ("CR LF", "\r\n"),
        ("two spaces", "  "),
    ] {
        for head in [
            "G:SY",
            "D:(A;;FR;;;WD)",
            "S:(ML;;NW;;;LW)",
            "D:",
            "D:P",
            "O:SYD:(A;;FR;;;WD)(A;;FW;;;BA)",
            "D:NO_ACCESS_CONTROL",
        ] {
            let mut text = Buf::<120>::new();
            let _ = write!(text, "{head}{ws}");
            sdsay!(t, "{name} after {head:?}:");
            show(t, text.as_str());
        }
        let mut text = Buf::<120>::new();
        let _ = write!(text, "G:SY{ws}D:(A;;FR;;;WD){ws}S:(ML;;NW;;;LW)");
        sdsay!(t, "{name} between three parts:");
        show(t, text.as_str());
        let mut text = Buf::<120>::new();
        let _ = write!(text, "D:(A;;FR;;;WD){ws}(A;;FW;;;BA)");
        sdsay!(t, "{name} between two ACEs:");
        show(t, text.as_str());
        let mut text = Buf::<120>::new();
        let _ = write!(text, "{ws}D:(A;;FR;;;WD)");
        sdsay!(t, "{name} before the string:");
        show(t, text.as_str());
    }
}
