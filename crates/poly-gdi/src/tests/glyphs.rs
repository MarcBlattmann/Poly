//! `GetFontData`, `GetGlyphOutlineW` and `GetCharABCWidthsFloatW` (`crate::glyphs`), with the
//! sheets' test font: what Learn says the calls return.

use super::*;
use crate::glyphs::*;

/// PolyTest Sans at -16 selected in a memory DC.
fn sans() -> (Gdi, u64, Vec<u8>) {
    let mut g = Gdi::new();
    let file = tfont::build(&tfont::SANS);
    assert!(g.add_font_mem(&file).is_some());
    let (dc, _px) = dib32(&mut g, 20, 20);
    select(&mut g, dc, &lf_full("PolyTest Sans", -16, 400, false, 3));
    (g, dc, file)
}

fn tag(t: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*t)
}

#[test]
fn font_data_is_the_selected_files_tables() {
    let (g, dc, file) = sans();
    // Table 0: the whole file.
    assert_eq!(g.font_table(dc, 0, 0), Some(Some(file.clone())));
    // A table by its tag: `head` starts with its version 1.0 and is 54 bytes long.
    let head = g.font_table(dc, tag(b"head"), 0).unwrap().unwrap();
    assert_eq!(head.len(), 54);
    assert_eq!(&head[..4], &[0, 1, 0, 0]);
    // From an offset: the rest of the table; at its end nothing; beyond it an error.
    assert_eq!(g.font_table(dc, tag(b"head"), 4).unwrap().unwrap(), head[4..].to_vec());
    assert_eq!(g.font_table(dc, tag(b"head"), 54), Some(Some(Vec::new())));
    assert_eq!(g.font_table(dc, tag(b"head"), 55), Some(None));
    // A table the font does not have.
    assert_eq!(g.font_table(dc, tag(b"zzzz"), 0), Some(None));
    // Not a DC.
    assert_eq!(g.font_table(0x1234, 0, 0), None);
    // A bitmap face (the stock system font of a new DC) has no file.
    let mut g2 = Gdi::new();
    let (dc2, _px) = dib32(&mut g2, 4, 4);
    assert_eq!(g2.font_table(dc2, 0, 0), Some(None));
    assert_eq!(g2.glyph_outline(dc2, 0x41, GGO_METRICS, Mat2::IDENTITY), Some(Err(GlyphError::NotOutline)));
}

#[test]
fn glyph_metrics_match_what_is_drawn_and_measured() {
    let (g, dc, _) = sans();
    let width = g.char_widths(dc, u32::from(b'A'), u32::from(b'A')).unwrap()[0];
    let (gm, data) = g.glyph_outline(dc, u16::from(b'A'), GGO_METRICS, Mat2::IDENTITY).unwrap().unwrap();
    assert!(data.is_empty());
    assert!(gm.black_box_x > 0 && gm.black_box_y > 0, "{gm:?}");
    assert!(gm.origin_y > 0, "the glyph stands on the baseline: {gm:?}");
    assert_eq!((gm.cell_inc_x, gm.cell_inc_y), (width as i16, 0));
    // The grey bitmap: rows padded to 32 bits, 65 levels, full coverage inside a stem.
    let (gm8, gray8) = g.glyph_outline(dc, u16::from(b'A'), GGO_GRAY8_BITMAP, Mat2::IDENTITY).unwrap().unwrap();
    assert_eq!(gm8, gm);
    let stride = (gm.black_box_x as usize).div_ceil(4) * 4;
    assert_eq!(gray8.len(), stride * gm.black_box_y as usize);
    assert!(gray8.iter().all(|&v| v <= 64) && gray8.contains(&64));
    let (_, gray2) = g.glyph_outline(dc, u16::from(b'A'), GGO_GRAY2_BITMAP, Mat2::IDENTITY).unwrap().unwrap();
    assert!(gray2.iter().all(|&v| v <= 4) && gray2.contains(&4));
    // One bit a pixel, rows of whole 32-bit words.
    let (_, mono) = g.glyph_outline(dc, u16::from(b'A'), GGO_BITMAP, Mat2::IDENTITY).unwrap().unwrap();
    assert_eq!(mono.len(), (gm.black_box_x as usize).div_ceil(32) * 4 * gm.black_box_y as usize);
    assert!(mono.iter().any(|&b| b != 0));
    // By glyph index: the same glyph.
    // 'A' is the font's sixth glyph (`testfonts.rs`'s order).
    let gid = 5;
    let (gi, _) = g.glyph_outline(dc, gid, GGO_METRICS | GGO_GLYPH_INDEX, Mat2::IDENTITY).unwrap().unwrap();
    assert_eq!(gi, gm);
    // MAT2 doubling the size doubles the advance.
    let two = Mat2 { m11: 2.0, m12: 0.0, m21: 0.0, m22: 2.0 };
    let (g2m, _) = g.glyph_outline(dc, u16::from(b'A'), GGO_METRICS, two).unwrap().unwrap();
    assert_eq!(g2m.cell_inc_x, 2 * width as i16);
    assert!(g2m.black_box_y >= 2 * gm.black_box_y - 1);
}

