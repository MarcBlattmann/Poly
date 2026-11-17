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
// WTSEnumerateSessionsW: shape of the array (entry size, names, states, ids), memory freed with WTSFreeMemory, repeated calls.

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

poly_wtest::main!(test);

fn test(t: &mut Test) {
    say!(t, "size of one entry on x64 is 24 bytes: {}", core::mem::size_of::<WtsSessionInfoW>() == 24);
    for round in 0..3 {
        let mut info: *mut WtsSessionInfoW = core::ptr::null_mut();
        let mut count = 0u32;
        // SAFETY: valid out pointers.
        let ok = unsafe { WTSEnumerateSessionsW(core::ptr::null_mut(), 0, 1, &mut info, &mut count) };
        say!(t, "round {round}: ok={ok} count>0 {}", count > 0);
        if ok == 0 {
            continue;
        }
        let mut unique = true;
        let mut names_ok = true;
        let mut states_ok = true;
        let mut name_in_block = true;
        for i in 0..count as usize {
            // SAFETY: the array holds `count` entries.
            let e = unsafe { &*info.add(i) };
            for j in 0..i {
                // SAFETY: as above.
                if unsafe { (*info.add(j)).session_id } == e.session_id {
                    unique = false;
                }
            }
            if e.name.is_null() {
                names_ok = false;
            } else if wstr_at(e.name, 64).is_empty() {
                names_ok = false;
            }
            if e.state > 9 {
                states_ok = false;
            }
            // the names live inside the same allocation, after the array
            let start = info as usize;
            let end = start + (count as usize) * 24 + 4096;
            if (e.name as usize) < start || (e.name as usize) > end {
                name_in_block = false;
            }
        }
        say!(t, "round {round}: ids unique {unique}, names non-empty {names_ok}, states 0..9 {states_ok}, names near the array {name_in_block}");
        // the id of the first entry is 0 (the services session) and the entries are sorted
        // SAFETY: at least one entry.
        let first = unsafe { (*info).session_id };
        say!(t, "round {round}: first entry is session 0: {}", first == 0);
        // SAFETY: memory the call returned.
        unsafe { WTSFreeMemory(info.cast()) };
    }
}
