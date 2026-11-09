//! BitBlt between colour and monochrome: colour -> 1-bit: source pixels
//! equal to the source DC's background colour become 1 (white), all
//! others 0 (black); 1-bit -> colour: 0 bits take the destination DC's
//! text colour and 1 bits its background colour; shown on small bitmaps.
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

const SRCCOPY: u32 = 0x00CC_0020;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
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

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
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
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: GDI calls on our DCs; reads/writes inside the bitmaps.
    unsafe {
        let (src, sbmp, sbits) = dib32(4, 1, true);
        *sbits = 0x00FF_0000;
        *sbits.add(1) = 0x0000_FF00;
        *sbits.add(2) = 0x00FF_FFFF;
        *sbits.add(3) = 0;
        GdiFlush();
        let m = CreateCompatibleDC(core::ptr::null_mut());
        let mono = CreateBitmap(4, 1, 1, 1, core::ptr::null());
        let old = SelectObject(m, mono);
        SetBkColor(src, rgb(0, 255, 0));
        BitBlt(m, 0, 0, 4, 1, src, 0, 0, SRCCOPY);
        say!(
            t,
            "colour->mono with source bk = green: {:06x} {:06x} {:06x} {:06x}",
            GetPixel(m, 0, 0),
            GetPixel(m, 1, 0),
            GetPixel(m, 2, 0),
            GetPixel(m, 3, 0)
        );
        SetBkColor(src, rgb(255, 255, 255));
        BitBlt(m, 0, 0, 4, 1, src, 0, 0, SRCCOPY);
        say!(
            t,
            "colour->mono with source bk = white: {:06x} {:06x} {:06x} {:06x}",
            GetPixel(m, 0, 0),
            GetPixel(m, 1, 0),
            GetPixel(m, 2, 0),
            GetPixel(m, 3, 0)
        );
        let (dst, dbmp, dbits) = dib32(4, 1, true);
        SetTextColor(dst, rgb(0x11, 0x22, 0x33));
        SetBkColor(dst, rgb(0xAA, 0xBB, 0xCC));
        BitBlt(dst, 0, 0, 4, 1, m, 0, 0, SRCCOPY);
        GdiFlush();
        say!(
            t,
            "mono->colour with dst text 0x332211 bk 0xccbbaa: {:06x} {:06x} {:06x} {:06x}",
            *dbits,
            *dbits.add(1),
            *dbits.add(2),
            *dbits.add(3)
        );
        SelectObject(m, old);
        DeleteObject(mono);
        DeleteDC(m);
        DeleteDC(src);
        DeleteDC(dst);
        DeleteObject(sbmp);
        DeleteObject(dbmp);
    }
}
