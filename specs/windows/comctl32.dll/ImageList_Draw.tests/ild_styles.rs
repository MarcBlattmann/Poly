//! ImageList_Draw: the styles ILD_NORMAL, ILD_TRANSPARENT, ILD_MASK,
//! ILD_IMAGE, ILD_BLEND25, ILD_BLEND50 (ILD_SELECTED), ILD_FOCUS drawn into a
//! 32-bit DIB over a magenta background, with a list that has a mask and a
//! list that has none, pixels sampled at fixed places (no fonts, no screen);
//! the list's background colour; the return value.
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

/// The bounding box and count of pixels that are not `bg`.
fn ink(bits: *mut u32, w: i32, h: i32, bg: u32) -> (i32, i32, i32, i32, u32) {
    let (mut l, mut tp, mut r, mut b, mut n) = (i32::MAX, i32::MAX, -1, -1, 0);
    for y in 0..h {
        for x in 0..w {
            if px(bits, w, x, y) != bg {
                l = l.min(x);
                tp = tp.min(y);
                r = r.max(x + 1);
                b = b.max(y + 1);
                n += 1;
            }
        }
    }
    if n == 0 { (0, 0, 0, 0, 0) } else { (l, tp, r, b, n) }
}

type HIMAGELIST = *mut c_void;
const ILC_MASK: u32 = 0x1;
const ILC_COLOR24: u32 = 0x18;
const ILC_COLOR32: u32 = 0x20;
const ILD_NORMAL: u32 = 0;
const ILD_TRANSPARENT: u32 = 1;
const ILD_MASK: u32 = 0x10;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn SetBkColor(hdc: HDC, color: COLORREF) -> COLORREF;
    fn GetPixel(hdc: HDC, x: i32, y: i32) -> COLORREF;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const c_void) -> HBITMAP;
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
}

#[link(name = "comctl32", kind = "raw-dylib")]
unsafe extern "system" {
    fn InitCommonControls();
    fn ImageList_Create(cx: i32, cy: i32, flags: u32, initial: i32, grow: i32) -> HIMAGELIST;
    fn ImageList_Destroy(il: HIMAGELIST) -> BOOL;
    fn ImageList_GetImageCount(il: HIMAGELIST) -> i32;
    fn ImageList_Add(il: HIMAGELIST, image: HBITMAP, mask: HBITMAP) -> i32;
    fn ImageList_GetBkColor(il: HIMAGELIST) -> COLORREF;
    fn ImageList_SetBkColor(il: HIMAGELIST, color: COLORREF) -> COLORREF;
    fn ImageList_Draw(il: HIMAGELIST, index: i32, hdc: HDC, x: i32, y: i32, style: u32) -> BOOL;
}

/// Call `$e` with the last error set to a marker; print "label: result err=N"
/// (err -1: the call did not touch the last error). Gives the result.
macro_rules! call {
    ($t:expr, $label:expr, $e:expr) => {{
        set_error(0xDEAD);
        let r = ($e) as i64;
        let e = last_error();
        if stringify!($e).contains("is_null") && r == 0 {
            say!($t, "{}: {}", $label, r);
        } else if r == 0 || r == -1 || stringify!($e).contains("is_null") {
            say!($t, "{}: {} err={}", $label, r, if e == 0xDEAD { -1 } else { e as i64 });
        } else {
            say!($t, "{}: {}", $label, r);
        }
        r
    }};
}

/// MAKEINTRESOURCE
fn mir(id: usize) -> *const u16 {
    id as *const u16
}

/// A menu handle made up by the test (never a real one).
const BOGUS_MENU: usize = 0x0bad_0bad;

/// MAKEINTRESOURCE for the ANSI functions
fn mira(id: usize) -> *const u8 {
    id as *const u8
}

// ---- image list helpers of these tests ----

const RED: u32 = 0x00FF_0000;
const GREEN: u32 = 0x0000_FF00;
const BLUE: u32 = 0x0000_00FF;
const YELLOW: u32 = 0x00FF_FF00;
const MAGENTA: u32 = 0x00FF_00FF;
const ILD_BLEND25: u32 = 0x2;
const ILD_BLEND50: u32 = 0x4;
const ILD_SELECTED: u32 = 0x4;
const ILD_FOCUS: u32 = 0x2;
const ILD_IMAGE: u32 = 0x20;
const ILD_ROP: u32 = 0x40;
const ILD_OVERLAYMASK: u32 = 0xF00;
const ILD_PRESERVEALPHA: u32 = 0x1000;
const ILCF_MOVE: u32 = 0;
const ILCF_SWAP: u32 = 1;

