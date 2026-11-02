//! System metrics and the frame of a window's non-client area, as the
//! spec sheets observed them on the oracle (96 DPI, 1024x768): `SM_CYCAPTION`
//! 23, `SM_CXBORDER` 1, `SM_CXDLGFRAME` 3, `SM_CXFRAME` 4,
//! `SM_CXPADDEDBORDER` 4, `SM_CXEDGE` 2, `SM_CYMENU` 20; what
//! `AdjustWindowRectEx` adds for each style (`AdjustWindowRectEx.yaml`) and
//! what the default `WM_NCCALCSIZE` leaves as the client area
//! (`GetClientRect.yaml`: scroll bars 17 pixels).
//!
//! Constants are `winuser.h`'s.

use crate::frame::Rect;

pub const WS_OVERLAPPED: u32 = 0x0000_0000;
pub const WS_POPUP: u32 = 0x8000_0000;
pub const WS_CHILD: u32 = 0x4000_0000;
pub const WS_MINIMIZE: u32 = 0x2000_0000;
pub const WS_VISIBLE: u32 = 0x1000_0000;
pub const WS_DISABLED: u32 = 0x0800_0000;
pub const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
pub const WS_CLIPCHILDREN: u32 = 0x0200_0000;
pub const WS_MAXIMIZE: u32 = 0x0100_0000;
pub const WS_CAPTION: u32 = 0x00C0_0000;
pub const WS_BORDER: u32 = 0x0080_0000;
pub const WS_DLGFRAME: u32 = 0x0040_0000;
pub const WS_VSCROLL: u32 = 0x0020_0000;
pub const WS_HSCROLL: u32 = 0x0010_0000;
pub const WS_SYSMENU: u32 = 0x0008_0000;
pub const WS_THICKFRAME: u32 = 0x0004_0000;

pub const WS_EX_DLGMODALFRAME: u32 = 0x0000_0001;
pub const WS_EX_NOPARENTNOTIFY: u32 = 0x0000_0004;
pub const WS_EX_TOPMOST: u32 = 0x0000_0008;
pub const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
pub const WS_EX_WINDOWEDGE: u32 = 0x0000_0100;
pub const WS_EX_CLIENTEDGE: u32 = 0x0000_0200;
pub const WS_EX_STATICEDGE: u32 = 0x0002_0000;
pub const WS_EX_NOACTIVATE: u32 = 0x0800_0000;

/// The screen when no compositor says otherwise (the oracle's).
pub const DEFAULT_SCREEN: (i32, i32) = (1024, 768);

pub const CAPTION: i32 = 23;
pub const MENU: i32 = 20;
pub const SCROLLBAR: i32 = 17;
pub const MIN_TRACK: (i32, i32) = (136, 39);
/// The minimized window's slot (`ShowWindow.yaml`: 160x28).
pub const MINIMIZED: (i32, i32) = (160, 28);

/// `GetSystemMetrics(index)` on a screen of `screen` pixels.
pub fn system_metric(index: i32, screen: (i32, i32)) -> i32 {
    match index {
        0 => screen.0,            // SM_CXSCREEN
        1 => screen.1,            // SM_CYSCREEN
        2 | 3 => SCROLLBAR,       // SM_CXVSCROLL, SM_CYHSCROLL
        4 => CAPTION,             // SM_CYCAPTION
        5 | 6 => 1,               // SM_CXBORDER, SM_CYBORDER
        7 | 8 => 3,               // SM_CXDLGFRAME, SM_CYDLGFRAME
        9 | 10 => SCROLLBAR,      // SM_CYVTHUMB, SM_CXHTHUMB
        15 => MENU,               // SM_CYMENU
        16 => screen.0,           // SM_CXFULLSCREEN
        17 => screen.1 - CAPTION, // SM_CYFULLSCREEN
        20 | 21 => SCROLLBAR,     // SM_CYVSCROLL, SM_CXHSCROLL
        32 | 33 => 4,             // SM_CXFRAME, SM_CYFRAME
        34 => MIN_TRACK.0,        // SM_CXMINTRACK
        35 => MIN_TRACK.1,        // SM_CYMINTRACK
        45 | 46 => 2,             // SM_CXEDGE, SM_CYEDGE
        57 => MINIMIZED.0,        // SM_CXMINIMIZED
        58 => MINIMIZED.1,        // SM_CYMINIMIZED
        59 => screen.0 + 20,      // SM_CXMAXTRACK
        60 => screen.1 + 20,      // SM_CYMAXTRACK
        61 => screen.0 + 16,      // SM_CXMAXIMIZED
        62 => screen.1 + 16,      // SM_CYMAXIMIZED
        78 => screen.0,           // SM_CXVIRTUALSCREEN
        79 => screen.1,           // SM_CYVIRTUALSCREEN
        80 => 1,                  // SM_CMONITORS
        92 => 4,                  // SM_CXPADDEDBORDER
        _ => 0,
    }
}

