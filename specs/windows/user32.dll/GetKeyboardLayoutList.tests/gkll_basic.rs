//! GetKeyboardLayoutList (4 calls in a Chrome run): the number of
//! installed layouts is returned for a buffer size of 0 (the count itself is
//! machine dependent: only "at least 1" is printed); with a large buffer
//! the call fills exactly that many entries and returns the count, and
//! leaves the rest of the buffer alone (a sentinel is checked); entries are
//! not zero and pairwise different; the list contains the thread's layout
//! (GetKeyboardLayout(0)); a second call gives the same list; buffers of 1 and
//! 2 entries return min(count, n) and fill only those; a size of 0 with a
//! buffer writes nothing; a negative size gives what? (return value and last
//! error printed). A NULL buffer is in gkll_null.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

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

type HKL = *mut c_void;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetKeyboardLayout(thread: u32) -> HKL;
    fn GetKeyboardLayoutList(n: i32, list: *mut HKL) -> i32;
}

const SENT: usize = 0xAAAA_BBBB_CCCC_DDDD;

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let sentinel = SENT as HKL;
    // SAFETY: plain calls; the buffers have the sizes passed.
    unsafe {
        set_error(0xBEEF);
        let count = GetKeyboardLayoutList(0, core::ptr::null_mut());
        say!(t, "size 0, no buffer: at least 1: {} (last error {:#x})", (count >= 1) as i32, last_error());

        let mut list = [sentinel; 64];
        let n = GetKeyboardLayoutList(64, list.as_mut_ptr());
        say!(t, "size 64: returns the count: {}", (n == count) as i32);
        let filled = n.clamp(0, 64) as usize;
        say!(t, "  the filled entries are not zero: {}", list[..filled].iter().all(|h| !h.is_null()) as i32);
        say!(t, "  the rest of the buffer is untouched: {}", list[filled..].iter().all(|&h| h == sentinel) as i32);
        let me = GetKeyboardLayout(0);
        say!(t, "  the list contains the thread's layout: {}", list[..filled].contains(&me) as i32);
        let mut distinct = true;
        for i in 0..filled {
            for j in i + 1..filled {
                if list[i] == list[j] {
                    distinct = false;
                }
            }
        }
        say!(t, "  entries are pairwise different: {}", distinct as i32);
        say!(t, "  every entry has a language identifier: {}", list[..filled].iter().all(|&h| (h as usize & 0xFFFF) != 0) as i32);

        let mut again = [sentinel; 64];
        let n2 = GetKeyboardLayoutList(64, again.as_mut_ptr());
        say!(t, "  a second call returns the same count and list: {}", (n2 == n && again == list) as i32);

        say!(t, "== small buffers");
        for size in [1i32, 2] {
            let mut small = [sentinel; 4];
            let r = GetKeyboardLayoutList(size, small.as_mut_ptr());
            let want = count.min(size);
            say!(t, "  size {size}: returns min(count, {size}): {}", (r == want) as i32);
            say!(t, "    the entries are the first of the full list: {}", (small[..want as usize] == list[..want as usize]) as i32);
            say!(t, "    the entries beyond are untouched: {}", small[want as usize..].iter().all(|&h| h == sentinel) as i32);
        }

        say!(t, "== size 0 with a buffer");
        let mut zero = [sentinel; 4];
        let r = GetKeyboardLayoutList(0, zero.as_mut_ptr());
        say!(t, "  returns the count: {}, buffer untouched: {}", (r == count) as i32, zero.iter().all(|&h| h == sentinel) as i32);

        say!(t, "== negative size");
        let mut neg = [sentinel; 4];
        set_error(0);
        let r = GetKeyboardLayoutList(-1, neg.as_mut_ptr());
        say!(t, "  size -1: returns {r}, buffer untouched: {}, last error {}", neg.iter().all(|&h| h == sentinel) as i32, last_error());
    }
}
