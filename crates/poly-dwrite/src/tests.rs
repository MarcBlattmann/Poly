//! Tests of the font-level rules over a small built-in font.

use alloc::string::ToString;
use alloc::vec::Vec;

use crate::face::{Cmap, FaceInfo, STYLE_ITALIC, STYLE_NORMAL, STYLE_OBLIQUE, design_glyph_metrics};
use crate::names::{to_string, utf16};
use crate::sfnt::{self, Sfnt};
use crate::testfont::{Spec, build};

fn info(s: &Spec) -> FaceInfo {
    FaceInfo::parse(&build(s), 0).expect("a font")
}

fn named(id: u16, text: &str) -> (u16, u16, alloc::string::String) {
    (0x409, id, text.to_string())
}

#[test]
fn the_test_font_is_a_font() {
    let data = build(&Spec::default());
    let a = sfnt::analyze(&data);
    assert!(a.supported);
    assert_eq!((a.file_type, a.face_type, a.faces), (sfnt::file_type::TRUETYPE, sfnt::face_type::TRUETYPE, 1));
    // cut after the directory, or in the middle of it
    assert!(!sfnt::analyze(&data[..100]).supported);
    assert!(Sfnt::parse(&data, 1).is_none());
}

#[test]
fn weights() {
    for (class, want) in
        [(0u16, 1), (1, 100), (5, 500), (9, 900), (10, 10), (350, 350), (999, 999), (1000, 999), (65535, 999)]
    {
        let i = info(&Spec { weight: class, ..Spec::default() });
        assert_eq!(i.weight, want, "class {class}");
    }
    // The bold flag raises a lighter weight to 700.
    let i = info(&Spec { fs_selection: 0x60, ..Spec::default() });
    assert_eq!(i.weight, 700);
}

#[test]
fn stretches() {
    for (class, want) in [(0u16, 5), (1, 1), (3, 3), (9, 9), (10, 9), (65535, 9)] {
        assert_eq!(info(&Spec { width: class, ..Spec::default() }).stretch, want, "class {class}");
    }
}

#[test]
fn styles() {
    assert_eq!(info(&Spec::default()).style, STYLE_NORMAL);
    assert_eq!(info(&Spec { fs_selection: 0x41, ..Spec::default() }).style, STYLE_ITALIC);
    let oblique = Spec { names: alloc::vec![named(2, "Oblique")], ..Spec::default() };
    assert_eq!(info(&oblique).style, STYLE_OBLIQUE);
    // The italic bit wins over the name.
    let both = Spec { fs_selection: 0x41, names: alloc::vec![named(2, "Oblique")], ..Spec::default() };
    assert_eq!(info(&both).style, STYLE_ITALIC);
}

#[test]
fn metrics() {
    let m = info(&Spec::default()).metrics;
    assert_eq!((m.design_units_per_em, m.ascent, m.descent, m.line_gap), (1000, 900, 300, 0));
    assert_eq!((m.cap_height, m.x_height), (700, 500));
    assert_eq!((m.underline_position, m.underline_thickness), (-100, 50));
    assert_eq!((m.strikethrough_position, m.strikethrough_thickness), (250, 40));
    // USE_TYPO_METRICS
    let m = info(&Spec { fs_selection: 0xC0, ..Spec::default() }).metrics;
    assert_eq!((m.ascent, m.descent, m.line_gap), (750, 250, 100));
    // The line gap makes up the difference to hhea.
    let m = info(&Spec { hhea: [800, -200, 300], ..Spec::default() }).metrics;
    assert_eq!(m.line_gap, 100);
    let m = info(&Spec { win: [700, 200], hhea: [1000, -300, 50], ..Spec::default() }).metrics;
    assert_eq!((m.ascent, m.descent, m.line_gap), (700, 200, 450));
    // Cap and x height derived when the file has none.
    let m = info(&Spec { cap: 0, x: 0, upem: 2000, ..Spec::default() }).metrics;
    assert_eq!((m.cap_height, m.x_height), (1400, 1000));
    // No strike-through size: the underline's thickness.
    let m = info(&Spec { strike: [0, 0], ..Spec::default() }).metrics;
    assert_eq!((m.strikethrough_position, m.strikethrough_thickness), (0, 50));
}

