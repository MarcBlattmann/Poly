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
// WinHttpSetStatusCallback: previous callback returned, flags, handle kinds, invalid handles (no network traffic).

#[link(name = "winhttp", kind = "raw-dylib")]
unsafe extern "system" {
    fn WinHttpOpen(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> HANDLE;
    fn WinHttpConnect(session: HANDLE, server: *const u16, port: u16, reserved: u32) -> HANDLE;
    fn WinHttpOpenRequest(connect: HANDLE, verb: *const u16, object: *const u16, version: *const u16, referrer: *const u16, accept: *mut *const u16, flags: u32) -> HANDLE;
    fn WinHttpCloseHandle(h: HANDLE) -> BOOL;
    fn WinHttpSetStatusCallback(h: HANDLE, cb: usize, flags: u32, reserved: usize) -> usize;
    fn WinHttpQueryOption(h: HANDLE, option: u32, buf: *mut c_void, len: *mut u32) -> BOOL;
}

poly_wtest::main!(test);

unsafe extern "system" fn cb_a(_h: HANDLE, _ctx: usize, _status: u32, _info: *mut c_void, _len: u32) {}
unsafe extern "system" fn cb_b(_h: HANDLE, _ctx: usize, _status: u32, _info: *mut c_void, _len: u32) {}

const INVALID_CB: usize = usize::MAX;

fn name(prev: usize, a: usize, b: usize) -> &'static str {
    if prev == 0 {
        "NULL"
    } else if prev == INVALID_CB {
        "WINHTTP_INVALID_STATUS_CALLBACK"
    } else if prev == a {
        "callback A"
    } else if prev == b {
        "callback B"
    } else {
        "another pointer"
    }
}

fn test(t: &mut Test) {
    let a = cb_a as usize;
    let b = cb_b as usize;
    let agent = wide("polyspecb4");
    // SAFETY: valid strings.
    let s = unsafe { WinHttpOpen(agent.as_ptr(), 0, core::ptr::null(), core::ptr::null(), 0) };
    // connection and request handles inherit and keep their own callback
    let host = wide("localhost");
    // SAFETY: valid strings and handles.
    unsafe {
        WinHttpSetStatusCallback(s, a, 0xFFFF_FFFF, 0);
        let c = WinHttpConnect(s, host.as_ptr(), 80, 0);
        clear_error();
        let p = WinHttpSetStatusCallback(c, b, 0xFFFF_FFFF, 0);
        say!(t, "connection (session has A): set B: previous = {} (error {})", name(p, a, b), if p == INVALID_CB { gle() } else { -2 });
        let verb = wide("GET");
        let obj = wide("/x");
        let r = WinHttpOpenRequest(c, verb.as_ptr(), obj.as_ptr(), core::ptr::null(), core::ptr::null(), core::ptr::null_mut(), 0);
        clear_error();
        let p = WinHttpSetStatusCallback(r, a, 0xFFFF_FFFF, 0);
        say!(t, "request (session A, connection B): set A: previous = {} (error {})", name(p, a, b), if p == INVALID_CB { gle() } else { -2 });
        let p = WinHttpSetStatusCallback(s, b, 0xFFFF_FFFF, 0);
        say!(t, "session again: set B: previous = {}", name(p, a, b));
        WinHttpCloseHandle(r);
        WinHttpCloseHandle(c);
        WinHttpCloseHandle(s);
    }
}
