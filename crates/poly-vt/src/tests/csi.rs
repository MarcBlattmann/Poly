//! Each control sequence, and the replies.

use super::*;
use crate::{CursorShape, MouseMode};

#[test]
fn cursor_movement() {
    let mut t = term(10, 6, "\x1b[3;4H");
    assert_eq!(cursor(&t), (2, 3));
    t.feed(b"\x1b[A");
    assert_eq!(cursor(&t), (1, 3));
    t.feed(b"\x1b[2B");
    assert_eq!(cursor(&t), (3, 3));
    t.feed(b"\x1b[3C");
    assert_eq!(cursor(&t), (3, 6));
    t.feed(b"\x1b[0D");
    assert_eq!(cursor(&t), (3, 5));
    t.feed(b"\x1b[99A");
    assert_eq!(cursor(&t), (0, 5));
    t.feed(b"\x1b[99C");
    assert_eq!(cursor(&t), (0, 9));
    t.feed(b"\x1b[2E");
    assert_eq!(cursor(&t), (2, 0));
    t.feed(b"\x1b[5G\x1b[F");
    assert_eq!(cursor(&t), (1, 0));
    t.feed(b"\x1b[7`");
    assert_eq!(cursor(&t), (1, 6));
    t.feed(b"\x1b[2a");
    assert_eq!(cursor(&t), (1, 8));
    t.feed(b"\x1b[5d");
    assert_eq!(cursor(&t), (4, 8));
    t.feed(b"\x1b[e");
    assert_eq!(cursor(&t), (5, 8));
    t.feed(b"\x1b[H");
    assert_eq!(cursor(&t), (0, 0));
    t.feed(b"\x1b[;3f");
    assert_eq!(cursor(&t), (0, 2));
    t.feed(b"\x1b[0;0H");
    assert_eq!(cursor(&t), (0, 0));
}

#[test]
fn cursor_up_down_stop_at_margins() {
    let mut t = term(5, 10, "\x1b[3;6r\x1b[4;1H\x1b[9A");
    assert_eq!(cursor(&t), (2, 0));
    t.feed(b"\x1b[9B");
    assert_eq!(cursor(&t), (5, 0));
    // Outside the region, the screen edges.
    t.feed(b"\x1b[8;1H\x1b[9B");
    assert_eq!(cursor(&t), (9, 0));
    t.feed(b"\x1b[2;1H\x1b[9A");
    assert_eq!(cursor(&t), (0, 0));
}

#[test]
fn tabs() {
    let mut t = term(30, 1, "\x1b[2I");
    assert_eq!(cursor(&t), (0, 16));
    t.feed(b"\x1b[Z");
    assert_eq!(cursor(&t), (0, 8));
    t.feed(b"\x1b[3Z");
    assert_eq!(cursor(&t), (0, 0));
    // Set a stop at 3, clear the one at 8.
    t.feed(b"\x1b[4G\x1bH\x1b[9G\x1b[g\r\t");
    assert_eq!(cursor(&t), (0, 3));
    t.feed(b"\t");
    assert_eq!(cursor(&t), (0, 16));
    t.feed(b"\x1b[3g\r\t");
    assert_eq!(cursor(&t), (0, 29));
}

#[test]
fn erase_in_line() {
    let t = term(6, 1, "abcdef\x1b[3G\x1b[K");
    assert_eq!(rows(&t), ["ab"]);
    let t = term(6, 1, "abcdef\x1b[3G\x1b[1K");
    assert_eq!(rows(&t), ["   def"]);
    let t = term(6, 1, "abcdef\x1b[3G\x1b[2K");
    assert_eq!(rows(&t), [""]);
    assert_eq!(cursor(&t), (0, 2));
}

#[test]
fn erase_in_display() {
    let fill = "aaa\r\nbbb\r\nccc\x1b[2;2H";
    let t = term(3, 3, &[fill, "\x1b[J"].concat());
    assert_eq!(rows(&t), ["aaa", "b", ""]);
    let t = term(3, 3, &[fill, "\x1b[1J"].concat());
    assert_eq!(rows(&t), ["", "  b", "ccc"]);
    let t = term(3, 3, &[fill, "\x1b[2J"].concat());
    assert_eq!(rows(&t), ["", "", ""]);
    assert_eq!(cursor(&t), (1, 1));
}

