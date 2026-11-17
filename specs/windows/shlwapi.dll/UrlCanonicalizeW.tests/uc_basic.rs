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
// UrlCanonicalizeW with flags 0: scheme handling, dot segments, spaces and other unsafe characters, file URLs, backslashes.

#[link(name = "shlwapi", kind = "raw-dylib")]
unsafe extern "system" {
    fn UrlCanonicalizeW(url: *const u16, out: *mut u16, cch: *mut u32, flags: u32) -> i32;
}

poly_wtest::main!(test);

fn canon(t: &mut Test, url: &str, flags: u32) {
    let w = wide(url);
    let mut out = [0xEEEEu16; 400];
    let mut cch = 400u32;
    // SAFETY: valid buffers.
    let hr = unsafe { UrlCanonicalizeW(w.as_ptr(), out.as_mut_ptr(), &mut cch, flags) };
    let n = out.iter().position(|&c| c == 0).unwrap_or(400);
    say!(t, "flags {flags:#x} <{}> -> hr {:#x}, cch {cch}, <{}>", units_text(&w[..url.chars().count()]).as_str(), hr, units_text(&out[..n]).as_str());
}

fn test(t: &mut Test) {
    for u in [
        "http://www.example.com/a/b",
        "http://www.example.com",
        "HTTP://WWW.EXAMPLE.COM/A",
        "http://a/./b/../c",
        "http://a/b/../../c",
        "http://a/b/./",
        "http://a/b/..",
        "http://a/b/c/../../../d",
        "http://a//b///c",
        "http://a/b c/d e",
        "http://a/b%20c",
        "http://a/b%2fc",
        "http://a/%7euser",
        "http://a/b?x=1 2&y=./z",
        "http://a/b?q=../c",
        "http://a/b#frag ment",
        "http://a/b\\c\\d",
        "http://a\\b",
        "http://user:pw@host:80/p",
        "http://a/b|c^d{e}f",
        "http://a/\u{e9}",
        "www.example.com/a/../b",
        "example",
        "a b",
        "/a/./b/../c",
        "./a/b",
        "..\\a",
        "C:\\a\\b c.txt",
        "C:\\a\\.\\b\\..\\c",
        "\\\\server\\share\\a b",
        "file:///C:/a/b%20c",
        "file:///C:/a b/c",
        "file:///C:\\a\\b",
        "file://server/share/a b",
        "file:///C:/a/./b/../c",
        "file:c:/a/b",
        "mailto:a@b.c",
        "javascript:alert(1)",
        "about:blank",
        "res://x.dll/1",
        "ftp://h/a b",
        "https://a/b/./c",
        "x-custom://a/./b/../c d",
        "",
        "  http://a/b  ",
        "http://a/b/",
        "http://a/b/.",
        "http://a/.",
        "http://a/..",
        "http://a/%",
        "http://a/%zz",
        "http://a/a%20%20b",
    ] {
        canon(t, u, 0);
    }
}
