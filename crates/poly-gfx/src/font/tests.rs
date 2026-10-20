use super::*;

/// Printable ASCII and Latin-1, which every built-in font covers.
fn printable() -> impl Iterator<Item = char> {
    (0x20u8..0x7f).chain(0xa0..=0xff).map(char::from)
}

#[test]
fn covers_ascii_and_latin1() {
    for font in FONTS {
        for c in printable() {
            assert!(font.lookup(c).is_some(), "{} lacks {c:?}", font.name());
        }
        // Swiss German needs these, and the shell's box drawing these.
        for c in "äöüÄÖÜéàèçÇ£§€«»°±µ└─┘│┌┐├┤═║█▒".chars() {
            assert!(font.lookup(c).is_some(), "{} lacks {c:?}", font.name());
        }
        assert!(font.glyph_count() > 800, "{}: {} glyphs", font.name(), font.glyph_count());
    }
}

#[test]
fn controls_and_unknown_characters_fall_back() {
    for font in FONTS {
        for c in ['\0', '\n', '\x1b', '\x7f', '\u{85}', '中', '\u{e0b0}', '\u{10ffff}', '\u{fffd}'] {
            assert_eq!(font.lookup(c), None, "{} has {c:?}", font.name());
            assert_eq!(font.glyph(c), font.fallback());
        }
        // The fallback is a box: the top row of the box is lit across.
        let fb = font.fallback();
        let top = (0..font.height()).find(|&y| fb.row(y) != 0).unwrap();
        assert!((1..font.width() - 1).all(|x| fb.is_set(x, top)));
        assert!(!fb.is_set(0, top) && !fb.is_set(font.width() - 1, top));
        assert_ne!(font.glyph('A'), fb);
    }
}

#[test]
fn golden_rows() {
    // From the BDFs in third_party/spleen: the glyph's bitmap lines, as hex.
    let a = SPLEEN_8X16.glyph('A');
    let rows: [u32; 16] = core::array::from_fn(|y| a.row(y as u32) >> 24);
    assert_eq!(rows, [0, 0, 0x7c, 0xc6, 0xc6, 0xc6, 0xfe, 0xc6, 0xc6, 0xc6, 0xc6, 0xc6, 0, 0, 0, 0]);
    let ae = SPLEEN_8X16.glyph('ä');
    assert_eq!(ae.row(2) >> 24, 0x6c);
    assert_eq!(ae.row(5) >> 24, 0x7c);
    assert_eq!(ae.row(7) >> 24, 0x7e);

    // 12 pixels wide: two bytes per row, the low four bits padding.
    let ae = SPLEEN_12X24.glyph('ä');
    let rows: [u32; 24] = core::array::from_fn(|y| ae.row(y as u32) >> 16);
    #[rustfmt::skip]
    let want = [
        0, 0, 0, 0x30c0, 0x30c0, 0, 0, 0, 0x3f80, 0x00c0, 0x0060, 0x0060,
        0x1fe0, 0x3060, 0x6060, 0x6060, 0x6060, 0x3060, 0x1fe0, 0, 0, 0, 0, 0,
    ];
    assert_eq!(rows, want);
    assert!(ae.is_set(2, 3) && ae.is_set(3, 3) && !ae.is_set(4, 3) && ae.is_set(8, 3) && ae.is_set(9, 3));
    assert!(!ae.is_set(12, 3) && !ae.is_set(100, 3) && ae.row(24) == 0);
}

#[test]
fn metrics() {
    assert_eq!((SPLEEN_8X16.width(), SPLEEN_8X16.height(), SPLEEN_8X16.ascent()), (8, 16, 12));
    assert_eq!((SPLEEN_12X24.width(), SPLEEN_12X24.height(), SPLEEN_12X24.ascent()), (12, 24, 19));
    assert_eq!(SPLEEN_8X16.text_width("Grüezi"), 48);
    let a = SPLEEN_8X16.glyph('a');
    assert_eq!((a.width(), a.height()), (8, 16));
}

#[test]
fn lookup_finds_the_first_and_last_glyph_of_every_range() {
    for font in FONTS {
        for r in font.ranges {
            for cp in [r.first, r.last] {
                let c = char::from_u32(cp).unwrap();
                let g = font.lookup(c).unwrap();
                let start = (r.index + (cp - r.first) as usize) * font.glyph_bytes();
                assert_eq!(g.bits, &font.bitmaps[start..start + font.glyph_bytes()]);
            }
            if let Some(c) = char::from_u32(r.last + 1) {
                let next = font.ranges.iter().any(|n| n.first == r.last + 1);
                assert_eq!(font.lookup(c).is_some(), next);
            }
        }
    }
}

#[test]
fn hex_decodes_pairs() {
    assert_eq!(hex::<3>("00a5ff"), [0x00, 0xa5, 0xff]);
    assert_eq!(hex::<0>(""), []);
}
