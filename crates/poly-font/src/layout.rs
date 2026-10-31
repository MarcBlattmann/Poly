//! Simple text layout: one font, left to right, characters mapped to
//! glyphs one to one, advances and pair kerning, lines broken at spaces
//! (and at `\n`) to fit a width. No shaping (ligatures, contextual forms,
//! bidirectional text) and no fallback fonts: characters the font lacks
//! use its `.notdef` glyph.

use crate::cache::{GlyphCache, GlyphKey, quantize};
use crate::font::{Font, GlyphId};
use crate::math::ceil;
use crate::render::{Image, RenderMode};
use alloc::vec::Vec;
use core::ops::Range;

/// How to lay text out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutOptions {
    /// Pixels per em.
    pub size_px: f32,
    /// Break lines at spaces so that none is wider than this, if possible
    /// (a single word wider than this stays on a line of its own).
    pub max_width: Option<f32>,
    /// Apply the font's pair kerning.
    pub kerning: bool,
    /// Baseline-to-baseline distance in pixels; `None`: the font's.
    pub line_height: Option<f32>,
}

impl LayoutOptions {
    pub fn new(size_px: f32) -> LayoutOptions {
        LayoutOptions { size_px, max_width: None, kerning: true, line_height: None }
    }
}

/// A glyph placed on a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    pub glyph: GlyphId,
    /// The character it stands for, and its byte offset in the text.
    pub ch: char,
    pub offset: usize,
    /// Pen position in pixels: x from the left edge, y of the baseline
    /// from the top of the layout.
    pub x: f32,
    pub y: f32,
}

/// One line of a layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Its glyphs in [`TextLayout::glyphs`] (spaces where it was broken are
    /// left out).
    pub glyphs: Range<usize>,
    /// The byte range of the text it shows.
    pub text: Range<usize>,
    /// From the left edge to the end of the last glyph's advance.
    pub width: f32,
    pub baseline: f32,
}

/// Laid-out text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextLayout {
    pub glyphs: Vec<PositionedGlyph>,
    pub lines: Vec<Line>,
    /// The widest line.
    pub width: f32,
    /// From the top of the first line to the bottom of the last.
    pub height: f32,
    pub size_px: f32,
}

/// A word (or run of spaces) about to be placed.
struct Piece {
    range: Range<usize>,
    space: bool,
}

