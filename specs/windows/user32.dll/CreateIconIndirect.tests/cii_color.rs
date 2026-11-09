//! CreateIconIndirect with a colour bitmap and a mask: a 16x16 32-bit DIB
//! (left half opaque red with alpha 0xFF, right half 0) and a 16x16
//! monochrome mask; fIcon FALSE keeps the hotspot 3,4 (also an hotspot
//! outside the bitmap, 100,200), fIcon TRUE reports the centre 8,8;
//! GetIconInfo returns new 16x16 bitmaps (a 32-bit colour bitmap and a
//! 1-bit mask) each time, never the originals, and they stay valid after
//! the originals are deleted; DrawIconEx at 16x16 draws the left half red
//! over the background and at 32x32 draws it scaled (red covers 512 of
//! 1024 pixels); a 24-bit colour bitmap and a monochrome colour bitmap are
//! accepted (the returned colour bitmap is 32 bits); a colour bitmap and
//! a mask of different sizes fail with ERROR_INVALID_PARAMETER (87); the
//! colour bitmap may be selected into a DC at the time of the call.
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
type BOOL = i32;
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

/// "window" or "null" (a handle is never printed).
fn hwnd_kind(h: HWND) -> &'static str {
    if h.is_null() { "null" } else { "window" }
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

/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct ICONINFO {
    f_icon: BOOL,
    x_hotspot: u32,
    y_hotspot: u32,
    mask: HBITMAP,
    color: HBITMAP,
}

type HICON = *mut c_void;
const DI_NORMAL: u32 = 0x0003;
/// Width, height, planes, bits per pixel of a bitmap (0s for a non-bitmap).
fn ly_bm(h: HBITMAP) -> (i32, i32, u16, u16) {
    let mut b = BITMAP::default();
    // SAFETY: b is writable.
    let n = unsafe { GetObjectW(h, core::mem::size_of::<BITMAP>() as i32, &mut b as *mut BITMAP as *mut c_void) };
    if n == 0 { (0, 0, 0, 0) } else { (b.width, b.height, b.planes, b.bits_pixel) }
}

/// One line about an icon or cursor: GetIconInfo's fIcon, hotspot, and the
/// sizes of its bitmaps; the bitmaps are deleted. Returns false when
/// GetIconInfo failed (the line then has the last error).
fn ly_info(t: &mut Test, label: &str, h: HICON) -> bool {
    let mut ii = ICONINFO {
        f_icon: 7,
        x_hotspot: 0xAAAA,
        y_hotspot: 0xBBBB,
        mask: core::ptr::null_mut(),
        color: core::ptr::null_mut(),
    };
    set_error(0x1234);
    // SAFETY: ii is writable; the bitmaps it returns are deleted.
    unsafe {
        if GetIconInfo(h, &mut ii) == 0 {
            say!(t, "{label}: GetIconInfo FALSE error {}", last_error());
            return false;
        }
        let m = ly_bm(ii.mask);
        let c = ly_bm(ii.color);
        say!(
            t,
            "{label}: fIcon {} hotspot {},{} mask {}x{} planes {} bits {} ; color {}x{} planes {} bits {}{}",
            ii.f_icon,
            ii.x_hotspot,
            ii.y_hotspot,
            m.0,
            m.1,
            m.2,
            m.3,
            c.0,
            c.1,
            c.2,
            c.3,
            if ii.color.is_null() { " (no color bitmap)" } else { "" }
        );
        DeleteObject(ii.mask);
        if !ii.color.is_null() {
            DeleteObject(ii.color);
        }
    }
    true
}

