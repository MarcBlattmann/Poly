//! CreateRestrictedToken privilege handling: DISABLE_MAX_PRIVILEGE deletes every privilege except
//! SeChangeNotifyPrivilege; PrivilegesToDelete deletes the ones named; flag 0 with nothing else
//! keeps the token as it is.
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

const TOKEN_PRIVILEGES_CLASS: u32 = 3;
const TOKEN_TYPE_CLASS: u32 = 8;
const TOKEN_HAS_RESTRICTIONS_CLASS: u32 = 21;
const DISABLE_MAX_PRIVILEGE: u32 = 0x1;
/// winnt.h: `LUID`.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct LUID {
    low: u32,
    high: i32,
}

/// winnt.h: `SID_AND_ATTRIBUTES`.
#[repr(C)]
#[derive(Clone, Copy)]
struct SID_AND_ATTRIBUTES {
    sid: PSID,
    attributes: u32,
}

/// winnt.h: `LUID_AND_ATTRIBUTES`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct LUID_AND_ATTRIBUTES {
    luid: LUID,
    attributes: u32,
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

/// A 32-bit token information value (class with a single DWORD), or -1 when the query fails.
fn tok_u32(token: HANDLE, class: u32) -> i64 {
    let mut raw = Raw::<64>::new();
    let mut ret = 0u32;
    // SAFETY: `raw` is 64 writable bytes; the length given is the 4 bytes the classes need.
    let ok = unsafe { GetTokenInformation(token, class, raw.p(), 4, &mut ret) };
    if ok != 0 { i64::from(raw.u32_at(0)) } else { -1 }
}

/// Print a TOKEN_PRIVILEGES result: "label[i]: name attrs".
fn print_privileges(t: &mut Test, label: &str, raw: &Raw<4096>) {
    let n = raw.u32_at(0) as usize;
    say!(t, "{label}: count={n}");
    for i in 0..n.min(40) {
        let off = 4 + 12 * i;
        let luid = LUID { low: raw.u32_at(off), high: raw.u32_at(off + 4) as i32 };
        let attrs = raw.u32_at(off + 8);
        let mut name = [0u16; 64];
        let mut len = 64u32;
        // SAFETY: valid LUID and buffer.
        let ok = unsafe { LookupPrivilegeNameW(core::ptr::null(), &luid, name.as_mut_ptr(), &mut len) };
        if ok != 0 {
            say!(t, "{label}[{i}]: {} attrs={attrs:#x}", wide_text::<64>(name.as_ptr()).as_str());
        } else {
            say!(t, "{label}[{i}]: luid {}:{} attrs={attrs:#x}", luid.high, luid.low);
        }
    }
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetTokenInformation(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> BOOL;
    fn CreateRestrictedToken(
        token: HANDLE,
        flags: u32,
        disable_count: u32,
        disable: *const SID_AND_ATTRIBUTES,
        delete_count: u32,
        delete: *const LUID_AND_ATTRIBUTES,
        restrict_count: u32,
        restrict: *const SID_AND_ATTRIBUTES,
        out: *mut HANDLE,
    ) -> BOOL;
    fn IsTokenRestricted(token: HANDLE) -> BOOL;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut LUID) -> BOOL;
    fn LookupPrivilegeNameW(system: *const u16, luid: *const LUID, name: *mut u16, len: *mut u32) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let src = process_token(TOKEN_ALL_ACCESS);
    let mut orig = Raw::<4096>::new();
    token_info(src, TOKEN_PRIVILEGES_CLASS, &mut orig);
    say!(t, "original has {} privileges", if orig.u32_at(0) > 3 { "more than three" } else { "three or fewer" });
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: valid token and out pointer; no arrays.
    let ok = unsafe {
        CreateRestrictedToken(src, 0, 0, core::ptr::null(), 0, core::ptr::null(), 0, core::ptr::null(), &mut out)
    };
    let mut r = Raw::<4096>::new();
    token_info(out, TOKEN_PRIVILEGES_CLASS, &mut r);
    say!(t, "flags 0: ok={ok} same privilege count as the original: {}", r.u32_at(0) == orig.u32_at(0));
    say!(
        t,
        "flags 0: type={} restricted={} hasrestrictions={}",
        tok_u32(out, TOKEN_TYPE_CLASS),
        unsafe { IsTokenRestricted(out) },
        tok_u32(out, TOKEN_HAS_RESTRICTIONS_CLASS)
    );
    // SAFETY: closing the handle.
    unsafe { CloseHandle(out) };
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: valid token and out pointer; no arrays.
    let ok = unsafe {
        CreateRestrictedToken(
            src,
            DISABLE_MAX_PRIVILEGE,
            0,
            core::ptr::null(),
            0,
            core::ptr::null(),
            0,
            core::ptr::null(),
            &mut out,
        )
    };
    token_info(out, TOKEN_PRIVILEGES_CLASS, &mut r);
    say!(t, "DISABLE_MAX_PRIVILEGE: ok={ok}");
    print_privileges(t, "  after", &r);
    // SAFETY: closing the handle.
    unsafe { CloseHandle(out) };
    let mut del = [LUID_AND_ATTRIBUTES::default(); 3];
    for (i, n) in ["SeBackupPrivilege", "SeRestorePrivilege", "SeFooPrivilegeNotReal"].iter().enumerate().take(2) {
        let w = wide(n);
        // SAFETY: valid name and out LUID.
        unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut del[i].luid) };
    }
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: two valid entries.
    let ok =
        unsafe { CreateRestrictedToken(src, 0, 0, core::ptr::null(), 2, del.as_ptr(), 0, core::ptr::null(), &mut out) };
    say!(t, "delete backup and restore: ok={ok} err={}", gle_if(ok));
    say!(
        t,
        "  backup attrs={} restore attrs={} shutdown attrs={} count {} -> {}",
        privilege_attrs(out, "SeBackupPrivilege"),
        privilege_attrs(out, "SeRestorePrivilege"),
        privilege_attrs(out, "SeShutdownPrivilege"),
        orig.u32_at(0),
        {
            token_info(out, TOKEN_PRIVILEGES_CLASS, &mut r);
            r.u32_at(0)
        }
    );
    // SAFETY: closing the handle.
    unsafe { CloseHandle(out) };
    // A privilege the token does not hold, and one deleted twice.
    let mut unheld = [LUID_AND_ATTRIBUTES::default(); 2];
    let w = wide("SeTcbPrivilege");
    // SAFETY: valid name and out LUID.
    unsafe { LookupPrivilegeValueW(core::ptr::null(), w.as_ptr(), &mut unheld[0].luid) };
    unheld[1] = unheld[0];
    let mut out: HANDLE = core::ptr::null_mut();
    clear_error();
    // SAFETY: two valid entries.
    let ok = unsafe {
        CreateRestrictedToken(src, 0, 0, core::ptr::null(), 2, unheld.as_ptr(), 0, core::ptr::null(), &mut out)
    };
    say!(t, "delete a privilege the token lacks (twice): ok={ok} err={}", gle_if(ok));
    if ok != 0 {
        // SAFETY: closing the handle.
        unsafe { CloseHandle(out) };
    }
    // SAFETY: closing the source.
    unsafe { CloseHandle(src) };
}
