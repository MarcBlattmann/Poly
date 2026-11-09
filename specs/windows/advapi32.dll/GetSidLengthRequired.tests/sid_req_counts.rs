//! GetSidLengthRequired: 8 + 4 * count for 0 to 15 sub-authorities, and the same formula (no
//! limit check) for larger counts up to 255.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

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
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSidLengthRequired(count: u8) -> u32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut line = Buf::<300>::new();
    for count in 0..=15u8 {
        // SAFETY: plain computation.
        let n = unsafe { GetSidLengthRequired(count) };
        let _ = write!(line, "{n} ");
    }
    t.line(line.as_str());
    for count in [16u8, 17, 63, 64, 100, 128, 255] {
        // SAFETY: plain computation.
        let n = unsafe { GetSidLengthRequired(count) };
        say!(t, "count {count}: {n}");
    }
}
