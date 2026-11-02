//! Drawing into a compositor framebuffer, clipped: filled and outlined
//! rectangles with anti-aliased round corners, translucent white over what
//! is below, and text in Alpine Shell's fonts with `poly-font` (or the
//! Spleen bitmap font when no outline font was found). Coordinates here
//! are physical pixels. The shapes are `poly-wayland`'s `paint`
//! primitives.

use alloc::collections::BTreeMap;
use core::cell::RefCell;
use poly_font::{Bitmap, Font, LayoutOptions, RenderMode, layout, quantize, render_glyph_scaled};
use poly_wayland::compose::paint::{self, Radii};
use poly_wayland::compose::{Framebuffer, Rect, TextRenderer};

pub use poly_wayland::compose::paint::mix;

/// Fill `r` (inside `clip`) with `rgb` at coverage `alpha`.
pub fn fill(fb: &mut dyn Framebuffer, clip: Rect, r: Rect, rgb: u32, alpha: u8) {
    let r = r.intersect(&clip).intersect(&Rect::new(0, 0, fb.width() as i32, fb.height() as i32));
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

/// Fill a rectangle with round corners (anti-aliased).
pub fn rounded_rect(fb: &mut dyn Framebuffer, clip: Rect, r: Rect, rad: i32, rgb: u32, alpha: u8) {
    paint::fill_rounded(fb, clip, r, Radii::all(rad as f32), rgb, alpha);
}

/// A one-pixel outline along the inside of a rectangle with round corners.
pub fn rounded_outline(fb: &mut dyn Framebuffer, clip: Rect, r: Rect, rad: i32, rgb: u32, alpha: u8) {
    paint::stroke_rounded(fb, clip, r, Radii::all(rad as f32), 1.0, rgb, alpha);
}

/// An outline `width` pixels wide along the inside of a rounded rectangle.
#[allow(clippy::too_many_arguments)]
pub fn rounded_border(fb: &mut dyn Framebuffer, clip: Rect, r: Rect, rad: i32, width: i32, rgb: u32, alpha: u8) {
    paint::stroke_rounded(fb, clip, r, Radii::all(rad as f32), width as f32, rgb, alpha);
}

/// A circle of radius `rad` around the centre of pixel (`cx`, `cy`),
/// filled (`ring` `None`) or as a ring that many pixels wide.
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
    let (x, y) = (cx as f32 + 0.5, cy as f32 + 0.5);
    paint::circle(fb, clip, x, y, rad as f32, ring.map(|w| w as f32), rgb, alpha);
}

/// Alpine Shell's typefaces (ADR 0126).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Face {
    /// Geist regular (400): body text.
    Sans,
    /// Geist medium (500): `font-medium`.
    Medium,
    /// Geist semibold (600): headings, eyebrows.
    Semibold,
    /// Averia Serif Libre (400): `font-display` page titles.
    Display,
    /// Geist Mono (400): key hints.
    Mono,
    /// Averia Serif Libre bold (700): `font-display font-semibold`, the
    /// settings window's name.
    DisplayBold,
}

impl Face {
    fn index(self) -> usize {
        self as usize
    }
}

/// How text is set: a face, a size in pixels per em and letter spacing in
/// em (Tailwind's `tracking-tight` is -0.025, eyebrows 0.05).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub face: Face,
    pub size: f32,
    pub tracking: f32,
}

impl Style {
    pub const fn new(face: Face, size: f32) -> Style {
        Style { face, size, tracking: 0.0 }
    }

    pub const fn tracking(self, em: f32) -> Style {
        Style { tracking: em, ..self }
    }

    /// The same at output scale `s`.
    pub fn scaled(self, s: i32) -> Style {
        Style { size: self.size * s as f32, ..self }
    }
}

