//! CreateIconIndirect with only a monochrome mask bitmap (hbmColor NULL):
//! the mask holds the AND mask in its top half and the XOR mask in its
//! bottom half (16x32 gives a 16x16 icon; GetIconInfo returns the 16x32
//! mask and no colour bitmap); an odd height (31) is accepted; fIcon TRUE
//! gives hotspot at the centre whatever xHotspot/yHotspot say (5,6 ->
//! 8,8; 7,7 with height 31 -> 8,7); fIcon FALSE keeps the hotspot given;
//! the source mask can be deleted afterwards; DrawIconEx with DI_NORMAL
//! puts the AND/XOR result into a DIB (opaque white where AND 0 and XOR 1,
//! the background where AND is 1 and XOR 0); DestroyIcon succeeds once and
//! then fails with ERROR_INVALID_CURSOR_HANDLE (1402).
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

fn lo(v: isize) -> i32 {
    (v as u32 & 0xFFFF) as u16 as i16 as i32
}

fn hi(v: isize) -> i32 {
    ((v as u32 >> 16) & 0xFFFF) as u16 as i16 as i32
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

type HCURSOR = *mut c_void;
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
/// winuser.h: DrawIconEx flags.
const DI_MASK: u32 = 0x0001;

const DI_IMAGE: u32 = 0x0002;
const DI_NORMAL: u32 = 0x0003;
/// "(lo-hi)" style result text of a BOOL.
fn ly_b(v: BOOL) -> &'static str {
    if v != 0 { "TRUE" } else { "FALSE" }
}

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
    fn DestroyCursor(cursor: HCURSOR) -> BOOL;
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
        let mut mono = [0u8; 16 * 4];
        for r in 0..16 {
            mono[r * 2] = 0x00;
            mono[r * 2 + 1] = 0xFF;
            mono[(16 + r) * 2] = 0xFF;
            mono[(16 + r) * 2 + 1] = 0x00;
        }
        let mb = CreateBitmap(16, 32, 1, 1, mono.as_ptr() as *const c_void);
        let ii = ICONINFO { f_icon: 1, x_hotspot: 5, y_hotspot: 6, mask: mb, color: core::ptr::null_mut() };
        set_error(0x1234);
        let ic = CreateIconIndirect(&ii);
        say!(t, "fIcon TRUE, mask 16x32: {} error {:#x}", hwnd_kind(ic), last_error());
        ly_info(t, "   shape", ic);
        let d = ly_draw(ic, 16, 16, DI_NORMAL);
        say!(t, "   DI_NORMAL: background {} other {}", d.0, d.3);
        let d = ly_draw(ic, 16, 16, DI_MASK);
        say!(t, "   DI_MASK: background {} other {}", d.0, d.3);
        let d = ly_draw(ic, 16, 16, DI_IMAGE);
        say!(t, "   DI_IMAGE: background {} other {}", d.0, d.3);
        DeleteObject(mb);
        ly_info(t, "   after deleting the source mask", ic);
        set_error(0x1234);
        let r = DestroyIcon(ic);
        say!(t, "DestroyIcon: {} error {:#x}", ly_b(r), last_error());
        set_error(0x1234);
        let r = DestroyIcon(ic);
        say!(t, "DestroyIcon again: {} error {}", ly_b(r), last_error());
        ly_info(t, "GetIconInfo on the destroyed icon", ic);
        // cursor from a mono mask
        let mb2 = CreateBitmap(16, 32, 1, 1, mono.as_ptr() as *const c_void);
        let cc = ICONINFO { f_icon: 0, x_hotspot: 5, y_hotspot: 6, mask: mb2, color: core::ptr::null_mut() };
        let cur = CreateIconIndirect(&cc);
        say!(t, "fIcon FALSE hotspot 5,6: {}", hwnd_kind(cur));
        ly_info(t, "   shape", cur);
        DestroyCursor(cur);
        // odd height
        let mb3 = CreateBitmap(16, 31, 1, 1, mono.as_ptr() as *const c_void);
        let oi = ICONINFO { f_icon: 1, x_hotspot: 7, y_hotspot: 7, mask: mb3, color: core::ptr::null_mut() };
        set_error(0x1234);
        let o = CreateIconIndirect(&oi);
        say!(t, "mask 16x31: {} error {:#x}", hwnd_kind(o), last_error());
        ly_info(t, "   shape", o);
        DestroyIcon(o);
        // other sizes
        for (w, h) in [(32, 64), (8, 16), (48, 96), (16, 16), (1, 2), (64, 128)] {
            let bits = [0xAAu8; 64 * 16];
            let m = CreateBitmap(w, h, 1, 1, bits.as_ptr() as *const c_void);
            let ii = ICONINFO { f_icon: 1, x_hotspot: 0, y_hotspot: 0, mask: m, color: core::ptr::null_mut() };
            set_error(0x1234);
            let i = CreateIconIndirect(&ii);
            say!(t, "mask {}x{}: {} error {:#x}", w, h, hwnd_kind(i), last_error());
            if !i.is_null() {
                ly_info(t, "   shape", i);
                DestroyIcon(i);
            }
            DeleteObject(m);
        }
        DeleteObject(mb2);
        DeleteObject(mb3);
    }
}
