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
    fn GetUserPreferredUILanguages(flags: u32, num: *mut u32, buf: *mut u16, cch: *mut u32) -> BOOL;
    fn GetUserDefaultUILanguage() -> u16;
}

#[link(name = "kernelbase", kind = "raw-dylib")]
unsafe extern "system" {
    fn LCIDToLocaleName(lcid: u32, name: *mut u16, cch: i32, flags: u32) -> i32;
    fn LocaleNameToLCID(name: *const u16, flags: u32) -> u32;
}

//@ use=k:GetUserPreferredUILanguages,k:GetUserDefaultUILanguage,b:LCIDToLocaleName,b:LocaleNameToLCID
poly_wtest::main!(test);

struct Langs {
    ok: bool,
    err: i64,
    num: u32,
    cch_size: u32,
    cch_got: u32,
    buf: [u16; 600],
}

/// Two calls: a size query, then the fetch.
fn get(flags: u32) -> Langs {
    let mut l = Langs { ok: false, err: -2, num: 0xAAAA, cch_size: 0, cch_got: 0, buf: [0; 600] };
    clear_error();
    // SAFETY: size query: NULL buffer, valid counters.
    let ok = unsafe { GetUserPreferredUILanguages(flags, &mut l.num, core::ptr::null_mut(), &mut l.cch_size) };
    if ok == 0 {
        l.err = gle();
        return l;
    }
    l.ok = true;
    let mut n2 = 0u32;
    l.cch_got = l.cch_size;
    // SAFETY: valid buffer of 600 units (the size is checked).
    if l.cch_size as usize <= 600 {
        let ok = unsafe { GetUserPreferredUILanguages(flags, &mut n2, l.buf.as_mut_ptr(), &mut l.cch_got) };
        l.ok = ok != 0;
        if ok == 0 {
            l.err = gle();
        }
    }
    l
}

/// Whether `b[..cch]` is a well-formed multi-string: strings, each non-empty, ended by an empty one; returns the number of strings (or -1).
fn multi(b: &[u16], cch: usize) -> i32 {
    let mut i = 0;
    let mut n = 0;
    while i < cch {
        let mut j = i;
        while j < cch && b[j] != 0 {
            j += 1;
        }
        if j >= cch {
            return -1;
        }
        if j == i {
            return if i + 1 == cch { n } else { -1 };
        }
        n += 1;
        i = j + 1;
    }
    -1
}

fn test(t: &mut Test) {
    // MUI_LANGUAGE_ID 4, MUI_LANGUAGE_NAME 8; the preferred list is the machine's, only its structure is checked
    for (nm, f) in [("NAME 8", 8u32), ("ID 4", 4), ("0", 0)] {
        let l = get(f);
        let n = multi(&l.buf, l.cch_got as usize);
        let first_len = ulen(&l.buf);
        let all_ascii = l.buf[..l.cch_got as usize].iter().all(|&c| c < 0x7f);
        say!(
            t,
            "flags {nm}: ok={} err={} num>=1:{} size_query_equals_fetch:{} well_formed_multistring:{} strings_equals_num:{} first_nonempty:{} ascii:{}",
            l.ok,
            l.err,
            l.num >= 1,
            l.cch_size == l.cch_got,
            n >= 0,
            n == l.num as i32,
            first_len > 0,
            all_ascii
        );
        if f == 4 {
            // every string of ID form is four hexadecimal digits
            let mut i = 0usize;
            let mut all4 = true;
            let mut digits = true;
            while i < l.cch_got as usize && l.buf[i] != 0 {
                let len = ulen(&l.buf[i..]);
                all4 &= len == 4;
                digits &= l.buf[i..i + len].iter().all(|&c| (0x30..=0x39).contains(&c) || (0x41..=0x46).contains(&c) || (0x61..=0x66).contains(&c));
                i += len + 1;
            }
            say!(t, "flags {nm}: every string has 4 characters: {all4}, hex digits only: {digits}");
        }
        if f == 8 {
            // every string is a locale name that LocaleNameToLCID knows
            let mut i = 0usize;
            let mut known = true;
            while i < l.cch_got as usize && l.buf[i] != 0 {
                let len = ulen(&l.buf[i..]);
                // SAFETY: NUL-terminated string inside the buffer.
                let lcid = unsafe { LocaleNameToLCID(l.buf[i..].as_ptr(), 0x0800_0000) };
                known &= lcid != 0;
                i += len + 1;
            }
            say!(t, "flags NAME: every name is known to LocaleNameToLCID: {known}");
        }
    }
    // the relation to the user's UI language
    // SAFETY: plain call.
    let ui = unsafe { GetUserDefaultUILanguage() };
    let mut out = [0u16; 100];
    // SAFETY: valid buffer.
    let r = unsafe { LCIDToLocaleName(u32::from(ui), out.as_mut_ptr(), 100, 0x0800_0000) };
    let name = get(8);
    let id = get(4);
    say!(t, "first preferred name is the name of GetUserDefaultUILanguage: {}", r > 0 && name.buf[..ulen(&name.buf)] == out[..ulen(&out)]);
    let mut idtxt = Buf::<8>::new();
    let _ = write!(idtxt, "{ui:04x}");
    let first: Buf<8> = {
        let mut b = Buf::<8>::new();
        for &c in &id.buf[..ulen(&id.buf).min(8)] {
            let _ = write!(b, "{}", (c as u8 as char).to_ascii_lowercase());
        }
        b
    };
    say!(t, "first preferred ID is GetUserDefaultUILanguage in four lowercase-insensitive hex digits: {}", idtxt.as_str() == first.as_str());
}
