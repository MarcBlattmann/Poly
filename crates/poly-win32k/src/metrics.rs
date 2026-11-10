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
pub const WS_MINIMIZEBOX: u32 = 0x0002_0000;

pub const WS_EX_DLGMODALFRAME: u32 = 0x0000_0001;
pub const WS_EX_NOPARENTNOTIFY: u32 = 0x0000_0004;
pub const WS_EX_TOPMOST: u32 = 0x0000_0008;
pub const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
pub const WS_EX_WINDOWEDGE: u32 = 0x0000_0100;
pub const WS_EX_CLIENTEDGE: u32 = 0x0000_0200;
// Extended layout styles (`winuser.h`).
pub const WS_EX_NOINHERITLAYOUT: u32 = 0x0010_0000;
pub const WS_EX_LAYOUTRTL: u32 = 0x0040_0000;
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
        63 => {
            if d.interactive {
                3
            } else {
                1
            }
        } // SM_NETWORK (bits)
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
        85 => 4,                               // undefined in winuser.h (same on the oracle)
        93 => 516,                             // undefined in winuser.h (same on the oracle)
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

/// What a frame adds on each side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Insets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// The DPIs the sheet (`GetSystemMetricsForDpi.yaml` `scaling_table`)
/// lists values for.
const DPIS: [u32; 8] = [96, 120, 144, 168, 192, 240, 288, 384];

/// `round(v * dpi / 96)`, halves up.
fn scale(v: i64, dpi: u32) -> i32 {
    ((v * i64::from(dpi) + 48) / 96).clamp(i64::MIN / 2, i64::from(i32::MAX)) as i32
}

/// A metric that grows with the DPI: the sheet's value at one of the
/// listed DPIs, else `v96` scaled.
fn listed(dpi: u32, row: [i32; 8], v96: i64) -> i32 {
    match DPIS.iter().position(|&d| d == dpi) {
        Some(k) => row[k],
        None => scale(v96, dpi),
    }
}

/// The caption height (`SM_CYCAPTION`) at `dpi`: a 22-pixel button
/// height scaled, plus the 1-pixel border; the tiny DPIs 1 and 2 give 24.
pub fn caption_for_dpi(dpi: u32) -> i32 {
    if dpi <= 2 { 24 } else { scale(22, dpi).saturating_add(1) }
}

/// `SM_CXFRAME` at `dpi`: 3 and the scaled border (round(dpi / 96)).
pub fn frame_for_dpi(dpi: u32) -> i32 {
    3 + scale(1, dpi)
}

/// `SM_CXPADDEDBORDER` at `dpi` (capped at 100).
pub fn padded_for_dpi(dpi: u32) -> i32 {
    scale(4, dpi).min(100)
}

/// `SM_CYMENU` at `dpi`: the scaled 19 plus the 1-pixel border.
pub fn menu_for_dpi(dpi: u32) -> i32 {
    match dpi {
        0..=2 => 24,
        24 => 7,
        47 => 11,
        _ => scale(19, dpi).saturating_add(1),
    }
}

/// The cursor size at `dpi` (`SM_CXCURSOR`): 32, 48, 64, 96 or 128.
pub fn cursor_for_dpi(dpi: u32) -> i32 {
    match dpi {
        0..=143 => 32,
        144..=191 => 48,
        192..=287 => 64,
        288..=383 => 96,
        _ => 128,
    }
}

/// The DPI `GetSystemMetricsForDpi` works with for its `dpi` argument:
/// 0, 0x10000 and 0xFFFFFFFF are refused (ERROR_INVALID_PARAMETER), every
/// other value above 0xFFFF counts as 0xFFFF (0x7FFFFFFF answers like
/// 0xFFFF; `GetSystemMetricsForDpi.yaml` `bad_dpi`, `odd_dpis`).
pub fn metric_dpi(dpi: u32) -> Option<u32> {
    if dpi == 0 || dpi == 0x10000 || dpi == u32::MAX { None } else { Some(dpi.min(0xFFFF)) }
}

