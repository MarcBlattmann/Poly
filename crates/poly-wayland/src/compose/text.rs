//! Text for window titles, behind a small trait: the fixed-cell bitmap
//! fonts of `poly-gfx` now, a rasterising font store (`poly-font`) later.

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
