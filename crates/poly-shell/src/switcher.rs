//! The window switcher (Alt+Tab, ADR 0117), in the look of Alpine Shell's:
//! a row of cards, each the window's name over a thumbnail of it, the
//! selected one ringed; floating in the middle of the screen (Alpine hangs
//! it from its bar; on Poly nothing drops from the ridge).
//!
//! Order is most recently used first ([`Mru`]); it opens on the second
//! window (the one Alt+Tab goes back to), Tab and Shift+Tab or the arrows
//! step, releasing Alt switches, Escape cancels. Minimized windows are in
//! it too: switching to one shows it again (Poly has no taskbar, as
//! Alpine has none: the switcher and the launcher are how a minimized
//! window comes back).

use alloc::string::String;
use alloc::vec::Vec;
use poly_wayland::compose::{Content, Rect, WindowId};

/// Alpine's card measurements, logical pixels.
pub mod look {
    /// The panel's padding and the gap between cards.
    pub const PADDING: i32 = 12;
    pub const GAP: i32 = 8;
    /// The thumbnail area's height; cards are as wide as the thumbnail's
    /// aspect gives, between these.
    pub const THUMB_HEIGHT: i32 = 124;
    pub const CARD_MIN: i32 = 100;
    pub const CARD_MAX: i32 = 200;
    /// The name row: 6 pixels of padding around a 16-pixel icon.
    pub const HEADER: i32 = 28;
    pub const ICON: i32 = 16;
    /// Under the thumbnail.
    pub const BOTTOM: i32 = 4;
    pub const RADIUS: i32 = 6;
    pub const TEXT_SIZE: f32 = 12.0;
    /// Fills and rings: white at 5 %, 10 %, 70 %.
    pub const CARD_ALPHA: u8 = 13;
    pub const RING_ALPHA: u8 = 26;
    pub const SELECTED_ALPHA: u8 = 179;
}

/// A thumbnail: opaque `0xRRGGBB` pixels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Thumb {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

/// A window's content scaled down to fit `max_w` × `max_h` (each output
/// pixel the average of the content pixels it covers), over `background`
/// where the content is translucent.
pub fn thumbnail(c: &Content, max_w: u32, max_h: u32, background: u32) -> Thumb {
    if c.width == 0 || c.height == 0 || max_w == 0 || max_h == 0 {
        return Thumb::default();
    }
    // Keep the aspect: the larger shrink wins, never enlarge.
    let (w, h) = if u64::from(c.width) * u64::from(max_h) > u64::from(c.height) * u64::from(max_w) {
        (max_w.min(c.width), (u64::from(c.height) * u64::from(max_w.min(c.width)) / u64::from(c.width)).max(1) as u32)
    } else {
        ((u64::from(c.width) * u64::from(max_h.min(c.height)) / u64::from(c.height)).max(1) as u32, max_h.min(c.height))
    };
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for ty in 0..h {
        let (y0, y1) = (ty * c.height / h, ((ty + 1) * c.height / h).max(ty * c.height / h + 1));
        for tx in 0..w {
            let (x0, x1) = (tx * c.width / w, ((tx + 1) * c.width / w).max(tx * c.width / w + 1));
            // Sample at most 4×4 points of the box: enough for a picture
            // this small, and cheap.
            let (sx, sy) = (((x1 - x0) / 4).max(1), ((y1 - y0) / 4).max(1));
            let mut sum = [0u32; 3];
            let mut n = 0;
            let mut y = y0;
            while y < y1 {
                let mut x = x0;
                while x < x1 {
                    let p = c.pixels[(y * c.width + x) as usize];
                    let p = poly_wayland::compose::over(p, background | 0xff00_0000);
                    sum[0] += (p >> 16) & 0xff;
                    sum[1] += (p >> 8) & 0xff;
                    sum[2] += p & 0xff;
                    n += 1;
                    x += sx;
                }
                y += sy;
            }
            let n = n.max(1);
            pixels.push((sum[0] / n) << 16 | (sum[1] / n) << 8 | (sum[2] / n));
        }
    }
    Thumb { width: w, height: h, pixels }
}

