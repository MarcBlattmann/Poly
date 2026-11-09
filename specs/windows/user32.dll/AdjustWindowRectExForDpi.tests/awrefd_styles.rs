//! AdjustWindowRectExForDpi on the client rectangle (0,0,100,100) for twelve
//! style/extended style combinations, with and without a menu bar, at the DPIs
//! 96, 120, 144, 168, 192, 240, 288 and 384: the rectangle grows by the frame,
//! caption and menu heights of that DPI; popup and child windows without
//! WS_CAPTION or WS_BORDER get no frame; the result is TRUE. AdjustWindowRectEx
//! for comparison at the system DPI.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::fmt::Write as _;
use poly_wtest::*;

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

const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const WS_EX_DLGMODALFRAME: u32 = 0x0000_0001;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_CLIENTEDGE: u32 = 0x0000_0200;
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

/// "l,t,r,b" of a rectangle.
struct R(RECT);

impl core::fmt::Display for R {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{},{},{},{}", self.0.left, self.0.top, self.0.right, self.0.bottom)
    }
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn AdjustWindowRectEx(rect: *mut RECT, style: u32, menu: BOOL, ex_style: u32) -> BOOL;
    fn AdjustWindowRectExForDpi(rect: *mut RECT, style: u32, menu: BOOL, ex_style: u32, dpi: u32) -> BOOL;
}

poly_wtest::main!(test);

const STYLES: [(&str, u32, u32); 12] = [
    ("overlapped", WS_OVERLAPPED, 0),
    ("overlappedwindow", WS_OVERLAPPEDWINDOW, 0),
    ("caption", WS_CAPTION, 0),
    ("popup", WS_POPUP, 0),
    ("popup+border", WS_POPUP | WS_BORDER, 0),
    ("popup+thickframe", WS_POPUP | WS_THICKFRAME, 0),
    ("popup+caption", WS_POPUP | WS_CAPTION, 0),
    ("child", WS_CHILD, 0),
    ("child+caption", WS_CHILD | WS_CAPTION, 0),
    ("overlappedwindow+clientedge", WS_OVERLAPPEDWINDOW, WS_EX_CLIENTEDGE),
    ("overlappedwindow+toolwindow", WS_OVERLAPPEDWINDOW, WS_EX_TOOLWINDOW),
    ("popup+caption+dlgmodalframe", WS_POPUP | WS_CAPTION, WS_EX_DLGMODALFRAME),
];

fn test(t: &mut Test) {
    let dpis = [96u32, 120, 144, 168, 192, 240, 288, 384];
    for (name, st, ex) in STYLES {
        for menu in [0, 1] {
            let mut row = Buf::<300>::new();
            for d in dpis {
                let mut r = RECT { left: 0, top: 0, right: 100, bottom: 100 };
                // SAFETY: r is writable.
                let ok = unsafe { AdjustWindowRectExForDpi(&mut r, st, menu, ex, d) };
                let _ = write!(row, " {}:{}:{}", d, ok, R(r));
            }
            say!(t, "{name} menu {menu}:{}", row.as_str());
        }
        let mut r = RECT { left: 0, top: 0, right: 100, bottom: 100 };
        // SAFETY: r is writable.
        let ok = unsafe { AdjustWindowRectEx(&mut r, st, 0, ex) };
        say!(t, "  AdjustWindowRectEx (system DPI, no menu) -> {} {}", ok, R(r));
    }
}