/// Lay out `text` in `font`.
pub fn layout(font: &Font<'_>, text: &str, opts: &LayoutOptions) -> TextLayout {
    let size = if opts.size_px.is_finite() { opts.size_px.clamp(0.0, crate::render::MAX_SIZE_PX) } else { 0.0 };
    let scale = size / f32::from(font.units_per_em());
    let vm = font.vmetrics();
    let ascent = f32::from(vm.ascender) * scale;
    let descent = -f32::from(vm.descender) * scale;
    let line_height = opts.line_height.filter(|h| h.is_finite() && *h > 0.0).unwrap_or(vm.line_height() as f32 * scale);
    let mut out = TextLayout { size_px: size, ..TextLayout::default() };
    let glyph_of = |c: char| font.glyph_index(c).unwrap_or(GlyphId(0));
    // The advance of a run of characters starting after glyph `prev`.
    let run_width = |range: &Range<usize>, prev: Option<GlyphId>| -> f32 {
        let mut w = 0.0;
        let mut prev = prev;
        for c in text[range.clone()].chars() {
            let g = glyph_of(c);
            if opts.kerning
                && let Some(p) = prev
            {
                w += f32::from(font.kerning(p, g)) * scale;
            }
            w += f32::from(font.advance(g)) * scale;
            prev = Some(g);
        }
        w
    };

    let mut baseline = ascent;
    let mut para_start = 0;
    for para in text.split('\n') {
        let base = para_start;
        para_start += para.len() + 1;
        // Words and space runs.
        let mut pieces: Vec<Piece> = Vec::new();
        for (i, c) in para.char_indices() {
            let space = c == ' ';
            match pieces.last_mut() {
                Some(p) if p.space == space => p.range.end = base + i + c.len_utf8(),
                _ => pieces.push(Piece { range: base + i..base + i + c.len_utf8(), space }),
            }
        }
        // Greedy filling: a word goes on the line if it fits after the
        // spaces before it; else the line ends before those spaces.
        let mut line_start_glyph = out.glyphs.len();
        let mut line_text_start = base;
        let mut x = 0.0f32;
        let mut prev: Option<GlyphId> = None;
        let mut pending_spaces: Option<Range<usize>> = None;
        let mut line_end_text = base;
        let mut line_width = 0.0f32;
        let finish = |out: &mut TextLayout, start_glyph: usize, text: Range<usize>, width: f32, baseline: f32| {
            out.width = out.width.max(width);
            out.lines.push(Line { glyphs: start_glyph..out.glyphs.len(), text, width, baseline });
        };
        for piece in pieces {
            if piece.space {
                pending_spaces = Some(piece.range);
                continue;
            }
            let spaces_w = pending_spaces.as_ref().map_or(0.0, |s| run_width(s, prev));
            let after_spaces =
                pending_spaces.as_ref().and_then(|s| text[s.clone()].chars().last()).map(glyph_of).or(prev);
            let word_w = run_width(&piece.range, after_spaces);
            let has_content = out.glyphs.len() > line_start_glyph;
            if let Some(max) = opts.max_width
                && has_content
                && x + spaces_w + word_w > max
            {
                finish(&mut out, line_start_glyph, line_text_start..line_end_text, line_width, baseline);
                baseline += line_height;
                line_start_glyph = out.glyphs.len();
                line_text_start = piece.range.start;
                x = 0.0;
                prev = None;
                pending_spaces = None;
            }
            let mut place = |range: Range<usize>, x: &mut f32, prev: &mut Option<GlyphId>| {
                for (i, c) in text[range.clone()].char_indices() {
                    let g = glyph_of(c);
                    if opts.kerning
                        && let Some(p) = *prev
                    {
                        *x += f32::from(font.kerning(p, g)) * scale;
                    }
                    out.glyphs.push(PositionedGlyph { glyph: g, ch: c, offset: range.start + i, x: *x, y: baseline });
                    *x += f32::from(font.advance(g)) * scale;
                    *prev = Some(g);
                }
            };
            if let Some(s) = pending_spaces.take() {
                place(s, &mut x, &mut prev);
            }
            place(piece.range.clone(), &mut x, &mut prev);
            line_end_text = piece.range.end;
            line_width = x;
        }
        // Trailing spaces stay on the line (they are part of its text) but
        // do not count toward its width.
        if let Some(s) = pending_spaces.take() {
            let mut sx = x;
            let mut p = prev;
            for (i, c) in text[s.clone()].char_indices() {
                let g = glyph_of(c);
                if opts.kerning
                    && let Some(pg) = p
                {
                    sx += f32::from(font.kerning(pg, g)) * scale;
                }
                out.glyphs.push(PositionedGlyph { glyph: g, ch: c, offset: s.start + i, x: sx, y: baseline });
                sx += f32::from(font.advance(g)) * scale;
                p = Some(g);
            }
            line_end_text = s.end;
        }
        finish(&mut out, line_start_glyph, line_text_start..line_end_text, line_width, baseline);
        baseline += line_height;
    }
    let lines = out.lines.len() as f32;
    out.height = if lines > 0.0 { ascent + descent + (lines - 1.0) * line_height } else { 0.0 };
    out
}