/// A window in the switcher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub window: WindowId,
    pub title: String,
    pub app_id: String,
    pub minimized: bool,
    pub thumb: Thumb,
}

/// Windows, most recently used first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mru(Vec<WindowId>);

impl Mru {
    /// `w` was used (focused) now.
    pub fn touch(&mut self, w: WindowId) {
        self.0.retain(|&x| x != w);
        self.0.insert(0, w);
    }

    /// `windows` (any order) most recently used first; windows never
    /// touched come last, in the order given. Forgets windows not among
    /// them.
    pub fn order(&mut self, windows: &[WindowId]) -> Vec<WindowId> {
        self.0.retain(|w| windows.contains(w));
        let mut out = self.0.clone();
        out.extend(windows.iter().filter(|w| !self.0.contains(w)));
        out
    }

    pub fn first(&self) -> Option<WindowId> {
        self.0.first().copied()
    }
}

/// The switcher's state.
#[derive(Debug, Default)]
pub struct Switcher {
    entries: Vec<Entry>,
    selected: usize,
}

impl Switcher {
    pub fn is_open(&self) -> bool {
        !self.entries.is_empty()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Open on `entries` (most recently used first): the second is
    /// selected (the last with Shift).
    pub fn open(&mut self, entries: Vec<Entry>, backward: bool) {
        let n = entries.len();
        self.entries = entries;
        self.selected = match n {
            0 | 1 => 0,
            _ if backward => n - 1,
            _ => 1,
        };
    }

    /// The next card (or the previous), wrapping.
    pub fn step(&mut self, forward: bool) {
        let n = self.entries.len();
        if n > 0 {
            self.selected = if forward { (self.selected + 1) % n } else { (self.selected + n - 1) % n };
        }
    }

    /// Select card `i` (the pointer over it).
    pub fn select(&mut self, i: usize) -> bool {
        if i < self.entries.len() && i != self.selected {
            self.selected = i;
            return true;
        }
        false
    }

    /// Close, switching: the window to activate.
    pub fn commit(&mut self) -> Option<WindowId> {
        let w = self.entries.get(self.selected).map(|e| e.window);
        self.cancel();
        w
    }

    pub fn cancel(&mut self) {
        self.entries.clear();
        self.selected = 0;
    }

    /// A card's width: the thumbnail's, between Alpine's limits.
    fn card_width(e: &Entry) -> i32 {
        let w = if e.thumb.height == 0 {
            look::CARD_MAX
        } else {
            (e.thumb.width as i32 * look::THUMB_HEIGHT / e.thumb.height as i32).max(1)
        };
        w.clamp(look::CARD_MIN, look::CARD_MAX)
    }

    /// The panel and each card's rectangle (logical), centred on a
    /// `width` × `height` screen: cards in rows no wider than 90 % of it.
    pub fn layout(&self, width: i32, height: i32) -> (Rect, Vec<Rect>) {
        let card_h = look::HEADER + look::THUMB_HEIGHT + look::BOTTOM;
        let max_row = (width * 9 / 10 - 2 * look::PADDING).max(look::CARD_MAX);
        // Rows: (cards' widths, the row's width).
        let mut rows: Vec<(Vec<i32>, i32)> = Vec::new();
        for e in &self.entries {
            let w = Self::card_width(e);
            match rows.last_mut() {
                Some((cards, used)) if *used + look::GAP + w <= max_row => {
                    cards.push(w);
                    *used += look::GAP + w;
                }
                _ => rows.push((alloc::vec![w], w)),
            }
        }
        let inner_w = rows.iter().map(|r| r.1).max().unwrap_or(0);
        let n_rows = rows.len() as i32;
        let inner_h = n_rows * card_h + (n_rows - 1).max(0) * look::GAP;
        let panel = Rect::new(
            (width - inner_w) / 2 - look::PADDING,
            (height - inner_h) / 2 - look::PADDING,
            inner_w + 2 * look::PADDING,
            inner_h + 2 * look::PADDING,
        );
        let mut cards = Vec::new();
        for (i, (widths, used)) in rows.iter().enumerate() {
            // Each row centred.
            let mut x = panel.x + look::PADDING + (inner_w - used) / 2;
            let y = panel.y + look::PADDING + i as i32 * (card_h + look::GAP);
            for w in widths {
                cards.push(Rect::new(x, y, *w, card_h));
                x += w + look::GAP;
            }
        }
        (panel, cards)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn entry(id: u32, w: u32, h: u32) -> Entry {
        Entry {
            window: WindowId(id),
            title: alloc::format!("w{id}"),
            app_id: String::new(),
            minimized: false,
            thumb: Thumb { width: w, height: h, pixels: vec![0; (w * h) as usize] },
        }
    }

    #[test]
    fn most_recently_used_first() {
        let mut m = Mru::default();
        for w in [1, 2, 3] {
            m.touch(WindowId(w));
        }
        m.touch(WindowId(1));
        let all = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)];
        assert_eq!(m.order(&all), [WindowId(1), WindowId(3), WindowId(2), WindowId(4)]);
        // Window 3 went away.
        assert_eq!(m.order(&[WindowId(1), WindowId(2)]), [WindowId(1), WindowId(2)]);
        assert_eq!(m.first(), Some(WindowId(1)));
    }

