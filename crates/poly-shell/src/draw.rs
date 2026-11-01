//! Drawing into a compositor framebuffer, clipped: filled and outlined
//! rectangles with antialiased round corners, translucent white over what
//! is below, and text with `poly-font` (or the Spleen bitmap font when no
//! outline font was found). Coordinates here are physical pixels.

use core::cell::RefCell;
use poly_font::{Font, GlyphCache, GlyphKey, LayoutOptions, RenderMode, layout, quantize};
use poly_wayland::compose::{Framebuffer, Rect, TextRenderer};

/// `src` (opaque `0xRRGGBB`) over the opaque `dst` with coverage `a`
/// (0-255).
pub fn mix(dst: u32, src: u32, a: u32) -> u32 {
    if a >= 255 {
        return src | 0xff00_0000;
    }
    if a == 0 {
        return dst;
    }
    let ch = |s: u32| {
        let (sv, dv) = ((src >> s) & 0xff, (dst >> s) & 0xff);
        ((sv * a + dv * (255 - a) + 127) / 255) << s
    };
    0xff00_0000 | ch(16) | ch(8) | ch(0)
}

/// Fill `r` (inside `clip`) with `rgb` at coverage `alpha`.
pub fn fill(fb: &mut dyn Framebuffer, clip: Rect, r: Rect, rgb: u32, alpha: u8) {
    let r = r.intersect(&clip);
    for y in r.y..r.bottom() {
        let row = &mut fb.row_mut(y as u32)[r.x as usize..r.right() as usize];
        if alpha == 255 {
            row.fill(rgb | 0xff00_0000);
        } else {
            for p in row {
                *p = mix(*p, rgb, u32::from(alpha));
            }
        }
    }
}

/// How much of pixel (`x`, `y`) a rectangle with corners of radius `rad`
/// covers, 0 to 256 (one sample per pixel centre, with a one-pixel
/// smooth edge on the corners).
fn rounded_coverage(r: Rect, rad: i32, x: i32, y: i32) -> u32 {
    if !r.contains(x, y) {
        return 0;
    }
    if rad <= 0 {
        return 256;
    }
    // The corner circle's centre nearest the pixel, if the pixel is in a
    // corner square.
    let cx = if x < r.x + rad {
        r.x + rad
    } else if x >= r.right() - rad {
        r.right() - rad
    } else {
        return 256;
    };
    let cy = if y < r.y + rad {
        r.y + rad
    } else if y >= r.bottom() - rad {
        r.bottom() - rad
    } else {
        return 256;
    };
    // Distance from the circle's centre to the pixel's centre, in 1/16
    // pixels, compared with the radius.
    let dx = (2 * (x - cx) + 1) * 8;
    let dy = (2 * (y - cy) + 1) * 8;
    let d2 = (dx * dx + dy * dy) as u32;
    let r16 = (rad * 16) as u32;
    let (inner, outer) = ((r16.saturating_sub(8)).pow(2), (r16 + 8).pow(2));
    if d2 <= inner {
        256
    } else if d2 >= outer {
        0
    } else {
        // Linear across the 1-pixel edge (in squared distance: close enough).
        (u64::from(outer - d2) * 256 / u64::from(outer - inner)) as u32
    }
}

/// Fill a rectangle with round corners.
pub fn rounded_rect(fb: &mut dyn Framebuffer, clip: Rect, r: Rect, rad: i32, rgb: u32, alpha: u8) {
    let area = r.intersect(&clip);
    let rad = rad.min(r.width / 2).min(r.height / 2);
    for y in area.y..area.bottom() {
        let corner_row = y < r.y + rad || y >= r.bottom() - rad;
        let row = fb.row_mut(y as u32);
        for x in area.x..area.right() {
            let cov = if corner_row { rounded_coverage(r, rad, x, y) } else { 256 };
            if cov > 0 {
                let a = u32::from(alpha) * cov / 256;
                row[x as usize] = mix(row[x as usize], rgb, a);
            }
        }
    }
}

