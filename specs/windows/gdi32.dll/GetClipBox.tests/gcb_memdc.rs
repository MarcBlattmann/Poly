//! GetClipBox on memory DCs: one with only the default 1x1 monochrome
//! bitmap, one with a 40x30 DIB, with IntersectClipRect (a simple clip),
//! ExcludeClipRect (a complex one: the box of what is left), an empty clip
//! (NULLREGION and what the box is then), a window origin moved
//! (SetWindowOrgEx: the box in logical coordinates), and bad DCs (the
//! return value ERROR 0 and the last error).
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
struct POINT {
    x: i32,
    y: i32,
}

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

type HRGN = *mut c_void;
/// "l,t,r,b" of a rectangle.
struct R(RECT);

impl core::fmt::Display for R {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{},{},{},{}", self.0.left, self.0.top, self.0.right, self.0.bottom)
    }
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
    fn GetClipBox(hdc: HDC, rect: *mut RECT) -> i32;
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
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn SelectClipRgn(hdc: HDC, rgn: HRGN) -> i32;
    fn IntersectClipRect(hdc: HDC, l: i32, t: i32, r: i32, b: i32) -> i32;
    fn ExcludeClipRect(hdc: HDC, l: i32, t: i32, r: i32, b: i32) -> i32;
    fn SetViewportOrgEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    fn SetWindowOrgEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
}

poly_wtest::main!(test);

fn gcb(t: &mut Test, label: &str, dc: HDC) {
    let mut bx = RECT { left: -7, top: -7, right: -7, bottom: -7 };
    set_error(0);
    // SAFETY: bx is writable; bad DCs are what is observed.
    let k = unsafe { GetClipBox(dc, &mut bx) };
    say!(t, "{label}: {k} {} error {}", R(bx), last_error());
}

fn test(t: &mut Test) {
    // SAFETY: DCs and bitmaps of ours, deleted at the end.
    unsafe {
        let plain = CreateCompatibleDC(core::ptr::null_mut());
        gcb(t, "memory DC, default bitmap", plain);
        let (dc, bmp, _) = dib32(40, 30, true);
        gcb(t, "memory DC, 40x30 DIB", dc);
        say!(t, "IntersectClipRect(5,5,30,20) = {}", IntersectClipRect(dc, 5, 5, 30, 20));
        gcb(t, "  then", dc);
        say!(t, "ExcludeClipRect(5,5,30,10) = {}", ExcludeClipRect(dc, 5, 5, 30, 10));
        gcb(t, "  then", dc);
        say!(t, "ExcludeClipRect(10,12,15,15) = {}", ExcludeClipRect(dc, 10, 12, 15, 15));
        gcb(t, "  then", dc);
        say!(t, "IntersectClipRect(100,100,110,110) = {}", IntersectClipRect(dc, 100, 100, 110, 110));
        gcb(t, "  then (empty)", dc);
        SelectClipRgn(dc, core::ptr::null_mut());
        gcb(t, "clip removed", dc);
        let mut old = POINT::default();
        SetWindowOrgEx(dc, 10, 20, &mut old);
        gcb(t, "window origin 10,20", dc);
        SetWindowOrgEx(dc, 0, 0, &mut old);
        SetViewportOrgEx(dc, 10, 20, &mut old);
        gcb(t, "viewport origin 10,20", dc);
        SetViewportOrgEx(dc, 0, 0, &mut old);
        let gone = CreateCompatibleDC(core::ptr::null_mut());
        DeleteDC(gone);
        gcb(t, "deleted DC", gone);
        gcb(t, "NULL", core::ptr::null_mut());
        gcb(t, "made-up DC", 0x0bad_0bad_usize as HDC);
        gcb(t, "a region handle", CreateRectRgn(0, 0, 5, 5) as HDC);
        DeleteDC(plain);
        DeleteDC(dc);
        DeleteObject(bmp);
    }
}
