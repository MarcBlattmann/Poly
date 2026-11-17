#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, unused_imports, clippy::all)]

// The declarations in this file are from Microsoft Learn (prototypes) and the
// public SDK headers named beside them (constants, layouts); the helpers are
// this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
type HANDLE = *mut c_void;
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

/// One observation line, formatted.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<700>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// A NUL-terminated UTF-16 copy of a string (up to 299 units; no characters outside the BMP).
fn wide(s: &str) -> [u16; 300] {
    let mut w = [0u16; 300];
    for (i, c) in s.chars().enumerate().take(299) {
        w[i] = c as u32 as u16;
    }
    w
}

/// UTF-16 units as text: printable ASCII as it is, anything else as `<xxxx>`.
fn units_text(u: &[u16]) -> Buf<700> {
    let mut b = Buf::<700>::new();
    for &c in u {
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
    }
    b
}

/// The units of a NUL-terminated UTF-16 string at `p` (at most `max`).
fn wstr_at<'a>(p: *const u16, max: usize) -> &'a [u16] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string the system returned; the scan stops at `max`.
    unsafe {
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

/// Whether two UTF-16 strings are equal without regard to ASCII case.
fn eq_nocase(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(&x, &y)| {
            let l = |c: u16| if (0x41..=0x5a).contains(&c) { c + 32 } else { c };
            l(x) == l(y)
        })
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
}

fn gle() -> i64 {
    // SAFETY: plain call.
    i64::from(unsafe { GetLastError() })
}

fn clear_error() {
    // SAFETY: plain call.
    unsafe { SetLastError(0xDEAD) };
}

/// The last error after a failed call, else -2 (success leaves it unspecified).
fn gle_if(ok: BOOL) -> i64 {
    if ok == 0 { gle() } else { -2 }
}
// K32QueryWorkingSetEx: a handle opened with only PROCESS_QUERY_LIMITED_INFORMATION (own process and a child), by token kind.

#[repr(C)]
#[derive(Clone, Copy)]
struct Wse {
    addr: usize,
    attr: usize,
}

#[repr(C)]
struct PROCESS_INFORMATION {
    process: HANDLE,
    thread: HANDLE,
    pid: u32,
    tid: u32,
}

#[repr(C)]
struct STARTUPINFOW {
    cb: u32,
    pad: [u8; 100],
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn K32QueryWorkingSetEx(p: HANDLE, info: *mut Wse, cb: u32) -> BOOL;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn GetCurrentProcessId() -> u32;
    fn TerminateProcess(p: HANDLE, code: u32) -> BOOL;
    fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: *const c_void, ta: *const c_void, inherit: BOOL, flags: u32, env: *const c_void, dir: *const u16, si: *const c_void, pi: *mut PROCESS_INFORMATION) -> BOOL;
}

poly_wtest::main!(test);

const PROCESS_QUERY_INFORMATION: u32 = 0x400;
const PROCESS_VM_READ: u32 = 0x10;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

fn call(t: &mut Test, label: &str, h: HANDLE) {
    let mut e = [Wse { addr: 0x10000, attr: 0xAAAA }];
    clear_error();
    // SAFETY: one valid entry.
    let ok = unsafe { K32QueryWorkingSetEx(h, e.as_mut_ptr(), 16) };
    say!(t, "{label}: ok={ok} err {}", gle_if(ok));
}

fn test(t: &mut Test) {
    for (name, rights) in [
        ("self, QUERY_LIMITED_INFORMATION", PROCESS_QUERY_LIMITED_INFORMATION),
        ("self, QUERY_INFORMATION", PROCESS_QUERY_INFORMATION),
        ("self, QUERY_LIMITED_INFORMATION|VM_READ", PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ),
        ("self, QUERY_INFORMATION|VM_READ", PROCESS_QUERY_INFORMATION | PROCESS_VM_READ),
    ] {
        // SAFETY: plain call.
        let h = unsafe { OpenProcess(rights, 0, GetCurrentProcessId()) };
        call(t, name, h);
        // SAFETY: valid handle.
        unsafe { CloseHandle(h) };
    }
    let mut cmd = wide("C:\\Windows\\System32\\cmd.exe /c exit");
    let mut si = STARTUPINFOW { cb: 104, pad: [0; 100] };
    let mut pi = PROCESS_INFORMATION { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 };
    // SAFETY: valid buffers (CREATE_SUSPENDED | CREATE_NO_WINDOW).
    let ok = unsafe { CreateProcessW(core::ptr::null(), cmd.as_mut_ptr(), core::ptr::null(), core::ptr::null(), 0, 4 | 0x0800_0000, core::ptr::null(), core::ptr::null(), (&mut si as *mut STARTUPINFOW).cast(), &mut pi) };
    if ok != 0 {
        for (name, rights) in [("child, QUERY_LIMITED_INFORMATION", PROCESS_QUERY_LIMITED_INFORMATION), ("child, QUERY_INFORMATION", PROCESS_QUERY_INFORMATION)] {
            // SAFETY: plain call.
            let h = unsafe { OpenProcess(rights, 0, pi.pid) };
            call(t, name, h);
            // SAFETY: valid handle.
            unsafe { CloseHandle(h) };
        }
        // SAFETY: valid handles.
        unsafe {
            TerminateProcess(pi.process, 0);
            CloseHandle(pi.thread);
            CloseHandle(pi.process);
        }
    }
}

// recorded with the elevated (full) administrator token
