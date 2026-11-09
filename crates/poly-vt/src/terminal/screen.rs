//! Lines of cells and the operations on whole screens that do not need
//! the rest of the terminal state: blank lines, wide-character repair,
//! and resizing a screen's columns and rows.

use crate::cell::Cell;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

/// One line of the screen or the scrollback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub cells: Vec<Cell>,
    /// The text continues on the next line (autowrap moved on from here).
    pub wrapped: bool,
}

impl Line {
    pub fn blank(cols: usize, blank: Cell) -> Line {
        Line { cells: vec![blank; cols], wrapped: false }
    }

    /// Make `cells[a..b]` safe to overwrite: a wide character cut by
    /// either edge loses its other half (it becomes `blank`).
    pub fn split_wide(&mut self, a: usize, b: usize, blank: Cell) {
        let len = self.cells.len();
        if a < len && self.cells[a].width == 0 && a > 0 {
            self.cells[a - 1] = blank;
        }
        if b > 0 && b <= len && self.cells[b - 1].width == 2 && b < len {
            self.cells[b] = blank;
        }
    }

    /// Fill `cells[a..b]` with `blank` (clamped to the line).
    pub fn erase(&mut self, a: usize, b: usize, blank: Cell) {
        let b = b.min(self.cells.len());
        if a >= b {
            return;
        }
        self.split_wide(a, b, blank);
        self.cells[a..b].fill(blank);
    }

    /// Truncate or pad to `cols` cells, never leaving half a wide character.
    pub fn set_width(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::BLANK);
        if let Some(last) = self.cells.last_mut()
            && last.width == 2
        {
            *last = Cell::BLANK;
        }
        if self.cells.first().is_some_and(|c| c.width == 0) {
            self.cells[0] = Cell::BLANK;
        }
    }

    /// Drop trailing default blank cells (for the scrollback).
    pub fn trim(&mut self) {
        if self.wrapped {
            return;
        }
        while self.cells.last() == Some(&Cell::BLANK) {
            self.cells.pop();
        }
        self.cells.shrink_to_fit();
    }
}

/// Change a screen's row count to `rows` (columns are already `cols`).
///
/// Shrinking drops lines below the cursor first; if the cursor line would
/// still be off the screen, lines from the top go to `history` (or are
/// dropped without one). Growing pulls lines back from `history` above the
/// screen (keeping the cursor on its line), then adds blank lines below.
/// Returns how many lines moved into the history (negative: out of it).
pub(crate) fn resize_rows(
    lines: &mut Vec<Line>,
    cursor_row: &mut usize,
    rows: usize,
    cols: usize,
    mut history: Option<(&mut VecDeque<Line>, usize)>,
) -> isize {
    let old = lines.len();
    let mut moved: isize = 0;
    if rows < old {
        let push = (*cursor_row + 1).saturating_sub(rows);
        for mut line in lines.drain(..push) {
            if let Some((h, cap)) = history.as_mut()
                && *cap > 0
            {
                line.trim();
                h.push_back(line);
                if h.len() > *cap {
                    h.pop_front();
                }
            }
        }
        moved = push as isize;
        *cursor_row -= push;
        lines.truncate(rows);
    } else if rows > old {
        if let Some((h, _)) = history.as_mut() {
            let pull = (rows - old).min(h.len());
            for _ in 0..pull {
                if let Some(mut line) = h.pop_back() {
                    line.set_width(cols);
                    lines.insert(0, line);
                }
            }
            *cursor_row += pull;
            moved = -(pull as isize);
        }
        while lines.len() < rows {
            lines.push(Line::blank(cols, Cell::BLANK));
        }
    }
    *cursor_row = (*cursor_row).min(rows.saturating_sub(1));
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(ch: char) -> [Cell; 2] {
        [Cell { ch, width: 2, ..Cell::BLANK }, Cell { width: 0, ..Cell::BLANK }]
    }

    #[test]
    fn erase_splits_wide_chars() {
        let mut l = Line::blank(4, Cell::BLANK);
        l.cells[0..2].copy_from_slice(&wide('中'));
        l.cells[2..4].copy_from_slice(&wide('文'));
        l.erase(1, 3, Cell::BLANK);
        assert!(l.cells.iter().all(|c| *c == Cell::BLANK));
    }

    #[test]
    fn set_width_drops_cut_wide_char() {
        let mut l = Line::blank(4, Cell::BLANK);
        l.cells[2..4].copy_from_slice(&wide('文'));
        l.set_width(3);
        assert_eq!(l.cells.len(), 3);
        assert_eq!(l.cells[2], Cell::BLANK);
    }

    #[test]
    fn resize_rows_moves_lines_through_history() {
        let mk = |c: char| Line { cells: vec![Cell { ch: c, ..Cell::BLANK }; 2], wrapped: false };
        let mut lines = vec![mk('a'), mk('b'), mk('c'), mk('d')];
        let mut h = VecDeque::new();
        let mut cur = 2;
        assert_eq!(resize_rows(&mut lines, &mut cur, 2, 2, Some((&mut h, 10))), 1);
        assert_eq!(cur, 1);
        assert_eq!(lines[0].cells[0].ch, 'b');
        assert_eq!(lines[1].cells[0].ch, 'c');
        assert_eq!(h.len(), 1);
        assert_eq!(resize_rows(&mut lines, &mut cur, 4, 2, Some((&mut h, 10))), -1);
        assert_eq!(cur, 2);
        assert_eq!(lines[0].cells[0].ch, 'a');
        assert_eq!(lines[3].cells[0].ch, ' ');
        assert!(h.is_empty());
    }
}