#[test]
fn erase_uses_background_colour() {
    let t = term(4, 2, "\x1b[44m\x1b[2J\x1b[42mx\x1b[K");
    assert_eq!(cell(&t, 1, 0).bg, crate::Color::Indexed(4));
    assert_eq!(cell(&t, 0, 0).bg, crate::Color::Indexed(2));
    assert_eq!(cell(&t, 0, 3).bg, crate::Color::Indexed(2));
    assert_eq!(cell(&t, 0, 3).ch, ' ');
}

#[test]
fn erase_scrollback() {
    let mut t = term(3, 2, "1\r\n2\r\n3\r\n4");
    assert_eq!(t.scrollback_len(), 2);
    t.feed(b"\x1b[3J");
    assert_eq!(t.scrollback_len(), 0);
    assert_eq!(rows(&t), ["3", "4"]);
}

#[test]
fn insert_delete_erase_chars() {
    let t = term(6, 1, "abcdef\x1b[2G\x1b[2@");
    assert_eq!(rows(&t), ["a  bcd"]);
    let t = term(6, 1, "abcdef\x1b[2G\x1b[2P");
    assert_eq!(rows(&t), ["adef"]);
    let t = term(6, 1, "abcdef\x1b[2G\x1b[2X");
    assert_eq!(rows(&t), ["a  def"]);
    assert_eq!(cursor(&t), (0, 1));
    let t = term(6, 1, "abcdef\x1b[2G\x1b[X");
    assert_eq!(rows(&t), ["a cdef"]);
}

#[test]
fn insert_mode() {
    let t = term(6, 1, "abcd\x1b[2G\x1b[4hXY\x1b[4lZ");
    assert_eq!(rows(&t), ["aXYZcd"]);
}

#[test]
fn insert_delete_lines() {
    let fill = "1\r\n2\r\n3\r\n4\x1b[2;1H";
    let t = term(3, 4, &[fill, "\x1b[L"].concat());
    assert_eq!(rows(&t), ["1", "", "2", "3"]);
    let t = term(3, 4, &[fill, "\x1b[2M"].concat());
    assert_eq!(rows(&t), ["1", "4", "", ""]);
    // DL never feeds the scrollback, even at row 0.
    let mut t = term(3, 4, "1\r\n2\x1b[H\x1b[M");
    assert_eq!(rows(&t), ["2", "", "", ""]);
    assert_eq!(t.scrollback_len(), 0);
    t.feed(b"x\x1b[3G\x1b[L");
    assert_eq!(cursor(&t), (0, 0));
}

#[test]
fn scroll_regions_with_il_dl_ri() {
    let fill = "1\r\n2\r\n3\r\n4\r\n5";
    // Region rows 2-4.
    let t = term(3, 5, &[fill, "\x1b[2;4r\x1b[2;1H\x1b[L"].concat());
    assert_eq!(rows(&t), ["1", "", "2", "3", "5"]);
    let t = term(3, 5, &[fill, "\x1b[2;4r\x1b[3;1H\x1b[M"].concat());
    assert_eq!(rows(&t), ["1", "2", "4", "", "5"]);
    // IL outside the region does nothing.
    let t = term(3, 5, &[fill, "\x1b[2;4r\x1b[5;1H\x1b[L"].concat());
    assert_eq!(rows(&t), ["1", "2", "3", "4", "5"]);
    // RI at the top margin scrolls the region down.
    let t = term(3, 5, &[fill, "\x1b[2;4r\x1b[2;1H\x1bM"].concat());
    assert_eq!(rows(&t), ["1", "", "2", "3", "5"]);
    // LF at the bottom margin scrolls the region up, nothing to history.
    let t = term(3, 5, &[fill, "\x1b[2;4r\x1b[4;1H\nX"].concat());
    assert_eq!(rows(&t), ["1", "3", "4", "X", "5"]);
    assert_eq!(t.scrollback_len(), 0);
    // SU / SD.
    let t = term(3, 5, &[fill, "\x1b[2;4r\x1b[S"].concat());
    assert_eq!(rows(&t), ["1", "3", "4", "", "5"]);
    let t = term(3, 5, &[fill, "\x1b[2;4r\x1b[2T"].concat());
    assert_eq!(rows(&t), ["1", "", "", "2", "5"]);
}

