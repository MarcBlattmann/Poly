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
// WTSEnumerateSessionsW and WTSFreeMemory: the array, session 0, the caller's own session, the state values.

#[repr(C)]
struct WtsSessionInfoW {
    session_id: u32,
    name: *const u16,
    state: u32,
}

#[link(name = "wtsapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn WTSEnumerateSessionsW(server: HANDLE, reserved: u32, version: u32, info: *mut *mut WtsSessionInfoW, count: *mut u32) -> BOOL;
    fn WTSFreeMemory(p: *mut c_void);
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn WTSGetActiveConsoleSessionId() -> u32;
    fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> BOOL;
    fn GetCurrentProcessId() -> u32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut own = 0xAAAAu32;
    // SAFETY: valid pointer.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut own) };
    // SAFETY: plain call.
    let console = unsafe { WTSGetActiveConsoleSessionId() };
    let mut info: *mut WtsSessionInfoW = core::ptr::null_mut();
    let mut count = 0xAAAAu32;
    clear_error();
    // SAFETY: valid out pointers; WTS_CURRENT_SERVER_HANDLE is NULL.
    let ok = unsafe { WTSEnumerateSessionsW(core::ptr::null_mut(), 0, 1, &mut info, &mut count) };
    say!(t, "WTSEnumerateSessionsW ok={ok} err {} at least 1 session: {}", gle_if(ok), count >= 1);
    if ok != 0 && !info.is_null() {
        let mut ids_ascending = true;
        let mut prev: i64 = -1;
        let mut found0 = false;
        let mut found_own = false;
        let mut found_console = false;
        for i in 0..count as usize {
            // SAFETY: the array holds `count` entries.
            let e = unsafe { &*info.add(i) };
            if i64::from(e.session_id) <= prev {
                ids_ascending = false;
            }
            prev = i64::from(e.session_id);
            let name = wstr_at(e.name, 64);
            if e.session_id == 0 {
                found0 = true;
                say!(t, "session 0: name {} state {}", units_text(name).as_str(), e.state);
            }
            if e.session_id == own {
                found_own = true;
                say!(t, "own session is session 0: {}; its state is WTSActive (0): {}", own == 0, e.state == 0);
            }
            if e.session_id == console {
                found_console = true;
                say!(t, "console session: name {} state {}", units_text(name).as_str(), e.state);
            }
            // names are text; states are 0..=9
            if e.state > 9 {
                say!(t, "state out of range: {}", e.state);
            }
        }
        say!(t, "ids in ascending order: {ids_ascending}; session 0 listed: {found0}; own session listed: {found_own}; console session listed: {found_console}");
        // SAFETY: memory the call returned.
        unsafe { WTSFreeMemory(info.cast()) };
    }
    // SAFETY: NULL is accepted
    unsafe { WTSFreeMemory(core::ptr::null_mut()) };
    say!(t, "WTSFreeMemory(NULL) returned");
    // a second call gives the same shape
    let mut info2: *mut WtsSessionInfoW = core::ptr::null_mut();
    let mut count2 = 0u32;
    // SAFETY: valid out pointers.
    let ok2 = unsafe { WTSEnumerateSessionsW(core::ptr::null_mut(), 0, 1, &mut info2, &mut count2) };
    say!(t, "again: ok={ok2} same count: {}", count2 == count);
    // SAFETY: memory the call returned.
    unsafe { WTSFreeMemory(info2.cast()) };
}
