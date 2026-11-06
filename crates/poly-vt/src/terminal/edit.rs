//! Writing characters, moving the cursor, scrolling, erasing, inserting
//! and deleting: the editing operations the control functions map to.
//!
//! Every operation clamps its arguments to the screen, so no parameter can
//! index out of bounds. Scrolling happens inside the scroll region
//! (`top..=bottom`); lines scrolled off the top of the primary screen with
//! the top margin at row 0 go to the scrollback.

use super::Terminal;
use super::screen::Line;
use crate::cell::Cell;
use crate::width::char_width;

impl Terminal {
    /// The blank erasing leaves: a space in the current background.
    pub(super) fn blank(&self) -> Cell {
        Cell::blank(self.pen.bg)
    }

    fn blank_line(&self) -> Line {
        Line::blank(self.cols, self.blank())
    }

    /// Note that screen row `row` changed.
    pub(super) fn mark(&mut self, row: usize) {
        let d = row + self.view_offset;
        if let Some(r) = self.dirty.get_mut(d) {
            *r = true;
        }
    }

    pub(super) fn mark_rows(&mut self, a: usize, b: usize) {
        for r in a..=b {
            self.mark(r);
        }
    }

    pub(super) fn print_char(&mut self, c: char) {
        let c = self.charsets[self.gl].map(c);
        let w = char_width(c);
        if w == 0 {
            // Combining marks and other zero-width characters are dropped
            // (see `crate::width`).
            return;
        }
        self.last_char = Some(c);
        self.put(c, usize::from(w));
    }

    /// Write `c`, `w` cells wide, at the cursor.
    pub(super) fn put(&mut self, c: char, w: usize) {
        let cols = self.cols;
        if w > cols {
            return;
        }
        if self.cur.pending_wrap {
            self.cur.pending_wrap = false;
            if self.modes.autowrap {
                self.wrap();
            }
        }
        if self.cur.col + w > cols {
            if self.modes.autowrap {
                let (row, col) = (self.cur.row, self.cur.col);
                let blank = self.blank();
                self.screen[row].erase(col, cols, blank);
                self.wrap();
            } else {
                self.cur.col = cols - w;
            }
        }
        let (row, col) = (self.cur.row, self.cur.col);
        if self.modes.insert {
            self.insert_cells(w);
        }
        let blank = self.blank();
        let pen = self.pen;
        let line = &mut self.screen[row];
        line.split_wide(col, col + w, blank);
        line.cells[col] = Cell { ch: c, fg: pen.fg, bg: pen.bg, attrs: pen.attrs, width: w as u8 };
        if w == 2 {
            line.cells[col + 1] = Cell { ch: ' ', fg: pen.fg, bg: pen.bg, attrs: pen.attrs, width: 0 };
        }
        self.mark(row);
        if col + w >= cols {
            self.cur.col = cols - 1;
            self.cur.pending_wrap = self.modes.autowrap;
        } else {
            self.cur.col = col + w;
        }
    }

    /// Move to the start of the next line for autowrap, marking this one
    /// as continued.
    fn wrap(&mut self) {
        self.screen[self.cur.row].wrapped = true;
        self.cur.col = 0;
        self.index();
    }

    /// REP: write the last character `n` more times.
    pub(super) fn repeat(&mut self, n: usize) {
        if let Some(c) = self.last_char {
            let w = usize::from(char_width(c));
            for _ in 0..n.min(self.cols * self.rows) {
                self.put(c, w);
            }
        }
    }

    pub(super) fn backspace(&mut self) {
        self.cur.pending_wrap = false;
        self.cur.col = self.cur.col.saturating_sub(1);
    }

    pub(super) fn carriage_return(&mut self) {
        self.cur.pending_wrap = false;
        self.cur.col = 0;
    }

    /// LF: down a line, scrolling at the bottom margin.
    pub(super) fn linefeed(&mut self) {
        self.index();
    }

    /// IND: down a line, scrolling the region up at its bottom margin.
    pub(super) fn index(&mut self) {
        self.cur.pending_wrap = false;
        if self.cur.row == self.bottom {
            self.scroll_up(self.top, self.bottom, 1, true);
        } else if self.cur.row + 1 < self.rows {
            self.cur.row += 1;
        }
    }