/// The fonts the shell draws with; any may be missing (a missing face
/// falls back to a near one, then to Geist regular, then to the bitmap
/// font).
#[derive(Default)]
pub struct Fonts {
    pub sans: Option<Font<'static>>,
    pub medium: Option<Font<'static>>,
    pub semibold: Option<Font<'static>>,
    pub display: Option<Font<'static>>,
    pub mono: Option<Font<'static>>,
    pub display_bold: Option<Font<'static>>,
}

impl core::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Fonts")
            .field("sans", &self.sans.is_some())
            .field("medium", &self.medium.is_some())
            .field("semibold", &self.semibold.is_some())
            .field("display", &self.display.is_some())
            .field("mono", &self.mono.is_some())
            .field("display_bold", &self.display_bold.is_some())
            .finish()
    }
}

/// A rendered glyph's key: face, size and vertical size in 1/64 pixel,
/// glyph, subpixel step.
type HintedKey = (u8, u32, u32, u16, u8);

/// At most this many rendered glyphs are kept (then the cache starts
/// over): plenty for a few sizes of Latin text in five faces.
const GLYPHS_KEPT: usize = 4096;

/// The gamma applied to glyph coverage (see [`Text`]).
const GAMMA: f32 = 1.2;

/// Text: Alpine's fonts through a glyph cache, or the 8×16 Spleen bitmap
/// font when none was found.
///
/// Glyphs are placed at quarter pixels horizontally (`poly-font`'s
/// subpixel positioning) and hinted vertically the light way: the outline
/// is scaled so that the font's x-height lands on a whole pixel, which
/// keeps the baseline and the tops of lower-case letters crisp at 11 to 15
/// pixels without changing the advances. Coverage goes through a gamma
/// curve ([`GAMMA`]) so that light text on Alpine's dark surfaces keeps the
/// weight DirectWrite gives it in Alpine Shell.
pub struct Text {
    fonts: [Option<Font<'static>>; 6],
    /// Each face's x-height and cap height, in em.
    heights: [(f32, f32); 6],
    glyphs: RefCell<BTreeMap<HintedKey, Bitmap>>,
    gamma: [u8; 256],
}

impl core::fmt::Debug for Text {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Text").field("outline", &self.has_outline_font()).finish()
    }
}

/// The top of a glyph's outline in em, if it has one.
fn glyph_top(font: &Font<'_>, c: char) -> Option<f32> {
    let g = font.glyph_index(c)?;
    let b = font.outline(g)?.bounds()?;
    Some(b.y_max / f32::from(font.units_per_em().max(1)))
}

