//! ImageList_GetImageInfo after the list changes: images added (the bitmap
//! grows and may be replaced by a bigger one), removed, replaced, the count
//! set with ImageList_SetImageCount, the icon size changed; the rectangles stay
//! within the bitmap, the rectangle of image k is k * width in a strip.
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
#[derive(Clone, Copy, Default)]
struct BITMAP {
    kind: i32,
    width: i32,
    height: i32,
    width_bytes: i32,
    planes: u16,
    bits_pixel: u16,
    bits: usize,
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

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
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
const ILC_COLOR32: u32 = 0x20;
/// commctrl.h
#[repr(C)]
#[derive(Clone, Copy)]
struct IMAGEINFO {
    hbm_image: HBITMAP,
    hbm_mask: HBITMAP,
    unused1: i32,
    unused2: i32,
    rc_image: RECT,
}

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
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
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
    fn ImageList_SetImageCount(il: HIMAGELIST, n: u32) -> BOOL;
    fn ImageList_Add(il: HIMAGELIST, image: HBITMAP, mask: HBITMAP) -> i32;
    fn ImageList_Replace(il: HIMAGELIST, index: i32, image: HBITMAP, mask: HBITMAP) -> BOOL;
    fn ImageList_Remove(il: HIMAGELIST, index: i32) -> BOOL;
    fn ImageList_SetIconSize(il: HIMAGELIST, cx: i32, cy: i32) -> BOOL;
    fn ImageList_GetImageInfo(il: HIMAGELIST, index: i32, info: *mut IMAGEINFO) -> BOOL;
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

fn info(t: &mut Test, label: &str, il: HIMAGELIST, i: i32) {
    let mut ii: IMAGEINFO = zeroed();
    // SAFETY: ii is an IMAGEINFO.
    let r = unsafe { ImageList_GetImageInfo(il, i, &mut ii) };
    if r == 0 {
        say!(t, "{label}: image {i}: FALSE");
        return;
    }
    let mut b: BITMAP = zeroed();
    // SAFETY: b is a BITMAP.
    unsafe { GetObjectW(ii.hbm_image, core::mem::size_of::<BITMAP>() as i32, &mut b as *mut BITMAP as *mut c_void) };
    say!(t, "{label}: image {i}: rcImage ({},{},{},{}) bitmap {}x{} rectangle inside the bitmap {}", ii.rc_image.left, ii.rc_image.top, ii.rc_image.right, ii.rc_image.bottom, b.width, b.height, (ii.rc_image.left >= 0 && ii.rc_image.top >= 0 && ii.rc_image.right <= b.width && ii.rc_image.bottom <= b.height) as i32);
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    unsafe { InitCommonControls() };
    unsafe {
        let il = ImageList_Create(8, 8, ILC_COLOR32 | ILC_MASK, 1, 1);
        let one = strip(8, 8, &[RED], false);
        let m1 = mask_strip(8, &[0x00]);
        for i in 0..5 {
            say!(t, "Add #{i}: {}", ImageList_Add(il, one, m1));
        }
        for i in 0..5 {
            info(t, "5 images", il, i);
        }
        say!(t, "ImageList_Remove(1): {}", ImageList_Remove(il, 1));
        say!(t, "count {}", count(il));
        for i in 0..5 {
            info(t, "after Remove(1)", il, i);
        }
        let g = strip(8, 8, &[GREEN], false);
        say!(t, "ImageList_Replace(0): {}", ImageList_Replace(il, 0, g, m1));
        info(t, "after Replace", il, 0);
        say!(t, "ImageList_SetImageCount(8): {}", ImageList_SetImageCount(il, 8));
        say!(t, "count {}", count(il));
        for i in [0, 3, 4, 7, 8] {
            info(t, "after SetImageCount(8)", il, i);
        }
        say!(t, "ImageList_SetImageCount(2): {}", ImageList_SetImageCount(il, 2));
        say!(t, "count {}", count(il));
        info(t, "after SetImageCount(2)", il, 1);
        info(t, "after SetImageCount(2)", il, 2);
        // a list with a bigger image size
        say!(t, "ImageList_SetIconSize(16, 16): {}", ImageList_SetIconSize(il, 16, 16));
        say!(t, "count after changing the size {}", count(il));
        let big = strip(16, 16, &[BLUE], false);
        let mb = mask_strip(16, &[0x00]);
        let _ = mb;
        say!(t, "Add a 16x16 image: {}", ImageList_Add(il, big, core::ptr::null_mut()));
        info(t, "16x16 list", il, 0);
        ImageList_Destroy(il);
        DeleteObject(one);
        DeleteObject(g);
        DeleteObject(big);
    }
}
