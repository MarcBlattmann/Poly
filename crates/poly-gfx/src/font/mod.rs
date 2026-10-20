//! Fixed-cell bitmap fonts.
//!
//! A [`Font`] is one bitmap per code point, all the size of the cell: rows
//! from top to bottom, each padded to whole bytes, the leftmost pixel in the
//! most significant bit. Code points are stored as ranges of consecutive
//! glyphs, found by binary search. Characters the font lacks draw as its
//! fallback glyph, the outline of a box.
//!
//! The built-in fonts are generated from BDF files in `third_party/` by
//! `cargo xtask font` (see `tools/xtask/src/font.rs`), which writes the
//! bitmaps as hex strings that [`hex`] turns into bytes at compile time.

mod spleen_12x24;
mod spleen_8x16;

pub use spleen_8x16::SPLEEN_8X16;
pub use spleen_12x24::SPLEEN_12X24;

/// Every built-in font, smallest first.
pub static FONTS: [&Font; 2] = [&SPLEEN_8X16, &SPLEEN_12X24];

/// Code points `first..=last` have glyphs `index..`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GlyphRange {
    pub first: u32,
    pub last: u32,
    pub index: usize,
}

/// A bitmap font with one cell size for every glyph.
#[derive(Debug)]
pub struct Font {
    pub(crate) name: &'static str,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) ascent: u32,
    pub(crate) ranges: &'static [GlyphRange],
    pub(crate) bitmaps: &'static [u8],
    pub(crate) fallback: &'static [u8],
}

impl Font {
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Cell width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Cell height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Rows from the top of the cell down to the baseline: the row just
    /// above the baseline is `ascent - 1`.
    pub const fn ascent(&self) -> u32 {
        self.ascent
    }

    const fn row_bytes(&self) -> usize {
        self.width.div_ceil(8) as usize
    }

    const fn glyph_bytes(&self) -> usize {
        self.row_bytes() * self.height as usize
    }

    /// Pixels `s` takes on one line: one cell per character.
    pub fn text_width(&self, s: &str) -> u32 {
        u32::try_from(s.chars().count()).unwrap_or(u32::MAX).saturating_mul(self.width)
    }

    /// How many glyphs the font has (not counting the fallback).
    pub fn glyph_count(&self) -> usize {
        self.bitmaps.len() / self.glyph_bytes()
    }

    /// The glyph for `c`, if the font has one.
    pub fn lookup(&self, c: char) -> Option<Glyph<'_>> {
        let cp = c as u32;
        let i = self.ranges.partition_point(|r| r.first <= cp).checked_sub(1)?;
        let r = self.ranges[i];
        if cp > r.last {
            return None;
        }
        let start = (r.index + (cp - r.first) as usize) * self.glyph_bytes();
        let bits = self.bitmaps.get(start..start + self.glyph_bytes())?;
        Some(self.make(bits))
    }

    /// The glyph for `c`, or the fallback glyph.
    pub fn glyph(&self, c: char) -> Glyph<'_> {
        self.lookup(c).unwrap_or_else(|| self.fallback())
    }

    /// What characters without a glyph look like.
    pub fn fallback(&self) -> Glyph<'_> {
        self.make(self.fallback)
    }

    const fn make<'a>(&self, bits: &'a [u8]) -> Glyph<'a> {
        Glyph { bits, width: self.width, height: self.height, row_bytes: self.row_bytes() }
    }
}

/// One glyph's bitmap: [`Font::width`] × [`Font::height`] pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph<'a> {
    bits: &'a [u8],
    width: u32,
    height: u32,
    row_bytes: usize,
}

impl Glyph<'_> {
    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Row `y` with pixel `x` in bit `31 - x` (so the leftmost pixel is the
    /// top bit); 0 below the last row.
    pub fn row(&self, y: u32) -> u32 {
        let start = y as usize * self.row_bytes;
        let Some(bytes) = self.bits.get(start..start + self.row_bytes) else { return 0 };
        bytes.iter().enumerate().fold(0, |acc, (i, &b)| acc | (b as u32) << (24 - 8 * i))
    }

    /// Whether pixel (`x`, `y`) is set; false outside the glyph.
    pub fn is_set(&self, x: u32, y: u32) -> bool {
        x < self.width && self.row(y) & (0x8000_0000 >> x) != 0
    }
}

/// The bytes a string of hex digit pairs stands for, at compile time.
/// Fails the build if the string is not exactly `2 * N` lowercase hex digits.
pub(crate) const fn hex<const N: usize>(s: &str) -> [u8; N] {
    const fn digit(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("not a lowercase hex digit"),
        }
    }
    let s = s.as_bytes();
    assert!(s.len() == 2 * N, "hex string of the wrong length");
    let mut out = [0u8; N];
    let mut i = 0;
    while i < N {
        out[i] = digit(s[2 * i]) << 4 | digit(s[2 * i + 1]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests;
