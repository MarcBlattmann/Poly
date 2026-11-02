//! Poly's neutral window frame, drawn by the compositor around every
//! toplevel (server-side decorations; the owner's decision, ADR 0106): a
//! one-pixel border, a title bar above the window with its title on the
//! left and three buttons on the right: minimize, maximize (restore when
//! the window is maximized or tiled) and close (ADR 0117). Sizes are
//! logical pixels. Its look follows Alpine Shell on Windows 11's dark mode
//! (ADR 0127): the title bar in the settings window's ground colour, the top
//! corners rounded, a hairline border and a soft shadow under the window.
//!
//! ```text
//!   ┌──────────────────────────────────────────┐  ← border (BORDER)
//!   │ Title text                     —  ▢  ✕   │  ← title bar (TITLE_HEIGHT), buttons (CLOSE_SIZE, BUTTON_GAP apart)
//!   ├──────────────────────────────────────────┤
//!   │                                          │
//!   │        the window's visible bounds       │
//!   │        (xdg window geometry)             │
//!   └──────────────────────────────────────────┘
//! ```

use super::Rect;
use super::paint::Shadow;

/// The title bar's height.
pub const TITLE_HEIGHT: i32 = 24;
/// The border's width on every side.
pub const BORDER: i32 = 1;
/// The side of the title bar's buttons (close, maximize, minimize).
pub const CLOSE_SIZE: i32 = 16;
/// Space before the title and around the close button.
pub const PADDING: i32 = 8;
/// Between two buttons of the title bar.
pub const BUTTON_GAP: i32 = 6;
/// The radius of the frame's top corners (Windows 11's windows, which
/// Alpine Shell's own windows are).
pub const CORNER_RADIUS: i32 = 8;

/// The frame's colours (opaque `0xRRGGBB`), and the desktop's; its
/// shadows (CSS `box-shadow` values, logical pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub background: u32,
    pub title_active: u32,
    pub title_inactive: u32,
    pub text_active: u32,
    pub text_inactive: u32,
    pub border_active: u32,
    pub border_inactive: u32,
    pub shadow_active: Option<Shadow>,
    pub shadow_inactive: Option<Shadow>,
}

impl Theme {
    /// Greys, nothing else: neutral by design. Alpine Shell's dark tokens
    /// as its screenshots show them: the ground `#1a1a19` for the title
    /// bar, ink `#ebebe7` and the quiet ink `#9a9a95` for its text, the
    /// strong line `#3b3b39` for the border; Windows 11's window shadow,
    /// `0 8px 32px rgba(0,0,0,.37)` (lighter for inactive windows).
    pub const NEUTRAL: Theme = Theme {
        background: 0x20_20_20,
        title_active: 0x1a_1a_19,
        title_inactive: 0x24_24_23,
        text_active: 0xeb_eb_e7,
        text_inactive: 0x9a_9a_95,
        border_active: 0x3b_3b_39,
        border_inactive: 0x2e_2e_2d,
        shadow_active: Some(Shadow::new(0.0, 8.0, 32.0, 0.0, 0, 94)),
        shadow_inactive: Some(Shadow::new(0.0, 4.0, 20.0, 0.0, 0, 64)),
    };

    /// The same without shadows (what tests of exact frame pixels use).
    pub const FLAT: Theme = Theme { shadow_active: None, shadow_inactive: None, ..Theme::NEUTRAL };

    /// The shadow of an active or inactive window.
    pub fn shadow(&self, active: bool) -> Option<Shadow> {
        if active { self.shadow_active } else { self.shadow_inactive }
    }

