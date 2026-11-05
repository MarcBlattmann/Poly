//! ToUnicode with the German dead keys (^ on VK_OEM_5, acute on VK_OEM_6,
//! grave on Shift+VK_OEM_6), flags 0 (the thread's dead-key state is
//! kept between calls): a dead key returns -1 and writes the spacing
//! character; the next key combines (^ a: U+00E2), or with Space gives the
//! accent alone, or with a key that does not combine gives both characters
//! (2); the dead key twice; flags 4 (state not changed) leaves no dead key
//! pending; a key-up (scan code with 0x8000) in between.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

const VK_SHIFT: usize = 0x10;
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

const MAPVK_VK_TO_VSC: u32 = 0;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MapVirtualKeyW(code: u32, map: u32) -> u32;
    fn ToUnicode(vk: u32, scan: u32, state: *const u8, buf: *mut u16, n: i32, flags: u32) -> i32;
}

fn sh(shift: bool) -> [u8; 256] {
    let mut ks = [0u8; 256];
    if shift {
        ks[VK_SHIFT as usize] = 0x80;
    }
    ks
}

fn tu(t: &mut Test, label: &str, vk: u32, up: bool, shift: bool, flags: u32) {
    // SAFETY: plain query.
    let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) } | if up { 0x8000 } else { 0 };
    let ks = sh(shift);
    let mut out = [0u16; 8];
    // SAFETY: a 256-byte state array and an 8-unit buffer.
    let r = unsafe { ToUnicode(vk, scan, ks.as_ptr(), out.as_mut_ptr(), 8, flags) };
    let mut b = Buf::<300>::new();
    let n = if r < 0 { 1 } else { r as usize };
    for i in 0..n.min(8) {
        let _ = write!(b, " {:04x}", out[i]);
    }
    say!(t, "  {label}: {r}{}", b.as_str());
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let (circ, acute, a, e, x, space) = (0xDCu32, 0xDDu32, 0x41u32, 0x45u32, 0x58u32, 0x20u32);
    say!(t, "^ then a");
    tu(t, "^", circ, false, false, 0);
    tu(t, "a", a, false, false, 0);
    tu(t, "a again", a, false, false, 0);
    say!(t, "^ then Space");
    tu(t, "^", circ, false, false, 0);
    tu(t, "Space", space, false, false, 0);
    say!(t, "^ then x");
    tu(t, "^", circ, false, false, 0);
    tu(t, "x", x, false, false, 0);
    say!(t, "^ twice");
    tu(t, "^", circ, false, false, 0);
    tu(t, "^", circ, false, false, 0);
    tu(t, "a", a, false, false, 0);
    say!(t, "acute then e, Shift+e");
    tu(t, "acute", acute, false, false, 0);
    tu(t, "e", e, false, false, 0);
    tu(t, "acute", acute, false, false, 0);
    tu(t, "E", e, false, true, 0);
    say!(t, "grave (Shift) then Shift+a");
    tu(t, "grave", acute, false, true, 0);
    tu(t, "A", a, false, true, 0);
    say!(t, "^ with flags 4, then a with flags 0");
    tu(t, "^ (flags 4)", circ, false, false, 4);
    tu(t, "a", a, false, false, 0);
    say!(t, "^, key-up of ^, then a");
    tu(t, "^", circ, false, false, 0);
    tu(t, "^ up", circ, true, false, 0);
    tu(t, "a", a, false, false, 0);
    say!(t, "^, Shift (VK_SHIFT) down, then a");
    tu(t, "^", circ, false, false, 0);
    tu(t, "VK_SHIFT", VK_SHIFT as u32, false, true, 0);
    tu(t, "a", a, false, false, 0);
    // Nothing is left pending: a plain key gives itself.
    tu(t, "final a", a, false, false, 0);
}
