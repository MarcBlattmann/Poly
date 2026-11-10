//! GetSystemMetricsForDpi between and beyond the usual scale factors: the
//! metrics that scale, at 22 DPI values from 12 to 800, to show how each one
//! is rounded, which have a minimum, which follow the caption font or the
//! theme at small sizes, and which do not scale at all.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

const SM_CYCAPTION: i32 = 4;
const SM_CXBORDER: i32 = 5;
const SM_CXDLGFRAME: i32 = 7;
const SM_CXFRAME: i32 = 32;
const SM_CYFRAME: i32 = 33;
const SM_CXMIN: i32 = 28;
const SM_CYMIN: i32 = 29;
const SM_CXMINTRACK: i32 = 34;
const SM_CYMINTRACK: i32 = 35;
const SM_CXPADDEDBORDER: i32 = 92;
const SM_CXEDGE: i32 = 45;
const SM_CYMENU: i32 = 15;
const SM_CXVSCROLL: i32 = 2;
const SM_CYHSCROLL: i32 = 3;
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

const SM_CXCURSOR: i32 = 13;
const SM_CYCURSOR: i32 = 14;
const SM_CXICON: i32 = 11;
const SM_CYICON: i32 = 12;
const SM_CXSMICON: i32 = 49;
const SM_CYSMICON: i32 = 50;
const SM_CXHSCROLL: i32 = 21;
const SM_CYVSCROLL: i32 = 20;
const SM_CXHTHUMB: i32 = 10;
const SM_CYVTHUMB: i32 = 9;
const SM_CXSIZE: i32 = 30;
const SM_CYSIZE: i32 = 31;
const SM_CXSMSIZE: i32 = 52;
const SM_CYSMSIZE: i32 = 53;
const SM_CYSMCAPTION: i32 = 51;
const SM_CXMENUSIZE: i32 = 54;
const SM_CYMENUSIZE: i32 = 55;
const SM_CXMENUCHECK: i32 = 71;
const SM_CYMENUCHECK: i32 = 72;
const SM_CXICONSPACING: i32 = 38;
const SM_CYICONSPACING: i32 = 39;
const SM_CXFOCUSBORDER: i32 = 83;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // winuser.h index numbers.
    let metrics: [(&str, i32); 36] = [
        ("SM_CXVSCROLL", 2),
        ("SM_CYHSCROLL", 3),
        ("SM_CYVSCROLL", 20),
        ("SM_CXHSCROLL", 21),
        ("SM_CYVTHUMB", 9),
        ("SM_CXHTHUMB", 10),
        ("SM_CYCAPTION", 4),
        ("SM_CYSMCAPTION", 51),
        ("SM_CYMENU", 15),
        ("SM_CXICON", 11),
        ("SM_CYICON", 12),
        ("SM_CXSMICON", 49),
        ("SM_CYSMICON", 50),
        ("SM_CXCURSOR", 13),
        ("SM_CYCURSOR", 14),
        ("SM_CXSIZE", 30),
        ("SM_CYSIZE", 31),
        ("SM_CXSMSIZE", 52),
        ("SM_CYSMSIZE", 53),
        ("SM_CXMENUSIZE", 54),
        ("SM_CYMENUSIZE", 55),
        ("SM_CXMENUCHECK", 71),
        ("SM_CYMENUCHECK", 72),
        ("SM_CXFRAME", 32),
        ("SM_CYFRAME", 33),
        ("SM_CXPADDEDBORDER", 92),
        ("SM_CXMIN", 28),
        ("SM_CYMIN", 29),
        ("SM_CXMINTRACK", 34),
        ("SM_CYMINTRACK", 35),
        ("SM_CXICONSPACING", 38),
        ("SM_CYICONSPACING", 39),
        ("SM_CXBORDER", 5),
        ("SM_CXEDGE", 45),
        ("SM_CXDLGFRAME", 7),
        ("SM_CXFOCUSBORDER", 83),
    ];
    let dpis: [u32; 22] = [12, 36, 60, 84, 97, 108, 115, 130, 150, 156, 175, 180, 200, 225, 250, 300, 350, 400, 450, 500, 600, 800];
    for (name, idx) in metrics {
        let mut b = Buf::<300>::new();
        let _ = write!(b, "{name}:");
        for dpi in dpis {
            // SAFETY: plain query.
            let v = unsafe { GetSystemMetricsForDpi(idx, dpi) };
            let _ = write!(b, " {dpi}={v}");
        }
        t.line(b.as_str());
    }
}
