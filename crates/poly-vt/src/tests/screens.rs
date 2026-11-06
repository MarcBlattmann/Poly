//! The alternate screen, scrollback and its view, resizing, damage.

use super::*;
use crate::Damage;

fn numbered(t: &mut Terminal, from: u32, to: u32) {
    for i in from..=to {
        t.feed(std::format!("{i}\r\n").as_bytes());
    }
}

#[test]
fn alt_screen_1049_restores_primary_and_cursor() {
    let mut t = term(10, 3, "one\r\ntwo\x1b[2;2H");
    t.feed(b"\x1b[?1049h");
    assert!(t.modes().alt_screen);
    assert_eq!(rows(&t), ["", "", ""]);
    t.feed(b"\x1b[Hfull screen\x1b[3;1Hbottom");
    assert_eq!(t.scrollback_len(), 0);
    t.feed(b"\x1b[?1049l");
    assert!(!t.modes().alt_screen);
    assert_eq!(rows(&t), ["one", "two", ""]);
    assert_eq!(cursor(&t), (1, 1));
    // Entering again starts with a clear screen.
    t.feed(b"\x1b[?1049h");
    assert_eq!(rows(&t), ["", "", ""]);
}

#[test]
fn alt_screen_47_and_1047() {
    let mut t = term(5, 2, "pri");
    t.feed(b"\x1b[?47hALT");
    assert_eq!(rows(&t), ["   AL", "T"]);
    t.feed(b"\x1b[?47l");
    assert_eq!(rows(&t), ["pri", ""]);
    // 47 does not clear: the alternate content is still there.
    t.feed(b"\x1b[?47h");
    assert_eq!(rows(&t), ["   AL", "T"]);
    // 1047 clears the alternate screen on leaving.
    t.feed(b"\x1b[?1047l");
    assert_eq!(rows(&t), ["pri", ""]);
    t.feed(b"\x1b[?47h");
    assert_eq!(rows(&t), ["", ""]);
}

#[test]
fn alt_screen_lines_do_not_enter_scrollback() {
    let mut t = term(5, 2, "\x1b[?1049h");
    numbered(&mut t, 1, 10);
    assert_eq!(t.scrollback_len(), 0);
    t.feed(b"\x1b[?1049l");
    assert_eq!(t.scrollback_len(), 0);
}

#[test]
fn separate_saved_cursors_per_screen() {
    let mut t = term(10, 5, "\x1b[2;2H\x1b7\x1b[?47h\x1b[4;4H\x1b7\x1b[H\x1b8");
    assert_eq!(cursor(&t), (3, 3));
    t.feed(b"\x1b[?47l\x1b8");
    assert_eq!(cursor(&t), (1, 1));
}

#[test]
fn scrollback_collects_and_caps() {
    let mut t = Terminal::new(5, 3, 4);
    numbered(&mut t, 1, 10);
    // Screen: 9, 10, empty; history capped at 4: 5..8.
    assert_eq!(rows(&t), ["9", "10", ""]);
    assert_eq!(t.scrollback_len(), 4);
    assert_eq!(t.scroll_count(), 8);
    assert_eq!(t.text((-4, 0), (-1, 9)), "5\n6\n7\n8");
    // No scrollback at all.
    let mut t = Terminal::new(5, 3, 0);
    numbered(&mut t, 1, 10);
    assert_eq!(t.scrollback_len(), 0);
}

#[test]
fn only_top_anchored_regions_feed_scrollback() {
    let mut t = term(5, 4, "a\r\nb\r\nc\r\nd\x1b[1;3r\x1b[3;1H\n");
    assert_eq!(t.scrollback_len(), 1);
    assert_eq!(rows(&t), ["b", "c", "", "d"]);
    t.feed(b"\x1b[2;4r\x1b[4;1H\n");
    assert_eq!(t.scrollback_len(), 1);
}

#[test]
fn view_offset_shows_history() {
    let mut t = Terminal::new(5, 3, 100);
    numbered(&mut t, 1, 6);
    // Screen: 5, 6, ""; history 1..4.
    t.scroll_view(2);
    assert_eq!(t.view_offset(), 2);
    assert_eq!(rows(&t), ["3", "4", "5"]);
    assert_eq!(t.display_to_absolute(0), -2);
    assert_eq!(t.cursor_on_display(), None);
    t.scroll_view(100);
    assert_eq!(t.view_offset(), 4);
    assert_eq!(rows(&t), ["1", "2", "3"]);
    t.scroll_view(-1);
    assert_eq!(rows(&t), ["2", "3", "4"]);
    t.reset_view();
    assert_eq!(t.view_offset(), 0);
    assert_eq!(rows(&t), ["5", "6", ""]);
    assert_eq!(t.cursor_on_display(), Some((2, 0)));
}

#[test]
fn view_stays_on_its_lines_while_output_arrives() {
    let mut t = Terminal::new(5, 3, 100);
    numbered(&mut t, 1, 6);
    t.scroll_view(2);
    let before = rows(&t);
    let _ = t.take_damage();
    numbered(&mut t, 7, 8);
    assert_eq!(t.view_offset(), 4);
    assert_eq!(rows(&t), before);
    // At offset 0 the view follows the output.
    t.reset_view();
    numbered(&mut t, 9, 9);
    assert_eq!(t.view_offset(), 0);
    assert_eq!(rows(&t), ["8", "9", ""]);
}

#[test]
fn view_at_the_top_of_a_full_scrollback_moves() {
    let mut t = Terminal::new(5, 2, 3);
    numbered(&mut t, 1, 4);
    t.scroll_view(3);
    assert_eq!(rows(&t), ["1", "2"]);
    let _ = t.take_damage();
    // "1" is dropped from the full scrollback: the view cannot stay.
    numbered(&mut t, 5, 5);
    assert_eq!(t.view_offset(), 3);
    assert_eq!(rows(&t), ["2", "3"]);
    assert_eq!(t.take_damage(), Damage::All);
}