#[test]
fn ind_nel_ri() {
    let mut t = term(5, 3, "ab\x1bDc\x1bEd");
    assert_eq!(rows(&t), ["ab", "  c", "d"]);
    t.feed(b"\x1b[H\x1bM");
    assert_eq!(rows(&t), ["", "ab", "  c"]);
    assert_eq!(cursor(&t), (0, 0));
}

#[test]
fn scroll_region_homes_cursor_and_rejects_bad_ranges() {
    let mut t = term(5, 5, "\x1b[3;3H\x1b[2;4r");
    assert_eq!(cursor(&t), (0, 0));
    t.feed(b"\x1b[3;3H\x1b[4;2r");
    assert_eq!(cursor(&t), (2, 2));
    t.feed(b"\x1b[r\x1b[5;1H\n");
    // Whole screen again: LF at the last row scrolls.
    assert_eq!(cursor(&t), (4, 0));
}

#[test]
fn origin_mode() {
    let mut t = term(5, 6, "\x1b[2;5r\x1b[?6h");
    assert_eq!(cursor(&t), (1, 0));
    t.feed(b"\x1b[2;2H");
    assert_eq!(cursor(&t), (2, 1));
    t.feed(b"\x1b[99;1H");
    assert_eq!(cursor(&t), (4, 0));
    t.feed(b"\x1b[6n");
    assert_eq!(t.take_replies(), b"\x1b[4;1R");
    t.feed(b"\x1b[?6l");
    assert_eq!(cursor(&t), (0, 0));
}

#[test]
fn status_and_identification_replies() {
    let mut t = term(10, 5, "\x1b[5n");
    assert_eq!(t.take_replies(), b"\x1b[0n");
    t.feed(b"\x1b[3;7H\x1b[6n");
    assert_eq!(t.take_replies(), b"\x1b[3;7R");
    t.feed(b"\x1b[?6n");
    assert_eq!(t.take_replies(), b"\x1b[?3;7R");
    t.feed(b"\x1b[c\x1b[0c");
    assert_eq!(t.take_replies(), b"\x1b[?62;22c\x1b[?62;22c");
    t.feed(b"\x1b[>c");
    assert_eq!(t.take_replies(), b"\x1b[>1;10;0c");
    assert!(t.take_replies().is_empty());
    t.feed(b"\x1b[18t\x1b[22;0t");
    assert_eq!(t.take_replies(), b"\x1b[8;5;10t");
    t.feed(b"\x1b[>q");
    let v = t.take_replies();
    assert!(v.starts_with(b"\x1bP>|poly-vt ") && v.ends_with(b"\x1b\\"));
}

#[test]
fn dsr_reports_last_column_with_pending_wrap() {
    let mut t = term(5, 2, "abcde\x1b[6n");
    assert_eq!(t.take_replies(), b"\x1b[1;5R");
}

#[test]
fn mode_reports() {
    let mut t = term(10, 5, "\x1b[?2004h\x1b[?2004$p\x1b[?1$p\x1b[?4242$p\x1b[4$p");
    assert_eq!(t.take_replies(), b"\x1b[?2004;1$y\x1b[?1;2$y\x1b[?4242;0$y\x1b[4;2$y");
}

#[test]
fn colour_queries() {
    let mut t = term(10, 5, "");
    t.set_default_colors((0xff, 0x80, 0), (0x10, 0x20, 0x30));
    t.feed(b"\x1b]10;?\x07\x1b]11;?\x1b\\");
    assert_eq!(t.take_replies(), b"\x1b]10;rgb:ffff/8080/0000\x07\x1b]11;rgb:1010/2020/3030\x1b\\".as_slice());
}

