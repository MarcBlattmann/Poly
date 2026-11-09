//! LoadImageW(NULL, OCR_*, IMAGE_CURSOR, ...) with LR_SHARED: the handle
//! equals LoadCursorW's for the same id (also with LR_DEFAULTSIZE); fIcon is
//! 0 and the hotspots are those of the standard cursors (arrow 0,0; ibeam
//! 8,9; wait and cross 16,16; up arrow 4,0; size-all 11,11; no 8,8; hand
//! 6,0); the colour cursors have 32x32 mask and 32-bit colour bitmaps, ibeam
//! and cross have a single 32x64 mask bitmap and no colour bitmap; the size
//! asked for (16, 32, 48, 64, 10x40) does not change the shared 32x32 image;
//! without LR_SHARED the system cursor ids do not load (NULL with
//! ERROR_RESOURCE_NAME_NOT_FOUND 1814); DestroyCursor on a shared cursor
//! returns TRUE and the handle stays usable; the ids 32512 as cursor and as
//! icon give OCR_NORMAL and IDI_APPLICATION; CopyImage and CopyIcon make new
//! handles (CopyImage at 16x16: size 16x16 and hotspot 3,0 for the hand).
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
type HINSTANCE = *mut c_void;
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

/// This executable's instance handle.
fn instance() -> HINSTANCE {
    // SAFETY: NULL names the executable.
    unsafe { GetModuleHandleW(core::ptr::null()) }
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

type HCURSOR = *mut c_void;
const SM_CXCURSOR: i32 = 13;
const SM_CYCURSOR: i32 = 14;
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
const IMAGE_ICON: u32 = 1;
const IMAGE_CURSOR: u32 = 2;
const LR_DEFAULTSIZE: u32 = 0x0040;
const LR_SHARED: u32 = 0x8000;
/// winuser.h: system icon ids (IDI_*) and cursor ids (OCR_*).
const IDI_APPLICATION: usize = 32512;

const OCR_NORMAL: usize = 32512;
const OCR_IBEAM: usize = 32513;
const OCR_WAIT: usize = 32514;
const OCR_CROSS: usize = 32515;
const OCR_UP: usize = 32516;
const OCR_SIZEALL: usize = 32646;
const OCR_NO: usize = 32648;
const OCR_HAND: usize = 32649;
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
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn LoadCursorW(instance: HINSTANCE, name: *const u16) -> HCURSOR;
    fn DestroyCursor(cursor: HCURSOR) -> BOOL;
    fn LoadImageW(inst: HINSTANCE, name: *const u16, kind: u32, cx: i32, cy: i32, flags: u32) -> HANDLE;
    fn LoadIconW(inst: HINSTANCE, name: *const u16) -> HICON;
    fn GetIconInfo(icon: HICON, info: *mut ICONINFO) -> BOOL;
    fn DestroyIcon(icon: HICON) -> BOOL;
    fn CopyIcon(icon: HICON) -> HICON;
    fn CopyImage(h: HANDLE, kind: u32, cx: i32, cy: i32, flags: u32) -> HANDLE;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: system resources.
    unsafe {
        let null = core::ptr::null_mut();
        say!(
            t,
            "metrics: SM_CXCURSOR {} SM_CYCURSOR {}",
            GetSystemMetrics(SM_CXCURSOR),
            GetSystemMetrics(SM_CXCURSOR + 1)
        );
        for (n, cid) in [
            ("NORMAL", OCR_NORMAL),
            ("IBEAM", OCR_IBEAM),
            ("WAIT", OCR_WAIT),
            ("CROSS", OCR_CROSS),
            ("UP", OCR_UP),
            ("SIZEALL", OCR_SIZEALL),
            ("NO", OCR_NO),
            ("HAND", OCR_HAND),
        ] {
            set_error(0x1234);
            let h = LoadImageW(null, cid as *const u16, IMAGE_CURSOR, 0, 0, LR_SHARED);
            let lc = LoadCursorW(null, cid as *const u16);
            let h2 = LoadImageW(null, cid as *const u16, IMAGE_CURSOR, 0, 0, LR_SHARED | LR_DEFAULTSIZE);
            say!(t, "OCR_{n}: {} same as LoadCursorW {} same with LR_DEFAULTSIZE {}", hwnd_kind(h), h == lc, h == h2);
            ly_info(t, "   shape", h);
        }
        for (lbl, cx, cy) in
            [("16x16", 16, 16), ("32x32", 32, 32), ("48x48", 48, 48), ("64x64", 64, 64), ("10x40", 10, 40)]
        {
            let h = LoadImageW(null, OCR_NORMAL as *const u16, IMAGE_CURSOR, cx, cy, LR_SHARED);
            ly_info(t, lbl, h);
        }
        set_error(0x1234);
        let h = LoadImageW(null, OCR_NORMAL as *const u16, IMAGE_CURSOR, 0, 0, 0);
        say!(t, "without LR_SHARED: {} error {}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(null, OCR_NORMAL as *const u16, IMAGE_CURSOR, 32, 32, LR_DEFAULTSIZE);
        say!(t, "LR_DEFAULTSIZE without LR_SHARED: {} error {}", hwnd_kind(h), last_error());
        // DestroyCursor of a shared handle
        let hand = LoadImageW(null, OCR_HAND as *const u16, IMAGE_CURSOR, 0, 0, LR_SHARED) as HCURSOR;
        set_error(0x1234);
        let r = DestroyCursor(hand);
        say!(t, "DestroyCursor(shared): {} error {:#x}", ly_b(r), last_error());
        ly_info(t, "   still usable", hand);
        let again = LoadImageW(null, OCR_HAND as *const u16, IMAGE_CURSOR, 0, 0, LR_SHARED);
        say!(t, "loaded again: same handle {}", again == hand);
        // the cursor icons ids as IMAGE_CURSOR with the icon id numbers overlap
        let ic = LoadImageW(null, IDI_APPLICATION as *const u16, IMAGE_CURSOR, 0, 0, LR_SHARED);
        say!(t, "id 32512 as cursor equals OCR_NORMAL: {}", ic == LoadCursorW(null, OCR_NORMAL as *const u16));
        let ic = LoadImageW(null, OCR_NORMAL as *const u16, IMAGE_ICON, 0, 0, LR_SHARED);
        say!(t, "id 32512 as icon equals IDI_APPLICATION: {}", ic == LoadIconW(null, IDI_APPLICATION as *const u16));
        // CopyImage of a cursor
        let c = CopyImage(hand as HANDLE, IMAGE_CURSOR, 0, 0, 0);
        say!(t, "CopyImage(cursor): {} new handle {}", hwnd_kind(c), c != hand as HANDLE);
        ly_info(t, "   copy", c);
        let c2 = CopyImage(hand as HANDLE, IMAGE_CURSOR, 16, 16, 0);
        ly_info(t, "   copy 16x16", c2);
        DestroyCursor(c);
        DestroyCursor(c2);
        let c3 = CopyIcon(hand);
        say!(t, "CopyIcon(cursor): {}", hwnd_kind(c3));
        ly_info(t, "   copy", c3);
        DestroyIcon(c3);
    }
}
