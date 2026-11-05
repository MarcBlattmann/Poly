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

/// The screen of the session win32k reproduces (ADR 0131): the slice A
/// oracle's SSH session (session 0: 1024x768, no visible desktop) unless
/// the kernel is told to reproduce the interactive desktop session (slice
/// B: 1920x1080, the taskbar 24 pixels high at the top) or a compositor
/// connects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Display {
    pub width: i32,
    pub height: i32,
    /// The work area: the screen less the taskbar.
    pub work: Rect,
    /// Windows are really visible: painted, activated, given input.
    pub interactive: bool,
}

impl Display {
    /// Session 0 of the slice A oracle.
    pub const SERVICE: Display =
        Display { width: 1024, height: 768, work: Rect::new(0, 0, 1024, 768), interactive: false };
    /// The interactive session of the slice B oracle (GetSystemMetrics.yaml).
    pub const INTERACTIVE: Display =
        Display { width: 1920, height: 1080, work: Rect::new(0, 24, 1920, 1080), interactive: true };

    /// An interactive desktop of `width` x `height` with a top bar of `bar`.
    pub const fn desktop(width: i32, height: i32, bar: i32) -> Display {
        Display { width, height, work: Rect::new(0, bar, width, height), interactive: true }
    }
}

/// The screen when no compositor says otherwise (the slice A oracle's).
pub const DEFAULT_SCREEN: (i32, i32) = (1024, 768);

pub const CAPTION: i32 = 23;
pub const MENU: i32 = 20;
pub const SCROLLBAR: i32 = 17;
pub const MIN_TRACK: (i32, i32) = (136, 39);
/// The minimized window's slot (`ShowWindow.yaml`: 160x28).
pub const MINIMIZED: (i32, i32) = (160, 28);

/// `GetSystemMetrics(index)` (GetSystemMetrics.yaml, winuser.h's SM_*
/// indices); `None` for an index winuser.h does not define.
pub fn system_metric(index: i32, d: &Display) -> Option<i32> {
    let (w, h) = (d.width, d.height);
    let (ww, wh) = (d.work.width(), d.work.height());
    Some(match index {
        0 => w,                                // SM_CXSCREEN
        1 => h,                                // SM_CYSCREEN
        2 | 3 | 9 | 10 | 20 | 21 => SCROLLBAR, // scroll bars and thumbs
        4 => CAPTION,                          // SM_CYCAPTION
        5 | 6 => 1,                            // SM_CXBORDER, SM_CYBORDER
        7 | 8 => 3,                            // SM_CXDLGFRAME, SM_CYDLGFRAME
        11 | 12 => 32,                         // SM_CXICON, SM_CYICON
        13 | 14 => 32,                         // SM_CXCURSOR, SM_CYCURSOR
        15 => MENU,                            // SM_CYMENU
        16 => ww,                              // SM_CXFULLSCREEN
        17 => wh - CAPTION,                    // SM_CYFULLSCREEN
        18 => 0,                               // SM_CYKANJIWINDOW
        19 => 0,                               // SM_MOUSEPRESENT (no mouse on the oracle)
        22 => 0,                               // SM_DEBUG
        23 => 0,                               // SM_SWAPBUTTON
        24..=27 => 0,                          // SM_RESERVED1..4
        28 => MIN_TRACK.0,                     // SM_CXMIN
        29 => MIN_TRACK.1,                     // SM_CYMIN
        30 => 36,                              // SM_CXSIZE
        31 => 22,                              // SM_CYSIZE
        32 | 33 => 4,                          // SM_CXFRAME, SM_CYFRAME
        34 => MIN_TRACK.0,                     // SM_CXMINTRACK
        35 => MIN_TRACK.1,                     // SM_CYMINTRACK
        36 | 37 => 4,                          // SM_CXDOUBLECLK, SM_CYDOUBLECLK
        38 | 39 => 75,                         // SM_CXICONSPACING, SM_CYICONSPACING
        40 => 0,                               // SM_MENUDROPALIGNMENT
        41 => 0,                               // SM_PENWINDOWS
        42 => 0,                               // SM_DBCSENABLED
        43 => 0,                               // SM_CMOUSEBUTTONS
        44 => 0,                               // SM_SECURE
        45 | 46 => 2,                          // SM_CXEDGE, SM_CYEDGE
        47 => 160,                             // SM_CXMINSPACING
        48 => 28,                              // SM_CYMINSPACING
        49 | 50 => 16,                         // SM_CXSMICON, SM_CYSMICON
        51 => CAPTION,                         // SM_CYSMCAPTION
        52 | 53 => 22,                         // SM_CXSMSIZE, SM_CYSMSIZE
        54 | 55 => 19,                         // SM_CXMENUSIZE, SM_CYMENUSIZE
        56 => 8,                               // SM_ARRANGE (ARW_HIDE)
        57 => MINIMIZED.0,                     // SM_CXMINIMIZED
        58 => MINIMIZED.1,                     // SM_CYMINIMIZED
        59 => w + 20,                          // SM_CXMAXTRACK
        60 => h + 20,                          // SM_CYMAXTRACK
        61 => ww + 16,                         // SM_CXMAXIMIZED
        62 => wh + 16,                         // SM_CYMAXIMIZED
        63 => 1,                               // SM_NETWORK (bit 0)
        64..=66 => 0,                          // reserved
        67 => 0,                               // SM_CLEANBOOT
        68 | 69 => 4,                          // SM_CXDRAG, SM_CYDRAG
        70 => 0,                               // SM_SHOWSOUNDS
        71 | 72 => 15,                         // SM_CXMENUCHECK, SM_CYMENUCHECK
        73 => 0,                               // SM_SLOWMACHINE
        74 => 0,                               // SM_MIDEASTENABLED
        75 => 0,                               // SM_MOUSEWHEELPRESENT
        76 | 77 => 0,                          // SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN
        78 => w,                               // SM_CXVIRTUALSCREEN
        79 => h,                               // SM_CYVIRTUALSCREEN
        80 => 1,                               // SM_CMONITORS
        81 => 1,                               // SM_SAMEDISPLAYFORMAT
        82 => 1,                               // SM_IMMENABLED
        83 | 84 => 1,                          // SM_CXFOCUSBORDER, SM_CYFOCUSBORDER
        86 => 0,                               // SM_TABLETPC
        87 => 0,                               // SM_MEDIACENTER
        88 => 0,                               // SM_STARTER
        89 => 0,                               // SM_SERVERR2
        90 => 0,                               // reserved
        91 => 0,                               // SM_MOUSEHORIZONTALWHEELPRESENT
        92 => 4,                               // SM_CXPADDEDBORDER
        94 => 0,                               // SM_DIGITIZER
        95 => 0,                               // SM_MAXIMUMTOUCHES
        96 => 0,                               // reserved
        0x1000 => 0,                           // SM_REMOTESESSION
        0x2000 => 0,                           // SM_SHUTTINGDOWN
        0x2001 => 0,                           // SM_REMOTECONTROL
        0x2002 => 1,                           // SM_CARETBLINKINGENABLED
        0x2003 => 0,                           // SM_CONVERTIBLESLATEMODE
        0x2004 => 0,                           // SM_SYSTEMDOCKED
        0x2005 => 0,                           // reserved
        _ => return None,
    })
}

