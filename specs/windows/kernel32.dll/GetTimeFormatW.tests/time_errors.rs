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
    fn GetTimeFormatW(locale: u32, flags: u32, time: *const ST, fmt: *const u16, out: *mut u16, cch: i32) -> i32;
}

//@ use=k:GetTimeFormatW
poly_wtest::main!(test);

fn tf(t: &mut Test, label: &str, loc: u32, flags: u32, s: &ST, pic: Option<&str>) {
    let mut out = [0u16; 120];
    let p = pic.map(wide);
    let pp = match &p {
        Some(w) => w.as_ptr(),
        None => core::ptr::null(),
    };
    clear_error();
    // SAFETY: valid SYSTEMTIME, picture (or NULL) and a 120-unit buffer.
    let n = unsafe { GetTimeFormatW(loc, flags, s, pp, out.as_mut_ptr(), 120) };
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

fn bad(h: u16, mi: u16, s: u16, ms: u16) -> ST {
    st(2024, 3, 2, 5, h, mi, s, ms)
}

fn test(t: &mut Test) {
    // buffer sizing: cch 0 asks for the size (with the NUL), an exact buffer works, one short fails
    for (ln, l) in [("en-US", 0x409u32), ("de-CH", 0x807)] {
        for (tn, s) in [("morning", S_MORNING), ("afternoon", S_AFTERNOON)] {
            let mut out = [0xAAAAu16; 64];
            clear_error();
            // SAFETY: cch 0: NULL buffer allowed.
            let n0 = unsafe { GetTimeFormatW(l, 0, &s, core::ptr::null(), core::ptr::null_mut(), 0) };
            let e0 = if n0 == 0 { gle() } else { -2 };
            say!(t, "{ln} {tn} size query: n={n0} err={e0}");
            for delta in [-3i32, -2, -1, 0, 1, 20] {
                let cch = n0 + delta;
                for v in out.iter_mut() {
                    *v = 0xAAAA;
                }
                clear_error();
                // SAFETY: cch <= buffer length (64).
                let n = unsafe { GetTimeFormatW(l, 0, &s, core::ptr::null(), out.as_mut_ptr(), cch) };
                let e = if n == 0 { gle() } else { -2 };
                let w = ulen(&out);
                let untouched = out.iter().all(|&c| c == 0xAAAA);
                if untouched {
                    say!(t, "{ln} {tn} cch={delta:+}: n={n} err={e} buffer untouched");
                } else {
                    say!(t, "{ln} {tn} cch={delta:+}: n={n} err={e} len={w} '{}' after={:#x}", txt(&out), out[(w + 1).min(63)]);
                }
            }
        }
    }
    // a picture sizes the same way
    {
        let p = wide("HH:mm:ss");
        let mut out = [0u16; 32];
        // SAFETY: valid pointers.
        let n0 = unsafe { GetTimeFormatW(0x409, 0, &S_MORNING, p.as_ptr(), core::ptr::null_mut(), 0) };
        clear_error();
        // SAFETY: valid pointers; the buffer is one unit too short.
        let n1 = unsafe { GetTimeFormatW(0x409, 0, &S_MORNING, p.as_ptr(), out.as_mut_ptr(), n0 - 1) };
        say!(t, "picture size query n={n0}, short n={n1} err={}", gle_if(n1));
    }
    // NULL output with cch > 0, negative cch
    {
        clear_error();
        // SAFETY: deliberately invalid: NULL buffer with a size.
        let n = unsafe { GetTimeFormatW(0x409, 0, &S_MORNING, core::ptr::null(), core::ptr::null_mut(), 10) };
        say!(t, "NULL buffer cch=10: n={n} err={}", gle_if(n));
        let mut out = [0u16; 16];
        clear_error();
        // SAFETY: negative size is invalid, the buffer is valid.
        let n = unsafe { GetTimeFormatW(0x409, 0, &S_MORNING, core::ptr::null(), out.as_mut_ptr(), -1) };
        say!(t, "cch=-1: n={n} err={}", gle_if(n));
    }
    // invalid flags and flag/picture combinations
    for (fname, f) in [("0x10", 0x10u32), ("0x20", 0x20), ("0x100", 0x100), ("0x1000", 0x1000), ("0x80", 0x80), ("0xFFFFFFFF", 0xFFFF_FFFF)] {
        let mut b = Buf::<80>::new();
        let _ = write!(b, "flags {fname}");
        tf(t, b.as_str(), 0x409, f, &S_MORNING, None);
    }
    for f in [1u32, 2, 4, 8] {
        let mut b = Buf::<80>::new();
        let _ = write!(b, "picture with flags {f}");
        tf(t, b.as_str(), 0x409, f, &S_MORNING, Some("HH:mm"));
    }
    // invalid SYSTEMTIME members (only the time of day is looked at)
    for (nm, s) in [
        ("hour 24", bad(24, 0, 0, 0)),
        ("hour 65535", bad(65535, 0, 0, 0)),
        ("minute 60", bad(1, 60, 0, 0)),
        ("second 60", bad(1, 1, 60, 0)),
        ("ms 1000", bad(1, 1, 1, 1000)),
        ("ms 65535", bad(1, 1, 1, 65535)),
        ("max valid", bad(23, 59, 59, 999)),
    ] {
        tf(t, nm, 0x409, 0, &s, None);
    }
    // the date part is not checked when only the time is formatted
    tf(t, "month 0 day 0", 0x409, 0, &st(2024, 0, 9, 0, 1, 2, 3, 4), None);
    tf(t, "year 0", 0x409, 0, &st(0, 1, 1, 1, 1, 2, 3, 4), None);
    tf(t, "year 65535", 0x409, 0, &st(65535, 12, 1, 31, 1, 2, 3, 4), None);
    // unknown locales
    for (nm, l) in [("0x1234", 0x1234u32), ("0xFFFF", 0xFFFF), ("0x10409 (sort id)", 0x10409), ("0x9 neutral en", 9)] {
        tf(t, nm, l, 0, &S_MORNING, None);
    }
    // these succeed and mean the user default; the invariant locale with a sort id means the invariant locale
    for (nm, l, user) in [("0x0000", 0u32, true), ("0x1000 custom unspecified", 0x1000, true), ("0x2000", 0x2000, true), ("0x1007F inv with sort id", 0x1007F, false)] {
        let mut a = [0u16; 64];
        let mut u = [0u16; 64];
        // SAFETY: valid pointers.
        let n = unsafe { GetTimeFormatW(l, 0, &S_AFTERNOON, core::ptr::null(), a.as_mut_ptr(), 64) };
        let e = if n == 0 { gle() } else { -2 };
        // SAFETY: valid pointers.
        unsafe { GetTimeFormatW(if user { 0x400 } else { 0x7F }, 0, &S_AFTERNOON, core::ptr::null(), u.as_mut_ptr(), 64) };
        say!(t, "{nm}: ok={} err={e} same as {} {}", n > 0, if user { "user default" } else { "invariant" }, a == u);
    }
    // NULL SYSTEMTIME: the current time; only its shape is checked
    {
        let mut out = [0u16; 64];
        clear_error();
        // SAFETY: NULL time is allowed.
        let n = unsafe { GetTimeFormatW(0x7F, 8, core::ptr::null(), core::ptr::null(), out.as_mut_ptr(), 64) };
        let w = ulen(&out);
        let shape = w == 8 && out[2] == b':' as u16 && out[5] == b':' as u16 && [0, 1, 3, 4, 6, 7].iter().all(|&i| (0x30..=0x39).contains(&out[i]));
        say!(t, "NULL time invariant FORCE24: n={n} len={w} shape_hh:mm:ss={shape}");
        let mut out = [0u16; 64];
        // SAFETY: NULL time allowed.
        let n = unsafe { GetTimeFormatW(0x409, 0, core::ptr::null(), core::ptr::null(), out.as_mut_ptr(), 64) };
        let w = ulen(&out);
        let tail = w >= 2 && (out[w - 2] == b'A' as u16 || out[w - 2] == b'P' as u16) && out[w - 1] == b'M' as u16;
        say!(t, "NULL time en-US: n={n} len_matches={} ends_AM_or_PM={tail}", n as usize == w + 1);
    }
}
