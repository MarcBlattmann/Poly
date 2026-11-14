//! The rules of the sharpened sheets of `ExtTextOutW`,
//! `GetTextExtentExPointW`, `RemoveFontResourceExW` and `DrawTextW`
//! (`specs/windows/`), one test per rule, with the sheets' own fonts.

use super::*;
use crate::fonttab::{self, FR_NOT_ENUM, FR_PRIVATE, FontTable, NameCheck};
use crate::text::*;
use std::string::String;

/// The surface, the text and the background as pixels.
const BG: u32 = 0x0080_8080;
const FG: u32 = 0x00FF_FFFF;
const BK: u32 = 0x0000_0080;

/// PolyTest Sans at -16, bi-level, white on dark blue, on a grey surface.
fn sans(w: i32, h: i32) -> (Gdi, u64, Vec<u32>) {
    let mut g = Gdi::new();
    assert!(g.add_font_mem(&tfont::build(&tfont::SANS)).is_some());
    let (dc, mut px) = dib32(&mut g, w, h);
    px.fill(BG);
    select(&mut g, dc, &lf_full("PolyTest Sans", -16, 400, false, 3));
    g.set_text_color(dc, FG);
    g.set_bk_color(dc, 0x0080_0000);
    (g, dc, px)
}

fn box_of(px: &[u32], w: i32, v: u32) -> Option<(i32, i32, i32, i32)> {
    let p = where_is(px, w, v);
    if p.is_empty() { None } else { Some(bounds(&p)) }
}

fn changed(px: &[u32], w: i32) -> Option<(i32, i32, i32, i32)> {
    let p = ink(px, w, BG);
    if p.is_empty() { None } else { Some(bounds(&p)) }
}

// ----- ExtTextOutW

#[test]
fn text_alignment_is_read_by_its_bits() {
    // alignment / eto_alignbits: the cell of "AB" (20x16, ascent 12) at (50,40).
    let (mut g, dc, mut px) = sans(100, 70);
    let left = [50, 30, 30, 40];
    let top = [40, 24, 28, 28];
    for align in 0..0x20u32 {
        if align & 1 != 0 {
            continue;
        }
        let (l, t) = (left[(align >> 1 & 3) as usize], top[(align >> 3 & 3) as usize]);
        px.fill(BG);
        g.set_text_align(dc, align);
        assert_eq!(g.text_align(dc), align, "read back");
        assert!(g.ext_text_out(dc, 50, 40, 0, None, &utf16("AB"), None));
        assert_eq!(changed(&px, 100), Some((l, t, l + 20, t + 16)), "alignment {align:#x}");
        // TextOutW draws exactly the same cells.
        let ext = px.clone();
        px.fill(BG);
        assert!(g.text_out(dc, 50, 40, &utf16("AB")));
        assert_eq!(px, ext, "TextOutW, alignment {align:#x}");
    }
    // The higher bits do not move the text.
    for align in [0x20u32, 0x40, 0x80, 0x200, 0x400, 0x8000, 0x1_0000, 0xFFFF_FFE0] {
        px.fill(BG);
        g.set_text_align(dc, align);
        assert_eq!(g.text_align(dc), align, "read back");
        assert!(g.ext_text_out(dc, 50, 40, 0, None, &utf16("AB"), None));
        assert_eq!(changed(&px, 100), Some((50, 40, 70, 56)), "alignment {align:#x}");
    }
    px.fill(BG);
    g.set_text_align(dc, 0xFFFF_FFE6);
    assert!(g.ext_text_out(dc, 50, 40, 0, None, &utf16("AB"), None));
    assert_eq!(changed(&px, 100), Some((40, 40, 60, 56)));
    // The bits, without a surface.
    let a = TextAlign::from_bits;
    assert_eq!((a(2).horizontal, a(4).horizontal, a(6).horizontal, a(0).horizontal), {
        (HAlign::Right, HAlign::Right, HAlign::Center, HAlign::Left)
    });
    assert_eq!((a(8).vertical, a(16).vertical, a(24).vertical, a(0).vertical), {
        (VAlign::Bottom, VAlign::Baseline, VAlign::Baseline, VAlign::Top)
    });
    assert!(a(1).update_cp && !a(0xFFFF_FFFE).update_cp);
    // TA_CENTER with an odd width puts the extra pixel on the left.
    assert_eq!((a(6).first_origin(70, 9), a(6).first_origin(71, 9), a(6).first_origin(70, 20)), (65, 66, 60));
    assert_eq!((a(6).first_origin(70, -7), a(6).first_origin(70, 0), a(6).first_origin(-3, 5)), (73, 70, -6));
    assert_eq!((a(2).first_origin(90, 12), a(2).first_origin(90, -6), a(0).first_origin(50, -6)), (78, 96, 50));
    // TA_UPDATECP: the other end of the text; not at all when centred.
    assert_eq!((a(1).end_x(50, -6), a(3).end_x(90, -6), a(7).end_x(70, 21), a(1).end_x(50, 12)), (44, 96, 70, 62));
}

/// One call of `eto_dx_cell`: the text colour runs of row 14, the columns
/// of the background box and the current position afterwards.
#[allow(clippy::type_complexity)]
fn dx_case(
    align: u32,
    text: &str,
    x: i32,
    dx: Option<&[i32]>,
    from: Option<(i32, i32)>,
) -> (Vec<(i32, i32)>, Option<(i32, i32)>, (i32, i32)) {
    let (mut g, dc, px) = sans(140, 40);
    g.set_text_align(dc, align);
    if let Some((mx, my)) = from {
        g.move_to(dc, mx, my);
    }
    assert!(g.ext_text_out(dc, x, 8, 0, None, &utf16(text), dx));
    let bk = box_of(&px, 140, BK);
    if let Some(b) = bk {
        assert_eq!((b.1, b.3), (8, 24), "the cell's height");
    }
    (runs(&px, 140, 14, FG), bk.map(|b| (b.0, b.2)), g.current_position(dc).unwrap())
}

#[test]
fn lpdx_may_be_zero_or_negative_and_its_sum_is_the_width() {
    // dx and dx_cell / eto_dx_cell: "BiB" (advances 8, 4, 8) at x=50.
    let left = |dx: Option<&[i32]>| {
        let (r, b, _) = dx_case(0, "BiB", 50, dx, None);
        (r, b.unwrap())
    };
    assert_eq!(left(None), (vec![(51, 57), (59, 61), (63, 69)], (50, 70)));
    assert_eq!(left(Some(&[10, 10, 10])), (vec![(51, 57), (61, 63), (71, 77)], (50, 80)));
    assert_eq!(left(Some(&[0, 0, 0])), (vec![(51, 57)], (50, 57)));
    assert_eq!(left(Some(&[0, 0, 8])), (vec![(51, 57)], (50, 58)));
    assert_eq!(left(Some(&[10, 10, 0])), (vec![(51, 57), (61, 63), (71, 77)], (50, 77)));
    assert_eq!(left(Some(&[-6, 3, 3])), (vec![(45, 47), (48, 57)], (45, 57)));
    assert_eq!(left(Some(&[-10, 2, 2])), (vec![(41, 49), (51, 57)], (41, 57)));
    assert_eq!(left(Some(&[16, -8, 4])), (vec![(51, 57), (59, 65), (67, 69)], (50, 69)));
    assert_eq!(left(Some(&[16, -8, -8])), (vec![(51, 57), (59, 65), (67, 69)], (50, 69)));
    assert_eq!(left(Some(&[5, 5, -20])), (vec![(51, 58), (61, 67)], (50, 67)));
    assert_eq!(left(Some(&[-3, -3, -3])), (vec![(45, 57)], (45, 57)));
    assert_eq!(left(Some(&[-30, 40, 2])), (vec![(21, 23), (51, 57), (61, 67)], (21, 67)));
    for d in [-5, 0, 3] {
        let (r, b, _) = dx_case(0, "B", 50, Some(&[d]), None);
        assert_eq!((r, b), (vec![(51, 57)], Some((50, 57))), "a single 'B' with {d}");
    }
    // TA_RIGHT: the first origin is x - sum.
    let right = |dx: Option<&[i32]>| {
        let (r, b, _) = dx_case(TA_RIGHT, "BiB", 90, dx, None);
        (r, b.unwrap())
    };
    assert_eq!(right(None), (vec![(71, 77), (79, 81), (83, 89)], (70, 90)));
    assert_eq!(right(Some(&[0, 0, 0])), (vec![(91, 97)], (90, 97)));
    assert_eq!(right(Some(&[-10, 2, 2])), (vec![(87, 95), (97, 103)], (87, 103)));
    assert_eq!(right(Some(&[16, -8, 4])), (vec![(79, 85), (87, 93), (95, 97)], (78, 97)));
    // TA_CENTER: floor(x - sum / 2).
    let centre = |text: &str, x: i32, dx: Option<&[i32]>| {
        let (r, b, _) = dx_case(TA_CENTER, text, x, dx, None);
        (r, b.unwrap())
    };
    assert_eq!(centre("BiB", 70, None), (vec![(61, 67), (69, 71), (73, 79)], (60, 80)));
    assert_eq!(centre("&", 70, None).1, (65, 74));
    assert_eq!(centre("&", 71, None).1, (66, 75));
    assert_eq!(centre("&B", 70, None).1, (61, 78));
    assert_eq!(centre("B", 70, Some(&[1])), (vec![(70, 76)], (69, 76)));
    assert_eq!(centre("B", 70, Some(&[3])), (vec![(69, 75)], (68, 75)));
    assert_eq!(centre("BiB", 70, Some(&[7, 7, 7])), (vec![(60, 66), (67, 69), (74, 80)], (59, 80)));
    assert_eq!(centre("BiB", 71, Some(&[7, 7, 7])), (vec![(61, 67), (68, 70), (75, 81)], (60, 81)));
    assert_eq!(centre("BiB", 70, Some(&[0, 0, 0])), (vec![(71, 77)], (70, 77)));
    assert_eq!(centre("BiB", 70, Some(&[-10, 2, 1])), (vec![(64, 72), (74, 80)], (64, 80)));
    // Without TA_UPDATECP the current position is untouched.
    assert_eq!(dx_case(0, "BiB", 50, Some(&[-10, 2, 2]), Some((3, 4))).2, (3, 4));
    // TA_UPDATECP: the arguments are ignored and the position moves by the sum.
    let cp = |align: u32, text: &str, dx: Option<&[i32]>, from: (i32, i32)| {
        let (r, b, p) = dx_case(align | TA_UPDATECP, text, 999, dx, Some(from));
        (r, b.unwrap(), p)
    };
    assert_eq!(cp(0, "BiB", Some(&[0, 0, 0]), (50, 8)), (vec![(51, 57)], (50, 57), (50, 8)));
    assert_eq!(cp(0, "BiB", Some(&[-10, 2, 2]), (50, 8)), (vec![(41, 49), (51, 57)], (41, 57), (44, 8)));
    assert_eq!(cp(0, "BiB", Some(&[16, -8, 4]), (50, 8)).2, (62, 8));
    assert_eq!(cp(TA_RIGHT, "BiB", Some(&[-10, 2, 2]), (90, 8)), (vec![(87, 95), (97, 103)], (87, 103), (96, 8)));
    assert_eq!(cp(TA_CENTER, "BiB", Some(&[7, 7, 7]), (70, 8)).2, (70, 8));
    assert_eq!(cp(TA_CENTER, "&", None, (70, 8)), (vec![(66, 73)], (65, 74), (70, 8)));
    // 4 alone aligns right, also for the current position.
    assert_eq!(cp(4, "BiB", Some(&[-10, 2, 2]), (90, 8)).2, (96, 8));
}

