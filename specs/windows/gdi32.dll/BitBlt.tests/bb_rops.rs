//! BitBlt raster operations on 32-bit DIBs, per pixel: SRCCOPY, SRCPAINT
//! (OR), SRCAND (AND), SRCINVERT (XOR), NOTSRCCOPY, DSTINVERT, BLACKNESS,
//! WHITENESS, PATCOPY (the selected brush; no source needed) - each
//! applied to the same destination/source pair and printed in hex; the
//! high byte of 32-bit pixels after each (observed).
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

const SRCCOPY: u32 = 0x00CC_0020;
const SRCPAINT: u32 = 0x00EE_0086;
const SRCAND: u32 = 0x0088_00C6;
const SRCINVERT: u32 = 0x0066_0046;
const NOTSRCCOPY: u32 = 0x0033_0008;
const BLACKNESS: u32 = 0x0000_0042;
const WHITENESS: u32 = 0x00FF_0062;
const PATCOPY: u32 = 0x00F0_0021;
const DSTINVERT: u32 = 0x0055_0009;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const WHITE_BRUSH: i32 = 0;
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetStockObject(index: i32) -> HGDIOBJ;
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
    let (src, sbmp, sbits) = dib32(2, 1, true);
    let (dst, dbmp, dbits) = dib32(2, 1, true);
    // SAFETY: GDI calls on our DCs; writes inside the DIBs.
    unsafe {
        let brush = CreateSolidBrush(rgb(0x0F, 0xF0, 0x55));
        SelectObject(dst, brush);
        for (rop, name) in [
            (SRCCOPY, "SRCCOPY"),
            (SRCPAINT, "SRCPAINT"),
            (SRCAND, "SRCAND"),
            (SRCINVERT, "SRCINVERT"),
            (NOTSRCCOPY, "NOTSRCCOPY"),
            (DSTINVERT, "DSTINVERT"),
            (BLACKNESS, "BLACKNESS"),
            (WHITENESS, "WHITENESS"),
            (PATCOPY, "PATCOPY"),
        ] {
            *sbits = 0x00F0_F00F;
            *sbits.add(1) = 0x0012_3456;
            *dbits = 0x000F_0FF0;
            *dbits.add(1) = 0x00AB_CDEF;
            GdiFlush();
            let r = BitBlt(dst, 0, 0, 2, 1, src, 0, 0, rop);
            GdiFlush();
            say!(t, "{name}: returned {} pixels {:08x} {:08x}", r, *dbits, *dbits.add(1));
        }
        *dbits = 0x0011_1111;
        let r = BitBlt(dst, 0, 0, 1, 1, core::ptr::null_mut(), 0, 0, PATCOPY);
        GdiFlush();
        say!(t, "PATCOPY with a NULL source DC: returned {} pixel {:08x}", r, *dbits);
        set_error(0);
        let r = BitBlt(dst, 0, 0, 1, 1, core::ptr::null_mut(), 0, 0, SRCCOPY);
        say!(t, "SRCCOPY with a NULL source DC: returned {} error {}", r, last_error());
        SelectObject(dst, GetStockObject(WHITE_BRUSH));
        DeleteObject(brush);
        DeleteDC(src);
        DeleteDC(dst);
        DeleteObject(sbmp);
        DeleteObject(dbmp);
    }
}
