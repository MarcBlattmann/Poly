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
// SetProcessWorkingSetSizeEx (flags 0 unless named): error cases (bad sizes, bad handles, handles without PROCESS_SET_QUOTA, threads).

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetProcessWorkingSetSizeEx(p: HANDLE, min: usize, max: usize, flags: u32) -> BOOL;
    fn GetProcessWorkingSetSize(p: HANDLE, min: *mut usize, max: *mut usize) -> BOOL;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThread() -> HANDLE;
    fn GetCurrentThreadId() -> u32;
    fn OpenThread(access: u32, inherit: BOOL, tid: u32) -> HANDLE;
}

poly_wtest::main!(test);

/// The flags-less form of the same call: SetProcessWorkingSetSizeEx with flags 0.
#[allow(non_snake_case)]
unsafe fn SetProcessWorkingSetSize(p: HANDLE, min: usize, max: usize) -> BOOL {
    // SAFETY: forwards its arguments.
    unsafe { SetProcessWorkingSetSizeEx(p, min, max, 0) }
}

const PROCESS_SET_QUOTA: u32 = 0x100;
const PROCESS_QUERY_INFORMATION: u32 = 0x400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const SYNCHRONIZE_RIGHT: u32 = 0x100000;

fn call(t: &mut Test, label: &str, h: HANDLE, min: usize, max: usize) {
    clear_error();
    // SAFETY: handle may be invalid on purpose; the call validates it.
    let ok = unsafe { SetProcessWorkingSetSize(h, min, max) };
    say!(t, "{label} = {ok} err {}", gle_if(ok));
}

fn test(t: &mut Test) {
    let me = GetCurrentProcessPseudo();
    call(t, "min > max (2MiB, 1MiB)", me, 0x200000, 0x100000);
    call(t, "min = max = 0", me, 0, 0);
    call(t, "min 0, max 4MiB", me, 0, 0x400000);
    call(t, "min 1, max 1", me, 1, 1);
    call(t, "min 4096, max 8192", me, 0x1000, 0x2000);
    call(t, "min 1MiB, max -1", me, 0x100000, usize::MAX);
    call(t, "min -1, max 4MiB", me, usize::MAX, 0x400000);
    call(t, "min max-1, max -2 (huge)", me, usize::MAX - 1, usize::MAX - 1);
    call(t, "min 0x7FFFFFFFFFFF, max the same", me, 0x7FFF_FFFF_FFFF, 0x7FFF_FFFF_FFFF);
    call(t, "NULL handle", core::ptr::null_mut(), 0x100000, 0x400000);
    call(t, "INVALID_HANDLE_VALUE (-1 = current process)", usize::MAX as HANDLE, 0x100000, 0x400000);
    call(t, "garbage handle 0x12345678", 0x12345678usize as HANDLE, 0x100000, 0x400000);
    call(t, "current thread pseudo handle (-2)", usize::MAX.wrapping_sub(1) as HANDLE, 0x100000, 0x400000);
    // SAFETY: plain call.
    let th = unsafe { OpenThread(0x1FFFFF, 0, GetCurrentThreadId()) };
    call(t, "a thread handle", th, 0x100000, 0x400000);
    // SAFETY: valid handle.
    unsafe { CloseHandle(th) };
    // handles without PROCESS_SET_QUOTA
    for (name, rights) in [("QUERY_INFORMATION only", PROCESS_QUERY_INFORMATION), ("QUERY_LIMITED_INFORMATION only", PROCESS_QUERY_LIMITED_INFORMATION), ("SYNCHRONIZE only", SYNCHRONIZE_RIGHT), ("SET_QUOTA only", PROCESS_SET_QUOTA)] {
        // SAFETY: plain call.
        let h = unsafe { OpenProcess(rights, 0, GetCurrentProcessId()) };
        call(t, name, h, 0x100000, 0x400000);
        let (mut a, mut b) = (0usize, 0usize);
        clear_error();
        // SAFETY: valid pointers.
        let ok = unsafe { GetProcessWorkingSetSize(h, &mut a, &mut b) };
        say!(t, "  GetProcessWorkingSetSize with that handle = {ok} err {}", gle_if(ok));
        // SAFETY: valid handle.
        unsafe { CloseHandle(h) };
    }
    // Ex: bad flags are judged before or after the handle?
    clear_error();
    // SAFETY: pseudo handle.
    let ok = unsafe { SetProcessWorkingSetSizeEx(core::ptr::null_mut(), 0x100000, 0x400000, 0xFFFF_FFFF) };
    say!(t, "Ex NULL handle + bad flags = {ok} err {}", gle_if(ok));
    clear_error();
    // SAFETY: pseudo handle.
    let ok = unsafe { SetProcessWorkingSetSizeEx(me, 0x200000, 0x100000, 0xFFFF_FFFF) };
    say!(t, "Ex min>max + bad flags = {ok} err {}", gle_if(ok));
    clear_error();
    // SAFETY: pseudo handle.
    let ok = unsafe { SetProcessWorkingSetSizeEx(me, 0x100000, 0x400000, 3) };
    say!(t, "Ex MIN_ENABLE|MIN_DISABLE = {ok} err {}", gle_if(ok));
    // GetProcessWorkingSetSize with bad pointers is not tested (it raises)
    let _ = GetCurrentThread as usize;
}

#[allow(non_snake_case)]
fn GetCurrentProcessPseudo() -> HANDLE {
    // SAFETY: plain call.
    unsafe { GetCurrentProcess() }
}