/// A 32-bit DIB section w*n x h, image k in the colour colors[k] (alpha 0xFF when `alpha`).
fn strip(w: i32, h: i32, colors: &[u32], alpha: bool) -> HBITMAP {
    // SAFETY: a DIB section filled by hand.
    unsafe {
        let n = colors.len() as i32;
        let bi = BITMAPINFO { header: BITMAPINFOHEADER { size: 40, width: w * n, height: -h, planes: 1, bit_count: 32, compression: 0, size_image: 0, x_pels_per_meter: 0, y_pels_per_meter: 0, clr_used: 0, clr_important: 0 }, colors: [0; 1] };
        let mut bits: *mut c_void = core::ptr::null_mut();
        let b = CreateDIBSection(core::ptr::null_mut(), &bi, 0, &mut bits, core::ptr::null_mut(), 0);
        let p = bits as *mut u32;
        for y in 0..h {
            for x in 0..w * n {
                *p.add((y * w * n + x) as usize) = colors[(x / w) as usize] | if alpha { 0xFF00_0000 } else { 0 };
            }
        }
        b
    }
}

/// A monochrome mask strip of n images 8 pixels wide: image k's rows are `rows[k]` (bit 7 = the left pixel; 1 = transparent).
fn mask_strip(h: i32, rows: &[u8]) -> HBITMAP {
    let n = rows.len() as i32;
    let mut bits = [0u8; 64];
    for y in 0..h as usize {
        for k in 0..n as usize {
            bits[y * 2 * ((n as usize + 1) / 2) + k] = rows[k];
        }
    }
    // SAFETY: bits holds 2 * ceil(n / 2) bytes per row, a 16 bit aligned scan line.
    unsafe { CreateBitmap(8 * n, h, 1, 1, bits.as_ptr() as *const c_void) }
}