    #[test]
    fn alt_tab_steps_and_commits() {
        let mut s = Switcher::default();
        assert!(!s.is_open());
        s.open(vec![entry(1, 200, 124), entry(2, 200, 124), entry(3, 200, 124)], false);
        assert_eq!(s.selected(), 1, "the window Alt+Tab goes back to");
        s.step(true);
        s.step(true);
        assert_eq!(s.selected(), 0, "wraps");
        s.step(false);
        assert_eq!(s.selected(), 2);
        assert_eq!(s.commit(), Some(WindowId(3)));
        assert!(!s.is_open());
        s.open(vec![entry(1, 1, 1), entry(2, 1, 1)], true);
        assert_eq!(s.selected(), 1, "Shift: from the end");
        s.cancel();
        assert_eq!(s.commit(), None);
        s.open(vec![entry(9, 1, 1)], false);
        assert_eq!(s.commit(), Some(WindowId(9)), "one window: itself");
    }

    #[test]
    fn cards_are_laid_out_centred() {
        let mut s = Switcher::default();
        // A wide window (card at its maximum), a tall one (at its minimum).
        s.open(vec![entry(1, 400, 124), entry(2, 50, 124)], false);
        let (panel, cards) = s.layout(1280, 800);
        assert_eq!(cards[0].width, 200);
        assert_eq!(cards[1].width, 100);
        assert_eq!(cards[0].height, 28 + 124 + 4);
        assert_eq!(cards[1].x - cards[0].right(), 8);
        assert_eq!(panel.width, 200 + 8 + 100 + 24);
        assert_eq!(panel.x + panel.width / 2, 640, "centred");
        assert_eq!(cards[0].x, panel.x + 12);
        // Many windows wrap into rows within 90 % of the screen.
        s.open((0..12).map(|i| entry(i, 200, 124)).collect(), false);
        let (panel, cards) = s.layout(1280, 800);
        assert!(panel.width <= 1280 * 9 / 10);
        assert!(cards.iter().any(|c| c.y > cards[0].y), "a second row");
    }

    #[test]
    fn thumbnails_keep_the_aspect() {
        let c = Content { width: 400, height: 200, scale: 1, pixels: vec![0xff20_4060; 400 * 200], ..Content::default() };
        let t = thumbnail(&c, 200, 124, 0);
        assert_eq!((t.width, t.height), (200, 100));
        assert!(t.pixels.iter().all(|&p| p == 0x20_4060));
        // Translucent content over the background colour.
        let c = Content { width: 2, height: 2, scale: 1, pixels: vec![0; 4], ..Content::default() };
        assert_eq!(thumbnail(&c, 10, 10, 0x11_2233).pixels, [0x11_2233; 4], "never enlarged");
    }
}
