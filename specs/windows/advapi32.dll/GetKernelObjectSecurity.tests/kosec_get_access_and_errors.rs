//! GetKernelObjectSecurity access rights and errors: a handle with READ_CONTROL only, with SYNCHRONIZE only, the
//! SACL without ACCESS_SYSTEM_SECURITY (before and after enabling SeSecurityPrivilege), invalid handles,
//! information values with unknown bits.
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
const INVALID_HANDLE: HANDLE = -1isize as HANDLE;
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

const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
const WRITE_DAC: u32 = 0x0004_0000;
const READ_CONTROL: u32 = 0x0002_0000;
const ACCESS_SYSTEM_SECURITY: u32 = 0x0100_0000;
const TOKEN_PRIVILEGES_CLASS: u32 = 3;
const SE_PRIVILEGE_ENABLED: u32 = 0x2;
const SDDL_REVISION_1: u32 = 1;
/// winnt.h: `LUID`.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct LUID {
    low: u32,
    high: i32,
}

/// winnt.h: `LUID_AND_ATTRIBUTES`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct LUID_AND_ATTRIBUTES {
    luid: LUID,
    attributes: u32,
}

/// winnt.h: `TOKEN_PRIVILEGES` with room for `N` privileges.
#[repr(C)]
#[derive(Clone, Copy)]
struct TOKEN_PRIVILEGES_N<const N: usize> {
    count: u32,
    privileges: [LUID_AND_ATTRIBUTES; N],
}

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

/// The process token with `access`, or null.
fn process_token(access: u32) -> HANDLE {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: the current process pseudo-handle and a valid out pointer.
    unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut h) };
    h
}

/// `GetTokenInformation` into `buf`; (success, needed length, last error).
fn token_info<const N: usize>(token: HANDLE, class: u32, buf: &mut Raw<N>) -> (bool, u32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    clear_error();
    // SAFETY: `buf` is N writable bytes, `ret` a valid out pointer.
    let ok = unsafe { GetTokenInformation(token, class, buf.p(), N as u32, &mut ret) } != 0;
    (ok, ret, gle())
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

/// Whether the token has the privilege (by name) and its attributes, or -1.
fn privilege_attrs(token: HANDLE, name: &str) -> i64 {
    let w = wide(name);
    let mut luid = LUID::default();
    // SAFETY: NUL-terminated name, valid out pointer.
    unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut luid) };
    let mut raw = Raw::<4096>::new();
    let (ok, _, _) = token_info(token, TOKEN_PRIVILEGES_CLASS, &mut raw);
    if !ok {
        return -2;
    }
    let n = raw.u32_at(0) as usize;
    for i in 0..n.min(60) {
        let off = 4 + 12 * i;
        if raw.u32_at(off) == luid.low && raw.u32_at(off + 4) as i32 == luid.high {
            return i64::from(raw.u32_at(off + 8));
        }
    }
    -1
}

/// `say!` with a longer line (SDDL texts, hex dumps).
macro_rules! sdsay {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<2000>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const EVENT_MODIFY_STATE: u32 = 0x2;
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

/// Enables or disables a privilege of a token by name.
fn sd_priv(token: HANDLE, name: &str, enable: bool) -> bool {
    let w = wide(name);
    let mut tp = TOKEN_PRIVILEGES_N::<1> { count: 1, privileges: [LUID_AND_ATTRIBUTES::default()] };
    // SAFETY: NUL-terminated name, valid out pointer.
    unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut tp.privileges[0].luid) };
    tp.privileges[0].attributes = if enable { SE_PRIVILEGE_ENABLED } else { 0 };
    // SAFETY: a one-entry TOKEN_PRIVILEGES.
    unsafe {
        AdjustTokenPrivileges(
            token,
            FALSE,
            (&tp as *const TOKEN_PRIVILEGES_N<1>).cast(),
            0,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        ) != 0
    }
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
    fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
        sd: *const c_void,
        revision: u32,
        info: u32,
        out: *mut *mut u16,
        len: *mut u32,
    ) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> BOOL;
    fn AdjustTokenPrivileges(
        token: HANDLE,
        disable_all: BOOL,
        new_state: *const c_void,
        buf_len: u32,
        previous: *mut c_void,
        ret_len: *mut u32,
    ) -> BOOL;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut LUID) -> BOOL;
    fn GetKernelObjectSecurity(h: HANDLE, info: u32, sd: PSECURITY_DESCRIPTOR, len: u32, needed: *mut u32) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryObject(h: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
}

