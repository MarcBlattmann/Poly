//! DeleteObject on a brush, a pen, a font and a bitmap that are selected
//! into a memory DC: the return value, what the DC then has selected
//! (GetCurrentObject) and what drawing with it gives: a pen or brush that
//! was deleted while selected is replaced by the DC's default object.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HWND = *mut c_void;
type HBRUSH = *mut c_void;
type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type HFONT = *mut c_void;
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

const GWLP_USERDATA: i32 = -21;
const PATCOPY: u32 = 0x00F0_0021;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const OBJ_PEN: u32 = 1;
const OBJ_BRUSH: u32 = 2;
const OBJ_FONT: u32 = 6;
const OBJ_BITMAP: u32 = 7;
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

/// How a window handle in a message is printed: "null", the tag letter of
/// a window of this process (GWLP_USERDATA), "ours" (this process, no tag),
/// "other" (another process: the desktop's windows, the shell) or
/// "invalid".
struct W(HWND);

impl core::fmt::Display for W {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0.is_null() {
            return f.write_str("null");
        }
        let mut pid = 0u32;
        // SAFETY: plain queries; pid is writable.
        let (tid, me) = unsafe { (GetWindowThreadProcessId(self.0, &mut pid), GetCurrentProcessId()) };
        if tid == 0 {
            return f.write_str("invalid");
        }
        if pid != me {
            return f.write_str("other");
        }
        // SAFETY: a window of this process.
        let tag = unsafe { GetWindowLongPtrW(self.0, GWLP_USERDATA) };
        if (0x41..=0x5A).contains(&tag) || (0x61..=0x7A).contains(&tag) {
            f.write_char(tag as u8 as char)
        } else {
            f.write_str("ours")
        }
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
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
    fn GetTextMetricsW(hdc: HDC, tm: *mut TEXTMETRICW) -> BOOL;
    fn CreatePen(style: i32, width: i32, color: COLORREF) -> HGDIOBJ;
    fn Rectangle(hdc: HDC, l: i32, t: i32, r: i32, b: i32) -> BOOL;
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
    fn CreateFontW(
        h: i32,
        w: i32,
        esc: i32,
        orient: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike: u32,
        charset: u32,
        outp: u32,
        clip: u32,
        quality: u32,
        pitch: u32,
        face: *const u16,
    ) -> HFONT;
}

/// "stock N" when h is the stock object N, "null", or "other".
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
        let rc = RECT { left: 0, top: 0, right: 8, bottom: 8 };

        // Brush.
        let br = CreateSolidBrush(rgb(0x20, 0x40, 0x60));
        let old = SelectObject(dc, br);
        say!(t, "brush: the DC had {} before", who(old));
        set_error(0);
        let r = DeleteObject(br);
        say!(t, "brush selected, deleted: {r} error {}", last_error());
        let cur = GetCurrentObject(dc, OBJ_BRUSH);
        say!(t, "  GetCurrentObject(brush): same handle {} / {} type {}", (cur == br) as i32, who(cur), GetObjectType(cur));
        fill_bits(bits, 8, 8, 0x0011_2233);
        let ok = PatBlt(dc, 0, 0, 4, 4, PATCOPY);
        GdiFlush();
        say!(t, "  PatBlt: {ok} pixel inside {:06x} outside {:06x}", px(bits, 8, 1, 1), px(bits, 8, 6, 6));
        fill_bits(bits, 8, 8, 0x0011_2233);
        Rectangle(dc, 0, 0, 5, 5);
        GdiFlush();
        say!(t, "  Rectangle: edge {:06x} inside {:06x}", px(bits, 8, 0, 2), px(bits, 8, 2, 2));
        SelectObject(dc, old);

        // Pen.
        let pen = CreatePen(0, 1, rgb(0xE0, 0x10, 0x10));
        let oldp = SelectObject(dc, pen);
        say!(t, "pen: the DC had {} before", who(oldp));
        fill_bits(bits, 8, 8, 0x0011_2233);
        Rectangle(dc, 0, 0, 5, 5);
        GdiFlush();
        say!(t, "  pen alive: edge {:06x}", px(bits, 8, 0, 2));
        set_error(0);
        let r = DeleteObject(pen);
        say!(t, "pen selected, deleted: {r} error {}", last_error());
        let cur = GetCurrentObject(dc, OBJ_PEN);
        say!(t, "  GetCurrentObject(pen): same handle {} / {} type {}", (cur == pen) as i32, who(cur), GetObjectType(cur));
        fill_bits(bits, 8, 8, 0x0011_2233);
        Rectangle(dc, 0, 0, 5, 5);
        GdiFlush();
        say!(t, "  Rectangle after: edge {:06x} inside {:06x}", px(bits, 8, 0, 2), px(bits, 8, 2, 2));
        SelectObject(dc, oldp);

        // Font.
        let face = wz!("Segoe UI");
        let font = CreateFontW(30, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 0, 0, face.as_ptr());
        let oldf = SelectObject(dc, font);
        let mut tm: TEXTMETRICW = zeroed();
        GetTextMetricsW(dc, &mut tm);
        let h_font = tm.height;
        say!(t, "font: the DC had {} before; height with it {}", who(oldf), h_font);
        set_error(0);
        let r = DeleteObject(font);
        say!(t, "font selected, deleted: {r} error {}", last_error());
        let cur = GetCurrentObject(dc, OBJ_FONT);
        say!(t, "  GetCurrentObject(font): same handle {} / {} type {}", (cur == font) as i32, who(cur), GetObjectType(cur));
        GetTextMetricsW(dc, &mut tm);
        let h_now = tm.height;
        SelectObject(dc, oldf);
        GetTextMetricsW(dc, &mut tm);
        say!(t, "  text height now {h_now} (default font {})", tm.height);

        // Bitmap.
        set_error(0);
        let r = DeleteObject(bmp);
        say!(t, "bitmap selected, deleted: {r} error {} type {}", last_error(), GetObjectType(bmp));
        let curb = GetCurrentObject(dc, OBJ_BITMAP);
        say!(t, "  GetCurrentObject(bitmap): same handle {}", (curb == bmp) as i32);
        let fill = CreateSolidBrush(rgb(0x01, 0x02, 0x03));
        FillRect(dc, &rc, fill);
        GdiFlush();
        say!(t, "  FillRect after: pixel {:06x}", px(bits, 8, 3, 3));
        DeleteObject(fill);
        DeleteDC(dc);
        set_error(0);
        let r = DeleteObject(bmp);
        say!(t, "bitmap after DeleteDC: {r} error {} type {}", last_error(), GetObjectType(bmp));
    }
}
