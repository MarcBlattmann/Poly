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
    fn LocaleNameToLCID(name: *const u16, flags: u32) -> u32;
    fn LCIDToLocaleName(lcid: u32, name: *mut u16, cch: i32, flags: u32) -> i32;
    fn GetUserDefaultLocaleName(name: *mut u16, cch: i32) -> i32;
    fn GetSystemDefaultLocaleName(name: *mut u16, cch: i32) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetUserDefaultLCID() -> u32;
    fn GetSystemDefaultLCID() -> u32;
}

//@ use=b:LocaleNameToLCID,b:LCIDToLocaleName,b:GetUserDefaultLocaleName,b:GetSystemDefaultLocaleName,k:GetUserDefaultLCID,k:GetSystemDefaultLCID
poly_wtest::main!(test);

fn n2l(name: Option<&str>, flags: u32) -> (u32, i64) {
    let w = name.map(wide);
    let p = match &w {
        Some(w) => w.as_ptr(),
        None => core::ptr::null(),
    };
    clear_error();
    // SAFETY: valid name pointer (or NULL).
    let r = unsafe { LocaleNameToLCID(p, flags) };
    (r, if r == 0 { gle() } else { -2 })
}

fn show(t: &mut Test, label: &str, name: Option<&str>, flags: u32) {
    let (r, e) = n2l(name, flags);
    say!(t, "{label} flags {flags:#x}: lcid={r:#x} err={e}");
}

fn test(t: &mut Test) {
    // case, separators and malformed names
    for n in [
        "EN-US", "en-us", "En-uS", "en_US", "en_us", "en-US_tradnl", "en-US-", "-en-US", "en--US", "en US", " en-US", "en-US ", "e", "en-", "-", "!", "zz", "zz-ZZ", "xx-XX", "en-ZZ", "0409", "00000409", "0x409", "409",
        "!x-sys-default-locale ", "!x-user-default-locale", "x-iv_mathan", "X-IV_MATHAN", "x", "x-", "aa-bb-cc-dd-ee", "i-klingon", "de-DE-1996", "de-u-co-phonebk",
    ] {
        show(t, n, Some(n), 0);
    }
    // NULL name: the user default locale; !x-sys-default-locale: the system default locale (the values are the machine's, only relations are printed)
    {
        let mut un = [0u16; 100];
        let mut sn = [0u16; 100];
        // SAFETY: valid buffers.
        unsafe {
            GetUserDefaultLocaleName(un.as_mut_ptr(), 100);
            GetSystemDefaultLocaleName(sn.as_mut_ptr(), 100);
        }
        // SAFETY: NUL-terminated names from the calls above.
        let ulcid = unsafe { LocaleNameToLCID(un.as_ptr(), 0) };
        // SAFETY: as above.
        let slcid = unsafe { LocaleNameToLCID(sn.as_ptr(), 0) };
        for f in [0u32, 0x0800_0000] {
            let (r, e) = n2l(None, f);
            say!(t, "NULL flags {f:#x}: nonzero={} err={e} equals the LCID of the user's own locale name {}", r != 0, r == ulcid);
        }
        for nm in ["!x-sys-default-locale", "!X-SYS-DEFAULT-LOCALE"] {
            let (r, e) = n2l(Some(nm), 0);
            say!(t, "{nm}: nonzero={} err={e} equals the LCID of the system's own locale name {}", r != 0, r == slcid);
        }
    }
    // a name that is too long (LOCALE_NAME_MAX_LENGTH 85 units with the NUL)
    let l84 = "en-US-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let l86 = "en-US-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    show(t, "84 units", Some(l84), 0);
    show(t, "86 units", Some(l86), 0);
    // flags
    for (nm, f) in [
        ("ALLOW_NEUTRAL_NAMES", 0x0800_0000u32),
        ("0x1", 1),
        ("0x2", 2),
        ("0x4", 4),
        ("0x8", 8),
        ("0x10", 0x10),
        ("0x100", 0x100),
        ("USE_CP_ACP", 0x4000_0000),
        ("NOUSEROVERRIDE", 0x8000_0000),
        ("0x08000001", 0x0800_0001),
        ("0xFFFFFFFF", 0xFFFF_FFFF),
    ] {
        let mut b = Buf::<80>::new();
        let _ = write!(b, "en-US with {nm}");
        show(t, b.as_str(), Some("en-US"), f);
        let mut b = Buf::<80>::new();
        let _ = write!(b, "en with {nm}");
        show(t, b.as_str(), Some("en"), f);
    }
    // the system default name, the user default (relations only)
    let mut un = [0u16; 100];
    let mut sn = [0u16; 100];
    // SAFETY: valid buffers.
    let (ul, sl) = unsafe {
        GetUserDefaultLocaleName(un.as_mut_ptr(), 100);
        GetSystemDefaultLocaleName(sn.as_mut_ptr(), 100);
        (GetUserDefaultLCID(), GetSystemDefaultLCID())
    };
    let a = unsafe { LocaleNameToLCID(sn.as_ptr(), 0) };
    let b = unsafe { LocaleNameToLCID(un.as_ptr(), 0) };
    let c = { let x = wide("!x-sys-default-locale"); unsafe { LocaleNameToLCID(x.as_ptr(), 0) } };
    say!(t, "user's own name -> user's own LCID: {}", b == ul || b == 0x1000);
    say!(t, "system's own name -> system's own LCID: {}", a == sl || a == 0x1000);
    say!(t, "!x-sys-default-locale -> system default LCID: {}", c == sl || c == 0x800);
    let _ = (ul, sl);
}
