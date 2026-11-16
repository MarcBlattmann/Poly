//! `DefWindowProc`'s `WM_NCHITTEST` for the non-client area of a top-level
//! window (ADR 0212): the caption, its buttons and the sizing frame with the
//! corner zones.
//!
//! Sources: the spec sheet `WM_NCHITTEST.yaml` and its visible tests'
//! recorded results (`hit_corners_cap`, `hit_corners_ovl_thick_nocap`,
//! `hit_corners_sizes`), Microsoft Learn ("WM_NCHITTEST": the meaning of the
//! codes), `winuser.h` for the codes and styles.
//!
//! The geometry, for a window `w` wide and `h` high whose frame is `f`
//! pixels wide (the client area's inset at the sides: 8 with a caption, 7
//! without) at 96 DPI:
//!
//! - **Captioned, sizable** (the corner lengths do not depend on the window's
//!   size): on the top rows `0..=f` the top-left corner is 19 pixels long, the
//!   top-right one 18 and the top-left and top-right corners reach 9 rows down
//!   the sides (the frame and the caption's first row); the caption's buttons
//!   (system menu, minimize, maximize, close) and the caption itself take the
//!   rows below that inside the frame, and a caption row has a left and right
//!   border zone as long as the top corners; the bottom corners are 18 and 17
//!   pixels long on the lowest 7 rows, the row above them has the 9 and 8
//!   pixel side margins, and the sides are `HTLEFT` / `HTRIGHT` between.
//!   A tool window's top corners are 15 and 14 pixels and 17 rows.
//! - **No caption, sizable**: every corner is the frame plus 22 pixels (the
//!   caption's width and height) long along both its edges.

use crate::frame::{
    HTBORDER, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLOSE, HTLEFT, HTMAXBUTTON, HTMINBUTTON, HTRIGHT,
    HTTOP, HTTOPLEFT, HTTOPRIGHT, Rect,
};
use crate::metrics::{
    WS_CAPTION, WS_CHILD, WS_EX_TOOLWINDOW, WS_MAXIMIZE, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
};

/// `HTSYSMENU` (winuser.h).
pub const HTSYSMENU: i32 = 3;

/// The caption's corner and button sizes at 96 DPI, as observed.
const TOP_LEFT: i32 = 19;
const TOP_RIGHT: i32 = 18;
const TOOL_TOP_LEFT: i32 = 15;
const TOOL_TOP_RIGHT: i32 = 14;
const TOOL_SIDE_TOP: i32 = 17;
const BOTTOM_LEFT: i32 = 18;
const BOTTOM_RIGHT: i32 = 17;
const SIDE_BOTTOM: i32 = 7;
/// The system menu button's width and the height of the button row.
const SYSMENU_W: i32 = 19;
const BUTTON_ROW: i32 = 22;
/// The caption buttons' widths: close, maximize, minimize, and the gap between the last two.
const CLOSE_W: i32 = 35;
const MAX_W: i32 = 33;
const MIN_W: i32 = 33;
const MIN_GAP: i32 = 2;
/// A captionless frame's corners reach this far beyond the frame (the caption's size).
const CAPTION_SIZE: i32 = 22;

/// The caption button at `rx` (relative to the window's left, the window `w` wide) on the button
/// rows, if any (`None`: the caption).
fn button(style: u32, f: i32, w: i32, rx: i32) -> Option<i32> {
    if style & WS_SYSMENU == 0 {
        return None;
    }
    if (f..f + SYSMENU_W).contains(&rx) {
        return Some(HTSYSMENU);
    }
    let right = w - f;
    let close = (right - CLOSE_W)..right;
    let max = (close.start - MAX_W)..close.start;
    let min = (max.start - MIN_GAP - MIN_W)..(max.start - MIN_GAP);
    if close.contains(&rx) {
        Some(HTCLOSE)
    } else if style & (WS_MINIMIZEBOX | WS_MAXIMIZEBOX) != 0 && max.contains(&rx) {
        Some(HTMAXBUTTON)
    } else if style & (WS_MINIMIZEBOX | WS_MAXIMIZEBOX) != 0 && min.contains(&rx) {
        Some(HTMINBUTTON)
    } else {
        None
    }
}

