//! `DrawTextW`: the empty last line after a final line break and the line a
//! too-wide word that ends at a break brings in (`DrawTextW.yaml`,
//! `last_line`). The tables in `fixtures/` are the visible tests'
//! `dt_pf_last_*` outputs recorded on Windows (`windows.json`): PolyTest Sans
//! at -16 (lines 16 high), a rectangle 60 wide at 10,10, each text, flag set
//! and rectangle height measured and drawn. In the texts of the tables and of
//! the tests below `n` is a line feed, `r` a carriage return and `t` a tab
//! (the letters of the texts are A, B and blanks).

use super::*;
use crate::text::*;
use std::format;
use std::string::String;

const PLAIN: &str = include_str!("fixtures/dt_pf_last_plain.txt");
const EDIT: &str = include_str!("fixtures/dt_pf_last_edit.txt");
const WRAP: &str = include_str!("fixtures/dt_pf_last_wrap.txt");
const WIDE: &str = include_str!("fixtures/dt_pf_last_wide.txt");

/// The DC with PolyTest Sans at -16 selected; the pixels are the DIB's
/// memory and must outlive the drawing.
fn sans() -> (Gdi, u64, Vec<u32>) {
    let mut g = Gdi::new();
    assert!(g.add_font_mem(&tfont::build(&tfont::SANS)).is_some());
    let (dc, px) = dib32(&mut g, 160, 130);
    select(&mut g, dc, &lf_full("PolyTest Sans", -16, 400, false, 3));
    (g, dc, px)
}

fn unescape(s: &str) -> Vec<u16> {
    s.chars()
        .map(|c| match c {
            'n' => 10,
            'r' => 13,
            't' => 9,
            c => c as u16,
        })
        .collect()
}

fn flags_of(letters: &str) -> u32 {
    letters.chars().fold(0, |f, c| {
        f | match c {
            'E' => DT_EDITCONTROL,
            'S' => DT_SINGLELINE,
            'N' => DT_NOCLIP,
            'W' => DT_WORDBREAK,
            'e' => DT_END_ELLIPSIS,
            _ => 0,
        }
    })
}

/// Draws and measures every row of a table; returns how many there were and
/// the rows that differ.
fn replay(table: &str, extra: u32) -> (usize, Vec<String>) {
    let (mut g, dc, _px) = sans();
    let (mut n, mut bad) = (0, Vec::new());
    for row in table.lines().filter(|l| !l.starts_with('#')) {
        let c: Vec<&str> = row.split('|').collect();
        let (text, flags, h) = (unescape(c[0]), flags_of(c[1]) | extra, c[2].parse::<i32>().unwrap());
        let want: Vec<i32> = c[3..].iter().map(|v| v.parse().unwrap()).collect();
        let area = Rect::new(10, 10, 70, 10 + h);
        let mut r = area;
        let draw = g.draw_text(dc, &text, &mut r, flags);
        assert_eq!(r, area, "{row}: drawing leaves the rectangle");
        let mut r = area;
        let calc = g.draw_text(dc, &text, &mut r, flags | DT_CALCRECT);
        let got = [draw, calc, r.right - r.left, r.bottom - r.top];
        n += 1;
        if got[..] != want[..] {
            bad.push(format!("{row}: got draw {} calc {} rect {}x{}", got[0], got[1], got[2], got[3]));
        }
    }
    (n, bad)
}