/// `GetSystemMetricsForDpi(index, dpi)`: the sizes scaled as the oracle
/// listed them (`GetSystemMetricsForDpi.yaml`), the rest as at 96 DPI.
pub fn system_metric_for_dpi(index: i32, dpi: u32, d: &Display) -> Option<i32> {
    let base = system_metric(index, d)?;
    Some(match index {
        2 | 3 | 9 | 10 | 20 | 21 => scale(17, dpi),
        4 | 51 => caption_for_dpi(dpi),
        11 | 12 => scale(32, dpi),
        13 | 14 => cursor_for_dpi(dpi),
        15 => menu_for_dpi(dpi),
        28 | 34 => listed(dpi, [136, 166, 198, 232, 258, 324, 384, 510], 136),
        29 | 35 => listed(dpi, [39, 47, 56, 64, 71, 88, 103, 135], 39),
        30 => listed(dpi, [36, 46, 54, 64, 72, 91, 109, 145], 36),
        31 | 52 | 53 => scale(22, dpi),
        32 | 33 => frame_for_dpi(dpi),
        38 | 39 => scale(75, dpi),
        49 | 50 => scale(16, dpi),
        54 | 55 => scale(19, dpi),
        71 | 72 => listed(dpi, [15, 19, 25, 29, 31, 41, 47, 65], 15),
        92 => padded_for_dpi(dpi),
        _ => base,
    })
}

/// Whether a style has a caption (both `WS_BORDER` and `WS_DLGFRAME`).
pub fn has_caption(style: u32) -> bool {
    style & WS_CAPTION == WS_CAPTION
}

/// The frame width on each side at `dpi` (`AdjustWindowRectExForDpi.yaml`):
/// a caption or thick frame with a border is the frame plus the padded
/// border, a thick frame alone one less.
fn side_dpi(style: u32, ex: u32, dpi: u32) -> i32 {
    let caption = has_caption(style);
    let thick = style & WS_THICKFRAME != 0;
    let dlg = style & WS_DLGFRAME != 0 || ex & WS_EX_DLGMODALFRAME != 0;
    let outer = frame_side_dpi(style, ex, dpi);
    let client = if ex & WS_EX_CLIENTEDGE != 0 { 2 } else { 0 };
    let mut stat = if ex & WS_EX_STATICEDGE != 0 { 1 } else { 0 };
    // Observed: a dialog frame with both edges adds 4 in all, not 6.
    if stat != 0 && client != 0 {
        stat = if dlg && !caption && !thick { -1 } else { 0 };
    }
    outer + client + stat
}

/// `AdjustWindowRectEx`'s insets: the frame, the caption, a menu bar.
pub fn frame_insets(style: u32, menu: bool, ex: u32) -> Insets {
    frame_insets_dpi(style, menu, ex, 96)
}

