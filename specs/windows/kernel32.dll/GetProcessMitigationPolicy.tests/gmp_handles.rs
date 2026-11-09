//! GetProcessMitigationPolicy: which process handles it accepts (pseudo-handle, real handle with
//! PROCESS_QUERY_INFORMATION / PROCESS_QUERY_LIMITED_INFORMATION / none, null, invalid) and a
//! null buffer.
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

const PROCESS_QUERY_INFORMATION: u32 = 0x400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const PROCESS_TERMINATE: u32 = 0x1;
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn GetCurrentProcessId() -> u32;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn GetProcessMitigationPolicy(process: HANDLE, policy: u32, buffer: PVOID, len: usize) -> BOOL;
}

poly_wtest::main!(test);

fn try_handle(t: &mut Test, label: &str, h: HANDLE) {
    let mut raw = Raw::<64>::new();
    clear_error();
    // SAFETY: a 64-byte buffer; the handle is whatever the test passes.
    let ok = unsafe { GetProcessMitigationPolicy(h, 3, raw.p(), 4) };
    say!(t, "{label}: ok={ok} err={}", if ok == 0 { i64::from(gle()) } else { -1 });
}

fn test(t: &mut Test) {
    // SAFETY: the pseudo-handle of the current process.
    let me = unsafe { GetCurrentProcess() };
    try_handle(t, "pseudo-handle", me);
    for (label, access) in [
        ("PROCESS_QUERY_INFORMATION", 0x400u32),
        ("PROCESS_QUERY_LIMITED_INFORMATION", 0x1000),
        ("SYNCHRONIZE only", 0x0010_0000),
        ("PROCESS_TERMINATE only", 1),
    ] {
        // SAFETY: OpenProcess on our own id.
        let h = unsafe { OpenProcess(access, 0, GetCurrentProcessId()) };
        if h.is_null() {
            say!(t, "{label}: OpenProcess failed, error {}", gle());
            continue;
        }
        try_handle(t, label, h);
        // SAFETY: closing the handle.
        unsafe { CloseHandle(h) };
    }
    try_handle(t, "null handle", core::ptr::null_mut());
    try_handle(t, "handle -2 (thread pseudo-handle)", -2isize as HANDLE);
    try_handle(t, "invalid handle value", 0x7FFF_FFF4usize as HANDLE);
    clear_error();
    // SAFETY: a null buffer with size 0 is rejected by the call.
    let ok = unsafe { GetProcessMitigationPolicy(me, 3, core::ptr::null_mut(), 0) };
    say!(t, "null buffer, size 0: ok={ok} err={}", if ok == 0 { i64::from(gle()) } else { -1 });
}
