//! Tests on the bundled fonts (DejaVu 2.37, `cargo xtask fonts fetch`):
//! parsing every glyph of every font, names, metrics, kerning, matching
//! with Poly's `fonts.conf`, and golden renderings of real text. Each test
//! skips (saying so) when the fonts have not been fetched.

mod common;

use common::{check, font_dir, glyph_image};
use poly_font::fc::{Config, FontSet, POLY_FONTS_CONF, Pattern, match_line};
use poly_font::{
    Font, GlyphCache, GlyphId, LayoutOptions, OutlineFormat, RenderMode, layout, name_id, render_glyph, render_layout,
};
use std::path::PathBuf;

/// The bundled files (xtask's `fonts.rs` lists the same).
const FILES: [&str; 12] = [
    "DejaVuSans.ttf",
    "DejaVuSans-Bold.ttf",
    "DejaVuSans-Oblique.ttf",
    "DejaVuSans-BoldOblique.ttf",
    "DejaVuSerif.ttf",
    "DejaVuSerif-Bold.ttf",
    "DejaVuSerif-Italic.ttf",
    "DejaVuSerif-BoldItalic.ttf",
    "DejaVuSansMono.ttf",
    "DejaVuSansMono-Bold.ttf",
    "DejaVuSansMono-Oblique.ttf",
    "DejaVuSansMono-BoldOblique.ttf",
];

/// The bundled fonts: their directory, and each file name with its bytes.
type Bundled = (PathBuf, Vec<(String, Vec<u8>)>);

fn fonts() -> Option<Bundled> {
    let Some(dir) = font_dir() else {
        eprintln!("skipped: the bundled fonts are not fetched (cargo xtask fonts fetch)");
        return None;
    };
    let data = FILES.iter().map(|f| (f.to_string(), std::fs::read(dir.join(f)).unwrap())).collect();
    Some((dir, data))
}

fn load(name: &str) -> Option<Vec<u8>> {
    font_dir().map(|d| std::fs::read(d.join(name)).unwrap())
}

#[test]
fn every_glyph_of_every_font_parses() {
    let Some((_, all)) = fonts() else { return };
    for (file, data) in &all {
        let font = Font::parse(data).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(font.outline_format(), OutlineFormat::TrueType, "{file}");
        let [x0, y0, x1, y1] = font.bounding_box().map(f32::from);
        let mut outlines = 0;
        for g in 0..font.num_glyphs() {
            if let Some(p) = font.outline(GlyphId(g)) {
                outlines += 1;
                let b = p.bounds().unwrap();
                // Control points may stick out of the box a little.
                assert!(
                    b.x_min >= x0 - 64.0 && b.y_min >= y0 - 64.0 && b.x_max <= x1 + 64.0 && b.y_max <= y1 + 64.0,
                    "{file}: glyph {g} outside the font box"
                );
            }
        }
        // Nearly every glyph has an outline (spaces and controls do not).
        assert!(outlines * 100 >= usize::from(font.num_glyphs()) * 95, "{file}: {outlines} of {}", font.num_glyphs());
        for c in ['A', 'g', 'é', 'Ω', 'Ж', '€', '→', '0'] {
            assert!(font.has_char(c), "{file} lacks {c}");
        }
    }
}

#[test]
fn names_metrics_and_kerning() {
    let Some(data) = load("DejaVuSans.ttf") else { return };
    let sans = Font::parse(&data).unwrap();
    assert_eq!(sans.family_name().as_deref(), Some("DejaVu Sans"));
    assert_eq!(sans.style_name().as_deref(), Some("Book"));
    assert_eq!(sans.name(name_id::FULL_NAME).as_deref(), Some("DejaVu Sans"));
    assert_eq!(sans.units_per_em(), 2048);
    let vm = sans.vmetrics();
    assert!(vm.ascender > 1500 && vm.descender < -300, "{vm:?}");
    let s = sans.style();
    assert_eq!((s.weight_class, s.width_class, s.italic, s.monospace), (400, 5, false, false));
    // Pair kerning: "AV" and "To" come closer.
    let g = |c| sans.glyph_index(c).unwrap();
    assert!(sans.kerning(g('A'), g('V')) < 0);
    assert!(sans.kerning(g('T'), g('o')) < 0);
    assert_eq!(sans.kerning(g('o'), g('o')), 0);
    let mono_data = load("DejaVuSansMono.ttf").unwrap();
    let mono = Font::parse(&mono_data).unwrap();
    assert!(mono.style().monospace);
    let adv = mono.advance(mono.glyph_index('i').unwrap());
    for c in ['W', 'm', '.', ' '] {
        assert_eq!(mono.advance(mono.glyph_index(c).unwrap()), adv, "{c}");
    }
    let bold_italic = load("DejaVuSerif-BoldItalic.ttf").unwrap();
    let oblique = Font::parse(&bold_italic).unwrap();
    let s = oblique.style();
    assert!(s.italic && s.bold && s.weight_class == 700 && s.italic_angle < 0.0, "{s:?}");
}

