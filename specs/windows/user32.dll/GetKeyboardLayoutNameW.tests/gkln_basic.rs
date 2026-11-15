//! GetKeyboardLayoutNameW: the active layout's name is KL_NAMELENGTH (9)
//! characters including the NUL: eight hexadecimal digits (upper case 0-9 A-F,
//! printed as a class, never the name itself). The call returns TRUE, writes
//! exactly nine characters (a sentinel after the ninth is untouched, and
//! the characters before the name are what the call wrote), the name does
//! not change between calls or with the thread, its last four digits equal
//! the low word of GetKeyboardLayout(0) (printed as a relation), the name
//! is all ASCII, and the last error is left
//! alone by a successful call. A NULL buffer is in gkln_null.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
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

fn lo(v: isize) -> i32 {
    (v as u32 & 0xFFFF) as u16 as i16 as i32
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

type HKL = *mut c_void;
const KL_NAMELENGTH: usize = 9;
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

/// The length of a wide string up to its NUL (or the slice's end).
fn wlen(buf: &[u16]) -> usize {
    buf.iter().position(|&c| c == 0).unwrap_or(buf.len())
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetKeyboardLayout(thread: u32) -> HKL;
    fn GetKeyboardLayoutNameW(buf: *mut u16) -> BOOL;
}

const SENT: u16 = 0xAAAA;

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls; the buffer has 16 characters and the call writes 9.
    unsafe {
        let mut buf = [SENT; 16];
        set_error(0xBEEF);
        let ok = GetKeyboardLayoutNameW(buf.as_mut_ptr());
        say!(t, "GetKeyboardLayoutNameW = {ok} (last error {:#x})", last_error());
        say!(t, "length of the name: {}", wlen(&buf));
        say!(t, "the ninth character is NUL: {}", (buf[KL_NAMELENGTH - 1] == 0) as i32);
        say!(t, "the characters after the ninth are untouched: {}", buf[KL_NAMELENGTH..].iter().all(|&c| c == SENT) as i32);
        say!(t, "every character is 0-9 or A-F: {}", hex_of(&buf).is_some() as i32);
        say!(t, "every character is ASCII: {}", buf[..8].iter().all(|&c| c < 0x80) as i32);
        let v = hex_of(&buf);
        let lo = GetKeyboardLayout(0) as usize & 0xFFFF;
        say!(t, "last four digits equal the low word of the layout: {}", (v.map(|x| x as usize & 0xFFFF) == Some(lo)) as i32);

        let mut again = [SENT; 16];
        GetKeyboardLayoutNameW(again.as_mut_ptr());
        say!(t, "a second call gives the same name: {}", (again == buf) as i32);

        let mut exact = [SENT; 10];
        let ok = GetKeyboardLayoutNameW(exact.as_mut_ptr());
        say!(t, "a buffer of exactly nine characters (a tenth is a sentinel): {ok}, name equal: {}, tenth untouched: {}", (exact[..9] == buf[..9]) as i32, (exact[9] == SENT) as i32);

        let mut n = 0;
        for _ in 0..32 {
            let mut b = [SENT; 16];
            GetKeyboardLayoutNameW(b.as_mut_ptr());
            n += (b == buf) as i32;
        }
        say!(t, "32 further calls all equal: {}", (n == 32) as i32);
    }
}
