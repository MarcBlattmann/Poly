mod decode;
mod model;
mod term;
mod words;

use super::*;
use std::string::{String, ToString};
use std::vec::Vec;
use term::Term;

/// A colored prompt, as the shell uses, and what it looks like.
const PROMPT: &str = "\x1b[1;32mpoly>\x1b[0m ";
const SHOWN: &str = "poly> ";

/// An editor attached to a terminal. Every edit checks that the screen shows
/// the prompt and the line with the cursor in the right place.
struct Rig {
    ed: Editor,
    term: Term,
    /// What the last edit wrote.
    out: String,
}

impl Rig {
    fn new() -> Rig {
        Rig::with_history(8)
    }

    fn with_history(n: usize) -> Rig {
        let mut rig = Rig { ed: Editor::new(n), term: Term::new(), out: String::new() };
        rig.start();
        rig
    }

    fn start(&mut self) {
        let mut out = String::new();
        self.ed.start(PROMPT, &mut out);
        self.term.feed(&out);
        self.check();
    }

    fn edit_with(&mut self, e: Edit, complete: &mut dyn Complete) -> Action {
        let before = self.ed.line();
        let mut out = String::new();
        let action = self.ed.handle(e, &mut out, complete);
        self.term.feed(&out);
        self.out = out;
        match &action {
            Action::Pending => self.check(),
            Action::Done(line) => {
                assert_eq!(line, &before);
                self.check_finished(line);
            }
            Action::Cancelled => self.check_finished(&std::format!("{before}^C")),
        }
        action
    }

    fn edit(&mut self, e: Edit) -> Action {
        self.edit_with(e, &mut NoCompletion)
    }

    fn edits(&mut self, es: &[Edit]) {
        for &e in es {
            assert_eq!(self.edit(e), Action::Pending, "{e:?}");
        }
    }

    fn type_str(&mut self, s: &str) {
        for c in s.chars() {
            self.edit(Edit::Insert(c));
        }
    }

    /// Submit and start the next line.
    fn enter(&mut self) -> String {
        let Action::Done(line) = self.edit(Edit::Enter) else { panic!("Enter did not finish the line") };
        self.start();
        line
    }

    fn check(&self) {
        let line = self.ed.line();
        assert_eq!(self.term.current(), std::format!("{SHOWN}{line}"), "screen shows the prompt and the line");
        assert_eq!(self.term.col, SHOWN.chars().count() + self.ed.cursor(), "cursor on screen in {line:?}");
        assert!(self.ed.cursor() <= line.chars().count());
    }

    /// After Enter or Ctrl-C: the finished row shows `text`, the cursor is
    /// at the start of the next row, and the editor is empty.
    fn check_finished(&self, text: &str) {
        assert_eq!(self.term.row_text(self.term.row - 1), std::format!("{SHOWN}{text}"));
        assert_eq!((self.term.col, self.term.current().as_str()), (0, ""));
        assert_eq!((self.ed.line().as_str(), self.ed.cursor()), ("", 0));
    }
}

fn completer(words: &'static [&'static str]) -> impl FnMut(&str) -> Completion {
    move |before: &str| {
        let start = before.rfind(' ').map_or(0, |i| i + 1);
        Completion { start, candidates: words.iter().map(ToString::to_string).collect() }
    }
}

#[test]
fn control_keys_map_to_edits() {
    assert_eq!(Edit::control('a'), Some(Edit::Home));
    assert_eq!(Edit::control('E'), Some(Edit::End));
    assert_eq!(Edit::control('c'), Some(Edit::Cancel));
    assert_eq!(Edit::control('w'), Some(Edit::KillWordBefore));
    assert_eq!(Edit::control('m'), Some(Edit::Enter));
    assert_eq!(Edit::control('i'), Some(Edit::Complete));
    assert_eq!(Edit::control('z'), None);
    assert_eq!(Edit::control('1'), None);
}

#[test]
fn typing_appends_without_redrawing() {
    let mut r = Rig::new();
    r.type_str("ls");
    assert_eq!(r.out, "s", "appending writes just the character");
    assert_eq!(r.ed.line(), "ls");
    assert_eq!(r.enter(), "ls");
}

