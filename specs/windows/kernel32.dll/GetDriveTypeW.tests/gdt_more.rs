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
// GetDriveTypeW: agreement with the volume calls, return value constants, long strings.

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn GetLogicalDriveStringsW(len: u32, buf: *mut u16) -> u32;
    fn GetVolumeInformationW(root: *const u16, name: *mut u16, nlen: u32, serial: *mut u32, maxc: *mut u32, flags: *mut u32, fs: *mut u16, fslen: u32) -> BOOL;
    fn GetDiskFreeSpaceExW(dir: *const u16, a: *mut u64, b: *mut u64, c: *mut u64) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // every logical drive string (C:\, D:\ ...) answers a type that is not 0 or 1
    let mut buf = [0u16; 400];
    // SAFETY: valid buffer.
    let n = unsafe { GetLogicalDriveStringsW(400, buf.as_mut_ptr()) };
    say!(t, "GetLogicalDriveStringsW fits, length multiple of 4: {} {}", n > 0 && n < 400, n % 4 == 0);
    let mut i = 0usize;
    let mut all_valid = true;
    let mut c_seen = false;
    while i + 3 < n as usize {
        let r = &buf[i..i + 4];
        // SAFETY: NUL-terminated by the API.
        let ty = unsafe { GetDriveTypeW(r.as_ptr()) };
        if ty < 2 {
            all_valid = false;
        }
        if r[0] == 0x43 {
            c_seen = true;
        }
        i += 4;
    }
    say!(t, "every listed drive has a type of 2 or more: {all_valid}; C: listed: {c_seen}");
    // the system drive is a fixed disk (3), has a file system, and answers GetDiskFreeSpaceEx
    let c = wide("C:\\");
    // SAFETY: valid string.
    let ty = unsafe { GetDriveTypeW(c.as_ptr()) };
    say!(t, "C:\\ is DRIVE_FIXED: {}", ty == 3);
    let mut fs = [0u16; 40];
    let mut flags = 0u32;
    // SAFETY: valid buffers.
    let ok = unsafe { GetVolumeInformationW(c.as_ptr(), core::ptr::null_mut(), 0, core::ptr::null_mut(), core::ptr::null_mut(), &mut flags, fs.as_mut_ptr(), 40) };
    let fsn = wstr_at(fs.as_ptr(), 40);
    say!(t, "GetVolumeInformationW ok={ok} fs={}", units_text(fsn).as_str());
    let (mut a, mut b, mut cc) = (0u64, 0u64, 0u64);
    // SAFETY: valid pointers.
    let ok = unsafe { GetDiskFreeSpaceExW(c.as_ptr(), &mut a, &mut b, &mut cc) };
    say!(t, "GetDiskFreeSpaceExW ok={ok} total>free: {}", b > cc);
    // SAFETY: valid string.
    let again = unsafe { GetDriveTypeW(c.as_ptr()) };
    say!(t, "repeatable: {}", again == ty);
    // a very long name
    let mut long = [0x41u16; 300];
    long[299] = 0;
    clear_error();
    // SAFETY: NUL-terminated.
    let r = unsafe { GetDriveTypeW(long.as_ptr()) };
    say!(t, "299 letters => {r}");
    let mut longer = [0x61u16; 5000];
    longer[0] = 0x43;
    longer[1] = 0x3a;
    longer[2] = 0x5c;
    longer[4999] = 0;
    // SAFETY: NUL-terminated.
    let r = unsafe { GetDriveTypeW(longer.as_ptr()) };
    say!(t, "C:\\ plus 4996 letters => {r}");
}