#[test]
fn alt_screen_has_no_scrollback_view() {
    let mut t = Terminal::new(5, 2, 100);
    numbered(&mut t, 1, 5);
    t.scroll_view(2);
    t.feed(b"\x1b[?1049h");
    assert_eq!(t.view_offset(), 0);
    assert_eq!(t.scrollback_len(), 0);
    t.scroll_view(2);
    assert_eq!(t.view_offset(), 0);
    t.feed(b"\x1b[?1049l");
    assert_eq!(t.scrollback_len(), 4);
}

#[test]
fn resize_shrink_keeps_cursor_line() {
    let mut t = term(10, 5, "a\r\nb\r\nc\r\nd\r\ne");
    assert_eq!(cursor(&t), (4, 1));
    t.resize(10, 3);
    assert_eq!(rows(&t), ["c", "d", "e"]);
    assert_eq!(cursor(&t), (2, 1));
    assert_eq!(t.scrollback_len(), 2);
    assert_eq!(t.text((-2, 0), (-1, 5)), "a\nb");
    // Grow back: lines return from the scrollback.
    t.resize(10, 5);
    assert_eq!(rows(&t), ["a", "b", "c", "d", "e"]);
    assert_eq!(cursor(&t), (4, 1));
    assert_eq!(t.scrollback_len(), 0);
    t.resize(10, 7);
    assert_eq!(rows(&t), ["a", "b", "c", "d", "e", "", ""]);
    assert_eq!(cursor(&t), (4, 1));
}

#[test]
fn resize_shrink_drops_lines_below_cursor_first() {
    let mut t = term(10, 5, "a\r\nb\x1b[5;1Hz\x1b[2;2H");
    t.resize(10, 3);
    assert_eq!(rows(&t), ["a", "b", ""]);
    assert_eq!(cursor(&t), (1, 1));
    assert_eq!(t.scrollback_len(), 0);
}

#[test]
fn resize_columns_truncates_and_pads() {
    let mut t = term(6, 2, "abcdef\x1b[2;6H");
    t.resize(3, 2);
    assert_eq!(rows(&t), ["abc", ""]);
    assert_eq!(cursor(&t), (1, 2));
    t.resize(8, 2);
    assert_eq!(rows(&t), ["abc", ""]);
    assert_eq!(t.visible_row(0).len(), 8);
    t.feed(b"\x1b[1;8HX");
    assert_eq!(rows(&t), ["abc    X", ""]);
    // A wide character cut in half is removed.
    let mut t = term(4, 1, "a中");
    t.resize(2, 1);
    assert_eq!(rows(&t), ["a"]);
}

#[test]
fn resize_resets_region_and_tabs_and_clamps() {
    let mut t = term(20, 10, "\x1b[2;5r\x1b[3g\x1b[10;20H");
    t.resize(12, 4);
    assert_eq!(cursor(&t), (3, 11));
    t.feed(b"\r\t");
    assert_eq!(cursor(&t), (3, 8));
    t.feed(b"\x1b[4;1H\nX");
    // The region is the whole screen again: LF scrolls at row 4.
    assert_eq!(cursor(&t), (3, 1));
    t.resize(0, 0);
    assert_eq!((t.cols(), t.rows()), (1, 1));
    assert_eq!(cursor(&t), (0, 0));
}

#[test]
fn resize_while_on_the_alt_screen() {
    let mut t = term(10, 4, "1\r\n2\r\n3\r\n4");
    t.feed(b"\x1b[?1049h\x1b[4;1Halt");
    t.resize(10, 2);
    assert_eq!(cursor(&t), (1, 3));
    assert_eq!(rows(&t), ["", "alt"]);
    t.feed(b"\x1b[?1049l");
    assert_eq!(rows(&t), ["3", "4"]);
    assert_eq!(cursor(&t), (1, 1));
    assert_eq!(t.scrollback_len(), 2);
}

#[test]
fn damage_tracks_rows() {
    let mut t = term(5, 4, "");
    assert_eq!(t.take_damage(), Damage::All);
    assert!(t.take_damage().is_empty());
    t.feed(b"\x1b[3;1Hx");
    let d = t.take_damage();
    // Row 0 (where the cursor was) and row 2 (written, cursor now).
    assert!(d.contains(0) && d.contains(2) && !d.contains(1) && !d.contains(3));
    t.feed(b"\x1b[3;3H");
    let d = t.take_damage();
    assert!(!d.contains(0) && d.contains(2));
    t.feed(b"\x1b[2;1H\x1b[K");
    let d = t.take_damage();
    assert!(d.contains(1) && d.contains(2) && !d.contains(3));
    t.scroll_view(0);
    assert!(t.take_damage().is_empty());
    t.feed(b"\x1b[?1049h");
    assert_eq!(t.take_damage(), Damage::All);
    t.resize(6, 4);
    assert_eq!(t.take_damage(), Damage::All);
    t.feed(b"\x1b[?5h");
    assert_eq!(t.take_damage(), Damage::All);
}

#[test]
fn damage_when_scrolled_back_maps_to_display_rows() {
    let mut t = Terminal::new(5, 4, 100);
    numbered(&mut t, 1, 6);
    t.scroll_view(2);
    let _ = t.take_damage();
    // Screen row 0 shows on display row 2.
    t.feed(b"\x1b[1;1HX");
    let d = t.take_damage();
    assert!(d.contains(2) && !d.contains(0) && !d.contains(1));
}
