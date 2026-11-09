//! SetKernelObjectSecurity with a mandatory label (what Chromium's sandbox does): LABEL_SECURITY_INFORMATION
//! with a descriptor holding S:(ML;;NW;;;LW), reading it back, removing it, labels above the caller's level,
//! SACL information without the privilege. (ambient: the caller is a High integrity administrator)
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

const ACCESS_SYSTEM_SECURITY: u32 = 0x0100_0000;
const DACL_SECURITY_INFORMATION: u32 = 0x4;
const SACL_SECURITY_INFORMATION: u32 = 0x8;
const LABEL_SECURITY_INFORMATION: u32 = 0x10;
const SDDL_REVISION_1: u32 = 1;
/// winbase.h: `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: PVOID,
    inherit: BOOL,
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

/// GetSecurityDescriptorControl: (success, control, revision, error); the outputs start as 0xBEEF and 0x77.
fn sd_get_control(sd: *const c_void) -> (i32, u16, u32, i64) {
    let mut c = 0xBEEFu16;
    let mut r = 0x77u32;
    clear_error();
    // SAFETY: `sd` is a descriptor; the out pointers are valid.
    let ok = unsafe { GetSecurityDescriptorControl(sd, &mut c, &mut r) };
    (ok, c, r, gle_if(ok))
}

fn sd_say_control(t: &mut Test, label: &str, sd: *const c_void) {
    let (ok, c, r, err) = sd_get_control(sd);
    sdsay!(t, "{label}: ok={ok} control={c:#06x} [{}] revision={r:#x} err={err}", sd_ctrl_names(c).as_str());
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
    fn GetCurrentProcessId() -> u32;
    fn CreateEventW(attrs: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
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
    fn GetSecurityDescriptorControl(sd: *const c_void, control: *mut u16, revision: *mut u32) -> BOOL;
    fn GetKernelObjectSecurity(h: HANDLE, info: u32, sd: PSECURITY_DESCRIPTOR, len: u32, needed: *mut u32) -> BOOL;
    fn SetKernelObjectSecurity(h: HANDLE, info: u32, sd: *const c_void) -> BOOL;
}

poly_wtest::main!(test);

fn label_text(t: &mut Test, label: &str, h: HANDLE) {
    let r = sd_get_kernel(h, LABEL_SECURITY_INFORMATION);
    if r.ok == 0 {
        sdsay!(t, "{label}: get failed err={}", r.err);
        return;
    }
    let sd = r.buf.b.as_ptr().cast::<c_void>();
    sd_sddl_say(t, label, sd, LABEL_SECURITY_INFORMATION);
    sd_say_control(t, "    control", sd);
}

fn test(t: &mut Test) {
    let ev = sd_event();
    label_text(t, "initial", ev);
    for s in [
        "S:(ML;;NW;;;LW)",
        "S:(ML;;NWNRNX;;;LW)",
        "S:(ML;;NW;;;ME)",
        "S:(ML;;NW;;;HI)",
        "S:(ML;;NW;;;SI)",
        "S:(ML;;NW;;;S-1-16-0)",
        "S:(ML;;;;;LW)",
        "S:(ML;;NR;;;LW)",
        "S:(ML;;NW;;;LW)(ML;;NR;;;ME)",
        "S:",
        "S:NO_ACCESS_CONTROL",
        "S:(ML;;NW;;;LW)",
    ] {
        let sd = sd_from(s);
        let mut l = Buf::<100>::new();
        let _ = write!(l, "set {s} (information LABEL)");
        sd_sko_say(t, l.as_str(), ev, LABEL_SECURITY_INFORMATION, sd.ptr());
        label_text(t, "  now", ev);
    }
    // SACL information (audit ACEs) needs the privilege and ACCESS_SYSTEM_SECURITY.
    let sd = sd_from("S:(AU;SAFA;0x1;;;WD)");
    sd_sko_say(t, "set S:(AU;SAFA;...) with information SACL", ev, SACL_SECURITY_INFORMATION, sd.ptr());
    let sd = sd_from("S:(ML;;NW;;;LW)");
    sd_sko_say(t, "set S:(ML;...) with information SACL", ev, SACL_SECURITY_INFORMATION, sd.ptr());
    sd_sko_say(
        t,
        "set S:(ML;...) with information SACL|LABEL",
        ev,
        SACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION,
        sd.ptr(),
    );
    label_text(t, "  now", ev);
    // A descriptor with an audit ACE and a label, information = LABEL only.
    let sd = sd_from("S:(AU;SAFA;0x1;;;WD)(ML;;NW;;;ME)");
    sd_sko_say(t, "set S:(AU;..)(ML;..ME) with information LABEL", ev, LABEL_SECURITY_INFORMATION, sd.ptr());
    label_text(t, "  now", ev);
    // A DACL in the same descriptor with LABEL only.
    let sd = sd_from("D:(A;;0x1f0003;;;WD)S:(ML;;NW;;;LW)");
    sd_sko_say(
        t,
        "set D:..S:(ML;..LW) with information LABEL|DACL",
        ev,
        LABEL_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
        sd.ptr(),
    );
    label_text(t, "  now", ev);
    sd_kernel_text(t, "  dacl now", ev, 4);
    // SAFETY: closing the handle.
    unsafe { CloseHandle(ev) };
}