#[test]
fn dec_modes() {
    let mut t = term(10, 5, "\x1b[?1h\x1b[?2004h\x1b[?1000h\x1b[?1006h\x1b[?1004h\x1b[?5h\x1b=");
    let m = *t.modes();
    assert!(m.app_cursor && m.bracketed_paste && m.sgr_mouse && m.focus_events && m.reverse_video && m.app_keypad);
    assert_eq!(m.mouse, MouseMode::Press);
    t.feed(b"\x1b[?1002h");
    assert_eq!(t.modes().mouse, MouseMode::Drag);
    t.feed(b"\x1b[?1003h");
    assert_eq!(t.modes().mouse, MouseMode::Any);
    t.feed(b"\x1b[?1000l");
    assert_eq!(t.modes().mouse, MouseMode::Any);
    t.feed(b"\x1b[?1003l");
    assert_eq!(t.modes().mouse, MouseMode::None);
    t.feed(b"\x1b[?1;2004;1004;5l\x1b>");
    let m = *t.modes();
    assert!(!m.app_cursor && !m.bracketed_paste && !m.focus_events && !m.reverse_video && !m.app_keypad);
    t.feed(b"\x1b[?25l");
    assert!(!t.cursor().visible);
    t.feed(b"\x1b[?25h\x1b[?12h");
    assert!(t.cursor().visible && t.cursor().blink);
}

#[test]
fn cursor_styles() {
    let mut t = term(10, 5, "\x1b[6 q");
    assert_eq!((t.cursor().shape, t.cursor().blink), (CursorShape::Bar, false));
    t.feed(b"\x1b[3 q");
    assert_eq!((t.cursor().shape, t.cursor().blink), (CursorShape::Underline, true));
    t.feed(b"\x1b[2 q");
    assert_eq!((t.cursor().shape, t.cursor().blink), (CursorShape::Block, false));
    t.feed(b"\x1b[ q");
    assert_eq!((t.cursor().shape, t.cursor().blink), (CursorShape::Block, true));
}

#[test]
fn save_restore_cursor() {
    let mut t = term(10, 5, "\x1b[2;3H\x1b[s\x1b[5;5H\x1b[u");
    assert_eq!(cursor(&t), (1, 2));
    t.feed(b"\x1b[4;4H\x1b7\x1b[H\x1b8");
    assert_eq!(cursor(&t), (3, 3));
    // `?1048`.
    t.feed(b"\x1b[?1048h\x1b[H\x1b[?1048l");
    assert_eq!(cursor(&t), (3, 3));
    // Restoring with nothing saved homes the cursor.
    let t = term(10, 5, "\x1b[3;3H\x1b8");
    assert_eq!(cursor(&t), (0, 0));
}

#[test]
fn soft_and_full_reset() {
    let mut t = term(10, 3, "abc\x1b[?25l\x1b[4h\x1b[?6h\x1b[2;3r\x1b[1m\x1b[!p");
    assert!(t.cursor().visible);
    assert!(!t.modes().insert && !t.modes().origin);
    assert_eq!(rows(&t)[0], "abc");
    t.feed(b"\x1b[3;1Hx");
    assert!(cell(&t, 2, 0).attrs.is_empty());
    t.feed(b"\x1b[?1049h\x1b[?2004hzz\x1bc");
    assert_eq!(rows(&t), ["", "", ""]);
    assert!(!t.modes().alt_screen && !t.modes().bracketed_paste);
    assert_eq!(cursor(&t), (0, 0));
}

#[test]
fn decaln() {
    let t = term(3, 2, "\x1b#8");
    assert_eq!(rows(&t), ["EEE", "EEE"]);
}

#[test]
fn xtwinops_ignored() {
    let mut t = term(10, 2, "a\x1b[8;50;100tb\x1b[22;2t\x1b[23;2tc");
    assert_eq!(rows(&t), ["abc", ""]);
    assert_eq!((t.cols(), t.rows()), (10, 2));
    assert!(t.take_replies().is_empty());
}