#[test]
fn the_lpdx_cell_holds_the_sum_and_the_ink() {
    // dx_cell, as columns.
    assert_eq!(dx_cell_columns(50, 30, Some((51, 77))), (50, 80));
    assert_eq!(dx_cell_columns(50, 0, Some((51, 57))), (50, 57));
    assert_eq!(dx_cell_columns(50, 8, Some((51, 57))), (50, 58));
    assert_eq!(dx_cell_columns(50, -6, Some((41, 57))), (41, 57));
    assert_eq!(dx_cell_columns(50, 12, Some((21, 67))), (21, 67));
    assert_eq!(dx_cell_columns(50, -10, Some((51, 67))), (50, 67));
    // No ink (blanks): only the sum, and nothing for a sum of zero or below.
    assert_eq!(dx_cell_columns(50, 12, None), (50, 62));
    assert_eq!(dx_cell_columns(50, -12, None), (50, 50));
    // In TRANSPARENT mode nothing but the glyphs changes.
    let (mut g, dc, px) = sans(140, 40);
    g.set_bk_mode(dc, TRANSPARENT);
    assert!(g.ext_text_out(dc, 50, 8, 0, None, &utf16("BiB"), Some(&[-10, 2, 2])));
    assert!(where_is(&px, 140, BK).is_empty());
    assert_eq!(runs(&px, 140, 14, FG), [(41, 49), (51, 57)]);
    // Blanks with distances: the cell is the sum.
    let (mut g, dc, px) = sans(140, 40);
    assert!(g.ext_text_out(dc, 50, 8, 0, None, &utf16("  "), Some(&[9, 9])));
    assert_eq!(box_of(&px, 140, BK), Some((50, 8, 68, 24)));
    // ETO_CLIPPED limits the cell and the glyphs alike.
    let (mut g, dc, px) = sans(140, 40);
    let r = Rect::new(44, 10, 55, 20);
    assert!(g.ext_text_out(dc, 50, 8, ETO_CLIPPED, Some(r), &utf16("BiB"), Some(&[-10, 2, 2])));
    assert_eq!(changed(&px, 140), Some((44, 10, 55, 20)));
}

#[test]
fn option_bits_outside_wingdi_are_ignored_unless_beside_glyph_index_or_ignorelanguage() {
    // option_bits / eto_options.
    assert_eq!(ETO_DEFINED, 0x13C96);
    for bit in 0..32 {
        let o = 1u32 << bit;
        assert_eq!(eto_options_ok(o, false), o != ETO_PDY, "bit {o:#x} alone");
        assert!(eto_options_ok(o, true), "bit {o:#x} with lpDx");
    }
    // Beside ETO_GLYPH_INDEX or ETO_IGNORELANGUAGE every stray bit fails.
    for base in [ETO_GLYPH_INDEX, ETO_IGNORELANGUAGE] {
        let failing = (0..32).map(|b| 1u32 << b).filter(|&b| b != ETO_PDY && !eto_options_ok(base | b, false));
        assert_eq!(failing.fold(0, |m, b| m | b), 0xFFFE_C369, "beside {base:#x}");
    }
    for base in [0x1u32, 0x8000_0000] {
        let failing = (0..32).map(|b| 1u32 << b).filter(|&b| b != ETO_PDY && !eto_options_ok(base | b, false));
        assert_eq!(failing.fold(0, |m, b| m | b), 0x1010, "beside {base:#x}");
    }
    for base in [ETO_OPAQUE, ETO_RTLREADING] {
        assert!((0..32).map(|b| 1u32 << b).all(|b| b == ETO_PDY || eto_options_ok(base | b, false)), "{base:#x}");
    }
    assert!(!eto_options_ok(0xFFFF_FFFF, false), "ETO_PDY without lpDx");
    assert!(!eto_options_ok(0xFFFF_FFFF, true), "stray bits beside ETO_GLYPH_INDEX");
    assert!(!eto_options_ok(0xFFFF_DFFF, false) && !eto_options_ok(0x11C97, false) && !eto_options_ok(0x2001, false));
    assert!(eto_options_ok(0x1010, false) && eto_options_ok(0x11C96, false) && eto_options_ok(0x13C96, true));
    assert!(eto_options_ok(0xFFFE_C369, false) && eto_options_ok(0xFFFF_0000, false));
    assert!(eto_options_ok(0x2001, true) && eto_options_ok(0xFFFE_E369, true), "ETO_PDY and stray bits with lpDx");

    // Drawn: "gi" at (6,5), TRANSPARENT.
    let (mut g, dc, mut px) = sans(100, 30);
    g.set_bk_mode(dc, TRANSPARENT);
    let gi = utf16("gi");
    let mut go = |g: &mut Gdi, options: u32, dx: Option<&[i32]>| {
        px.fill(BG);
        let r = g.try_ext_text_out(dc, 6, 5, options, None, &gi, dx);
        (r, changed(&px, 100), ink(&px, 100, BG).len())
    };
    let plain = go(&mut g, 0, None);
    assert_eq!(plain, (Ok(()), Some((7, 5, 19, 21)), 104));
    let glyphs = go(&mut g, ETO_GLYPH_INDEX, None);
    assert_eq!((glyphs.0, glyphs.1), (Ok(()), Some((7, 5, 25, 17))), ".notdef twice");
    for bit in 0..32 {
        let o = 1u32 << bit;
        let want = match o {
            ETO_GLYPH_INDEX => glyphs,
            ETO_PDY => (Err(TextOutError::Refused), None, 0),
            _ => plain,
        };
        assert_eq!(go(&mut g, o, None), want, "option bit {o:#x}");
    }
    for o in [0xFFFF_FFFFu32, 0xFFFF_DFFF, 0x11C97, 0x2001] {
        assert_eq!(go(&mut g, o, None), (Err(TextOutError::Refused), None, 0), "options {o:#x}");
    }
    // Every defined option but ETO_PDY draws what ETO_GLYPH_INDEX alone draws.
    assert_eq!(go(&mut g, 0x1010, None), glyphs);
    assert_eq!(go(&mut g, 0x11C96, None), glyphs);
    assert_eq!(go(&mut g, 0xFFFE_C369, None), plain);
    assert_eq!(go(&mut g, 0xFFFF_0000, None), plain);
    // Not a DC: the one failure with an error code.
    assert_eq!(g.try_ext_text_out(0, 6, 5, 0, None, &gi, None), Err(TextOutError::BadDc));
    assert_eq!(g.try_ext_text_out(0, 6, 5, 0xFFFF_FFFF, None, &gi, None), Err(TextOutError::BadDc));
    let font = u64::from(g.stock(13));
    assert_eq!(g.try_ext_text_out(font, 6, 5, 0, None, &gi, None), Err(TextOutError::BadDc));
    // A failing call does not fill the ETO_OPAQUE rectangle either.
    px.fill(BG);
    assert!(!g.ext_text_out(dc, 6, 5, ETO_PDY | ETO_OPAQUE, Some(Rect::new(0, 0, 50, 20)), &gi, None));
    assert_eq!(changed(&px, 100), None);
}

