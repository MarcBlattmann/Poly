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
// SetProcessWorkingSetSize(Ex) on another process (a child), with the rights a caller may hold, and the token's working-set privilege.

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetProcessWorkingSetSize(p: HANDLE, min: usize, max: usize) -> BOOL;
    fn GetProcessWorkingSetSize(p: HANDLE, min: *mut usize, max: *mut usize) -> BOOL;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn TerminateProcess(p: HANDLE, code: u32) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: *const c_void, ta: *const c_void, inherit: BOOL, flags: u32, env: *const c_void, dir: *const u16, si: *const c_void, pi: *mut PROCESS_INFORMATION) -> BOOL;
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

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: u32, info: *mut c_void, len: u32, ret_len: *mut u32) -> BOOL;
}

poly_wtest::main!(test);

const PROCESS_SET_QUOTA: u32 = 0x100;
const PROCESS_QUERY_INFORMATION: u32 = 0x400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_SUSPENDED: u32 = 4;

fn test(t: &mut Test) {
    // is SeIncreaseWorkingSetPrivilege (LUID 33) in the token, and in what state
    let mut tok: HANDLE = core::ptr::null_mut();
    // SAFETY: valid out pointer.
    unsafe { OpenProcessToken(GetCurrentProcess(), 8, &mut tok) };
    let mut buf = [0u8; 4096];
    let mut r = 0u32;
    // SAFETY: valid buffer.
    unsafe { GetTokenInformation(tok, 3, buf.as_mut_ptr().cast(), 4096, &mut r) };
    let n = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    let mut found = false;
    for i in 0..n {
        let e = 4 + 12 * i;
        if buf[e] == 33 && buf[e + 1] == 0 {
            found = true;
        }
    }
    say!(t, "SeIncreaseWorkingSetPrivilege is in the token (enabled or not): {found}");
    // SAFETY: valid handle.
    unsafe { CloseHandle(tok) };

    // a suspended child
    let mut cmd = wide("C:\\Windows\\System32\\cmd.exe /c exit");
    let mut si = STARTUPINFOW { cb: 104, pad: [0; 100] };
    let mut pi = PROCESS_INFORMATION { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 };
    // SAFETY: valid buffers.
    let ok = unsafe { CreateProcessW(core::ptr::null(), cmd.as_mut_ptr(), core::ptr::null(), core::ptr::null(), 0, CREATE_SUSPENDED | CREATE_NO_WINDOW, core::ptr::null(), core::ptr::null(), (&mut si as *mut STARTUPINFOW).cast(), &mut pi) };
    say!(t, "CreateProcessW suspended child ok={ok}");
    if ok == 0 {
        return;
    }
    let (mut a, mut b) = (0usize, 0usize);
    // SAFETY: valid handle and pointers.
    let ok = unsafe { GetProcessWorkingSetSize(pi.process, &mut a, &mut b) };
    say!(t, "child default working set ok={ok} min={a:#x} max={b:#x}");
    clear_error();
    // SAFETY: valid handle.
    let ok = unsafe { SetProcessWorkingSetSize(pi.process, 0x100000, 0x400000) };
    say!(t, "set child 1MiB..4MiB = {ok} err {}", gle_if(ok));
    // SAFETY: valid handle and pointers.
    unsafe { GetProcessWorkingSetSize(pi.process, &mut a, &mut b) };
    say!(t, "child after min={a:#x} max={b:#x}");
    clear_error();
    // SAFETY: valid handle.
    let ok = unsafe { SetProcessWorkingSetSize(pi.process, usize::MAX, usize::MAX) };
    say!(t, "trim child (-1,-1) = {ok} err {}", gle_if(ok));
    // SAFETY: valid handle and pointers.
    unsafe { GetProcessWorkingSetSize(pi.process, &mut a, &mut b) };
    say!(t, "child after trim min={a:#x} max={b:#x}");
    // a handle opened by pid
    for (name, rights) in [("SET_QUOTA|QUERY_INFORMATION", PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION), ("QUERY_LIMITED_INFORMATION", PROCESS_QUERY_LIMITED_INFORMATION), ("SET_QUOTA", PROCESS_SET_QUOTA)] {
        // SAFETY: plain call.
        let h = unsafe { OpenProcess(rights, 0, pi.pid) };
        clear_error();
        // SAFETY: valid handle (or NULL).
        let ok = unsafe { SetProcessWorkingSetSize(h, 0x100000, 0x400000) };
        say!(t, "OpenProcess({name}) set = {ok} err {}", gle_if(ok));
        // SAFETY: valid handle.
        unsafe { CloseHandle(h) };
    }
    // SAFETY: valid handles.
    unsafe {
        TerminateProcess(pi.process, 0);
        WaitForSingleObject(pi.process, 5000);
        CloseHandle(pi.thread);
        CloseHandle(pi.process);
    }
}