/// What a frame adds on each side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Insets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Whether a style has a caption (both `WS_BORDER` and `WS_DLGFRAME`).
pub fn has_caption(style: u32) -> bool {
    style & WS_CAPTION == WS_CAPTION
}

/// The width of the frame on each side for `style`/`ex_style`.
fn side(style: u32, ex: u32) -> i32 {
    let caption = has_caption(style);
    let thick = style & WS_THICKFRAME != 0;
    let border = style & WS_BORDER != 0;
    let dlg = style & WS_DLGFRAME != 0 || ex & WS_EX_DLGMODALFRAME != 0;
    let outer = if caption || (thick && border) {
        8
    } else if thick {
        7
    } else if dlg {
        3
    } else if border {
        1
    } else {
        0
    };
    let client = if ex & WS_EX_CLIENTEDGE != 0 { 2 } else { 0 };
    let mut stat = if ex & WS_EX_STATICEDGE != 0 { 1 } else { 0 };
    // Observed: a dialog frame with both edges adds 4 in all, not 6.
    if stat != 0 && dlg && !caption && !thick && client != 0 {
        stat = -1;
    }
    outer + client + stat
}

/// `AdjustWindowRectEx`'s insets: the frame, the caption, a menu bar.
pub fn frame_insets(style: u32, menu: bool, ex: u32) -> Insets {
    let s = side(style, ex);
    let mut top = s;
    if has_caption(style) {
        top += CAPTION;
    }
    if menu {
        top += MENU;
    }
    Insets { left: s, top, right: s, bottom: s }
}

/// The default `WM_NCCALCSIZE`'s insets: the frame plus scroll bars (and
/// a menu bar when the window has a menu).
pub fn client_insets(style: u32, ex: u32, menu: bool) -> Insets {
    let mut i = frame_insets(style, menu, ex);
    if style & WS_VSCROLL != 0 {
        i.right += SCROLLBAR;
    }
    if style & WS_HSCROLL != 0 {
        i.bottom += SCROLLBAR;
    }
    i
}

/// `AdjustWindowRectEx`.
pub fn adjust_window_rect(r: Rect, style: u32, menu: bool, ex: u32) -> Rect {
    let i = frame_insets(style, menu, ex);
    Rect::new(r.left - i.left, r.top - i.top, r.right + i.right, r.bottom + i.bottom)
}

/// The default `WM_NCCALCSIZE`: the client rectangle inside window
/// rectangle `w`; a window smaller than its frame has an empty client area
/// at its top-left inside corner.
pub fn default_client(w: Rect, style: u32, ex: u32, menu: bool) -> Rect {
    let i = client_insets(style, ex, menu);
    let left = w.left + i.left;
    let top = w.top + i.top;
    // Smaller than its frame: each side is empty on its own (observed: a
    // 3x3 captioned window has a 0x0 client area, an overlapped 136x39
    // one 120x0).
    let right = (w.right - i.right).max(left);
    let bottom = (w.bottom - i.bottom).max(top);
    Rect::new(left, top, right, bottom)
}

/// Whether a framed top-level window's size is held to its minimum and
/// maximum track sizes: an overlapped window, or one with a sizing border
/// (observed: an overlapped window asked 0x0 is 136x39; a captioned popup
/// asked 3x3 keeps its size).
pub fn track_limited(style: u32) -> bool {
    style & WS_POPUP == 0 || style & WS_THICKFRAME != 0
}

