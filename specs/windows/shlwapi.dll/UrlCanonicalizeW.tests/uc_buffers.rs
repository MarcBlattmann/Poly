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
// UrlCanonicalizeW: buffer sizes, the needed length, NULL arguments, in-place use, long input.

#[link(name = "shlwapi", kind = "raw-dylib")]
unsafe extern "system" {
    fn UrlCanonicalizeW(url: *const u16, out: *mut u16, cch: *mut u32, flags: u32) -> i32;
}

poly_wtest::main!(test);

fn sized(t: &mut Test, label: &str, url: &str, cap: u32, flags: u32) {
    let w = wide(url);
    let mut out = [0xEEEEu16; 400];
    let mut cch = cap;
    // SAFETY: valid buffers; `cap` is at most 400.
    let hr = unsafe { UrlCanonicalizeW(w.as_ptr(), out.as_mut_ptr(), &mut cch, flags) };
    let n = out.iter().position(|&c| c == 0).unwrap_or(400);
    let touched = out.iter().take(30).filter(|&&c| c != 0xEEEE).count();
    say!(t, "{label}: capacity {cap}: hr {:#x}, cch {cch}, text len {}, units written (of first 30) {touched}", hr, if touched == 0 { 0 } else { n });
}

fn test(t: &mut Test) {
    // with URL_ESCAPE_UNSAFE "http://a/b c" becomes "http://a/b%20c" (14 units + NUL)
    for cap in [0, 1, 13, 14, 15, 16, 100] {
        sized(t, "escape grows", "http://a/b c", cap, 0x2000_0000);
    }
    // flags 0 leave the space alone: 12 units + NUL
    for cap in [0, 12, 13, 14] {
        sized(t, "space kept", "http://a/b c", cap, 0);
    }
    // an unchanged URL of 13 units
    for cap in [0, 12, 13, 14, 15] {
        sized(t, "unchanged", "http://a/b/cd", cap, 0);
    }
    // a shrinking URL
    for cap in [5, 12, 14] {
        sized(t, "shrinks", "http://a/./b/../c", cap, 0);
    }
    // NULL arguments
    {
        let w = wide("http://a/b");
        let mut out = [0u16; 50];
        let mut cch = 50u32;
        // SAFETY: the NULLs are the cases under test; other pointers are valid.
        unsafe {
            let hr = UrlCanonicalizeW(core::ptr::null(), out.as_mut_ptr(), &mut cch, 0);
            say!(t, "NULL url: hr {hr:#x}, cch {cch}");
            cch = 50;
            let hr = UrlCanonicalizeW(w.as_ptr(), core::ptr::null_mut(), &mut cch, 0);
            say!(t, "NULL output buffer, capacity 50: hr {hr:#x}, cch {cch}");
            let hr = UrlCanonicalizeW(w.as_ptr(), out.as_mut_ptr(), core::ptr::null_mut(), 0);
            say!(t, "NULL cch: hr {hr:#x}");
            cch = 0;
            let hr = UrlCanonicalizeW(w.as_ptr(), core::ptr::null_mut(), &mut cch, 0);
            say!(t, "NULL output buffer, capacity 0: hr {hr:#x}, cch {cch}");
        }
    }
    // the empty URL
    sized(t, "empty url", "", 10, 0);
    sized(t, "empty url, capacity 0", "", 0, 0);
    sized(t, "empty url, capacity 1", "", 1, 0);
    // in-place: the output buffer is the input
    {
        let mut buf = wide("http://a/./b c");
        let mut cch = 100u32;
        // SAFETY: the same valid buffer for input and output.
        let hr = unsafe { UrlCanonicalizeW(buf.as_ptr(), buf.as_mut_ptr(), &mut cch, 0) };
        let n = buf.iter().position(|&c| c == 0).unwrap_or(300);
        say!(t, "in place: hr {hr:#x}, cch {cch}, <{}>", units_text(&buf[..n]).as_str());
    }
    // a long URL (about 250 units) into a big buffer
    {
        let mut s = Buf::<400>::new();
        let _ = write!(s, "http://a/");
        for _ in 0..24 {
            let _ = write!(s, "0123456789");
        }
        let _ = write!(s, " z");
        let w = wide(s.as_str());
        let mut out = [0u16; 400];
        let mut cch = 400u32;
        // SAFETY: valid buffers.
        let hr = unsafe { UrlCanonicalizeW(w.as_ptr(), out.as_mut_ptr(), &mut cch, 0) };
        say!(t, "long url (250 units, one space): hr {hr:#x}, cch {cch}");
    }
    // 2048-unit limit related: a URL longer than 2084? (INTERNET_MAX_URL_LENGTH) is not tested; 300-unit URL:
    {
        let mut v = [0x61u16; 311];
        v[310] = 0;
        v[0] = 0x68;
        v[1] = 0x74;
        v[2] = 0x74;
        v[3] = 0x70;
        v[4] = 0x3a;
        v[5] = 0x2f;
        v[6] = 0x2f;
        v[7] = 0x61;
        v[8] = 0x2f;
        let mut out = [0u16; 400];
        let mut cch = 400u32;
        // SAFETY: valid buffers.
        let hr = unsafe { UrlCanonicalizeW(v.as_ptr(), out.as_mut_ptr(), &mut cch, 0) };
        say!(t, "long url (310 units): hr {hr:#x}, cch {cch}");
    }
}
