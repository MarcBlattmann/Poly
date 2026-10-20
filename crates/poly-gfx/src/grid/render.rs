//! Drawing a [`TextGrid`] onto a canvas, and the colours it uses.

use super::{Attrs, CursorShape, Paint, Span, Style, TextGrid};
use crate::canvas::{Canvas, Decorations};
use crate::color::Color;
use crate::font::Font;
use crate::geom::Rect;

/// The colours a grid is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// The 8 ANSI colours (black, red, green, yellow, blue, magenta, cyan,
    /// white), then their bright versions.
    pub ansi: [Color; 16],
    pub foreground: Color,
    pub background: Color,
    pub cursor: Color,
}

impl Palette {
    /// Poly's dark default.
    pub const DEFAULT: Palette = Palette {
        ansi: [
            Color::from_rgb24(0x20232a),
            Color::from_rgb24(0xe0585b),
            Color::from_rgb24(0x7fbf6a),
            Color::from_rgb24(0xe6b450),
            Color::from_rgb24(0x5c9cf5),
            Color::from_rgb24(0xc27fd9),
            Color::from_rgb24(0x4fbfbf),
            Color::from_rgb24(0xc8ccd4),
            Color::from_rgb24(0x5b616e),
            Color::from_rgb24(0xff7a7d),
            Color::from_rgb24(0x9fdc86),
            Color::from_rgb24(0xffd173),
            Color::from_rgb24(0x82b6ff),
            Color::from_rgb24(0xdda2f2),
            Color::from_rgb24(0x76dede),
            Color::from_rgb24(0xf2f4f8),
        ],
        foreground: Color::from_rgb24(0xd4d7dd),
        background: Color::from_rgb24(0x15171c),
        cursor: Color::from_rgb24(0xe8c66a),
    };

    /// Indexed colour `n` of 256: the 16 ANSI colours, then a 6×6×6 cube
    /// with channel levels 0, 95, 135, 175, 215, 255, then 24 greys from 8
    /// to 238 in steps of 10.
    pub const fn indexed(&self, n: u8) -> Color {
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        match n {
            0..=15 => self.ansi[n as usize],
            16..=231 => {
                let i = n - 16;
                Color::rgb(LEVELS[(i / 36) as usize], LEVELS[(i / 6 % 6) as usize], LEVELS[(i % 6) as usize])
            }
            _ => {
                let v = 8 + 10 * (n - 232);
                Color::rgb(v, v, v)
            }
        }
    }

    /// `paint` as a colour; `default` for [`Paint::Default`].
    pub const fn resolve(&self, paint: Paint, default: Color) -> Color {
        match paint {
            Paint::Default => default,
            Paint::Indexed(n) => self.indexed(n),
            Paint::Rgb(r, g, b) => Color::rgb(r, g, b),
        }
    }

    /// Foreground and background of a cell in `style`, attributes applied.
    pub const fn colors(&self, style: Style) -> (Color, Color) {
        let mut fg = self.resolve(style.fg, self.foreground);
        let mut bg = self.resolve(style.bg, self.background);
        if style.attrs.contains(Attrs::INVERSE) {
            (fg, bg) = (bg, fg);
        }
        if style.attrs.contains(Attrs::FAINT) {
            fg = fg.mix(bg);
        }
        if style.attrs.contains(Attrs::HIDDEN) {
            fg = bg;
        }
        (fg, bg)
    }
}

impl Default for Palette {
    fn default() -> Palette {
        Palette::DEFAULT
    }
}

