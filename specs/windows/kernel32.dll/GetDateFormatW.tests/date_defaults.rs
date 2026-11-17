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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDateFormatW(locale: u32, flags: u32, time: *const ST, fmt: *const u16, out: *mut u16, cch: i32) -> i32;
    fn GetUserDefaultLCID() -> u32;
    fn GetSystemDefaultLCID() -> u32;
}

//@ use=k:GetDateFormatW,k:GetUserDefaultLCID,k:GetSystemDefaultLCID
poly_wtest::main!(test);

fn df(t: &mut Test, label: &str, loc: u32, flags: u32, s: &ST, pic: Option<&str>) {
    let mut out = [0u16; 160];
    let p = pic.map(wide);
    let pp = match &p {
        Some(w) => w.as_ptr(),
        None => core::ptr::null(),
    };
    clear_error();
    // SAFETY: valid SYSTEMTIME, picture (or NULL) and a 160-unit buffer.
    let n = unsafe { GetDateFormatW(loc, flags, s, pp, out.as_mut_ptr(), 160) };
    let e = if n == 0 { gle() } else { -2 };
    say!(t, "{label}: n={n} err={e} '{}'", txt(&out));
}

const LOCS: [(&str, u32); 8] = [
    ("en-US", 0x409),
    ("en-GB", 0x809),
    ("de-DE", 0x407),
    ("de-CH", 0x807),
    ("fr-FR", 0x40C),
    ("inv", 0x7F),
    ("it-IT", 0x410),
    ("ja-JP", 0x411),
];

const DATES: [(&str, ST); 6] = [
    ("tue-2024-03-05", S_MORNING),
    ("sun-2023-12-31", S_NIGHT),
    ("thu-2024-02-29", S_LEAP),
    ("sun-2024-07-14", S_MIDNIGHT),
    ("mon-1601-01-01", S_EPOCH),
    ("wed-2025-11-05", S_AFTERNOON),
];

fn fmt(l: u32, f: u32, s: &ST, p: Option<&str>) -> ([u16; 160], i32) {
    let mut out = [0u16; 160];
    let w = p.map(wide);
    let pp = match &w {
        Some(w) => w.as_ptr(),
        None => core::ptr::null(),
    };
    // SAFETY: valid pointers.
    let n = unsafe { GetDateFormatW(l, f, s, pp, out.as_mut_ptr(), 160) };
    (out, n)
}

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    let (ulcid, slcid) = unsafe { (GetUserDefaultLCID(), GetSystemDefaultLCID()) };
    for (nm, s) in [("morning", S_MORNING), ("afternoon", S_AFTERNOON), ("night", S_NIGHT)] {
        for (fname, f) in [("none", 0u32), ("SHORT", 1), ("LONG", 2), ("YEARMONTH", 8), ("NOUSEROVERRIDE", 0x8000_0000), ("NOUSEROVERRIDE|LONG", 0x8000_0002)] {
            let (a, an) = fmt(0x400, f, &s, None);
            let (b, bn) = fmt(ulcid, f, &s, None);
            let (c, cn) = fmt(0x800, f, &s, None);
            let (d, dn) = fmt(slcid, f, &s, None);
            say!(t, "{nm} {fname}: user-default==own-lcid {} (n {} {}); system-default==own-lcid {} (n {} {}); nonempty {}", a == b, an > 1, bn > 1, c == d, cn > 1, dn > 1, ulen(&a) > 0);
        }
    }
    let (a, _) = fmt(0, 0, &S_MORNING, None);
    let (b, _) = fmt(0x400, 0, &S_MORNING, None);
    say!(t, "LCID 0 == user default: {}", a == b);
    let (a, _) = fmt(0x7F, 0, &S_AFTERNOON, None);
    let (b, _) = fmt(0x1007F, 0, &S_AFTERNOON, None);
    say!(t, "inv 0x7F '{}' vs 0x1007F '{}'", txt(&a), txt(&b));
    for (nm, l) in [("0x1000 custom unspecified", 0x1000u32), ("0x2000", 0x2000)] {
        let (a, n) = fmt(l, 0, &S_AFTERNOON, None);
        let (u, _) = fmt(0x400, 0, &S_AFTERNOON, None);
        say!(t, "{nm}: ok={} same as user default {}", n > 0, a == u);
    }
    let (a, _) = fmt(0x400, 0, &S_AFTERNOON, Some("yyyy.MM.dd dddd"));
    say!(t, "user default with picture: '{}'", txt(&a));
}