fn hex(c: u32) -> Buf<16> {
    let mut b = Buf::<16>::new();
    let _ = write!(b, "#{:02x}{:02x}{:02x}", (c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    b
}

/// A target DIB filled with the background; (dc, bitmap, bits).
fn target(w: i32, h: i32, bg: u32) -> (HDC, HBITMAP, *mut u32) {
    let (dc, bmp, bits) = dib32(w, h, true);
    fill_bits(bits, w, h, bg);
    (dc, bmp, bits)
}

fn free_target(t: (HDC, HBITMAP, *mut u32)) {
    // SAFETY: the DC and bitmap came from dib32.
    unsafe {
        DeleteDC(t.0);
        DeleteObject(t.1);
    }
}

/// Samples of an 8x8 image drawn at (ox, oy) into a target of width w: the pixels at (1,1), (6,1), (1,6), (6,6) of the image and the ink box.
fn samples(t: &mut Test, label: &str, bits: *mut u32, w: i32, h: i32, ox: i32, oy: i32, bg: u32) {
    let (l, tp, r, b, n) = ink(bits, w, h, bg);
    say!(t, "{label}: ink box ({l},{tp})-({r},{b}) {n} pixels; at (1,1) {} (6,1) {} (1,6) {} (6,6) {}", hex(px(bits, w, ox + 1, oy + 1)).as_str(), hex(px(bits, w, ox + 6, oy + 1)).as_str(), hex(px(bits, w, ox + 1, oy + 6)).as_str(), hex(px(bits, w, ox + 6, oy + 6)).as_str());
}

fn count(il: HIMAGELIST) -> i32 {
    // SAFETY: a list handle.
    unsafe { ImageList_GetImageCount(il) }
}

/// A GetPixel colour (0x00BBGGRR) as 0x00RRGGBB.
fn bgr(c: u32) -> u32 {
    ((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF)
}

fn draw(t: &mut Test, label: &str, il: HIMAGELIST, idx: i32, style: u32) {
    let (w, h) = (16, 12);
    let tg = target(w, h, MAGENTA);
    set_error(0xDEAD);
    // SAFETY: tg.0 is a memory DC.
    let r = unsafe { ImageList_Draw(il, idx, tg.0, 4, 2, style) };
    let e = last_error();
    say!(t, "{label}: ImageList_Draw = {r} err={}", if r != 0 { -2 } else if e == 0xDEAD { -1 } else { e as i64 });
    samples(t, "   ", tg.2, w, h, 4, 2, MAGENTA);
    free_target(tg);
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    unsafe { InitCommonControls() };
    unsafe {
        // image 0 is opaque in the right half only (mask bit 1 = transparent: 0xF0 = left four pixels), image 1 fully opaque
        let il = ImageList_Create(8, 8, ILC_COLOR32 | ILC_MASK, 4, 4);
        let img = strip(8, 8, &[RED, GREEN, BLUE], false);
        let mask = mask_strip(8, &[0xF0, 0x00, 0xFF]);
        say!(t, "ImageList_Add: {}", ImageList_Add(il, img, mask));
        say!(t, "bk colour {:#x} count {}", ImageList_GetBkColor(il), count(il));
        for (label, st) in [("ILD_NORMAL", ILD_NORMAL), ("ILD_TRANSPARENT", ILD_TRANSPARENT), ("ILD_MASK", ILD_MASK), ("ILD_IMAGE", ILD_IMAGE), ("ILD_BLEND25", ILD_BLEND25), ("ILD_BLEND50", ILD_BLEND50), ("ILD_TRANSPARENT | ILD_BLEND50", ILD_TRANSPARENT | ILD_BLEND50), ("ILD_MASK | ILD_BLEND25", ILD_MASK | ILD_BLEND25)] {
            let mut l = Buf::<100>::new();
            let _ = write!(l, "image 0, {label}");
            draw(t, l.as_str(), il, 0, st);
        }
        draw(t, "image 1 (opaque), ILD_NORMAL", il, 1, ILD_NORMAL);
        draw(t, "image 2 (mask all transparent), ILD_NORMAL", il, 2, ILD_NORMAL);
        draw(t, "image 2, ILD_TRANSPARENT", il, 2, ILD_TRANSPARENT);
        ImageList_SetBkColor(il, 0x0000_FFFF);
        say!(t, "bk colour after SetBkColor(yellow in BGR) {:#x}", ImageList_GetBkColor(il));
        draw(t, "image 0 with a list background colour, ILD_NORMAL", il, 0, ILD_NORMAL);
        draw(t, "image 0 with a list background colour, ILD_TRANSPARENT", il, 0, ILD_TRANSPARENT);
        ImageList_Destroy(il);
        // a list without a mask
        let il2 = ImageList_Create(8, 8, ILC_COLOR32, 4, 4);
        let img2 = strip(8, 8, &[RED, GREEN], false);
        say!(t, "list without mask: ImageList_Add {}", ImageList_Add(il2, img2, core::ptr::null_mut()));
        say!(t, "bk colour {:#x}", ImageList_GetBkColor(il2));
        draw(t, "no mask, ILD_NORMAL", il2, 0, ILD_NORMAL);
        draw(t, "no mask, ILD_TRANSPARENT", il2, 0, ILD_TRANSPARENT);
        draw(t, "no mask, ILD_MASK", il2, 0, ILD_MASK);
        draw(t, "no mask, ILD_BLEND50", il2, 0, ILD_BLEND50);
        ImageList_Destroy(il2);
        // a list of 24 bit colours
        let il3 = ImageList_Create(8, 8, ILC_COLOR24 | ILC_MASK, 4, 4);
        let img3 = strip(8, 8, &[BLUE, YELLOW], false);
        let mask3 = mask_strip(8, &[0x0F, 0x00]);
        say!(t, "ILC_COLOR24 list: ImageList_Add {}", ImageList_Add(il3, img3, mask3));
        draw(t, "ILC_COLOR24, ILD_NORMAL", il3, 0, ILD_NORMAL);
        draw(t, "ILC_COLOR24, ILD_TRANSPARENT", il3, 0, ILD_TRANSPARENT);
        ImageList_Destroy(il3);
        DeleteObject(img);
        DeleteObject(img2);
        DeleteObject(img3);
    }
}