fn all_rows_match(table: &str, extra: u32, rows: usize) {
    let (n, bad) = replay(table, extra);
    assert_eq!(n, rows);
    assert!(bad.is_empty(), "{} of {n} rows differ:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn a_final_break_adds_an_empty_last_line_without_the_edit_control_flag() {
    // dt_pf_last_plain: 11 texts x 4 flag sets x 6 heights.
    all_rows_match(PLAIN, 0, 264);
}

#[test]
fn a_final_break_adds_no_empty_last_line_with_the_edit_control_flag() {
    // dt_pf_last_edit.
    all_rows_match(EDIT, 0, 330);
}

#[test]
fn word_wrapped_lines_follow_the_same_rule() {
    // dt_pf_last_wrap: "AAAA BBBB AAAA" is three lines in 60.
    all_rows_match(WRAP, 0, 252);
}

#[test]
fn a_too_wide_line_that_ends_at_a_break_is_followed_by_an_empty_line_when_drawn() {
    // dt_pf_last_wide, with DT_EXPANDTABS as the test has it.
    all_rows_match(WIDE, DT_EXPANDTABS, 160);
}

/// (the drawing call's return value, DT_CALCRECT's) for `text` in a rectangle
/// `h` high.
fn height(text: &str, h: i32, flags: u32) -> (i32, i32) {
    let (mut g, dc, _px) = sans();
    let t = unescape(text);
    let area = Rect::new(10, 10, 70, 10 + h);
    let mut r = area;
    let draw = g.draw_text(dc, &t, &mut r, flags);
    let mut r = area;
    let calc = g.draw_text(dc, &t, &mut r, flags | DT_CALCRECT);
    (draw, calc)
}

#[test]
fn measuring_counts_the_empty_last_line_in_any_height() {
    for (text, want) in [("AB", 16), ("ABn", 32), ("ABn", 32), ("n", 32), ("ABnn", 48), ("ABnABn", 48)] {
        for h in [0, 10, 100] {
            assert_eq!(height(text, h, 0).1, want, "{text} measured in {h}");
            assert_eq!(height(text, h, DT_NOCLIP).1, want);
            assert_eq!(height(text, h, DT_WORDBREAK).1, want);
        }
    }
}

#[test]
fn drawing_adds_the_empty_last_line_to_the_lines_that_fit() {
    // Line k (from 0) is laid out when 16 k <= H, the first always; the empty last line on top.
    for h in [10, 20, 31, 32, 48, 100] {
        assert_eq!(height("ABn", h, 0).0, 32, "ABn in {h}");
    }
    for (h, want) in [(10, 32), (15, 32), (16, 48), (100, 48)] {
        assert_eq!(height("ABnABn", h, 0).0, want, "ABnABn in {h}");
        assert_eq!(height("ABnn", h, 0).0, want, "ABnn in {h}");
    }
    for (h, want) in [(10, 32), (16, 48), (31, 48), (32, 64), (100, 64)] {
        assert_eq!(height("ABnnABn", h, 0).0, want, "ABnnABn in {h}");
    }
    // With DT_NOCLIP every line is laid out.
    assert_eq!(height("ABnnABn", 10, DT_NOCLIP).0, 64);
}

#[test]
fn the_edit_control_has_no_empty_last_line() {
    // Line k needs 16 (k + 1) <= H, the first line always.
    for (h, want) in [(10, 16), (31, 16), (32, 32), (100, 32)] {
        assert_eq!(height("ABnABn", h, DT_EDITCONTROL).0, want, "edit, ABnABn in {h}");
    }
    for (h, want) in [(10, 16), (31, 16), (32, 32), (47, 32), (48, 48)] {
        assert_eq!(height("ABnnABn", h, DT_EDITCONTROL).0, want, "edit, ABnnABn in {h}");
    }
    // dt_pf_calc_low: measured in 31 high.
    assert_eq!(height("ABnABn", 31, DT_EDITCONTROL).1, 32);
    assert_eq!(height("ABnABn", 31, 0).1, 48);
    for text in ["ABn", "ABrn", "ABnr", "ABr", "n", "rn"] {
        assert_eq!(height(text, 100, DT_EDITCONTROL).1, 16, "{text}");
    }
    // DT_END_ELLIPSIS changes none of it.
    assert_eq!(height("ABnABn", 31, DT_EDITCONTROL | DT_END_ELLIPSIS), (16, 32));
}

#[test]
fn a_single_line_is_one_line_whatever_the_text() {
    for h in [0, 10, 100] {
        assert_eq!(height("ABnABn", h, DT_SINGLELINE), (16, 16));
        assert_eq!(height("ABn", h, DT_SINGLELINE | DT_EDITCONTROL), (16, 16));
    }
}

#[test]
fn the_four_break_forms_are_one_break_each() {
    for text in ["ABn", "ABrn", "ABnr", "ABr"] {
        assert_eq!(height(text, 100, 0), (32, 32), "{text}");
        assert_eq!(height(text, 100, DT_EDITCONTROL), (16, 16), "{text} with DT_EDITCONTROL");
    }
}

#[test]
fn a_wide_word_at_a_break_brings_an_empty_line_in_the_drawing_call_only() {
    // "AAAAAB" is 68 wide in 60, "AAAAA" with exactly 60 is not.
    let w = DT_WORDBREAK;
    assert_eq!(height("AAAAABnAB", 100, w), (48, 32), "the word, the empty line, AB; measured without it");
    assert_eq!(height("AAAAABnAB", 20, w).0, 32, "the empty line is laid out when 16 k <= H");
    assert_eq!(height("AAAAAnAB", 100, w), (32, 32), "a word of exactly the width");
    assert_eq!(height("AB AABBAABBABnAB", 100, w).0, 64, "AB, the word, the empty line, AB");
    assert_eq!(height("AABBAABBABn", 100, w), (48, 32), "the word, the extra line, the empty last line");
    // The blank after the word already leaves an empty line when wrapped: no second one.
    assert_eq!(height("AABBAABBAB nAB", 100, w), (48, 48));
    assert_eq!(height("AABBAABBAB nAB", 20, w).0, 32);
    // None at the end of the text, for a word followed by a blank and more text, without
    // DT_WORDBREAK, or with DT_EDITCONTROL (the word is broken into lines that fit).
    assert_eq!(height("AABBAABBAB", 100, w).0, 16);
    assert_eq!(height("AABBAABBAB AB", 100, w), (32, 32));
    assert_eq!(height("AAAAABnAB", 100, 0).0, 32);
    assert_eq!(height("AAAAABnAB", 100, w | DT_EDITCONTROL), (48, 48));
    // DT_NOCLIP does not change it.
    assert_eq!(height("AAAAABnAB", 20, w | DT_NOCLIP).0, 48);
}

#[test]
fn an_expanding_tab_ends_a_word_and_the_edit_control_has_its_own_average_width() {
    let w = DT_WORDBREAK | DT_EXPANDTABS;
    // "AABBAABBAB" is 100 wide, the tab would take the line to 128: the line breaks at the tab,
    // which is dropped (dt_pf_last_wide).
    assert_eq!(height("AABBAABBABtABnAB", 100, w), (48, 48));
    // Without DT_EXPANDTABS the tab has no width and is no break.
    assert_eq!(height("AABBAABBABtAB", 100, DT_WORDBREAK), (16, 16));
    // The edit control's average width is 7 here (the 52 letters are 338 wide, the letters the
    // font lacks as wide as a blank), so a tab stop is 56: 100 goes to 112, 48 to 56.
    let (mut g, dc, _px) = sans();
    let t = unescape("AABBAABBABtAB");
    for (flags, width) in [(DT_EXPANDTABS, 148), (DT_EXPANDTABS | DT_EDITCONTROL, 132)] {
        let mut r = Rect::new(10, 10, 70, 100);
        g.draw_text(dc, &t, &mut r, flags | DT_CALCRECT);
        assert_eq!(r.right - r.left, width, "flags {flags:#x}");
    }
}
