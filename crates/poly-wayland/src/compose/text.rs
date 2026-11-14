//! Text for window titles, behind a small trait: the fixed-cell bitmap
//! fonts of `poly-gfx` now, a rasterising font store (`poly-font`) later.

use super::Scale;
use poly_gfx::Font;

/// Draws characters as coverage maps, one cell per character on a line.
pub trait TextRenderer: core::fmt::Debug {
    /// The height of a line, in logical pixels.
    fn line_height(&self) -> u32;
    /// How far `c` moves the pen, in logical pixels.
    fn advance(&self, c: char) -> u32;
    /// How much of the pixel (`x`, `y`) of `c`'s cell (`x` below
    /// [`TextRenderer::advance`], `y` below [`TextRenderer::line_height`])
    /// the glyph covers: 0 none, 255 all.
    fn coverage(&self, c: char, x: u32, y: u32) -> u8;

    /// [`TextRenderer::line_height`] in physical pixels at output scale `s`.
    fn line_height_at(&self, s: Scale) -> u32 {
        s.px(self.line_height() as i32) as u32
    }

    /// [`TextRenderer::advance`] in physical pixels at output scale `s`.
    fn advance_at(&self, c: char, s: Scale) -> u32 {
        s.px(self.advance(c) as i32) as u32
    }

    /// [`TextRenderer::coverage`] for the physical pixel (`x`, `y`) of a
    /// cell drawn at output scale `s`. The default stretches the logical
    /// picture (what a bitmap font can do); an outline font draws the glyphs
    /// at the scaled size instead.
    fn coverage_at(&self, c: char, x: u32, y: u32, s: Scale) -> u8 {
        self.coverage(c, s.to_logical(x as i32) as u32, s.to_logical(y as i32) as u32)
    }
}

/// A `poly-gfx` bitmap font: every pixel covered fully or not at all.
#[derive(Clone, Copy, Debug)]
pub struct BitmapFont(pub &'static Font);

impl TextRenderer for BitmapFont {
    fn line_height(&self) -> u32 {
        self.0.height()
    }

    fn advance(&self, _c: char) -> u32 {
        self.0.width()
    }

    fn coverage(&self, c: char, x: u32, y: u32) -> u8 {
        if self.0.glyph(c).is_set(x, y) { 255 } else { 0 }
    }
}
