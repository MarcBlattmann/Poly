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
    fn GetDateFormatEx(name: *const u16, flags: u32, time: *const ST, fmt: *const u16, out: *mut u16, cch: i32, cal: *const u16) -> i32;
    fn GetUserDefaultLocaleName(name: *mut u16, cch: i32) -> i32;
    fn GetSystemDefaultLocaleName(name: *mut u16, cch: i32) -> i32;
}

//@ use=b:GetDateFormatEx,b:GetUserDefaultLocaleName,b:GetSystemDefaultLocaleName
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

const DATES: [(&str, ST); 6] = [
    ("tue-2024-03-05", S_MORNING),
    ("sun-2023-12-31", S_NIGHT),
    ("thu-2024-02-29", S_LEAP),
    ("sun-2024-07-14", S_MIDNIGHT),
    ("mon-1601-01-01", S_EPOCH),
    ("wed-2025-11-05", S_AFTERNOON),
];

fn df(t: &mut Test, label: &str, name: Option<&str>, flags: u32, s: &ST, pic: Option<&str>, cal: Option<&str>) {
    let mut out = [0u16; 160];
    let nw = name.map(wide);
    let p = pic.map(wide);
    let c = cal.map(wide);
    let pp = match &p {
        Some(w) => w.as_ptr(),
        None => core::ptr::null(),
    };
    let cp = match &c {
        Some(w) => w.as_ptr(),
        None => core::ptr::null(),
    };
    clear_error();
    // SAFETY: valid SYSTEMTIME, name/picture/calendar (or NULL) and a 160-unit buffer.
    let n = unsafe { GetDateFormatEx(np(&nw), flags, s, pp, out.as_mut_ptr(), 160, cp) };
    let e = if n == 0 { gle() } else { -2 };
    say!(t, "{label}: n={n} err={e} '{}'", txt(&out));
}

fn same(a: &[u16], b: &[u16]) -> bool {
    a[..ulen(a)] == b[..ulen(b)]
}

fn test(t: &mut Test) {
    for nm in [
        "en-US", "EN-us", "en", "de", "fr", "de-DE_phoneb", "en-US_tradnl", "zh-Hans", "zh-CN", "x-IV_mathan", "ja-JP",
    ] {
        let mut b = Buf::<120>::new();
        let _ = write!(b, "name [{nm}]");
        df(t, b.as_str(), Some(nm), 2, &S_AFTERNOON, None, None);
    }
    for nm in ["xx-XX", "en_US", "en US", " en-US", "en-US ", "0409", "00000409", "zz", "e", "en-", "-US"] {
        let mut b = Buf::<120>::new();
        let _ = write!(b, "bad name [{nm}]");
        df(t, b.as_str(), Some(nm), 0, &S_AFTERNOON, None, None);
    }
    let long = "en-US-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    df(t, "name of 86 units", Some(long), 0, &S_AFTERNOON, None, None);
    // NULL is the user default; !x-sys-default-locale the system default
    let mut un = [0u16; 90];
    let mut sn = [0u16; 90];
    // SAFETY: valid buffers.
    unsafe {
        GetUserDefaultLocaleName(un.as_mut_ptr(), 90);
        GetSystemDefaultLocaleName(sn.as_mut_ptr(), 90);
    }
    let run = |name: *const u16, flags: u32| -> ([u16; 160], i32) {
        let mut o = [0u16; 160];
        // SAFETY: valid pointers.
        let n = unsafe { GetDateFormatEx(name, flags, &S_AFTERNOON, core::ptr::null(), o.as_mut_ptr(), 160, core::ptr::null()) };
        (o, n)
    };
    let sysname = wide("!x-sys-default-locale");
    for (fname, f) in [("none", 0u32), ("LONG", 2), ("YEARMONTH", 8), ("NOUSEROVERRIDE", 0x8000_0000)] {
        let (a, an) = run(core::ptr::null(), f);
        let (b, _) = run(un.as_ptr(), f);
        let (c, cn) = run(sysname.as_ptr(), f);
        let (d, _) = run(sn.as_ptr(), f);
        say!(t, "{fname}: NULL ok={} same as the user's own name {}; !x-sys-default-locale ok={} same as the system's own name {}", an > 0, same(&a, &b), cn > 0, same(&c, &d));
    }
    // the calendar parameter (Learn: reserved, must be NULL)
    for (nm, c) in [("NULL", None), ("empty", Some("")), ("gregorian text", Some("Gregorian")), ("1", Some("1"))] {
        let mut b = Buf::<80>::new();
        let _ = write!(b, "calendar {nm}");
        df(t, b.as_str(), Some("en-US"), 0, &S_MORNING, None, c);
    }
}
