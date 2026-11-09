//! DrawTextW with DT_CALCRECT (nothing drawn): for a single line the
//! rectangle's width becomes the text extent and the height the line
//! height, and the return value is the height; "&" prefixes are removed
//! from the measure (a "&&" stays as one "&") unless DT_NOPREFIX; with
//! DT_WORDBREAK the text wraps at the rectangle's width (lines counted
//! from the height); "\n" breaks lines without DT_SINGLELINE; tabs with
//! DT_EXPANDTABS; an empty string; count -1 means NUL-terminated.
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
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

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

const DT_WORDBREAK: u32 = 0x0010;
const DT_SINGLELINE: u32 = 0x0020;
const DT_EXPANDTABS: u32 = 0x0040;
const DT_CALCRECT: u32 = 0x0400;
const DT_NOPREFIX: u32 = 0x0800;
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

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DrawTextW(hdc: HDC, text: *const u16, n: i32, rect: *mut RECT, format: u32) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetTextExtentPoint32W(hdc: HDC, text: *const u16, n: i32, size: *mut SIZE) -> BOOL;
    fn GetTextMetricsW(hdc: HDC, tm: *mut TEXTMETRICW) -> BOOL;
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

fn calc(t: &mut Test, label: &str, dc: HDC, s: &[u16], n: i32, width: i32, fmt: u32) {
    let mut r = RECT { left: 10, top: 20, right: 10 + width, bottom: 20 };
    // SAFETY: s is NUL-terminated; r is writable.
    let ret = unsafe { DrawTextW(dc, s.as_ptr(), n, &mut r, fmt | DT_CALCRECT) };
    say!(t, "{label}: returned {} rect {},{},{},{}", ret, r.left, r.top, r.right, r.bottom);
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let (dc, bmp, bits) = dib32(4, 4, true);
    fill_bits(bits, 4, 4, 0x0055_5555);
    let mut tm: TEXTMETRICW = zeroed();
    // SAFETY: tm is writable.
    unsafe { GetTextMetricsW(dc, &mut tm) };
    say!(t, "line height {}", tm.height);
    let mut sz = SIZE::default();
    // SAFETY: 5 characters.
    unsafe { GetTextExtentPoint32W(dc, wz!("Hello").as_ptr(), 5, &mut sz) };
    say!(t, "extent of Hello {}x{}", sz.cx, sz.cy);
    calc(t, "Hello single line", dc, wz!("Hello"), -1, 0, DT_SINGLELINE);
    calc(t, "Hello no flags", dc, wz!("Hello"), -1, 0, 0);
    calc(t, "count 3 of Hello", dc, wz!("Hello"), 3, 0, DT_SINGLELINE);
    calc(t, "&File", dc, wz!("&File"), -1, 0, DT_SINGLELINE);
    calc(t, "&File DT_NOPREFIX", dc, wz!("&File"), -1, 0, DT_SINGLELINE | DT_NOPREFIX);
    calc(t, "A&&B", dc, wz!("A&&B"), -1, 0, DT_SINGLELINE);
    calc(t, "two lines with \\n", dc, wz!("one\ntwo"), -1, 0, 0);
    calc(t, "\\n with DT_SINGLELINE", dc, wz!("one\ntwo"), -1, 0, DT_SINGLELINE);
    calc(t, "wordbreak width 60", dc, wz!("the quick brown fox jumps"), -1, 60, DT_WORDBREAK);
    calc(t, "wordbreak width 1000", dc, wz!("the quick brown fox jumps"), -1, 1000, DT_WORDBREAK);
    calc(t, "wordbreak long word width 20", dc, wz!("abcdefghij"), -1, 20, DT_WORDBREAK);
    calc(t, "tab without DT_EXPANDTABS", dc, wz!("a\tb"), -1, 0, DT_SINGLELINE);
    calc(t, "tab with DT_EXPANDTABS", dc, wz!("a\tb"), -1, 0, DT_SINGLELINE | DT_EXPANDTABS);
    calc(t, "empty", dc, wz!(""), -1, 0, DT_SINGLELINE);
    calc(t, "empty no flags", dc, wz!(""), -1, 0, 0);
    check_unchanged(t, bits);
    // SAFETY: our DC and bitmap.
    unsafe {
        DeleteDC(dc);
        DeleteObject(bmp);
    }
}

fn check_unchanged(t: &mut Test, bits: *mut u32) {
    // SAFETY: plain call.
    unsafe { GdiFlush() };
    let (_, _, _, _, n) = ink(bits, 4, 4, 0x0055_5555);
    say!(t, "pixels changed by DT_CALCRECT: {n}");
}