#[test]
fn inserting_in_the_middle_redraws_the_tail() {
    let mut r = Rig::new();
    r.type_str("hllo");
    r.edits(&[Edit::Home, Edit::Right]);
    r.type_str("e");
    assert_eq!(r.out, "ello\x1b[3D");
    assert_eq!((r.ed.line().as_str(), r.ed.cursor()), ("hello", 2));
}

#[test]
fn backspace_and_delete() {
    let mut r = Rig::new();
    r.type_str("abcd");
    r.edit(Edit::Backspace);
    assert_eq!(r.out, "\x1b[D\x1b[K");
    r.edits(&[Edit::Left, Edit::Left, Edit::Delete]);
    assert_eq!(r.ed.line(), "ac");
    r.edits(&[Edit::Backspace, Edit::Backspace]); // the second one at column 0 does nothing
    assert_eq!((r.ed.line().as_str(), r.ed.cursor()), ("c", 0));
    r.edits(&[Edit::End, Edit::Delete, Edit::Right]);
    assert_eq!(r.out, "", "Right at the end writes nothing");
    assert_eq!(r.ed.line(), "c");
}

#[test]
fn multibyte_characters_move_one_column_each() {
    let mut r = Rig::new();
    r.type_str("ä€x");
    assert_eq!(r.ed.cursor(), 3);
    r.edits(&[Edit::Left, Edit::Left]);
    assert_eq!(r.ed.cursor(), 1);
    r.type_str("ü");
    assert_eq!(r.ed.line(), "äü€x");
    r.edit(Edit::Delete);
    assert_eq!(r.ed.line(), "äüx");
    r.edit(Edit::Backspace);
    assert_eq!((r.ed.line().as_str(), r.ed.cursor()), ("äx", 1));
    r.type_str("😀");
    assert_eq!(r.enter(), "ä😀x");
}

#[test]
fn control_characters_are_not_inserted() {
    let mut r = Rig::new();
    r.edits(&[Edit::Insert('\u{7}'), Edit::Insert('\n'), Edit::Insert('\u{9b}')]);
    assert_eq!(r.ed.line(), "");
}

#[test]
fn word_movement() {
    let mut r = Rig::new();
    r.type_str("test  böse_x--wört");
    r.edit(Edit::WordLeft);
    assert_eq!(r.ed.cursor(), 14, "start of wört");
    r.edit(Edit::WordLeft);
    assert_eq!(r.ed.cursor(), 6, "start of böse_x, skipping the dashes");
    r.edit(Edit::WordLeft);
    assert_eq!(r.ed.cursor(), 0);
    r.edit(Edit::WordLeft);
    assert_eq!(r.ed.cursor(), 0);
    r.edit(Edit::WordRight);
    assert_eq!(r.ed.cursor(), 4, "end of test");
    r.edit(Edit::WordRight);
    assert_eq!(r.ed.cursor(), 12, "end of böse_x");
    r.edits(&[Edit::WordRight, Edit::WordRight]);
    assert_eq!(r.ed.cursor(), 18);
}

#[test]
fn kill_commands() {
    let mut r = Rig::new();
    r.type_str("echo one two");
    r.edit(Edit::KillWordBefore);
    assert_eq!(r.ed.line(), "echo one ");
    r.type_str("  ");
    r.edit(Edit::KillWordBefore);
    assert_eq!(r.ed.line(), "echo ", "trailing spaces go with the word");
    r.type_str("three");
    r.edits(&[Edit::WordLeft, Edit::KillToStart]);
    assert_eq!((r.ed.line().as_str(), r.ed.cursor()), ("three", 0));
    r.edits(&[Edit::Right, Edit::Right, Edit::KillToEnd]);
    assert_eq!(r.ed.line(), "th");
    r.edits(&[Edit::Home, Edit::KillWordBefore, Edit::KillToStart]);
    assert_eq!(r.ed.line(), "th", "nothing before the cursor");
}

