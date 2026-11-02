//! SelectObject changes what drawing uses: PatBlt(PATCOPY) paints with
//! the selected brush; Rectangle outlines with the selected pen and fills
//! with the selected brush (NULL_BRUSH leaves the inside, NULL_PEN draws
//! no outline: the rectangle is then one pixel smaller each way); the
//! DC_BRUSH stock object paints SetDCBrushColor's colour; shown as pixel
//! rows of a 10x6 DIB.
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

const PATCOPY: u32 = 0x00F0_0021;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const WHITE_BRUSH: i32 = 0;
const NULL_BRUSH: i32 = 5;
const BLACK_PEN: i32 = 7;
const NULL_PEN: i32 = 8;
const DC_BRUSH: i32 = 18;
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

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetStockObject(index: i32) -> HGDIOBJ;
    fn CreatePen(style: i32, width: i32, color: COLORREF) -> HGDIOBJ;
    fn Rectangle(hdc: HDC, l: i32, t: i32, r: i32, b: i32) -> BOOL;
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
    fn PatBlt(hdc: HDC, x: i32, y: i32, w: i32, h: i32, rop: u32) -> BOOL;
    fn GdiFlush() -> BOOL;
    fn SetDCBrushColor(hdc: HDC, color: COLORREF) -> COLORREF;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let (w, h) = (10, 6);
    let (dc, bmp, bits) = dib32(w, h, true);
    let key =
        [(0x00FF_FFFF, '.'), (0x00FF_0000, 'R'), (0x0000_FF00, 'G'), (0x0000_00FF, 'B'), (0, 'K'), (0x0012_3456, 'D')];
    // SAFETY: GDI calls on our DC and objects.
    unsafe {
        let red = CreateSolidBrush(rgb(255, 0, 0));
        let green = CreatePen(0, 1, rgb(0, 255, 0));
        let blue = CreateSolidBrush(rgb(0, 0, 255));
        fill_bits(bits, w, h, 0x00FF_FFFF);
        SelectObject(dc, red);
        PatBlt(dc, 0, 0, 3, 2, PATCOPY);
        SelectObject(dc, green);
        SelectObject(dc, blue);
        Rectangle(dc, 4, 0, 9, 4);
        GdiFlush();
        t.line("PatBlt red 0,0 3x2; Rectangle green pen blue brush (4,0)-(9,4):");
        rows(t, bits, w, h, &key);
        fill_bits(bits, w, h, 0x00FF_FFFF);
        SelectObject(dc, GetStockObject(NULL_BRUSH));
        Rectangle(dc, 0, 0, 5, 4);
        SelectObject(dc, GetStockObject(NULL_PEN));
        SelectObject(dc, blue);
        Rectangle(dc, 5, 1, 9, 5);
        GdiFlush();
        t.line("NULL_BRUSH rectangle (0,0)-(5,4); NULL_PEN blue rectangle (5,1)-(9,5):");
        rows(t, bits, w, h, &key);
        fill_bits(bits, w, h, 0x00FF_FFFF);
        SelectObject(dc, GetStockObject(DC_BRUSH));
        let old = SetDCBrushColor(dc, rgb(0x12, 0x34, 0x56));
        say!(t, "SetDCBrushColor returned the old colour {:#08x}", old);
        PatBlt(dc, 1, 1, 2, 2, PATCOPY);
        GdiFlush();
        t.line("DC_BRUSH PatBlt (1,1) 2x2:");
        rows(t, bits, w, h, &key);
        SelectObject(dc, GetStockObject(WHITE_BRUSH));
        SelectObject(dc, GetStockObject(BLACK_PEN));
        DeleteObject(red);
        DeleteObject(green);
        DeleteObject(blue);
        DeleteDC(dc);
        DeleteObject(bmp);
    }
}
