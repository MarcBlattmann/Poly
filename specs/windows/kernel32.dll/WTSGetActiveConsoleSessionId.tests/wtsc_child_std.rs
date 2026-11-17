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
// WTSGetActiveConsoleSessionId: the same answer from every thread and process, a child of this process runs in this process's session.

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
    fn WTSGetActiveConsoleSessionId() -> u32;
    fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> BOOL;
    fn GetCurrentProcessId() -> u32;
    fn TerminateProcess(p: HANDLE, code: u32) -> BOOL;
    fn CreateThread(sa: *const c_void, stack: usize, start: extern "system" fn(*mut c_void) -> u32, arg: *mut c_void, flags: u32, tid: *mut u32) -> HANDLE;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: *const c_void, ta: *const c_void, inherit: BOOL, flags: u32, env: *const c_void, dir: *const u16, si: *const c_void, pi: *mut PROCESS_INFORMATION) -> BOOL;
}

poly_wtest::main!(test);

static FROM_THREAD: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0xAAAA);

extern "system" fn thread_main(_: *mut c_void) -> u32 {
    // SAFETY: plain call.
    let id = unsafe { WTSGetActiveConsoleSessionId() };
    FROM_THREAD.store(id, core::sync::atomic::Ordering::SeqCst);
    0
}

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    let console = unsafe { WTSGetActiveConsoleSessionId() };
    let mut own = 0xAAAAu32;
    // SAFETY: valid pointer.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut own) };
    let mut tid = 0u32;
    // SAFETY: valid thread start.
    let th = unsafe { CreateThread(core::ptr::null(), 0, thread_main, core::ptr::null_mut(), 0, &mut tid) };
    // SAFETY: valid handle.
    unsafe {
        WaitForSingleObject(th, 5000);
        CloseHandle(th);
    }
    say!(t, "another thread gets the same id: {}", FROM_THREAD.load(core::sync::atomic::Ordering::SeqCst) == console);
    let mut cmd = wide("C:\\Windows\\System32\\cmd.exe /c exit");
    let mut si = STARTUPINFOW { cb: 104, pad: [0; 100] };
    let mut pi = PROCESS_INFORMATION { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 };
    // SAFETY: valid buffers (CREATE_SUSPENDED | CREATE_NO_WINDOW).
    let ok = unsafe { CreateProcessW(core::ptr::null(), cmd.as_mut_ptr(), core::ptr::null(), core::ptr::null(), 0, 4 | 0x0800_0000, core::ptr::null(), core::ptr::null(), (&mut si as *mut STARTUPINFOW).cast(), &mut pi) };
    say!(t, "child created: {ok}");
    if ok != 0 {
        let mut cs = 0xBBBBu32;
        // SAFETY: valid pointer.
        let r = unsafe { ProcessIdToSessionId(pi.pid, &mut cs) };
        say!(t, "ProcessIdToSessionId(child) ok={r}; the child is in this process's session: {}", cs == own);
        // SAFETY: valid handles.
        unsafe {
            TerminateProcess(pi.process, 0);
            CloseHandle(pi.thread);
            CloseHandle(pi.process);
        }
    }
    say!(t, "the console session is a small number: {}", console < 64);
}
