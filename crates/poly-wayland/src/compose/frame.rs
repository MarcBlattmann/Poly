//! Poly's neutral window frame, drawn by the compositor around every
//! toplevel (server-side decorations; the owner's decision, ADR 0106): a
//! one-pixel border, a title bar above the window with its title on the
//! left and a close button on the right. Sizes are logical pixels.
//!
//! ```text
//!   ┌──────────────────────────────────────────┐  ← border (BORDER)
//!   │ Title text                           ✕   │  ← title bar (TITLE_HEIGHT), close button (CLOSE_SIZE)
//!   ├──────────────────────────────────────────┤
//!   │                                          │
//!   │        the window's visible bounds       │
//!   │        (xdg window geometry)             │
//!   └──────────────────────────────────────────┘
//! ```

use super::Rect;

/// The title bar's height.
pub const TITLE_HEIGHT: i32 = 24;
/// The border's width on every side.
pub const BORDER: i32 = 1;
/// The close button's side.
pub const CLOSE_SIZE: i32 = 16;
/// Space before the title and around the close button.
pub const PADDING: i32 = 8;

/// The frame's colours (opaque `0xRRGGBB`), and the desktop's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub background: u32,
    pub title_active: u32,
    pub title_inactive: u32,
    pub text_active: u32,
    pub text_inactive: u32,
    pub border_active: u32,
    pub border_inactive: u32,
}

impl Theme {
    /// Greys, nothing else: neutral by design.
    pub const NEUTRAL: Theme = Theme {
        background: 0x20_20_20,
        title_active: 0x3c_3c_3c,
        title_inactive: 0x2a_2a_2a,
        text_active: 0xe8_e8_e8,
        text_inactive: 0x96_96_96,
        border_active: 0x5a_5a_5a,
        border_inactive: 0x38_38_38,
    };
}

impl Default for Theme {
    fn default() -> Theme {
        Theme::NEUTRAL
    }
}

/// Where the parts of a frame go, around the visible bounds `g`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameLayout {
    /// Everything, border included.
    pub outer: Rect,
    pub title: Rect,
    pub close: Rect,
    /// Where the title text may go.
    pub text: Rect,
    pub bounds: Rect,
}

impl FrameLayout {
    pub fn around(g: Rect) -> FrameLayout {
        let outer = Rect::new(
            g.x - BORDER,
            g.y - TITLE_HEIGHT - BORDER,
            g.width + 2 * BORDER,
            g.height + TITLE_HEIGHT + 2 * BORDER,
        );
        let title = Rect::new(g.x, g.y - TITLE_HEIGHT, g.width, TITLE_HEIGHT);
        let margin = (TITLE_HEIGHT - CLOSE_SIZE) / 2;
        let close = Rect::new(title.right() - margin - CLOSE_SIZE, title.y + margin, CLOSE_SIZE, CLOSE_SIZE);
        let close = if close.x < title.x + PADDING { Rect::new(close.x, close.y, 0, 0) } else { close };
        let text_right = if close.is_empty() { title.right() - PADDING } else { close.x - PADDING };
        let text = Rect::new(title.x + PADDING, title.y, text_right - title.x - PADDING, TITLE_HEIGHT);
        FrameLayout { outer, title, close, text, bounds: g }
    }

    /// The border's four strips.
    pub fn border(&self) -> [Rect; 4] {
        let (o, g) = (self.outer, self.bounds);
        [
            Rect::new(o.x, o.y, o.width, BORDER),
            Rect::new(o.x, g.bottom(), o.width, BORDER),
            Rect::new(o.x, o.y + BORDER, BORDER, o.height - 2 * BORDER),
            Rect::new(g.right(), o.y + BORDER, BORDER, o.height - 2 * BORDER),
        ]
    }
}

/// Whether the close button's cross covers pixel (`u`, `v`) of a button
/// `side` pixels wide drawn at scale `s`: two diagonals, `2 * s` thick,
/// inset by a quarter of the button.
pub(crate) fn cross_covers(u: i32, v: i32, side: i32, s: i32) -> bool {
    let inset = side / 4;
    if u < inset || v < inset || u >= side - inset || v >= side - inset {
        return false;
    }
    (u - v).abs() < s || (u + v - (side - 1)).abs() < s
}
