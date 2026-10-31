//! Glyph bitmaps: an outline scaled to a pixel size, shifted by a
//! subpixel offset and rasterised, in grey or (hinting-free) LCD mode.

use crate::font::{Font, GlyphId};
use crate::math::{ceil, floor};
use crate::path::Point;
use crate::raster::Rasterizer;
use alloc::vec;
use alloc::vec::Vec;

/// How a glyph is rendered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RenderMode {
    /// One coverage byte per pixel.
    #[default]
    Gray,
    /// Three coverage bytes per pixel, for the red, green and blue
    /// subpixels of an LCD with horizontal RGB stripes: the outline is
    /// rasterised at three times the horizontal resolution and filtered
    /// with a 5-tap triangle filter (1 2 3 2 1)/9 against colour fringes.
    Lcd,
}

impl RenderMode {
    /// Bytes per pixel.
    pub fn channels(self) -> usize {
        match self {
            RenderMode::Gray => 1,
            RenderMode::Lcd => 3,
        }
    }
}

/// A rendered glyph.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    /// Where the bitmap's left column is, in pixels right of the pen
    /// position (the glyph origin, before the subpixel offset).
    pub left: i32,
    /// Where the bitmap's top row is, in pixels below the baseline
    /// (negative: above it).
    pub top: i32,
    pub mode: RenderMode,
    /// Rows top to bottom, `width * mode.channels()` bytes each.
    pub data: Vec<u8>,
}

impl Bitmap {
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The coverage at a pixel (the channel average in LCD mode).
    pub fn coverage(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        let ch = self.mode.channels();
        let at = (y as usize * self.width as usize + x as usize) * ch;
        let sum: u32 = self.data[at..at + ch].iter().map(|&v| u32::from(v)).sum();
        (sum / ch as u32) as u8
    }
}

/// The largest pixel size rendered, and the largest bitmap side, so that
/// a bad request cannot allocate without bound.
pub const MAX_SIZE_PX: f32 = 2048.0;
const MAX_SIDE: f32 = 4096.0;

/// Render `glyph` at `size_px` pixels per em, its origin shifted right by
/// `dx` pixels (a subpixel offset, `0.0 <= dx < 1.0`, for positioning
/// between pixels). A glyph without an outline gives an empty bitmap.
pub fn render_glyph(font: &Font<'_>, glyph: GlyphId, size_px: f32, dx: f32, mode: RenderMode) -> Bitmap {
    let empty = Bitmap { mode, ..Bitmap::default() };
    if !(size_px > 0.0 && size_px <= MAX_SIZE_PX) || !dx.is_finite() {
        return empty;
    }
    let Some(path) = font.outline(glyph) else { return empty };
    let Some(b) = path.bounds() else { return empty };
    let scale = size_px / f32::from(font.units_per_em());
    let pad = if mode == RenderMode::Lcd { 1.0 } else { 0.0 };
    let left = floor(b.x_min * scale + dx) - pad;
    let right = ceil(b.x_max * scale + dx) + pad;
    let top = floor(-b.y_max * scale);
    let bottom = ceil(-b.y_min * scale);
    let (w, h) = (right - left, bottom - top);
    if !(w > 0.0 && h > 0.0 && w <= MAX_SIDE && h <= MAX_SIDE) {
        return empty;
    }
    let (w, h) = (w as usize, h as usize);
    let data = match mode {
        RenderMode::Gray => {
            let mut r = Rasterizer::new(w, h);
            r.path(&path, |p| Point::new(p.x * scale + dx - left, -p.y * scale - top));
            r.coverage()
        }
        RenderMode::Lcd => {
            let mut r = Rasterizer::new(w * 3, h);
            r.path(&path, |p| Point::new((p.x * scale + dx - left) * 3.0, -p.y * scale - top));
            lcd_filter(&r.coverage_f32(), w, h)
        }
    };
    Bitmap { width: w as u32, height: h as u32, left: left as i32, top: top as i32, mode, data }
}

/// The LCD filter: each subpixel is the weighted sum of its neighbours'
/// coverage, (1 2 3 2 1)/9, which spreads a stem over at least a whole
/// pixel so that it does not show as a coloured line.
fn lcd_filter(sub: &[f32], w: usize, h: usize) -> Vec<u8> {
    const WEIGHTS: [f32; 5] = [1.0 / 9.0, 2.0 / 9.0, 3.0 / 9.0, 2.0 / 9.0, 1.0 / 9.0];
    let sw = w * 3;
    let mut out = vec![0u8; sw * h];
    for y in 0..h {
        let row = &sub[y * sw..(y + 1) * sw];
        for x in 0..sw {
            let mut v = 0.0;
            for (k, wt) in WEIGHTS.iter().enumerate() {
                if let Some(&c) = (x + k).checked_sub(2).and_then(|i| row.get(i)) {
                    v += c * wt;
                }
            }
            out[y * sw + x] = (v.min(1.0) * 255.0 + 0.5) as u8;
        }
    }
    out
}

/// A coverage image to draw glyphs into: one byte per pixel (grey) or
/// three (LCD).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub mode: RenderMode,
    pub data: Vec<u8>,
}

