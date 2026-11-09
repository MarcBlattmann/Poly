//! PtInRegion and RectInRegion: RectInRegion with rectangles inside,
//! overlapping, touching from outside (sharing only an edge), swapped and
//! empty, against a rectangle and an L-shaped region (a rectangle in its
//! cut-out corner); PtInRegion and RectInRegion with handles that are not
//! regions (the return value and the last error); PtInRegion at extreme
//! coordinates.
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

const BLACK_BRUSH: i32 = 4;
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

type HRGN = *mut c_void;
const RGN_OR: i32 = 2;
/// "l,t,r,b" of a rectangle.
struct R(RECT);

impl core::fmt::Display for R {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{},{},{},{}", self.0.left, self.0.top, self.0.right, self.0.bottom)
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn GetStockObject(index: i32) -> HGDIOBJ;
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn CombineRgn(dst: HRGN, a: HRGN, b: HRGN, mode: i32) -> i32;
    fn PtInRegion(rgn: HRGN, x: i32, y: i32) -> BOOL;
    fn RectInRegion(rgn: HRGN, rect: *const RECT) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: regions of ours; invalid handles are what is observed.
    unsafe {
        let r = CreateRectRgn(10, 10, 20, 20);
        let a = CreateRectRgn(0, 0, 10, 30);
        let c = CreateRectRgn(0, 20, 30, 30);
        let l = CreateRectRgn(0, 0, 0, 0);
        CombineRgn(l, a, c, RGN_OR);
        let rects = [
            ("inside", 12, 12, 15, 15),
            ("overlapping", 15, 15, 25, 25),
            ("touching right edge", 20, 10, 30, 20),
            ("touching left edge", 0, 10, 10, 20),
            ("swapped inside", 15, 15, 12, 12),
            ("swapped over the right edge", 25, 15, 15, 18),
            ("empty inside", 15, 15, 15, 15),
            ("zero height inside", 12, 15, 18, 15),
            ("in the L's cut-out", 15, 5, 25, 15),
            ("covering the L", -5, -5, 40, 40),
        ];
        for (label, x0, y0, x1, y1) in rects {
            let rc = RECT { left: x0, top: y0, right: x1, bottom: y1 };
            say!(t, "{label} {}: rect region {} L region {}", R(rc), RectInRegion(r, &rc), RectInRegion(l, &rc));
        }
        say!(
            t,
            "extremes: ({},{})={} ({},{})={}",
            i32::MIN,
            i32::MIN,
            PtInRegion(r, i32::MIN, i32::MIN),
            i32::MAX,
            i32::MAX,
            PtInRegion(r, i32::MAX, i32::MAX)
        );
        let rc = RECT { left: 1, top: 1, right: 2, bottom: 2 };
        for (label, h) in [
            ("NULL", core::ptr::null_mut()),
            ("made-up", 0x0bad_0bad_usize as HRGN),
            ("a brush", GetStockObject(BLACK_BRUSH) as HRGN),
        ] {
            set_error(0);
            let p = PtInRegion(h, 1, 1);
            let e1 = last_error();
            set_error(0);
            let q = RectInRegion(h, &rc);
            say!(t, "{label}: PtInRegion {p} error {e1}, RectInRegion {q} error {}", last_error());
        }
        for h in [r, a, c, l] {
            DeleteObject(h);
        }
    }
}