#[test]
fn eto_pdy_takes_pairs_and_moves_the_following_characters_across_the_line() {
    // pdy / eto_options: "gi" at (6,5).
    let (mut g, dc, mut px) = sans(100, 30);
    g.set_bk_mode(dc, TRANSPARENT);
    let gi = utf16("gi");
    let mut go = |g: &mut Gdi, options: u32, dx: &[i32]| {
        px.fill(BG);
        assert!(g.ext_text_out(dc, 6, 5, options, None, &gi, Some(dx)));
        (changed(&px, 100).unwrap(), ink(&px, 100, BG).len())
    };
    let plain = go(&mut g, 0, &[20, 7]);
    assert_eq!(plain, ((7, 5, 29, 21), 104));
    assert_eq!(go(&mut g, ETO_PDY, &[20, 0, 7, 0]), plain, "no rise: the drawing of lpDx 20,7");
    // A positive second value moves what follows up (clipped by the surface), a negative one down.
    assert_eq!(go(&mut g, ETO_PDY, &[20, 6, 7, 0]), ((7, 0, 29, 21), 102));
    assert_eq!(go(&mut g, ETO_PDY, &[20, -4, 7, 9]), ((7, 9, 29, 21), 104));
    assert_eq!(go(&mut g, ETO_PDY, &[0, 10, 0, 0]), ((7, 0, 15, 21), 94));
    // TA_UPDATECP: by the sum of the first values and against the sum of the second ones.
    g.set_text_align(dc, TA_UPDATECP);
    g.move_to(dc, 10, 20);
    assert!(g.ext_text_out(dc, 0, 0, ETO_PDY, None, &gi, Some(&[20, 6, 7, -2])));
    assert_eq!(g.current_position(dc), Some((37, 16)));
    g.move_to(dc, 10, 20);
    assert!(g.ext_text_out(dc, 0, 0, 0, None, &gi, Some(&[20, 6])));
    assert_eq!(g.current_position(dc), Some((36, 20)), "without ETO_PDY the values are both distances");
    g.set_text_align(dc, 0);
    // OPAQUE: one rectangle around the cells of all the characters, each at its own height.
    let (mut g, dc, px) = sans(100, 40);
    assert!(g.ext_text_out(dc, 6, 10, ETO_PDY, None, &gi, Some(&[20, 6, 7, 0])));
    // (The right edge is one pixel farther than that of the same lpDx without the option.)
    assert_eq!(box_of(&px, 100, BK), Some((6, 4, 34, 26)));
    let (mut g, dc, px) = sans(100, 40);
    assert!(g.ext_text_out(dc, 6, 10, ETO_PDY, None, &gi, Some(&[20, -5, 7, 0])));
    assert_eq!(box_of(&px, 100, BK), Some((6, 10, 34, 31)));
}

#[test]
fn eto_pdy_cell_is_one_pixel_wider_on_the_right_than_the_plain_lpdx_cell() {
    // eto_pdy_cell, eto_pdy_move: 'g' (advance 10, ink to column 69 at x=60), 'i' (advance 4).
    let cell_of = |text: &str, x: i32, align: u32, options: u32, dx: &[i32]| {
        let (mut g, dc, px) = sans(160, 60);
        g.set_text_align(dc, align);
        assert!(g.ext_text_out(dc, x, 30, ETO_OPAQUE.min(2) | options, None, &utf16(text), Some(dx)));
        box_of(&px, 160, BK).map(|b| (b.0, b.2))
    };
    assert_eq!(cell_of("g", 60, 0, 0, &[14]), Some((60, 74)));
    assert_eq!(cell_of("g", 60, 0, ETO_PDY, &[14, 0]), Some((60, 75)));
    for dx in [0, -9, 9] {
        assert_eq!(cell_of("g", 60, 0, 0, &[dx]), Some((60, 69)), "{dx}");
        assert_eq!(cell_of("g", 60, 0, ETO_PDY, &[dx, 0]), Some((60, 70)), "{dx}");
    }
    assert_eq!(cell_of("i", 60, 0, 0, &[1]), Some((60, 63)));
    assert_eq!(cell_of("i", 60, 0, ETO_PDY, &[1, 0]), Some((60, 64)));
    // The same from the first origin that TA_RIGHT and TA_CENTER give.
    assert_eq!(cell_of("g", 100, TA_RIGHT, 0, &[14]), Some((86, 100)));
    assert_eq!(cell_of("g", 100, TA_RIGHT, ETO_PDY, &[14, 0]), Some((86, 101)));
    assert_eq!(cell_of("g", 80, TA_CENTER, 0, &[14]), Some((73, 87)));
    assert_eq!(cell_of("g", 80, TA_CENTER, ETO_PDY, &[14, 0]), Some((73, 88)));
    // gAi at (60,30): the background is 60,25 to 92,50.
    let (mut g, dc, px) = sans(160, 80);
    assert!(g.ext_text_out(dc, 60, 30, ETO_OPAQUE | ETO_PDY, None, &utf16("gAi"), Some(&[14, 5, 14, -9, 3, 0])));
    assert_eq!(box_of(&px, 160, BK), Some((60, 25, 92, 50)));
}

#[test]
fn eto_pdy_with_update_cp_moves_the_y_by_all_second_values_for_every_alignment() {
    // eto_pdy_cp: from (80,30) with (4,3) (5,3) (6,-1).
    let end = |align: u32, dx: &[i32]| {
        let (mut g, dc, _px) = sans(160, 60);
        g.set_text_align(dc, align | TA_UPDATECP);
        g.move_to(dc, 80, 30);
        assert!(g.ext_text_out(dc, 0, 0, ETO_PDY, None, &utf16("gAi"), Some(dx)));
        g.current_position(dc)
    };
    let a = [4, 3, 5, 3, 6, -1];
    assert_eq!(end(0, &a), Some((95, 25)));
    assert_eq!(end(TA_RIGHT, &a), Some((65, 25)));
    assert_eq!(end(TA_CENTER, &a), Some((80, 25)));
    let b = [14, 8, 14, 8, 3, 8];
    assert_eq!(end(0, &b), Some((111, 6)));
    assert_eq!(end(TA_RIGHT, &b), Some((49, 6)));
    assert_eq!(end(TA_CENTER, &b), Some((80, 6)));
    assert_eq!(end(TA_CENTER | TA_BOTTOM, &b), Some((80, 6)), "the vertical alignment does not matter");
}

#[test]
fn long_strings_and_empty_ones() {
    // count / eto_options: OPAQUE, 'B' at (6,5) on a surface 100 wide.
    let (mut g, dc, mut px) = sans(100, 30);
    let long = vec![u16::from(b'B'); 20000];
    for (n, right) in [(5usize, 46), (8192, 100), (8193, 100), (9000, 100), (20000, 100)] {
        px.fill(BG);
        assert!(g.ext_text_out(dc, 6, 5, 0, None, &long[..n], None));
        assert_eq!(changed(&px, 100), Some((6, 5, right, 21)), "{n} characters");
    }
    // TA_RIGHT at x: from the surface's left edge to x.
    px.fill(BG);
    g.set_text_align(dc, TA_RIGHT);
    assert!(g.ext_text_out(dc, 90, 5, 0, None, &long, None));
    assert_eq!(changed(&px, 100), Some((0, 5, 90, 21)));
    g.set_text_align(dc, 0);
    // A count of 0 draws nothing in either mode.
    px.fill(BG);
    assert!(g.ext_text_out(dc, 6, 5, 0, None, &[], None));
    assert!(g.ext_text_out(dc, 6, 5, 0, None, &[], Some(&[])));
    assert_eq!(changed(&px, 100), None);
}

// ----- GetTextExtentExPointW

#[test]
fn extent_ex_checks_come_in_the_order_of_the_sheet() {
    use DcArg::*;
    use ExtentExCheck::*;
    let c = extent_ex_check;
    // The DC and lpSize.
    assert_eq!(c(Null, false, false, 6, 35, true), Fail(None));
    assert_eq!(c(Null, false, false, 0, 35, true), Fail(None), "also with a count of 0");
    assert_eq!(c(Null, false, false, 6, -1, true), Fail(None));
    assert_eq!(c(NotDc, false, false, 6, 35, true), Fail(Some(ERROR_INVALID_HANDLE)));
    assert_eq!(c(Dc, true, false, 6, 35, true), Fail(None));
    assert_eq!(c(Dc, true, false, 0, 35, true), Fail(None), "a NULL lpSize with a count of 0");
    assert_eq!(c(Dc, true, false, 6, 35, false), Fail(None), "a NULL lpSize without lpnFit");
    assert_eq!(c(Dc, true, true, 6, 35, false), Fail(None));
    // A NULL string is an empty string whatever the count and the maximum are.
    for (count, max) in [(0, 35), (3, 35), (3, -1), (-1, 35), (-6, -7)] {
        assert_eq!(c(Dc, false, true, count, max, true), Empty, "NULL string, count {count}, max {max}");
    }
    // A count of 0, also with a negative maximum.
    assert_eq!(c(Dc, false, false, 0, 35, true), Empty);
    assert_eq!(c(Dc, false, false, 0, -1, true), Empty);
    assert_eq!(c(Dc, false, false, 0, -1, false), Empty);
    // A negative count, with or without lpnFit.
    for count in [-1, -6, i32::MIN] {
        assert_eq!(c(Dc, false, false, count, 35, true), Fail(None));
        assert_eq!(c(Dc, false, false, count, 35, false), Fail(None));
    }
    // A negative maximum only counts when lpnFit is given.
    for max in [-1, -7, i32::MIN] {
        assert_eq!(c(Dc, false, false, 6, max, true), Fail(None), "max {max} with lpnFit");
        assert_eq!(c(Dc, false, false, 6, max, false), Measure, "max {max} without lpnFit");
    }
    for max in [0, 35, 59, 60, i32::MAX] {
        assert_eq!(c(Dc, false, false, 6, max, true), Measure);
        assert_eq!(c(Dc, false, false, 6, max, false), Measure);
    }
}

#[test]
fn extent_ex_fit_and_running_extents() {
    // fit: PolyTest Sans -16, "AB gi": 12 20 26 36 40.
    let (g, dc, _px) = sans(8, 8);
    let ((w, h), _, running) = g.text_extent_ex(dc, &utf16("AB gi"), 0).unwrap();
    assert_eq!(((w, h), running.as_slice()), ((40, 16), &[12, 20, 26, 36, 40][..]));
    for (max, fit) in [(0, 0), (11, 0), (12, 1), (19, 1), (20, 2), (25, 2), (26, 3), (35, 3), (36, 4), (39, 4)] {
        assert_eq!(extent_ex_fit(&running, max, true), (fit, fit), "max {max}");
        // Without lpnFit the maximum plays no part: every entry is written.
        assert_eq!(extent_ex_fit(&running, max, false).1, 5, "max {max}, no lpnFit");
    }
    for max in [40, 41, 0x7FFF_FFFF] {
        assert_eq!(extent_ex_fit(&running, max, true), (5, 5), "max {max}");
    }
    assert_eq!(extent_ex_fit(&running, -1, false).1, 5, "a negative maximum without lpnFit");
    assert_eq!(extent_ex_fit(&[], 35, true), (0, 0));
    // narrow: one character that does not fit; the size is the character's.
    let ((w, h), _, running) = g.text_extent_ex(dc, &utf16("W"), 3).unwrap();
    assert_eq!(((w, h), extent_ex_fit(&running, 3, true)), ((16, 16), (0, 0)));
    assert!(g.text_extent_ex(0, &utf16("W"), 3).is_none());
}

