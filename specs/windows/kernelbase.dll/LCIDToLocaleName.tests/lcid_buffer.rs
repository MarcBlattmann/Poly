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

// ---- group L helpers ----

#[repr(C)]
#[derive(Clone, Copy)]
struct ST {
    year: u16,
    month: u16,
    dow: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    ms: u16,
}

const fn st(year: u16, month: u16, dow: u16, day: u16, hour: u16, minute: u16, second: u16, ms: u16) -> ST {
    ST { year, month, dow, day, hour, minute, second, ms }
}

// The fixed instants used by the tests (SYSTEMTIME of Learn: year, month, day of week, day, hour, minute, second, ms).
const S_MORNING: ST = st(2024, 3, 2, 5, 7, 8, 9, 10); // Tuesday 2024-03-05 07:08:09.010
const S_NIGHT: ST = st(2023, 12, 0, 31, 23, 59, 58, 999); // Sunday 2023-12-31 23:59:58.999
const S_LEAP: ST = st(2024, 2, 4, 29, 12, 0, 0, 0); // Thursday 2024-02-29 12:00:00 (noon)
const S_MIDNIGHT: ST = st(2024, 7, 0, 14, 0, 0, 0, 0); // Sunday 2024-07-14 00:00:00
const S_EPOCH: ST = st(1601, 1, 1, 1, 0, 5, 0, 0); // Monday 1601-01-01 00:05:00 (first valid year)
const S_AFTERNOON: ST = st(2025, 11, 3, 5, 15, 30, 45, 123); // Wednesday 2025-11-05 15:30:45.123

/// The text of a NUL-terminated (or `n`-long) UTF-16 buffer as printable text.
fn txt(b: &[u16]) -> Buf<700> {
    let mut n = 0;
    while n < b.len() && b[n] != 0 {
        n += 1;
    }
    units_text(&b[..n])
}

/// The length of the NUL-terminated string in `b` (units before the first NUL), or `b.len()` when none.
fn ulen(b: &[u16]) -> usize {
    let mut n = 0;
    while n < b.len() && b[n] != 0 {
        n += 1;
    }
    n
}

impl<const N: usize> core::fmt::Display for Buf<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[link(name = "kernelbase", kind = "raw-dylib")]
unsafe extern "system" {
    fn LCIDToLocaleName(lcid: u32, name: *mut u16, cch: i32, flags: u32) -> i32;
    fn LocaleNameToLCID(name: *const u16, flags: u32) -> u32;
    fn GetUserDefaultLocaleName(name: *mut u16, cch: i32) -> i32;
    fn GetSystemDefaultLocaleName(name: *mut u16, cch: i32) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetUserDefaultLCID() -> u32;
    fn GetSystemDefaultLCID() -> u32;
    fn GetUserDefaultUILanguage() -> u16;
}

//@ use=b:LCIDToLocaleName,b:LocaleNameToLCID,b:GetUserDefaultLocaleName,b:GetSystemDefaultLocaleName,k:GetUserDefaultLCID,k:GetSystemDefaultLCID,k:GetUserDefaultUILanguage
poly_wtest::main!(test);

/// Calls LCIDToLocaleName with an `n`-unit buffer filled with 0xAAAA; returns (result, last error or -2, text or "untouched").
fn l2n(lcid: u32, flags: u32, n: i32) -> (i32, i64, Buf<200>) {
    let mut out = [0xAAAAu16; 100];
    clear_error();
    // SAFETY: n <= 100.
    let r = unsafe { LCIDToLocaleName(lcid, out.as_mut_ptr(), n, flags) };
    let e = if r == 0 { gle() } else { -2 };
    let mut b = Buf::<200>::new();
    if out.iter().all(|&c| c == 0xAAAA) {
        let _ = write!(b, "untouched");
    } else {
        let _ = write!(b, "'{}'", txt(&out));
    }
    (r, e, b)
}

fn test(t: &mut Test) {
    // the size query and every buffer size around the 6 units "en-US\0" needs
    for (nm, l) in [("0x409", 0x409u32), ("0x7F inv", 0x7F), ("0x10407", 0x10407)] {
        clear_error();
        // SAFETY: cch 0: NULL buffer allowed.
        let n0 = unsafe { LCIDToLocaleName(l, core::ptr::null_mut(), 0, 0) };
        say!(t, "{nm} size query: n={n0} err={}", gle_if(n0));
        for n in [1i32, 2, 5, 6, 7, 8, 20, 85, 100] {
            let (r, e, s) = l2n(l, 0, n);
            say!(t, "{nm} cch={n}: n={r} err={e} {}", s);
        }
        for n in [-1i32, -100] {
            let (r, e, s) = l2n(l, 0, n);
            say!(t, "{nm} cch={n}: n={r} err={e} {}", s);
        }
    }
    // NULL buffer with a size
    clear_error();
    // SAFETY: deliberately invalid: NULL buffer with a size.
    let r = unsafe { LCIDToLocaleName(0x409, core::ptr::null_mut(), 10, 0) };
    say!(t, "NULL buffer cch=10: n={r} err={}", gle_if(r));
    // a failing lcid with cch 0
    for l in [0x1234u32, 0xFFFF, 0x10409] {
        clear_error();
        // SAFETY: cch 0: NULL buffer allowed.
        let r = unsafe { LCIDToLocaleName(l, core::ptr::null_mut(), 0, 0) };
        say!(t, "lcid {l:#x} size query: n={r} err={}", gle_if(r));
    }
    // flags
    for (nm, f) in [
        ("ALLOW_NEUTRAL_NAMES 0x08000000", 0x0800_0000u32),
        ("0x1", 1),
        ("0x2", 2),
        ("0x4", 4),
        ("0x8", 8),
        ("0x10", 0x10),
        ("0x100", 0x100),
        ("0x400", 0x400),
        ("0x1000", 0x1000),
        ("USE_CP_ACP 0x40000000", 0x4000_0000),
        ("NOUSEROVERRIDE 0x80000000", 0x8000_0000),
        ("0x08000001", 0x0800_0001),
        ("0x0C000000", 0x0C00_0000),
        ("0xFFFFFFFF", 0xFFFF_FFFF),
    ] {
        for l in [0x409u32, 0x09] {
            let (r, e, s) = l2n(l, f, 100);
            say!(t, "lcid {l:#x} flags {nm}: n={r} err={e} {}", s);
        }
    }
}
