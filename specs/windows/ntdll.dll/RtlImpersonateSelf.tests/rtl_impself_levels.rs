//! RtlImpersonateSelf: levels 0 to 3 put a copy of the process token on the thread at that level; 4 and
//! above fail with an NTSTATUS; RevertToSelf removes it.
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
type DWORD = u32;
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
const TOKEN_TYPE_CLASS: u32 = 8;
const TOKEN_IMPERSONATION_LEVEL_CLASS: u32 = 9;
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

/// A 32-bit token information value (class with a single DWORD), or -1 when the query fails.
fn tok_u32(token: HANDLE, class: u32) -> i64 {
    let mut raw = Raw::<64>::new();
    let mut ret = 0u32;
    // SAFETY: `raw` is 64 writable bytes; the length given is the 4 bytes the classes need.
    let ok = unsafe { GetTokenInformation(token, class, raw.p(), 4, &mut ret) };
    if ok != 0 { i64::from(raw.u32_at(0)) } else { -1 }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentThread() -> HANDLE;
    fn OpenThreadToken(thread: HANDLE, access: u32, as_self: BOOL, token: *mut HANDLE) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetTokenInformation(token: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> BOOL;
    fn RevertToSelf() -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlImpersonateSelf(level: u32) -> i32;
}

poly_wtest::main!(test);

fn thread_level() -> i64 {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: the thread pseudo-handle and a valid out pointer.
    let ok = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &mut h) };
    if ok == 0 {
        return -i64::from(gle());
    }
    let l = tok_u32(h, TOKEN_IMPERSONATION_LEVEL_CLASS) * 10 + tok_u32(h, TOKEN_TYPE_CLASS);
    // SAFETY: closing the handle.
    unsafe { CloseHandle(h) };
    l
}

fn test(t: &mut Test) {
    say!(t, "before: thread token (-error or level*10+type) = {}", thread_level());
    for level in [0u32, 1, 2, 3, 4, 5, 0xFFFF_FFFF] {
        // SAFETY: impersonates the process token on this thread.
        let st = unsafe { RtlImpersonateSelf(level) };
        say!(t, "level {level}: status={:#010x} thread token now = {}", st as u32, thread_level());
        // SAFETY: ends the impersonation.
        unsafe { RevertToSelf() };
    }
    say!(t, "after the last revert: {}", thread_level());
}
