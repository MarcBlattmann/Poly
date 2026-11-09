//! ConvertStringSidToSidW: the string forms it accepts and rejects, with the resulting SID
//! (as text) or the error code.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type PSID = *mut c_void;
type PVOID = *mut c_void;
type BOOL = i32;
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

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// The last-error code after a failed call, as a plain number.
fn gle() -> u32 {
    // SAFETY: reads the calling thread's last error.
    unsafe { GetLastError() }
}

fn clear_error() {
    // SAFETY: sets the calling thread's last error.
    unsafe { SetLastError(0xDEAD) };
}

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 255 units).
fn wide(s: &str) -> [u16; 256] {
    let mut w = [0u16; 256];
    for (i, c) in s.bytes().enumerate().take(255) {
        w[i] = u16::from(c);
    }
    w
}

/// The units of a NUL-terminated UTF-16 string.
fn wide_len(p: *const u16) -> usize {
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
    }
    n
}

/// A UTF-16 string pointer as text (ASCII only; others as `?`).
fn wide_text<const N: usize>(p: *const u16) -> Buf<N> {
    let mut b = Buf::<N>::new();
    if p.is_null() {
        let _ = write!(b, "(null)");
        return b;
    }
    let n = wide_len(p);
    for i in 0..n {
        // SAFETY: i < n, inside the string.
        let c = unsafe { *p.add(i) };
        let _ = write!(b, "{}", if c < 0x80 { c as u8 as char } else { '?' });
    }
    b
}

/// The SID in the text form of `ConvertSidToStringSidW`, or an error marker.
fn sid_text(sid: *const c_void) -> Buf<200> {
    let mut out: *mut u16 = core::ptr::null_mut();
    // SAFETY: `sid` is a valid SID or the call fails; `out` is a valid out pointer.
    let ok = unsafe { ConvertSidToStringSidW(sid, &mut out) };
    if ok == 0 {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "<invalid sid>");
        return b;
    }
    let b = wide_text::<200>(out);
    // SAFETY: the string was allocated by the call with LocalAlloc.
    unsafe { LocalFree(out.cast()) };
    b
}

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn LocalFree(p: PVOID) -> PVOID;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let cases = [
        "S-1-5-18",
        "s-1-5-18",
        "S-1-5-18 ",
        " S-1-5-18",
        "S-1-5-18-",
        "S-1-5",
        "S-1",
        "S-1-",
        "S",
        "S-",
        "",
        "X-1-5-18",
        "S-2-5-18",
        "S-0-5-18",
        "S-1-0x5-18",
        "S-1-0X5-18",
        "S-1-0x500000000000-1",
        "S-1-281474976710655-1",
        "S-1-281474976710656-1",
        "S-1-4294967295-1",
        "S-1-4294967296-1",
        "S-1-5-4294967295",
        "S-1-5-4294967296",
        "S-1-5-0x12",
        "S-1-5-012",
        "S-1-5--18",
        "S-1-5-+18",
        "S-1-5-1a",
        "S-1-5-1-2-3-4-5-6-7-8-9-10-11-12-13-14-15",
        "S-1-5-1-2-3-4-5-6-7-8-9-10-11-12-13-14-15-16",
        "S-1-5-18-0",
    ];
    for s in cases {
        let w = wide(s);
        let mut sid: PSID = core::ptr::null_mut();
        clear_error();
        // SAFETY: NUL-terminated string, valid out pointer.
        let ok = unsafe { ConvertStringSidToSidW(w.as_ptr(), &mut sid) };
        let err = gle_if(ok);
        if ok != 0 {
            // SAFETY: allocated by the call.
            let (len, text) = unsafe { (GetLengthSid(sid), sid_text(sid)) };
            say!(t, "[{s}]: ok len={len} text={}", text.as_str());
            // SAFETY: allocated by the call with LocalAlloc.
            unsafe { LocalFree(sid) };
        } else {
            say!(t, "[{s}]: FAIL err={err} out_null={}", sid.is_null());
        }
    }
}
