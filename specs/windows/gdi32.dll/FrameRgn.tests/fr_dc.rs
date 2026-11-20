//! FrameRgn and the device context: window and viewport origins, a clip region, mapping modes,
//! a world transform, brushes other than a solid colour, the raster operation, and the current
//! brush staying selected. Each line is the bounding box and count of the painted pixels.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the public SDK headers
// wingdi.h, windef.h and winerror.h (constants, layouts); the helpers are this test's own.

// ---- shared helpers of the GDI tests (this kit's own code) ----
use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type HRGN = *mut c_void;
type HBRUSH = *mut c_void;
type HPEN = *mut c_void;
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

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct POINT {
    x: i32,
    y: i32,
}

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy)]
struct XFORM {
    m11: f32,
    m12: f32,
    m21: f32,
    m22: f32,
    dx: f32,
    dy: f32,
}

const WHITE_BRUSH: i32 = 0;
const BLACK_BRUSH: i32 = 4;
const NULL_BRUSH: i32 = 5;
const BLACK_PEN: i32 = 7;
const NULL_PEN: i32 = 8;
const PS_SOLID: i32 = 0;
const PS_DASH: i32 = 1;
const PS_INSIDEFRAME: i32 = 6;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const GM_ADVANCED: i32 = 2;

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
    fn push(&mut self, c: u8) {
        if self.n < N {
            self.b[self.n] = c;
            self.n += 1;
        }
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            self.push(c);
        }
        Ok(())
    }
}

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetStockObject(i: i32) -> HGDIOBJ;
    fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
    fn CreatePen(style: i32, width: i32, color: COLORREF) -> HPEN;
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
    fn RoundRect(hdc: HDC, l: i32, t: i32, r: i32, b: i32, w: i32, h: i32) -> BOOL;
    fn Rectangle(hdc: HDC, l: i32, t: i32, r: i32, b: i32) -> BOOL;
    fn FrameRgn(hdc: HDC, rgn: HRGN, brush: HBRUSH, w: i32, h: i32) -> BOOL;
    fn FillRgn(hdc: HDC, rgn: HRGN, brush: HBRUSH) -> BOOL;
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn CreateEllipticRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn CreateRoundRectRgn(l: i32, t: i32, r: i32, b: i32, w: i32, h: i32) -> HRGN;
    fn CombineRgn(dst: HRGN, a: HRGN, b: HRGN, mode: i32) -> i32;
    fn OffsetRgn(rgn: HRGN, dx: i32, dy: i32) -> i32;
    fn SelectClipRgn(hdc: HDC, rgn: HRGN) -> i32;
    fn OffsetWindowOrgEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    fn SetWindowOrgEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    fn GetWindowOrgEx(hdc: HDC, p: *mut POINT) -> BOOL;
    fn SetViewportOrgEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    fn GetViewportOrgEx(hdc: HDC, p: *mut POINT) -> BOOL;
    fn SetWindowExtEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    fn SetViewportExtEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    fn SetMapMode(hdc: HDC, mode: i32) -> i32;
    fn GetMapMode(hdc: HDC) -> i32;
    fn LPtoDP(hdc: HDC, p: *mut POINT, n: i32) -> BOOL;
    fn DPtoLP(hdc: HDC, p: *mut POINT, n: i32) -> BOOL;
    fn SaveDC(hdc: HDC) -> i32;
    fn RestoreDC(hdc: HDC, saved: i32) -> BOOL;
    fn SetGraphicsMode(hdc: HDC, mode: i32) -> i32;
    fn SetWorldTransform(hdc: HDC, xf: *const XFORM) -> BOOL;
    fn MoveToEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    fn GetCurrentPositionEx(hdc: HDC, p: *mut POINT) -> BOOL;
    fn SetROP2(hdc: HDC, rop: i32) -> i32;
    fn GetROP2(hdc: HDC) -> i32;
    fn SetBrushOrgEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    fn CreateHatchBrush(style: i32, color: COLORREF) -> HBRUSH;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// COLORREFs (0x00BBGGRR).