/// Draw `h` with DrawIconEx (flags) into a w x h 32-bit DIB filled with
/// magenta and count the pixels: (background, red, green, other).
fn ly_draw(h: HICON, w: i32, hh: i32, flags: u32) -> (i32, i32, i32, i32) {
    let (dc, bmp, bits) = dib32(w, hh, true);
    fill_bits(bits, w, hh, 0x00FF00FF);
    // SAFETY: a DIB DC of our own, deleted below.
    unsafe { DrawIconEx(dc, 0, 0, h, w, hh, 0, core::ptr::null_mut(), flags) };
    let (mut bg, mut red, mut green, mut other) = (0, 0, 0, 0);
    for y in 0..hh {
        for x in 0..w {
            match px(bits, w, x, y) & 0xFFFFFF {
                0xFF00FF => bg += 1,
                0xFF0000 => red += 1,
                0x00FF00 => green += 1,
                _ => other += 1,
            }
        }
    }
    // SAFETY: our DC and bitmap.
    unsafe {
        DeleteDC(dc);
        DeleteObject(bmp);
    }
    (bg, red, green, other)
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateIconIndirect(info: *const ICONINFO) -> HICON;
    fn GetIconInfo(icon: HICON, info: *mut ICONINFO) -> BOOL;
    fn DestroyIcon(icon: HICON) -> BOOL;
    fn DrawIconEx(dc: HDC, x: i32, y: i32, icon: HICON, cx: i32, cy: i32, step: u32, brush: HBRUSH, flags: u32)
    -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
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

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: bitmaps and icons created here are destroyed.
    unsafe {
        let null = core::ptr::null_mut();
        let (cdc, cbmp, cbits) = dib32(16, 16, true);
        for y in 0..16 {
            for x in 0..16 {
                *cbits.add(y * 16 + x) = if x < 8 { 0xFF_FF0000 } else { 0x00_000000 };
            }
        }
        let mut mask = [0u8; 32];
        for r in 0..16 {
            mask[r * 2] = 0x00;
            mask[r * 2 + 1] = 0xFF;
        }
        let mb = CreateBitmap(16, 16, 1, 1, mask.as_ptr() as *const c_void);
        let ci = ICONINFO { f_icon: 0, x_hotspot: 3, y_hotspot: 4, mask: mb, color: cbmp };
        set_error(0x1234);
        let cur = CreateIconIndirect(&ci);
        say!(
            t,
            "fIcon FALSE hotspot 3,4 (colour bitmap selected in a DC): {} error {:#x}",
            hwnd_kind(cur),
            last_error()
        );
        ly_info(t, "   shape", cur);
        let d = ly_draw(cur, 16, 16, DI_NORMAL);
        say!(t, "   drawn 16x16: background {} red {} other {}", d.0, d.1, d.3);
        let d = ly_draw(cur, 32, 32, DI_NORMAL);
        say!(t, "   drawn 32x32: background {} red {} other {}", d.0, d.1, d.3);
        let ci2 = ICONINFO { f_icon: 1, x_hotspot: 3, y_hotspot: 4, mask: mb, color: cbmp };
        let ic = CreateIconIndirect(&ci2);
        ly_info(t, "fIcon TRUE", ic);
        let ci3 = ICONINFO { f_icon: 0, x_hotspot: 100, y_hotspot: 200, mask: mb, color: cbmp };
        let far = CreateIconIndirect(&ci3);
        say!(t, "fIcon FALSE hotspot 100,200 outside the bitmaps: {}", hwnd_kind(far));
        ly_info(t, "   shape", far);
        // originals deleted
        DeleteDC(cdc);
        DeleteObject(cbmp);
        DeleteObject(mb);
        ly_info(t, "after deleting the original bitmaps", cur);
        let d = ly_draw(cur, 16, 16, DI_NORMAL);
        say!(t, "   drawn 16x16 again: background {} red {}", d.0, d.1);
        for h in [cur, ic, far] {
            if !h.is_null() {
                DestroyIcon(h);
            }
        }
        // 24 bpp colour bitmap, mono colour bitmap
        let mb2 = CreateBitmap(16, 16, 1, 1, mask.as_ptr() as *const c_void);
        for (lbl, planes, bpp) in [("24 bpp", 1u32, 24u32), ("1 bpp", 1, 1), ("8 bpp", 1, 8), ("16 bpp", 1, 16)] {
            let data = [0u8; 16 * 16 * 3];
            let c = CreateBitmap(16, 16, planes, bpp, data.as_ptr() as *const c_void);
            let ii = ICONINFO { f_icon: 1, x_hotspot: 0, y_hotspot: 0, mask: mb2, color: c };
            set_error(0x1234);
            let i = CreateIconIndirect(&ii);
            say!(t, "{lbl} colour bitmap: {} error {:#x}", hwnd_kind(i), last_error());
            if !i.is_null() {
                ly_info(t, "   shape", i);
                DestroyIcon(i);
            }
            DeleteObject(c);
        }
        // the returned bitmaps are different from call to call
        let (d2, b2, _) = dib32(16, 16, true);
        let ii = ICONINFO { f_icon: 1, x_hotspot: 0, y_hotspot: 0, mask: mb2, color: b2 };
        let i = CreateIconIndirect(&ii);
        let mut a = ICONINFO { f_icon: 9, x_hotspot: 0, y_hotspot: 0, mask: null, color: null };
        let mut b = a;
        GetIconInfo(i, &mut a);
        GetIconInfo(i, &mut b);
        say!(
            t,
            "GetIconInfo twice: masks differ {} colours differ {} ; not the originals {}",
            a.mask != b.mask,
            a.color != b.color,
            a.mask != mb2 && a.color != b2
        );
        for h in [a.mask, a.color, b.mask, b.color] {
            DeleteObject(h);
        }
        DestroyIcon(i);
        // mismatched sizes
        let big = CreateBitmap(32, 32, 1, 1, [0u8; 128].as_ptr() as *const c_void);
        let bad = ICONINFO { f_icon: 1, x_hotspot: 0, y_hotspot: 0, mask: big, color: b2 };
        set_error(0x1234);
        let x = CreateIconIndirect(&bad);
        say!(t, "colour 16x16 with mask 32x32: {} error {}", hwnd_kind(x), last_error());
        let wide = CreateBitmap(16, 32, 1, 1, [0u8; 128].as_ptr() as *const c_void);
        let bad2 = ICONINFO { f_icon: 1, x_hotspot: 0, y_hotspot: 0, mask: wide, color: b2 };
        set_error(0x1234);
        let x = CreateIconIndirect(&bad2);
        say!(t, "colour 16x16 with mask 16x32: {} error {}", hwnd_kind(x), last_error());
        if !x.is_null() {
            ly_info(t, "   shape", x);
            DestroyIcon(x);
        }
        DeleteObject(big);
        DeleteObject(wide);
        DeleteObject(mb2);
        DeleteDC(d2);
        DeleteObject(b2);
    }
}
