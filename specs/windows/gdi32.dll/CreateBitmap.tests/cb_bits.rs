//! CreateBitmap with real bits: a 16x16 monochrome bitmap (2 bytes per
//! row) selected into a memory DC and read with GetPixel (0 bits black, 1
//! bits white), copied with BitBlt onto a 32-bit DIB with a text colour and
//! a background colour set (0 bits take the text colour, 1 bits the
//! background colour), a 10x3 monochrome bitmap whose rows are padded to 16
//! bits (GetBitmapBits shows the stored bytes and the count), SetBitmapBits
//! on it, and a 4x2 32-bit bitmap with a pixel array (GetPixel through a DC).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

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

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BITMAP {
    kind: i32,
    width: i32,
    height: i32,
    width_bytes: i32,
    planes: u16,
    bits_pixel: u16,
    bits: usize,
}

const SRCCOPY: u32 = 0x00CC_0020;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const CLR_INVALID: u32 = 0xFFFF_FFFF;
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
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
    fn SetTextColor(hdc: HDC, color: COLORREF) -> COLORREF;
    fn SetBkColor(hdc: HDC, color: COLORREF) -> COLORREF;
    fn GetPixel(hdc: HDC, x: i32, y: i32) -> COLORREF;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const c_void) -> HBITMAP;
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
    fn BitBlt(dst: HDC, x: i32, y: i32, w: i32, h: i32, src: HDC, sx: i32, sy: i32, rop: u32) -> BOOL;
    fn GdiFlush() -> BOOL;
    fn GetBitmapBits(bmp: HBITMAP, count: i32, bits: *mut c_void) -> i32;
    fn SetBitmapBits(bmp: HBITMAP, count: u32, bits: *const c_void) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: GDI objects of this test; the bit arrays have the sizes the
    // bitmaps need.
    unsafe {
        let mut mask = [0u8; 32];
        for y in 0..16usize {
            mask[2 * y] = 0x80u8 >> (y % 8);
            mask[2 * y + 1] = !(0x80u8 >> (y % 8));
        }
        let b = CreateBitmap(16, 16, 1, 1, mask.as_ptr() as *const c_void);
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        let old = SelectObject(dc, b);
        for y in 0..6 {
            let mut line = Buf::<80>::new();
            let _ = write!(line, "  |");
            for x in 0..16 {
                let c = match GetPixel(dc, x, y) {
                    0 => '#',
                    0x00FF_FFFF => '.',
                    _ => '?',
                };
                let _ = line.write_char(c);
            }
            let _ = line.write_str("|");
            t.line(line.as_str());
        }
        say!(t, "GetPixel outside the bitmap: {}", (GetPixel(dc, 16, 0) == CLR_INVALID) as i32);

        let (dd, dbmp, dbits) = dib32(16, 4, true);
        SetTextColor(dd, rgb(255, 0, 0));
        SetBkColor(dd, rgb(0, 0, 255));
        say!(t, "BitBlt mono to colour: {}", BitBlt(dd, 0, 0, 16, 4, dc, 0, 0, SRCCOPY));
        GdiFlush();
        rows(t, dbits, 16, 4, &[(0x00FF_0000, 'T'), (0x0000_00FF, 'B')]);
        SelectObject(dc, old);

        let raw = [0xFFu8, 0xC0, 0x00, 0x40, 0xAA, 0x80];
        let n = CreateBitmap(10, 3, 1, 1, raw.as_ptr() as *const c_void);
        let mut got = [0u8; 16];
        let r = GetBitmapBits(n, 16, got.as_mut_ptr() as *mut c_void);
        say!(t, "10x3 mono: GetBitmapBits(16) returns {r}: {:02x?}", &got[..6]);
        let mut small = [0u8; 4];
        say!(t, "GetBitmapBits(4) returns {}: {:02x?}", GetBitmapBits(n, 4, small.as_mut_ptr() as *mut c_void), small);
        let mut bm = BITMAP::default();
        GetObjectW(n, core::mem::size_of::<BITMAP>() as i32, &mut bm as *mut BITMAP as *mut c_void);
        say!(t, "its bytes per row {}", bm.width_bytes);
        let newbits = [0x00u8, 0x00, 0xFF, 0xC0, 0x00, 0x00];
        say!(t, "SetBitmapBits(6) returns {}", SetBitmapBits(n, 6, newbits.as_ptr() as *const c_void));
        GetBitmapBits(n, 16, got.as_mut_ptr() as *mut c_void);
        say!(t, "after: {:02x?}", &got[..6]);
        let o2 = SelectObject(dc, n);
        say!(t, "pixels: row1 x0 {:06x} x9 {:06x}, row0 x0 {:06x}", GetPixel(dc, 0, 1), GetPixel(dc, 9, 1), GetPixel(dc, 0, 0));
        SelectObject(dc, o2);

        let px32: [u32; 8] = [0x0011_2233, 0x0044_5566, 0x0077_8899, 0x00AA_BBCC, 0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0x00FF_FFFF];
        let c = CreateBitmap(4, 2, 1, 32, px32.as_ptr() as *const c_void);
        let mut cm = BITMAP::default();
        GetObjectW(c, core::mem::size_of::<BITMAP>() as i32, &mut cm as *mut BITMAP as *mut c_void);
        say!(t, "4x2 32-bit: bytes per row {} bpp {}", cm.width_bytes, cm.bits_pixel);
        let o3 = SelectObject(dc, c);
        say!(t, "GetPixel select result null {}", o3.is_null() as i32);
        for (x, y) in [(0, 0), (3, 0), (0, 1), (1, 1), (2, 1), (3, 1)] {
            let p = GetPixel(dc, x, y);
            say!(t, "  ({x},{y}) = {p:06x}");
        }
        SelectObject(dc, o3);

        DeleteDC(dc);
        DeleteDC(dd);
        for h in [b, n, c, dbmp] {
            DeleteObject(h);
        }
    }
}
