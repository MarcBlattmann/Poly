//! SetPixel on a 32-bit DIB: writes 0x00RRGGBB for COLORREF 0x00BBGGRR
//! and returns the colour actually set (the same for 32 bits); outside the
//! bitmap it returns -1 (CLR_INVALID) and writes nothing; SetPixelV
//! returns nonzero / 0; a bad DC returns -1; a palette-index COLORREF
//! (0x01000000 | n) is observed.
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

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn SetPixel(hdc: HDC, x: i32, y: i32, color: COLORREF) -> COLORREF;
    fn SetPixelV(hdc: HDC, x: i32, y: i32, color: COLORREF) -> BOOL;
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
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let (w, h) = (4, 2);
    let (dc, bmp, bits) = dib32(w, h, true);
    fill_bits(bits, w, h, 0x0077_7777);
    // SAFETY: GDI calls on our DC.
    unsafe {
        say!(t, "SetPixel(0,0,0x563412) returned {:#010x}", SetPixel(dc, 0, 0, rgb(0x12, 0x34, 0x56)));
        say!(t, "SetPixel(3,1,white) returned {:#010x}", SetPixel(dc, 3, 1, rgb(255, 255, 255)));
        say!(t, "SetPixel(4,0) outside returned {:#010x}", SetPixel(dc, 4, 0, 0));
        say!(t, "SetPixel(-1,-1) outside returned {:#010x}", SetPixel(dc, -1, -1, 0));
        say!(t, "SetPixelV(1,0,red) returned {}", SetPixelV(dc, 1, 0, rgb(255, 0, 0)));
        say!(t, "SetPixelV(9,9) returned {}", SetPixelV(dc, 9, 9, 0));
        say!(t, "SetPixel(2,0,0x01000002) returned {:#010x}", SetPixel(dc, 2, 0, 0x0100_0002));
        say!(t, "SetPixel bad DC returned {:#010x}", SetPixel(0x0bad_0bad_usize as HDC, 0, 0, 0));
        GdiFlush();
        let mut line = Buf::<300>::new();
        for y in 0..h {
            for x in 0..w {
                let _ = write!(line, "{:08x} ", px(bits, w, x, y));
            }
        }
        say!(t, "pixels: {}", line.as_str());
        DeleteDC(dc);
        DeleteObject(bmp);
    }
}
