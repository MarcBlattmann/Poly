use crate::{Edit, TermDecoder};
use std::vec::Vec;

fn decode(bytes: &[u8]) -> Vec<Edit> {
    let mut d = TermDecoder::new();
    bytes.iter().filter_map(|&b| d.feed(b)).collect()
}

fn text(s: &str) -> Vec<Edit> {
    s.chars().map(Edit::Insert).collect()
}

#[test]
fn printable_ascii_and_utf8() {
    assert_eq!(decode(b"ls -l"), text("ls -l"));
    assert_eq!(decode("äö€😀".as_bytes()), text("äö€😀"));
}

#[test]
fn malformed_utf8_is_dropped() {
    // A continuation byte alone, a truncated sequence followed by ASCII,
    // an overlong encoding of '/', a surrogate, past U+10FFFF, and C0/F5.
    assert_eq!(decode(b"\x80a"), text("a"));
    assert_eq!(decode(b"\xe2\x82b"), text("b"));
    assert_eq!(decode(b"\xe0\x80\xaf"), []);
    assert_eq!(decode(b"\xed\xa0\x80"), []);
    assert_eq!(decode(b"\xf4\x90\x80\x80"), []);
    assert_eq!(decode(b"\xc0\xaf\xf5c"), text("c"));
    // A truncated sequence followed by an escape sequence keeps the escape.
    assert_eq!(decode(b"\xc3\x1b[D"), [Edit::Left]);
    // C1 control characters encoded in UTF-8 are not text.
    assert_eq!(decode("\u{85}".as_bytes()), []);
}

#[test]
fn enter_backspace_tab_and_control_keys() {
    assert_eq!(decode(b"\r"), [Edit::Enter]);
    assert_eq!(decode(b"\n"), [Edit::Enter]);
    assert_eq!(decode(b"a\r\nb"), [Edit::Insert('a'), Edit::Enter, Edit::Insert('b')], "CR LF is one Enter");
    assert_eq!(decode(b"\r\r"), [Edit::Enter, Edit::Enter]);
    assert_eq!(decode(b"\n\n"), [Edit::Enter, Edit::Enter]);
    assert_eq!(decode(b"\x7f\x08"), [Edit::Backspace, Edit::Backspace]);
    assert_eq!(decode(b"\t"), [Edit::Complete]);
    assert_eq!(decode(b"\x01\x05\x03\x0c\x0b\x15\x17"), {
        use Edit::*;
        [Home, End, Cancel, ClearScreen, KillToEnd, KillToStart, KillWordBefore]
    });
    assert_eq!(decode(b"\x00\x1a\x1c\x1f"), [], "NUL, Ctrl-Z and the other separators do nothing");
}

#[test]
fn cursor_keys_in_both_modes() {
    use Edit::*;
    assert_eq!(decode(b"\x1b[A\x1b[B\x1b[C\x1b[D"), [HistoryPrev, HistoryNext, Right, Left]);
    assert_eq!(decode(b"\x1bOA\x1bOB\x1bOC\x1bOD\x1bOH\x1bOF"), [HistoryPrev, HistoryNext, Right, Left, Home, End]);
    assert_eq!(decode(b"\x1b[1;5C\x1b[1;5D\x1b[1;3D\x1b[5C"), [WordRight, WordLeft, WordLeft, WordRight]);
    assert_eq!(decode(b"\x1b[1;2D"), [Left], "Shift-Left is just Left");
    assert_eq!(decode(b"\x1bb\x1bf"), [WordLeft, WordRight]);
}

#[test]
fn home_end_delete_variants() {
    use Edit::*;
    assert_eq!(decode(b"\x1b[H\x1b[1~\x1b[7~"), [Home, Home, Home]);
    assert_eq!(decode(b"\x1b[F\x1b[4~\x1b[8~"), [End, End, End]);
    assert_eq!(decode(b"\x1b[3~"), [Delete]);
}

#[test]
fn unknown_sequences_are_dropped_whole() {
    // Insert, Page Up, F5, a CSI with a long parameter, and an SS3 F1.
    assert_eq!(decode(b"\x1b[2~\x1b[5~\x1b[15~x"), text("x"));
    assert_eq!(decode(b"\x1b[1;2;3;4;5;6;7;8;9Dy"), text("y"));
    assert_eq!(decode(b"\x1bOPz"), text("z"));
}

#[test]
fn a_broken_escape_falls_back_to_the_next_byte() {
    // ESC followed by a letter that starts nothing: the letter counts.
    assert_eq!(decode(b"\x1bxa"), text("xa"));
    assert_eq!(decode(b"\x1b\x1b[D"), [Edit::Left], "a second ESC starts over");
    // A control byte inside a CSI aborts it and is taken on its own.
    assert_eq!(decode(b"\x1b[1\r"), [Edit::Enter]);
}

#[test]
fn bytes_can_arrive_one_at_a_time() {
    let mut d = TermDecoder::default();
    let mut got = Vec::new();
    for chunk in [&b"\x1b"[..], b"[", b"1", b";5", b"C", b"\xe2", b"\x82", b"\xac", b"\r", b"\n"] {
        got.extend(chunk.iter().filter_map(|&b| d.feed(b)));
    }
    assert_eq!(got, [Edit::WordRight, Edit::Insert('€'), Edit::Enter]);
}
