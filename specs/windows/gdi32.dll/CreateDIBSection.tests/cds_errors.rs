//! CreateDIBSection failures: bit counts 0, 7 and 64, planes 2, width 0,
//! height 0, negative width, an unknown compression and a too-small
//! biSize each give NULL or a bitmap (observed, with the last error and
//! whether the bits pointer was set); NULL ppvBits is allowed (observed);
//! a huge size fails.
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn CreateDIBSection(
        hdc: HDC,
        info: *const BITMAPINFO,
        usage: u32,
        bits: *mut *mut c_void,
        section: HANDLE,
        offset: u32,
    ) -> HBITMAP;
}

fn try_make(t: &mut Test, label: &str, size: u32, bpp: u16, planes: u16, w: i32, h: i32, comp: u32, want_bits: bool) {
    let info = BITMAPINFO {
        header: BITMAPINFOHEADER {
            size,
            width: w,
            height: h,
            planes,
            bit_count: bpp,
            compression: comp,
            size_image: 0,
            x_pels_per_meter: 0,
            y_pels_per_meter: 0,
            clr_used: 0,
            clr_important: 0,
        },
        colors: [0],
    };
    let mut bits: *mut c_void = 1 as *mut c_void;
    set_error(0);
    // SAFETY: a BITMAPINFO (maybe invalid on purpose); bits is writable or NULL.
    let b = unsafe {
        CreateDIBSection(
            core::ptr::null_mut(),
            &info,
            DIB_RGB_COLORS,
            if want_bits { &mut bits } else { core::ptr::null_mut() },
            core::ptr::null_mut(),
            0,
        )
    };
    let e = last_error();
    let bits_state = if !want_bits {
        "n/a"
    } else if bits.is_null() {
        "null"
    } else if bits as usize == 1 {
        "untouched"
    } else {
        "set"
    };
    say!(t, "{label}: {} error={} bits={}", hwnd_kind(b), e, bits_state);
    if !b.is_null() {
        // SAFETY: our bitmap.
        unsafe { DeleteObject(b) };
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    try_make(t, "valid 32bpp 4x4", 40, 32, 1, 4, 4, BI_RGB, true);
    try_make(t, "bpp 0", 40, 0, 1, 4, 4, BI_RGB, true);
    try_make(t, "bpp 7", 40, 7, 1, 4, 4, BI_RGB, true);
    try_make(t, "bpp 64", 40, 64, 1, 4, 4, BI_RGB, true);
    try_make(t, "planes 2", 40, 32, 2, 4, 4, BI_RGB, true);
    try_make(t, "width 0", 40, 32, 1, 0, 4, BI_RGB, true);
    try_make(t, "height 0", 40, 32, 1, 4, 0, BI_RGB, true);
    try_make(t, "width -4", 40, 32, 1, -4, 4, BI_RGB, true);
    try_make(t, "compression 99", 40, 32, 1, 4, 4, 99, true);
    try_make(t, "biSize 12", 12, 32, 1, 4, 4, BI_RGB, true);
    try_make(t, "biSize 39", 39, 32, 1, 4, 4, BI_RGB, true);
    try_make(t, "NULL ppvBits", 40, 32, 1, 4, 4, BI_RGB, false);
    try_make(t, "huge 100000x100000", 40, 32, 1, 100_000, 100_000, BI_RGB, true);
}
