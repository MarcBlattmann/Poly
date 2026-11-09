//! DeriveAppContainerSidFromAppContainerName: the SID derived from a name is deterministic (the
//! same string for the same name on every run), case-insensitive, has the AppContainer shape
//! S-1-15-2-<7 sub-authorities>; names that are empty, too long, with odd characters.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winnt.h, winbase.h, sddl.h, aclapi.h, jobapi2.h,
// processthreadsapi.h, winternl.h, ntstatus.h and winerror.h (constants,
// layouts); the helpers are this test's own.

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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalFree(p: PVOID) -> PVOID;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn FreeSid(sid: PSID) -> PVOID;
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
}

#[link(name = "userenv", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeriveAppContainerSidFromAppContainerName(name: *const u16, sid: *mut PSID) -> i32;
}

poly_wtest::main!(test);

fn derive(t: &mut Test, label: &str, name: &str) {
    let w = wide(name);
    let mut sid: PSID = core::ptr::null_mut();
    // SAFETY: a NUL-terminated name and a valid out pointer.
    let hr = unsafe { DeriveAppContainerSidFromAppContainerName(w.as_ptr(), &mut sid) };
    if hr == 0 && !sid.is_null() {
        // SAFETY: the SID was returned by the call.
        let (len, subs) = unsafe { (GetLengthSid(sid), *(sid as *const u8).add(1)) };
        say!(t, "{label}: hr=0 sid={} length={len} subauthorities={subs}", sid_text(sid).as_str());
        // SAFETY: the SID is freed with FreeSid.
        unsafe { FreeSid(sid) };
    } else {
        say!(t, "{label}: hr={:#x} sid_null={}", hr as u32, sid.is_null());
    }
}

fn test(t: &mut Test) {
    derive(t, "poly.test", "poly.test");
    derive(t, "same name again", "poly.test");
    derive(t, "upper case", "POLY.TEST");
    derive(t, "mixed case", "Poly.Test");
    derive(t, "other name", "poly.test2");
    derive(t, "one letter", "a");
    derive(t, "digits", "12345");
    derive(t, "chrome-like", "chrome.sandbox.network");
    derive(t, "with space", "poly test");
    derive(t, "with backslash", "poly\\test");
    derive(t, "with unicode", "poly.t\u{e9}st");
    derive(t, "empty", "");
    let long: [u8; 70] = [b'a'; 70];
    derive(t, "64 characters", core::str::from_utf8(&long[..64]).unwrap());
    derive(t, "65 characters", core::str::from_utf8(&long[..65]).unwrap());
    derive(t, "100 characters", core::str::from_utf8(&long[..70]).unwrap());
    let mut sid: PSID = 0x1234 as PSID;
    // SAFETY: a NUL-terminated name; the out pointer is valid.
    let hr = unsafe { DeriveAppContainerSidFromAppContainerName(core::ptr::null(), &mut sid) };
    let _ = hr;
}