/// `GetSystemMetricsForDpi(index, dpi)`: the sizes scaled as the oracle
/// listed (GetSystemMetrics.yaml `for_dpi`), the rest as at 96 DPI.
pub fn system_metric_for_dpi(index: i32, dpi: u32, d: &Display) -> Option<i32> {
    let base = system_metric(index, d)?;
    let scaled = |v: i32| -> i32 { ((i64::from(v) * i64::from(dpi) + 48) / 96) as i32 };
    // Values the oracle listed at 96/120/144/192 that plain rounding does
    // not give.
    let table = |v: [i32; 4]| -> i32 {
        match dpi {
            96 => v[0],
            120 => v[1],
            144 => v[2],
            192 => v[3],
            _ => scaled(v[0]),
        }
    };
    Some(match index {
        2 | 3 | 9 | 10 | 20 | 21 => table([17, 21, 26, 34]),
        4 | 51 => match dpi {
            1 => 24,
            48 => 12,
            1000 => 230,
            _ => table([23, 29, 34, 45]),
        },
        11 | 12 => table([32, 40, 48, 64]),
        13 | 14 => table([32, 32, 48, 64]),
        15 => table([20, 25, 30, 39]),
        28 | 34 => table([136, 166, 198, 258]),
        29 | 35 => table([39, 47, 56, 71]),
        30 => table([36, 46, 54, 72]),
        31 | 52 | 53 => table([22, 28, 33, 44]),
        32 | 33 => table([4, 4, 5, 5]),
        38 | 39 => table([75, 94, 113, 150]),
        49 | 50 => table([16, 20, 24, 32]),
        54 | 55 => table([19, 24, 29, 38]),
        71 | 72 => table([15, 19, 25, 31]),
        92 => table([4, 5, 6, 8]),
        _ => base,
    })
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
/// (max size, max position, min track, max track). The maximized size is
/// the work area plus the frame on each side; the position is relative to
/// the work area (its origin is added when maximizing).
pub fn minmax(style: u32, ex: u32, d: &Display) -> [(i32, i32); 4] {
    let s = side(style, ex);
    [(d.work.width() + 2 * s, d.work.height() + 2 * s), (-s, -s), MIN_TRACK, (d.width + 20, d.height + 20)]
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
            minmax(WS_CAPTION | WS_THICKFRAME, 0, &Display::SERVICE),
            [(1040, 784), (-8, -8), (136, 39), (1044, 788)]
        );
    }
}
