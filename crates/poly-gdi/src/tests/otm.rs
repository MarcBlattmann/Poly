//! `GetOutlineTextMetricsA/W` (`crate::otm`) with the sheet's test font: the numbers of
//! `GetOutlineTextMetricsA.yaml` (its "PolyDlg Mixed" is the same font as "PolyTest Sans" here).

use super::*;
use crate::otm::*;
use alloc::string::String;

fn otm(height: i32) -> OutlineMetrics {
    let mut g = Gdi::new();
    let file = tfont::build(&tfont::SANS);
    assert!(g.add_font_mem(&file).is_some());
    let (dc, _px) = dib32(&mut g, 4, 4);
    select(&mut g, dc, &lf_full("PolyTest Sans", height, 400, false, 3));
    g.outline_text_metrics(dc).unwrap().unwrap()
}

fn at(b: &[u8], i: usize) -> i32 {
    i32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}

#[test]
fn the_numbers_are_the_font_files_scaled() {
    for (h, asc, desc, gap, sub, sub_off, sup_off, strike, under) in [
        (-16, 12, -4, 2, [10, 9], [0, 1], [0, 5], (1, 4), (1, -2)),
        (-32, 24, -8, 4, [20, 19], [0, 2], [0, 11], (2, 8), (2, -4)),
        (-12, 9, -3, 2, [8, 7], [0, 1], [0, 4], (1, 3), (1, -2)),
        (16, 12, -4, 2, [10, 9], [0, 1], [0, 5], (1, 4), (1, -2)),
    ] {
        let m = otm(h);
        let em = -asc + desc; // -(em)
        assert_eq!(m.tm.height, -em, "{h}");
        assert_eq!((m.ascent, m.descent, m.line_gap), (asc, desc, gap), "{h}");
        assert_eq!((m.mac_ascent, m.mac_descent, m.mac_line_gap), (asc, desc, gap), "{h}");
        assert_eq!(m.font_box, [0, asc, -em, desc], "{h}");
        assert_eq!((m.cap_height, m.x_height), (-em / 2, -em / 4), "{h}");
        assert_eq!((m.sub_size, m.sub_offset, m.super_size, m.super_offset), (sub, sub_off, sub, sup_off), "{h}");
        assert_eq!((m.strike_size, m.strike_pos), strike, "{h}");
        assert_eq!((m.under_size, m.under_pos), under, "{h}");
        assert_eq!((m.em_square, m.min_ppem, m.slope_rise, m.slope_run, m.italic_angle), (1024, 8, 1, 0, 0));
        assert_eq!((m.fs_selection, m.fs_type), (0x40, 0));
        assert_eq!(m.panose, [2, 0, 0, 3, 0, 0, 0, 0, 0, 0]);
        assert_eq!(m.names, ["PolyTest Sans", "PolyTest Sans", "Regular", "PolyTest Sans 1.0"].map(String::from));
    }
}

#[test]
fn the_structure_ends_with_the_four_names() {
    let m = otm(-16);
    // A: 232 bytes and the names as bytes: 14 + 14 + 8 + 18.
    let a = m.to_bytes(false);
    assert_eq!(a.len(), 286);
    assert_eq!(at(&a, 0), 286);
    // the TEXTMETRICA: height at 4, first character byte at 4 + 44
    assert_eq!(at(&a, 4), 16);
    assert_eq!(a[48], m.tm.first_char as u8);
    // otmEMSquare: the sixth member after the PANOSE (which ends at 71, padded to 72)
    assert_eq!(at(&a, 72 + 20), 1024);
    let offs: Vec<usize> =
        (0..4).map(|i| u64::from_le_bytes(a[200 + 8 * i..208 + 8 * i].try_into().unwrap()) as usize).collect();
    assert_eq!(offs, [232, 246, 260, 268]);
    assert_eq!(&a[260..268], b"Regular\0");
    assert_eq!(&a[268..286], b"PolyTest Sans 1.0\0");
    // W: the same fixed size, TEXTMETRICW, UTF-16 names.
    let w = m.to_bytes(true);
    assert_eq!(w.len(), 232 + 2 * 54);
    assert_eq!(at(&w, 0), w.len() as i32);
    assert_eq!(at(&w, 76 + 20), 1024);
    let first = u64::from_le_bytes(w[200..208].try_into().unwrap()) as usize;
    assert_eq!(first, 232);
    assert_eq!(&w[232..236], &[b'P', 0, b'o', 0]);
}

#[test]
fn a_bitmap_face_has_none() {
    let mut g = Gdi::new();
    let (dc, _px) = dib32(&mut g, 4, 4);
    assert_eq!(g.outline_text_metrics(dc), Some(None));
    assert_eq!(g.outline_text_metrics(0x1234), None);
}