#[test]
fn extent_ex_uses_the_advances_and_height_of_the_font() {
    // sizes / gte_ex_long.
    let mut g = Gdi::new();
    for d in [&tfont::SANS, &tfont::TALL, &tfont::MONO] {
        assert!(g.add_font_mem(&tfont::build(d)).is_some());
    }
    let dc = u64::from(g.create_compatible_dc(0));
    select(&mut g, dc, &lf_full("PolyTest Tall", -32, 400, false, 3));
    let (size, _, running) = g.text_extent_ex(dc, &utf16("WAM-.Bg"), 0).unwrap();
    assert_eq!((size, running.as_slice()), ((136, 38), &[32, 56, 80, 92, 100, 116, 136][..]));
    for (max, fit) in [(0, 0), (31, 0), (32, 1), (55, 1), (56, 2), (80, 3), (99, 4), (100, 5), (135, 6), (136, 7)] {
        assert_eq!(extent_ex_fit(&running, max, true).0, fit, "max {max}");
    }
    select(&mut g, dc, &lf_full("PolyTest Mono", -16, 400, false, 3));
    let (size, _, running) = g.text_extent_ex(dc, &utf16("AB gi."), 0).unwrap();
    assert_eq!((size, running.as_slice()), ((60, 16), &[10, 20, 30, 40, 50, 60][..]));
    assert_eq!((extent_ex_fit(&running, 35, true), extent_ex_fit(&running, 59, true)), ((3, 3), (5, 5)));
    // A string of 24 characters is totalled the same way.
    select(&mut g, dc, &lf_full("PolyTest Sans", -16, 400, false, 3));
    let (size, _, running) = g.text_extent_ex(dc, &utf16("Ai B.gAi B.gAi B.gAi B.g"), 0).unwrap();
    assert_eq!((size, running.len(), running[5], running[12], running[23]), ((176, 16), 24, 44, 100, 176));
    for (max, fit) in [(43, 5), (44, 6), (100, 13), (175, 23), (176, 24), (177, 24)] {
        assert_eq!(extent_ex_fit(&running, max, true).0, fit, "max {max}");
    }
}

// ----- RemoveFontResourceExW

#[test]
fn a_font_file_is_known_by_where_its_name_leads() {
    // spelling / rfr_spellings.
    let key = |s: &str| fonttab::file_key(&utf16(s));
    let written = key("C:\\Temp\\poly\\Named.ttf");
    for other in [
        "C:\\TEMP\\POLY\\NAMED.TTF",
        "c:\\temp\\poly\\named.ttf",
        "C:/Temp/poly/Named.ttf",
        "C:\\Temp\\poly\\nodir\\..\\Named.ttf",
        "C:\\Temp\\nodir\\..\\poly\\nodir\\..\\Named.ttf",
        "C:\\Temp/poly\\NODIR/../named.TTF",
        "\\\\?\\C:\\Temp\\poly\\Named.ttf",
        "C:\\Temp\\poly\\Named.ttf.",
        "C:\\Temp\\poly\\.\\Named.ttf",
    ] {
        assert_eq!(key(other), written, "{other}");
    }
    // Other files.
    for other in ["C:\\Temp\\poly\\Named2.ttf", "C:\\Temp\\Named.ttf", "D:\\Temp\\poly\\Named.ttf", "C:\\Temp\\poly"] {
        assert_ne!(key(other), written, "{other}");
    }
    // `..` never climbs above the drive.
    assert_eq!(key("C:\\..\\..\\Named.ttf"), key("C:\\Named.ttf"));

    // Adds under different spellings are adds of one file and are counted together.
    let font = tfont::build(&tfont::MONO);
    let mut t = FontTable::new();
    let name = utf16("PolyTest Mono");
    let p = |s: &str| utf16(s);
    assert!(t.add_file(&p("C:\\Temp\\poly\\Named.ttf"), FR_PRIVATE, font.clone()));
    assert!(t.add_file(&p("C:\\TEMP\\POLY\\NAMED.TTF"), FR_PRIVATE, font.clone()));
    assert!(t.remove_file(&p("c:\\temp\\poly\\named.ttf"), FR_PRIVATE));
    assert!(t.find(&name, false, false).is_some(), "the family stays until the last remove");
    assert!(t.remove_file(&p("C:/Temp/poly/Named.ttf"), FR_PRIVATE));
    assert!(t.find(&name, false, false).is_none());
    assert!(!t.remove_file(&p("C:\\Temp\\poly\\Named.ttf"), FR_PRIVATE), "a third remove");
    // Added respelled, removed as written.
    for added in ["C:/Temp/poly/Named.ttf", "C:\\Temp\\poly\\nodir\\..\\Named.ttf", "c:\\temp\\POLY\\named.ttf"] {
        assert!(t.add_file(&p(added), FR_PRIVATE, font.clone()));
        assert!(t.remove_file(&p("C:\\Temp\\poly\\Named.ttf"), FR_PRIVATE), "{added}");
        assert!(t.is_empty());
    }
    // A copy of the file under another name was never added.
    assert!(t.add_file(&p("C:\\Temp\\poly\\Named.ttf"), FR_PRIVATE, font.clone()));
    assert!(!t.remove_file(&p("C:\\Temp\\poly\\Copy.ttf"), FR_PRIVATE));
    assert_eq!(t.len(), 1);
}

#[test]
fn the_flags_of_a_remove_are_exactly_those_of_the_add() {
    // flags / rfr_selected.
    let font = tfont::build(&tfont::MONO);
    let mut t = FontTable::new();
    let path = utf16("C:\\Temp\\Named.ttf");
    let name = utf16("PolyTest Mono");
    let both = FR_PRIVATE | FR_NOT_ENUM;
    assert!(t.add_file(&path, both, font.clone()));
    for flags in [FR_PRIVATE, FR_NOT_ENUM, 0] {
        assert!(!t.remove_file(&path, flags), "flags {flags:#x}");
        assert!(t.find(&name, false, false).is_some(), "the font stays");
    }
    assert!(t.remove_file(&path, both));
    assert!(!t.remove_file(&path, both), "once more");
    assert!(t.find(&name, false, false).is_none());
    // An FR_PRIVATE add is not removed with flags 0.
    assert!(t.add_file(&path, FR_PRIVATE, font.clone()));
    assert!(!t.remove_file(&path, 0));
    assert!(t.remove_file(&path, FR_PRIVATE));
    // Adds of one file with different flags are counted apart.
    assert!(t.add_file(&path, FR_PRIVATE, font.clone()));
    assert!(t.add_file(&path, both, font.clone()));
    assert!(t.remove_file(&path, FR_PRIVATE));
    assert!(!t.remove_file(&path, FR_PRIVATE), "a second remove with the same flags");
    assert!(t.find(&name, false, false).is_some(), "the family stays until both are gone");
    assert!(t.remove_file(&path, both));
    assert!(!t.remove_file(&path, both));
    assert!(t.find(&name, false, false).is_none());
    // Each add needs its own remove (rfr_counts).
    assert!(t.add_file(&path, FR_PRIVATE, font.clone()));
    assert!(t.add_file(&path, FR_PRIVATE, font));
    assert!(t.remove_file(&path, FR_PRIVATE));
    assert!(t.find(&name, false, false).is_some());
    assert!(t.remove_file(&path, FR_PRIVATE));
    assert!(!t.remove_file(&path, FR_PRIVATE));
}

#[test]
fn remove_names_that_lead_to_no_file() {
    // errors / rfr_names.
    let c = |s: &str, flags: u32| fonttab::remove_name_check(&utf16(s), flags);
    // An empty path, or one of blanks only, and unknown flag bits: 87.
    for name in ["", " ", "    "] {
        assert_eq!(c(name, FR_PRIVATE), NameCheck::Error(87), "{name:?}");
    }
    for flags in [1, 0x11, 0x70, 0xFFFF, 0x8000_0010, 0x40, 0xFFFF_FFFF] {
        assert_eq!(c("C:\\Temp\\Named.ttf", flags), NameCheck::Error(87), "flags {flags:#x}");
    }
    for flags in [0, FR_PRIVATE, FR_NOT_ENUM, FR_PRIVATE | FR_NOT_ENUM] {
        assert_eq!(c("C:\\Temp\\Named.ttf", flags), NameCheck::Look, "flags {flags:#x}");
    }
    // Wildcards are neither expanded nor refused as bad names: a file that is not there.
    for name in ["C:\\Temp\\*.ttf", "C:\\Temp\\Name?.ttf", "C:\\Temp\\*", "*.ttf", "?", "\\\\?\\C:\\Temp\\*.ttf"] {
        assert_eq!(c(name, FR_PRIVATE), NameCheck::Error(2), "{name}");
    }
    // What the name leads to: nothing is error 2, and so is a file's path with a backslash after
    // it; a file or a directory that is there is looked for among the added fonts.
    use fonttab::Leads::{Directory, File, Nowhere};
    let l = |s: &str, leads: fonttab::Leads| fonttab::remove_lookup_check(&utf16(s), leads);
    for name in ["C:\\Temp\\Other.ttf", "C:\\Temp\\nodir\\Named.ttf", "C:\\Temp\\N", "C:\\Temp\\Named.ttf\\"] {
        assert_eq!(l(name, Nowhere), NameCheck::Error(2), "{name}");
    }
    assert_eq!(l("C:\\Temp\\Named.ttf\\", File), NameCheck::Error(2));
    assert_eq!(l("C:/Temp/Named.ttf/", File), NameCheck::Error(2));
    assert_eq!(l("C:\\Temp\\Named.ttf", File), NameCheck::Look);
    // A directory, with or without a backslash at its end, is no added font: no error code.
    for name in ["C:\\Temp", "C:\\Temp\\", "C:/Temp/", "C:\\", "C:"] {
        assert_eq!(c(name, FR_PRIVATE), NameCheck::Look, "{name}");
        assert_eq!(l(name, Directory), NameCheck::Look, "{name}");
    }
    // Names that are looked up: the \\?\ prefix is no wildcard.
    for name in ["C:\\Temp", "Named.ttf", "\\\\?\\C:\\Temp\\Named.ttf", " Named.ttf", "C:\\Temp\\no such\\Named.ttf"] {
        assert_eq!(c(name, FR_PRIVATE), NameCheck::Look, "{name}");
    }
    // Longer than MAX_PATH, and two names joined with '|': FALSE without an error code.
    let long: String = core::iter::repeat_n('a', 300).collect();
    assert_eq!(c(&long, FR_PRIVATE), NameCheck::Refused);
    assert_eq!(c("C:\\Temp\\A.ttf|C:\\Temp\\B.ttf", FR_PRIVATE), NameCheck::Refused);
    // Bad flags come first.
    assert_eq!(c("C:\\Temp\\*.ttf", 1), NameCheck::Error(87));
}

