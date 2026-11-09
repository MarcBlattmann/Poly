//! GetPixel on other DIB formats: a 24-bit DIB (bytes B,G,R per pixel,
//! rows padded to 4 bytes), a 16-bit 5-5-5 DIB (5-bit channels expanded to
//! 8 bits, observed: shifted or replicated), an 8-bit DIB with a colour
//! table (returns the table colour) and a 1-bit DIB with a two-colour
//! table.
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

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
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

fn info(bpp: u16, w: i32, h: i32, colors: &[u32]) -> ([u8; 40 + 4 * 4], usize) {
    let mut b = [0u8; 40 + 16];
    let hdr = BITMAPINFOHEADER {
        size: 40,
        width: w,
        height: -h,
        planes: 1,
        bit_count: bpp,
        compression: BI_RGB,
        size_image: 0,
        x_pels_per_meter: 0,
        y_pels_per_meter: 0,
        clr_used: colors.len() as u32,
        clr_important: 0,
    };
    // SAFETY: BITMAPINFOHEADER is 40 bytes of plain data.
    let raw = unsafe { core::slice::from_raw_parts(&hdr as *const BITMAPINFOHEADER as *const u8, 40) };
    b[..40].copy_from_slice(raw);
    for (i, c) in colors.iter().enumerate() {
        b[40 + 4 * i..44 + 4 * i].copy_from_slice(&c.to_le_bytes());
    }
    (b, 40 + 4 * colors.len())
}

fn make(bpp: u16, w: i32, h: i32, colors: &[u32]) -> (HDC, HBITMAP, *mut u8) {
    let (b, _) = info(bpp, w, h, colors);
    let mut bits: *mut c_void = core::ptr::null_mut();
    // SAFETY: b holds a BITMAPINFO with its colour table.
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        let bmp =
            CreateDIBSection(dc, b.as_ptr() as *const BITMAPINFO, DIB_RGB_COLORS, &mut bits, core::ptr::null_mut(), 0);
        SelectObject(dc, bmp);
        (dc, bmp, bits as *mut u8)
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: writes inside each DIB; GDI calls on our DCs.
    unsafe {
        let (dc, bmp, p) = make(24, 3, 2, &[]);
        let row1 = p.add(12);
        *p = 0x11;
        *p.add(1) = 0x22;
        *p.add(2) = 0x33;
        *row1.add(3) = 0xAA;
        *row1.add(4) = 0xBB;
        *row1.add(5) = 0xCC;
        GdiFlush();
        say!(t, "24-bit (0,0) {:#010x} (1,1) {:#010x}", GetPixel(dc, 0, 0), GetPixel(dc, 1, 1));
        DeleteDC(dc);
        DeleteObject(bmp);

        let (dc, bmp, p) = make(16, 2, 1, &[]);
        let q = p as *mut u16;
        *q = 0x7FFF;
        *q.add(1) = (1 << 10) | (2 << 5) | 3;
        GdiFlush();
        say!(t, "16-bit 0x7fff {:#010x} r1g2b3 {:#010x}", GetPixel(dc, 0, 0), GetPixel(dc, 1, 0));
        DeleteDC(dc);
        DeleteObject(bmp);

        let (dc, bmp, p) = make(8, 4, 1, &[0x0000_0000, 0x00FF_0000, 0x0000_FF00, 0x0012_3456]);
        *p = 3;
        *p.add(1) = 1;
        *p.add(2) = 2;
        *p.add(3) = 9;
        GdiFlush();
        say!(
            t,
            "8-bit index 3 {:#010x} index 1 {:#010x} index 2 {:#010x} index 9 (past the table) {:#010x}",
            GetPixel(dc, 0, 0),
            GetPixel(dc, 1, 0),
            GetPixel(dc, 2, 0),
            GetPixel(dc, 3, 0)
        );
        DeleteDC(dc);
        DeleteObject(bmp);

        let (dc, bmp, p) = make(1, 8, 1, &[0x0000_00FF, 0x00FF_FF00]);
        *p = 0b1010_0000;
        GdiFlush();
        say!(
            t,
            "1-bit pixels 0..3: {:#010x} {:#010x} {:#010x} {:#010x}",
            GetPixel(dc, 0, 0),
            GetPixel(dc, 1, 0),
            GetPixel(dc, 2, 0),
            GetPixel(dc, 3, 0)
        );
        DeleteDC(dc);
        DeleteObject(bmp);
    }
}
