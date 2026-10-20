//! The kernel shell's line editor, independent of any device.
//!
//! Input is a stream of abstract [`Edit`]s (insert a character, move the
//! cursor, recall history, complete, submit). Where they come from is the
//! caller's business: [`TermDecoder`] turns the byte stream of a terminal on
//! a serial port into edits, and a keyboard driver maps its key events to
//! the same edits (most of them through [`Edit::control`]). Output is the
//! text and ANSI escape sequences that redraw the line, written to any
//! [`core::fmt::Write`].
//!
//! ```text
//!  serial bytes ──► TermDecoder ──┐
//!                                 ├──► Edit ──► Editor ──► text + escapes ──► terminal
//!  key events ────► (caller) ─────┘               │  ▲
//!                                                 │  └── Complete (Tab)
//!                                                 └──► Action::Done(line) / Cancelled
//! ```
//!
//! # Output
//!
//! The editor uses only what a small text grid supports (ECMA-48):
//!
//! | Bytes | Meaning |
//! |---|---|
//! | printable characters | written at the cursor, which moves right |
//! | `\n` | new line: column 0 of the next row (the console adds a CR if it needs one) |
//! | `\r` | column 0 of the same row |
//! | `ESC [ n D`, `ESC [ n C` | cursor left / right by `n` (CUB / CUF; `n` omitted means 1) |
//! | `ESC [ K` | erase from the cursor to the end of the row (EL) |
//! | `ESC [ 2 J`, `ESC [ H` | erase the screen, cursor to the top left (ED, CUP); only for [`Edit::ClearScreen`] |
//!
//! The prompt is written as given (it may contain SGR color sequences); the
//! editor never measures it, because every movement is relative to the
//! cursor. Every character is taken to be one column wide, which holds for
//! Latin, Greek and Cyrillic letters and symbols such as `€`, but not for
//! East Asian wide characters or combining marks. Lines are not wrapped:
//! a line longer than the terminal is wide is drawn wrong (the line is still
//! edited correctly).
//!
//! # History and completion
//!
//! [`History`] keeps the last submitted lines (fixed capacity, no blank
//! lines, no repeat of the newest entry). Browsing it with
//! [`Edit::HistoryPrev`] / [`Edit::HistoryNext`] keeps the line being typed
//! and brings it back past the newest entry. [`Edit::Complete`] (Tab) asks a
//! [`Complete`] callback for candidates for the word before the cursor,
//! completes their common prefix, and lists them on a second Tab when that
//! does not get any further.
//!
//! # Words
//!
//! [`words`] splits a submitted line into arguments (whitespace-separated,
//! optional quotes) and finds the closest known word for a typo.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod decode;
mod editor;
mod history;
pub mod words;

pub use decode::TermDecoder;
pub use editor::{Action, Complete, Completion, DEFAULT_WIDTH, Editor, MAX_LINE, NoCompletion};
pub use history::History;

/// One editing operation. Keys map to these; the editor never sees a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edit {
    /// Insert a character at the cursor. Control characters are ignored.
    Insert(char),
    /// Delete the character before the cursor.
    Backspace,
    /// Delete the character under the cursor.
    Delete,
    Left,
    Right,
    /// To the start of the word before the cursor (Ctrl-Left).
    WordLeft,
    /// To the end of the word after the cursor (Ctrl-Right).
    WordRight,
    /// To the start of the line (Home, Ctrl-A).
    Home,
    /// To the end of the line (End, Ctrl-E).
    End,
    /// Delete from the cursor to the end of the line (Ctrl-K).
    KillToEnd,
    /// Delete from the start of the line to the cursor (Ctrl-U).
    KillToStart,
    /// Delete the whitespace-separated word before the cursor (Ctrl-W).
    KillWordBefore,
    /// The previous (older) history entry (Up, Ctrl-P).
    HistoryPrev,
    /// The next (newer) history entry, or the line being typed (Down, Ctrl-N).
    HistoryNext,
    /// Complete the word before the cursor (Tab).
    Complete,
    /// Submit the line (Enter).
    Enter,
    /// Discard the line (Ctrl-C).
    Cancel,
    /// Clear the screen and draw the prompt and line at the top (Ctrl-L).
    ClearScreen,
    /// Draw the prompt and line again on the current row, after other
    /// output disturbed it. The cursor must be at the start of a row (or on
    /// the disturbed row, which is erased).
    Redraw,
}

impl Edit {
    /// The edit for Ctrl plus `letter` (either case), the usual Emacs-style
    /// bindings of a Unix shell: A Home, B Left, C Cancel, D Delete, E End,
    /// F Right, H Backspace, I Tab, J and M Enter, K KillToEnd, L
    /// ClearScreen, N HistoryNext, P HistoryPrev, U KillToStart, W
    /// KillWordBefore. `None` for any other key.
    pub const fn control(letter: char) -> Option<Edit> {
        Some(match letter.to_ascii_lowercase() {
            'a' => Edit::Home,
            'b' => Edit::Left,
            'c' => Edit::Cancel,
            'd' => Edit::Delete,
            'e' => Edit::End,
            'f' => Edit::Right,
            'h' => Edit::Backspace,
            'i' => Edit::Complete,
            'j' | 'm' => Edit::Enter,
            'k' => Edit::KillToEnd,
            'l' => Edit::ClearScreen,
            'n' => Edit::HistoryNext,
            'p' => Edit::HistoryPrev,
            'u' => Edit::KillToStart,
            'w' => Edit::KillWordBefore,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests;
