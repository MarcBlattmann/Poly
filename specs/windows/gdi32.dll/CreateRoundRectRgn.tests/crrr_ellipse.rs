//! When is a round rectangle the same region as the ellipse of its box?
//! EqualRgn and the type and box of the XOR of the two.
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

/// (w, h, corner width, corner height): is the round rectangle the same region as the ellipse of the box?
const CASES: &[(i32, i32, i32, i32)] = &[
    (20, 20, 20, 20), (20, 20, 19, 19), (20, 20, 21, 21), (20, 20, 200, 200),
    (20, 20, -20, -20), (31, 17, 31, 17), (31, 17, 30, 16), (31, 17, 32, 18),
    (10, 40, 10, 400), (10, 40, 9, 40), (45, 45, 45, 44), (7, 7, 7, 7), (3, 3, 3, 3),
];

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for &(w, h, cw, ch) in CASES {
        // SAFETY: regions of ours, deleted after use.
        unsafe {
            let e = CreateEllipticRgn(0, 0, w, h);
            let r = CreateRoundRectRgn(0, 0, w, h, cw, ch);
            let x = CreateRectRgn(0, 0, 0, 0);
            let kind = CombineRgn(x, r, e, RGN_XOR);
            let mut bx = RECT::default();
            GetRgnBox(x, &mut bx);
            say!(
                t,
                "{w}x{h} corner {cw}x{ch}: equal to the ellipse {}, xor type {kind} box {},{},{},{}",
                EqualRgn(e, r),
                bx.left,
                bx.top,
                bx.right,
                bx.bottom
            );
            DeleteObject(e);
            DeleteObject(r);
            DeleteObject(x);
        }
    }
}
