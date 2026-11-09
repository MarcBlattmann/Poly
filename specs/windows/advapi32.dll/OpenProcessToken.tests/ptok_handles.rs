//! OpenProcessToken with different process handles: the current-process pseudo-handle, a real
//! handle of the same process, handles with too little access, NULL, the thread pseudo-handle.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winnt.h, winbase.h, sddl.h, aclapi.h, jobapi2.h,
// processthreadsapi.h, winternl.h, ntstatus.h and winerror.h (constants,
// layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
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

const TOKEN_QUERY: u32 = 0x0008;
const STANDARD_RIGHTS_REQUIRED: u32 = 0x000F_0000;
const PROCESS_QUERY_INFORMATION: u32 = 0x400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const PROCESS_TERMINATE: u32 = 0x1;
const PROCESS_ALL_ACCESS: u32 = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0xFFFF;
/// The last-error code after a failed call, as a plain number.
fn gle() -> u32 {
    // SAFETY: reads the calling thread's last error.
    unsafe { GetLastError() }
}

fn clear_error() {
    // SAFETY: sets the calling thread's last error.
    unsafe { SetLastError(0xDEAD) };
}

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcessId() -> u32;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut h: HANDLE = core::ptr::null_mut();
    clear_error();
    // SAFETY: valid out pointer.
    let ok = unsafe { OpenProcessToken(core::ptr::null_mut(), TOKEN_QUERY, &mut h) };
    say!(t, "NULL process: ok={ok} err={}", gle_if(ok));
    clear_error();
    // SAFETY: valid out pointer; -1 is the current-process pseudo-handle.
    let ok = unsafe { OpenProcessToken(-1isize as HANDLE, TOKEN_QUERY, &mut h) };
    say!(t, "pseudo process (-1): ok={ok} err={}", gle_if(ok));
    if ok != 0 {
        // SAFETY: closing the handle just opened.
        unsafe { CloseHandle(h) };
    }
    clear_error();
    // SAFETY: valid out pointer; -2 is the current-thread pseudo-handle, not a process.
    let ok = unsafe { OpenProcessToken(-2isize as HANDLE, TOKEN_QUERY, &mut h) };
    say!(t, "thread pseudo-handle as process: ok={ok} err={}", gle_if(ok));
    if ok != 0 {
        // SAFETY: closing the handle just opened.
        unsafe { CloseHandle(h) };
    }
    let pid = unsafe { GetCurrentProcessId() };
    for (name, access) in [
        ("PROCESS_QUERY_LIMITED_INFORMATION", PROCESS_QUERY_LIMITED_INFORMATION),
        ("PROCESS_QUERY_INFORMATION", PROCESS_QUERY_INFORMATION),
        ("PROCESS_TERMINATE", PROCESS_TERMINATE),
        ("SYNCHRONIZE", SYNCHRONIZE),
        ("PROCESS_ALL_ACCESS", PROCESS_ALL_ACCESS),
    ] {
        // SAFETY: opens our own process.
        let p = unsafe { OpenProcess(access, FALSE, pid) };
        if p.is_null() {
            say!(t, "{name}: OpenProcess failed");
            continue;
        }
        clear_error();
        // SAFETY: valid process handle and out pointer.
        let ok = unsafe { OpenProcessToken(p, TOKEN_QUERY, &mut h) };
        say!(t, "{name}: ok={ok} err={}", gle_if(ok));
        if ok != 0 {
            // SAFETY: closing the handle just opened.
            unsafe { CloseHandle(h) };
        }
        // SAFETY: closing our process handle.
        unsafe { CloseHandle(p) };
    }
    clear_error();
    // SAFETY: a handle value that is not open.
    let ok = unsafe { OpenProcessToken(0x7FFC_usize as HANDLE, TOKEN_QUERY, &mut h) };
    say!(t, "bad handle: ok={ok} err={}", gle_if(ok));
}