#[test]
fn matching_with_polys_fonts_conf() {
    let Some((dir, all)) = fonts() else { return };
    let config = Config::parse(POLY_FONTS_CONF).unwrap();
    let mut set = FontSet::new();
    for (file, data) in &all {
        let path = dir.join(file);
        assert_eq!(set.add_file(&path.to_string_lossy(), data), 1);
    }
    let file = |pattern: &str| {
        let m = set.font_match(&config, &Pattern::parse(pattern).unwrap()).unwrap();
        PathBuf::from(m.get_str("file").unwrap()).file_name().unwrap().to_string_lossy().into_owned()
    };
    for (pattern, want) in [
        ("sans-serif", "DejaVuSans.ttf"),
        ("serif", "DejaVuSerif.ttf"),
        ("monospace", "DejaVuSansMono.ttf"),
        ("DejaVu Sans:bold", "DejaVuSans-Bold.ttf"),
        ("DejaVu Sans:italic", "DejaVuSans-Oblique.ttf"),
        ("DejaVu Sans:bold:oblique", "DejaVuSans-BoldOblique.ttf"),
        ("DejaVu Sans:style=Bold Oblique", "DejaVuSans-BoldOblique.ttf"),
        ("serif:bold", "DejaVuSerif-Bold.ttf"),
        ("serif:italic", "DejaVuSerif-Italic.ttf"),
        ("monospace:italic", "DejaVuSansMono-Oblique.ttf"),
        ("dejavusansmono-11:bold", "DejaVuSansMono-Bold.ttf"),
        ("Arial:bold", "DejaVuSans-Bold.ttf"),
        ("Times New Roman:italic", "DejaVuSerif-Italic.ttf"),
        ("Courier New", "DejaVuSansMono.ttf"),
        ("Some Font Nobody Has", "DejaVuSans.ttf"),
        ("", "DejaVuSans.ttf"),
        (":weight=600", "DejaVuSans-Bold.ttf"),
    ] {
        assert_eq!(file(pattern), want, "fc-match {pattern:?}");
    }
    let m = set.font_match(&config, &Pattern::parse("monospace-10").unwrap()).unwrap();
    assert_eq!(match_line(&m), r#"DejaVuSansMono.ttf: "DejaVu Sans Mono" "Book""#);
    assert_eq!(set.list(&Pattern::parse(":spacing=mono").unwrap()).len(), 4);
    assert_eq!(set.list(&Pattern::parse("DejaVu Serif").unwrap()).len(), 4);
    assert_eq!(set.list(&Pattern::parse(":slant=0:weight=bold").unwrap()).len(), 3);
}

fn text_image(file: &str, text: &str, size: f32, mode: RenderMode) -> Option<poly_font::Image> {
    let data = load(file)?;
    let font = Font::parse(&data).unwrap();
    let l = layout(&font, text, &LayoutOptions::new(size));
    Some(render_layout(&font, 0, &mut GlyphCache::new(1 << 20), &l, mode))
}

#[test]
fn golden_text() {
    let Some(img) = text_image("DejaVuSans.ttf", "Poly AV To", 24.0, RenderMode::Gray) else { return };
    check("dejavu-sans-24", "DejaVu Sans 24 px \"Poly AV To\" (kerned)", &img);
    let img = text_image("DejaVuSerif-Italic.ttf", "Hello, world!", 18.0, RenderMode::Gray).unwrap();
    check("dejavu-serif-italic-18", "DejaVu Serif Italic 18 px \"Hello, world!\"", &img);
    let img = text_image("DejaVuSansMono.ttf", "fn main() {}", 14.0, RenderMode::Lcd).unwrap();
    check("dejavu-mono-14-lcd", "DejaVu Sans Mono 14 px \"fn main() {}\", LCD", &img);
    let img = text_image("DejaVuSans-Bold.ttf", "Ωé€ Жg", 12.0, RenderMode::Gray).unwrap();
    check("dejavu-sans-bold-12", "DejaVu Sans Bold 12 px \"Ωé€ Жg\"", &img);
}

#[test]
fn golden_glyph() {
    let Some(data) = load("DejaVuSans.ttf") else { return };
    let font = Font::parse(&data).unwrap();
    let g = font.glyph_index('g').unwrap();
    check(
        "dejavu-sans-g-48",
        "DejaVu Sans 'g' at 48 px",
        &glyph_image(&render_glyph(&font, g, 48.0, 0.0, RenderMode::Gray)),
    );
    let amp = font.glyph_index('&').unwrap();
    for (i, dx) in [0.0, 0.5].into_iter().enumerate() {
        let b = render_glyph(&font, amp, 13.0, dx, RenderMode::Gray);
        check(
            &format!("dejavu-sans-amp-13-{i}"),
            &format!("DejaVu Sans '&' at 13 px, {dx} px right"),
            &glyph_image(&b),
        );
    }
}