    /// How far any of the shadows reaches past a frame, logical, on each
    /// side (for damage).
    pub fn shadow_extent(&self, outer: Rect) -> Rect {
        [self.shadow_active, self.shadow_inactive].into_iter().flatten().fold(outer, |e, s| e.union(&s.extent(outer)))
    }
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
    /// Left of the close button (empty when the title bar is too narrow).
    pub maximize: Rect,
    pub minimize: Rect,
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
        let step = CLOSE_SIZE + BUTTON_GAP;
        let fits = |r: Rect| if close.is_empty() || r.x < title.x + PADDING { Rect::new(r.x, r.y, 0, 0) } else { r };
        let maximize = fits(close.translate(-step, 0));
        let minimize = if maximize.is_empty() { maximize } else { fits(maximize.translate(-step, 0)) };
        let leftmost = [minimize, maximize, close].into_iter().find(|r| !r.is_empty());
        let text_right = leftmost.map_or(title.right() - PADDING, |r| r.x - PADDING);
        let text = Rect::new(title.x + PADDING, title.y, text_right - title.x - PADDING, TITLE_HEIGHT);
        FrameLayout { outer, title, close, maximize, minimize, text, bounds: g }
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

/// Which glyph a title bar button shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Glyph {
    Close,
    Maximize,
    /// The maximize button of a maximized or tiled window.
    Restore,
    Minimize,
}

/// Whether `glyph` covers pixel (`u`, `v`) of a button `side` pixels wide
/// drawn at scale `s`: inside the inner half of the button, lines `s`
/// pixels thick (the cross's diagonals `2 * s`).
pub(crate) fn glyph_covers(glyph: Glyph, u: i32, v: i32, side: i32, s: i32) -> bool {
    let (lo, hi) = (side / 4, side - side / 4);
    if u < lo || v < lo || u >= hi || v >= hi {
        return false;
    }
    let outline = |l: i32, t: i32, r: i32, b: i32| {
        u >= l && v >= t && u < r && v < b && (u < l + s || v < t + s || u >= r - s || v >= b - s)
    };
    match glyph {
        Glyph::Close => cross_covers(u, v, side, s),
        // A line along the bottom of the inner square.
        Glyph::Minimize => v >= hi - s,
        Glyph::Maximize => outline(lo, lo, hi, hi),
        // Two squares: one in front at the bottom left, the top and right
        // edges of another behind it.
        Glyph::Restore => {
            let d = (side / 8).max(s);
            let front = outline(lo, lo + d, hi - d, hi);
            let inside_front = u >= lo && v >= lo + d && u < hi - d && v < hi;
            let back = outline(lo + d, lo, hi, hi - d);
            front || (back && !inside_front)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_buttons_right_to_left() {
        let f = FrameLayout::around(Rect::new(100, 100, 300, 200));
        // Close 4 from the right edge, then maximize and minimize, each 22 further left.
        assert_eq!(f.close, Rect::new(400 - 4 - 16, 80, 16, 16));
        assert_eq!(f.maximize, f.close.translate(-22, 0));
        assert_eq!(f.minimize, f.close.translate(-44, 0));
        assert_eq!(f.text.right(), f.minimize.x - PADDING);
        // A narrow window loses the minimize button first.
        let f = FrameLayout::around(Rect::new(0, 30, 60, 50));
        assert!(!f.close.is_empty() && !f.maximize.is_empty() && f.minimize.is_empty());
        let f = FrameLayout::around(Rect::new(0, 30, 20, 50));
        assert!(f.close.is_empty() && f.maximize.is_empty() && f.minimize.is_empty());
    }

    #[test]
    fn glyphs() {
        // A 16-pixel button at scale 1: the inner square is 4..12.
        let count = |g: Glyph| {
            (0..16).flat_map(|v| (0..16).map(move |u| (u, v))).filter(|&(u, v)| glyph_covers(g, u, v, 16, 1)).count()
        };
        assert_eq!(count(Glyph::Minimize), 8, "one row of eight");
        assert_eq!(count(Glyph::Maximize), 28, "the outline of an 8x8 square");
        assert!(glyph_covers(Glyph::Restore, 4, 11, 16, 1), "the front square's corner");
        assert!(glyph_covers(Glyph::Restore, 11, 4, 16, 1), "the back square's corner");
        assert!(!glyph_covers(Glyph::Restore, 7, 8, 16, 1), "hollow");
        assert!(!glyph_covers(Glyph::Close, 0, 0, 16, 1));
    }
}