/// `DefWindowProc`'s answer to `WM_NCHITTEST` for a point of `window` that is outside
/// its client area: `HTBORDER` for a window that cannot be sized, the caption and its buttons,
/// and for one that can, the sizing frame (a maximized window has none).
pub fn nc_hit(style: u32, ex_style: u32, window: Rect, client: Rect, x: i32, y: i32) -> i32 {
    let (w, h) = (window.width(), window.height());
    let (rx, ry) = (x - window.left, y - window.top);
    let f = (client.left - window.left).max(window.right - client.right).max(0);
    let captioned = style & WS_CAPTION == WS_CAPTION;
    let caption_bottom = client.top - window.top;
    let sizable = style & WS_THICKFRAME != 0 && style & (WS_MAXIMIZE | WS_CHILD) == 0 && f > 0;
    if !sizable {
        // Without a sizing frame the caption is all there is (its buttons are Poly's frame's).
        return if captioned && ry < caption_bottom { HTCAPTION } else { HTBORDER };
    }
    if !captioned {
        // Corners of the frame plus the caption's size.
        let c = f + CAPTION_SIZE;
        let (left, right) = (rx < c, rx >= w - c);
        let (top, bottom) = (ry < f, ry >= h - f);
        let (side_left, side_right) = (rx < f, rx >= w - f);
        let (side_top, side_bottom) = (ry < c, ry >= h - c);
        return match (top, bottom, side_left, side_right) {
            (true, _, _, _) if left => HTTOPLEFT,
            (true, _, _, _) if right => HTTOPRIGHT,
            (true, ..) => HTTOP,
            (_, true, _, _) if left => HTBOTTOMLEFT,
            (_, true, _, _) if right => HTBOTTOMRIGHT,
            (_, true, ..) => HTBOTTOM,
            (_, _, true, _) if side_top => HTTOPLEFT,
            (_, _, true, _) if side_bottom => HTBOTTOMLEFT,
            (_, _, true, _) => HTLEFT,
            (_, _, _, true) if side_top => HTTOPRIGHT,
            (_, _, _, true) if side_bottom => HTBOTTOMRIGHT,
            _ => HTRIGHT,
        };
    }
    let tool = ex_style & WS_EX_TOOLWINDOW != 0;
    let (tl, tr) = if tool { (TOOL_TOP_LEFT, TOOL_TOP_RIGHT) } else { (TOP_LEFT, TOP_RIGHT) };
    // The top rows: the frame and the first caption row.
    if ry <= f {
        let in_button_row = ry == f && rx >= f && rx < w - f;
        if in_button_row && let Some(b) = button(style, f, w, rx) {
            return b;
        }
        return if rx < tl {
            HTTOPLEFT
        } else if rx >= w - tr {
            HTTOPRIGHT
        } else {
            HTTOP
        };
    }
    // The caption's rows.
    if ry < caption_bottom {
        if tool && ry < TOOL_SIDE_TOP && rx < f {
            return HTTOPLEFT;
        }
        if tool && ry < TOOL_SIDE_TOP && rx >= w - f {
            return HTTOPRIGHT;
        }
        if ry < f + BUTTON_ROW
            && rx >= f
            && rx < w - f
            && let Some(b) = button(style, f, w, rx)
        {
            return b;
        }
        return if rx < tl {
            HTLEFT
        } else if rx >= w - tr {
            HTRIGHT
        } else {
            HTCAPTION
        };
    }
    // Below the caption: the sides, then the bottom frame.
    let bottom_row = h - f;
    if ry < bottom_row {
        return if rx < f { HTLEFT } else { HTRIGHT };
    }
    if ry == bottom_row {
        if rx < f + 1 {
            return HTLEFT;
        }
        if rx >= w - f {
            return HTRIGHT;
        }
    }
    if rx < BOTTOM_LEFT {
        if ry >= h - SIDE_BOTTOM || rx >= f + 1 { HTBOTTOMLEFT } else { HTLEFT }
    } else if rx >= w - BOTTOM_RIGHT {
        HTBOTTOMRIGHT
    } else {
        HTBOTTOM
    }
}

#[cfg(test)]
#[path = "nchit_tests.rs"]
mod tests;