#[test]
fn a_font_object_does_not_hold_on_to_the_file() {
    // in_use / rfr_selected.
    let mut g = Gdi::new();
    let font = tfont::build(&tfont::MONO);
    let path = utf16("C:\\Temp\\Named.ttf");
    let dc = u64::from(g.create_compatible_dc(0));
    let dc2 = u64::from(g.create_compatible_dc(0));
    assert!(g.add_font_file(&path, FR_PRIVATE, font.clone()));
    let lf = lf_full("PolyTest Mono", -16, 400, false, 3);
    let selected = select(&mut g, dc, &lf);
    let before = u64::from(g.create_font(&lf));
    let is_mono = |g: &Gdi, dc: u64| {
        (g.text_face(dc).unwrap() == utf16("PolyTest Mono"), g.text_extent(dc, &utf16("AB")).unwrap().0 == 20)
    };
    assert_eq!(is_mono(&g, dc), (true, true));
    // Removed while a font of the family is selected: every font object maps to another font at once.
    assert!(g.remove_font_file(&path, FR_PRIVATE));
    assert_eq!(is_mono(&g, dc), (false, false), "the font that was selected all along");
    g.select_object(dc2, before).unwrap();
    assert_eq!(is_mono(&g, dc2), (false, false), "made before the remove, selected afterwards");
    select(&mut g, dc2, &lf);
    assert_eq!(is_mono(&g, dc2), (false, false), "made afterwards");
    assert!(!g.remove_font_file(&path, FR_PRIVATE), "a second remove");
    // Added again: the same objects are the family again.
    assert!(g.add_font_file(&path, FR_PRIVATE, font));
    assert_eq!(is_mono(&g, dc), (true, true), "the font that stayed selected");
    g.select_object(dc2, before).unwrap();
    assert_eq!(is_mono(&g, dc2), (true, true));
    assert!(g.remove_font_file(&utf16("c:/temp/NAMED.TTF"), FR_PRIVATE));
    assert_eq!(is_mono(&g, dc), (false, false));
    let _ = selected;
}

// ----- DrawTextW

fn draw(g: &mut Gdi, dc: u64, s: &str, r: Rect, fmt: u32) -> (i32, Rect) {
    let mut r = r;
    let n = g.draw_text(dc, &utf16(s), &mut r, fmt);
    (n, r)
}

/// Draws in OPAQUE mode on a fresh surface: the return value, the
/// rectangle afterwards and the box of the background colour.
#[allow(clippy::type_complexity)]
fn cell(s: &str, r: Rect, fmt: u32) -> (i32, Rect, Option<(i32, i32, i32, i32)>) {
    let (mut g, dc, px) = sans(240, 100);
    let (n, after) = draw(&mut g, dc, s, r, fmt);
    (n, after, box_of(&px, 240, BK))
}

/// The ink of each 16-pixel line band below `top`, drawn in TRANSPARENT mode.
fn band_ink(s: &str, r: Rect, fmt: u32) -> Vec<Option<(i32, i32)>> {
    let (mut g, dc, px) = sans(240, 100);
    g.set_bk_mode(dc, TRANSPARENT);
    draw(&mut g, dc, s, r, fmt);
    (0..4)
        .map(|i| {
            let y = r.top + 16 * i;
            let p: Vec<(i32, i32)> = where_is(&px, 240, FG).into_iter().filter(|q| q.1 >= y && q.1 < y + 16).collect();
            if p.is_empty() { None } else { Some((bounds(&p).0, bounds(&p).2)) }
        })
        .collect()
}

const WIDE: Rect = Rect { left: 10, top: 10, right: 230, bottom: 50 };

#[test]
fn dt_tabstop_makes_bits_8_to_15_the_tab_length_and_nothing_else() {
    // tabstop / dt_pf_tabstop: PolyTest Sans, average width 8.
    let ts = |n: u32| DT_TABSTOP | n << 8;
    let x = DT_EXPANDTABS;
    let plain = (16, WIDE, Some((10, 10, 30, 26)));
    assert_eq!(cell("A\tB", WIDE, 0), plain, "a tab without DT_EXPANDTABS has no width");
    assert_eq!(cell("A\tB", WIDE, x), (16, WIDE, Some((10, 10, 82, 26))));
    // DT_TABSTOP alone does not expand tabs.
    assert_eq!(cell("A\tB", WIDE, DT_TABSTOP), plain);
    assert_eq!(cell("A\tB", WIDE, DT_TABSTOP | x), (16, WIDE, Some((10, 10, 82, 26))), "a length of 0 means 8");
    for (n, b_at) in [(1u32, 16), (2, 16), (3, 24), (4, 32), (5, 40), (8, 64), (10, 80), (12, 96), (16, 128), (20, 160)]
    {
        assert_eq!(cell("A\tB", WIDE, ts(n)), plain, "length {n} without DT_EXPANDTABS");
        assert_eq!(cell("A\tB", WIDE, ts(n) | x), (16, WIDE, Some((10, 10, 10 + b_at + 8, 26))), "length {n}");
    }
    // Always forward, from the start of the line; every line has its own stops.
    assert_eq!(cell("A\tB\tA\tB", WIDE, ts(3) | x).2, Some((10, 10, 90, 26)));
    assert_eq!(cell("\tA", WIDE, ts(3) | x).2, Some((10, 10, 46, 26)));
    assert_eq!(cell("\t\tA", WIDE, ts(3) | x).2, Some((10, 10, 70, 26)));
    assert_eq!(cell("AAA\tB", WIDE, ts(3) | x).2, Some((10, 10, 66, 26)), "36 wide before the tab");
    assert_eq!(cell("AA\tB", WIDE, ts(3) | x).2, Some((10, 10, 66, 26)), "text that ends on a stop goes to the next");
    assert_eq!(cell("A\tB\nAB\tB", WIDE, ts(3) | x), (32, WIDE, Some((10, 10, 42, 42))));
    assert_eq!(cell("A\tB", WIDE, ts(3) | x | DT_SINGLELINE).2, Some((10, 10, 42, 26)));
    // DT_RIGHT aligns the expanded line.
    assert_eq!(cell("A\tB", WIDE, ts(3) | x | DT_RIGHT).2, Some((198, 10, 230, 26)));
    assert_eq!(cell("A\tB", WIDE, ts(255) | x).2, Some((10, 10, 230, 26)), "the cell reaches the rectangle's edge");
    // The bits are not the flags: 1 still clips (DT_NOCLIP) ...
    let small = Rect::new(10, 10, 20, 15);
    assert_eq!(cell("AB\tB", small, x), (16, small, Some((10, 10, 12, 15))));
    assert_eq!(cell("AB\tB", small, ts(1) | x), (16, small, Some((10, 10, 12, 15))));
    assert_eq!(cell("AB\tB", small, ts(3) | x), (16, small, Some((10, 10, 12, 15))));
    assert_eq!(cell("AB\tB", small, DT_NOCLIP | x).2, Some((10, 10, 82, 26)));
    // ... 2 returns the plain height (DT_EXTERNALLEADING) ...
    assert_eq!(cell("A\tB", WIDE, ts(2) | x).0, 16);
    assert_eq!(cell("AB", WIDE, DT_EXTERNALLEADING).0, 18);
    // ... 4 draws and leaves the rectangle alone (DT_CALCRECT) ...
    assert_eq!(cell("AB", WIDE, DT_CALCRECT), (16, Rect::new(10, 10, 30, 26), None));
    // ... 8 still takes the '&' as a prefix (DT_NOPREFIX) ...
    assert_eq!(cell("&A\tB", WIDE, ts(8) | x).2, Some((10, 10, 82, 26)));
    assert_eq!(band_ink("&A\tB", WIDE, ts(8) | x)[0], Some((10, 81)), "the underline starts at the cell's edge");
    assert_eq!(band_ink("&A\tB", WIDE, DT_NOPREFIX | x)[0], Some((11, 81)), "the '&' is drawn");
    // ... 32 does not hold back a partial line (DT_EDITCONTROL) ...
    let short = Rect::new(10, 10, 230, 34);
    assert_eq!(cell("AB\nAB\nAB", short, ts(32)).0, 32);
    assert_eq!(cell("AB\nAB\nAB", short, ts(1)).0, 32);
    assert_eq!(cell("AB\nAB\nAB", short, DT_EDITCONTROL).0, 16);
    // ... 64 and 128 add no dots (DT_PATH_ELLIPSIS, DT_END_ELLIPSIS).
    let narrow = Rect::new(10, 10, 70, 50);
    assert_eq!(band_ink("AABBAABBAB", narrow, ts(128))[0], Some((12, 70)));
    assert_eq!(band_ink("AABBAABBAB", narrow, DT_END_ELLIPSIS)[0], Some((12, 61)));
    assert_eq!(band_ink("AAAA\\BBBB\\AAAA\\BBB.AB", narrow, ts(64))[0], Some((12, 70)));
    // Flags above bit 15 keep their meaning.
    assert_eq!(band_ink("AABBAABBAB", narrow, ts(3) | DT_WORD_ELLIPSIS)[0], Some((12, 61)));
    assert_eq!(band_ink("&A\tB", WIDE, ts(3) | x | DT_HIDEPREFIX)[0], Some((12, 41)), "no underline");
}

