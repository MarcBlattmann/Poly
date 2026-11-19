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
// K32QueryWorkingSetEx: sizes, several entries, handle rights, other processes, bad arguments.

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
    fn VirtualAlloc(addr: *mut c_void, size: usize, ty: u32, prot: u32) -> *mut c_void;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThread() -> HANDLE;
    fn TerminateProcess(p: HANDLE, code: u32) -> BOOL;
    fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: *const c_void, ta: *const c_void, inherit: BOOL, flags: u32, env: *const c_void, dir: *const u16, si: *const c_void, pi: *mut PROCESS_INFORMATION) -> BOOL;
}

poly_wtest::main!(test);

const PROCESS_VM_READ: u32 = 0x10;
const PROCESS_QUERY_INFORMATION: u32 = 0x400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

fn call(t: &mut Test, label: &str, h: HANDLE, e: &mut [Wse], cb: u32) {
    clear_error();
    // SAFETY: the buffer is at least `cb` bytes in the cases below (or cb is invalid on purpose).
    let ok = unsafe { K32QueryWorkingSetEx(h, e.as_mut_ptr(), cb) };
    say!(t, "{label}: ok={ok} err {}", gle_if(ok));
}

fn test(t: &mut Test) {
    // SAFETY: allocation.
    let page = unsafe { VirtualAlloc(core::ptr::null_mut(), 0x3000, 0x3000, 4) } as usize;
    // SAFETY: our page.
    unsafe { core::ptr::write_volatile(page as *mut u8, 1) };
    let me = GetCurrentProcessHandle();
    // sizes
    let mut e = [Wse { addr: page, attr: 0xAAAA }; 3];
    call(t, "cb 48 (3 entries)", me, &mut e, 48);
    say!(t, "  attrs valid bits: {} {} {}; addresses unchanged: {}", e[0].attr & 1, e[1].attr & 1, e[2].attr & 1, e[0].addr == page);
    e[1].addr = page + 0x1000;
    e[2].addr = page + 0x2000;
    call(t, "three different pages", me, &mut e, 48);
    say!(t, "  valid bits: {} {} {}", e[0].attr & 1, e[1].attr & 1, e[2].attr & 1);
    let mut e = [Wse { addr: page, attr: 0xAAAA }; 3];
    call(t, "cb 0", me, &mut e, 0);
    say!(t, "  attr untouched: {}", e[0].attr == 0xAAAA);
    call(t, "cb 8 (half an entry)", me, &mut e, 8);
    call(t, "cb 15", me, &mut e, 15);
    call(t, "cb 16", me, &mut e, 16);
    call(t, "cb 17", me, &mut e, 17);
    call(t, "cb 24 (one and a half entries)", me, &mut e, 24);
    call(t, "cb 32", me, &mut e, 32);
    // handles
    // SAFETY: plain call.
    let h = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, GetCurrentProcessId()) };
    call(t, "handle QUERY_INFORMATION|VM_READ", h, &mut e, 16);
    // SAFETY: valid handle.
    unsafe { CloseHandle(h) };
    // SAFETY: plain call.
    let h = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, 0, GetCurrentProcessId()) };
    call(t, "handle QUERY_INFORMATION only", h, &mut e, 16);
    // SAFETY: valid handle.
    unsafe { CloseHandle(h) };
    // SAFETY: plain call.
    let h = unsafe { OpenProcess(PROCESS_VM_READ, 0, GetCurrentProcessId()) };
    call(t, "handle VM_READ only", h, &mut e, 16);
    // SAFETY: valid handle.
    unsafe { CloseHandle(h) };
    call(t, "NULL handle", core::ptr::null_mut(), &mut e, 16);
    call(t, "garbage handle", 0x12345678usize as HANDLE, &mut e, 16);
    // SAFETY: plain call.
    let th = unsafe { GetCurrentThread() };
    call(t, "thread pseudo handle", th, &mut e, 16);
    // a pointer that is not writable raises or fails; not tested.
    // another process (a child with default rights)
    let mut cmd = wide("C:\\Windows\\System32\\cmd.exe /c exit");
    let mut si = STARTUPINFOW { cb: 104, pad: [0; 100] };
    let mut pi = PROCESS_INFORMATION { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 };
    // SAFETY: valid buffers (CREATE_SUSPENDED | CREATE_NO_WINDOW).
    let ok = unsafe { CreateProcessW(core::ptr::null(), cmd.as_mut_ptr(), core::ptr::null(), core::ptr::null(), 0, 4 | 0x0800_0000, core::ptr::null(), core::ptr::null(), (&mut si as *mut STARTUPINFOW).cast(), &mut pi) };
    say!(t, "child created: {ok}");
    if ok != 0 {
        let mut c = [Wse { addr: 0x10000, attr: 0xAAAA }];
        call(t, "child handle (all access), low address", pi.process, &mut c, 16);
        // the child's image base is its own; ask for an address of ours that is not mapped there
        let mut c = [Wse { addr: page, attr: 0xAAAA }];
        call(t, "child handle, an address that is our allocation", pi.process, &mut c, 16);
        say!(t, "  that page is not valid in the child: {}", c[0].attr & 1 == 0);
        // SAFETY: valid handle.
        let h = core::ptr::null_mut();
        // SAFETY: valid handles.
        unsafe {
            CloseHandle(h);
            TerminateProcess(pi.process, 0);
            CloseHandle(pi.thread);
            CloseHandle(pi.process);
        }
    }
}

fn GetCurrentProcessHandle() -> HANDLE {
    // SAFETY: plain call.
    unsafe { GetCurrentProcess() }
}
