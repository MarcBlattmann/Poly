//! CreateSolidBrush: a brush object (GetObjectType OBJ_BRUSH) whose
//! LOGBRUSH (GetObjectW, 16 bytes on x64) is style BS_SOLID, the colour
//! as given, hatch 0; two calls give two objects; selected into a DC it
//! paints that colour with PatBlt(PATCOPY); stock brushes' LOGBRUSH
//! values (white, black, null, DC brush); after DeleteObject the handle
//! still reads as the same brush (observed: deleted solid brushes stay
//! valid handles).
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

const PATCOPY: u32 = 0x00F0_0021;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const WHITE_BRUSH: i32 = 0;
const BLACK_BRUSH: i32 = 4;
const NULL_BRUSH: i32 = 5;
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
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
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
}

poly_wtest::main!(test);

fn logbrush(t: &mut Test, label: &str, b: HGDIOBJ) {
    let mut lb = LOGBRUSH { style: 0xFFFF, color: 0xFFFF_FFFF, hatch: 0xFFFF };
    // SAFETY: lb is a writable LOGBRUSH.
    let n = unsafe { GetObjectW(b, core::mem::size_of::<LOGBRUSH>() as i32, &mut lb as *mut LOGBRUSH as *mut c_void) };
    // SAFETY: plain query.
    let ty = unsafe { GetObjectType(b) };
    say!(t, "{label}: type={} GetObject={} style={} color={:#08x} hatch={}", ty, n, lb.style, lb.color, lb.hatch);
}

fn test(t: &mut Test) {
    // SAFETY: GDI object calls.
    unsafe {
        let a = CreateSolidBrush(rgb(0x11, 0x22, 0x33));
        let b = CreateSolidBrush(rgb(0x11, 0x22, 0x33));
        t.check("two_objects", !a.is_null() && !b.is_null() && a != b, 0);
        logbrush(t, "solid", a);
        logbrush(t, "white stock", GetStockObject(WHITE_BRUSH));
        logbrush(t, "black stock", GetStockObject(BLACK_BRUSH));
        logbrush(t, "null stock", GetStockObject(NULL_BRUSH));
        logbrush(t, "dc brush stock", GetStockObject(DC_BRUSH));
        let n = GetObjectW(a, 0, core::ptr::null_mut());
        say!(t, "GetObjectW size query: {}", n);
        let (dc, bmp, bits) = dib32(3, 2, true);
        let old = SelectObject(dc, a);
        say!(t, "select returned the white stock brush {}", (old == GetStockObject(WHITE_BRUSH)) as i32);
        PatBlt(dc, 0, 0, 2, 1, PATCOPY);
        GdiFlush();
        rows(t, bits, 3, 2, &[(0, '.'), (0x0011_2233, 'S')]);
        SelectObject(dc, old);
        say!(t, "DeleteObject(a) {}", DeleteObject(a));
        logbrush(t, "deleted", a);
        DeleteObject(b);
        DeleteDC(dc);
        DeleteObject(bmp);
    }
}