/// `DrawTextW` with `DT_MODIFYSTRING`: the return value and the text afterwards.
fn ell(s: &str, w: i32, h: i32, fmt: u32) -> (i32, String) {
    let (mut g, dc, _px) = sans(160, 80);
    let mut r = Rect::new(10, 10, 10 + w, 10 + h);
    let p = DrawParams { tab_in_format: true, ..DrawParams::default() };
    let out = g.draw_text_ex(dc, &utf16(s), &mut r, fmt | DT_MODIFYSTRING, p);
    assert_eq!(r, Rect::new(10, 10, 10 + w, 10 + h), "the rectangle is not changed");
    (out.height, out.modified.map_or_else(|| String::from(s), |m| String::from_utf16(&m).unwrap()))
}

#[test]
fn ellipsis_flags_act_per_line_as_the_sheet_says() {
    // ellipsis, modifystring, lines / dt_pf_ellipsis_lines: 60 wide, room for four lines.
    let e = DT_END_ELLIPSIS;
    let w = DT_WORD_ELLIPSIS;
    let lsl = "AABBAABBAB\nAB\nBBAABBAABA";
    let same = |s: &str, n: i32| (n, String::from(s));
    let cut = |n: i32, s: &str| (n, String::from(s));
    assert_eq!(ell(lsl, 60, 64, 0), same(lsl, 48));
    assert_eq!(ell(lsl, 60, 64, e), cut(48, "AABBAABBAB\nAB\nBBAAB..."));
    assert_eq!(ell(lsl, 60, 64, w), cut(48, "AABB..."));
    assert_eq!(ell(lsl, 60, 64, DT_PATH_ELLIPSIS), same(lsl, 48));
    assert_eq!(ell(lsl, 60, 64, e | w), cut(48, "AABB..."));
    let sls = "AB\nAABBAABBAB\nAB";
    assert_eq!(ell(sls, 60, 64, e), same(sls, 48));
    assert_eq!(ell(sls, 60, 64, w), cut(48, "AB\nAABB..."));
    assert_eq!(ell("AABBAABBAB\nBBAABBAABA", 60, 64, e), cut(32, "AABBAABBAB\nBBAAB..."));
    // A line break at the very end does not make a new last line.
    assert_eq!(ell("AABBAABBAB\nBBAABBAABA\n", 60, 64, e), cut(48, "AABBAABBAB\nBBAAB..."));
    assert_eq!(ell("AABBAA\nAB", 60, 64, e | DT_SINGLELINE), cut(16, "AABB..."));
    assert_eq!(ell("AABBAABBAB\nAB", 60, 64, e | DT_RIGHT), same("AABBAABBAB\nAB", 32));
    // The return value counts the lines laid out.
    assert_eq!(ell("AB\nBA\nAB", 60, 31, 0).0, 32);
    assert_eq!(ell("AB\nBA\nAB", 60, 32, 0).0, 48);
    assert_eq!(ell("AB\nBA\nAB", 60, 33, 0).0, 48);
    assert_eq!(ell("AB\nBA\nAB", 60, 32, e), same("AB\nBA\nAB", 48));
    // The height decides which line is the last: it takes all the text that is left.
    assert_eq!(ell(sls, 60, 32, e), same(sls, 48));
    assert_eq!(ell(sls, 60, 31, e), cut(32, "AB\nAABB..."));
    assert_eq!(ell(lsl, 60, 33, e), cut(48, "AABBAABBAB\nAB\nBBAAB..."));
    assert_eq!(ell(lsl, 60, 32, e), cut(48, "AABBAABBAB\nAB\nBBAAB..."));
    assert_eq!(ell(lsl, 60, 31, e), cut(32, "AABBAABBAB\nAB\nBBA..."));
    assert_eq!(ell(lsl, 60, 16, e), cut(32, "AABBAABBAB\nAB\nBBA..."));
    assert_eq!(ell(lsl, 60, 15, e), cut(16, "AABB..."));
    assert_eq!(ell(lsl, 60, 15, e | DT_NOCLIP), cut(48, "AABBAABBAB\nAB\nBBAAB..."), "the height plays no part");
    // DT_WORD_ELLIPSIS never takes over the text that follows.
    assert_eq!(ell(lsl, 60, 31, w), cut(32, "AABB..."));
    // DT_EDITCONTROL: the last line that still fits whole.
    assert_eq!(ell(lsl, 60, 32, e | DT_EDITCONTROL), cut(32, "AABBAABBAB\nAB\nBBA..."));
    // The same with DT_WORDBREAK.
    let words = "AAAA BBBB AAAA BBBB";
    let wb = DT_WORDBREAK;
    assert_eq!(ell(words, 60, 64, wb), same(words, 64));
    assert_eq!(ell(words, 60, 64, wb | e), same(words, 64));
    assert_eq!(ell(words, 60, 32, wb), same(words, 48));
    assert_eq!(ell(words, 60, 32, wb | e), cut(48, "AAAA BBBB AAAA..."));
    assert_eq!(ell(words, 60, 31, wb | e), cut(32, "AAAA BBBB ..."));
    assert_eq!(ell(words, 60, 32, wb | w), same(words, 48));
    assert_eq!(ell(words, 60, 40, wb | e), cut(48, "AAAA BBBB AAAA..."));
    // With DT_WORDBREAK only a word wider than the rectangle makes a line DT_WORD_ELLIPSIS cuts.
    let wide_word = "AB BBAABBAABA AB";
    assert_eq!(ell(wide_word, 60, 64, wb), same(wide_word, 48));
    assert_eq!(ell(wide_word, 60, 64, wb | w), cut(48, "AB BBAAB..."));
    assert_eq!(ell(wide_word, 60, 64, wb | e), same(wide_word, 48));
    assert_eq!(ell(wide_word, 60, 64, w), cut(16, "AB BB..."));
    assert_eq!(ell("AAAAAA BB\nBBBBBBBBB", 60, 64, w), cut(32, "AAAA..."));
    // Paths: the last line only, or with DT_WORD_ELLIPSIS every too-wide line in the path form.
    let paths = "AAAA\\BBBB\\AAAA\\BB.A\nAB\nBBBB\\AAAA\\BBBB\\AA.B";
    let last_cut = "AAAA\\BBBB\\AAAA\\BB.A\nAB\nBBBB\\...\\AA.B";
    assert_eq!(ell(paths, 100, 64, DT_PATH_ELLIPSIS), cut(48, last_cut));
    assert_eq!(ell(paths, 100, 64, DT_PATH_ELLIPSIS | e), cut(48, last_cut));
    assert_eq!(ell(paths, 100, 64, DT_PATH_ELLIPSIS | w), cut(48, "AAAA...\\BB.A"));
    // DT_CALCRECT measures the cut text and writes it back.
    let (mut g, dc, _px) = sans(160, 80);
    let mut r = Rect::new(10, 10, 70, 74);
    let p = DrawParams { tab_in_format: true, ..DrawParams::default() };
    let out = g.draw_text_ex(dc, &utf16(lsl), &mut r, e | DT_CALCRECT | DT_MODIFYSTRING, p);
    assert_eq!((out.height, r), (48, Rect::new(10, 10, 110, 58)));
    assert_eq!(String::from_utf16(&out.modified.unwrap()).unwrap(), "AABBAABBAB\nAB\nBBAAB...");
}

#[test]
fn ellipsis_lines_are_drawn_where_the_sheet_has_their_ink() {
    // dt_pf_ellipsis_lines: the ink of the line bands.
    let e = DT_END_ELLIPSIS;
    let rc = |h: i32| Rect::new(10, 10, 70, 10 + h);
    let lsl = "AABBAABBAB\nAB\nBBAABBAABA";
    let ink = |s: &str, h: i32, fmt: u32| band_ink(s, rc(h), fmt);
    assert_eq!(ink(lsl, 64, 0), [Some((12, 70)), Some((12, 29)), Some((11, 70)), None]);
    assert_eq!(ink(lsl, 64, e), [Some((12, 70)), Some((12, 29)), Some((11, 69)), None]);
    assert_eq!(ink(lsl, 64, DT_WORD_ELLIPSIS), [Some((12, 61)), Some((12, 29)), Some((11, 69)), None]);
    assert_eq!(ink("AABBAABBAB\nAB", 64, e | DT_RIGHT), [Some((12, 69)), Some((52, 69)), None, None]);
    // Three short lines: the third is laid out but lies outside the visible part.
    assert_eq!(ink("AB\nBA\nAB", 32, 0), [Some((12, 29)), Some((11, 28)), None, None]);
    assert_eq!(ink("AB\nBA\nAB", 33, 0), [Some((12, 29)), Some((11, 28)), Some((12, 20)), None]);
    // The last line takes the rest: "AB" + "BBA..." on the second line.
    assert_eq!(ink(lsl, 31, e), [Some((12, 70)), Some((12, 69)), None, None]);
    assert_eq!(ink(lsl, 33, e), [Some((12, 70)), Some((12, 29)), Some((28, 48)), None]);
    assert_eq!(ink(lsl, 15, e), [Some((12, 61)), None, None, None]);
    assert_eq!(ink(lsl, 15, e | DT_NOCLIP), [Some((12, 109)), Some((12, 29)), Some((11, 69)), None]);
    assert_eq!(ink(lsl, 31, DT_WORD_ELLIPSIS), [Some((12, 61)), Some((12, 29)), None, None]);
    let words = "AAAA BBBB AAAA BBBB";
    assert_eq!(ink(words, 31, DT_WORDBREAK | e), [Some((12, 56)), Some((11, 59)), None, None]);
    assert_eq!(ink(words, 64, DT_WORDBREAK), [Some((12, 56)), Some((11, 41)), Some((12, 56)), Some((11, 41))]);
}