#[test]
fn enter_submits_from_anywhere_and_records_history() {
    let mut r = Rig::new();
    r.type_str("uptime");
    r.edit(Edit::Home);
    assert_eq!(r.enter(), "uptime");
    assert_eq!(r.ed.history().get(0), Some("uptime"));
    assert_eq!(r.enter(), "", "an empty line is submitted");
    assert_eq!(r.ed.history().len(), 1, "but not recorded");
}

#[test]
fn cancel_discards_the_line() {
    let mut r = Rig::new();
    r.type_str("reboot");
    r.edit(Edit::Left);
    assert_eq!(r.edit(Edit::Cancel), Action::Cancelled);
    assert!(r.ed.history().is_empty());
    r.start();
    r.type_str("x");
}

#[test]
fn history_navigation_keeps_the_line_being_typed() {
    let mut r = Rig::new();
    for l in ["one", "two", "three"] {
        r.type_str(l);
        r.enter();
    }
    r.type_str("draft");
    r.edit(Edit::HistoryPrev);
    assert_eq!(r.ed.line(), "three");
    r.edit(Edit::HistoryPrev);
    assert_eq!(r.ed.line(), "two");
    r.edits(&[Edit::HistoryPrev, Edit::HistoryPrev]);
    assert_eq!(r.ed.line(), "one", "stops at the oldest entry");
    r.edit(Edit::HistoryNext);
    assert_eq!(r.ed.line(), "two");
    r.edits(&[Edit::HistoryNext, Edit::HistoryNext]);
    assert_eq!((r.ed.line().as_str(), r.ed.cursor()), ("draft", 5), "the draft is back");
    r.edit(Edit::HistoryNext);
    assert_eq!(r.ed.line(), "draft", "nothing newer than the draft");
}

#[test]
fn a_recalled_entry_can_be_edited_and_submitted() {
    let mut r = Rig::new();
    r.type_str("echo hi");
    r.enter();
    r.edit(Edit::HistoryPrev);
    r.edit(Edit::Backspace);
    r.type_str("o");
    assert_eq!(r.enter(), "echo ho");
    assert_eq!(r.ed.history().iter().collect::<Vec<_>>(), ["echo hi", "echo ho"]);
    // The entry itself was not changed by the edit.
    r.edits(&[Edit::HistoryPrev, Edit::HistoryPrev]);
    assert_eq!(r.ed.line(), "echo hi");
}

#[test]
fn history_without_entries_or_capacity() {
    let mut r = Rig::with_history(0);
    r.type_str("x");
    r.enter();
    r.edits(&[Edit::HistoryPrev, Edit::HistoryNext]);
    assert_eq!(r.ed.line(), "");
}

#[test]
fn history_keeps_the_newest_without_repeats() {
    let mut h = History::new(3);
    assert!(h.push("a"));
    assert!(!h.push("a"), "repeat of the newest entry");
    assert!(!h.push("   "), "blank");
    assert!(h.push("b"));
    assert!(h.push("a"), "an older duplicate is fine");
    assert!(h.push("c"));
    assert_eq!(h.iter().collect::<Vec<_>>(), ["b", "a", "c"]);
    assert_eq!((h.get(0), h.get(2), h.get(3)), (Some("c"), Some("b"), None));
    assert_eq!((h.len(), h.capacity()), (3, 3));
    h.clear();
    assert!(h.is_empty());
}

#[test]
fn tab_completes_a_unique_word_and_adds_a_space() {
    let mut r = Rig::new();
    let mut c = completer(&["help", "uptime", "lspci"]);
    r.type_str("up");
    r.edit_with(Edit::Complete, &mut c);
    assert_eq!(r.ed.line(), "uptime ");
    r.type_str("x");
    assert_eq!(r.ed.line(), "uptime x");
}

