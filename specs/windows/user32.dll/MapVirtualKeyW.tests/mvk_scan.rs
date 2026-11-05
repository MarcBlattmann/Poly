//! MapVirtualKeyW from scan codes on the German layout: MAPVK_VSC_TO_VK (1)
//! and MAPVK_VSC_TO_VK_EX (3) for every scan code 0x00..0x7F, then for the
//! extended codes 0xE0nn (and 0xE1nn) of the arrows, Insert/Delete/Home/End/
//! Page keys, right Ctrl/Alt, the Windows keys, numpad Enter and divide
//! (only _EX knows the extended prefix), and the same with only the low
//! byte given. Lines of eight: "scan: vk/vk_ex".
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
        let mut b = Buf::<300>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const MAPVK_VSC_TO_VK: u32 = 1;
const MAPVK_VSC_TO_VK_EX: u32 = 3;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MapVirtualKeyW(code: u32, map: u32) -> u32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut sc = 0u32;
    while sc < 0x80 {
        let mut b = Buf::<300>::new();
        let _ = write!(b, "{sc:#04x}:");
        for i in 0..8 {
            // SAFETY: plain queries.
            let (v, x) =
                unsafe { (MapVirtualKeyW(sc + i, MAPVK_VSC_TO_VK), MapVirtualKeyW(sc + i, MAPVK_VSC_TO_VK_EX)) };
            let _ = write!(b, " {v:02x}/{x:02x}");
        }
        t.line(b.as_str());
        sc += 8;
    }
    let ext: [u32; 22] = [
        0xE01C, 0xE01D, 0xE035, 0xE037, 0xE038, 0xE047, 0xE048, 0xE049, 0xE04B, 0xE04D, 0xE04F, 0xE050, 0xE051, 0xE052,
        0xE053, 0xE05B, 0xE05C, 0xE05D, 0xE11D, 0xE02A, 0xE046, 0xE0FF,
    ];
    for e in ext {
        // SAFETY: plain queries.
        let (v, x) = unsafe { (MapVirtualKeyW(e, MAPVK_VSC_TO_VK), MapVirtualKeyW(e, MAPVK_VSC_TO_VK_EX)) };
        say!(t, "{e:#06x}: vk={v:#04x} vk_ex={x:#04x}");
    }
}
