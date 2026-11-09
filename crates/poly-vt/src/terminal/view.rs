//! What the renderer and the selection see: the displayed rows with the
//! scrollback view applied, damage tracking, selected text and search.
//!
//! # Coordinates
//!
//! *Display rows* are what is on the window: `0..rows`. With the view
//! scrolled `n` lines back, display row `d` shows *absolute line* `d - n`.
//! Absolute line 0 is the first row of the screen, 1 the second, and so
//! on; -1 is the newest scrollback line, -2 the one before. Absolute lines
//! move by one each time a line scrolls into the scrollback;
//! [`Terminal::scroll_count`] tells by how much, so a selection can follow
//! its text.
//!
//! Scrollback lines keep the width they had (trailing default blanks are
//! trimmed to save memory), so [`Terminal::visible_row`] may return fewer
//! cells than `cols` (the rest is blank) or, after the terminal got
//! narrower, more (draw only the first `cols`).

use super::Terminal;
use super::screen::Line;
use crate::cell::Cell;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// What changed on the display since [`Terminal::take_damage`] was last
/// called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Damage {
    /// Redraw everything (resize, view scroll, screen switch, reverse
    /// video, ...).
    All,
    /// `rows[d]`: display row `d` changed (its cells, or the cursor moved
    /// onto or off it).
    Rows(Vec<bool>),
}

impl Damage {
    /// Whether display row `row` needs drawing.
    pub fn contains(&self, row: u16) -> bool {
        match self {
            Damage::All => true,
            Damage::Rows(r) => r.get(usize::from(row)).copied().unwrap_or(false),
        }
    }

    /// Whether nothing needs drawing.
    pub fn is_empty(&self) -> bool {
        match self {
            Damage::All => false,
            Damage::Rows(r) => !r.contains(&true),
        }
    }
}

impl Terminal {
    /// Lines in the scrollback (0 while the alternate screen is shown: it
    /// has none).
    pub fn scrollback_len(&self) -> usize {
        if self.modes.alt_screen { 0 } else { self.scrollback.len() }
    }

    /// How many lines the view is scrolled back (0: the live screen).
    pub fn view_offset(&self) -> usize {
        self.view_offset
    }

    /// Scroll the view `delta` lines back into the history (negative:
    /// toward the live screen), clamped.
    pub fn scroll_view(&mut self, delta: isize) {
        let max = self.scrollback_len();
        let new = self.view_offset.saturating_add_signed(delta).min(max);
        if new != self.view_offset {
            self.view_offset = new;
            self.damage_all = true;
        }
    }

    /// Back to the live screen.
    pub fn reset_view(&mut self) {
        self.scroll_view(isize::MIN);
    }

    /// How many lines ever scrolled into the scrollback; absolute line
    /// numbers shift down by one for each.
    pub fn scroll_count(&self) -> u64 {
        self.scrolled
    }

    /// The absolute line display row `row` shows.
    pub fn display_to_absolute(&self, row: u16) -> i64 {
        i64::from(row) - self.view_offset as i64
    }

    fn line_at(&self, abs: i64) -> Option<&Line> {
        if abs >= 0 {
            self.screen.get(usize::try_from(abs).ok()?)
        } else {
            let back = usize::try_from(abs.unsigned_abs()).ok()?;
            let len = self.scrollback_len();
            if back > len { None } else { self.scrollback.get(len - back) }
        }
    }

    /// The cells of display row `row` (empty past the last row).
    pub fn visible_row(&self, row: u16) -> &[Cell] {
        if usize::from(row) >= self.rows {
            return &[];
        }
        self.line_at(self.display_to_absolute(row)).map_or(&[], |l| &l.cells)
    }

    /// Whether display row `row` continues on the next one (it was
    /// autowrapped).
    pub fn line_wrapped(&self, row: u16) -> bool {
        usize::from(row) < self.rows && self.line_at(self.display_to_absolute(row)).is_some_and(|l| l.wrapped)
    }