/// A one-pixel outline along the inside of a rectangle with round corners.
pub fn rounded_outline(fb: &mut dyn Framebuffer, clip: Rect, r: Rect, rad: i32, rgb: u32, alpha: u8) {
    let area = r.intersect(&clip);
    let inner = Rect::new(r.x + 1, r.y + 1, r.width - 2, r.height - 2);
    let rad = rad.min(r.width / 2).min(r.height / 2);
    for y in area.y..area.bottom() {
        let row = fb.row_mut(y as u32);
        for x in area.x..area.right() {
            let cov = rounded_coverage(r, rad, x, y).saturating_sub(rounded_coverage(inner, rad - 1, x, y));
            if cov > 0 {
                row[x as usize] = mix(row[x as usize], rgb, u32::from(alpha) * cov / 256);
            }
        }
    }
}

/// A circle of radius `rad` around (`cx`, `cy`) (pixel centres), filled
/// (`ring` `None`) or as a ring that many pixels wide.
#[allow(clippy::too_many_arguments)]
pub fn circle(
    fb: &mut dyn Framebuffer,
    clip: Rect,
    cx: i32,
    cy: i32,
    rad: i32,
    ring: Option<i32>,
    rgb: u32,
    alpha: u8,
) {
    let bounds = Rect::new(cx - rad - 1, cy - rad - 1, 2 * rad + 3, 2 * rad + 3).intersect(&clip);
    let r16 = rad * 16;
    for y in bounds.y..bounds.bottom() {
        let row = fb.row_mut(y as u32);
        for x in bounds.x..bounds.right() {
            let (dx, dy) = ((x - cx) * 16, (y - cy) * 16);
            let d = isqrt((dx * dx + dy * dy) as u32) as i32;
            // Coverage of the disc (or ring) at distance d (1/16 px).
            let edge = |r: i32| ((r + 8 - d).clamp(0, 16) * 16) as u32;
            let cov = match ring {
                None => edge(r16),
                Some(w) => edge(r16).saturating_sub(edge(r16 - w * 16)),
            };
            if cov > 0 {
                row[x as usize] = mix(row[x as usize], rgb, u32::from(alpha) * cov.min(256) / 256);
            }
        }
    }
}

fn isqrt(v: u32) -> u32 {
    if v < 2 {
        return v;
    }
    let mut x = v;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + v / x) / 2;
    }
    x
}

/// Text: an outline font through a glyph cache, or the 8×16 Spleen bitmap
/// font.
pub struct Text {
    font: Option<Font<'static>>,
    cache: RefCell<GlyphCache>,
}

impl core::fmt::Debug for Text {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Text").field("outline", &self.font.is_some()).finish()
    }
}

/// Glyph cache budget: plenty for a few sizes of Latin text.
const CACHE_BYTES: usize = 512 * 1024;

impl Text {
    /// Text in `font` (DejaVu Sans, normally), or in the bitmap font.
    pub fn new(font: Option<Font<'static>>) -> Text {
        Text { font, cache: RefCell::new(GlyphCache::new(CACHE_BYTES)) }
    }

    pub fn has_outline_font(&self) -> bool {
        self.font.is_some()
    }

    /// How wide `s` is at `size` pixels per em.
    pub fn width(&self, s: &str, size: f32) -> f32 {
        match &self.font {
            Some(f) => layout(f, s, &LayoutOptions::new(size)).width,
            None => (s.chars().count() * 8) as f32,
        }
    }

    /// The ascent of capital letters, roughly: where to put a baseline to
    /// centre a line in a box.
    pub fn cap_height(&self, size: f32) -> f32 {
        if self.font.is_some() { size * 0.73 } else { 12.0 }
    }

