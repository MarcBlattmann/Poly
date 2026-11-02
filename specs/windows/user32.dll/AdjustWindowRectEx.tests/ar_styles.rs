//! AdjustWindowRectEx: the window rectangle for client rectangle
//! (100,100)-(400,300) for each frame style (caption, border, dialog
//! frame, thick frame, the extended edges, tool window, a menu bar),
//! scroll bars being ignored, WS_CHILD/WS_POPUP not mattering; the same
//! numbers through AdjustWindowRect; and the frame metrics they come from.
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
const WS_DLGFRAME: u32 = 0x0040_0000;
const WS_VSCROLL: u32 = 0x0020_0000;
const WS_HSCROLL: u32 = 0x0010_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const WS_EX_DLGMODALFRAME: u32 = 0x0000_0001;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_WINDOWEDGE: u32 = 0x0000_0100;
const WS_EX_CLIENTEDGE: u32 = 0x0000_0200;
const WS_EX_STATICEDGE: u32 = 0x0002_0000;
const WS_EX_OVERLAPPEDWINDOW: u32 = WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE;
const SM_CYCAPTION: i32 = 4;
const SM_CXBORDER: i32 = 5;
const SM_CYBORDER: i32 = 6;
const SM_CXDLGFRAME: i32 = 7;
const SM_CYDLGFRAME: i32 = 8;
const SM_CXFRAME: i32 = 32;
const SM_CYFRAME: i32 = 33;
const SM_CXPADDEDBORDER: i32 = 92;
const SM_CXEDGE: i32 = 45;
const SM_CYEDGE: i32 = 46;
const SM_CYMENU: i32 = 15;
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

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn AdjustWindowRectEx(rect: *mut RECT, style: u32, menu: BOOL, ex_style: u32) -> BOOL;
    fn AdjustWindowRect(rect: *mut RECT, style: u32, menu: BOOL) -> BOOL;
    fn GetSystemMetrics(index: i32) -> i32;
}

fn case(t: &mut Test, label: &str, style: u32, menu: i32, ex: u32) {
    let mut r = RECT { left: 100, top: 100, right: 400, bottom: 300 };
    // SAFETY: r is writable.
    let ok = unsafe { AdjustWindowRectEx(&mut r, style, menu, ex) };
    say!(
        t,
        "{label}: ok={} rect {},{},{},{} adds left={} top={} right={} bottom={}",
        ok,
        r.left,
        r.top,
        r.right,
        r.bottom,
        100 - r.left,
        100 - r.top,
        r.right - 400,
        r.bottom - 300
    );
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain queries.
    unsafe {
        say!(
            t,
            "metrics caption={} border={},{} dlgframe={},{} frame={},{} padded={} edge={},{} menu={}",
            GetSystemMetrics(SM_CYCAPTION),
            GetSystemMetrics(SM_CXBORDER),
            GetSystemMetrics(SM_CYBORDER),
            GetSystemMetrics(SM_CXDLGFRAME),
            GetSystemMetrics(SM_CYDLGFRAME),
            GetSystemMetrics(SM_CXFRAME),
            GetSystemMetrics(SM_CYFRAME),
            GetSystemMetrics(SM_CXPADDEDBORDER),
            GetSystemMetrics(SM_CXEDGE),
            GetSystemMetrics(SM_CYEDGE),
            GetSystemMetrics(SM_CYMENU)
        );
    }
    case(t, "none", 0, 0, 0);
    case(t, "popup", WS_POPUP, 0, 0);
    case(t, "child", WS_CHILD, 0, 0);
    case(t, "overlapped", WS_OVERLAPPED, 0, 0);
    case(t, "caption", WS_CAPTION, 0, 0);
    case(t, "border", WS_BORDER, 0, 0);
    case(t, "dlgframe", WS_DLGFRAME, 0, 0);
    case(t, "thickframe", WS_THICKFRAME, 0, 0);
    case(t, "popup_thickframe", WS_POPUP | WS_THICKFRAME, 0, 0);
    case(t, "border_thickframe", WS_BORDER | WS_THICKFRAME, 0, 0);
    case(t, "overlappedwindow", WS_OVERLAPPEDWINDOW, 0, 0);
    case(t, "overlappedwindow_menu", WS_OVERLAPPEDWINDOW, 1, 0);
    case(t, "popup_menu", WS_POPUP, 1, 0);
    case(t, "child_menu", WS_CHILD, 1, 0);
    case(t, "scrollbars", WS_VSCROLL | WS_HSCROLL, 0, 0);
    case(t, "clientedge", 0, 0, WS_EX_CLIENTEDGE);
    case(t, "staticedge", 0, 0, WS_EX_STATICEDGE);
    case(t, "windowedge", 0, 0, WS_EX_WINDOWEDGE);
    case(t, "dlgmodalframe", 0, 0, WS_EX_DLGMODALFRAME);
    case(t, "dlgmodalframe_border", WS_BORDER, 0, WS_EX_DLGMODALFRAME);
    case(t, "overlappedwindow_ex_overlapped", WS_OVERLAPPEDWINDOW, 0, WS_EX_OVERLAPPEDWINDOW);
    case(t, "caption_toolwindow", WS_CAPTION, 0, WS_EX_TOOLWINDOW);
    case(t, "popup_caption_sysmenu", WS_POPUP | WS_CAPTION | WS_SYSMENU, 0, 0);
    case(t, "border_clientedge", WS_BORDER, 0, WS_EX_CLIENTEDGE);
    case(t, "dlgframe_clientedge_staticedge", WS_DLGFRAME, 0, WS_EX_CLIENTEDGE | WS_EX_STATICEDGE);
    let mut a = RECT { left: 0, top: 0, right: 10, bottom: 10 };
    let mut b = a;
    // SAFETY: writable rects.
    unsafe {
        AdjustWindowRect(&mut a, WS_OVERLAPPEDWINDOW, 0);
        AdjustWindowRectEx(&mut b, WS_OVERLAPPEDWINDOW, 0, 0);
    }
    t.check("adjustwindowrect_is_ex_with_0", a == b, 0);
    let mut e = RECT { left: 5, top: 5, right: 0, bottom: 0 };
    // SAFETY: a writable rect.
    unsafe { AdjustWindowRectEx(&mut e, WS_CAPTION, 0, 0) };
    say!(t, "inverted client rect: {},{},{},{}", e.left, e.top, e.right, e.bottom);
}