impl Image {
    /// A blank image. Sides are capped at 16384 pixels.
    pub fn new(width: u32, height: u32, mode: RenderMode) -> Image {
        let (width, height) = (width.min(16384), height.min(16384));
        Image { width, height, mode, data: vec![0; width as usize * height as usize * mode.channels()] }
    }

    /// Add a glyph's coverage with its pen position at `(x, y)` (the
    /// baseline), saturating; clipped to the image. A grey glyph in an LCD
    /// image covers all three subpixels; an LCD glyph in a grey image
    /// adds its channel average.
    pub fn draw(&mut self, glyph: &Bitmap, x: i32, y: i32) {
        let ch = self.mode.channels();
        for gy in 0..glyph.height {
            let iy = y.saturating_add(glyph.top).saturating_add(gy as i32);
            if iy < 0 || iy >= self.height as i32 {
                continue;
            }
            for gx in 0..glyph.width {
                let ix = x.saturating_add(glyph.left).saturating_add(gx as i32);
                if ix < 0 || ix >= self.width as i32 {
                    continue;
                }
                let at = (iy as usize * self.width as usize + ix as usize) * ch;
                let src = (gy as usize * glyph.width as usize + gx as usize) * glyph.mode.channels();
                for c in 0..ch {
                    let v = match (glyph.mode, self.mode) {
                        (RenderMode::Lcd, RenderMode::Lcd) => glyph.data[src + c],
                        (RenderMode::Gray, _) => glyph.data[src],
                        (RenderMode::Lcd, RenderMode::Gray) => glyph.coverage(gx, gy),
                    };
                    self.data[at + c] = self.data[at + c].saturating_add(v);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfont;

    #[test]
    fn bitmaps_are_placed_around_the_origin() {
        let data = testfont::build();
        let font = Font::parse(&data).unwrap();
        // 'A' spans x 50..550, y 0..700 of 1000 units: at 20 px that is
        // x 1..11, y -14..0.
        let a = render_glyph(&font, GlyphId(1), 20.0, 0.0, RenderMode::Gray);
        assert_eq!((a.left, a.top, a.width, a.height), (1, -14, 10, 14));
        assert_eq!(a.data.len(), 140);
        // The bottom row's ends are inside the triangle's feet.
        assert!(a.coverage(0, 13) > 100);
        assert!(a.coverage(9, 13) > 100);
        assert_eq!(a.coverage(0, 0), 0);
        // A half-pixel shift moves the box.
        let s = render_glyph(&font, GlyphId(1), 20.0, 0.5, RenderMode::Gray);
        assert_eq!((s.left, s.width), (1, 11));
        assert_ne!(s.data, a.data);
        // LCD: a pixel of padding each side, three bytes per pixel.
        let l = render_glyph(&font, GlyphId(1), 20.0, 0.0, RenderMode::Lcd);
        assert_eq!((l.left, l.width, l.height), (0, 12, 14));
        assert_eq!(l.data.len(), 12 * 14 * 3);
        // Nothing to draw, or nonsense sizes: empty.
        assert!(render_glyph(&font, GlyphId(4), 20.0, 0.0, RenderMode::Gray).is_empty());
        assert!(render_glyph(&font, GlyphId(1), 0.0, 0.0, RenderMode::Gray).is_empty());
        assert!(render_glyph(&font, GlyphId(1), f32::NAN, 0.0, RenderMode::Gray).is_empty());
        assert!(render_glyph(&font, GlyphId(1), 1e9, 0.0, RenderMode::Gray).is_empty());
    }

    #[test]
    fn coverage_matches_the_outline_area() {
        let data = testfont::build();
        let font = Font::parse(&data).unwrap();
        // 'A' is a triangle (base 500, height 700) less a hole (base 200,
        // height 300): 145000 square units; at 100 px per 1000 units that
        // is 1450 pixels of coverage.
        let a = render_glyph(&font, GlyphId(1), 100.0, 0.0, RenderMode::Gray);
        let area: f32 = a.data.iter().map(|&v| f32::from(v) / 255.0).sum();
        assert!((area - 1450.0).abs() < 2.0, "{area}");
        // The LCD filter keeps the total (its padding holds the spread).
        let l = render_glyph(&font, GlyphId(1), 100.0, 0.0, RenderMode::Lcd);
        let area: f32 = l.data.iter().map(|&v| f32::from(v) / 255.0).sum::<f32>() / 3.0;
        assert!((area - 1450.0).abs() < 3.0, "{area}");
    }

    #[test]
    fn images_add_glyphs_and_clip() {
        let g = Bitmap { width: 2, height: 1, left: 0, top: -1, mode: RenderMode::Gray, data: alloc::vec![200, 100] };
        let mut img = Image::new(3, 2, RenderMode::Gray);
        img.draw(&g, 0, 1);
        img.draw(&g, 1, 1);
        assert_eq!(img.data, [200, 255, 100, 0, 0, 0]);
        img.draw(&g, -5, 1);
        img.draw(&g, 2, 10);
        assert_eq!(img.data, [200, 255, 100, 0, 0, 0]);
        let mut rgb = Image::new(1, 1, RenderMode::Lcd);
        rgb.draw(&g, 0, 1);
        assert_eq!(rgb.data, [200, 200, 200]);
    }
}
