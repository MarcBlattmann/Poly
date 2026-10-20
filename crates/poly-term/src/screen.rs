//! Where the terminal's parts go on a display.

use poly_gfx::{Cell, Font, MAX_ROWS, Rect, SPLEEN_8X16, SPLEEN_12X24, TextGrid};

/// Lines of scrollback the terminal keeps, unless the memory budget says
/// fewer.
pub const SCROLLBACK_LINES: usize = 1000;

/// The most memory the grid's cells (screen and scrollback) may take: well
/// below the 4 MiB one kernel heap allocation can have.
pub const CELL_BUDGET_BYTES: usize = 3 << 20;

/// The most columns a grid can have (`poly_gfx`'s limit).
const MAX_COLS: usize = 4096;

/// The layout of the terminal on a `width` × `height` display: a status bar
/// one text row high across the top, the text grid below it from the left
/// edge. Pixels right of and below the last whole cell stay background.
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    pub font: &'static Font,
    /// The status bar: the top row, the display's full width.
    pub status: Rect,
    /// The grid's cells, in pixels: `cols` × `rows` cells of `font`.
    pub grid: Rect,
    pub cols: usize,
    pub rows: usize,
    /// Lines of scrollback the grid gets.
    pub scrollback: usize,
}

impl PartialEq for Screen {
    fn eq(&self, other: &Screen) -> bool {
        let fields = |s: &Screen| (s.width, s.height, s.status, s.grid, s.cols, s.rows, s.scrollback);
        core::ptr::eq(self.font, other.font) && fields(self) == fields(other)
    }
}

impl Eq for Screen {}

impl Screen {
    /// The layout for a display of `width` × `height` pixels: the 8x16
    /// font, or 12x24 from 1920x1200 up (so text stays readable on large
    /// screens). `None` if not even a status bar and one row of one cell
    /// fit.
    pub fn new(width: u32, height: u32) -> Option<Screen> {
        let font: &'static Font = if width >= 1920 && height >= 1200 { &SPLEEN_12X24 } else { &SPLEEN_8X16 };
        let (cw, ch) = (font.width(), font.height());
        let cols = ((width / cw) as usize).min(MAX_COLS);
        let rows = ((height / ch) as usize).saturating_sub(1).min(MAX_ROWS);
        if cols == 0 || rows == 0 {
            return None;
        }
        let status = Rect::new(0, 0, width, ch);
        let grid = Rect::new(0, ch as i32, cols as u32 * cw, rows as u32 * ch);
        let lines = CELL_BUDGET_BYTES / (cols * core::mem::size_of::<Cell>());
        let scrollback = lines.saturating_sub(rows).min(SCROLLBACK_LINES);
        Some(Screen { width, height, font, status, grid, cols, rows, scrollback })
    }

    /// Cells the grid needs (screen and scrollback), for
    /// [`TextGrid::new`].
    pub const fn cells(&self) -> usize {
        TextGrid::storage_len(self.cols, self.rows, self.scrollback)
    }

    /// The whole display.
    pub const fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    /// Where the cell at `col`, `row` of the grid is on the display.
    pub fn cell_rect(&self, col: usize, row: usize) -> Rect {
        let (cw, ch) = (self.font.width(), self.font.height());
        Rect::new(self.grid.x + (col as u32 * cw) as i32, self.grid.y + (row as u32 * ch) as i32, cw, ch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_gets_the_small_font_and_a_status_row() {
        let s = Screen::new(1280, 800).unwrap();
        assert_eq!(s.font.height(), 16);
        assert_eq!((s.cols, s.rows), (160, 49));
        assert_eq!(s.status, Rect::new(0, 0, 1280, 16));
        assert_eq!(s.grid, Rect::new(0, 16, 1280, 784));
        assert_eq!(s.cell_rect(2, 1), Rect::new(16, 32, 8, 16));
        assert!(s.scrollback > 0 && s.scrollback <= SCROLLBACK_LINES);
        assert!(s.cells() * core::mem::size_of::<Cell>() <= CELL_BUDGET_BYTES);
    }

    #[test]
    fn odd_sizes_leave_a_margin() {
        let s = Screen::new(1023, 770).unwrap();
        assert_eq!((s.cols, s.rows), (127, 47));
        assert_eq!(s.grid.width, 1016);
        assert_eq!(s.grid.y as u32 + s.grid.height, 768);
    }

    #[test]
    fn large_screens_get_the_large_font_within_the_budget() {
        let s = Screen::new(3840, 2160).unwrap();
        assert_eq!(s.font.height(), 24);
        assert_eq!((s.cols, s.rows), (320, 89));
        assert!(s.cells() * core::mem::size_of::<Cell>() <= CELL_BUDGET_BYTES);
        assert!(s.scrollback >= 100, "{}", s.scrollback);
    }

    #[test]
    fn tiny_or_huge_displays() {
        assert_eq!(Screen::new(0, 0), None);
        assert_eq!(Screen::new(7, 100), None);
        assert_eq!(Screen::new(100, 31), None);
        let s = Screen::new(8, 32).unwrap();
        assert_eq!((s.cols, s.rows), (1, 1));
        let s = Screen::new(u32::MAX, u32::MAX).unwrap();
        assert_eq!((s.cols, s.rows), (MAX_COLS, MAX_ROWS));
        assert_eq!(s.scrollback, 0);
        let mut cells = std::vec![Cell::BLANK; s.cells()];
        assert!(TextGrid::new(&mut cells, s.cols, s.rows).is_ok());
    }
}