impl TextGrid<'_> {
    /// Draw what changed since the last call onto `canvas`, the grid's top
    /// left corner at (`x`, `y`), each cell a `font` cell. Returns the
    /// rectangle of `canvas` that was drawn on, `None` if nothing was.
    ///
    /// The canvas must still show what the previous call drew, at the same
    /// place in the same font and palette; otherwise call
    /// [`invalidate`](Self::invalidate) first. Pixels outside the grid's
    /// `cols × rows` cells are never touched.
    pub fn render(&mut self, canvas: &mut Canvas<'_>, x: i32, y: i32, font: &Font, palette: &Palette) -> Option<Rect> {
        let (cw, ch) = (font.width(), font.height());
        let area = Rect::new(x, y, cw.saturating_mul(self.cols as u32), ch.saturating_mul(self.rows as u32));
        let cursor = self.cursor_cell();
        if self.full {
            self.full = false;
            self.scrolled = 0;
            self.redraw_cursor = false;
            self.dirty[..self.rows].fill(Span::EMPTY);
            for row in 0..self.rows {
                self.draw_cells(canvas, x, y, font, palette, row, 0..self.cols, cursor);
            }
            self.drawn_cursor = cursor;
            return (!area.is_empty()).then_some(area);
        }
        let mut changed = Rect::default();
        if self.scrolled > 0 {
            // Move what is still on screen up; the lines that came in at the
            // bottom are marked dirty already, and the cursor moved with
            // the pixels, so erase it where it went.
            let shift = (self.scrolled as u32).saturating_mul(ch);
            let rest = Rect::new(x, y.saturating_add_unsigned(shift), area.width, area.height - shift);
            canvas.copy_within(rest, x, y);
            changed = area;
            if let Some((c, r)) = self.drawn_cursor.take()
                && r >= self.scrolled
            {
                self.mark_visible(r - self.scrolled, c, c + 1);
            }
            self.scrolled = 0;
        }
        if self.drawn_cursor != cursor || self.redraw_cursor {
            for (c, r) in [self.drawn_cursor, cursor].into_iter().flatten() {
                self.mark_visible(r, c, c + 1);
            }
            self.redraw_cursor = false;
        }
        for row in 0..self.rows {
            let span = core::mem::replace(&mut self.dirty[row], Span::EMPTY);
            if span.is_empty() {
                continue;
            }
            let (lo, hi) = (span.lo as usize, span.hi as usize);
            self.draw_cells(canvas, x, y, font, palette, row, lo..hi, cursor);
            let cells = Rect::new(
                x.saturating_add_unsigned(lo as u32 * cw),
                y.saturating_add_unsigned(row as u32 * ch),
                (hi - lo) as u32 * cw,
                ch,
            );
            changed = changed.union(&cells);
        }
        self.drawn_cursor = cursor;
        (!changed.is_empty()).then_some(changed)
    }

    /// Draw columns `cols` of visible row `row`.
    #[allow(clippy::too_many_arguments, reason = "render's arguments plus the cells")]
    fn draw_cells(
        &self,
        canvas: &mut Canvas<'_>,
        x: i32,
        y: i32,
        font: &Font,
        palette: &Palette,
        row: usize,
        cols: core::ops::Range<usize>,
        cursor: Option<(usize, usize)>,
    ) {
        let (cw, ch) = (font.width(), font.height());
        let line = self.line(self.visible_ring(row));
        let py = y.saturating_add_unsigned(row as u32 * ch);
        // An underline sits just below the baseline, a strike-through
        // around the middle of a lowercase letter.
        let underline = font.ascent().min(ch.saturating_sub(1));
        let strike = font.ascent() * 2 / 3;
        for col in cols {
            let cell = line[col];
            let px = x.saturating_add_unsigned(col as u32 * cw);
            let (mut fg, mut bg) = palette.colors(cell.style);
            let at_cursor = cursor == Some((col, row));
            if at_cursor && self.cursor_shape == CursorShape::Block {
                (fg, bg) = (bg, palette.cursor);
            }
            let attrs = cell.style.attrs;
            let deco = Decorations {
                bold: attrs.contains(Attrs::BOLD),
                underline: attrs.contains(Attrs::UNDERLINE).then_some(underline),
                strike: attrs.contains(Attrs::STRIKE).then_some(strike),
            };
            canvas.draw_decorated(px, py, font.glyph(cell.ch), fg, Some(bg), 1, deco);
            if at_cursor {
                match self.cursor_shape {
                    CursorShape::Block => {}
                    CursorShape::Underline => canvas.fill_rect(
                        Rect::new(px, py.saturating_add_unsigned(ch.saturating_sub(2)), cw, 2),
                        palette.cursor,
                    ),
                    CursorShape::Bar => canvas.fill_rect(Rect::new(px, py, 2, ch), palette.cursor),
                }
            }
        }
    }
}
