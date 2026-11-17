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
// SetProcessWorkingSetSize(Ex): the trim call (-1, -1) the installers make, set and read back, flags.

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetProcessWorkingSetSize(p: HANDLE, min: usize, max: usize) -> BOOL;
    fn GetProcessWorkingSetSize(p: HANDLE, min: *mut usize, max: *mut usize) -> BOOL;
    fn GetProcessWorkingSetSizeEx(p: HANDLE, min: *mut usize, max: *mut usize, flags: *mut u32) -> BOOL;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn GetCurrentProcessId() -> u32;
}

poly_wtest::main!(test);

const PROCESS_SET_QUOTA: u32 = 0x100;
const PROCESS_QUERY_INFORMATION: u32 = 0x400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const NEG1: usize = usize::MAX;

fn get(t: &mut Test, label: &str) {
    let (mut a, mut b, mut f) = (0usize, 0usize, 0xFFu32);
    // SAFETY: valid pointers.
    let ok = unsafe { GetProcessWorkingSetSizeEx(GetCurrentProcess(), &mut a, &mut b, &mut f) };
    say!(t, "{label}: read ok={ok} min={a:#x} max={b:#x} flags={f:#x}");
}

fn test(t: &mut Test) {
    get(t, "start");
    // the trim call
    clear_error();
    // SAFETY: pseudo handle.
    let ok = unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), NEG1, NEG1) };
    say!(t, "SetProcessWorkingSetSize(self,-1,-1) = {ok} err {}", gle_if(ok));
    get(t, "after trim");
    // an ordinary change
    // SAFETY: pseudo handle.
    let ok = unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), 0x100000, 0x400000) };
    say!(t, "set 1MiB..4MiB = {ok} err {}", gle_if(ok));
    get(t, "after 1M..4M");
    // SAFETY: pseudo handle.
    let ok = unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), 0x200000, 0x200000) };
    say!(t, "set 2MiB..2MiB = {ok} err {}", gle_if(ok));
    get(t, "after 2M..2M");
    // a real handle of our own process
    // SAFETY: plain call.
    let h = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION, 0, GetCurrentProcessId()) };
    say!(t, "OpenProcess(SET_QUOTA|QUERY_INFORMATION, self) non-null: {}", !h.is_null());
    clear_error();
    // SAFETY: valid handle.
    let ok = unsafe { SetProcessWorkingSetSize(h, 0x180000, 0x300000) };
    say!(t, "set through the handle = {ok} err {}", gle_if(ok));
    let (mut a, mut b) = (0usize, 0usize);
    // SAFETY: valid pointers.
    let ok = unsafe { GetProcessWorkingSetSize(h, &mut a, &mut b) };
    say!(t, "GetProcessWorkingSetSize through the handle ok={ok} min={a:#x} max={b:#x}");
    // SAFETY: valid handle.
    unsafe { CloseHandle(h) };
}
