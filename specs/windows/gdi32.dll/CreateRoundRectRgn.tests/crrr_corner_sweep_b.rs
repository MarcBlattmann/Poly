//! A 47x33 box with many corner sizes.
//! The rows of round-rectangle regions: each rectangle of GetRegionData as
//! top+rows:left-right (right exclusive), relative to the origin; the
//! line starts with the region type (GetRgnBox) and the rectangle count.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers wingdi.h, windef.h and winerror.h (constants,
// layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HGDIOBJ = *mut c_void;
type HRGN = *mut c_void;
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

/// RGNDATAHEADER (wingdi.h) followed by room for the rectangles.
#[repr(C)]
struct RgnData {
    size: u32,
    kind: u32,
    count: u32,
    rgn_size: u32,
    bound: RECT,
    rects: [RECT; 1200],
}

const RGN_AND: i32 = 1;
const RGN_OR: i32 = 2;
const RGN_XOR: i32 = 3;
const RGN_DIFF: i32 = 4;

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
    fn clear(&mut self) {
        self.n = 0;
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn CreateEllipticRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn CreateEllipticRgnIndirect(rect: *const RECT) -> HRGN;
    fn CreateRoundRectRgn(l: i32, t: i32, r: i32, b: i32, w: i32, h: i32) -> HRGN;
    fn GetRegionData(rgn: HRGN, n: u32, data: *mut RgnData) -> u32;
    fn GetRgnBox(rgn: HRGN, rect: *mut RECT) -> i32;
    fn CombineRgn(dst: HRGN, a: HRGN, b: HRGN, mode: i32) -> i32;
    fn EqualRgn(a: HRGN, b: HRGN) -> BOOL;
    fn PtInRegion(rgn: HRGN, x: i32, y: i32) -> BOOL;
}

/// The region data (a static buffer: the rectangle list can be long).
static mut DATA: RgnData = RgnData {
    size: 0,
    kind: 0,
    count: 0,
    rgn_size: 0,
    bound: RECT { left: 0, top: 0, right: 0, bottom: 0 },
    rects: [RECT { left: 0, top: 0, right: 0, bottom: 0 }; 1200],
};

/// Print `label` then the rectangles of `rgn` relative to (ox, oy), each as
/// "top+rows:left-right" (right exclusive), wrapped into lines of at most
/// about 200 characters (continuation lines start with "  ..").
/// The region type (GetRgnBox) and the rectangle count come first.
fn show(t: &mut Test, label: &str, rgn: HRGN, ox: i32, oy: i32) {
    // SAFETY: rgn is a region of ours; DATA is only used by this thread.
    unsafe {
        let mut bx = RECT::default();
        let ty = GetRgnBox(rgn, &mut bx);
        let d = &raw mut DATA;
        let n = GetRegionData(rgn, core::mem::size_of::<RgnData>() as u32, d);
        let count = if n == 0 { 0 } else { (*d).count as usize };
        let mut line = Buf::<240>::new();
        let _ = write!(line, "{label} t={ty} n={count}:");
        for i in 0..count.min(1200) {
            let r = (*d).rects[i];
            let mut item = Buf::<40>::new();
            let _ = write!(item, " {}+{}:{}-{}", r.top - oy, r.bottom - r.top, r.left - ox, r.right - ox);
            if line.n + item.n > 200 {
                t.line(line.as_str());
                line.clear();
                let _ = write!(line, "  ..");
            }
            let _ = write!(line, "{}", item.as_str());
        }
        t.line(line.as_str());
    }
}

/// The round rectangles (w, h, corner width, corner height) of this test, all at the origin.
const SIZES: &[(i32, i32, i32, i32)] = &[
    (47, 33, 1, 1), (47, 33, 1, 3), (47, 33, 1, 7), (47, 33, 1, 12), (47, 33, 1, 32),
    (47, 33, 1, 33), (47, 33, 4, 1), (47, 33, 4, 3), (47, 33, 4, 7), (47, 33, 4, 12),
    (47, 33, 4, 32), (47, 33, 4, 33), (47, 33, 6, 1), (47, 33, 6, 3), (47, 33, 6, 7),
    (47, 33, 6, 12), (47, 33, 6, 32), (47, 33, 6, 33), (47, 33, 10, 1), (47, 33, 10, 3),
    (47, 33, 10, 7), (47, 33, 10, 12), (47, 33, 10, 32), (47, 33, 10, 33), (47, 33, 17, 1),
    (47, 33, 17, 3), (47, 33, 17, 7), (47, 33, 17, 12), (47, 33, 17, 32), (47, 33, 17, 33),
    (47, 33, 26, 1), (47, 33, 26, 3), (47, 33, 26, 7), (47, 33, 26, 12), (47, 33, 26, 32),
    (47, 33, 26, 33), (47, 33, 46, 1), (47, 33, 46, 3), (47, 33, 46, 7), (47, 33, 46, 12),
    (47, 33, 46, 32), (47, 33, 46, 33), (47, 33, 47, 1), (47, 33, 47, 3), (47, 33, 47, 7),
    (47, 33, 47, 12), (47, 33, 47, 32), (47, 33, 47, 33), (47, 33, 60, 1), (47, 33, 60, 3),
    (47, 33, 60, 7), (47, 33, 60, 12), (47, 33, 60, 32), (47, 33, 60, 33),
];

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for &(w, h, cw, ch) in SIZES {
        // SAFETY: a region of ours, deleted after use.
        unsafe {
            let rgn = CreateRoundRectRgn(0, 0, w, h, cw, ch);
            let mut label = Buf::<48>::new();
            let _ = write!(label, "r {w}x{h} c{cw}x{ch}");
            show(t, label.as_str(), rgn, 0, 0);
            DeleteObject(rgn);
        }
    }
}