    /// RI: up a line, scrolling the region down at its top margin.
    pub(super) fn reverse_index(&mut self) {
        self.cur.pending_wrap = false;
        if self.cur.row == self.top {
            self.scroll_down(self.top, self.bottom, 1);
        } else {
            self.cur.row = self.cur.row.saturating_sub(1);
        }
    }

    /// Scroll lines `top..=bottom` up by `n`, blank lines coming in at the
    /// bottom. With `history`, lines leaving the top of the primary screen
    /// at row 0 go to the scrollback.
    pub(super) fn scroll_up(&mut self, top: usize, bottom: usize, n: usize, history: bool) {
        if top > bottom || bottom >= self.rows {
            return;
        }
        let n = n.min(bottom - top + 1);
        if n == 0 {
            return;
        }
        let blank = self.blank_line();
        let removed: alloc::vec::Vec<Line> = self.screen.drain(top..top + n).collect();
        let at = bottom + 1 - n;
        self.screen.splice(at..at, core::iter::repeat_n(blank, n));
        if history && top == 0 && !self.modes.alt_screen {
            let full = bottom == self.rows - 1;
            for line in removed {
                self.push_history(line, full);
            }
        }
        self.mark_rows(top, bottom);
    }

    /// Scroll lines `top..=bottom` down by `n`, blank lines coming in at
    /// the top.
    pub(super) fn scroll_down(&mut self, top: usize, bottom: usize, n: usize) {
        if top > bottom || bottom >= self.rows {
            return;
        }
        let n = n.min(bottom - top + 1);
        if n == 0 {
            return;
        }
        let blank = self.blank_line();
        self.screen.drain(bottom + 1 - n..=bottom);
        self.screen.splice(top..top, core::iter::repeat_n(blank, n));
        self.mark_rows(top, bottom);
    }

    fn push_history(&mut self, mut line: Line, full_screen: bool) {
        if self.scrollback_cap == 0 {
            return;
        }
        line.trim();
        self.scrollback.push_back(line);
        self.scrolled += 1;
        if self.scrollback.len() > self.scrollback_cap {
            self.scrollback.pop_front();
        }
        if self.view_offset > 0 {
            // Keep the view on the same lines.
            self.view_offset += 1;
            if self.view_offset > self.scrollback.len() {
                self.view_offset = self.scrollback.len();
                self.damage_all = true;
            }
            if !full_screen {
                self.damage_all = true;
            }
        }
    }

    // Cursor movement. All of these end any pending wrap.

    /// Go to `row`, `col` (screen coordinates), clamped.
    pub(super) fn goto(&mut self, row: usize, col: usize) {
        self.cur.row = row.min(self.rows - 1);
        self.cur.col = col.min(self.cols - 1);
        self.cur.pending_wrap = false;
    }

    /// Go to 0-based `row`, `col` as CUP addresses them: relative to the
    /// scroll region and confined to it in origin mode.
    pub(super) fn goto_origin(&mut self, row: usize, col: usize) {
        if self.modes.origin {
            let row = (self.top + row).min(self.bottom);
            self.goto(row, col);
        } else {
            self.goto(row, col);
        }
    }

    pub(super) fn cursor_up(&mut self, n: usize) {
        let min = if self.cur.row >= self.top { self.top } else { 0 };
        let row = self.cur.row.saturating_sub(n).max(min);
        self.goto(row, self.cur.col);
    }

    pub(super) fn cursor_down(&mut self, n: usize) {
        let max = if self.cur.row <= self.bottom { self.bottom } else { self.rows - 1 };
        let row = self.cur.row.saturating_add(n).min(max);
        self.goto(row, self.cur.col);
    }

    pub(super) fn cursor_right(&mut self, n: usize) {
        self.goto(self.cur.row, self.cur.col.saturating_add(n));
    }

    pub(super) fn cursor_left(&mut self, n: usize) {
        self.goto(self.cur.row, self.cur.col.saturating_sub(n));
    }

    /// HT / CHT: forward `n` tab stops (the last column if none).
    pub(super) fn tab(&mut self, n: usize) {
        let mut col = self.cur.col;
        for _ in 0..n.min(self.cols) {
            col = (col + 1..self.cols).find(|&c| self.tabs[c]).unwrap_or(self.cols - 1);
        }
        self.cur.col = col;
        self.cur.pending_wrap = false;
    }

