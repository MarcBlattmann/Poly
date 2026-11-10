//! A pen or brush deleted while selected: does the DC keep drawing with it
//! when it had already been used for drawing before the deletion (the DC
//! having captured its colour), and what happens when it is drawn with only
//! once, drawn with through other calls (FillRgn, Rectangle, PatBlt), or
//! when the DC state is saved and restored around the deletion.
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
const BLACK_BRUSH: i32 = 4;
const BLACK_PEN: i32 = 7;
const OBJ_BRUSH: u32 = 2;
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

type HRGN = *mut c_void;

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetStockObject(index: i32) -> HGDIOBJ;
    fn GetCurrentObject(hdc: HDC, kind: u32) -> HGDIOBJ;
    fn CreatePen(style: i32, width: i32, color: COLORREF) -> HGDIOBJ;
    fn SaveDC(hdc: HDC) -> i32;
    fn RestoreDC(hdc: HDC, saved: i32) -> BOOL;
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
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn FillRgn(hdc: HDC, rgn: HRGN, brush: HBRUSH) -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MoveToEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    fn LineTo(hdc: HDC, x: i32, y: i32) -> BOOL;
}

fn pen_case(t: &mut Test, label: &str, use_first: bool, save_restore: bool, reselect_stock_between: bool) {
    // SAFETY: GDI objects of this test, all released at the end.
    unsafe {
        let (dc, bmp, bits) = dib32(8, 8, true);
        let pen = CreatePen(0, 1, rgb(0xE0, 0x10, 0x10));
        let prev = SelectObject(dc, pen);
        if use_first {
            MoveToEx(dc, 0, 1, core::ptr::null_mut());
            LineTo(dc, 8, 1);
        }
        let saved = if save_restore { SaveDC(dc) } else { 0 };
        if reselect_stock_between {
            SelectObject(dc, GetStockObject(BLACK_PEN));
        }
        let r = DeleteObject(pen);
        if save_restore {
            RestoreDC(dc, saved);
        }
        fill_bits(bits, 8, 8, 0x0010_2030);
        MoveToEx(dc, 0, 3, core::ptr::null_mut());
        LineTo(dc, 8, 3);
        MoveToEx(dc, 0, 5, core::ptr::null_mut());
        LineTo(dc, 8, 5);
        GdiFlush();
        say!(t, "pen {label}: delete {r}; line after: {:06x} second line {:06x}", px(bits, 8, 3, 3), px(bits, 8, 3, 5));
        SelectObject(dc, prev);
        DeleteDC(dc);
        DeleteObject(bmp);
    }
}

fn brush_case(t: &mut Test, label: &str, use_first: bool, save_restore: bool, reselect_stock_between: bool) {
    // SAFETY: GDI objects of this test, all released at the end.
    unsafe {
        let (dc, bmp, bits) = dib32(8, 8, true);
        let br = CreateSolidBrush(rgb(0x20, 0x40, 0x60));
        let prev = SelectObject(dc, br);
        if use_first {
            PatBlt(dc, 0, 0, 8, 2, PATCOPY);
        }
        let saved = if save_restore { SaveDC(dc) } else { 0 };
        if reselect_stock_between {
            SelectObject(dc, GetStockObject(BLACK_BRUSH));
        }
        let r = DeleteObject(br);
        if save_restore {
            RestoreDC(dc, saved);
        }
        fill_bits(bits, 8, 8, 0x0010_2030);
        PatBlt(dc, 0, 0, 8, 4, PATCOPY);
        let rgn = CreateRectRgn(0, 4, 8, 6);
        FillRgn(dc, rgn, GetCurrentObject(dc, OBJ_BRUSH));
        DeleteObject(rgn);
        GdiFlush();
        say!(t, "brush {label}: delete {r}; PatBlt after: {:06x} FillRgn with the current brush {:06x}", px(bits, 8, 3, 3), px(bits, 8, 3, 5));
        SelectObject(dc, prev);
        DeleteDC(dc);
        DeleteObject(bmp);
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for (label, used, save, stock) in [
        ("unused", false, false, false),
        ("used before", true, false, false),
        ("unused, saved and restored", false, true, false),
        ("used before, saved and restored", true, true, false),
        ("unused, deselected before the delete, saved and restored", false, true, true),
        ("used, deselected before the delete, saved and restored", true, true, true),
    ] {
        pen_case(t, label, used, save, stock);
    }
    for (label, used, save, stock) in [
        ("unused", false, false, false),
        ("used before", true, false, false),
        ("unused, saved and restored", false, true, false),
        ("used before, saved and restored", true, true, false),
        ("unused, deselected before the delete, saved and restored", false, true, true),
        ("used, deselected before the delete, saved and restored", true, true, true),
    ] {
        brush_case(t, label, used, save, stock);
    }
}
