//! Poly's frame as the non-client area of a Windows top-level window
//! (ADR 0128, Decision 4).
//!
//! The owner's decision (ADR 0106) is one neutral frame drawn by Poly for
//! every app. For a Windows window with a caption or a sizing border the
//! frame *is* its non-client caption and border: the window rectangle
//! (`GetWindowRect`) includes it, the compositor draws it around the
//! window's surface exactly there, and `DefWindowProc` answers
//! `WM_NCHITTEST` over it from the same geometry ([`hit_test`]). Everything
//! inside the frame (the app's menu bar, scroll bars, client edges and the
//! client area) is the surface, drawn by the app.
//!
//! ```text
//!   window rectangle ┌──────────────────────────────────────┐ ← border
//!                    │ Title                    —   ▢   ✕   │ ← title bar
//!                    ├──────────────────────────────────────┤
//!                    │ surface: menu bar, client area, ...  │
//!                    └──────────────────────────────────────┘
//! ```
//!
//! The numbers are the compositor's (`poly-wayland`'s `compose::frame`,
//! sent in the link's `Hello`); [`FrameMetrics::POLY`] are its values, which
//! the compositor's tests check against its own constants.

/// A rectangle as Windows' `RECT` has it: right and bottom exclusive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }

    pub const fn width(&self) -> i32 {
        self.right - self.left
    }

    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub const fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    pub const fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// The frame's sizes, logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameMetrics {
    /// The title bar's height.
    pub title: u16,
    /// The border's width on every side.
    pub border: u16,
    /// How far from the window's outer edge a point of the frame counts as
    /// a sizing border or corner.
    pub resize: u16,
}

impl FrameMetrics {
    /// Poly's frame as the compositor draws it (`TITLE_HEIGHT` 24,
    /// `BORDER` 1); the sizing zone is the border and 4 pixels of the title
    /// bar's edges.
    pub const POLY: FrameMetrics = FrameMetrics { title: 24, border: 1, resize: 4 };

    /// The side of a title bar button (the compositor's `CLOSE_SIZE`).
    pub const BUTTON: i32 = 16;
    /// Between two buttons (`BUTTON_GAP`).
    pub const GAP: i32 = 6;
    /// Space before the title; a button closer to the title bar's left end
    /// is left out (`PADDING`).
    pub const PADDING: i32 = 8;

    /// The non-client sizes the frame adds on each side:
    /// (left, top, right, bottom).
    pub const fn insets(&self) -> (i32, i32, i32, i32) {
        let b = self.border as i32;
        (b, b + self.title as i32, b, b)
    }

    /// The window rectangle around a surface.
    pub const fn window_from_surface(&self, s: Rect) -> Rect {
        let (l, t, r, b) = self.insets();
        Rect::new(s.left - l, s.top - t, s.right + r, s.bottom + b)
    }

    /// The surface inside a window rectangle (empty when the window is
    /// smaller than its frame).
    pub const fn surface_from_window(&self, w: Rect) -> Rect {
        let (l, t, r, b) = self.insets();
        let s = Rect::new(w.left + l, w.top + t, w.right - r, w.bottom - b);
        if s.is_empty() { Rect::new(s.left, s.top, s.left, s.top) } else { s }
    }

    /// The title bar of a window rectangle.
    pub const fn title_bar(&self, w: Rect) -> Rect {
        let b = self.border as i32;
        Rect::new(w.left + b, w.top + b, w.right - b, w.top + b + self.title as i32)
    }

    /// The title bar's buttons, right to left as the compositor draws them;
    /// a button that does not fit is empty.
    pub fn buttons(&self, w: Rect) -> Buttons {
        let t = self.title_bar(w);
        let margin = (self.title as i32 - Self::BUTTON) / 2;
        let empty_at = |x: i32, y: i32| Rect::new(x, y, x, y);
        let at = |x: i32| Rect::new(x, t.top + margin, x + Self::BUTTON, t.top + margin + Self::BUTTON);
        let close = at(t.right - margin - Self::BUTTON);
        let close = if close.left < t.left + Self::PADDING { empty_at(close.left, close.top) } else { close };
        let step = Self::BUTTON + Self::GAP;
        let fits = |r: Rect| {
            if close.is_empty() || r.left < t.left + Self::PADDING { empty_at(r.left, r.top) } else { r }
        };
        let maximize = fits(at(close.left - step));
        let minimize = if maximize.is_empty() { maximize } else { fits(at(maximize.left - step)) };
        Buttons { close, maximize, minimize }
    }
}

/// The title bar's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buttons {
    pub close: Rect,
    pub maximize: Rect,
    pub minimize: Rect,
}

// Hit-test codes of `winuser.h` (`WM_NCHITTEST`'s return values).
pub const HTNOWHERE: i32 = 0;
pub const HTCLIENT: i32 = 1;
pub const HTCAPTION: i32 = 2;
pub const HTMINBUTTON: i32 = 8;
pub const HTMAXBUTTON: i32 = 9;
pub const HTLEFT: i32 = 10;
pub const HTRIGHT: i32 = 11;
pub const HTTOP: i32 = 12;
pub const HTTOPLEFT: i32 = 13;
pub const HTTOPRIGHT: i32 = 14;
pub const HTBOTTOM: i32 = 15;
pub const HTBOTTOMLEFT: i32 = 16;
pub const HTBOTTOMRIGHT: i32 = 17;
pub const HTBORDER: i32 = 18;
pub const HTCLOSE: i32 = 20;

/// Where a point of the screen is relative to a framed window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NcHit {
    /// Outside the window rectangle (`HTNOWHERE`).
    Outside,
    /// Inside the surface: the app's own non-client parts or its client
    /// area, which `user32` sorts out.
    Surface,
    /// On Poly's frame: the `HT*` code `DefWindowProc` answers.
    Frame(i32),
}