poly_wtest::main!(test);

fn try_get(t: &mut Test, label: &str, h: HANDLE, info: u32) {
    let r = sd_get_kernel(h, info);
    if r.ok != 0 {
        sdsay!(t, "{label}: ok=1 needed={}", r.needed);
    } else {
        sdsay!(t, "{label}: ok=0 err={} needed={:#x}", r.err, r.needed);
    }
}

fn test(t: &mut Test) {
    let ev = sd_event();
    let rc = sd_dup_handle(ev, READ_CONTROL);
    let sync = sd_dup_handle(ev, SYNCHRONIZE);
    let wdac = sd_dup_handle(ev, WRITE_DAC);
    let none = sd_dup_handle(ev, EVENT_MODIFY_STATE);
    sdsay!(
        t,
        "handle accesses: rc={:#x} sync={:#x} wdac={:#x} modify={:#x}",
        granted_access(rc),
        granted_access(sync),
        granted_access(wdac),
        granted_access(none)
    );
    for (name, h) in [
        ("full access", ev),
        ("READ_CONTROL only", rc),
        ("SYNCHRONIZE only", sync),
        ("WRITE_DAC only", wdac),
        ("EVENT_MODIFY_STATE only", none),
    ] {
        sdsay!(t, "== {name}");
        for (what, info) in [
            ("owner", 1u32),
            ("group", 2),
            ("dacl", 4),
            ("sacl", 8),
            ("label", 0x10),
            ("owner+group+dacl", 7),
            ("owner+group+dacl+sacl", 0xF),
        ] {
            let mut label = Buf::<60>::new();
            let _ = write!(label, "  {what}");
            try_get(t, label.as_str(), h, info);
        }
    }
    // ACCESS_SYSTEM_SECURITY needs the privilege to be enabled when the handle is made.
    let token = process_token(TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY);
    let was = privilege_attrs(token, "SeSecurityPrivilege");
    sdsay!(t, "SeSecurityPrivilege in the process token: attributes {was:#x}");
    let enabled = sd_priv(token, "SeSecurityPrivilege", true);
    let sys = sd_dup_handle(ev, ACCESS_SYSTEM_SECURITY | READ_CONTROL);
    sdsay!(t, "privilege enabled: {enabled}; handle with ACCESS_SYSTEM_SECURITY made: {}", !sys.is_null());
    if !sys.is_null() {
        sdsay!(t, "granted access {:#x}", granted_access(sys));
    }
    try_get(t, "sacl through the privileged handle (privilege enabled)", sys, 8);
    try_get(t, "sacl through the full-access handle (privilege enabled)", ev, 8);
    sd_priv(token, "SeSecurityPrivilege", false);
    try_get(t, "sacl through the privileged handle (privilege disabled again)", sys, 8);
    try_get(t, "sacl through the full-access handle (privilege disabled again)", ev, 8);
    if !sys.is_null() {
        sd_kernel_text(t, "sacl text via privileged handle", sys, 8);
    }
    // Errors
    sdsay!(t, "== errors");
    try_get(t, "NULL handle", core::ptr::null_mut(), 7);
    try_get(t, "handle 0x1234", 0x1234 as HANDLE, 7);
    try_get(t, "INVALID_HANDLE_VALUE (-1 is the current process)", INVALID_HANDLE, 7);
    try_get(t, "information 0", ev, 0);
    try_get(t, "information 0x20 (attribute)", ev, 0x20);
    try_get(t, "information 0x40 (scope)", ev, 0x40);
    try_get(t, "information 0x80", ev, 0x80);
    try_get(t, "information 0x100", ev, 0x100);
    try_get(t, "information 0x10000", ev, 0x10000);
    try_get(t, "information 0x8000_0000 (protected dacl)", ev, 0x8000_0000);
    try_get(t, "information 0x4000_0000", ev, 0x4000_0000);
    try_get(t, "information 0xFFFF_FFFF", ev, 0xFFFF_FFFF);
    try_get(t, "information dacl | 0x8000_0000", ev, 0x8000_0004);
    // SAFETY: closing the handles.
    unsafe {
        CloseHandle(token);
        CloseHandle(ev);
        CloseHandle(rc);
        CloseHandle(sync);
        CloseHandle(wdac);
        CloseHandle(none);
        if !sys.is_null() {
            CloseHandle(sys);
        }
    }
}