/// The `MINMAXINFO` win32k fills before `WM_GETMINMAXINFO`:
/// (max size, max position, min track, max track).
pub fn minmax(style: u32, ex: u32, screen: (i32, i32)) -> [(i32, i32); 4] {
    let s = side(style, ex);
    [(screen.0 + 2 * s, screen.1 + 2 * s), (-s, -s), MIN_TRACK, (screen.0 + 20, screen.1 + 20)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adds(style: u32, menu: bool, ex: u32) -> (i32, i32, i32, i32) {
        let r = adjust_window_rect(Rect::new(100, 100, 400, 300), style, menu, ex);
        (100 - r.left, 100 - r.top, r.right - 400, r.bottom - 300)
    }

    #[test]
    fn adjust_window_rect_matches_the_sheet() {
        assert_eq!(adds(0, false, 0), (0, 0, 0, 0));
        assert_eq!(adds(WS_POPUP, false, 0), (0, 0, 0, 0));
        assert_eq!(adds(WS_CHILD, false, 0), (0, 0, 0, 0));
        assert_eq!(adds(WS_CAPTION, false, 0), (8, 31, 8, 8));
        assert_eq!(adds(WS_BORDER, false, 0), (1, 1, 1, 1));
        assert_eq!(adds(WS_DLGFRAME, false, 0), (3, 3, 3, 3));
        assert_eq!(adds(WS_THICKFRAME, false, 0), (7, 7, 7, 7));
        assert_eq!(adds(WS_BORDER | WS_THICKFRAME, false, 0), (8, 8, 8, 8));
        let ow = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | 0x0003_0000;
        assert_eq!(adds(ow, false, 0), (8, 31, 8, 8));
        assert_eq!(adds(ow, true, 0), (8, 51, 8, 8));
        assert_eq!(adds(WS_CHILD, true, 0), (0, 20, 0, 0));
        assert_eq!(adds(WS_VSCROLL | WS_HSCROLL, false, 0), (0, 0, 0, 0));
        assert_eq!(adds(0, false, WS_EX_CLIENTEDGE), (2, 2, 2, 2));
        assert_eq!(adds(0, false, WS_EX_STATICEDGE), (1, 1, 1, 1));
        assert_eq!(adds(0, false, WS_EX_WINDOWEDGE), (0, 0, 0, 0));
        assert_eq!(adds(0, false, WS_EX_DLGMODALFRAME), (3, 3, 3, 3));
        assert_eq!(adds(WS_BORDER, false, WS_EX_DLGMODALFRAME), (3, 3, 3, 3));
        assert_eq!(adds(ow, false, WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE), (10, 33, 10, 10));
        assert_eq!(adds(WS_CAPTION, false, WS_EX_TOOLWINDOW), (8, 31, 8, 8));
        assert_eq!(adds(WS_BORDER, false, WS_EX_CLIENTEDGE), (3, 3, 3, 3));
        assert_eq!(adds(WS_DLGFRAME, false, WS_EX_CLIENTEDGE | WS_EX_STATICEDGE), (4, 4, 4, 4));
        // An inverted rectangle the same way.
        assert_eq!(adjust_window_rect(Rect::new(5, 5, 0, 0), WS_CAPTION, false, 0), Rect::new(-3, -26, 8, 8));
    }

    #[test]
    fn client_areas_match_the_sheet() {
        let w = Rect::new(0, 0, 200, 100);
        let size = |s, e| {
            let c = default_client(w, s, e, false);
            (c.width(), c.height())
        };
        assert_eq!(size(WS_POPUP, 0), (200, 100));
        assert_eq!(size(WS_POPUP | WS_BORDER, 0), (198, 98));
        assert_eq!(size(WS_POPUP | WS_CAPTION, 0), (184, 61));
        assert_eq!(size(WS_POPUP | WS_THICKFRAME, 0), (186, 86));
        assert_eq!(size(WS_POPUP, WS_EX_CLIENTEDGE), (196, 96));
        assert_eq!(size(WS_POPUP | WS_VSCROLL, 0), (183, 100));
        assert_eq!(size(WS_POPUP | WS_VSCROLL | WS_HSCROLL, 0), (183, 83));
        let tiny = default_client(Rect::new(10, 10, 14, 14), WS_CAPTION, 0, false);
        assert!(tiny.is_empty());
        // An overlapped window at its minimum track size: 120x0 at 8,31.
        let min = default_client(Rect::new(0, 0, 136, 39), WS_CAPTION | WS_THICKFRAME, 0, false);
        assert_eq!(min, Rect::new(8, 31, 128, 31));
        assert!(track_limited(WS_CAPTION) && track_limited(WS_POPUP | WS_THICKFRAME));
        assert!(!track_limited(WS_POPUP | WS_CAPTION));
        assert_eq!(
            minmax(WS_CAPTION | WS_THICKFRAME, 0, DEFAULT_SCREEN),
            [(1040, 784), (-8, -8), (136, 39), (1044, 788)]
        );
    }
}