/// `x^(1/g)` by Newton's method on `r^n = x` for `n = 1/(1 - 1/g)`
/// rounded (6 for 1.2): `x^(1/g) = x / x^(1/n)`.
fn gamma_table(g: f32) -> [u8; 256] {
    let n = (1.0 / (1.0 - 1.0 / g) + 0.5) as i32;
    let mut t = [0u8; 256];
    for (i, v) in t.iter_mut().enumerate() {
        let x = i as f32 / 255.0;
        let y = if x <= 0.0 || n < 2 {
            x
        } else {
            let mut r = 1.0f32;
            for _ in 0..60 {
                let rn1 = (0..n - 1).fold(1.0f32, |a, _| a * r);
                r -= (rn1 * r - x) / (n as f32 * rn1).max(1e-12);
            }
            x / r
        };
        *v = (y * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    }
    t
}

impl Text {
    /// Text in `font` (Geist regular, normally) for every face, or in the
    /// bitmap font.
    pub fn new(font: Option<Font<'static>>) -> Text {
        Text::with_fonts(Fonts { sans: font, ..Fonts::default() })
    }

    /// Text in Alpine's faces.
    pub fn with_fonts(f: Fonts) -> Text {
        let fonts = [f.sans, f.medium, f.semibold, f.display, f.mono, f.display_bold];
        let mut heights = [(0.52, 0.72); 6];
        for (h, font) in heights.iter_mut().zip(&fonts) {
            if let Some(font) = font {
                *h = (glyph_top(font, 'x').unwrap_or(0.52), glyph_top(font, 'H').unwrap_or(0.72));
            }
        }
        Text { fonts, heights, glyphs: RefCell::new(BTreeMap::new()), gamma: gamma_table(GAMMA) }
    }

    pub fn has_outline_font(&self) -> bool {
        self.fonts.iter().any(Option::is_some)
    }

    /// The font for `face` and the face it really is.
    fn font(&self, face: Face) -> Option<(Face, &Font<'static>)> {
        let order: &[Face] = match face {
            Face::Sans => &[Face::Sans, Face::Medium],
            Face::Medium => &[Face::Medium, Face::Semibold, Face::Sans],
            Face::Semibold => &[Face::Semibold, Face::Medium, Face::Sans],
            Face::Display => &[Face::Display, Face::Sans],
            Face::Mono => &[Face::Mono, Face::Sans],
            Face::DisplayBold => &[Face::DisplayBold, Face::Display, Face::Semibold, Face::Sans],
        };
        order.iter().find_map(|&f| self.fonts[f.index()].as_ref().map(|font| (f, font)))
    }

    /// How wide `s` is in `st`.
    pub fn measure(&self, s: &str, st: Style) -> f32 {
        match self.font(st.face) {
            Some((_, f)) => {
                let l = layout(f, s, &LayoutOptions::new(st.size));
                l.width + st.tracking * st.size * l.glyphs.len() as f32
            }
            None => (s.chars().count() * 8) as f32,
        }
    }

    /// How wide `s` is at `size` pixels per em (Geist regular).
    pub fn width(&self, s: &str, size: f32) -> f32 {
        self.measure(s, Style::new(Face::Sans, size))
    }

    /// `s` cut to fit `max_w` in `st`, with an ellipsis if cut.
    pub fn fit_style(&self, s: &str, st: Style, max_w: f32) -> alloc::string::String {
        if self.measure(s, st) <= max_w {
            return s.into();
        }
        let mut out = alloc::string::String::new();
        for c in s.chars() {
            out.push(c);
            out.push('\u{2026}');
            let too_wide = self.measure(&out, st) > max_w;
            out.pop();
            if too_wide {
                out.pop();
                break;
            }
        }
        out.push('\u{2026}');
        out
    }

    /// `s` cut to fit `max_w` at `size`, with an ellipsis if cut.
    pub fn fit(&self, s: &str, size: f32, max_w: f32) -> alloc::string::String {
        self.fit_style(s, Style::new(Face::Sans, size), max_w)
    }

    /// `s` broken into lines no wider than `max_w` at `size`, at spaces
    /// (a word too long for a line is cut), at most `max_lines` (the last
    /// ellipsized if there was more).
    pub fn wrap(&self, s: &str, size: f32, max_w: f32, max_lines: usize) -> alloc::vec::Vec<alloc::string::String> {
        self.wrap_style(s, Style::new(Face::Sans, size), max_w, max_lines)
    }

    /// [`Text::wrap`] in `st`.
    pub fn wrap_style(
        &self,
        s: &str,
        st: Style,
        max_w: f32,
        max_lines: usize,
    ) -> alloc::vec::Vec<alloc::string::String> {
        let mut lines: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();
        for para in s.split('\n') {
            let mut line = alloc::string::String::new();
            for word in para.split(' ').filter(|w| !w.is_empty()) {
                let candidate = if line.is_empty() { word.into() } else { alloc::format!("{line} {word}") };
                if self.measure(&candidate, st) <= max_w {
                    line = candidate;
                } else {
                    if !line.is_empty() {
                        lines.push(core::mem::take(&mut line));
                    }
                    line = if self.measure(word, st) <= max_w { word.into() } else { self.fit_style(word, st, max_w) };
                }
            }
            lines.push(line);
        }
        while lines.len() > 1 && lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        if lines.len() > max_lines {
            lines.truncate(max_lines);
            if let Some(last) = lines.last_mut() {
                let more = alloc::format!("{last}\u{2026}");
                *last = self.fit_style(&more, st, max_w);
            }
        }
        lines
    }

    /// The height of capital letters in `st`.
    pub fn cap(&self, st: Style) -> f32 {
        match self.font(st.face) {
            Some((f, _)) => self.heights[f.index()].1 * st.size,
            None => 12.0,
        }
    }

    /// The height of capitals at `size` (Geist regular): where to put a
    /// baseline to centre a line in a box.
    pub fn cap_height(&self, size: f32) -> f32 {
        self.cap(Style::new(Face::Sans, size))
    }

    /// The baseline that centres capitals of `st` in `top..top + h`.
    pub fn centre(&self, top: i32, h: i32, st: Style) -> i32 {
        top + ((h as f32 + self.cap(st)) / 2.0 + 0.5) as i32
    }

    /// The vertical size that puts face `f`'s x-height on a whole pixel at
    /// `size` (light hinting; unchanged above 24 pixels).
    fn hinted_size(&self, f: Face, size: f32) -> f32 {
        let xh = self.heights[f.index()].0 * size;
        if size > 24.0 || xh < 3.0 {
            return size;
        }
        let snapped = paint::floor(xh + 0.5);
        size * (snapped / xh).clamp(0.9, 1.1)
    }

    /// Draw `s` in `st` with its baseline at `baseline` from `x` on, in
    /// `rgb` at `alpha`, cut at `max_x`. Returns the pen position after it.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_style(
        &self,
        fb: &mut dyn Framebuffer,
        clip: Rect,
        x: f32,
        baseline: i32,
        max_x: i32,
        s: &str,
        st: Style,
        rgb: u32,
        alpha: u8,
    ) -> f32 {
        let clip = clip.intersect(&Rect::new(clip.x, clip.y, max_x - clip.x, clip.height));
        let Some((face, font)) = self.font(st.face) else {
            return self.draw_bitmap(fb, clip, x, baseline, s, rgb, alpha);
        };
        let l = layout(font, s, &LayoutOptions::new(st.size));
        let size_y = self.hinted_size(face, st.size);
        let track = st.tracking * st.size;
        let mut glyphs = self.glyphs.borrow_mut();
        if glyphs.len() > GLYPHS_KEPT {
            glyphs.clear();
        }
        let screen = Rect::new(0, 0, fb.width() as i32, fb.height() as i32);
        for (i, g) in l.glyphs.iter().enumerate() {
            let (whole, sub) = quantize(x + g.x + track * i as f32);
            let key = (face as u8, (st.size * 64.0) as u32, (size_y * 64.0) as u32, g.glyph.0, sub);
            let bm = glyphs.entry(key).or_insert_with(|| {
                let dx = f32::from(sub) / f32::from(poly_font::SUBPIXEL_STEPS);
                render_glyph_scaled(font, g.glyph, st.size, size_y, dx, RenderMode::Gray)
            });
            let (left, top) = (whole + bm.left, baseline + bm.top);
            let area = Rect::new(left, top, bm.width as i32, bm.height as i32).intersect(&clip).intersect(&screen);
            for py in area.y..area.bottom() {
                let row = fb.row_mut(py as u32);
                for px in area.x..area.right() {
                    let cov = self.gamma[usize::from(bm.coverage((px - left) as u32, (py - top) as u32))];
                    if cov > 0 {
                        row[px as usize] = mix(row[px as usize], rgb, u32::from(cov) * u32::from(alpha) / 255);
                    }
                }
            }
        }
        x + l.width + track * l.glyphs.len() as f32
    }

    /// Draw `s` at `size` (Geist regular); see [`Text::draw_style`].
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
        self.draw_style(fb, clip, x, baseline, max_x, s, Style::new(Face::Sans, size), rgb, alpha)
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
        let screen = Rect::new(0, 0, fb.width() as i32, fb.height() as i32);
        for c in s.chars() {
            let glyph = font.glyph(c);
            let cell = Rect::new(pen, top, 8, 16).intersect(&clip).intersect(&screen);
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