/// Draw a layout into a new coverage image just big enough for it (its
/// width rounded up, plus a pixel each side for antialiasing and LCD
/// padding), rendering glyphs through `cache` (`font_id` is the font's
/// number there) at quarter-pixel positions.
pub fn render_layout(font: &Font<'_>, font_id: u32, cache: &mut GlyphCache, l: &TextLayout, mode: RenderMode) -> Image {
    let w = ceil(l.width) as u32 + 2;
    let h = ceil(l.height) as u32 + 2;
    let mut img = Image::new(w, h, mode);
    for g in &l.glyphs {
        let (whole, sub) = quantize(g.x + 1.0);
        let key = GlyphKey::new(font_id, l.size_px, g.glyph, sub, mode);
        let bm = cache.get(font, key);
        img.draw(bm, whole, crate::math::round(g.y) as i32 + 1);
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfont;

    #[test]
    fn advances_and_kerning() {
        let data = testfont::build();
        let font = Font::parse(&data).unwrap();
        // At 10 px: 'A' advances 6, 'O' 7, kerning A-O is -0.4.
        let l = layout(&font, "AOA", &LayoutOptions::new(10.0));
        let xs: Vec<f32> = l.glyphs.iter().map(|g| g.x).collect();
        assert_eq!(xs, [0.0, 5.6, 12.6]);
        assert_eq!(l.lines.len(), 1);
        assert!((l.width - 18.6).abs() < 1e-4);
        assert_eq!(l.glyphs[0].y, 8.0);
        assert_eq!(l.height, 10.0);
        let plain = layout(&font, "AOA", &LayoutOptions { kerning: false, ..LayoutOptions::new(10.0) });
        assert_eq!(plain.glyphs[1].x, 6.0);
        // Unknown characters use .notdef.
        let l = layout(&font, "z", &LayoutOptions::new(10.0));
        assert_eq!(l.glyphs[0].glyph, GlyphId(0));
    }

    #[test]
    fn breaks_lines_at_spaces() {
        let data = testfont::build();
        let font = Font::parse(&data).unwrap();
        // "AA" is 12 px wide at 10 px, a space 2.5 px.
        let opts = LayoutOptions { max_width: Some(30.0), ..LayoutOptions::new(10.0) };
        let text = "AA AA AA\nO";
        let l = layout(&font, text, &opts);
        let lines: Vec<&str> = l.lines.iter().map(|ln| &text[ln.text.clone()]).collect();
        assert_eq!(lines, ["AA AA", "AA", "O"]);
        assert!((l.lines[0].width - 26.5).abs() < 1e-4);
        assert_eq!(l.lines[1].baseline, 8.0 + 11.0);
        assert_eq!(l.lines[2].baseline, 8.0 + 22.0);
        // The space where the line broke is not laid out.
        assert_eq!(l.glyphs[l.lines[1].glyphs.start].x, 0.0);
        assert_eq!(l.glyphs.len(), 5 + 2 + 1);
        assert!((l.height - (10.0 + 22.0)).abs() < 1e-4);
        // A word too long for the width stays whole on its own line.
        let narrow = LayoutOptions { max_width: Some(5.0), ..LayoutOptions::new(10.0) };
        let l = layout(&font, "AAAA A", &narrow);
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[0].glyphs.len(), 4);
        // Empty text, empty lines.
        assert_eq!(layout(&font, "", &opts).lines.len(), 1);
        assert_eq!(layout(&font, "\n\n", &opts).lines.len(), 3);
    }

    #[test]
    fn renders_through_the_cache() {
        let data = testfont::build();
        let font = Font::parse(&data).unwrap();
        let l = layout(&font, "AO", &LayoutOptions::new(20.0));
        let mut cache = GlyphCache::new(1 << 20);
        let img = render_layout(&font, 7, &mut cache, &l, RenderMode::Gray);
        assert_eq!((img.width, img.height), (ceil(l.width) as u32 + 2, 22));
        assert!(img.data.contains(&255));
        assert_eq!(cache.len(), 2);
        let again = render_layout(&font, 7, &mut cache, &l, RenderMode::Gray);
        assert_eq!(again, img);
        assert_eq!(cache.stats().hits, 2);
    }
}