#[test]
fn names_and_informational_strings() {
    let s = Spec {
        names: alloc::vec![
            named(1, "Fam"),
            named(2, "Bold"),
            named(16, "Typo Fam"),
            named(17, "Heavy"),
            named(22, "wws sub"),
            named(4, "Full Name"),
            named(6, "Post-Script"),
        ],
        ..Spec::default()
    };
    let i = info(&s);
    assert_eq!(to_string(i.family.primary().unwrap()), "Typo Fam");
    assert_eq!(to_string(i.face.primary().unwrap()), "wws sub");
    let full = i.informational(16).unwrap();
    assert_eq!(to_string(full.primary().unwrap()), "Full Name");
    assert_eq!(to_string(i.informational(17).unwrap().primary().unwrap()), "Post-Script");
    assert_eq!(to_string(i.informational(11).unwrap().primary().unwrap()), "Fam");
    assert_eq!(to_string(i.informational(13).unwrap().primary().unwrap()), "Typo Fam");
    for id in [0, 15, 18, 19, 20, 22, 24, 1000] {
        assert!(i.informational(id).is_none(), "id {id}");
    }
}

#[test]
fn character_map() {
    let data = build(&Spec { format12: true, ..Spec::default() });
    let sf = Sfnt::parse(&data, 0).unwrap();
    let c = Cmap::of(&sf);
    assert!(!c.is_symbol);
    assert_eq!((c.glyph(0x20), c.glyph(0x41), c.glyph(0x1F600)), (1, 2, 2));
    for cp in [0x44, 0, 0xFFFF, 0xD800, 0xDC00, 0x110000, 0xFFFF_FFFF] {
        assert_eq!(c.glyph(cp), 0, "{cp:#x}");
    }
    // Without a format 12 subtable the planes beyond the BMP are empty.
    let data = build(&Spec::default());
    let sf = Sfnt::parse(&data, 0).unwrap();
    assert_eq!(Cmap::of(&sf).glyph(0x1F600), 0);
    // A symbol font answers for both the plain and the 0xF000 code points.
    let data = build(&Spec { symbol: true, ..Spec::default() });
    let sf = Sfnt::parse(&data, 0).unwrap();
    let c = Cmap::of(&sf);
    assert!(c.is_symbol);
    assert_eq!((c.glyph(0x20), c.glyph(0x41), c.glyph(0xF020), c.glyph(0xF041)), (1, 2, 1, 2));
}

#[test]
fn glyph_metrics() {
    let data = build(&Spec::default());
    let sf = Sfnt::parse(&data, 0).unwrap();
    let i = FaceInfo::parse(&data, 0).unwrap();
    let m = design_glyph_metrics(&sf, &i, 2);
    assert_eq!((m.left_side_bearing, m.advance_width, m.right_side_bearing), (100, 1000, 100));
    // typographic ascender 750, descender -250: origin 750, height 1000.
    assert_eq!((m.vertical_origin_y, m.advance_height), (750, 1000));
    assert_eq!((m.top_side_bearing, m.bottom_side_bearing), (-50, 250));
    // A glyph without outline.
    let m = design_glyph_metrics(&sf, &i, 1);
    assert_eq!((m.left_side_bearing, m.advance_width, m.right_side_bearing), (0, 300, 300));
    assert_eq!((m.top_side_bearing, m.bottom_side_bearing), (750, 250));
    // A glyph beyond the font is glyph 0.
    assert_eq!(design_glyph_metrics(&sf, &i, 99), design_glyph_metrics(&sf, &i, 0));
}

#[test]
fn rendering_a_glyph_of_the_test_font() {
    let data = build(&Spec::default());
    let font = poly_font::Font::parse(&data).unwrap();
    let info = FaceInfo::parse(&data, 0).unwrap();
    let segs = crate::render::glyph_segments(&font, &info, 2, 10.0, false);
    let mut polys = crate::render::Polys::default();
    crate::render::flatten(&segs, 0.0, 0.0, &crate::render::Matrix::IDENTITY, &mut polys);
    let r = crate::render::rasterize(&polys, crate::render::Kind::Aliased);
    assert_eq!(r.rect, [1, -8, 9, 0]);
    // Sideways: the quarter turn of the outline.
    let segs = crate::render::glyph_segments(&font, &info, 2, 10.0, true);
    let mut polys = crate::render::Polys::default();
    crate::render::flatten(&segs, 0.0, 0.0, &crate::render::Matrix::IDENTITY, &mut polys);
    let b = polys.bounds().unwrap();
    assert_eq!(b, [-0.5, -4.0, 7.5, 4.0]);
    let _ = (utf16(""), Vec::<u8>::new());
}
