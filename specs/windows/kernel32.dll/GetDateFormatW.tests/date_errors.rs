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
}

//@ use=k:GetDateFormatW
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

fn test(t: &mut Test) {
    for (ln, l) in [("en-US", 0x409u32), ("de-CH", 0x807)] {
        for (fname, f) in [("short", 1u32), ("long", 2)] {
            let mut out = [0xAAAAu16; 96];
            clear_error();
            // SAFETY: cch 0: NULL buffer allowed.
            let n0 = unsafe { GetDateFormatW(l, f, &S_MORNING, core::ptr::null(), core::ptr::null_mut(), 0) };
            say!(t, "{ln} {fname} size query: n={n0} err={}", gle_if(n0));
            for delta in [-3i32, -1, 0, 1, 30] {
                for v in out.iter_mut() {
                    *v = 0xAAAA;
                }
                clear_error();
                // SAFETY: cch <= buffer length.
                let n = unsafe { GetDateFormatW(l, f, &S_MORNING, core::ptr::null(), out.as_mut_ptr(), n0 + delta) };
                let e = if n == 0 { gle() } else { -2 };
                if out.iter().all(|&c| c == 0xAAAA) {
                    say!(t, "{ln} {fname} cch={delta:+}: n={n} err={e} buffer untouched");
                } else {
                    say!(t, "{ln} {fname} cch={delta:+}: n={n} err={e} '{}'", txt(&out));
                }
            }
        }
    }
    {
        clear_error();
        // SAFETY: deliberately invalid: NULL buffer with a size.
        let n = unsafe { GetDateFormatW(0x409, 0, &S_MORNING, core::ptr::null(), core::ptr::null_mut(), 10) };
        say!(t, "NULL buffer cch=10: n={n} err={}", gle_if(n));
        let mut out = [0u16; 16];
        clear_error();
        // SAFETY: negative size invalid; buffer valid.
        let n = unsafe { GetDateFormatW(0x409, 0, &S_MORNING, core::ptr::null(), out.as_mut_ptr(), -1) };
        say!(t, "cch=-1: n={n} err={}", gle_if(n));
    }
    // flag combinations
    for (fname, f) in [
        ("SHORT|LONG", 3u32),
        ("SHORT|YEARMONTH", 9),
        ("LONG|YEARMONTH", 10),
        ("ALT_CALENDAR", 4),
        ("LTR", 0x10),
        ("RTL", 0x20),
        ("LTR|RTL", 0x30),
        ("AUTOLAYOUT", 0x40),
        ("AUTOLAYOUT|LONG", 0x42),
        ("MONTHDAY|LONG", 0x82),
        ("0x100", 0x100),
        ("0x1000", 0x1000),
        ("NOUSEROVERRIDE", 0x8000_0000),
        ("USE_CP_ACP", 0x4000_0000),
        ("NOUSEROVERRIDE|LONG", 0x8000_0002),
        ("0xFFFFFFFF", 0xFFFF_FFFF),
    ] {
        let mut b = Buf::<80>::new();
        let _ = write!(b, "flags {fname}");
        df(t, b.as_str(), 0x409, f, &S_MORNING, None);
    }
    for f in [1u32, 2, 8, 0x40] {
        let mut b = Buf::<80>::new();
        let _ = write!(b, "picture with flags {f}");
        df(t, b.as_str(), 0x409, f, &S_MORNING, Some("yyyy-MM-dd"));
    }
    // invalid SYSTEMTIME members
    for (nm, s) in [
        ("month 0", st(2024, 0, 1, 5, 0, 0, 0, 0)),
        ("month 13", st(2024, 13, 1, 5, 0, 0, 0, 0)),
        ("day 0", st(2024, 3, 1, 0, 0, 0, 0, 0)),
        ("day 32", st(2024, 3, 1, 32, 0, 0, 0, 0)),
        ("feb 30", st(2024, 2, 1, 30, 0, 0, 0, 0)),
        ("feb 29 2023", st(2023, 2, 1, 29, 0, 0, 0, 0)),
        ("feb 29 1900", st(1900, 2, 1, 29, 0, 0, 0, 0)),
        ("feb 29 2000", st(2000, 2, 1, 29, 0, 0, 0, 0)),
        ("apr 31", st(2024, 4, 1, 31, 0, 0, 0, 0)),
        ("year 1600", st(1600, 12, 1, 31, 0, 0, 0, 0)),
        ("year 1601", st(1601, 1, 1, 1, 0, 0, 0, 0)),
        ("year 30827", st(30827, 12, 1, 31, 0, 0, 0, 0)),
        ("year 30828", st(30828, 1, 1, 1, 0, 0, 0, 0)),
        ("year 0", st(0, 1, 1, 1, 0, 0, 0, 0)),
        ("year 65535", st(65535, 1, 1, 1, 0, 0, 0, 0)),
        ("hour 99 (not looked at)", st(2024, 3, 1, 5, 99, 99, 99, 9999)),
    ] {
        df(t, nm, 0x409, 0, &s, None);
    }
    for (nm, l) in [("0x1234", 0x1234u32), ("0xFFFF", 0xFFFF), ("0x10409 (sort id)", 0x10409), ("0x9 neutral en", 9)] {
        df(t, nm, l, 0, &S_MORNING, None);
    }
    // NULL SYSTEMTIME: today; check the shape only (invariant short date is MM/dd/yyyy)
    {
        let mut out = [0u16; 64];
        clear_error();
        // SAFETY: NULL time is allowed.
        let n = unsafe { GetDateFormatW(0x7F, 0, core::ptr::null(), core::ptr::null(), out.as_mut_ptr(), 64) };
        let w = ulen(&out);
        let ok = w == 10 && out[2] == b'/' as u16 && out[5] == b'/' as u16 && [0, 1, 3, 4, 6, 7, 8, 9].iter().all(|&i| (0x30..=0x39).contains(&out[i]));
        say!(t, "NULL time invariant: n={n} len={w} shape_MM/dd/yyyy={ok}");
        let mut out = [0u16; 64];
        // SAFETY: NULL time allowed.
        let n = unsafe { GetDateFormatW(0x409, 2, core::ptr::null(), core::ptr::null(), out.as_mut_ptr(), 64) };
        say!(t, "NULL time en-US long: n_matches_len={} nonempty={}", n as usize == ulen(&out) + 1, n > 1);
    }
}