    /// Where the cursor shows on the display, if it is visible there.
    pub fn cursor_on_display(&self) -> Option<(u16, u16)> {
        let row = self.cur.row + self.view_offset;
        (self.cursor_visible && row < self.rows).then_some((row as u16, self.cur.col as u16))
    }

    /// The display rows that changed since the last call. The rows the
    /// cursor was on and is on now count as changed when it moved.
    pub fn take_damage(&mut self) -> Damage {
        let cursor = self.cursor_on_display().map(|(r, c)| (usize::from(r), usize::from(c)));
        if cursor != self.last_cursor {
            for (r, _) in [self.last_cursor, cursor].into_iter().flatten() {
                if let Some(d) = self.dirty.get_mut(r) {
                    *d = true;
                }
            }
            self.last_cursor = cursor;
        }
        if core::mem::take(&mut self.damage_all) {
            self.dirty.fill(false);
            return Damage::All;
        }
        Damage::Rows(core::mem::replace(&mut self.dirty, vec![false; self.rows]))
    }

    /// The text from `from` to `to` (inclusive, absolute line and column,
    /// either order). Trailing blanks of each line are dropped; lines end
    /// in `\n` unless they wrapped onto the next.
    pub fn text(&self, from: (i64, u16), to: (i64, u16)) -> String {
        let (mut a, mut b) = if from <= to { (from, to) } else { (to, from) };
        let first = -(self.scrollback_len() as i64);
        let last = self.rows as i64 - 1;
        if a.0 > last || b.0 < first {
            return String::new();
        }
        if a.0 < first {
            a = (first, 0);
        }
        if b.0 > last {
            b = (last, u16::MAX);
        }
        let mut out = String::new();
        let mut abs = a.0;
        loop {
            let cells = self.line_at(abs).map_or(&[][..], |l| &l.cells[..]);
            let start = if abs == a.0 { usize::from(a.1) } else { 0 };
            let end = if abs == b.0 { usize::from(b.1).saturating_add(1) } else { usize::MAX };
            let end = end.min(cells.len()).min(self.cols);
            let at_end = end == cells.len().min(self.cols);
            let wrapped = abs != b.0 && at_end && self.line_at(abs).is_some_and(|l| l.wrapped);
            let seg_start = out.len();
            for cell in cells.get(start..end).unwrap_or(&[]) {
                if cell.width != 0 {
                    out.push(if cell.ch == '\0' { ' ' } else { cell.ch });
                }
            }
            if !wrapped {
                let trimmed = out[seg_start..].trim_end_matches(' ').len();
                out.truncate(seg_start + trimmed);
            }
            if abs >= b.0 {
                break;
            }
            if !wrapped {
                out.push('\n');
            }
            abs += 1;
        }
        out
    }

    /// The last place `needle` occurs, searching from the bottom of the
    /// screen up through the scrollback, as (absolute line, column).
    /// Matches do not span lines.
    pub fn search(&self, needle: &str) -> Option<(i64, u16)> {
        self.search_before(needle, (self.rows as i64, 0))
    }

    /// The last place `needle` occurs before `before` (absolute line,
    /// column), for "find previous".
    pub fn search_before(&self, needle: &str, before: (i64, u16)) -> Option<(i64, u16)> {
        let needle: Vec<char> = needle.chars().collect();
        if needle.is_empty() {
            return None;
        }
        let first = -(self.scrollback_len() as i64);
        let mut abs = before.0.min(self.rows as i64 - 1);
        while abs >= first {
            if let Some(line) = self.line_at(abs) {
                let (chars, cols): (Vec<char>, Vec<u16>) =
                    line.cells.iter().enumerate().filter(|(_, c)| c.width != 0).map(|(i, c)| (c.ch, i as u16)).unzip();
                let limit = if abs == before.0 { before.1 } else { u16::MAX };
                if chars.len() >= needle.len() {
                    for i in (0..=chars.len() - needle.len()).rev() {
                        if cols[i] < limit && chars[i..i + needle.len()] == needle[..] {
                            return Some((abs, cols[i]));
                        }
                    }
                }
            }
            abs -= 1;
        }
        None
    }
}