/// `DefWindowProc`'s answer to `WM_NCHITTEST` for the part of a framed
/// window that is Poly's frame, at screen point (`x`, `y`) of window
/// rectangle `w`. A window that cannot be sized (`sizable` false: no
/// `WS_THICKFRAME`) has a plain border (`HTBORDER`) and no sizing corners.
/// The meanings of the codes are Learn's (`WM_NCHITTEST`); which part of
/// Poly's frame answers which code is Poly's frame's geometry.
pub fn hit_test(m: &FrameMetrics, w: Rect, x: i32, y: i32, sizable: bool) -> NcHit {
    if !w.contains(x, y) {
        return NcHit::Outside;
    }
    if m.surface_from_window(w).contains(x, y) {
        return NcHit::Surface;
    }
    let r = i32::from(m.resize.max(m.border));
    let (left, right) = (x < w.left + r, x >= w.right - r);
    let (top, bottom) = (y < w.top + r, y >= w.bottom - r);
    let b = i32::from(m.border);
    let on_border = x < w.left + b || x >= w.right - b || y < w.top + b || y >= w.bottom - b;
    if sizable && (left || right || top || bottom) {
        // Corners take the zone along both edges; the sides only count on
        // the border itself or the title bar's edges.
        let code = match (left, right, top, bottom) {
            (true, _, true, _) => HTTOPLEFT,
            (_, true, true, _) => HTTOPRIGHT,
            (true, _, _, true) => HTBOTTOMLEFT,
            (_, true, _, true) => HTBOTTOMRIGHT,
            (_, _, true, _) => HTTOP,
            (_, _, _, true) => HTBOTTOM,
            (true, _, _, _) => HTLEFT,
            _ => HTRIGHT,
        };
        return NcHit::Frame(code);
    }
    if on_border {
        return NcHit::Frame(HTBORDER);
    }
    let buttons = m.buttons(w);
    if buttons.close.contains(x, y) {
        NcHit::Frame(HTCLOSE)
    } else if buttons.maximize.contains(x, y) {
        NcHit::Frame(HTMAXBUTTON)
    } else if buttons.minimize.contains(x, y) {
        NcHit::Frame(HTMINBUTTON)
    } else {
        NcHit::Frame(HTCAPTION)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: FrameMetrics = FrameMetrics::POLY;

    #[test]
    fn the_frame_wraps_the_surface() {
        let s = Rect::new(100, 100, 400, 300);
        let w = M.window_from_surface(s);
        assert_eq!(w, Rect::new(99, 75, 401, 301));
        assert_eq!(M.surface_from_window(w), s);
        assert_eq!(M.title_bar(w), Rect::new(100, 76, 400, 100));
        // A window smaller than its frame has an empty surface.
        assert!(M.surface_from_window(Rect::new(0, 0, 10, 10)).is_empty());
    }

    #[test]
    fn buttons_right_to_left_like_the_compositors() {
        let w = M.window_from_surface(Rect::new(100, 100, 400, 300));
        let b = M.buttons(w);
        // Close 4 from the title bar's right end, then maximize and
        // minimize, each 22 further left.
        assert_eq!(b.close, Rect::new(400 - 4 - 16, 80, 400 - 4, 96));
        assert_eq!(b.maximize.left, b.close.left - 22);
        assert_eq!(b.minimize.left, b.close.left - 44);
        // A narrow window loses the minimize button first.
        let b = M.buttons(M.window_from_surface(Rect::new(0, 30, 60, 80)));
        assert!(!b.close.is_empty() && !b.maximize.is_empty() && b.minimize.is_empty());
        let b = M.buttons(M.window_from_surface(Rect::new(0, 30, 20, 80)));
        assert!(b.close.is_empty() && b.maximize.is_empty() && b.minimize.is_empty());
    }

    #[test]
    fn hit_test_names_the_frame_parts() {
        let w = M.window_from_surface(Rect::new(100, 100, 400, 300));
        let b = M.buttons(w);
        let hit = |x, y| hit_test(&M, w, x, y, true);
        assert_eq!(hit(200, 200), NcHit::Surface);
        assert_eq!(hit(50, 50), NcHit::Outside);
        assert_eq!(hit(200, 88), NcHit::Frame(HTCAPTION));
        assert_eq!(hit(b.close.left + 2, b.close.top + 2), NcHit::Frame(HTCLOSE));
        assert_eq!(hit(b.maximize.left + 2, b.maximize.top + 2), NcHit::Frame(HTMAXBUTTON));
        assert_eq!(hit(b.minimize.left + 2, b.minimize.top + 2), NcHit::Frame(HTMINBUTTON));
        assert_eq!(hit(w.left, 200), NcHit::Frame(HTLEFT));
        assert_eq!(hit(w.right - 1, 200), NcHit::Frame(HTRIGHT));
        assert_eq!(hit(200, w.bottom - 1), NcHit::Frame(HTBOTTOM));
        assert_eq!(hit(200, w.top), NcHit::Frame(HTTOP));
        assert_eq!(hit(200, w.top + 3), NcHit::Frame(HTTOP));
        assert_eq!(hit(w.left + 2, w.top + 2), NcHit::Frame(HTTOPLEFT));
        assert_eq!(hit(w.right - 1, w.bottom - 1), NcHit::Frame(HTBOTTOMRIGHT));
        // Not sizable: the border is a border and the top is the caption.
        assert_eq!(hit_test(&M, w, w.left, 200, false), NcHit::Frame(HTBORDER));
        assert_eq!(hit_test(&M, w, 200, w.top + 3, false), NcHit::Frame(HTCAPTION));
    }
}