#[test]
fn a_space_has_a_one_pixel_box_and_no_bits() {
    let (g, dc, _) = sans();
    let (gm, data) = g.glyph_outline(dc, 0x20, GGO_GRAY8_BITMAP, Mat2::IDENTITY).unwrap().unwrap();
    assert_eq!((gm.black_box_x, gm.black_box_y, gm.origin_x, gm.origin_y), (1, 1, 0, 0));
    assert!(data.is_empty());
    assert!(gm.cell_inc_x > 0);
}

#[test]
fn native_and_bezier_outlines_are_polygon_records() {
    let (g, dc, _) = sans();
    for (format, curve) in [(GGO_NATIVE, TT_PRIM_QSPLINE), (GGO_BEZIER, TT_PRIM_CSPLINE)] {
        let (_, data) = g.glyph_outline(dc, u16::from(b'O'), format, Mat2::IDENTITY).unwrap().unwrap();
        assert!(!data.is_empty());
        // Walk the contours: each header's cb covers its records exactly.
        let mut at = 0;
        let mut contours = 0;
        while at < data.len() {
            let cb = u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize;
            assert_eq!(u32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()), TT_POLYGON_TYPE);
            let mut r = at + 16;
            while r < at + cb {
                let kind = u16::from_le_bytes([data[r], data[r + 1]]);
                let n = u16::from_le_bytes([data[r + 2], data[r + 3]]) as usize;
                assert!(kind == TT_PRIM_LINE || kind == curve, "format {format}: record type {kind}");
                if kind == TT_PRIM_CSPLINE {
                    assert_eq!(n % 3, 0);
                }
                r += 4 + 8 * n;
            }
            assert_eq!(r, at + cb, "format {format}: records fill the contour");
            at += cb;
            contours += 1;
        }
        assert!(contours >= 1);
    }
    // Asking for an unknown format.
    assert_eq!(g.glyph_outline(dc, u16::from(b'A'), 9, Mat2::IDENTITY), Some(Err(GlyphError::BadFormat)));
}

#[test]
fn abc_widths_add_up_to_the_advance() {
    let (g, dc, _) = sans();
    let abc = g.char_abc_float(dc, u32::from(b'A'), u32::from(b'C')).unwrap();
    let widths = g.char_widths(dc, u32::from(b'A'), u32::from(b'C')).unwrap();
    for (v, w) in abc.iter().zip(widths) {
        assert!(v[1] > 0.0);
        assert!((v[0] + v[1] + v[2] - w as f32).abs() <= 0.5, "{v:?} vs {w}");
    }
    assert_eq!(g.char_abc_float(dc, 5, 4), None);
}

#[test]
fn whole_pixel_abc_and_glyph_widths_follow_the_advance() {
    let (g, dc, _) = sans();
    let width = g.char_widths(dc, u32::from(b'A'), u32::from(b'A')).unwrap()[0];
    let abc = g.char_abc(dc, &[u16::from(b'A')], false).unwrap().unwrap();
    let (a, b, c) = abc[0];
    assert_eq!(a + b as i32 + c, width);
    let (gm, _) = g.glyph_outline(dc, u16::from(b'A'), GGO_METRICS, Mat2::IDENTITY).unwrap().unwrap();
    assert_eq!((a, b), (gm.origin_x, gm.black_box_x));
    // By glyph index: 'A' is glyph 5 of the test font.
    assert_eq!(g.char_abc(dc, &[5], true).unwrap().unwrap(), abc);
    assert_eq!(g.glyph_widths(dc, &[5, 5]), Some(Some(alloc::vec![width, width])));
    // A bitmap face has neither.
    let mut g2 = Gdi::new();
    let (dc2, _px) = dib32(&mut g2, 4, 4);
    assert_eq!(g2.char_abc(dc2, &[65], false), Some(None));
    assert_eq!(g2.glyph_widths(dc2, &[5]), Some(None));
}
