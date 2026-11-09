//! DestroyIcon on an icon from CreateIconIndirect: TRUE, then FALSE with
//! ERROR_INVALID_CURSOR_HANDLE (1402) when called again; GetIconInfo,
//! CopyIcon and DrawIconEx fail with 1402 on the destroyed handle;
//! destroying the original does not affect a copy made before; the last
//! error is not touched on success.
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
    fn CopyIcon(icon: HICON) -> HICON;
    fn DrawIconEx(dc: HDC, x: i32, y: i32, icon: HICON, cx: i32, cy: i32, step: u32, brush: HBRUSH, flags: u32)
    -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const c_void) -> HBITMAP;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: icons created here are destroyed; others are system shared.
    unsafe {
        let null = core::ptr::null_mut();
        let mono = [0u8; 8];
        let mm = CreateBitmap(8, 16, 1, 1, mono.as_ptr() as *const c_void);
        let ii = ICONINFO { f_icon: 1, x_hotspot: 0, y_hotspot: 0, mask: mm, color: null };
        let ic = CreateIconIndirect(&ii);
        let cp = CopyIcon(ic);
        set_error(0x1234);
        let r = DestroyIcon(ic);
        say!(t, "DestroyIcon(created): {} error {:#x}", ly_b(r), last_error());
        set_error(0x1234);
        let r = DestroyIcon(ic);
        say!(t, "again: {} error {}", ly_b(r), last_error());
        ly_info(t, "GetIconInfo(destroyed)", ic);
        set_error(0x1234);
        let c2 = CopyIcon(ic);
        say!(t, "CopyIcon(destroyed): {} error {}", hwnd_kind(c2), last_error());
        set_error(0x1234);
        let r = DrawIconEx(CreateCompatibleDC(null), 0, 0, ic, 8, 8, 0, null, DI_NORMAL);
        say!(t, "DrawIconEx(destroyed): {} error {}", ly_b(r), last_error());
        ly_info(t, "the copy made before is still usable", cp);
        set_error(0x1234);
        let r = DestroyIcon(cp);
        say!(t, "DestroyIcon(copy): {} error {:#x}", ly_b(r), last_error());
        DeleteObject(mm);
    }
}
