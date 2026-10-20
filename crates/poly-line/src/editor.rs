//! The editor: a line of characters with a cursor, redrawn on a terminal
//! with relative cursor movement.

use crate::{Edit, History, words};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

/// Longest line in characters; further insertions are ignored.
pub const MAX_LINE: usize = 1024;

/// Terminal width assumed for listing completion candidates in columns.
pub const DEFAULT_WIDTH: usize = 80;

/// What an edit did to the line as a whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Still editing.
    Pending,
    /// Enter: the submitted line (added to the history). The cursor is on a
    /// new row; call [`Editor::start`] for the next line.
    Done(String),
    /// Ctrl-C: the line was discarded. The cursor is on a new row.
    Cancelled,
}

/// Candidates for completing the word that ends at the cursor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completion {
    /// Byte offset in the text before the cursor where the word to complete
    /// starts. The word is everything from there to the cursor.
    pub start: usize,
    /// Every word that could go there. Candidates that do not start with
    /// the word are ignored; order and duplicates do not matter.
    pub candidates: Vec<String>,
}

/// A source of completions, asked on Tab with the text before the cursor.
pub trait Complete {
    fn complete(&mut self, before_cursor: &str) -> Completion;
}

impl<F: FnMut(&str) -> Completion> Complete for F {
    fn complete(&mut self, before_cursor: &str) -> Completion {
        self(before_cursor)
    }
}

/// Completes nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoCompletion;

impl Complete for NoCompletion {
    fn complete(&mut self, before_cursor: &str) -> Completion {
        Completion { start: before_cursor.len(), candidates: Vec::new() }
    }
}

/// A line editor with history and completion. See the crate documentation
/// for the output it produces.
///
/// The screen and the editor agree on one invariant: after every call, the
/// prompt and the line are on the terminal's current row and the cursor
/// sits on the line's character at [`cursor`](Self::cursor). Every redraw
/// moves relative to that position, so the editor never needs to know the
/// prompt's width or the cursor's column.
#[derive(Clone, Debug)]
pub struct Editor {
    prompt: String,
    line: Vec<char>,
    /// Characters before the cursor.
    cursor: usize,
    history: History,
    /// How far back in the history the line was recalled from (0: the
    /// newest entry); `None` while editing a new line.
    browsing: Option<usize>,
    /// The new line, kept while browsing the history.
    draft: Vec<char>,
    /// The last edit was a Tab that could not complete any further, so the
    /// next one lists the candidates.
    tab_pending: bool,
    width: usize,
}

impl Editor {
    /// An editor whose history keeps the last `history` lines.
    pub fn new(history: usize) -> Editor {
        Editor {
            prompt: String::new(),
            line: Vec::new(),
            cursor: 0,
            history: History::new(history),
            browsing: None,
            draft: Vec::new(),
            tab_pending: false,
            width: DEFAULT_WIDTH,
        }
    }

    /// Start a new, empty line: print `prompt` at the cursor, which should
    /// be at the start of a row.
    pub fn start(&mut self, prompt: &str, out: &mut dyn Write) {
        self.prompt.clear();
        self.prompt.push_str(prompt);
        self.reset();
        let _ = out.write_str(prompt);
    }

    /// The line as it stands.
    pub fn line(&self) -> String {
        self.line.iter().collect()
    }

