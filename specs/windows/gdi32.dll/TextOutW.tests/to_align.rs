//! TextOutW reference point and alignment: with TA_LEFT|TA_TOP (default)
//! the opaque cell starts at (x, y); TA_RIGHT ends it at x; TA_CENTER
//! centres it on x; TA_BOTTOM ends it at y; TA_BASELINE puts the ascent
//! above y; TA_UPDATECP draws at the current position and moves it by the
//! extent (observed via a second string); negative coordinates clip;
//! count 0 draws nothing and returns nonzero; a bad DC fails.
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
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SIZE {
    cx: i32,
    cy: i32,
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

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy)]
struct TEXTMETRICW {
    height: i32,
    ascent: i32,
    descent: i32,
    internal_leading: i32,
    external_leading: i32,
    ave_char_width: i32,
    max_char_width: i32,
    weight: i32,
    overhang: i32,
    digitized_aspect_x: i32,
    digitized_aspect_y: i32,
    first_char: u16,
    last_char: u16,
    default_char: u16,
    break_char: u16,
    italic: u8,
    underlined: u8,
    struck_out: u8,
    pitch_and_family: u8,
    char_set: u8,
}

const TA_LEFT: u32 = 0;
const TA_TOP: u32 = 0;
const TA_UPDATECP: u32 = 1;
const TA_RIGHT: u32 = 2;
const TA_CENTER: u32 = 6;
const TA_BOTTOM: u32 = 8;
const TA_BASELINE: u32 = 24;
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

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn SetBkColor(hdc: HDC, color: COLORREF) -> COLORREF;
    fn TextOutW(hdc: HDC, x: i32, y: i32, text: *const u16, n: i32) -> BOOL;
    fn GetTextExtentPoint32W(hdc: HDC, text: *const u16, n: i32, size: *mut SIZE) -> BOOL;
    fn GetTextMetricsW(hdc: HDC, tm: *mut TEXTMETRICW) -> BOOL;
    fn SetTextAlign(hdc: HDC, align: u32) -> u32;
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

fn cell(t: &mut Test, label: &str, dc: HDC, bits: *mut u32, w: i32, h: i32, align: u32, x: i32, y: i32) {
    fill_bits(bits, w, h, 0x0044_4444);
    // SAFETY: GDI calls on our DC.
    unsafe {
        SetTextAlign(dc, align);
        let ok = TextOutW(dc, x, y, wz!("Mm").as_ptr(), 2);
        GdiFlush();
        let (l, tp, r, b, _) = ink(bits, w, h, 0x0044_4444);
        say!(t, "{label}: returned {} cell {},{},{},{}", ok, l, tp, r, b);
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let (w, h) = (120, 60);
    let (dc, bmp, bits) = dib32(w, h, true);
    let mut tm: TEXTMETRICW = zeroed();
    let mut sz = SIZE::default();
    // SAFETY: GDI calls on our DC.
    unsafe {
        SetBkColor(dc, rgb(255, 255, 255));
        GetTextMetricsW(dc, &mut tm);
        GetTextExtentPoint32W(dc, wz!("Mm").as_ptr(), 2, &mut sz);
    }
    say!(t, "extent {}x{} ascent {}", sz.cx, sz.cy, tm.ascent);
    cell(t, "TA_LEFT|TA_TOP at 50,20", dc, bits, w, h, TA_LEFT | TA_TOP, 50, 20);
    cell(t, "TA_RIGHT at 50,20", dc, bits, w, h, TA_RIGHT, 50, 20);
    cell(t, "TA_CENTER at 50,20", dc, bits, w, h, TA_CENTER, 50, 20);
    cell(t, "TA_BOTTOM at 50,40", dc, bits, w, h, TA_BOTTOM, 50, 40);
    cell(t, "TA_BASELINE at 50,40", dc, bits, w, h, TA_BASELINE, 50, 40);
    cell(t, "TA_LEFT at -5,-3", dc, bits, w, h, TA_LEFT, -5, -3);
    fill_bits(bits, w, h, 0x0044_4444);
    // SAFETY: GDI calls on our DC.
    unsafe {
        SetTextAlign(dc, TA_LEFT);
        let ok = TextOutW(dc, 10, 10, wz!("Mm").as_ptr(), 0);
        GdiFlush();
        let (_, _, _, _, n) = ink(bits, w, h, 0x0044_4444);
        say!(t, "count 0: returned {} changed {}", ok, n);
        SetTextAlign(dc, TA_UPDATECP);
        TextOutW(dc, 99, 99, wz!("Mm").as_ptr(), 2);
        TextOutW(dc, 99, 99, wz!("Mm").as_ptr(), 2);
        GdiFlush();
        let (l, tp, r, b, _) = ink(bits, w, h, 0x0044_4444);
        say!(t, "TA_UPDATECP twice from the start position: cells span {},{},{},{}", l, tp, r, b);
        set_error(0);
        say!(t, "bad DC: {} error {}", TextOutW(0x0bad_0bad_usize as HDC, 0, 0, wz!("x").as_ptr(), 1), last_error());
        DeleteDC(dc);
        DeleteObject(bmp);
    }
}
