//! GetIconInfo failures: a NULL, made-up, destroyed or non-icon handle
//! (a window, a bitmap) gives FALSE with ERROR_INVALID_CURSOR_HANDLE (1402)
//! and leaves the ICONINFO untouched.
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
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type BOOL = i32;
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
/// "(lo-hi)" style result text of a BOOL.
fn ly_b(v: BOOL) -> &'static str {
    if v != 0 { "TRUE" } else { "FALSE" }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDesktopWindow() -> HWND;
    fn CreateIconIndirect(info: *const ICONINFO) -> HICON;
    fn GetIconInfo(icon: HICON, info: *mut ICONINFO) -> BOOL;
    fn DestroyIcon(icon: HICON) -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn GetStockObject(index: i32) -> HGDIOBJ;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const c_void) -> HBITMAP;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe {
        let null = core::ptr::null_mut();
        let mk = || ICONINFO {
            f_icon: 9,
            x_hotspot: 0xAAAA,
            y_hotspot: 0xBBBB,
            mask: 1usize as HBITMAP,
            color: 2usize as HBITMAP,
        };
        let same = |i: &ICONINFO| {
            i.f_icon == 9
                && i.x_hotspot == 0xAAAA
                && i.y_hotspot == 0xBBBB
                && i.mask as usize == 1
                && i.color as usize == 2
        };
        for (lbl, h) in [
            ("NULL", null),
            ("made-up", 0x0bad_0bad_usize as HICON),
            ("1", 1usize as HICON),
            ("a window handle", GetDesktopWindow() as HICON),
            ("a bitmap handle", CreateBitmap(2, 2, 1, 1, core::ptr::null()) as HICON),
            ("a brush handle", GetStockObject(0) as HICON),
        ] {
            let mut i = mk();
            set_error(0x1234);
            let r = GetIconInfo(h, &mut i);
            say!(t, "{lbl}: {} error {} ICONINFO untouched {}", ly_b(r), last_error(), same(&i));
        }
        // destroyed
        let mono = [0u8; 8];
        let mm = CreateBitmap(8, 16, 1, 1, mono.as_ptr() as *const c_void);
        let ii = ICONINFO { f_icon: 1, x_hotspot: 0, y_hotspot: 0, mask: mm, color: null };
        let ic = CreateIconIndirect(&ii);
        DestroyIcon(ic);
        let mut i = mk();
        set_error(0x1234);
        let r = GetIconInfo(ic, &mut i);
        say!(t, "destroyed icon: {} error {} ICONINFO untouched {}", ly_b(r), last_error(), same(&i));
        DeleteObject(mm);
    }
}
