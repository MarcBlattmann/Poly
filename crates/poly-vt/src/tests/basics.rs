//! Printing, control characters, autowrap, wide characters, charsets and
//! robustness.

use super::*;
use crate::Color;

#[test]
fn prints_and_moves() {
    let t = term(10, 3, "hello");
    assert_eq!(rows(&t), ["hello", "", ""]);
    assert_eq!(cursor(&t), (0, 5));
}

#[test]
fn lf_does_not_return_unless_newline_mode() {
    let t = term(10, 3, "ab\ncd");
    assert_eq!(rows(&t), ["ab", "  cd", ""]);
    let t = term(10, 3, "\x1b[20hab\ncd");
    assert_eq!(rows(&t), ["ab", "cd", ""]);
    let t = term(10, 3, "ab\x0bc\x0cd");
    assert_eq!(rows(&t), ["ab", "  c", "   d"]);
}

#[test]
fn cr_bs_ht() {
    let t = term(20, 2, "abc\rX\x08\x08Y\tZ");
    assert_eq!(rows(&t), ["Ybc     Z", ""]);
    let t = term(20, 1, "\t\t\t\tx");
    assert_eq!(row(&t, 0).find('x'), Some(19));
    // BS at column 0 stays.
    let t = term(5, 1, "\x08\x08a");
    assert_eq!(rows(&t), ["a"]);
}

#[test]
fn bell_and_title() {
    let mut t = term(10, 2, "a\x07");
    assert!(t.take_bell());
    assert!(!t.take_bell());
    t.feed(b"\x1b]0;my title\x07");
    assert_eq!(t.title(), "my title");
    assert!(t.take_title_changed());
    assert!(!t.take_title_changed());
    t.feed("\x1b]2;caf\u{e9} \x1b\\".as_bytes());
    assert_eq!(t.title(), "caf\u{e9} ");
    t.feed(b"\x1b]1;icon\x07");
    assert_eq!(t.title(), "caf\u{e9} ");
    t.feed(b"\x1b]7;file://host/home/me\x07");
    assert_eq!(t.working_directory(), "file://host/home/me");
    // OSC does not print.
    assert_eq!(rows(&t), ["a", ""]);
}

#[test]
fn pending_wrap_at_last_column() {
    let mut t = term(5, 3, "abcde");
    assert_eq!(cursor(&t), (0, 4));
    assert!(!t.line_wrapped(0));
    t.feed(b"\r\n");
    // A line of exactly `cols` characters and a newline: no empty line.
    assert_eq!(cursor(&t), (1, 0));
    t.feed(b"fghijk");
    assert_eq!(rows(&t), ["abcde", "fghij", "k"]);
    assert!(t.line_wrapped(1));
    assert!(!t.line_wrapped(0));
}

#[test]
fn pending_wrap_cleared_by_cursor_motion() {
    let t = term(5, 2, "abcde\x1b[DX");
    assert_eq!(rows(&t), ["abcXe", ""]);
    let t = term(5, 2, "abcde\rX");
    assert_eq!(rows(&t), ["Xbcde", ""]);
    let t = term(5, 2, "abcde\x08X");
    assert_eq!(rows(&t), ["abcXe", ""]);
}

#[test]
fn autowrap_off_overwrites_last_column() {
    let t = term(5, 2, "\x1b[?7labcdefg");
    assert_eq!(rows(&t), ["abcdg", ""]);
    assert_eq!(cursor(&t), (0, 4));
}

#[test]
fn wrap_scrolls_at_bottom() {
    let t = term(3, 2, "abcdefgh");
    assert_eq!(rows(&t), ["def", "gh"]);
    assert_eq!(t.scrollback_len(), 1);
}

#[test]
fn wide_chars() {
    let t = term(6, 2, "a中b");
    let c = cell(&t, 0, 1);
    assert_eq!((c.ch, c.width), ('中', 2));
    assert_eq!(cell(&t, 0, 2).width, 0);
    assert_eq!(cell(&t, 0, 3).ch, 'b');
    assert_eq!(cursor(&t), (0, 4));
}

#[test]
fn wide_char_at_line_end_wraps_early() {
    let t = term(5, 2, "abcd中");
    assert_eq!(cell(&t, 0, 4), Cell::BLANK);
    assert_eq!(cell(&t, 1, 0).ch, '中');
    assert!(t.line_wrapped(0));
    assert_eq!(cursor(&t), (1, 2));
    // Exactly filling the line leaves a wrap pending.
    let t = term(4, 2, "ab中");
    assert_eq!(cursor(&t), (0, 3));
    assert_eq!(rows(&t), ["ab中", ""]);
    // Too wide for a 1-column terminal: dropped.
    let t = term(1, 1, "中");
    assert_eq!(rows(&t), [""]);
}

#[test]
fn overwriting_half_a_wide_char_clears_the_other_half() {
    let t = term(6, 1, "中文\x1b[2GX");
    assert_eq!(cell(&t, 0, 0), Cell::BLANK);
    assert_eq!(cell(&t, 0, 1).ch, 'X');
    assert_eq!(cell(&t, 0, 2).ch, '文');
    let t = term(6, 1, "中文\x1b[1GX");
    assert_eq!(cell(&t, 0, 1), Cell::BLANK);
    assert_eq!(cell(&t, 0, 1).width, 1);
}

#[test]
fn combining_marks_are_dropped() {
    let t = term(6, 1, "e\u{301}x");
    assert_eq!(rows(&t), ["ex"]);
}