/// [`frame_insets`] at `dpi` (`AdjustWindowRectExForDpi`).
pub fn frame_insets_dpi(style: u32, menu: bool, ex: u32, dpi: u32) -> Insets {
    let s = side_dpi(style, ex, dpi);
    let mut top = s;
    if has_caption(style) {
        top += caption_for_dpi(dpi);
    }
    if menu {
        top += menu_for_dpi(dpi);
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

/// `AdjustWindowRectExForDpi`: the amounts added with wrap-around
/// arithmetic, as observed for rectangles at the edge of the range.
pub fn adjust_window_rect_dpi(r: Rect, style: u32, menu: bool, ex: u32, dpi: u32) -> Rect {
    let i = frame_insets_dpi(style, menu, ex, dpi);
    Rect::new(
        r.left.wrapping_sub(i.left),
        r.top.wrapping_sub(i.top),
        r.right.wrapping_add(i.right),
        r.bottom.wrapping_add(i.bottom),
    )
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

/// The width of a window's frame proper on each side (caption frame,
/// sizing frame, dialog frame or border line), without the client and
/// static edges of the extended style.
fn frame_side(style: u32, ex: u32) -> i32 {
    frame_side_dpi(style, ex, 96)
}

/// [`frame_side`] at `dpi` (`AdjustWindowRectExForDpi.yaml`): a caption or
/// a thick frame with a border is the frame plus the padded border (8 at
/// 96 DPI), a thick frame alone one less.
fn frame_side_dpi(style: u32, ex: u32, dpi: u32) -> i32 {
    let fp = frame_for_dpi(dpi).saturating_add(padded_for_dpi(dpi));
    let caption = has_caption(style);
    let thick = style & WS_THICKFRAME != 0;
    let border = style & WS_BORDER != 0;
    let dlg = style & WS_DLGFRAME != 0 || ex & WS_EX_DLGMODALFRAME != 0;
    if caption || (thick && border) {
        fp
    } else if thick {
        fp - 1
    } else if dlg {
        3
    } else if border {
        1
    } else {
        0
    }
}

/// The `MINMAXINFO` win32k fills before `WM_GETMINMAXINFO`:
/// (max size, max position, min track, max track), as GetSystemMetrics.yaml
/// `window_metrics` and ShowWindow.yaml `show_maximized_from_hidden` list
/// them. The maximized size is the screen plus the frame on each side, at
/// minus the frame (the frame proper: a client edge does not count). The
/// smallest tracking size is SM_CXMINTRACK x SM_CYMINTRACK for a window
/// with a border line or a dialog frame in its style (a caption is both),
/// twice the frame for a tool window, and twice the frame but at least 2
/// for the others (a plain popup: 2,2; one with a sizing frame: 14,14).
pub fn minmax(style: u32, ex: u32, d: &Display) -> [(i32, i32); 4] {
    let s = frame_side(style, ex);
    let min = if ex & WS_EX_TOOLWINDOW != 0 {
        (2 * s, 2 * s)
    } else if style & (WS_BORDER | WS_DLGFRAME) != 0 {
        MIN_TRACK
    } else {
        ((2 * s).max(2), (2 * s).max(2))
    };
    [(d.width + 2 * s, d.height + 2 * s), (-s, -s), min, (d.width + 20, d.height + 20)]
}

/// `WS_MAXIMIZEBOX` (winuser.h).
pub const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
/// `WS_EX_APPWINDOW` (winuser.h).
pub const WS_EX_APPWINDOW: u32 = 0x0004_0000;

/// The rectangle of a maximized top-level window (screen coordinates), from
/// the maximized size and position its procedure answered to
/// `WM_GETMINMAXINFO`. A window with a maximize box is fitted to the work
/// area (its frame overhangs it: -8,16,1928,1088 over the work area
/// 0,24,1920,1080); any other takes the position and size as they are and
/// covers the taskbar (GetSystemMetrics.yaml `window_metrics`).
pub fn maximized_rect(style: u32, ex: u32, d: &Display, max_size: (i32, i32), max_pos: (i32, i32)) -> Rect {
    if style & WS_MAXIMIZEBOX != 0 {
        let s = frame_side(style, ex);
        let (x, y) = (d.work.left + max_pos.0, d.work.top + max_pos.1);
        let w = max_size.0.min(d.work.width() + 2 * s);
        let h = max_size.1.min(d.work.height() + 2 * s);
        Rect::new(x, y, x + w, y + h)
    } else {
        Rect::new(max_pos.0, max_pos.1, max_pos.0 + max_size.0, max_pos.1 + max_size.1)
    }
}

/// Where a minimized top-level window is put (its top-left corner). A
/// window with a taskbar button on the interactive desktop is parked at
/// -32000,-32000: one that is not owned, not a tool window and not
/// `WS_EX_NOACTIVATE`, or any with `WS_EX_APPWINDOW`. The others, and every
/// window of session 0 (no shell hides minimized windows there), sit in the
/// bottom-left slot of the screen (GetActiveWindow.yaml `minimized_position`,
/// ShowWindow.yaml `minimize`).
pub fn minimized_origin(ex: u32, owned: bool, d: &Display) -> (i32, i32) {
    if d.interactive && has_taskbar_button(ex, owned) { (-32000, -32000) } else { (0, d.height - MINIMIZED.1) }
}

/// Whether a top-level window has a taskbar button on the interactive
/// desktop: one that is not owned, not a tool window and not
/// `WS_EX_NOACTIVATE`, or any with `WS_EX_APPWINDOW`.
pub fn has_taskbar_button(ex: u32, owned: bool) -> bool {
    ex & WS_EX_APPWINDOW != 0 || (!owned && ex & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) == 0)
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
    fn system_metrics_of_both_sessions_match_the_sheet() {
        let i = Display::INTERACTIVE;
        let m = |index| system_metric(index, &i);
        // GetSystemMetrics.yaml `screen`, `work_area_sizes`.
        assert_eq!([0, 1, 76, 77, 78, 79, 80, 81].map(|x| m(x).unwrap()), [1920, 1080, 0, 0, 1920, 1080, 1, 1]);
        assert_eq!(
            [16, 17, 61, 62, 59, 60, 34, 35].map(|x| m(x).unwrap()),
            [1920, 1033, 1936, 1072, 1940, 1100, 136, 39]
        );
        // `sizes_96`.
        let sizes = [
            (2, 17),
            (3, 17),
            (4, 23),
            (5, 1),
            (6, 1),
            (7, 3),
            (8, 3),
            (9, 17),
            (10, 17),
            (11, 32),
            (12, 32),
            (13, 32),
            (14, 32),
            (15, 20),
            (18, 0),
            (20, 17),
            (21, 17),
            (28, 136),
            (29, 39),
            (30, 36),
            (31, 22),
            (32, 4),
            (33, 4),
            (36, 4),
            (37, 4),
            (38, 75),
            (39, 75),
            (45, 2),
            (46, 2),
            (47, 160),
            (48, 28),
            (49, 16),
            (50, 16),
            (51, 23),
            (52, 22),
            (53, 22),
            (54, 19),
            (55, 19),
            (57, 160),
            (58, 28),
            (68, 4),
            (69, 4),
            (71, 15),
            (72, 15),
            (83, 1),
            (84, 1),
            (92, 4),
        ];
        for (index, v) in sizes {
            assert_eq!(m(index), Some(v), "SM {index}");
        }
        // `flags`.
        for index in [
            19, 22, 23, 40, 41, 42, 43, 44, 67, 70, 73, 74, 75, 86, 87, 88, 89, 91, 94, 95, 0x1000, 0x2000, 0x2001,
            0x2003, 0x2004,
        ] {
            assert_eq!(m(index), Some(0), "SM {index}");
        }
        // SM_NETWORK is 3 in GetSystemMetricsForDpi.yaml's later recording (1 in GetSystemMetrics.yaml's).
        assert_eq!([56, 82, 0x2002, 63].map(|x| m(x).unwrap()), [8, 1, 1, 3]);
        // `errors.undefined`: none for the reserved holes, undefined beyond.
        for index in [24, 25, 26, 27, 64, 65, 66, 90, 96, 0x2005] {
            assert_eq!(m(index), Some(0), "SM {index}");
        }
        for index in [-1, -100, 97, 0x1001, i32::MAX] {
            assert_eq!(m(index), None, "SM {index}");
        }
        // Two indices winuser.h does not define answer on the later oracle (GetSystemMetricsForDpi.yaml).
        assert_eq!([85, 93].map(|x| m(x).unwrap()), [4, 516]);
        // The service session (slice A's sheets).
        assert_eq!(system_metric(0, &Display::SERVICE), Some(1024));
        assert_eq!(system_metric(17, &Display::SERVICE), Some(745));
        assert_eq!(system_metric(61, &Display::SERVICE), Some(1040));
    }

    /// GetSystemMetricsForDpi.yaml, every value the sheet states:
    /// `scaling_table`, `not_scaled`, `odd_dpis`, `bad_dpi`, `bad_index`.
    #[test]
    fn system_metrics_for_dpi_have_every_value_of_the_sheet() {
        let d = Display::INTERACTIVE;
        let f = |index: i32, dpi: u32| system_metric_for_dpi(index, metric_dpi(dpi).unwrap(), &d).unwrap();
        let dpis = [96u32, 120, 144, 168, 192, 240, 288, 384];
        let table: [(&[i32], [i32; 8]); 16] = [
            (&[2, 3, 9, 10, 20, 21], [17, 21, 26, 30, 34, 43, 51, 68]),
            (&[4, 51], [23, 29, 34, 40, 45, 56, 67, 89]),
            (&[11, 12], [32, 40, 48, 56, 64, 80, 96, 128]),
            (&[13, 14], [32, 32, 48, 48, 64, 64, 96, 128]),
            (&[15], [20, 25, 30, 34, 39, 49, 58, 77]),
            (&[28, 34], [136, 166, 198, 232, 258, 324, 384, 510]),
            (&[29, 35], [39, 47, 56, 64, 71, 88, 103, 135]),
            (&[30], [36, 46, 54, 64, 72, 91, 109, 145]),
            (&[31, 52, 53], [22, 28, 33, 39, 44, 55, 66, 88]),
            (&[32, 33], [4, 4, 5, 5, 5, 6, 6, 7]),
            (&[38, 39], [75, 94, 113, 131, 150, 188, 225, 300]),
            (&[49, 50], [16, 20, 24, 28, 32, 40, 48, 64]),
            (&[54, 55], [19, 24, 29, 33, 38, 48, 57, 76]),
            (&[71, 72], [15, 19, 25, 29, 31, 41, 47, 65]),
            (&[92], [4, 5, 6, 7, 8, 10, 12, 16]),
            (&[36, 37], [4; 8]),
        ];
        for (indices, want) in table {
            for &i in indices {
                assert_eq!(dpis.map(|dpi| f(i, dpi)), want, "SM {i}");
            }
        }
        // Unchanged at all DPIs (also the odd ones).
        let fixed: [(i32, i32); 37] = [
            (0, 1920),
            (1, 1080),
            (16, 1920),
            (17, 1033),
            (5, 1),
            (6, 1),
            (7, 3),
            (8, 3),
            (45, 2),
            (46, 2),
            (36, 4),
            (37, 4),
            (68, 4),
            (69, 4),
            (83, 1),
            (84, 1),
            (56, 8),
            (47, 160),
            (48, 28),
            (57, 160),
            (58, 28),
            (59, 1940),
            (60, 1100),
            (61, 1936),
            (62, 1072),
            (63, 3),
            (78, 1920),
            (79, 1080),
            (80, 1),
            (81, 1),
            (82, 1),
            (85, 4),
            (93, 516),
            // Flags and counts that are 0.
            (19, 0),
            (43, 0),
            (75, 0),
            (86, 0),
        ];
        for (i, want) in fixed {
            for dpi in [96u32, 120, 144, 168, 192, 240, 288, 384, 1, 24, 100, 1000, 0xFFFF] {
                assert_eq!(f(i, dpi), want, "SM {i} at {dpi}");
            }
        }
        // Rounding at odd DPIs.
        let odd = |index: i32, pairs: &[(u32, i32)]| {
            for &(dpi, want) in pairs {
                assert_eq!(f(index, dpi), want, "SM {index} at {dpi}");
            }
        };
        odd(
            4,
            &[
                (1, 24),
                (2, 24),
                (24, 7),
                (48, 12),
                (72, 18),
                (95, 23),
                (97, 23),
                (100, 24),
                (119, 28),
                (121, 29),
                (130, 31),
                (1000, 230),
                (4000, 918),
                (0xFFFF, 15019),
                (0x7FFF_FFFF, 15019),
            ],
        );
        odd(
            11,
            &[
                (1, 0),
                (2, 1),
                (24, 8),
                (48, 16),
                (72, 24),
                (95, 32),
                (100, 33),
                (1000, 333),
                (4000, 1333),
                (0xFFFF, 21845),
            ],
        );
        odd(13, &[(1, 32), (24, 32), (95, 32), (100, 32), (130, 32), (1000, 128), (4000, 128), (0xFFFF, 128)]);
        odd(92, &[(1, 0), (24, 1), (48, 2), (72, 3), (95, 4), (1000, 42), (4000, 100)]);
        odd(32, &[(1, 3), (2, 3), (24, 3), (48, 4), (72, 4), (95, 4), (1000, 13), (4000, 45), (0xFFFF, 686)]);
        // DPIs that are refused; indices that are none.
        for bad in [0u32, 0x10000, 0xFFFF_FFFF] {
            assert_eq!(metric_dpi(bad), None);
        }
        assert_eq!(metric_dpi(0xFFFF), Some(0xFFFF));
        assert_eq!(metric_dpi(0x7FFF_FFFF), Some(0xFFFF));
        for bad in [-1, -100, 100, 1000, 0xFFF, 0x1001, 0x7FFF_FFFF, i32::MIN] {
            assert_eq!(system_metric_for_dpi(bad, 96, &d), None, "{bad}");
        }
    }

    #[test]
    fn system_metrics_for_dpi_match_the_sheet() {
        let d = Display::INTERACTIVE;
        let f = |index, dpi| system_metric_for_dpi(index, dpi, &d).unwrap();
        for (index, want) in [
            (2, [17, 21, 26, 34]),
            (4, [23, 29, 34, 45]),
            (51, [23, 29, 34, 45]),
            (11, [32, 40, 48, 64]),
            (13, [32, 32, 48, 64]),
            (15, [20, 25, 30, 39]),
            (28, [136, 166, 198, 258]),
            (29, [39, 47, 56, 71]),
            (30, [36, 46, 54, 72]),
            (31, [22, 28, 33, 44]),
            (52, [22, 28, 33, 44]),
            (32, [4, 4, 5, 5]),
            (38, [75, 94, 113, 150]),
            (49, [16, 20, 24, 32]),
            (54, [19, 24, 29, 38]),
            (71, [15, 19, 25, 31]),
            (92, [4, 5, 6, 8]),
            // Unchanged at every DPI.
            (5, [1; 4]),
            (7, [3; 4]),
            (45, [2; 4]),
            (36, [4; 4]),
            (68, [4; 4]),
            (83, [1; 4]),
            (57, [160; 4]),
            (58, [28; 4]),
            (47, [160; 4]),
            (48, [28; 4]),
            (18, [0; 4]),
            (0, [1920; 4]),
            (78, [1920; 4]),
            (61, [1936; 4]),
        ] {
            assert_eq!([96, 120, 144, 192].map(|dpi| f(index, dpi)), want, "SM {index}");
        }
        // SM_CYCAPTION at unusual DPIs.
        assert_eq!([f(4, 1), f(4, 48), f(4, 1000)], [24, 12, 230]);
        // At the system DPI it equals GetSystemMetrics.
        for index in 0..96 {
            if let Some(v) = system_metric(index, &d) {
                assert_eq!(f(index, 96), v, "SM {index}");
            }
        }
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
