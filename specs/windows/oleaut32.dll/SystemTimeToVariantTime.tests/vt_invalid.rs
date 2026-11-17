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
// SystemTimeToVariantTime: fields out of range (month, day, hour, minute, second), February 29 and 30.

#[repr(C)]
struct SystemTime {
    year: u16,
    month: u16,
    dow: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    ms: u16,
}

#[link(name = "oleaut32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SystemTimeToVariantTime(st: *const SystemTime, out: *mut f64) -> i32;
}

poly_wtest::main!(test);

fn conv(t: &mut Test, label: &str, y: u16, mo: u16, d: u16, h: u16, mi: u16, s: u16, ms: u16, dow: u16) {
    let st = SystemTime { year: y, month: mo, dow, day: d, hour: h, minute: mi, second: s, ms };
    let mut out = f64::from_bits(0xAAAA_AAAA_AAAA_AAAA);
    // SAFETY: valid pointers.
    let r = unsafe { SystemTimeToVariantTime(&st, &mut out) };
    if r != 0 {
        // print the value scaled to microdays (rounded) and the exact bit pattern
        let scaled = (out * 1e6 + if out >= 0.0 { 0.5 } else { -0.5 }) as i64;
        say!(t, "{label}: ret={r} value*1e6={scaled} bits={:#018x}", out.to_bits());
    } else {
        say!(t, "{label}: ret={r} out untouched: {}", out.to_bits() == 0xAAAA_AAAA_AAAA_AAAA);
    }
}

fn test(t: &mut Test) {
    conv(t, "month 0", 2000, 0, 1, 0, 0, 0, 0, 0);
    conv(t, "month 13", 2000, 13, 1, 0, 0, 0, 0, 0);
    conv(t, "day 0", 2000, 1, 0, 0, 0, 0, 0, 0);
    conv(t, "day 32", 2000, 1, 32, 0, 0, 0, 0, 0);
    conv(t, "Feb 30", 2000, 2, 30, 0, 0, 0, 0, 0);
    conv(t, "Feb 29 in a non-leap year", 2001, 2, 29, 0, 0, 0, 0, 0);
    conv(t, "hour 24", 2000, 1, 1, 24, 0, 0, 0, 0);
    conv(t, "minute 60", 2000, 1, 1, 0, 60, 0, 0, 0);
    conv(t, "second 60", 2000, 1, 1, 0, 0, 60, 0, 0);
}