    /// Draw `s` with its baseline at `baseline` from `x` on, in `rgb` at
    /// `alpha`, cut at `max_x`. Returns the pen position after it.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        fb: &mut dyn Framebuffer,
        clip: Rect,
        x: f32,
        baseline: i32,
        max_x: i32,
        s: &str,
        size: f32,
        rgb: u32,
        alpha: u8,
    ) -> f32 {
        let clip = clip.intersect(&Rect::new(clip.x, clip.y, max_x - clip.x, clip.height));
        let Some(font) = &self.font else {
            return self.draw_bitmap(fb, clip, x, baseline, s, rgb, alpha);
        };
        let l = layout(font, s, &LayoutOptions::new(size));
        let mut cache = self.cache.borrow_mut();
        for g in &l.glyphs {
            let (whole, sub) = quantize(x + g.x);
            let bm = cache.get(font, GlyphKey::new(0, size, g.glyph, sub, RenderMode::Gray));
            let (left, top) = (whole + bm.left, baseline + bm.top);
            let area = Rect::new(left, top, bm.width as i32, bm.height as i32).intersect(&clip);
            for py in area.y..area.bottom() {
                let row = fb.row_mut(py as u32);
                for px in area.x..area.right() {
                    let cov = u32::from(bm.coverage((px - left) as u32, (py - top) as u32));
                    if cov > 0 {
                        row[px as usize] = mix(row[px as usize], rgb, cov * u32::from(alpha) / 255);
                    }
                }
            }
        }
        x + l.width
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_bitmap(
        &self,
        fb: &mut dyn Framebuffer,
        clip: Rect,
        x: f32,
        baseline: i32,
        s: &str,
        rgb: u32,
        a: u8,
    ) -> f32 {
        let font = &poly_gfx::SPLEEN_8X16;
        let mut pen = x as i32;
        let top = baseline - 12;
        for c in s.chars() {
            let glyph = font.glyph(c);
            let cell = Rect::new(pen, top, 8, 16).intersect(&clip);
            for py in cell.y..cell.bottom() {
                let row = fb.row_mut(py as u32);
                for px in cell.x..cell.right() {
                    if glyph.is_set((px - pen) as u32, (py - top) as u32) {
                        row[px as usize] = mix(row[px as usize], rgb, u32::from(a));
                    }
                }
            }
            pen += 8;
        }
        pen as f32
    }
}

/// Window titles in an outline font, for the compositor's neutral frame
/// (its [`TextRenderer`] cells): each character rendered once at
/// `size` pixels and placed in a cell as wide as its advance (rounded;
/// no kerning), on a baseline shared by all.
pub struct TitleFont {
    font: Font<'static>,
    size: f32,
    line_height: u32,
    baseline: i32,
    glyphs: RefCell<alloc::collections::BTreeMap<char, (u32, poly_font::Bitmap)>>,
}

impl core::fmt::Debug for TitleFont {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TitleFont").field("size", &self.size).finish()
    }
}

impl TitleFont {
    /// `font` at `size` pixels per em, in lines `line_height` high.
    pub fn new(font: Font<'static>, size: f32, line_height: u32) -> TitleFont {
        let v = font.vmetrics();
        let scale = size / f32::from(v.units_per_em.max(1));
        let ascent = f32::from(v.ascender) * scale;
        let descent = -f32::from(v.descender) * scale;
        // Centre ascent + descent in the line.
        let baseline = ((line_height as f32 - (ascent + descent)) / 2.0 + ascent + 0.5) as i32;
        TitleFont { font, size, line_height, baseline, glyphs: RefCell::new(alloc::collections::BTreeMap::new()) }
    }

    fn with_glyph<R>(&self, c: char, f: impl FnOnce(u32, &poly_font::Bitmap) -> R) -> R {
        let mut glyphs = self.glyphs.borrow_mut();
        let entry = glyphs.entry(c).or_insert_with(|| {
            let g = self.font.glyph_index(c).unwrap_or(poly_font::GlyphId(0));
            let upem = f32::from(self.font.units_per_em().max(1));
            let advance = (f32::from(self.font.advance(g)) * self.size / upem + 0.5) as u32;
            (advance, poly_font::render_glyph(&self.font, g, self.size, 0.0, RenderMode::Gray))
        });
        f(entry.0, &entry.1)
    }
}

impl TextRenderer for TitleFont {
    fn line_height(&self) -> u32 {
        self.line_height
    }

