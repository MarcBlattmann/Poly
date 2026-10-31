//! Rasterisation golden tests on the font built in code
//! (`src/testfont.rs`): straight edges, curves through implied on-curve
//! points, a composite glyph, subpixel offsets, the LCD filter and laid
//! out text. They run everywhere; `tests/dejavu.rs` adds real fonts.

extern crate alloc;

mod common;
#[allow(dead_code)]
#[path = "../src/testfont.rs"]
mod testfont;

use common::{check, glyph_image};
use poly_font::{Font, GlyphCache, GlyphId, LayoutOptions, RenderMode, layout, render_glyph, render_layout};

#[test]
fn straight_edges() {
    let data = testfont::build();
    let font = Font::parse(&data).unwrap();
    let a = render_glyph(&font, GlyphId(1), 24.0, 0.0, RenderMode::Gray);
    check("test-A-24", "test font 'A' (triangle with a hole) at 24 px", &glyph_image(&a));
}

#[test]
fn implied_points_and_a_subpixel_offset() {
    let data = testfont::build();
    let font = Font::parse(&data).unwrap();
    let o = render_glyph(&font, GlyphId(2), 24.0, 0.25, RenderMode::Gray);
    check("test-O-24-q1", "test font 'O' (off-curve-only contour) at 24 px, 1/4 px right", &glyph_image(&o));
}

#[test]
fn composite_glyph() {
    let data = testfont::build();
    let font = Font::parse(&data).unwrap();
    let g = render_glyph(&font, GlyphId(3), 32.0, 0.0, RenderMode::Gray);
    check("test-Aacute-32", "test font composite 'Á' at 32 px", &glyph_image(&g));
}

#[test]
fn lcd_mode() {
    let data = testfont::build();
    let font = Font::parse(&data).unwrap();
    let o = render_glyph(&font, GlyphId(2), 16.0, 0.0, RenderMode::Lcd);
    check("test-O-16-lcd", "test font 'O' at 16 px, LCD (RGB subpixels)", &glyph_image(&o));
}

#[test]
fn laid_out_text() {
    let data = testfont::build();
    let font = Font::parse(&data).unwrap();
    let l = layout(&font, "AOÁ A\nOA", &LayoutOptions::new(18.0));
    let img = render_layout(&font, 0, &mut GlyphCache::new(1 << 20), &l, RenderMode::Gray);
    check("test-text-18", "test font \"AOÁ A\\nOA\" at 18 px with kerning", &img);
}