const RED: COLORREF = 0x0000FF;
const GREEN: COLORREF = 0x00FF00;
const BLUE: COLORREF = 0xFF0000;
const WHITE: COLORREF = 0xFFFFFF;
const BLACK: COLORREF = 0;

/// A memory DC with a 32-bit top-down DIB section of w*h pixels, filled with `bg`
/// (a COLORREF). Pixels read back as 0x00RRGGBB.
struct Bmp {
    dc: HDC,
    bits: *mut u32,
    w: i32,
    h: i32,
}

/// The pixel (0x00RRGGBB) a COLORREF gives.
fn pix(c: COLORREF) -> u32 {
    (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF
}

impl Bmp {
    fn new(w: i32, h: i32, bg: COLORREF) -> Bmp {
        let info = BITMAPINFO {
            header: BITMAPINFOHEADER {
                size: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
                width: w,
                height: -h,
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
        // SAFETY: a filled BITMAPINFO; bits receives the pixel pointer; the DIB is selected into
        // the DC and lives as long as the program.
        unsafe {
            let dc = CreateCompatibleDC(core::ptr::null_mut());
            let bmp = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, core::ptr::null_mut(), 0);
            SelectObject(dc, bmp);
            let b = Bmp { dc, bits: bits as *mut u32, w, h };
            for i in 0..(w * h) as usize {
                *b.bits.add(i) = pix(bg);
            }
            b
        }
    }
    /// The pixel as 0x00RRGGBB, or 0xFFFF_FFFF outside the bitmap.
    fn px(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return 0xFFFF_FFFF;
        }
        // SAFETY: inside the bitmap's w*h pixels.
        unsafe {
            GdiFlush();
            *self.bits.add((y * self.w + x) as usize) & 0x00FF_FFFF
        }
    }
    /// The raw 32-bit pixel (alpha byte included), 0 outside.
    fn raw(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return 0;
        }
        // SAFETY: inside the bitmap's w*h pixels.
        unsafe {
            GdiFlush();
            *self.bits.add((y * self.w + x) as usize)
        }
    }
    fn set_raw(&self, x: i32, y: i32, v: u32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        // SAFETY: inside the bitmap's w*h pixels.
        unsafe {
            GdiFlush();
            *self.bits.add((y * self.w + x) as usize) = v;
        }
    }
    fn count(&self, color: u32) -> i32 {
        let mut n = 0;
        for y in 0..self.h {
            for x in 0..self.w {
                if self.px(x, y) == color {
                    n += 1;
                }
            }
        }
        n
    }
    /// Bounding box of the pixels of `color`: (l, t, r, b) inclusive, or None.
    fn bbox(&self, color: u32) -> Option<(i32, i32, i32, i32)> {
        let mut r: Option<(i32, i32, i32, i32)> = None;
        for y in 0..self.h {
            for x in 0..self.w {
                if self.px(x, y) == color {
                    r = Some(match r {
                        None => (x, y, x, y),
                        Some((l, t, rr, b)) => (l.min(x), t.min(y), rr.max(x), b.max(y)),
                    });
                }
            }
        }
        r
    }
}

/// One character per pixel colour: '.' white, '#' black, 'R' red, 'G' green, 'B' blue, 'y' yellow,
/// 'c' cyan, 'm' magenta, 'g' grey (0x808080), '?' anything else (printed on a line of its own).
/// These are pixel values 0x00RRGGBB.
fn pixch(c: u32) -> u8 {
    match c {
        0xFFFFFF => b'.',
        0x000000 => b'#',
        0xFF0000 => b'R',
        0x00FF00 => b'G',
        0x0000FF => b'B',
        0xFFFF00 => b'y',
        0x00FFFF => b'c',
        0xFF00FF => b'm',
        0x808080 => b'g',
        _ => b'?',
    }
}

