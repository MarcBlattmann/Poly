//! GetSystemMetrics, the size indices of window parts in the interactive
//! session (DPI-unaware process at 96 DPI): borders, frames, caption, scroll
//! bars and thumbs, icons and small icons, cursors, menus and menu check
//! marks, caption buttons, minimum, minimized and spacing sizes, edges,
//! focus borders, the double-click and drag rectangles. Each printed with
//! GetSystemMetricsForDpi at 96, 120, 144 and 192 DPI.
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
const SM_CYBORDER: i32 = 6;
const SM_CXDLGFRAME: i32 = 7;
const SM_CYDLGFRAME: i32 = 8;
const SM_CXFRAME: i32 = 32;
const SM_CYFRAME: i32 = 33;
const SM_CXMIN: i32 = 28;
const SM_CYMIN: i32 = 29;
const SM_CXPADDEDBORDER: i32 = 92;
const SM_CXEDGE: i32 = 45;
const SM_CYEDGE: i32 = 46;
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

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<300>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const SM_CXCURSOR: i32 = 13;
const SM_CYCURSOR: i32 = 14;
const SM_CXICON: i32 = 11;
const SM_CYICON: i32 = 12;
const SM_CXSMICON: i32 = 49;
const SM_CYSMICON: i32 = 50;
const SM_CXDOUBLECLK: i32 = 36;
const SM_CYDOUBLECLK: i32 = 37;
const SM_CXDRAG: i32 = 68;
const SM_CYDRAG: i32 = 69;
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
const SM_CXSIZEFRAME: i32 = 32;
const SM_CXFOCUSBORDER: i32 = 83;
const SM_CYFOCUSBORDER: i32 = 84;
const SM_CXMINIMIZED: i32 = 57;
const SM_CYMINIMIZED: i32 = 58;
const SM_CXMINSPACING: i32 = 47;
const SM_CYMINSPACING: i32 = 48;
const SM_CYKANJIWINDOW: i32 = 18;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetDoubleClickTime() -> u32;
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
}

const GSM_FRAMES: [(&str, i32); 46] = [
    ("SM_CXVSCROLL", 2),
    ("SM_CYHSCROLL", 3),
    ("SM_CYCAPTION", 4),
    ("SM_CXBORDER", 5),
    ("SM_CYBORDER", 6),
    ("SM_CXDLGFRAME", 7),
    ("SM_CYDLGFRAME", 8),
    ("SM_CYVTHUMB", 9),
    ("SM_CXHTHUMB", 10),
    ("SM_CXICON", 11),
    ("SM_CYICON", 12),
    ("SM_CXCURSOR", 13),
    ("SM_CYCURSOR", 14),
    ("SM_CYMENU", 15),
    ("SM_CYKANJIWINDOW", 18),
    ("SM_CYVSCROLL", 20),
    ("SM_CXHSCROLL", 21),
    ("SM_CXMIN", 28),
    ("SM_CYMIN", 29),
    ("SM_CXSIZE", 30),
    ("SM_CYSIZE", 31),
    ("SM_CXFRAME", 32),
    ("SM_CYFRAME", 33),
    ("SM_CXDOUBLECLK", 36),
    ("SM_CYDOUBLECLK", 37),
    ("SM_CXICONSPACING", 38),
    ("SM_CYICONSPACING", 39),
    ("SM_CXEDGE", 45),
    ("SM_CYEDGE", 46),
    ("SM_CXMINSPACING", 47),
    ("SM_CYMINSPACING", 48),
    ("SM_CXSMICON", 49),
    ("SM_CYSMICON", 50),
    ("SM_CYSMCAPTION", 51),
    ("SM_CXSMSIZE", 52),
    ("SM_CYSMSIZE", 53),
    ("SM_CXMENUSIZE", 54),
    ("SM_CYMENUSIZE", 55),
    ("SM_CXMINIMIZED", 57),
    ("SM_CYMINIMIZED", 58),
    ("SM_CXDRAG", 68),
    ("SM_CYDRAG", 69),
    ("SM_CXMENUCHECK", 71),
    ("SM_CYMENUCHECK", 72),
    ("SM_CXFOCUSBORDER", 83),
    ("SM_CXPADDEDBORDER", 92),
];

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for &(name, index) in GSM_FRAMES.iter() {
        // SAFETY: plain query.
        let v = unsafe { GetSystemMetrics(index) };
        let mut b = Buf::<300>::new();
        let _ = write!(b, "{name} ({index}) = {v}; ForDpi");
        for dpi in [96u32, 120, 144, 192] {
            // SAFETY: plain query.
            let d = unsafe { GetSystemMetricsForDpi(index, dpi) };
            let _ = write!(b, " {dpi}={d}");
        }
        t.line(b.as_str());
    }
    // SAFETY: plain queries.
    unsafe {
        say!(t, "SM_CYFOCUSBORDER (84) = {}", GetSystemMetrics(84));
        t.expect(
            "SM_CXFRAME == SM_CXSIZEFRAME (same index 32)",
            GetSystemMetrics(32) as i64,
            GetSystemMetrics(SM_CXSIZEFRAME) as i64,
        );
        say!(t, "GetDoubleClickTime = {}", GetDoubleClickTime());
    }
}
