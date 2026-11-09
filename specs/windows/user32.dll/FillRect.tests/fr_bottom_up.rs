//! FillRect on a bottom-up DIB section (positive height): logical row 0
//! is the last row in memory; on a 16-bit (5-5-5) DIB the colour is
//! reduced to 5 bits per channel (the stored value printed); the brush
//! made with CreateSolidBrush of a colour with a high byte set is
//! observed.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HWND = *mut c_void;
type HBRUSH = *mut c_void;
type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type BOOL = i32;
type COLORREF = u32;
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
#[derive(Clone, Copy, Default)]
struct LOGBRUSH {
    style: u32,
    color: COLORREF,
    hatch: usize,
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

/// "window" or "null" (a handle is never printed).
fn hwnd_kind(h: HWND) -> &'static str {
    if h.is_null() { "null" } else { "window" }
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

/// Set every pixel of a w x h 32-bit DIB to v.
fn fill_bits(bits: *mut u32, w: i32, h: i32, v: u32) {
    for i in 0..(w * h) as usize {
        // SAFETY: inside the DIB.
        unsafe { *bits.add(i) = v };
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
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
    fn GetPixel(hdc: HDC, x: i32, y: i32) -> COLORREF;
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
    let (w, h) = (4, 3);
    let (dc, bmp, bits) = dib32(w, h, false);
    fill_bits(bits, w, h, 0);
    // SAFETY: GDI calls on our memory DC and brush; reads inside the DIB.
    unsafe {
        let b = CreateSolidBrush(rgb(0x10, 0x20, 0x30));
        let r = RECT { left: 1, top: 0, right: 3, bottom: 1 };
        FillRect(dc, &r, b);
        GdiFlush();
        for row in 0..h {
            let mut s = Buf::<300>::new();
            for x in 0..w {
                let _ = write!(s, "{:06x} ", *bits.add((row * w + x) as usize));
            }
            say!(t, "memory row {row}: {}", s.as_str());
        }
        DeleteObject(b);
        DeleteDC(dc);
        DeleteObject(bmp);

        // 16 bits per pixel, BI_RGB = 5-5-5.
        let info = BITMAPINFO {
            header: BITMAPINFOHEADER {
                size: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
                width: 2,
                height: -1,
                planes: 1,
                bit_count: 16,
                compression: BI_RGB,
                size_image: 0,
                x_pels_per_meter: 0,
                y_pels_per_meter: 0,
                clr_used: 0,
                clr_important: 0,
            },
            colors: [0],
        };
        let mut p: *mut c_void = core::ptr::null_mut();
        let dc16 = CreateCompatibleDC(core::ptr::null_mut());
        let b16 = CreateDIBSection(dc16, &info, DIB_RGB_COLORS, &mut p, core::ptr::null_mut(), 0);
        SelectObject(dc16, b16);
        let br = CreateSolidBrush(rgb(0x12, 0x34, 0x56));
        let r = RECT { left: 0, top: 0, right: 1, bottom: 1 };
        FillRect(dc16, &r, br);
        let br2 = CreateSolidBrush(rgb(0xFF, 0xFF, 0xFF));
        let r2 = RECT { left: 1, top: 0, right: 2, bottom: 1 };
        FillRect(dc16, &r2, br2);
        GdiFlush();
        let px16 = p as *const u16;
        say!(t, "16-bit pixels: {:#06x} {:#06x}", *px16, *px16.add(1));
        say!(t, "GetPixel of them: {:#08x} {:#08x}", GetPixel(dc16, 0, 0), GetPixel(dc16, 1, 0));
        let hb = CreateSolidBrush(0x0100_00FF);
        let mut lb = LOGBRUSH::default();
        let n = GetObjectW(hb, core::mem::size_of::<LOGBRUSH>() as i32, &mut lb as *mut LOGBRUSH as *mut c_void);
        say!(t, "CreateSolidBrush(0x010000ff): {} GetObject size {} color {:#010x}", hwnd_kind(hb), n, lb.color);
        DeleteObject(hb);
        DeleteObject(br);
        DeleteObject(br2);
        DeleteDC(dc16);
        DeleteObject(b16);
    }
}
