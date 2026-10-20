//! A terminal screen: a grid of character cells with a cursor, scrollback
//! and the common escape sequences, which knows what it has to redraw.
//!
//! # Model
//!
//! The screen is `rows` lines of `cols` [`Cell`]s. Lines that scroll off
//! the top go into the scrollback; the oldest are dropped when it is full.
//! Screen and scrollback share one ring of lines in caller-provided storage
//! ([`TextGrid::new`]), so scrolling moves no cells. Writing works like a
//! VT100 in its usual modes:
//!
//! - Printable characters go at the cursor, one cell each, and move it
//!   right. At the last column the cursor stays put with a wrap pending:
//!   the next printable character goes to the start of the next line
//!   (scrolling at the bottom). So a line of exactly `cols` characters
//!   followed by a newline does not leave an empty line.
//! - `\n` (and VT, FF) moves down a line, scrolling at the bottom, and back
//!   to the first column ("newline mode", which is what kernel output and a
//!   shell want; [`TextGrid::set_newline_mode`] turns the return off). `\r`
//!   returns, `\x08` moves one cell left (not erasing), `\t` moves to the
//!   next multiple of 8. Other control characters are ignored.
//! - Escape sequences (see the table below). Erasing fills with blanks in
//!   the current background colour.
//!
//! | Sequence | Does |
//! |---|---|
//! | `CSI n A` / `B` / `C` / `D` | cursor up / down / right / left `n` (default 1) |
//! | `CSI n E` / `F` | down / up `n` lines, to the first column |
//! | `CSI n G`, `CSI n d` | to column `n` / row `n` (1-based) |
//! | `CSI r ; c H` (or `f`) | to row `r`, column `c` (1-based, default 1) |
//! | `CSI n J` | erase below (0), above (1), the screen (2), the scrollback (3) |
//! | `CSI n K` | erase to the right (0), to the left (1), the line (2) |
//! | `CSI n @` / `P` / `X` | insert / delete / erase `n` cells at the cursor |
//! | `CSI n L` / `M` | insert / delete `n` lines at the cursor |
//! | `CSI n S` / `T` | scroll up / down `n` lines |
//! | `CSI s`, `CSI u`, `ESC 7`, `ESC 8` | save, restore the cursor (and style) |
//! | `CSI ? 25 h` / `l` | show / hide the cursor |
//! | `CSI ? 7 h` / `l` | line wrap on / off |
//! | `ESC D`, `ESC E`, `ESC M` | index, next line, reverse index |
//! | `ESC c` | reset everything |
//! | `CSI ... m` | style, below |
//!
//! Styles (SGR): 0 reset; 1 bold, 2 faint, 4 underline, 7 inverse, 8 hidden,
//! 9 crossed out, and 22, 24, 27, 28, 29 to undo them; 30-37 / 40-47 the 8
//! ANSI colours, 90-97 / 100-107 their bright versions, 39 / 49 the default,
//! `38;5;n` / `48;5;n` one of 256 indexed colours and `38;2;r;g;b` /
//! `48;2;r;g;b` any RGB colour. Colours stay symbolic in the cells
//! ([`Paint`]) and are looked up in a [`Palette`] when drawn. OSC and other
//! control strings are skipped, as are sequences not listed here.
//!
//! # Drawing
//!
//! [`TextGrid::render`] draws the grid onto a [`Canvas`](crate::Canvas) at
//! any offset, in any [`Font`](crate::Font). It keeps track of what changed
//! since the last call: per screen row, the span of columns written; how
//! many lines the screen scrolled, which it applies by moving pixels
//! ([`Canvas::copy_within`](crate::Canvas::copy_within)) before drawing the
//! new lines; and where it drew the cursor. Only changed cells are drawn,
//! and the call returns the rectangle it touched, for flushing a back buffer.
//! It assumes the canvas still shows what it drew last time; after anything
//! else drew over it, or to draw at another place, call
//! [`TextGrid::invalidate`].