#[test]
fn tab_completes_the_common_prefix_then_lists() {
    let mut r = Rig::new();
    let mut c = completer(&["hello", "help", "history", "uptime", "help"]);
    r.type_str("h");
    r.edit_with(Edit::Complete, &mut c);
    assert_eq!(r.ed.line(), "h", "no common prefix beyond h");
    assert_eq!(r.out, "");
    let rows = r.term.rows();
    r.edit_with(Edit::Complete, &mut c);
    assert_eq!(r.term.row_text(rows), "hello    help     history", "second Tab lists, sorted, once each");
    assert_eq!(r.ed.line(), "h");
    r.type_str("e");
    r.edit_with(Edit::Complete, &mut c);
    assert_eq!(r.ed.line(), "hel");
    r.edit_with(Edit::Complete, &mut c);
    assert_eq!(r.out, "", "a Tab after a completion does not list yet");
    r.type_str("p");
    r.edit_with(Edit::Complete, &mut c);
    assert_eq!(r.ed.line(), "help ");
}

#[test]
fn completion_lists_in_columns_down_first() {
    let mut r = Rig::new();
    r.ed.set_width(19);
    let mut c = completer(&["a1", "a2", "a3", "a4", "a5"]);
    r.type_str("a");
    let rows = r.term.rows();
    r.edit_with(Edit::Complete, &mut c);
    r.edit_with(Edit::Complete, &mut c);
    // Four 4-column entries fit in 19 columns: 5 entries, 2 rows.
    assert_eq!(r.term.row_text(rows), "a1  a3  a5");
    assert_eq!(r.term.row_text(rows + 1), "a2  a4");
}

#[test]
fn completion_in_the_middle_of_a_line_and_of_later_words() {
    let mut r = Rig::new();
    let mut c = completer(&["filter", "fil€"]);
    r.type_str("test f x");
    r.edits(&[Edit::Left, Edit::Left]);
    r.edit_with(Edit::Complete, &mut c);
    assert_eq!((r.ed.line().as_str(), r.ed.cursor()), ("test fil x", 8));
    r.type_str("€");
    r.edit_with(Edit::Complete, &mut c);
    assert_eq!(r.ed.line(), "test fil€  x");
}

#[test]
fn completion_ignores_bad_answers() {
    let mut r = Rig::new();
    r.type_str("ä");
    // A start inside the ä, past the cursor, and candidates that do not fit.
    for start in [1, 3] {
        let mut c = |_: &str| Completion { start, candidates: std::vec!["äx".into()] };
        r.edit_with(Edit::Complete, &mut c);
        assert_eq!(r.ed.line(), "ä");
    }
    let mut c = completer(&["b", "c"]);
    r.edit_with(Edit::Complete, &mut c);
    r.edit_with(Edit::Complete, &mut c);
    assert_eq!((r.ed.line().as_str(), r.out.as_str()), ("ä", ""));
}

#[test]
fn clear_screen_and_redraw() {
    let mut r = Rig::new();
    r.type_str("abc");
    r.edit(Edit::Left);
    r.edit(Edit::ClearScreen);
    assert_eq!((r.term.row, r.term.row_text(0)), (0, std::format!("{SHOWN}abc")));
    // Something else wrote over the row and left the cursor elsewhere.
    r.term.feed("\rlog message\n");
    r.edit(Edit::Redraw);
    assert_eq!(r.term.row_text(0), "log message");
    r.type_str("d");
    assert_eq!(r.ed.line(), "abdc");
}

#[test]
fn lines_stop_growing_at_the_limit() {
    let mut r = Rig::new();
    for _ in 0..MAX_LINE + 5 {
        r.ed.handle(Edit::Insert('x'), &mut String::new(), &mut NoCompletion);
    }
    assert_eq!(r.ed.line().len(), MAX_LINE);
    r.ed.handle(Edit::Home, &mut String::new(), &mut NoCompletion);
    let mut c = completer(&["xxxxxxxxyz"]);
    r.ed.handle(Edit::Complete, &mut String::new(), &mut c);
    assert_eq!(r.ed.line().len(), MAX_LINE);
}

#[test]
fn start_resets_a_line_left_unfinished() {
    let mut r = Rig::new();
    r.type_str("half");
    r.term.feed("\n");
    r.start();
    assert_eq!(r.ed.line(), "");
    assert_eq!(r.ed.prompt(), PROMPT);
}
