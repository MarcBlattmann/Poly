//! CreateRectRgnIndirect with a list of rectangles: normal, left > right,
//! top > bottom, both reversed, empty (zero width, zero height, all zero),
//! negative, straddling zero, 1x1, very large and the extreme int values.
//! For each: whether a region comes back, GetObjectType (OBJ_REGION),
//! GetRgnBox (region type and the box), the rectangles the region is made
//! of, PtInRegion at the four corners (the last row and column are outside),
//! and EqualRgn against CreateRectRgn with the normalised numbers. Then
//! DeleteObject once and twice and what GetRgnBox and PtInRegion say about
//! the deleted region and about NULL. (A NULL rectangle pointer is not
//! passed: the call reads the rectangle in user mode.)
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

const GR_GDIOBJECTS: u32 = 0;
/// The GDI object count of this process (only differences are printed).
fn gdi_count() -> u32 {
    // SAFETY: the pseudo handle of this process.
    unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcess() -> HANDLE;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetGuiResources(process: HANDLE, flags: u32) -> u32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn CreateRectRgnIndirect(rect: *const RECT) -> HRGN;
    fn GetRgnBox(rgn: HRGN, rect: *mut RECT) -> i32;
    fn PtInRegion(rgn: HRGN, x: i32, y: i32) -> BOOL;
    fn EqualRgn(a: HRGN, b: HRGN) -> BOOL;
    fn GetRegionData(rgn: HRGN, n: u32, data: *mut RGNDATA64) -> u32;
}

poly_wtest::main!(test);

fn case(t: &mut Test, l: i32, tp: i32, r: i32, b: i32) {
    // SAFETY: the RECT is ours; the regions are deleted before returning.
    unsafe {
        let rc = RECT { left: l, top: tp, right: r, bottom: b };
        set_error(0x5A5A);
        let rgn = CreateRectRgnIndirect(&rc);
        let e = last_error();
        if rgn.is_null() {
            say!(t, "({l},{tp},{r},{b}): NULL (error {})", if e == 0x5A5A { 0 } else { e });
            return;
        }
        let mut bx = RECT::default();
        let kind = GetRgnBox(rgn, &mut bx);
        let mut text = Buf::<300>::new();
        rgn_text(rgn, &mut text);
        say!(t, "({l},{tp},{r},{b}): type {} box type {kind} box {} ; rects {}", GetObjectType(rgn), R(bx), text.as_str());
        let corners = [(bx.left, bx.top), (bx.right.wrapping_sub(1), bx.top), (bx.left, bx.bottom.wrapping_sub(1)), (bx.right.wrapping_sub(1), bx.bottom.wrapping_sub(1)), (bx.right, bx.top), (bx.left, bx.bottom), (bx.right, bx.bottom)];
        let mut hits = Buf::<20>::new();
        for (x, y) in corners {
            let _ = write!(hits, "{}", (PtInRegion(rgn, x, y) != 0) as i32);
        }
        let (nl, nt, nr, nb) = (l.min(r), tp.min(b), l.max(r), tp.max(b));
        let same = CreateRectRgn(nl, nt, nr, nb);
        say!(t, "  corners tl tr bl br (inside) and right-top, left-bottom, right-bottom (outside): {} ; equals CreateRectRgn of the normalised numbers {}", hits.as_str(), EqualRgn(rgn, same));
        DeleteObject(same);
        DeleteObject(rgn);
    }
}

fn test(t: &mut Test) {
    for (l, tp, r, b) in [
        (10, 20, 30, 40),
        (30, 20, 10, 40),
        (10, 40, 30, 20),
        (30, 40, 10, 20),
        (10, 20, 10, 40),
        (10, 20, 30, 20),
        (10, 20, 10, 20),
        (0, 0, 0, 0),
        (0, 0, 1, 1),
        (-1, -1, 0, 0),
        (-30, -20, -10, -5),
        (-5, -5, 5, 5),
        (5, 5, -5, -5),
        (0, 0, 0x7FFF_FFFF, 0x7FFF_FFFF),
        (0x7FFF_FFFF, 0x7FFF_FFFF, 0, 0),
        (-0x7FFF_FFFF, -0x7FFF_FFFF, 0, 0),
        (0, 0, 0x0100_0000, 0x0100_0000),
        (0x7FFF_FFFF, 0x7FFF_FFFF, 0x7FFF_FFFF, 0x7FFF_FFFF),
        (i32::MIN, i32::MIN, i32::MAX, i32::MAX),
        (0, 0, 70000, 3),
        (0, 0, 3, 70000),
    ] {
        case(t, l, tp, r, b);
    }
    // SAFETY: regions of this test.
    unsafe {
        let rc = RECT { left: 2, top: 3, right: 12, bottom: 9 };
        let rgn = CreateRectRgnIndirect(&rc);
        let before = gdi_count();
        let copy = CreateRectRgnIndirect(&rc);
        say!(t, "one more GDI object per region: {} ; handles differ {}", (gdi_count() == before + 1) as i32, (copy != rgn) as i32);
        say!(t, "the RECT is not changed by the call: {}", ((rc.left, rc.top, rc.right, rc.bottom) == (2, 3, 12, 9)) as i32);
        DeleteObject(copy);
        set_error(0x5A5A);
        let a = DeleteObject(rgn);
        let ea = last_error();
        set_error(0x5A5A);
        let b = DeleteObject(rgn);
        let eb = last_error();
        say!(t, "DeleteObject: first {a} (error {}) second {b} (error {})", if ea == 0x5A5A { 0 } else { ea }, if eb == 0x5A5A { 0 } else { eb });
        let mut bx = RECT { left: 7, top: 7, right: 7, bottom: 7 };
        set_error(0x5A5A);
        let k = GetRgnBox(rgn, &mut bx);
        say!(t, "GetRgnBox of the deleted region: {k} box {} (error {}) ; PtInRegion {} ; type {}", R(bx), if last_error() == 0x5A5A { 0 } else { last_error() }, PtInRegion(rgn, 5, 5), GetObjectType(rgn));
        let mut bx = RECT { left: 7, top: 7, right: 7, bottom: 7 };
        let k = GetRgnBox(core::ptr::null_mut(), &mut bx);
        say!(t, "GetRgnBox(NULL): {k} box {} ; PtInRegion(NULL) {} ; EqualRgn(NULL, NULL) {}", R(bx), PtInRegion(core::ptr::null_mut(), 5, 5), EqualRgn(core::ptr::null_mut(), core::ptr::null_mut()));
        say!(t, "GDI count back: {}", (gdi_count() == before - 1) as i32);
    }
}
