//! SetKeyboardState with a NULL table: the return value and the last error
//! are printed, with a real GetKeyboardState / SetKeyboardState round trip
//! (the table is put back unchanged) before and after to show the function
//! still works. Kept in its own test so that a program that dies on a NULL
//! pointer loses nothing else.
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetKeyboardState(state: *mut u8) -> BOOL;
    fn SetKeyboardState(state: *const u8) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut orig = [0u8; 256];
    // SAFETY: orig has 256 bytes throughout.
    unsafe {
        let ok = GetKeyboardState(orig.as_mut_ptr());
        let r = SetKeyboardState(orig.as_ptr());
        say!(t, "a real round trip first: GetKeyboardState = {ok}, SetKeyboardState = {r}");
        set_error(0);
        let r = SetKeyboardState(core::ptr::null());
        say!(t, "NULL table: returns {r} (last error {})", last_error());
        let r = SetKeyboardState(orig.as_ptr());
        say!(t, "a real call afterwards: {r}");
    }
}