#[test]
fn an_ellipsis_last_line_that_takes_the_rest_counts_the_empty_line_after_a_final_break() {
    // dt_pf_ellipsis_mix: "AB\nAB\n" in 31 high shows two lines and returns three.
    let rc = Rect::new(10, 10, 70, 41);
    let ret = |s: &str, fmt: u32| {
        let (mut g, dc, _px) = sans(240, 100);
        draw(&mut g, dc, s, rc, fmt).0
    };
    assert_eq!(band_ink("AB\nAB\n", rc, DT_END_ELLIPSIS), [Some((12, 29)), Some((12, 29)), None, None]);
    assert_eq!(ret("AB\nAB\n", DT_END_ELLIPSIS), 48);
    // Without a final break the rest is one line.
    assert_eq!(ret("AB\nAB\nAB", DT_END_ELLIPSIS), 32);
    assert_eq!(ret("AB\nAB\nAB", DT_END_ELLIPSIS | DT_WORD_ELLIPSIS), 32);
}

#[test]
fn dt_internal_lays_lines_out_at_the_height_of_the_stock_system_font() {
    // dt_pf_internal: the selected font's height never counts; its width does.
    let mut g = Gdi::new();
    let (dc, _px) = dib32(&mut g, 8, 8);
    let sys = g.text_metrics(dc).expect("a DC").height;
    assert!(sys > 0);
    assert!(g.add_font_mem(&tfont::build(&tfont::SANS)).is_some());
    assert!(g.add_font_mem(&tfont::build(&tfont::TALL)).is_some());
    for (face, h) in [
        ("PolyTest Sans", -8),
        ("PolyTest Sans", -16),
        ("PolyTest Sans", -32),
        ("PolyTest Tall", -16),
        ("PolyTest Tall", -32),
    ] {
        select(&mut g, dc, &lf_full(face, h, 400, false, 3));
        let r = Rect::new(10, 10, 210, 90);
        let calc = |g: &mut Gdi, s: &str, fmt: u32| {
            let mut r = r;
            let n = g.draw_text(dc, &utf16(s), &mut r, fmt | DT_CALCRECT);
            (n, r.right - r.left, r.bottom - r.top)
        };
        let (_, pw, _) = calc(&mut g, "AB", 0);
        let (n, w, bh) = calc(&mut g, "AB", DT_INTERNAL);
        assert_eq!((n, bh, w), (sys, sys, pw), "{face} {h}");
        assert_eq!(calc(&mut g, "AB\nAB", DT_INTERNAL).0, 2 * sys);
        assert_eq!(calc(&mut g, "AB", DT_INTERNAL | DT_EXTERNALLEADING).0, sys);
        assert_eq!(calc(&mut g, "AB", DT_INTERNAL | DT_EDITCONTROL).0, sys);
    }
}

#[test]
fn line_breaks_break_once_per_pair() {
    // line_breaks / dt_pf_breaks: the measured size.
    let (mut g, dc, _px) = sans(8, 8);
    let mut size = |s: &str, fmt: u32| {
        let (n, w, h) = calc(&mut g, dc, s, 100, fmt);
        assert_eq!(n, h);
        (w, h)
    };
    for s in ["AB\nW", "AB\rW", "AB\r\nW", "AB\n\rW"] {
        assert_eq!(size(s, 0), (20, 32), "{s:?}");
    }
    for s in [
        "AB\n\nW",
        "AB\r\rW",
        "AB\r\n\rW",
        "AB\n\r\nW",
        "AB\r\n\r\nW",
        "AB\n\r\n\rW",
        "AB\r\r\nW",
        "AB\n\n\rW",
        "AB\r\n\nW",
        "AB\n\r\rW",
    ] {
        assert_eq!(size(s, 0), (20, 48), "{s:?}");
    }
    assert_eq!(size("\n\rW", 0), (16, 32));
    assert_eq!(size("\r\nW", 0), (16, 32));
    // A break at the very end of the text adds an empty line.
    for s in ["W\n\r", "W\r\n", "W\r", "W\n"] {
        assert_eq!(size(s, 0), (16, 32), "{s:?}");
    }
    assert_eq!((size("\n\r", 0), size("\r", 0), size("\n", 0)), ((0, 32), (0, 32), (0, 32)));
    assert_eq!(size("AB \n\r W", 0), (26, 32));
    // With DT_SINGLELINE none of them breaks the line and none has a width.
    assert_eq!(size("AB\n\rW", DT_SINGLELINE), (36, 16));
    assert_eq!(size("AB\rW", DT_SINGLELINE), (36, 16));
    assert_eq!(size("AB\n\rW", DT_WORDBREAK), (20, 32));
    assert_eq!(size("AB\r\n\rW", DT_WORDBREAK), (20, 48));
    // Drawn: the second line is one line down, each line aligned on its own.
    assert_eq!(
        cell("AB\n\rW", Rect::new(10, 10, 130, 90), 0),
        (32, Rect::new(10, 10, 130, 90), Some((10, 10, 30, 42)))
    );
    assert_eq!(cell("AB\n\rW", Rect::new(10, 10, 130, 90), DT_CENTER).2, Some((60, 10, 80, 42)));
    assert_eq!(cell("AB\n\rW", Rect::new(10, 10, 130, 90), DT_SINGLELINE).2, Some((10, 10, 46, 26)));
}

#[test]
fn dt_editcontrol_breaks_wide_words_and_leaves_out_partial_lines() {
    // editcontrol / dt_pf_editctl: measured size, and the return value of the drawing call.
    let e = DT_EDITCONTROL;
    let wb = DT_WORDBREAK;
    let run = |s: &str, w: i32, h: i32, fmt: u32| {
        let (mut g, dc, _px) = sans(160, 180);
        let mut c = Rect::new(10, 10, 10 + w, 10 + h);
        let cret = g.draw_text(dc, &utf16(s), &mut c, fmt | DT_CALCRECT);
        let (ret, _) = draw(&mut g, dc, s, Rect::new(10, 10, 10 + w, 10 + h), fmt);
        ((c.right - c.left, c.bottom - c.top), cret, ret)
    };
    // Without DT_WORDBREAK, and without DT_EDITCONTROL, a word is never broken.
    assert_eq!(run("WWWWW", 40, 150, wb), ((80, 16), 16, 16));
    assert_eq!(run("WWWWW", 40, 150, e), ((80, 16), 16, 16));
    // As many characters to a line as fit, and at least one.
    assert_eq!(run("WWWWW", 40, 150, e | wb), ((32, 48), 48, 48));
    assert_eq!(run("WWWWW", 48, 150, e | wb), ((48, 32), 32, 32));
    assert_eq!(run("WWWWW", 16, 150, e | wb), ((16, 80), 80, 80));
    assert_eq!(run("WWW", 10, 150, e | wb), ((16, 48), 48, 48));
    assert_eq!(run("WWW", 0, 150, e | wb), ((0, 150), 1, 1), "width 0 leaves the rectangle alone");
    assert_eq!(run("AB WWWWW AB", 40, 150, wb), ((80, 48), 48, 48));
    assert_eq!(run("AB WWWWW AB", 40, 150, e | wb), ((32, 80), 80, 80));
    // Such a word first goes to a line of its own.
    assert_eq!(run("B WWWWW", 40, 150, e | wb), ((32, 64), 64, 64));
    // What follows its last piece goes on behind that piece if it fits.
    assert_eq!(run("WWWWW B", 40, 150, e | wb), ((32, 48), 48, 48));
    assert_eq!(run("WWWWWB B", 40, 150, e | wb), ((38, 48), 48, 48));
    assert_eq!(run("BABABABA", 30, 150, e | wb), ((28, 64), 64, 64));
    assert_eq!(run("BABA\nABABAB", 30, 150, e | wb), ((28, 80), 80, 80));
    assert_eq!(run("AB AB", 60, 150, e | wb), ((46, 16), 16, 16));
    assert_eq!(run("WWWWW", 40, 150, e | wb | DT_CENTER), ((32, 48), 48, 48));
    // The height: after the first line, only lines that fit whole.
    assert_eq!(run("WWWWW", 40, 48, e | wb), ((32, 48), 48, 48));
    assert_eq!(run("WWWWW", 40, 47, e | wb), ((32, 48), 48, 32));
    assert_eq!(run("WWWWW", 40, 32, e | wb), ((32, 48), 48, 32));
    assert_eq!(run("WWWWW", 40, 24, e | wb), ((32, 48), 48, 16));
    for (h, fmt) in [(16, e), (15, e), (15, 0), (15, e | DT_SINGLELINE), (0, e)] {
        assert_eq!(run("AB", 60, h, fmt), ((20, 16), 16, 16), "the first line is always laid out ({h} high)");
    }
    assert_eq!(run("AB\nAB", 60, 31, e), ((20, 32), 32, 16));
    assert_eq!(run("AB\nAB", 60, 32, e), ((20, 32), 32, 32));
    assert_eq!(run("AB\nAB", 60, 31, 0), ((20, 32), 32, 32));
    assert_eq!(run("AB\nAB", 60, 31, e | DT_NOCLIP), ((20, 32), 32, 32));
    // The lines as drawn.
    let lines = |s: &str, w: i32, fmt: u32| band_ink(s, Rect::new(10, 10, 10 + w, 160), fmt);
    assert_eq!(lines("WWWWW", 40, e | wb), [Some((12, 40)), Some((12, 40)), Some((12, 24)), None]);
    assert_eq!(lines("B WWWWW", 40, e | wb), [Some((11, 17)), Some((12, 40)), Some((12, 40)), Some((12, 24))]);
    assert_eq!(lines("WWWWW B", 40, e | wb), [Some((12, 40)), Some((12, 40)), Some((12, 39)), None]);
    assert_eq!(lines("WWWWWB B", 40, e | wb), [Some((12, 40)), Some((12, 40)), Some((12, 47)), None]);
    assert_eq!(lines("BABABABA", 30, e | wb), [Some((11, 37)), Some((12, 29)), Some((12, 29)), Some((12, 20))]);
    assert_eq!(lines("WWWWW", 40, e | wb | DT_CENTER), [Some((16, 44)), Some((16, 44)), Some((24, 36)), None]);
}

