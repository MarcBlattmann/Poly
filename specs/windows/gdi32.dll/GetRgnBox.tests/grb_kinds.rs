//! GetRgnBox's return value for an empty region (NULLREGION 1), a
//! rectangle (SIMPLEREGION 2) and a region of several rectangles
//! (COMPLEXREGION 3, the box being their bounding rectangle), what it
//! writes for an empty region (all zero, whatever the empty rectangle was),
//! and that its kind agrees with CombineRgn's.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HGDIOBJ = *mut c_void;
type BOOL = i32;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

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

type HRGN = *mut c_void;
const RGN_AND: i32 = 1;
const RGN_OR: i32 = 2;
const RGN_XOR: i32 = 3;
const RGN_DIFF: i32 = 4;
/// "l,t,r,b" of a rectangle.
struct R(RECT);

impl core::fmt::Display for R {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{},{},{},{}", self.0.left, self.0.top, self.0.right, self.0.bottom)
    }
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn CombineRgn(dst: HRGN, a: HRGN, b: HRGN, mode: i32) -> i32;
    fn GetRgnBox(rgn: HRGN, rect: *mut RECT) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: regions of ours, deleted after use.
    unsafe {
        for (label, l, tp, r, b) in
            [("empty at 5,5", 5, 5, 5, 5), ("empty 10..20 x", 10, 7, 20, 7), ("rect", 3, 4, 30, 40)]
        {
            let rgn = CreateRectRgn(l, tp, r, b);
            let mut bx = RECT { left: -9, top: -9, right: -9, bottom: -9 };
            let k = GetRgnBox(rgn, &mut bx);
            say!(t, "{label}: kind {k} box {}", R(bx));
            DeleteObject(rgn);
        }
        let a = CreateRectRgn(0, 0, 10, 10);
        let b = CreateRectRgn(20, 30, 25, 50);
        let d = CreateRectRgn(0, 0, 0, 0);
        let kc = CombineRgn(d, a, b, RGN_OR);
        let mut bx = RECT::default();
        let k = GetRgnBox(d, &mut bx);
        say!(t, "two rects: CombineRgn {kc} GetRgnBox {k} box {}", R(bx));
        let kc = CombineRgn(d, a, b, RGN_AND);
        let k = GetRgnBox(d, &mut bx);
        say!(t, "their intersection: CombineRgn {kc} GetRgnBox {k} box {}", R(bx));
        let c = CreateRectRgn(0, 0, 10, 20);
        let kc = CombineRgn(d, a, c, RGN_XOR);
        let k = GetRgnBox(d, &mut bx);
        say!(t, "XOR leaving one rect: CombineRgn {kc} GetRgnBox {k} box {}", R(bx));
        let kc = CombineRgn(d, c, a, RGN_DIFF);
        let k = GetRgnBox(d, &mut bx);
        say!(t, "DIFF leaving one rect: CombineRgn {kc} GetRgnBox {k} box {}", R(bx));
        DeleteObject(a);
        DeleteObject(b);
        DeleteObject(c);
        DeleteObject(d);
    }
}