/// Prints the pixels x0..=x1, y0..=y1 as text rows `label yyy|chars`.
fn dump(t: &mut Test, bmp: &Bmp, label: &str, x0: i32, y0: i32, x1: i32, y1: i32) {
    for y in y0..=y1 {
        let mut row = Buf::<200>::new();
        for x in x0..=x1 {
            let c = bmp.px(x, y);
            row.push(pixch(c));
            if pixch(c) == b'?' {
                say!(t, "{} odd pixel ({},{}) = {:06x}", label, x, y, c);
            }
        }
        say!(t, "{} {:3}|{}", label, y, row.as_str());
    }
}

/// Prints the pixels of pixel value `color` as bands relative to (ox, oy): `top+rows:left-right`
/// (right exclusive), runs of consecutive rows with the same run merged.
fn bands(t: &mut Test, bmp: &Bmp, label: &str, color: u32, ox: i32, oy: i32) {
    let mut line = Buf::<400>::new();
    let _ = write!(line, "{}:", label);
    let mut cur: Option<(i32, i32, i32, i32)> = None; // top, rows, left, right
    for y in 0..bmp.h {
        let mut x = 0;
        while x < bmp.w {
            if bmp.px(x, y) == color {
                let s = x;
                while x < bmp.w && bmp.px(x, y) == color {
                    x += 1;
                }
                let band = (y - oy, 1, s - ox, x - ox);
                match cur {
                    Some(c) if c.0 + c.1 == band.0 && c.2 == band.2 && c.3 == band.3 => {
                        cur = Some((c.0, c.1 + 1, c.2, c.3));
                    }
                    Some(c) => {
                        push_band(t, &mut line, c);
                        cur = Some(band);
                    }
                    None => cur = Some(band),
                }
            } else {
                x += 1;
            }
        }
    }
    if let Some(c) = cur {
        push_band(t, &mut line, c);
    }
    t.line(line.as_str());
}

fn push_band(t: &mut Test, line: &mut Buf<400>, b: (i32, i32, i32, i32)) {
    let mut item = Buf::<40>::new();
    let _ = write!(item, " {}+{}:{}-{}", b.0, b.1, b.2, b.3);
    if line.n + item.n > 190 {
        t.line(line.as_str());
        line.n = 0;
        let _ = write!(line, "  ..");
    }
    let _ = write!(line, "{}", item.as_str());
}

fn bbox_line(t: &mut Test, label: &str, bmp: &Bmp, color: u32) {
    match bmp.bbox(color) {
        Some((l, tt, r, b)) => say!(t, "{} bbox ({},{})-({},{}) count {}", label, l, tt, r, b, bmp.count(color)),
        None => say!(t, "{} bbox none", label),
    }
}

/// Selects a pen: `kind` -1 is the stock null pen, otherwise CreatePen(kind, width, color).
fn sel_pen(dc: HDC, kind: i32, width: i32, color: COLORREF) {
    // SAFETY: GDI calls on our own DC.
    unsafe {
        if kind == -1 {
            SelectObject(dc, GetStockObject(NULL_PEN));
        } else {
            SelectObject(dc, CreatePen(kind, width, color));
        }
    }
}

/// Selects a solid brush of `color`, or the stock null brush for None.
fn sel_brush(dc: HDC, color: Option<COLORREF>) {
    // SAFETY: GDI calls on our own DC.
    unsafe {
        match color {
            None => {
                SelectObject(dc, GetStockObject(NULL_BRUSH));
            }
            Some(c) => {
                SelectObject(dc, CreateSolidBrush(c));
            }
        }
    }
}
// ---- end of the shared helpers ----

poly_wtest::main!(test);

fn show(t: &mut Test, name: &str, b: &Bmp, rv: i32, color: u32) {
    say!(t, "{}: ret {}", name, rv);
    bbox_line(t, "  painted", b, color);
}

