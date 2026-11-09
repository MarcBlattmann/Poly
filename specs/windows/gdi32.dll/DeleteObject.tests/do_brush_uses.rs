//! What a brush that was deleted gets used for afterwards: selecting its
//! handle again, FillRect with it, a DC state restored with RestoreDC that
//! had it selected, a brush selected into two DCs, a new brush made after
//! the deletion, and stock objects (DeleteObject succeeds but changes
//! nothing).
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

const PATCOPY: u32 = 0x00F0_0021;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const WHITE_BRUSH: i32 = 0;
const BLACK_BRUSH: i32 = 4;
const NULL_BRUSH: i32 = 5;
const BLACK_PEN: i32 = 7;
const DEFAULT_PALETTE: i32 = 15;
const DC_BRUSH: i32 = 18;
const DC_PEN: i32 = 19;
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
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
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn GetCurrentObject(hdc: HDC, kind: u32) -> HGDIOBJ;
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

fn who(h: HGDIOBJ) -> &'static str {
    const NAMES: [&str; 20] = [
        "stock 0", "stock 1", "stock 2", "stock 3", "stock 4", "stock 5", "stock 6", "stock 7", "stock 8", "stock 9",
        "stock 10", "stock 11", "stock 12", "stock 13", "stock 14", "stock 15", "stock 16", "stock 17", "stock 18",
        "stock 19",
    ];
    if h.is_null() {
        return "null";
    }
    for i in 0..20 {
        // SAFETY: GetStockObject only reads its index.
        if unsafe { GetStockObject(i) } == h {
            return NAMES[i as usize];
        }
    }
    "other"
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: GDI objects of this test; every DC and object is released.
    unsafe {
        let (dc, bmp, bits) = dib32(8, 8, true);
        let (dc2, bmp2, bits2) = dib32(8, 8, true);
        let rc = RECT { left: 0, top: 0, right: 8, bottom: 8 };
        let pixel = |bits: *mut u32| px(bits, 8, 2, 2);
        let blank = |bits: *mut u32| fill_bits(bits, 8, 8, 0x0010_2030);

        // Delete first, select afterwards.
        let b1 = CreateSolidBrush(rgb(0xC0, 0x40, 0x20));
        DeleteObject(b1);
        set_error(0);
        let prev = SelectObject(dc, b1);
        say!(t, "SelectObject(deleted brush): returned {} error {}", who(prev), last_error());
        say!(t, "  current brush now {}", who(GetCurrentObject(dc, OBJ_BRUSH)));
        blank(bits);
        PatBlt(dc, 0, 0, 4, 4, PATCOPY);
        GdiFlush();
        say!(t, "  PatBlt: pixel {:06x}", pixel(bits));

        // FillRect with a deleted brush.
        blank(bits);
        set_error(0);
        let n = FillRect(dc, &rc, b1);
        GdiFlush();
        say!(t, "FillRect(deleted brush): {n} error {} pixel {:06x}", last_error(), pixel(bits));

        // A brush selected into two DCs, then deleted.
        let b2 = CreateSolidBrush(rgb(0x30, 0x90, 0x50));
        let o1 = SelectObject(dc, b2);
        let o2 = SelectObject(dc2, b2);
        say!(t, "two DCs: previous brushes {} and {}", who(o1), who(o2));
        set_error(0);
        say!(t, "  delete: {} error {}", DeleteObject(b2), last_error());
        blank(bits);
        blank(bits2);
        PatBlt(dc, 0, 0, 4, 4, PATCOPY);
        PatBlt(dc2, 0, 0, 4, 4, PATCOPY);
        GdiFlush();
        say!(t, "  PatBlt in both: {:06x} and {:06x}", pixel(bits), pixel(bits2));
        say!(t, "  current brushes {} and {}", who(GetCurrentObject(dc, OBJ_BRUSH)), who(GetCurrentObject(dc2, OBJ_BRUSH)));

        // SaveDC / RestoreDC across the deletion.
        let b3 = CreateSolidBrush(rgb(0x10, 0xA0, 0xF0));
        SelectObject(dc, b3);
        let saved = SaveDC(dc);
        let other = CreateSolidBrush(rgb(0x01, 0x01, 0x01));
        SelectObject(dc, other);
        DeleteObject(b3);
        let ok = RestoreDC(dc, saved);
        say!(t, "RestoreDC with a deleted brush saved: {ok}; current brush {}", who(GetCurrentObject(dc, OBJ_BRUSH)));
        blank(bits);
        PatBlt(dc, 0, 0, 4, 4, PATCOPY);
        GdiFlush();
        say!(t, "  PatBlt: pixel {:06x}", pixel(bits));
        DeleteObject(other);

        // A new brush after a deletion works as usual.
        let b4 = CreateSolidBrush(rgb(0x77, 0x66, 0x55));
        let was = SelectObject(dc, b4);
        say!(t, "new brush selected over a deleted one: previous {}", who(was));
        blank(bits);
        PatBlt(dc, 0, 0, 4, 4, PATCOPY);
        GdiFlush();
        say!(t, "  PatBlt: pixel {:06x}", pixel(bits));
        say!(t, "  type of the deleted b3: {}, of b4: {}", GetObjectType(b3), GetObjectType(b4));
        SelectObject(dc, GetStockObject(WHITE_BRUSH));
        DeleteObject(b4);

        // Stock objects.
        for (label, idx) in [("NULL_BRUSH", NULL_BRUSH), ("DC_BRUSH", DC_BRUSH), ("BLACK_PEN", BLACK_PEN), ("DC_PEN", DC_PEN), ("DEFAULT_PALETTE", 15)] {
            let s = GetStockObject(idx);
            set_error(0);
            let r = DeleteObject(s);
            say!(t, "{label}: DeleteObject {r} error {} type {}, GetStockObject again same: {}", last_error(), GetObjectType(s), (GetStockObject(idx) == s) as i32);
        }
        // A stock brush selected into the DC and deleted: still drawing.
        SelectObject(dc, GetStockObject(BLACK_BRUSH));
        DeleteObject(GetStockObject(BLACK_BRUSH));
        blank(bits);
        PatBlt(dc, 0, 0, 4, 4, PATCOPY);
        GdiFlush();
        say!(t, "stock black brush selected and deleted: PatBlt pixel {:06x}", pixel(bits));

        DeleteDC(dc);
        DeleteDC(dc2);
        DeleteObject(bmp);
        DeleteObject(bmp2);
    }
}