#[test]
fn dec_line_drawing() {
    let t = term(10, 2, "\x1b(0lqqk\x1b(B lq\r\n\x1b)0\x0exj\x0fx");
    assert_eq!(rows(&t), ["┌──┐ lq", "│┘x"]);
}

#[test]
fn decsc_saves_charset_and_attributes() {
    let t = term(10, 2, "\x1b[31m\x1b(0\x1b7\x1b[0m\x1b(B\x1b[2;5Hq\x1b8q");
    let c = cell(&t, 0, 0);
    assert_eq!(c.ch, '─');
    assert_eq!(c.fg, Color::Indexed(1));
    assert_eq!(cell(&t, 1, 4).ch, 'q');
    assert_eq!(cell(&t, 1, 4).fg, Color::Default);
}

#[test]
fn rep_repeats_last_char() {
    let t = term(10, 1, "ab\x1b[3b");
    assert_eq!(rows(&t), ["abbbb"]);
    let t = term(4, 2, "\x1b[65535b");
    assert_eq!(rows(&t), ["", ""]);
    // At most a screenful of repeats.
    let t = term(4, 2, "x\x1b[65535b");
    assert_eq!(rows(&t), ["xxxx", "x"]);
}

#[test]
fn can_and_sub_cancel_sequences() {
    let t = term(10, 1, "\x1b[3\x18Ab\x1b[1\x1aC");
    assert_eq!(rows(&t), ["AbC"]);
}

#[test]
fn unknown_sequences_are_ignored() {
    let mut t = term(
        10,
        1,
        "a\x1b[?9999h\x1b[5;5y\x1b[>4;1m\x1b[=c\x1bZ\x1b%G\x1b P\x1bPq#0;2;0;0;0#0~~\x1b\\\x1b_x\x1b\\\x1b[?u\x1b[>1ub",
    );
    assert_eq!(rows(&t), ["ab"]);
    assert!(t.take_replies().is_empty());
}

#[test]
fn huge_parameters_saturate() {
    let t = term(10, 5, "\x1b[99999999999;99999999999HX");
    assert_eq!(cursor(&t), (4, 9));
    assert_eq!(cell(&t, 4, 9).ch, 'X');
    let t = term(10, 5, "\x1b[99999999999@\x1b[99999999999P\x1b[99999999999L\x1b[99999999999M\x1b[999999999X");
    assert_eq!(rows(&t), ["", "", "", "", ""]);
}

#[test]
fn split_sequences_across_feeds() {
    let s = "\x1b[1;31mré\x1b]0;t\x07\x1b[2;3H中\x1b(0q";
    let whole = term(10, 3, s);
    for split in 0..s.len() {
        let mut t = Terminal::new(10, 3, 10);
        t.feed(&s.as_bytes()[..split]);
        t.feed(&s.as_bytes()[split..]);
        assert_eq!(rows(&t), rows(&whole), "split at {split}");
        assert_eq!(cell(&t, 0, 0), cell(&whole, 0, 0));
        assert_eq!(t.title(), "t");
    }
}

#[test]
fn malformed_utf8_shows_replacement() {
    let mut t = Terminal::new(10, 1, 0);
    t.feed(b"a\xffb\xe4\xb8");
    t.feed(b"c");
    assert_eq!(rows(&t), ["a\u{FFFD}b\u{FFFD}c"]);
}

/// A tiny deterministic generator for the no-panic test.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn random_input_never_panics() {
    let pieces: &[&[u8]] = &[
        b"\x1b[",
        b"\x1b]",
        b"\x1b",
        b";",
        b":",
        b"?",
        b">",
        b"1",
        b"9",
        b"65535",
        b"0",
        b"h",
        b"l",
        b"H",
        b"r",
        b"m",
        b"J",
        b"K",
        b"L",
        b"M",
        b"P",
        b"@",
        b"S",
        b"T",
        b"X",
        b"b",
        b"\n",
        b"\r",
        b"\t",
        b"\x08",
        b"\x07",
        b"\x0e",
        b"\x0f",
        "中".as_bytes(),
        "😀".as_bytes(),
        b"\xe4",
        b"\xff",
        b"a",
        b"1049",
        b"47",
        b"6",
        b"7",
        b" q",
        b"!p",
        b"$p",
        b"(0",
        b"#8",
        b"c",
        b"D",
        b"E",
        b"8",
        b"\x18",
        b"38;2;1;2;3",
        b"38:5",
    ];
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    for round in 0..200 {
        let cols = (rng.next() % 12) as u16;
        let rows_n = (rng.next() % 8) as u16;
        let mut t = Terminal::new(cols, rows_n, (rng.next() % 5) as usize);
        for _ in 0..400 {
            let p = pieces[(rng.next() % pieces.len() as u64) as usize];
            t.feed(p);
            match rng.next() % 40 {
                0 => t.resize((rng.next() % 12) as u16, (rng.next() % 8) as u16),
                1 => t.scroll_view((rng.next() % 7) as isize - 3),
                2 => {
                    let _ = t.take_damage();
                }
                3 => {
                    let a = (rng.next() % 20) as i64 - 10;
                    let b = (rng.next() % 20) as i64 - 10;
                    let _ = t.text((a, (rng.next() % 15) as u16), (b, (rng.next() % 15) as u16));
                    let _ = t.search("a");
                }
                _ => {}
            }
            let c = t.cursor();
            assert!(c.row < t.rows() && c.col < t.cols(), "round {round}");
        }
        for r in 0..t.rows() {
            let _ = t.visible_row(r);
            let _ = t.line_wrapped(r);
        }
        let _ = t.take_replies();
    }
}