fn test(t: &mut Test) {
    // SAFETY: GDI calls on our own memory DCs and objects.
    unsafe {
        let brush = CreateSolidBrush(BLUE);
        let rgn = CreateRectRgn(8, 6, 28, 18);

        let b = Bmp::new(50, 36, WHITE);
        let rv = FrameRgn(b.dc, rgn, brush, 2, 2);
        show(t, "plain", &b, rv, pix(BLUE));

        let b = Bmp::new(50, 36, WHITE);
        SetWindowOrgEx(b.dc, -10, -5, core::ptr::null_mut());
        let rv = FrameRgn(b.dc, rgn, brush, 2, 2);
        show(t, "window origin (-10,-5)", &b, rv, pix(BLUE));

        let b = Bmp::new(50, 36, WHITE);
        SetWindowOrgEx(b.dc, 3, 2, core::ptr::null_mut());
        OffsetWindowOrgEx(b.dc, 4, 1, core::ptr::null_mut());
        let rv = FrameRgn(b.dc, rgn, brush, 2, 2);
        show(t, "window origin (3,2)+(4,1)", &b, rv, pix(BLUE));

        let b = Bmp::new(50, 36, WHITE);
        SetViewportOrgEx(b.dc, 9, 4, core::ptr::null_mut());
        let rv = FrameRgn(b.dc, rgn, brush, 2, 2);
        show(t, "viewport origin (9,4)", &b, rv, pix(BLUE));

        let b = Bmp::new(50, 36, WHITE);
        SelectClipRgn(b.dc, CreateRectRgn(12, 4, 24, 14));
        let rv = FrameRgn(b.dc, rgn, brush, 2, 2);
        show(t, "clip (12,4,24,14)", &b, rv, pix(BLUE));

        // The border width is in logical units: it follows the mapping mode.
        let b = Bmp::new(50, 36, WHITE);
        SetMapMode(b.dc, 8);
        SetWindowExtEx(b.dc, 1, 1, core::ptr::null_mut());
        SetViewportExtEx(b.dc, 2, 2, core::ptr::null_mut());
        let rv = FrameRgn(b.dc, rgn, brush, 1, 1);
        show(t, "anisotropic 2x, border 1x1 (region in logical units)", &b, rv, pix(BLUE));

        let b = Bmp::new(50, 36, WHITE);
        SetMapMode(b.dc, 8);
        SetWindowExtEx(b.dc, 2, 1, core::ptr::null_mut());
        SetViewportExtEx(b.dc, 1, 1, core::ptr::null_mut());
        let rv = FrameRgn(b.dc, rgn, brush, 4, 3);
        show(t, "anisotropic x/2, border 4x3", &b, rv, pix(BLUE));

        let b = Bmp::new(50, 36, WHITE);
        SetGraphicsMode(b.dc, GM_ADVANCED);
        let xf = XFORM { m11: 2.0, m12: 0.0, m21: 0.0, m22: 2.0, dx: 0.0, dy: 0.0 };
        SetWorldTransform(b.dc, &xf);
        let rv = FrameRgn(b.dc, rgn, brush, 1, 1);
        show(t, "world transform 2x, border 1x1", &b, rv, pix(BLUE));

        // The brush passed in is used, not the one selected into the DC.
        let b = Bmp::new(50, 36, WHITE);
        SelectObject(b.dc, CreateSolidBrush(GREEN));
        let rv = FrameRgn(b.dc, rgn, brush, 2, 2);
        show(t, "selected brush green, brush blue", &b, rv, pix(BLUE));
        bbox_line(t, "  green", &b, pix(GREEN));

        // FrameRgn draws on a DC with a fresh default (white) brush and a NULL brush selected.
        let b = Bmp::new(50, 36, WHITE);
        SelectObject(b.dc, GetStockObject(NULL_BRUSH));
        let rv = FrameRgn(b.dc, rgn, brush, 2, 2);
        show(t, "null brush selected", &b, rv, pix(BLUE));
    }
}