mod parser;
mod render;
#[cfg(test)]
mod tests;

pub use render::Palette;

use core::fmt;
use parser::{Action, Csi, Parser, Utf8};

/// The most rows a grid can have (a 4K screen with an 8x16 font has 135).
pub const MAX_ROWS: usize = 256;
/// The most columns a grid can have.
pub const MAX_COLS: usize = 4096;
const TAB_WIDTH: usize = 8;

/// A colour as the text asked for it, resolved through a [`Palette`] when
/// drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Paint {
    /// The palette's default foreground or background.
    #[default]
    Default,
    /// One of 256 colours: the 16 ANSI colours (0-7, bright 8-15), a 6×6×6
    /// colour cube (16-231) and 24 greys (232-255).
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Text attributes, as a set of flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Attrs(u8);

impl Attrs {
    pub const BOLD: Attrs = Attrs(1 << 0);
    pub const FAINT: Attrs = Attrs(1 << 1);
    pub const UNDERLINE: Attrs = Attrs(1 << 2);
    pub const INVERSE: Attrs = Attrs(1 << 3);
    pub const HIDDEN: Attrs = Attrs(1 << 4);
    pub const STRIKE: Attrs = Attrs(1 << 5);

    pub const fn empty() -> Attrs {
        Attrs(0)
    }

    pub const fn contains(self, other: Attrs) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Attrs) -> Attrs {
        Attrs(self.0 | other.0)
    }

    pub const fn difference(self, other: Attrs) -> Attrs {
        Attrs(self.0 & !other.0)
    }
}

/// How a cell is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    pub fg: Paint,
    pub bg: Paint,
    pub attrs: Attrs,
}

/// One character cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
}

impl Cell {
    /// A space in the default colours.
    pub const BLANK: Cell = Cell { ch: ' ', style: Style { fg: Paint::Default, bg: Paint::Default, attrs: Attrs(0) } };
}

impl Default for Cell {
    fn default() -> Cell {
        Cell::BLANK
    }
}

/// How the cursor is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    /// The cell in the cursor colour, its character in the cell's background.
    #[default]
    Block,
    /// A two-pixel line at the bottom of the cell.
    Underline,
    /// A two-pixel bar at the left of the cell.
    Bar,
}

/// Why [`TextGrid::new`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridError {
    /// No rows or no columns.
    Empty,
    /// More than [`MAX_ROWS`] rows or [`MAX_COLS`] columns.
    TooLarge,
    /// The storage does not hold `rows` lines of `cols` cells.
    StorageTooSmall,
}

/// Columns `lo..hi` of a row; empty when `lo >= hi`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Span {
    lo: u16,
    hi: u16,
}

impl Span {
    const EMPTY: Span = Span { lo: u16::MAX, hi: 0 };

    const fn is_empty(self) -> bool {
        self.lo >= self.hi
    }

    fn add(&mut self, lo: usize, hi: usize) {
        self.lo = self.lo.min(lo as u16);
        self.hi = self.hi.max(hi as u16);
    }
}

/// What `ESC 7` / `CSI s` keep.
#[derive(Clone, Copy, Debug)]
struct Saved {
    col: usize,
    row: usize,
    wrap_pending: bool,
    style: Style,
}

/// A terminal screen over caller-provided cells; see the module
/// documentation.
pub struct TextGrid<'a> {
    cells: &'a mut [Cell],
    cols: usize,
    rows: usize,
    /// Lines in the ring: the screen plus the scrollback's capacity.
    lines: usize,
    /// Ring index of the oldest line kept.
    head: usize,
    /// Lines in the scrollback, above the screen.
    history: usize,
    /// How many lines the view is scrolled back into the scrollback.
    view: usize,
    col: usize,
    row: usize,
    wrap_pending: bool,
    style: Style,
    saved: Option<Saved>,
    autowrap: bool,
    newline_mode: bool,
    cursor_visible: bool,
    cursor_shape: CursorShape,
    parser: Parser,
    utf8: Utf8,
    // What the canvas shows, relative to the cells (see `render`).
    /// Per visible row, the columns changed since the last render.
    dirty: [Span; MAX_ROWS],
    /// Everything must be drawn.
    full: bool,
    /// Lines the visible screen moved up since the last render.
    scrolled: usize,
    /// Where the cursor was drawn (column, visible row).
    drawn_cursor: Option<(usize, usize)>,
    /// The cursor must be drawn again even if it did not move.
    redraw_cursor: bool,
}