    /// Characters before the cursor.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn history_mut(&mut self) -> &mut History {
        &mut self.history
    }

    /// The terminal width in columns, for listing completions.
    pub fn set_width(&mut self, columns: usize) {
        self.width = columns.max(1);
    }

    /// Apply one edit, writing what changes on the screen to `out`.
    pub fn handle(&mut self, edit: Edit, out: &mut dyn Write, complete: &mut dyn Complete) -> Action {
        let tab_again = core::mem::replace(&mut self.tab_pending, false);
        let (old_cursor, old_len) = (self.cursor, self.line.len());
        match edit {
            Edit::Insert(c) => {
                if !c.is_control() && self.line.len() < MAX_LINE {
                    self.line.insert(self.cursor, c);
                    self.cursor += 1;
                    self.redraw(out, old_cursor, old_cursor, old_len);
                }
            }
            Edit::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.line.remove(self.cursor);
                self.redraw(out, self.cursor, old_cursor, old_len);
            }
            Edit::Delete if self.cursor < self.line.len() => {
                self.line.remove(self.cursor);
                self.redraw(out, self.cursor, old_cursor, old_len);
            }
            Edit::Left if self.cursor > 0 => self.move_to(out, self.cursor - 1),
            Edit::Right if self.cursor < self.line.len() => self.move_to(out, self.cursor + 1),
            Edit::Home => self.move_to(out, 0),
            Edit::End => self.move_to(out, self.line.len()),
            Edit::WordLeft => self.move_to(out, self.word_left()),
            Edit::WordRight => self.move_to(out, self.word_right()),
            Edit::KillToEnd => {
                self.line.truncate(self.cursor);
                self.redraw(out, self.cursor, old_cursor, old_len);
            }
            Edit::KillToStart => {
                self.line.drain(..self.cursor);
                self.cursor = 0;
                self.redraw(out, 0, old_cursor, old_len);
            }
            Edit::KillWordBefore => {
                let start = self.word_start_by_whitespace();
                self.line.drain(start..self.cursor);
                self.cursor = start;
                self.redraw(out, start, old_cursor, old_len);
            }
            Edit::HistoryPrev => {
                let back = self.browsing.map_or(0, |b| b + 1);
                if let Some(entry) = self.history.get(back) {
                    let entry = entry.chars().collect();
                    if self.browsing.is_none() {
                        self.draft = self.line.clone();
                    }
                    self.browsing = Some(back);
                    self.replace(out, entry);
                }
            }
            Edit::HistoryNext => match self.browsing {
                None => {}
                Some(0) => {
                    self.browsing = None;
                    let draft = core::mem::take(&mut self.draft);
                    self.replace(out, draft);
                }
                Some(back) => {
                    self.browsing = Some(back - 1);
                    let entry = self.history.get(back - 1).unwrap_or_default().chars().collect();
                    self.replace(out, entry);
                }
            },
            Edit::Complete => self.complete(out, complete, tab_again),
            Edit::Enter => {
                self.move_to(out, self.line.len());
                let _ = out.write_str("\n");
                let line = self.line();
                self.history.push(&line);
                self.reset();
                return Action::Done(line);
            }
            Edit::Cancel => {
                self.move_to(out, self.line.len());
                let _ = out.write_str("^C\n");
                self.reset();
                return Action::Cancelled;
            }
            Edit::ClearScreen => {
                let _ = out.write_str("\x1b[2J\x1b[H");
                self.draw(out);
            }
            Edit::Redraw => {
                let _ = out.write_str("\r\x1b[K");
                self.draw(out);
            }
            // Backspace, Delete, Left and Right at the end of the line.
            _ => {}
        }
        Action::Pending
    }

    fn reset(&mut self) {
        self.line.clear();
        self.cursor = 0;
        self.browsing = None;
        self.draft.clear();
        self.tab_pending = false;
    }

    /// Bring the screen up to date after the line changed from index `from`
    /// on (everything before it is unchanged), given where the cursor was and
    /// how long the line was: go to `from`, write the rest of the line, erase
    /// what is left of a longer old line, and go back to the cursor.
    fn redraw(&self, out: &mut dyn Write, from: usize, old_cursor: usize, old_len: usize) {
        shift(out, old_cursor, from);
        put(out, &self.line[from..]);
        if self.line.len() < old_len {
            let _ = out.write_str("\x1b[K");
        }
        shift(out, self.line.len(), self.cursor);
    }

    /// The prompt and the whole line, from the start of the row.
    fn draw(&self, out: &mut dyn Write) {
        let _ = out.write_str(&self.prompt);
        put(out, &self.line);
        shift(out, self.line.len(), self.cursor);
    }

    fn move_to(&mut self, out: &mut dyn Write, to: usize) {
        shift(out, self.cursor, to);
        self.cursor = to;
    }

    /// Replace the whole line, with the cursor at its end.
    fn replace(&mut self, out: &mut dyn Write, line: Vec<char>) {
        let (old_cursor, old_len) = (self.cursor, self.line.len());
        self.line = line;
        self.cursor = self.line.len();
        self.redraw(out, 0, old_cursor, old_len);
    }

    /// Insert `text` at the cursor, as far as the line has room.
    fn insert(&mut self, out: &mut dyn Write, text: &str) {
        let (old_cursor, old_len) = (self.cursor, self.line.len());
        for c in text.chars().take(MAX_LINE - self.line.len()) {
            self.line.insert(self.cursor, c);
            self.cursor += 1;
        }
        self.redraw(out, old_cursor, old_cursor, old_len);
    }

    fn complete(&mut self, out: &mut dyn Write, complete: &mut dyn Complete, tab_again: bool) {
        let before: String = self.line[..self.cursor].iter().collect();
        let completion = complete.complete(&before);
        // A start past the cursor or inside a character completes nothing.
        let Some(word) = before.get(completion.start..) else { return };
        let mut candidates: Vec<String> = completion.candidates.into_iter().filter(|c| c.starts_with(word)).collect();
        candidates.sort_unstable();
        candidates.dedup();
        let insert = match candidates.as_slice() {
            [] => return,
            [only] => format!("{} ", &only[word.len()..]),
            many => String::from(&words::common_prefix(many)[word.len()..]),
        };
        if !insert.is_empty() {
            self.insert(out, &insert);
            return;
        }
        // Ambiguous, and the common prefix is typed already.
        if tab_again {
            self.list(out, &candidates);
        }
        self.tab_pending = true;
    }

    /// Print the candidates below the line in columns (sorted down each
    /// column), then the prompt and line again.
    fn list(&self, out: &mut dyn Write, candidates: &[String]) {
        let column = candidates.iter().map(|c| c.chars().count()).max().unwrap_or(0) + 2;
        let columns = (self.width / column).max(1);
        let rows = candidates.len().div_ceil(columns);
        shift(out, self.cursor, self.line.len());
        let _ = out.write_str("\n");
        for row in 0..rows {
            let in_row: Vec<&String> = candidates.iter().skip(row).step_by(rows).collect();
            for (i, c) in in_row.iter().enumerate() {
                let pad = if i + 1 < in_row.len() { column - c.chars().count() } else { 0 };
                let _ = write!(out, "{c}{:pad$}", "");
            }
            let _ = out.write_str("\n");
        }
        self.draw(out);
    }

    /// Start of the word before the cursor (letters, digits and `_`).
    fn word_left(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && !is_word(self.line[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(self.line[i - 1]) {
            i -= 1;
        }
        i
    }

    /// End of the word after the cursor.
    fn word_right(&self) -> usize {
        let mut i = self.cursor;
        while i < self.line.len() && !is_word(self.line[i]) {
            i += 1;
        }
        while i < self.line.len() && is_word(self.line[i]) {
            i += 1;
        }
        i
    }

    /// Start of the whitespace-separated word before the cursor, for Ctrl-W.
    fn word_start_by_whitespace(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && self.line[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !self.line[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Move the cursor from column `from` to column `to` of the line.
fn shift(out: &mut dyn Write, from: usize, to: usize) {
    let _ = match (from.abs_diff(to), to < from) {
        (0, _) => Ok(()),
        (1, true) => out.write_str("\x1b[D"),
        (1, false) => out.write_str("\x1b[C"),
        (n, true) => write!(out, "\x1b[{n}D"),
        (n, false) => write!(out, "\x1b[{n}C"),
    };
}

fn put(out: &mut dyn Write, chars: &[char]) {
    for &c in chars {
        let _ = out.write_char(c);
    }
}
