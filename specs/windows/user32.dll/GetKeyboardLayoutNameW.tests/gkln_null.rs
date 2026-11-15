//! GetKeyboardLayoutNameW with a NULL buffer: the return value and the
//! last error are printed (the call is documented to write the name into the
//! buffer, so a NULL pointer is an error case that must be rejected without
//! writing). Kept in its own test so that a program that dies on it loses
//! nothing else. A real call before and after shows the function still
//! works.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

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
        let mut b = Buf::<300>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// SetLastError.
fn set_error(code: u32) {
    // SAFETY: sets this thread's last-error value.
    unsafe { SetLastError(code) }
}

/// GetLastError.
fn last_error() -> u32 {
    // SAFETY: reads this thread's last-error value.
    unsafe { GetLastError() }
}

/// The value of up to 8 hexadecimal digits (0-9, A-F upper case only) of a
/// wide string, or None when a character is none of them. The string is
/// read up to its NUL or the end of the slice.
fn hex_of(buf: &[u16]) -> Option<u32> {
    let mut v = 0u32;
    let mut n = 0;
    for &c in buf {
        if c == 0 {
            break;
        }
        let d = match c {
            0x30..=0x39 => c - 0x30,
            0x41..=0x46 => c - 0x41 + 10,
            _ => return None,
        };
        v = (v << 4) | d as u32;
        n += 1;
    }
    if n == 0 || n > 8 { None } else { Some(v) }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetKeyboardLayoutNameW(buf: *mut u16) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls; the real buffer has 16 characters.
    unsafe {
        let mut buf = [0u16; 16];
        let ok = GetKeyboardLayoutNameW(buf.as_mut_ptr());
        say!(t, "a real call first: {ok}, a hexadecimal name: {}", hex_of(&buf).is_some() as i32);
        set_error(0);
        let r = GetKeyboardLayoutNameW(core::ptr::null_mut());
        say!(t, "NULL buffer: returns {r} (last error {})", last_error());
        let mut after = [0u16; 16];
        let ok = GetKeyboardLayoutNameW(after.as_mut_ptr());
        say!(t, "a real call afterwards: {ok}, the same name: {}", (after == buf) as i32);
    }
}