impl fmt::Debug for TextGrid<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextGrid")
            .field("cols", &self.cols)
            .field("rows", &self.rows)
            .field("history", &self.history)
            .field("cursor", &(self.col, self.row))
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

impl<'a> TextGrid<'a> {
    /// Cells needed for `cols` × `rows` and `scrollback` lines of history.
    pub const fn storage_len(cols: usize, rows: usize, scrollback: usize) -> usize {
        cols.saturating_mul(rows.saturating_add(scrollback))
    }

    /// A blank grid of `cols` × `rows` in `cells`. Whole lines of `cells`
    /// beyond the screen become the scrollback (see
    /// [`storage_len`](Self::storage_len)).
    pub fn new(cells: &'a mut [Cell], cols: usize, rows: usize) -> Result<TextGrid<'a>, GridError> {
        if cols == 0 || rows == 0 {
            return Err(GridError::Empty);
        }
        if cols > MAX_COLS || rows > MAX_ROWS {
            return Err(GridError::TooLarge);
        }
        let lines = cells.len() / cols;
        if lines < rows {
            return Err(GridError::StorageTooSmall);
        }
        let mut grid = TextGrid {
            cells,
            cols,
            rows,
            lines,
            head: 0,
            history: 0,
            view: 0,
            col: 0,
            row: 0,
            wrap_pending: false,
            style: Style::default(),
            saved: None,
            autowrap: true,
            newline_mode: true,
            cursor_visible: true,
            cursor_shape: CursorShape::Block,
            parser: Parser::default(),
            utf8: Utf8::default(),
            dirty: [Span::EMPTY; MAX_ROWS],
            full: true,
            scrolled: 0,
            drawn_cursor: None,
            redraw_cursor: false,
        };
        grid.reset();
        Ok(grid)
    }

    pub const fn cols(&self) -> usize {
        self.cols
    }

    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// Lines the scrollback can hold.
    pub const fn scrollback_capacity(&self) -> usize {
        self.lines - self.rows
    }

    /// Lines in the scrollback now.
    pub const fn history(&self) -> usize {
        self.history
    }

    /// The cursor: (column, row), 0-based, on the screen.
    pub const fn cursor(&self) -> (usize, usize) {
        (self.col, self.row)
    }

    pub const fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    /// Show or hide the cursor (as `CSI ? 25 h` / `l` do); a terminal blinks
    /// it by toggling this.
    pub fn set_cursor_visible(&mut self, visible: bool) {
        self.cursor_visible = visible;
    }

    pub const fn cursor_shape(&self) -> CursorShape {
        self.cursor_shape
    }

    pub fn set_cursor_shape(&mut self, shape: CursorShape) {
        if shape != self.cursor_shape {
            self.cursor_shape = shape;
            self.redraw_cursor = true;
        }
    }

    /// The style new text gets, as escape sequences left it.
    pub const fn style(&self) -> Style {
        self.style
    }

    pub fn set_style(&mut self, style: Style) {
        self.style = style;
    }

    /// Whether `\n` also returns to the first column (the default).
    pub fn set_newline_mode(&mut self, on: bool) {
        self.newline_mode = on;
    }

    /// Screen row `row`, as cells; `None` past the last row.
    pub fn screen_line(&self, row: usize) -> Option<&[Cell]> {
        (row < self.rows).then(|| self.line(self.screen_ring(row)))
    }

    /// Line `n` of the scrollback, counting up from the line just above the
    /// screen (0); `None` past the oldest.
    pub fn history_line(&self, n: usize) -> Option<&[Cell]> {
        (n < self.history).then(|| self.line(self.ring(self.history - 1 - n)))
    }

    /// The cell at `col`, `row` on the screen.
    pub fn cell(&self, col: usize, row: usize) -> Option<Cell> {
        self.screen_line(row)?.get(col).copied()
    }

    /// How many lines the view is scrolled back (0: it shows the screen).
    pub const fn view_offset(&self) -> usize {
        self.view
    }

    /// Scroll the view `lines` into the scrollback (negative: back toward
    /// the screen), within what there is. New output does not move the
    /// view: it keeps showing the same lines while they last.
    pub fn scroll_view(&mut self, lines: isize) {
        let view = self.view.saturating_add_signed(lines).min(self.history);
        if view != self.view {
            self.view = view;
            self.full = true;
        }
    }

    pub fn scroll_view_to_bottom(&mut self) {
        self.scroll_view(isize::MIN);
    }

    /// Whether [`render`](Self::render) has anything to draw. (The cursor
    /// moving or blinking counts.)
    pub fn needs_render(&self) -> bool {
        self.full
            || self.scrolled > 0
            || self.redraw_cursor
            || self.drawn_cursor != self.cursor_cell()
            || self.dirty[..self.rows].iter().any(|s| !s.is_empty())
    }

    /// Forget what the canvas shows: the next render draws everything.
    pub fn invalidate(&mut self) {
        self.full = true;
    }

    /// Back to the initial state: a blank screen in the default style, an
    /// empty scrollback, the cursor visible at the top left, all modes
    /// default.
    pub fn reset(&mut self) {
        self.cells.fill(Cell::BLANK);
        self.head = 0;
        self.history = 0;
        self.view = 0;
        self.col = 0;
        self.row = 0;
        self.wrap_pending = false;
        self.style = Style::default();
        self.saved = None;
        self.autowrap = true;
        self.cursor_visible = true;
        self.parser.reset();
        self.utf8 = Utf8::default();
        self.full = true;
    }

    /// Write text, interpreting control characters and escape sequences.
    pub fn write_str(&mut self, s: &str) {
        if let Some(c) = self.utf8.flush() {
            self.advance(c);
        }
        for c in s.chars() {
            self.advance(c);
        }
    }

    /// Write UTF-8 bytes, which may stop in the middle of a character (the
    /// rest comes with the next call). Malformed UTF-8 shows as U+FFFD.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            for c in self.utf8.push(b).into_iter().flatten() {
                self.advance(c);
            }
        }
    }

    /// Write one character, as [`write_str`](Self::write_str) would.
    pub fn write_char(&mut self, c: char) {
        let mut buf = [0u8; 4];
        self.write_str(c.encode_utf8(&mut buf));
    }

    // --- Storage -------------------------------------------------------------

    /// Ring index of line `n`, counting from the oldest kept line.
    const fn ring(&self, n: usize) -> usize {
        (self.head + n) % self.lines
    }

    const fn screen_ring(&self, row: usize) -> usize {
        self.ring(self.history + row)
    }

    /// Ring index of the line shown on visible row `row`.
    const fn visible_ring(&self, row: usize) -> usize {
        self.ring(self.history - self.view + row)
    }

    fn line(&self, ring: usize) -> &[Cell] {
        &self.cells[ring * self.cols..(ring + 1) * self.cols]
    }

    fn line_mut(&mut self, ring: usize) -> &mut [Cell] {
        &mut self.cells[ring * self.cols..(ring + 1) * self.cols]
    }

    fn screen_line_mut(&mut self, row: usize) -> &mut [Cell] {
        self.line_mut(self.screen_ring(row))
    }

    /// What erased cells become: blanks in the current background.
    const fn blank(&self) -> Cell {
        Cell { ch: ' ', style: Style { fg: Paint::Default, bg: self.style.bg, attrs: Attrs::empty() } }
    }

    // --- Dirty tracking --------------------------------------------------------

    /// Columns `lo..hi` of visible row `row` need drawing.
    fn mark_visible(&mut self, row: usize, lo: usize, hi: usize) {
        if row < self.rows && lo < hi {
            self.dirty[row].add(lo, hi.min(self.cols));
        }
    }

    /// Columns `lo..hi` of screen row `row` changed.
    fn mark(&mut self, row: usize, lo: usize, hi: usize) {
        self.mark_visible(row + self.view, lo, hi);
    }

    fn mark_rows(&mut self, rows: core::ops::Range<usize>) {
        for row in rows {
            self.mark(row, 0, self.cols);
        }
    }

    /// Where the cursor is drawn now: (column, visible row), if at all.
    fn cursor_cell(&self) -> Option<(usize, usize)> {
        let row = self.row + self.view;
        (self.cursor_visible && row < self.rows).then_some((self.col, row))
    }

    // --- Editing -----------------------------------------------------------------

    fn erase(&mut self, row: usize, lo: usize, hi: usize) {
        let (blank, hi) = (self.blank(), hi.min(self.cols));
        if lo < hi {
            self.screen_line_mut(row)[lo..hi].fill(blank);
            self.mark(row, lo, hi);
        }
    }

    /// Move the screen up a line: the top line goes into the scrollback
    /// (dropping the oldest when it is full), a blank one comes in at the
    /// bottom.
    fn scroll_up(&mut self) {
        let full_history = self.history == self.scrollback_capacity();
        if full_history {
            self.head = (self.head + 1) % self.lines;
        } else {
            self.history += 1;
        }
        let blank = self.blank();
        self.screen_line_mut(self.rows - 1).fill(blank);
        if self.view > 0 {
            // Keep showing the same lines, unless they were the oldest and
            // just dropped out.
            if full_history && self.view == self.history {
                self.full = true;
            } else {
                self.view += 1;
            }
        } else {
            // The pixels move up with the lines, and so does what still has
            // to be drawn; the new bottom line is all new.
            self.dirty.copy_within(1..self.rows, 0);
            self.dirty[self.rows - 1] = Span::EMPTY;
            self.mark_visible(self.rows - 1, 0, self.cols);
            self.scrolled += 1;
            if self.scrolled >= self.rows {
                self.full = true;
            }
        }
    }

    /// Move screen lines `top..` down by `n` (`down`) or lines `top + n..`
    /// up to `top` (`!down`), blanking the lines that open up. The
    /// scrollback is not involved.
    fn shift_lines(&mut self, top: usize, n: usize, down: bool) {
        let n = n.min(self.rows - top);
        let cols = self.cols;
        let moved = self.rows - top - n;
        for i in 0..moved {
            let (from, to) = if down { (self.rows - 1 - n - i, self.rows - 1 - i) } else { (top + n + i, top + i) };
            let (src, dst) = (self.screen_ring(from) * cols, self.screen_ring(to) * cols);
            self.cells.copy_within(src..src + cols, dst);
        }
        let opened = if down { top..top + n } else { self.rows - n..self.rows };
        for row in opened {
            self.erase(row, 0, cols);
        }
        self.mark_rows(top..self.rows);
    }

    fn linefeed(&mut self) {
        if self.row + 1 == self.rows {
            self.scroll_up();
        } else {
            self.row += 1;
        }
    }

    fn reverse_index(&mut self) {
        if self.row == 0 {
            self.shift_lines(0, 1, true);
        } else {
            self.row -= 1;
        }
    }

    fn print(&mut self, c: char) {
        if self.wrap_pending && self.autowrap {
            self.col = 0;
            self.linefeed();
        }
        self.wrap_pending = false;
        let (col, row, style) = (self.col, self.row, self.style);
        self.screen_line_mut(row)[col] = Cell { ch: c, style };
        self.mark(row, col, col + 1);
        if col + 1 < self.cols {
            self.col += 1;
        } else {
            self.wrap_pending = self.autowrap;
        }
    }

    fn execute(&mut self, c: char) {
        match c {
            '\n' | '\x0b' | '\x0c' => {
                self.linefeed();
                if self.newline_mode {
                    self.col = 0;
                }
            }
            '\r' => self.col = 0,
            '\x08' => self.col = self.col.saturating_sub(1),
            '\t' => self.col = ((self.col / TAB_WIDTH + 1) * TAB_WIDTH).min(self.cols - 1),
            _ => return,
        }
        self.wrap_pending = false;
    }

    fn save_cursor(&mut self) {
        self.saved = Some(Saved { col: self.col, row: self.row, wrap_pending: self.wrap_pending, style: self.style });
    }

    fn restore_cursor(&mut self) {
        let s = self.saved.unwrap_or(Saved { col: 0, row: 0, wrap_pending: false, style: Style::default() });
        (self.col, self.row) = (s.col.min(self.cols - 1), s.row.min(self.rows - 1));
        (self.wrap_pending, self.style) = (s.wrap_pending, s.style);
    }

    fn esc(&mut self, c: char) {
        match c {
            '7' => self.save_cursor(),
            '8' => self.restore_cursor(),
            'c' => self.reset(),
            'D' => self.linefeed(),
            'E' => {
                self.col = 0;
                self.linefeed();
            }
            'M' => self.reverse_index(),
            _ => return,
        }
        if c != '8' {
            self.wrap_pending = false;
        }
    }

    fn csi(&mut self, p: &Csi) {
        if p.intermediates {
            return;
        }
        if p.private == b'?' {
            let on = match p.final_byte {
                b'h' => true,
                b'l' => false,
                _ => return,
            };
            for i in 0..p.len() {
                match p.get(i) {
                    25 => self.cursor_visible = on,
                    7 => self.autowrap = on,
                    _ => {}
                }
            }
            return;
        }
        if p.private != 0 {
            return;
        }
        let n = p.or(0, 1) as usize;
        let (cols, rows) = (self.cols, self.rows);
        match p.final_byte {
            b'A' => self.row = self.row.saturating_sub(n),
            b'B' => self.row = (self.row + n).min(rows - 1),
            b'C' => self.col = (self.col + n).min(cols - 1),
            b'D' => self.col = self.col.saturating_sub(n),
            b'E' => (self.row, self.col) = ((self.row + n).min(rows - 1), 0),
            b'F' => (self.row, self.col) = (self.row.saturating_sub(n), 0),
            b'G' | b'`' => self.col = (n - 1).min(cols - 1),
            b'd' => self.row = (n - 1).min(rows - 1),
            b'H' | b'f' => {
                self.row = (p.or(0, 1) as usize - 1).min(rows - 1);
                self.col = (p.or(1, 1) as usize - 1).min(cols - 1);
            }
            b'J' => match p.get(0) {
                0 => {
                    self.erase(self.row, self.col, cols);
                    for row in self.row + 1..rows {
                        self.erase(row, 0, cols);
                    }
                }
                1 => {
                    for row in 0..self.row {
                        self.erase(row, 0, cols);
                    }
                    self.erase(self.row, 0, self.col + 1);
                }
                2 => {
                    for row in 0..rows {
                        self.erase(row, 0, cols);
                    }
                }
                3 => {
                    self.head = self.ring(self.history);
                    self.history = 0;
                    self.view = 0;
                    self.full = true;
                }
                _ => {}
            },
            b'K' => match p.get(0) {
                0 => self.erase(self.row, self.col, cols),
                1 => self.erase(self.row, 0, self.col + 1),
                2 => self.erase(self.row, 0, cols),
                _ => {}
            },
            b'@' | b'P' => {
                let (col, row, blank) = (self.col, self.row, self.blank());
                let n = n.min(cols - col);
                let line = &mut self.screen_line_mut(row)[col..];
                if p.final_byte == b'@' {
                    line.copy_within(..line.len() - n, n);
                    line[..n].fill(blank);
                } else {
                    line.copy_within(n.., 0);
                    let len = line.len();
                    line[len - n..].fill(blank);
                }
                self.mark(row, col, cols);
            }
            b'X' => self.erase(self.row, self.col, self.col.saturating_add(n)),
            b'L' => self.shift_lines(self.row, n, true),
            b'M' => self.shift_lines(self.row, n, false),
            b'S' => {
                for _ in 0..n.min(rows) {
                    self.scroll_up();
                }
            }
            b'T' => self.shift_lines(0, n, true),
            b'm' => self.sgr(p),
            b's' => self.save_cursor(),
            b'u' => {
                self.restore_cursor();
                return;
            }
            _ => return,
        }
        self.wrap_pending = false;
    }

    fn sgr(&mut self, p: &Csi) {
        if p.len() == 0 {
            self.style = Style::default();
            return;
        }
        let s = &mut self.style;
        let mut i = 0;
        while i < p.len() {
            match p.get(i) {
                0 => *s = Style::default(),
                1 => s.attrs = s.attrs.union(Attrs::BOLD),
                2 => s.attrs = s.attrs.union(Attrs::FAINT),
                4 | 21 => s.attrs = s.attrs.union(Attrs::UNDERLINE),
                7 => s.attrs = s.attrs.union(Attrs::INVERSE),
                8 => s.attrs = s.attrs.union(Attrs::HIDDEN),
                9 => s.attrs = s.attrs.union(Attrs::STRIKE),
                22 => s.attrs = s.attrs.difference(Attrs::BOLD.union(Attrs::FAINT)),
                24 => s.attrs = s.attrs.difference(Attrs::UNDERLINE),
                27 => s.attrs = s.attrs.difference(Attrs::INVERSE),
                28 => s.attrs = s.attrs.difference(Attrs::HIDDEN),
                29 => s.attrs = s.attrs.difference(Attrs::STRIKE),
                n @ 30..=37 => s.fg = Paint::Indexed(n as u8 - 30),
                n @ 40..=47 => s.bg = Paint::Indexed(n as u8 - 40),
                n @ 90..=97 => s.fg = Paint::Indexed(n as u8 - 90 + 8),
                n @ 100..=107 => s.bg = Paint::Indexed(n as u8 - 100 + 8),
                39 => s.fg = Paint::Default,
                49 => s.bg = Paint::Default,
                n @ (38 | 48) => {
                    let (paint, used) = extended_color(p, i + 1);
                    if let Some(paint) = paint {
                        if n == 38 {
                            s.fg = paint;
                        } else {
                            s.bg = paint;
                        }
                    }
                    i += used;
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn advance(&mut self, c: char) {
        match self.parser.advance(c) {
            Action::None => {}
            Action::Print(c) => self.print(c),
            Action::Execute(c) => self.execute(c),
            Action::Esc(c) => self.esc(c),
            Action::Csi(csi) => self.csi(&csi),
        }
    }
}

/// The colour after `38` / `48` at parameter `i`: `5;n` or `2;r;g;b`, and
/// how many parameters it took.
fn extended_color(p: &Csi, i: usize) -> (Option<Paint>, usize) {
    let byte = |j: usize| p.get(j).min(255) as u8;
    match p.get(i) {
        5 if i + 1 < p.len() => (Some(Paint::Indexed(byte(i + 1))), 2),
        2 if i + 3 < p.len() => (Some(Paint::Rgb(byte(i + 1), byte(i + 2), byte(i + 3))), 4),
        // Malformed: skip the rest of the sequence, as its meaning is lost.
        _ => (None, p.len()),
    }
}

impl fmt::Write for TextGrid<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        TextGrid::write_str(self, s);
        Ok(())
    }
}
