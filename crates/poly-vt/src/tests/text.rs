//! Selection text and search.

use super::*;

#[test]
fn text_on_one_line() {
    let t = term(10, 2, "hello world");
    assert_eq!(t.text((0, 0), (0, 4)), "hello");
    assert_eq!(t.text((0, 4), (0, 0)), "hello");
    assert_eq!(t.text((0, 6), (0, 200)), "worl");
}

#[test]
fn text_trims_and_joins_lines() {
    let t = term(10, 3, "ab   \r\ncd\r\n");
    assert_eq!(t.text((0, 0), (2, 9)), "ab\ncd\n");
    assert_eq!(t.text((0, 1), (1, 0)), "b\nc");
}

#[test]
fn text_joins_soft_wrapped_lines() {
    let t = term(4, 4, "abcdefg h\r\nxy");
    assert_eq!(rows(&t), ["abcd", "efg", "h", "xy"]);
    // Rows 0-2 are one logical line (with the space at the wrap kept).
    assert_eq!(t.text((0, 0), (2, 3)), "abcdefg h");
    assert_eq!(t.text((0, 2), (1, 1)), "cdef");
}

#[test]
fn text_with_wide_chars() {
    let t = term(10, 1, "a中b");
    assert_eq!(t.text((0, 0), (0, 9)), "a中b");
    // Selecting the continuation cell alone gives nothing for it.
    assert_eq!(t.text((0, 2), (0, 3)), "b");
}

#[test]
fn text_across_scrollback() {
    let mut t = Terminal::new(6, 2, 100);
    t.feed(b"first\r\nsecond\r\nthird\r\nfour");
    assert_eq!(t.scrollback_len(), 2);
    assert_eq!(t.text((-2, 0), (1, 5)), "first\nsecond\nthird\nfour");
    assert_eq!(t.text((-1, 2), (0, 1)), "cond\nth");
    // Out-of-range lines are clamped.
    assert_eq!(t.text((-100, 0), (100, 0)), "first\nsecond\nthird\nfour");
    assert_eq!(t.text((50, 0), (60, 0)), "");
    assert_eq!(t.text((i64::MIN, 0), (i64::MAX, u16::MAX)), "first\nsecond\nthird\nfour");
    // Through the view.
    t.scroll_view(1);
    let line = t.display_to_absolute(0);
    assert_eq!(t.text((line, 0), (line, 9)), "second");
}

#[test]
fn wrapped_line_in_scrollback_still_joins() {
    let mut t = Terminal::new(3, 2, 100);
    t.feed(b"abcdef\r\nx\r\ny");
    assert_eq!(t.scrollback_len(), 2);
    assert_eq!(t.text((-2, 0), (-1, 2)), "abcdef");
    t.scroll_view(2);
    assert!(t.line_wrapped(0));
    assert!(!t.line_wrapped(1));
}

#[test]
fn search_finds_the_newest_match() {
    let mut t = Terminal::new(10, 2, 100);
    t.feed(b"foo bar\r\nbaz foo\r\nqux");
    assert_eq!(t.search("foo"), Some((0, 4)));
    assert_eq!(t.search_before("foo", (0, 4)), Some((-1, 0)));
    assert_eq!(t.search_before("foo", (-1, 0)), None);
    assert_eq!(t.search("qux"), Some((1, 0)));
    assert_eq!(t.search("nope"), None);
    assert_eq!(t.search(""), None);
}
