//! CombineRgn RGN_OR/AND/DIFF/XOR on rectangles that touch side by side,
//! touch top to bottom, are disjoint, nested, identical, offset by one
//! band, or empty; and a union built from many rectangles (a staircase, a
//! frame, a cross): the return kind and the band decomposition, which shows
//! when touching rectangles are merged into one.
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

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// The pixel at (x, y) of a top-down 32-bit DIB of width w.
fn px(bits: *mut u32, w: i32, x: i32, y: i32) -> u32 {
    // SAFETY: callers stay inside the DIB.
    unsafe { *bits.add((y * w + x) as usize) }
}

/// The rows of a small top-down 32-bit DIB as text: one character per
/// pixel from `key` (pixel value, char), '?' for others.
fn rows(t: &mut Test, bits: *mut u32, w: i32, h: i32, key: &[(u32, char)]) {
    for y in 0..h {
        let mut b = Buf::<300>::new();
        let _ = write!(b, "  |");
        for x in 0..w {
            let v = px(bits, w, x, y);
            let c = key.iter().find(|k| k.0 == v).map(|k| k.1).unwrap_or('?');
            let _ = b.write_char(c);
        }
        let _ = b.write_str("|");
        t.line(b.as_str());
    }
}

type HRGN = *mut c_void;
/// wingdi.h: RGNDATAHEADER followed by up to 64 rectangles.
#[repr(C)]
struct RGNDATA64 {
    size: u32,
    kind: u32,
    count: u32,
    rgn_size: u32,
    bound: RECT,
    rects: [RECT; 64],
}

const RGN_AND: i32 = 1;
const RGN_OR: i32 = 2;
const RGN_XOR: i32 = 3;
const RGN_DIFF: i32 = 4;
/// The rectangles of a region, "l,t,r,b" joined with " + " (at most 64),
/// or "empty".
fn rgn_text(rgn: HRGN, out: &mut Buf<300>) {
    let mut d: RGNDATA64 = zeroed();
    // SAFETY: d is large enough for the header and 64 rectangles.
    let n = unsafe { GetRegionData(rgn, core::mem::size_of::<RGNDATA64>() as u32, &mut d) };
    if n == 0 {
        let _ = out.write_str("(no data)");
        return;
    }
    if d.count == 0 {
        let _ = out.write_str("empty");
        return;
    }
    for i in 0..(d.count as usize).min(64) {
        let r = d.rects[i];
        if i > 0 {
            let _ = out.write_str(" + ");
        }
        let _ = write!(out, "{},{},{},{}", r.left, r.top, r.right, r.bottom);
    }
}

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
    fn GetRegionData(rgn: HRGN, n: u32, data: *mut RGNDATA64) -> u32;
}

fn cr_pair(t: &mut Test, label: &str, a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) {
    // SAFETY: regions of ours, deleted after use.
    unsafe {
        let ra = CreateRectRgn(a.0, a.1, a.2, a.3);
        let rb = CreateRectRgn(b.0, b.1, b.2, b.3);
        for (name, mode) in [("OR", RGN_OR), ("AND", RGN_AND), ("DIFF", RGN_DIFF), ("XOR", RGN_XOR)] {
            let d = CreateRectRgn(0, 0, 0, 0);
            let k = CombineRgn(d, ra, rb, mode);
            let mut s = Buf::<300>::new();
            rgn_text(d, &mut s);
            say!(t, "{label} {name}: kind {k} [{}]", s.as_str());
            DeleteObject(d);
        }
        DeleteObject(ra);
        DeleteObject(rb);
    }
}

fn cr_union(t: &mut Test, label: &str, rects: &[(i32, i32, i32, i32)]) {
    // SAFETY: regions of ours, deleted after use.
    unsafe {
        let acc = CreateRectRgn(0, 0, 0, 0);
        let mut k = 0;
        for r in rects {
            let one = CreateRectRgn(r.0, r.1, r.2, r.3);
            k = CombineRgn(acc, acc, one, RGN_OR);
            DeleteObject(one);
        }
        let mut s = Buf::<300>::new();
        rgn_text(acc, &mut s);
        let mut bx = RECT::default();
        GetRgnBox(acc, &mut bx);
        say!(t, "union {label}: kind {k} box {} [{}]", R(bx), s.as_str());
        DeleteObject(acc);
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    cr_pair(t, "side by side", (0, 0, 10, 10), (10, 0, 20, 10));
    cr_pair(t, "side by side, different heights", (0, 0, 10, 10), (10, 5, 20, 15));
    cr_pair(t, "top to bottom", (0, 0, 10, 10), (0, 10, 10, 20));
    cr_pair(t, "top to bottom, different widths", (0, 0, 10, 10), (5, 10, 15, 20));
    cr_pair(t, "disjoint", (0, 0, 10, 10), (20, 20, 30, 30));
    cr_pair(t, "disjoint same band", (0, 0, 10, 10), (20, 0, 30, 10));
    cr_pair(t, "nested", (0, 0, 30, 30), (10, 10, 20, 20));
    cr_pair(t, "identical", (5, 5, 15, 15), (5, 5, 15, 15));
    cr_pair(t, "corner overlap 1px", (0, 0, 10, 10), (9, 9, 20, 20));
    cr_pair(t, "empty and rect", (0, 0, 0, 0), (5, 5, 15, 15));
    cr_pair(t, "empty and empty", (3, 3, 3, 3), (7, 7, 7, 9));
    cr_union(t, "staircase", &[(0, 0, 10, 10), (10, 10, 20, 20), (20, 20, 30, 30)]);
    cr_union(t, "frame", &[(0, 0, 30, 5), (0, 25, 30, 30), (0, 0, 5, 30), (25, 0, 30, 30)]);
    cr_union(t, "cross", &[(10, 0, 20, 30), (0, 10, 30, 20)]);
    cr_union(t, "two rows of tiles", &[(0, 0, 10, 10), (10, 0, 20, 10), (0, 10, 10, 20), (10, 10, 20, 20)]);
    cr_union(t, "comb", &[(0, 0, 2, 10), (4, 0, 6, 10), (8, 0, 10, 10), (0, 10, 10, 12)]);
}