    fn advance(&self, c: char) -> u32 {
        self.with_glyph(c, |adv, _| adv)
    }

    fn coverage(&self, c: char, x: u32, y: u32) -> u8 {
        self.with_glyph(c, |_, bm| {
            let bx = x as i32 - bm.left;
            let by = y as i32 - (self.baseline + bm.top);
            if bx < 0 || by < 0 { 0 } else { bm.coverage(bx as u32, by as u32) }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_wayland::compose::MemoryFramebuffer;

    #[test]
    fn mixing() {
        assert_eq!(mix(0xff00_0000, 0xff_ffff, 255), 0xffff_ffff);
        assert_eq!(mix(0xff10_2030, 0xff_ffff, 0), 0xff10_2030);
        // White at 10 % over rgb(32, 32, 32): 32 + (255 - 32) * 26 / 255.
        assert_eq!(mix(0xff20_2020, 0xff_ffff, 26) & 0xff, 55);
    }

    #[test]
    fn round_corners_are_cut_and_smooth() {
        let mut fb = MemoryFramebuffer::new(40, 40);
        let all = Rect::new(0, 0, 40, 40);
        rounded_rect(&mut fb, all, Rect::new(0, 0, 40, 40), 10, 0xff_ffff, 255);
        assert_eq!(fb.pixel(0, 0), Some(0), "the corner is outside");
        assert_eq!(fb.pixel(20, 0), Some(0xff_ffff), "an edge's middle is inside");
        assert_eq!(fb.pixel(20, 20), Some(0xff_ffff));
        // Somewhere on the corner's arc a pixel is partly covered.
        let partial = (0..100).any(|i| fb.pixel(i % 10, i / 10).is_some_and(|p| p != 0 && p != 0xff_ffff));
        assert!(partial);
        // Symmetric.
        assert_eq!(fb.pixel(1, 3), fb.pixel(38, 36));
        // Clipped: nothing outside the clip.
        let mut fb = MemoryFramebuffer::new(40, 40);
        rounded_rect(&mut fb, Rect::new(0, 0, 20, 40), Rect::new(0, 0, 40, 40), 4, 0xff_ffff, 255);
        assert_eq!(fb.pixel(30, 20), Some(0));
    }

    #[test]
    fn outlines_and_circles() {
        let mut fb = MemoryFramebuffer::new(40, 40);
        let all = Rect::new(0, 0, 40, 40);
        rounded_outline(&mut fb, all, Rect::new(0, 0, 40, 40), 6, 0xff_ffff, 255);
        assert_eq!(fb.pixel(20, 0), Some(0xff_ffff));
        assert_eq!(fb.pixel(20, 1), Some(0), "one pixel thick");
        assert_eq!(fb.pixel(20, 20), Some(0));
        let mut fb = MemoryFramebuffer::new(40, 40);
        circle(&mut fb, all, 20, 20, 10, None, 0xff_ffff, 255);
        assert_eq!(fb.pixel(20, 20), Some(0xff_ffff));
        assert_eq!(fb.pixel(20, 35), Some(0));
        let mut fb = MemoryFramebuffer::new(40, 40);
        circle(&mut fb, all, 20, 20, 10, Some(2), 0xff_ffff, 255);
        assert_eq!(fb.pixel(20, 20), Some(0), "a ring is empty inside");
        assert_eq!(fb.pixel(20, 11), Some(0xff_ffff));
    }

    #[test]
    fn bitmap_text_without_a_font() {
        let t = Text::new(None);
        assert!(!t.has_outline_font());
        assert_eq!(t.width("abc", 12.0), 24.0);
        let mut fb = MemoryFramebuffer::new(40, 20);
        let end = t.draw(&mut fb, Rect::new(0, 0, 40, 20), 0.0, 14, 40, "H", 12.0, 0xff_ffff, 255);
        assert_eq!(end, 8.0);
        assert!(fb.pixels().iter().any(|&p| p & 0xff_ffff == 0xff_ffff), "something was drawn");
    }
}
