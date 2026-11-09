//! ACL flags after "D:" and "S:" (P, AR, AI, NO_ACCESS_CONTROL, combinations, placement relative to the ACEs):
//! the control word, whether the ACL is present, and the number of ACEs.
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

/// GetSecurityDescriptorControl: (success, control, revision, error); the outputs start as 0xBEEF and 0x77.
fn sd_get_control(sd: *const c_void) -> (i32, u16, u32, i64) {
    let mut c = 0xBEEFu16;
    let mut r = 0x77u32;
    clear_error();
    // SAFETY: `sd` is a descriptor; the out pointers are valid.
    let ok = unsafe { GetSecurityDescriptorControl(sd, &mut c, &mut r) };
    (ok, c, r, gle_if(ok))
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
    fn GetSecurityDescriptorDacl(sd: *const c_void, present: *mut BOOL, dacl: *mut PACL, defaulted: *mut BOOL) -> BOOL;
    fn GetSecurityDescriptorSacl(sd: *const c_void, present: *mut BOOL, sacl: *mut PACL, defaulted: *mut BOOL) -> BOOL;
    fn GetSecurityDescriptorControl(sd: *const c_void, control: *mut u16, revision: *mut u32) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for s in [
        "D:",
        "D:P",
        "D:AR",
        "D:AI",
        "D:PAI",
        "D:PAR",
        "D:ARAI",
        "D:PARAI",
        "D:AIP",
        "D:ARP",
        "D:PP",
        "D:AIAI",
        "D:NO_ACCESS_CONTROL",
        "D:PNO_ACCESS_CONTROL",
        "D:NO_ACCESS_CONTROLP",
        "D:PAINO_ACCESS_CONTROL",
        "D:NULL",
        "D:NO_ACCESS_CONTROL(A;;GA;;;WD)",
        "D:P(A;;GA;;;WD)",
        "D:AI(A;;GA;;;WD)",
        "D:(A;;GA;;;WD)P",
        "D:(A;;GA;;;WD)AI",
        "D:PAI(A;;GA;;;WD)(D;;GW;;;BA)",
        "D:p",
        "D:ai",
        "D:Z",
        "D:PZ",
        "S:",
        "S:P",
        "S:AR",
        "S:AI",
        "S:PAI",
        "S:PARAI",
        "S:NO_ACCESS_CONTROL",
        "S:PNO_ACCESS_CONTROL",
        "S:NULL",
        "S:P(AU;SAFA;GA;;;WD)",
        "D:PS:P",
        "D:AIS:AI",
        "D:PARAIS:PARAI",
        "D:NO_ACCESS_CONTROLS:NO_ACCESS_CONTROL",
        "O:BAG:SYD:PAI(A;;GA;;;WD)S:AR(AU;SAFA;GA;;;WD)",
    ] {
        let sd = sd_from(s);
        if sd.ok == 0 {
            sdsay!(t, "{s:?}: failed err={}", sd.err);
            continue;
        }
        let (_, c, _, _) = sd_get_control(sd.ptr());
        let d = sd_get_dacl(sd.ptr());
        let sc = sd_get_sacl(sd.ptr());
        let count = |acl: *mut c_void, present: BOOL| -> i64 {
            if present == 0 || acl.is_null() {
                -1
            } else {
                // SAFETY: an ACL header inside the descriptor.
                i64::from(unsafe { u16::from_le_bytes([*(acl as *const u8).add(4), *(acl as *const u8).add(5)]) })
            }
        };
        sdsay!(
            t,
            "{s:?}: control={c:#06x} [{}] dacl present={} aces={} sacl present={} aces={} back={}",
            sd_ctrl_names(c).as_str(),
            d.present,
            count(d.acl, d.present),
            sc.present,
            count(sc.acl, sc.present),
            sd_sddl(sd.ptr(), 0x1F).text.as_str()
        );
    }
}
