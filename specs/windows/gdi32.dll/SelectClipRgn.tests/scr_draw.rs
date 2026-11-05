//! SelectClipRgn on a memory DC with a 20x10 32-bit DIB: its return value
//! (the kind of the new clip region), that the region is copied (deleting it
//! right after keeps the clip), FillRect over the whole bitmap drawing only
//! inside the clip (shown as pixel rows: '.' white, '#' black, 'r' red), an
//! empty clip region (nothing drawn), a clip larger than the bitmap, NULL
//! removing the clip, and the clip staying in device coordinates when the
//! viewport origin moves.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HBRUSH = *mut c_void;
type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type BOOL = i32;
type COLORREF = u32;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
}

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy)]
struct BITMAPINFOHEADER {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    clr_used: u32,
    clr_important: u32,
}

/// wingdi.h (one RGBQUAD as a u32)
#[repr(C)]
struct BITMAPINFO {
    header: BITMAPINFOHEADER,
    colors: [u32; 1],
}

const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
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

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// A memory DC with a 32-bit top-down (or bottom-up) DIB section selected:
/// (dc, bitmap, pixels). Pixels are 0x00RRGGBB, row by row from the top
/// for top-down.
fn dib32(w: i32, h: i32, top_down: bool) -> (HDC, HBITMAP, *mut u32) {
    let info = BITMAPINFO {
        header: BITMAPINFOHEADER {
            size: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
            width: w,
            height: if top_down { -h } else { h },
            planes: 1,
            bit_count: 32,
            compression: BI_RGB,
            size_image: 0,
            x_pels_per_meter: 0,
            y_pels_per_meter: 0,
            clr_used: 0,
            clr_important: 0,
        },
        colors: [0],
    };
    let mut bits: *mut c_void = core::ptr::null_mut();
    // SAFETY: a filled BITMAPINFO; bits receives the pixel pointer.
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        let bmp = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, core::ptr::null_mut(), 0);
        SelectObject(dc, bmp);
        (dc, bmp, bits as *mut u32)
    }
}

/// The pixel at (x, y) of a top-down 32-bit DIB of width w.
fn px(bits: *mut u32, w: i32, x: i32, y: i32) -> u32 {
    // SAFETY: callers stay inside the DIB.
    unsafe { *bits.add((y * w + x) as usize) }
}

/// Set every pixel of a w x h 32-bit DIB to v.
fn fill_bits(bits: *mut u32, w: i32, h: i32, v: u32) {
    for i in 0..(w * h) as usize {
        // SAFETY: inside the DIB.
        unsafe { *bits.add(i) = v };
    }
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

/// The bounding box and count of pixels that are not `bg`.
fn ink(bits: *mut u32, w: i32, h: i32, bg: u32) -> (i32, i32, i32, i32, u32) {
    let (mut l, mut tp, mut r, mut b, mut n) = (i32::MAX, i32::MAX, -1, -1, 0);
    for y in 0..h {
        for x in 0..w {
            if px(bits, w, x, y) != bg {
                l = l.min(x);
                tp = tp.min(y);
                r = r.max(x + 1);
                b = b.max(y + 1);
                n += 1;
            }
        }
    }
    if n == 0 { (0, 0, 0, 0, 0) } else { (l, tp, r, b, n) }
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

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn FillRect(hdc: HDC, rect: *const RECT, brush: HBRUSH) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetStockObject(index: i32) -> HGDIOBJ;
    fn GetClipBox(hdc: HDC, rect: *mut RECT) -> i32;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn CreateDIBSection(
        hdc: HDC,
        info: *const BITMAPINFO,
        usage: u32,
        bits: *mut *mut c_void,
        section: HANDLE,
        offset: u32,
    ) -> HBITMAP;
    fn GdiFlush() -> BOOL;
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn CombineRgn(dst: HRGN, a: HRGN, b: HRGN, mode: i32) -> i32;
    fn SelectClipRgn(hdc: HDC, rgn: HRGN) -> i32;
    fn SetViewportOrgEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
}

poly_wtest::main!(test);

fn scr_rows(t: &mut Test, label: &str, bits: *mut u32) {
    say!(t, "{label}:");
    rows(t, bits, 20, 10, &[(0x00FF_FFFF, '.'), (0, '#'), (0x00FF_0000, 'r')]);
}

fn test(t: &mut Test) {
    let (dc, bmp, bits) = dib32(20, 10, true);
    let all = RECT { left: 0, top: 0, right: 20, bottom: 10 };
    // SAFETY: our DC, regions and brushes; all deleted at the end.
    unsafe {
        let black = GetStockObject(BLACK_BRUSH) as HBRUSH;
        let red = CreateSolidBrush(rgb(255, 0, 0));
        fill_bits(bits, 20, 10, 0x00FF_FFFF);
        let a = CreateRectRgn(2, 2, 8, 6);
        let b = CreateRectRgn(12, 4, 18, 9);
        let c = CreateRectRgn(0, 0, 0, 0);
        CombineRgn(c, a, b, RGN_OR);
        say!(t, "SelectClipRgn(two rects) = {}", SelectClipRgn(dc, c));
        say!(t, "DeleteObject(the region) = {}", DeleteObject(c));
        FillRect(dc, &all, black);
        GdiFlush();
        scr_rows(t, "FillRect black over everything, clip two rects (region deleted)", bits);
        fill_bits(bits, 20, 10, 0x00FF_FFFF);
        let e = CreateRectRgn(5, 5, 5, 5);
        say!(t, "SelectClipRgn(empty) = {}", SelectClipRgn(dc, e));
        FillRect(dc, &all, black);
        GdiFlush();
        let (_, _, _, _, n) = ink(bits, 20, 10, 0x00FF_FFFF);
        say!(t, "  pixels drawn with an empty clip: {n}");
        let big = CreateRectRgn(-10, -10, 100, 100);
        say!(t, "SelectClipRgn(-10,-10,100,100) = {}", SelectClipRgn(dc, big));
        let mut bx = RECT::default();
        say!(t, "  GetClipBox {} {}", GetClipBox(dc, &mut bx), R(bx));
        say!(t, "SelectClipRgn(NULL) = {}", SelectClipRgn(dc, core::ptr::null_mut()));
        FillRect(dc, &all, red);
        GdiFlush();
        let (_, _, _, _, n) = ink(bits, 20, 10, 0x00FF_FFFF);
        say!(t, "  pixels drawn red with no clip: {n}");
        fill_bits(bits, 20, 10, 0x00FF_FFFF);
        say!(t, "SelectClipRgn(a = 2,2,8,6) = {}", SelectClipRgn(dc, a));
        let mut old = POINT::default();
        SetViewportOrgEx(dc, 5, 2, &mut old);
        FillRect(dc, &all, black);
        GdiFlush();
        scr_rows(t, "viewport origin 5,2, FillRect logical 0,0,20,10", bits);
        say!(t, "  GetClipBox (logical) {} {}", GetClipBox(dc, &mut bx), R(bx));
        fill_bits(bits, 20, 10, 0x00FF_FFFF);
        say!(t, "SelectClipRgn(a) with the viewport origin at 5,2 = {}", SelectClipRgn(dc, a));
        SetViewportOrgEx(dc, 0, 0, &mut old);
        FillRect(dc, &all, black);
        GdiFlush();
        scr_rows(t, "  then origin back to 0,0, FillRect everything", bits);
        for h in [a, b, e, big] {
            DeleteObject(h);
        }
        DeleteObject(red);
        DeleteDC(dc);
        DeleteObject(bmp);
    }
}