    /// CBT: back `n` tab stops (the first column if none).
    pub(super) fn back_tab(&mut self, n: usize) {
        let mut col = self.cur.col;
        for _ in 0..n.min(self.cols) {
            col = (0..col).rev().find(|&c| self.tabs[c]).unwrap_or(0);
        }
        self.cur.col = col;
        self.cur.pending_wrap = false;
    }

    // Erasing.

    /// EL: erase to the right (0), to the left (1), or the line (2).
    pub(super) fn erase_line(&mut self, mode: u16) {
        let (row, col, cols) = (self.cur.row, self.cur.col, self.cols);
        let blank = self.blank();
        let line = &mut self.screen[row];
        match mode {
            0 => {
                line.erase(col, cols, blank);
                line.wrapped = false;
            }
            1 => line.erase(0, col + 1, blank),
            2 => {
                line.erase(0, cols, blank);
                line.wrapped = false;
            }
            _ => return,
        }
        self.mark(row);
    }

    /// ED: erase below (0), above (1), the screen (2), the scrollback (3).
    pub(super) fn erase_display(&mut self, mode: u16) {
        let row = self.cur.row;
        let blank = self.blank_line();
        match mode {
            0 => {
                self.erase_line(0);
                for r in row + 1..self.rows {
                    self.screen[r] = blank.clone();
                }
                self.mark_rows(row, self.rows - 1);
            }
            1 => {
                self.erase_line(1);
                for r in 0..row {
                    self.screen[r] = blank.clone();
                }
                self.mark_rows(0, row);
            }
            2 => {
                self.screen.fill(blank);
                self.mark_rows(0, self.rows - 1);
            }
            3 => {
                self.scrollback.clear();
                self.view_offset = 0;
                self.damage_all = true;
            }
            _ => {}
        }
    }

    /// ECH: erase `n` cells from the cursor on.
    pub(super) fn erase_chars(&mut self, n: usize) {
        let (row, col) = (self.cur.row, self.cur.col);
        let blank = self.blank();
        self.screen[row].erase(col, col.saturating_add(n.max(1)), blank);
        self.mark(row);
    }

    /// ICH (and insert mode): shift the rest of the line right `n` cells,
    /// blanks coming in at the cursor.
    pub(super) fn insert_cells(&mut self, n: usize) {
        let (row, col, cols) = (self.cur.row, self.cur.col, self.cols);
        let n = n.min(cols - col);
        let blank = self.blank();
        let line = &mut self.screen[row];
        line.split_wide(col, col, blank);
        if line.cells[col].width == 0 {
            line.cells[col] = blank;
        }
        line.cells[col..].rotate_right(n);
        line.cells[col..col + n].fill(blank);
        if line.cells[cols - 1].width == 2 {
            line.cells[cols - 1] = blank;
        }
        self.cur.pending_wrap = false;
        self.mark(row);
    }

    /// DCH: delete `n` cells at the cursor, the rest of the line moving
    /// left and blanks coming in at the end.
    pub(super) fn delete_cells(&mut self, n: usize) {
        let (row, col, cols) = (self.cur.row, self.cur.col, self.cols);
        let n = n.min(cols - col);
        let blank = self.blank();
        let line = &mut self.screen[row];
        line.split_wide(col, col + n, blank);
        if line.cells[col].width == 0 {
            line.cells[col] = blank;
        }
        line.cells[col..].rotate_left(n);
        line.cells[cols - n..].fill(blank);
        self.cur.pending_wrap = false;
        self.mark(row);
    }

    /// IL: insert `n` blank lines at the cursor (inside the scroll region).
    pub(super) fn insert_lines(&mut self, n: usize) {
        if (self.top..=self.bottom).contains(&self.cur.row) {
            self.scroll_down(self.cur.row, self.bottom, n);
            self.carriage_return();
        }
    }

    /// DL: delete `n` lines at the cursor (inside the scroll region).
    pub(super) fn delete_lines(&mut self, n: usize) {
        if (self.top..=self.bottom).contains(&self.cur.row) {
            self.scroll_up(self.cur.row, self.bottom, n, false);
            self.carriage_return();
        }
    }

    /// DECALN: fill the screen with `E`, reset the margins, home.
    pub(super) fn alignment_test(&mut self) {
        let e = Cell { ch: 'E', ..Cell::BLANK };
        self.screen.fill(Line::blank(self.cols, e));
        self.top = 0;
        self.bottom = self.rows - 1;
        self.goto(0, 0);
        self.damage_all = true;
    }
}
