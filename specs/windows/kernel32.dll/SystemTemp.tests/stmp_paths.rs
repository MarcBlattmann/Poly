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
// GetTempPathW / GetTempPath2W and the environment against C:\Windows\SystemTemp, for each token kind.

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetTempPathW(len: u32, buf: *mut u16) -> u32;
    fn GetTempPath2W(len: u32, buf: *mut u16) -> u32;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, len: u32) -> u32;
    fn GetWindowsDirectoryW(buf: *mut u16, len: u32) -> u32;
    fn GetSystemWindowsDirectoryW(buf: *mut u16, len: u32) -> u32;
}

poly_wtest::main!(test);

fn starts_with_nocase(a: &[u16], b: &[u16]) -> bool {
    a.len() >= b.len() && eq_nocase(&a[..b.len()], b)
}

fn test(t: &mut Test) {
    let mut a = [0u16; 300];
    let mut b = [0u16; 300];
    // SAFETY: valid buffers.
    let (na, nb) = unsafe { (GetTempPathW(300, a.as_mut_ptr()), GetTempPath2W(300, b.as_mut_ptr())) };
    let (ta, tb) = (&a[..na as usize], &b[..nb as usize]);
    say!(t, "GetTempPathW ends with a backslash: {}", ta.last() == Some(&0x5c));
    say!(t, "GetTempPath2W equals GetTempPathW: {}", ta == tb);
    let sys = wide("C:\\Windows\\SystemTemp");
    let sys = &sys[..wstr_at(sys.as_ptr(), 300).len()];
    say!(t, "GetTempPathW is below C:\\Windows\\SystemTemp: {}", starts_with_nocase(ta, sys));
    say!(t, "GetTempPath2W is below C:\\Windows\\SystemTemp: {}", starts_with_nocase(tb, sys));
    let win = wide("C:\\Windows");
    let win = &win[..10];
    say!(t, "GetTempPathW is below C:\\Windows: {}", starts_with_nocase(ta, win));
    // the environment
    for name in ["TMP", "TEMP"] {
        let w = wide(name);
        let mut v = [0u16; 300];
        // SAFETY: valid buffers.
        let n = unsafe { GetEnvironmentVariableW(w.as_ptr(), v.as_mut_ptr(), 300) };
        let same = n > 0 && starts_with_nocase(ta, &v[..n as usize]);
        say!(t, "{name} is set: {}; GetTempPathW starts with it: {same}", n > 0);
    }
    let mut s = [0u16; 300];
    // SAFETY: valid buffers.
    let n = unsafe { GetSystemWindowsDirectoryW(s.as_mut_ptr(), 300) };
    let mut w2 = [0u16; 300];
    // SAFETY: valid buffers.
    let n2 = unsafe { GetWindowsDirectoryW(w2.as_mut_ptr(), 300) };
    say!(t, "GetSystemWindowsDirectoryW equals GetWindowsDirectoryW: {} (C:\\Windows: {})", s[..n as usize] == w2[..n2 as usize], eq_nocase(&s[..n as usize], win));
}
