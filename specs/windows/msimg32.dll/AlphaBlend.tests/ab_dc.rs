//! AlphaBlend (msimg32) and the device contexts: the destination's window origin and clip
//! region, a mapping mode that scales, source bitmaps that are bottom-up, destinations and
//! sources of 24 and 16 bits per pixel, a source with per-pixel alpha onto a 24-bit destination,
//! and a source bitmap that is not a DIB section.
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
    /// `raw` without GdiFlush (for loops over many pixels; flush before and after the loop).
    fn raw_nf(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return 0;
        }
        // SAFETY: inside the bitmap's w*h pixels.
        unsafe { *self.bits.add((y * self.w + x) as usize) }
    }
    /// `set_raw` without GdiFlush.
    fn set_nf(&self, x: i32, y: i32, v: u32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        // SAFETY: inside the bitmap's w*h pixels.
        unsafe { *self.bits.add((y * self.w + x) as usize) = v };
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
// ---- helpers of the AlphaBlend tests (this kit's own code; follows lib/gdi_prelude.rs) ----

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy)]
struct BLENDFUNCTION {
    blend_op: u8,
    blend_flags: u8,
    source_constant_alpha: u8,
    alpha_format: u8,
}

const AC_SRC_OVER: u8 = 0;
const AC_SRC_ALPHA: u8 = 1;

#[link(name = "msimg32", kind = "raw-dylib")]
unsafe extern "system" {
    fn AlphaBlend(
        hdc_dest: HDC,
        x_dest: i32,
        y_dest: i32,
        w_dest: i32,
        h_dest: i32,
        hdc_src: HDC,
        x_src: i32,
        y_src: i32,
        w_src: i32,
        h_src: i32,
        bf: BLENDFUNCTION,
    ) -> BOOL;
}

/// AlphaBlend of the whole of `src` onto (dx, dy) of `dst`, same size.
fn blend(dst: &Bmp, dx: i32, dy: i32, src: &Bmp, sca: u8, af: u8) -> (i32, u32) {
    let bf = BLENDFUNCTION { blend_op: AC_SRC_OVER, blend_flags: 0, source_constant_alpha: sca, alpha_format: af };
    // SAFETY: GDI calls on our own memory DCs.
    unsafe {
        GdiFlush();
        SetLastError(0x5A5A);
        let r = AlphaBlend(dst.dc, dx, dy, src.w, src.h, src.dc, 0, 0, src.w, src.h, bf);
        let e = GetLastError();
        GdiFlush();
        (r, e)
    }
}

