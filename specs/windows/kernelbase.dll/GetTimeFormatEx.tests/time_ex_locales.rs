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
    fn GetTimeFormatEx(name: *const u16, flags: u32, time: *const ST, fmt: *const u16, out: *mut u16, cch: i32) -> i32;
}

//@ use=b:GetTimeFormatEx
poly_wtest::main!(test);

const NAMES: [(&str, Option<&str>); 8] = [
    ("en-US", Some("en-US")),
    ("en-GB", Some("en-GB")),
    ("de-DE", Some("de-DE")),
    ("de-CH", Some("de-CH")),
    ("fr-FR", Some("fr-FR")),
    ("inv", Some("")),
    ("it-IT", Some("it-IT")),
    ("ja-JP", Some("ja-JP")),
];

/// A locale name as a pointer: NULL for None.
fn np(n: &Option<[u16; 300]>) -> *const u16 {
    match n {
        Some(w) => w.as_ptr(),
        None => core::ptr::null(),
    }
}

fn tf(t: &mut Test, label: &str, name: Option<&str>, flags: u32, s: &ST, pic: Option<&str>) {
    let mut out = [0u16; 120];
    let nw = name.map(wide);
    let p = pic.map(wide);
    let pp = match &p {
        Some(w) => w.as_ptr(),
        None => core::ptr::null(),
    };
    clear_error();
    // SAFETY: valid SYSTEMTIME, name/picture (or NULL) and a 120-unit buffer.
    let n = unsafe { GetTimeFormatEx(np(&nw), flags, s, pp, out.as_mut_ptr(), 120) };
    let e = if n == 0 { gle() } else { -2 };
    say!(t, "{label}: n={n} err={e} '{}'", txt(&out));
}

fn test(t: &mut Test) {
    let times = [("morning", S_MORNING), ("night", S_NIGHT), ("noon", S_LEAP), ("midnight", S_MIDNIGHT), ("afternoon", S_AFTERNOON)];
    for (ln, n) in NAMES {
        for (tn, s) in &times {
            let mut b = Buf::<80>::new();
            let _ = write!(b, "{ln} {tn}");
            tf(t, b.as_str(), n, 0, s, None);
        }
    }
    let flags: [(&str, u32); 8] = [("NOMINSEC", 1), ("NOSEC", 2), ("NOMARK", 4), ("FORCE24", 8), ("NOSEC|NOMARK", 6), ("FORCE24|NOSEC", 10), ("FORCE24|NOMINSEC", 9), ("NOUSEROVERRIDE", 0x8000_0000)];
    for (ln, n) in [("en-US", Some("en-US")), ("de-CH", Some("de-CH")), ("inv", Some(""))] {
        for (fname, f) in flags {
            for (tn, s) in [("morning", S_MORNING), ("afternoon", S_AFTERNOON), ("midnight", S_MIDNIGHT)] {
                let mut b = Buf::<120>::new();
                let _ = write!(b, "{ln} {fname} {tn}");
                tf(t, b.as_str(), n, f, &s, None);
            }
        }
    }
    for p in ["h:mm:ss tt", "HH:mm", "hh 'o''clock' tt", "H.m.s", "t", "tt", "'x'", ""] {
        for (ln, n) in [("en-US", Some("en-US")), ("de-CH", Some("de-CH")), ("inv", Some(""))] {
            let mut b = Buf::<120>::new();
            let _ = write!(b, "{ln} [{p}]");
            tf(t, b.as_str(), n, 0, &S_AFTERNOON, Some(p));
        }
    }
}
