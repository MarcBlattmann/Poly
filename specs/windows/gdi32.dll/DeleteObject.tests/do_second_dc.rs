//! A brush or a pen deleted while it is selected into a DC, then selected
//! into a second DC: whether that works, what each DC draws, what
//! GetObjectType, GetObject and another DeleteObject say, and what is left
//! after the first and after both DCs selected something else; for an
//! object the first DC had not drawn with yet and for one it had. Then the
//! same delete around SaveDC and RestoreDC in four orders: which state
//! (the live one, the saved one) keeps drawing with the deleted object.
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
const WHITE_BRUSH: i32 = 0;
const BLACK_PEN: i32 = 7;
const OBJ_PEN: u32 = 1;
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

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetStockObject(index: i32) -> HGDIOBJ;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
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
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MoveToEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    fn LineTo(hdc: HDC, x: i32, y: i32) -> BOOL;
}

fn draw(dc: HDC, bits: *mut u32, is_pen: bool) -> u32 {
    // SAFETY: our DC and DIB.
    unsafe {
        fill_bits(bits, 12, 6, 0x0050_5848);
        if is_pen {
            MoveToEx(dc, 0, 3, core::ptr::null_mut());
            LineTo(dc, 12, 3);
        } else {
            PatBlt(dc, 0, 0, 12, 6, PATCOPY);
        }
        GdiFlush();
        px(bits, 12, 5, 3)
    }
}

fn two_dcs(t: &mut Test, is_pen: bool, used_first: bool) {
    // SAFETY: GDI objects of this test.
    unsafe {
        let (dc, bmp, bits) = dib32(12, 6, true);
        let (dc2, bmp2, bits2) = dib32(12, 6, true);
        let kind = if is_pen { OBJ_PEN } else { OBJ_BRUSH };
        let o: HGDIOBJ = if is_pen { CreatePen(0, 1, rgb(0x10, 0x90, 0xD0)) } else { CreateSolidBrush(rgb(0x70, 0x30, 0xA0)) };
        let prev = SelectObject(dc, o);
        let mut first = 0;
        if used_first {
            first = draw(dc, bits, is_pen);
        }
        let r = DeleteObject(o);
        let prev2 = SelectObject(dc2, o);
        say!(
            t,
            "{} used_first={} (drew {:06x}): delete {r}; SelectObject(dc2) null={} is the stock one={}; current(dc2)==o {}; type {}",
            if is_pen { "pen" } else { "brush" },
            used_first as i32,
            first,
            prev2.is_null() as i32,
            (prev2 == GetStockObject(if is_pen { BLACK_PEN } else { WHITE_BRUSH })) as i32,
            (GetCurrentObject(dc2, kind) == o) as i32,
            GetObjectType(o)
        );
        let a = draw(dc2, bits2, is_pen);
        let b = draw(dc, bits, is_pen);
        let c = draw(dc2, bits2, is_pen);
        say!(t, "  dc2 draws {:06x}; dc1 draws {:06x}; dc2 again {:06x}", a, b, c);
        let mut buf = [0u8; 64];
        let g = GetObjectW(o, 64, buf.as_mut_ptr() as *mut c_void);
        let d2 = DeleteObject(o);
        say!(t, "  GetObject {} second delete {} type {}", g, d2, GetObjectType(o));
        SelectObject(dc, prev);
        let ty = GetObjectType(o);
        let cur = (GetCurrentObject(dc2, kind) == o) as i32;
        let a = draw(dc2, bits2, is_pen);
        let again = SelectObject(dc, o).is_null() as i32;
        let b = draw(dc, bits, is_pen);
        say!(t, "  after dc1 deselects: type {ty}; current(dc2)==o {cur}; dc2 draws {:06x}; select into dc1 again null={again}; dc1 draws {:06x}", a, b);
        SelectObject(dc, prev);
        if !prev2.is_null() {
            SelectObject(dc2, prev2);
        }
        let ty = GetObjectType(o);
        let d3 = DeleteObject(o);
        let s3 = SelectObject(dc, o).is_null() as i32;
        say!(t, "  after both deselect: type {ty} delete {d3} select null={s3}");
        SelectObject(dc, prev);
        DeleteDC(dc);
        DeleteDC(dc2);
        DeleteObject(bmp);
        DeleteObject(bmp2);
    }
}

fn savedc(t: &mut Test, is_pen: bool, order: u8) {
    // SAFETY: GDI objects of this test.
    unsafe {
        let (dc, bmp, bits) = dib32(12, 6, true);
        let o: HGDIOBJ = if is_pen { CreatePen(0, 1, rgb(0x10, 0x90, 0xD0)) } else { CreateSolidBrush(rgb(0x70, 0x30, 0xA0)) };
        let what = if is_pen { "pen" } else { "brush" };
        let kind = if is_pen { OBJ_PEN } else { OBJ_BRUSH };
        match order {
            0 => {
                let prev = SelectObject(dc, o);
                let lvl = SaveDC(dc);
                let r = DeleteObject(o);
                let live = draw(dc, bits, is_pen);
                RestoreDC(dc, lvl);
                let after = draw(dc, bits, is_pen);
                say!(t, "{what}: select, SaveDC, delete {r}: the live state draws {:06x}; after RestoreDC {:06x}", live, after);
                SelectObject(dc, prev);
            }
            1 => {
                let lvl = SaveDC(dc);
                SelectObject(dc, o);
                let r = DeleteObject(o);
                let live = draw(dc, bits, is_pen);
                RestoreDC(dc, lvl);
                let after = draw(dc, bits, is_pen);
                say!(t, "{what}: SaveDC, select, delete {r}: draws {:06x}; after RestoreDC (the saved state had the stock object) {:06x}; type {}", live, after, GetObjectType(o));
            }
            2 => {
                let prev = SelectObject(dc, o);
                let lvl = SaveDC(dc);
                SelectObject(dc, prev);
                let r = DeleteObject(o);
                let ty = GetObjectType(o);
                let live = draw(dc, bits, is_pen);
                RestoreDC(dc, lvl);
                let after = draw(dc, bits, is_pen);
                say!(
                    t,
                    "{what}: select, SaveDC, select the stock one, delete {r} (type {ty}): live draws {:06x}; after RestoreDC {:06x}; current==o {}",
                    live,
                    after,
                    (GetCurrentObject(dc, kind) == o) as i32
                );
                SelectObject(dc, prev);
            }
            _ => {
                let prev = SelectObject(dc, o);
                let r = DeleteObject(o);
                let lvl = SaveDC(dc);
                let live = draw(dc, bits, is_pen);
                RestoreDC(dc, lvl);
                let after = draw(dc, bits, is_pen);
                say!(t, "{what}: select, delete {r}, SaveDC: live draws {:06x}; after RestoreDC {:06x}", live, after);
                SelectObject(dc, prev);
            }
        }
        DeleteDC(dc);
        DeleteObject(bmp);
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for is_pen in [false, true] {
        for used in [false, true] {
            two_dcs(t, is_pen, used);
        }
    }
    for is_pen in [false, true] {
        for order in 0..4 {
            savedc(t, is_pen, order);
        }
    }
}