#[test]
fn flags_given_together_although_they_exclude_each_other() {
    // pairs / dt_pf_pairs: "AB" (20x16) in 4,4,76,36.
    let rc = Rect::new(4, 4, 76, 36);
    let sl = DT_SINGLELINE;
    let at = |s: &str, fmt: u32| {
        let (n, after, bk) = cell(s, rc, fmt);
        assert_eq!(after, rc, "the rectangle is not changed");
        (n, bk)
    };
    assert_eq!(at("AB", DT_CENTER | DT_RIGHT), (16, Some((30, 4, 50, 20))));
    assert_eq!(at("AB", DT_CENTER | DT_RIGHT | sl), (16, Some((30, 4, 50, 20))));
    assert_eq!(at("AB", sl | DT_VCENTER), (24, Some((4, 12, 24, 28))));
    assert_eq!(at("AB", sl | DT_BOTTOM), (32, Some((4, 20, 24, 36))));
    assert_eq!(at("AB", sl | DT_VCENTER | DT_BOTTOM), (16, Some((4, 4, 24, 20))));
    assert_eq!(at("AB", DT_VCENTER | DT_BOTTOM), (16, Some((4, 4, 24, 20))));
    assert_eq!(at("AB AB AB AB AB", sl | DT_BOTTOM | DT_WORDBREAK), (32, Some((4, 20, 76, 36))));
    assert_eq!(at("AB AB AB AB AB", DT_WORDBREAK), (32, Some((4, 4, 76, 36))));
    // DT_CALCRECT with a vertical alignment returns what the drawing call returns.
    let measure = |fmt: u32| {
        let (n, after, bk) = cell("AB", rc, fmt | DT_CALCRECT);
        assert_eq!(bk, None, "nothing is drawn");
        (n, after)
    };
    assert_eq!(measure(sl), (16, Rect::new(4, 4, 24, 20)));
    assert_eq!(measure(sl | DT_VCENTER), (24, Rect::new(4, 4, 24, 28)));
    assert_eq!(measure(sl | DT_BOTTOM), (32, Rect::new(4, 4, 24, 36)));
    assert_eq!(measure(sl | DT_VCENTER | DT_BOTTOM), (16, Rect::new(4, 4, 24, 20)));
    for fmt in [DT_CENTER, DT_RIGHT, DT_NOCLIP, DT_VCENTER, DT_BOTTOM] {
        assert_eq!(measure(fmt), (16, Rect::new(4, 4, 24, 20)), "{fmt:#x}");
    }
    // DT_EXTERNALLEADING puts the leading (2) below each line.
    assert_eq!(at("AB", DT_EXTERNALLEADING), (18, Some((4, 4, 24, 20))));
    assert_eq!(at("AB", DT_EXTERNALLEADING | sl | DT_VCENTER), (25, Some((4, 11, 24, 27))));
    assert_eq!(at("AB", DT_EXTERNALLEADING | sl | DT_BOTTOM), (32, Some((4, 18, 24, 34))));
    assert_eq!(at("AB\nAB", DT_EXTERNALLEADING), (36, Some((4, 4, 24, 36))));
    // The prefix flags.
    assert_eq!(at("&AB", DT_HIDEPREFIX | DT_PREFIXONLY), (16, Some((4, 4, 24, 20))));
    assert_eq!(at("&AB", DT_NOPREFIX | DT_PREFIXONLY), (16, Some((4, 4, 33, 20))));
    assert_eq!(at("&AB", DT_NOPREFIX | DT_HIDEPREFIX), (16, Some((4, 4, 33, 20))));
    assert_eq!(at("&AB", DT_PREFIXONLY), (16, None));
    let underline_only = {
        let (mut g, dc, px) = sans(240, 100);
        draw(&mut g, dc, "&AB", rc, DT_PREFIXONLY);
        where_is(&px, 240, FG)
    };
    assert_eq!((underline_only.len(), bounds(&underline_only)), (12, (4, 17, 16, 18)));
    let no_underline = {
        let (mut g, dc, px) = sans(240, 100);
        draw(&mut g, dc, "&AB", rc, DT_HIDEPREFIX | DT_PREFIXONLY);
        where_is(&px, 240, FG).len()
    };
    assert_eq!(no_underline, 144);
    // Many at once; with DT_TABSTOP bits 8 to 15 are no flags.
    let all = !DT_MODIFYSTRING;
    assert_eq!(at("AB", all), (16, Some((30, 4, 50, 20))));
    assert_eq!(cell("AB", rc, all & !DT_TABSTOP), (16, Rect::new(4, 4, 24, 20), None), "DT_CALCRECT");
    assert_eq!(at("AB", all & !DT_VCENTER), (32, Some((30, 20, 50, 36))));
    assert_eq!(at("AB", all & !DT_CENTER), (16, Some((56, 4, 76, 20))));
    // Bits 22 to 31 have no meaning.
    assert_eq!(at("AB", 0xFFC0_0000), (16, Some((4, 4, 24, 20))));
    assert_eq!(at("AB", 0xFFC0_0000 | sl | DT_BOTTOM | DT_RIGHT), (32, Some((56, 20, 76, 36))));
    assert_eq!(at("AB", 0x40_0000), (16, Some((4, 4, 24, 20))));
}

#[test]
fn counts_and_empty_strings() {
    // count, bad_dc / dt_pf_count.
    assert_eq!(count_check(true, false, true, 0), Early::Return(0), "a NULL string");
    assert_eq!(count_check(true, false, true, -1), Early::Return(0));
    assert_eq!(count_check(false, true, false, 5), Early::Return(0), "a NULL rectangle");
    for n in [-2, -3, -1000, i32::MIN] {
        assert_eq!(count_check(false, false, false, n), Early::Return(0), "count {n}");
        assert_eq!(count_check(false, false, true, n), Early::Return(0), "count {n}, empty string");
    }
    // A count of 0 on a text that does not start with a NUL: 1, before the DC is looked at.
    assert_eq!(count_check(false, false, false, 0), Early::Return(1));
    // The empty string, and every other count.
    assert_eq!(count_check(false, false, true, 0), Early::Layout);
    assert_eq!(count_check(false, false, true, -1), Early::Layout);
    for n in [-1, 1, 5, i32::MAX] {
        assert_eq!(count_check(false, false, false, n), Early::Layout, "count {n}");
    }

    let (mut g, dc, px) = sans(80, 40);
    let rc = Rect::new(4, 4, 76, 36);
    let text = utf16("MgiBA");
    let measure = |g: &mut Gdi, dc: u64, s: &[u16], fmt: u32| {
        let mut r = rc;
        let n = g.draw_text(dc, s, &mut r, fmt | DT_CALCRECT);
        (n, r)
    };
    // An explicit count is taken as given.
    for (n, right) in [(5usize, 50), (4, 38), (3, 30), (2, 26), (1, 16)] {
        assert_eq!(measure(&mut g, dc, &text[..n], 0), (16, Rect::new(4, 4, right, 20)), "count {n}");
    }
    assert_eq!(measure(&mut g, dc, &text[..2], DT_SINGLELINE), (16, Rect::new(4, 4, 26, 20)));
    // The empty string: 1 without DT_SINGLELINE and the line height with it.
    assert_eq!(measure(&mut g, dc, &[], 0), (1, Rect::new(4, 4, 4, 4)));
    assert_eq!(measure(&mut g, dc, &[], DT_SINGLELINE), (16, Rect::new(4, 4, 4, 20)));
    assert_eq!(measure(&mut g, dc, &utf16("\n"), 0), (32, Rect::new(4, 4, 4, 36)), "a lone line break");
    assert_eq!(changed(&px, 80), None, "measuring draws nothing");
    let drawn = |g: &mut Gdi, dc: u64, s: &[u16], fmt: u32| {
        let mut r = rc;
        let n = g.draw_text(dc, s, &mut r, fmt);
        assert_eq!(r, rc);
        n
    };
    assert_eq!(drawn(&mut g, dc, &[], 0), 1);
    assert_eq!(drawn(&mut g, dc, &[], DT_SINGLELINE), 16);
    assert_eq!(drawn(&mut g, dc, &[], DT_SINGLELINE | DT_VCENTER), 24);
    assert_eq!(drawn(&mut g, dc, &[], DT_SINGLELINE | DT_BOTTOM), 32);
    assert_eq!(changed(&px, 80), None, "an empty string draws nothing");
    assert_eq!(drawn(&mut g, dc, &text[..2], DT_SINGLELINE | DT_VCENTER), 24);
    assert_eq!(drawn(&mut g, dc, &text[..2], DT_SINGLELINE | DT_BOTTOM), 32);
    // A NULL DC: 0.
    assert_eq!(drawn(&mut g, 0, &text, 0), 0);
    assert_eq!(drawn(&mut g, 0, &[], 0), 0);
    assert_eq!(measure(&mut g, 0, &text, 0), (0, rc));
    assert_eq!(measure(&mut g, 0, &[], DT_SINGLELINE), (0, rc));
    // odd_input.
    let long = vec![u16::from(b'A'); 1000];
    assert_eq!(measure(&mut g, dc, &long, DT_SINGLELINE).1.right, 4 + 12000);
    let mut huge = Rect::new(-100_000, -100_000, 100_000, 100_000);
    assert_eq!(g.draw_text(dc, &utf16("AB"), &mut huge, DT_CENTER | DT_VCENTER | DT_SINGLELINE), 100_008);
}
