//! ToUnicode's buffer and arguments: buffer sizes 0 and 1 for a key giving
//! one character and for a dead-key combination giving two (^ then x);
//! a NULL state array; vk 0 and 0xFF; a key-up (scan | 0x8000) of 'a';
//! Ctrl+[ (0x1B) and Ctrl+Space; the number of units written and whether
//! a NUL follows them.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

const VK_CONTROL: usize = 0x11;
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

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ToUnicode(vk: u32, scan: u32, state: *const u8, buf: *mut u16, n: i32, flags: u32) -> i32;
}

fn ks() -> [u8; 256] {
    [0u8; 256]
}

fn call(t: &mut Test, label: &str, vk: u32, scan: u32, state: &[u8; 256], n: i32, flags: u32) {
    let mut out = [0xAAAAu16; 8];
    // SAFETY: a 256-byte state array and an 8-unit buffer (n <= 8).
    let r = unsafe { ToUnicode(vk, scan, state.as_ptr(), out.as_mut_ptr(), n, flags) };
    say!(t, "{label}: {r}, buffer {:04x} {:04x} {:04x} {:04x}", out[0], out[1], out[2], out[3]);
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let s = ks();
    call(t, "a, size 8", 0x41, 0x1E, &s, 8, 0);
    call(t, "a, size 1", 0x41, 0x1E, &s, 1, 0);
    call(t, "a, size 0", 0x41, 0x1E, &s, 0, 0);
    call(t, "a key-up, size 8", 0x41, 0x1E | 0x8000, &s, 8, 0);
    call(t, "^, size 8", 0xDC, 0x29, &s, 8, 0);
    call(t, "then x, size 1", 0x58, 0x2D, &s, 1, 0);
    call(t, "then x again, size 8", 0x58, 0x2D, &s, 8, 0);
    call(t, "^, size 0", 0xDC, 0x29, &s, 0, 0);
    call(t, "then x, size 8", 0x58, 0x2D, &s, 8, 0);
    call(t, "vk 0", 0, 0, &s, 8, 0);
    call(t, "vk 0xff", 0xFF, 0, &s, 8, 0);
    call(t, "a with scan 0", 0x41, 0, &s, 8, 0);
    let mut c = ks();
    c[VK_CONTROL as usize] = 0x80;
    call(t, "Ctrl+ue key (0xba)", 0xBA, 0x1A, &c, 8, 0);
    call(t, "Ctrl+Space", 0x20, 0x39, &c, 8, 0);
    call(t, "Ctrl+2", 0x32, 0x03, &c, 8, 0);
    call(t, "Ctrl+Back", 0x08, 0x0E, &c, 8, 0);
    call(t, "Ctrl+Enter", 0x0D, 0x1C, &c, 8, 0);
    let mut out = [0u16; 8];
    // SAFETY: NULL state array is the case observed; the buffer is writable.
    let r = unsafe { ToUnicode(0x41, 0x1E, core::ptr::null(), out.as_mut_ptr(), 8, 0) };
    say!(t, "NULL state array: {r}");
    // Nothing pending afterwards.
    call(t, "final a", 0x41, 0x1E, &s, 8, 0);
}