/// FNV-1a over u32 words.
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn add(&mut self, v: u32) {
        for i in 0..4 {
            self.0 ^= ((v >> (8 * i)) & 0xFF) as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// Integer helpers of the formulas the sheet states: (a * b + 127) / 255.
fn mul255(a: u32, b: u32) -> u32 {
    (a * b + 127) / 255
}
// ---- end of the AlphaBlend helpers ----

poly_wtest::main!(test);

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateCompatibleBitmap(hdc: HDC, w: i32, h: i32) -> HBITMAP;
    fn GetDIBits(
        hdc: HDC,
        bmp: HBITMAP,
        start: u32,
        lines: u32,
        bits: *mut c_void,
        info: *mut BITMAPINFO,
        usage: u32,
    ) -> i32;
}

/// A DIB section of `bits` bits per pixel (16: 5-5-5, 24, 32), top-down or bottom-up.
struct Dib {
    dc: HDC,
    data: *mut u8,
    stride: i32,
    w: i32,
    h: i32,
    bpp: i32,
    top_down: bool,
}

impl Dib {
    fn new(w: i32, h: i32, bpp: i32, top_down: bool) -> Dib {
        let info = BITMAPINFO {
            header: BITMAPINFOHEADER {
                size: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
                width: w,
                height: if top_down { -h } else { h },
                planes: 1,
                bit_count: bpp as u16,
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
            let stride = ((w * bpp + 31) / 32) * 4;
            Dib { dc, data: bits as *mut u8, stride, w, h, bpp, top_down }
        }
    }
    /// The byte offset of pixel (x, y) as the user sees it (y = 0 is the top row).
    fn off(&self, x: i32, y: i32) -> isize {
        let row = if self.top_down { y } else { self.h - 1 - y };
        (row * self.stride + x * self.bpp / 8) as isize
    }
    /// Sets a pixel from 8-bit r, g, b (alpha for 32 bits).
    fn set(&self, x: i32, y: i32, a: u8, r: u8, g: u8, b: u8) {
        // SAFETY: inside the DIB's memory.
        unsafe {
            GdiFlush();
            let p = self.data.offset(self.off(x, y));
            match self.bpp {
                32 => {
                    *p = b;
                    *p.add(1) = g;
                    *p.add(2) = r;
                    *p.add(3) = a;
                }
                24 => {
                    *p = b;
                    *p.add(1) = g;
                    *p.add(2) = r;
                }
                _ => {
                    let v: u16 = ((r as u16 >> 3) << 10) | ((g as u16 >> 3) << 5) | (b as u16 >> 3);
                    *(p as *mut u16) = v;
                }
            }
        }
    }
    /// The pixel as text: 32 bits `aarrggbb`, 24 bits `rrggbb`, 16 bits `xxxx` (the raw word).
    fn text(&self, x: i32, y: i32, out: &mut Buf<300>) {
        // SAFETY: inside the DIB's memory.
        unsafe {
            GdiFlush();
            let p = self.data.offset(self.off(x, y));
            match self.bpp {
                32 => {
                    let _ = write!(out, " {:08x}", *(p as *mut u32));
                }
                24 => {
                    let _ = write!(out, " {:02x}{:02x}{:02x}", *p.add(2), *p.add(1), *p);
                }
                _ => {
                    let _ = write!(out, " {:04x}", *(p as *mut u16));
                }
            }
        }
    }
}

fn row_text(t: &mut Test, label: &str, d: &Dib, y: i32) {
    let mut line = Buf::<300>::new();
    for x in 0..d.w {
        d.text(x, y, &mut line);
    }
    say!(t, "{} row {}:{}", label, y, line.as_str());
}

fn fill(d: &Dib, a: u8, r: u8, g: u8, b: u8) {
    for y in 0..d.h {
        for x in 0..d.w {
            d.set(x, y, a, r, g, b);
        }
    }
}

fn blend_dib(dst: &Dib, dx: i32, dy: i32, w: i32, h: i32, src: &Dib, sca: u8, af: u8) -> (i32, u32) {
    let bf = BLENDFUNCTION { blend_op: AC_SRC_OVER, blend_flags: 0, source_constant_alpha: sca, alpha_format: af };
    // SAFETY: GDI calls on our own memory DCs.
    unsafe {
        GdiFlush();
        SetLastError(0x5A5A);
        let r = AlphaBlend(dst.dc, dx, dy, w, h, src.dc, 0, 0, src.w, src.h, bf);
        let e = GetLastError();
        GdiFlush();
        (r, e)
    }
}

fn test(t: &mut Test) {
    // colour depths: source 32 (no alpha use), 24 or 16 bits onto destinations of 32, 24, 16 bits
    for &sbpp in &[32, 24, 16] {
        for &dbpp in &[32, 24, 16] {
            let src = Dib::new(4, 1, sbpp, true);
            let dst = Dib::new(4, 1, dbpp, true);
            for x in 0..4 {
                src.set(x, 0, 0, (x * 80) as u8, (255 - x * 80) as u8, (x * 40 + 8) as u8);
                dst.set(x, 0, 0, 200, (x * 60) as u8, 16);
            }
            let (r, e) = blend_dib(&dst, 0, 0, 4, 1, &src, 100, 0);
            say!(t, "source {} bits onto {} bits, sca 100: ret {} err {:#x}", sbpp, dbpp, r, e);
            row_text(t, "  src", &src, 0);
            row_text(t, "  dst", &dst, 0);
        }
    }
    // per-pixel alpha from a 32-bit source onto 32, 24 and 16 bit destinations; sources of 24 bits
    for &dbpp in &[32, 24, 16] {
        let src = Dib::new(4, 1, 32, true);
        let dst = Dib::new(4, 1, dbpp, true);
        for x in 0..4 {
            let a = (x * 85) as u8;
            src.set(x, 0, a, a / 2, a / 4, 0);
            dst.set(x, 0, 255, 100, 150, 200);
        }
        let (r, e) = blend_dib(&dst, 0, 0, 4, 1, &src, 255, AC_SRC_ALPHA);
        say!(t, "per-pixel alpha, 32 bit source onto {} bits: ret {} err {:#x}", dbpp, r, e);
        row_text(t, "  dst", &dst, 0);
    }
    for &sbpp in &[24, 16] {
        let src = Dib::new(4, 1, sbpp, true);
        let dst = Dib::new(4, 1, 32, true);
        fill(&src, 0, 10, 20, 30);
        fill(&dst, 255, 100, 150, 200);
        let (r, e) = blend_dib(&dst, 0, 0, 4, 1, &src, 255, AC_SRC_ALPHA);
        say!(t, "per-pixel alpha with a {} bit source: ret {} err {:#x}", sbpp, r, e);
        row_text(t, "  dst", &dst, 0);
    }
    // bottom-up and top-down sources and destinations
    for &(sdown, ddown) in &[(true, true), (false, true), (true, false), (false, false)] {
        let src = Dib::new(2, 2, 32, sdown);
        let dst = Dib::new(2, 2, 32, ddown);
        // row 0 (top) red, row 1 green, as the user sees them
        for x in 0..2 {
            src.set(x, 0, 0, 255, 0, 0);
            src.set(x, 1, 0, 0, 255, 0);
            dst.set(x, 0, 0, 0, 0, 0);
            dst.set(x, 1, 0, 0, 0, 0);
        }
        let (r, _) = blend_dib(&dst, 0, 0, 2, 2, &src, 255, 0);
        say!(t, "source top-down {} destination top-down {}: ret {}", sdown, ddown, r);
        row_text(t, "  dst", &dst, 0);
        row_text(t, "  dst", &dst, 1);
    }
    // a source that is a device dependent bitmap
    // SAFETY: GDI calls on our own memory DCs.
    unsafe {
        let screen = CreateCompatibleDC(core::ptr::null_mut());
        let ddb = CreateCompatibleBitmap(screen, 4, 4);
        let sdc = CreateCompatibleDC(core::ptr::null_mut());
        SelectObject(sdc, ddb);
        let dst = Dib::new(4, 4, 32, true);
        fill(&dst, 0, 0, 0, 0);
        let bf = BLENDFUNCTION { blend_op: AC_SRC_OVER, blend_flags: 0, source_constant_alpha: 128, alpha_format: 0 };
        SetLastError(0x5A5A);
        let r = AlphaBlend(dst.dc, 0, 0, 4, 4, sdc, 0, 0, 4, 4, bf);
        say!(
            t,
            "source is a compatible bitmap made from a memory DC (monochrome): ret {} err {:#x}",
            r,
            GetLastError()
        );
    }
}
