//! RtlAdjustPrivilege: privileges the token does not hold, privilege numbers that do not exist, the
//! previous-state output on failure, and CurrentThread TRUE without a thread token. Ambient: the oracle's
//! elevated administrator token does not hold SeTcbPrivilege (7) or SeCreateTokenPrivilege (2).
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
const TRUE: BOOL = 1;
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
const TOKEN_PRIVILEGES_CLASS: u32 = 3;
/// winnt.h: `LUID`.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct LUID {
    low: u32,
    high: i32,
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
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut LUID) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlAdjustPrivilege(privilege: u32, enable: u8, current_thread: u8, was_enabled: *mut u8) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let tok = process_token(TOKEN_QUERY);
    for n in [0u32, 1, 2, 3, 7, 9, 36, 37, 38, 100, 0xFFFF_FFFF] {
        for enable in [1u8, 0] {
            let mut was = 0xAAu8;
            // SAFETY: valid out pointer.
            let st = unsafe { RtlAdjustPrivilege(n, enable, 0, &mut was) };
            say!(t, "privilege {n} enable={enable}: status={:#010x} previous={was:#x}", st as u32);
        }
    }
    for (label, current_thread) in [("thread TRUE", 1u8), ("thread 2", 2)] {
        let mut was = 0xAAu8;
        // SAFETY: valid out pointer; this thread has no token of its own.
        let st = unsafe { RtlAdjustPrivilege(23, 1, current_thread, &mut was) };
        say!(t, "SeChangeNotifyPrivilege, {label}, no thread token: status={:#010x} previous={was:#x}", st as u32);
    }
    for enable in [2u8, 0xFF] {
        let mut was = 0xAAu8;
        // SAFETY: valid out pointer.
        let st = unsafe { RtlAdjustPrivilege(19, enable, 0, &mut was) };
        say!(
            t,
            "SeShutdownPrivilege enable={enable:#x}: status={:#010x} previous={was:#x} attrs now={}",
            st as u32,
            privilege_attrs(tok, "SeShutdownPrivilege")
        );
    }
    // SAFETY: closing the handle.
    unsafe { CloseHandle(tok) };
}
