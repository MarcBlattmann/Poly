//! Cooked-mode reads: `ReadConsole` / `ReadFile` with `ENABLE_LINE_INPUT`
//! (Learn: "High-Level Console Modes": "The ReadFile or ReadConsole
//! function returns only when a carriage return is read"; with
//! `ENABLE_ECHO_INPUT` "characters read ... are written to the active
//! screen buffer as they are typed"; with `ENABLE_PROCESSED_INPUT`
//! "backspace, carriage return, and line feed characters are handled by
//! the system").
//!
//! [`LineEditor`] takes key presses until Enter and gives the line with
//! `\r\n` appended (what a cooked `ReadConsole` returns). Editing, Poly's
//! subset of the classic console's: Backspace and Delete, Left, Right,
//! Home and End, Insert (toggles overwrite), Escape (clears the line). A
//! control character other than tab is kept in the line and echoed as its
//! IBM PC glyph (Poly's choice; to be confirmed by the spec agent, the
//! classic console shows some as `^X`). History (F7, Up, Down) is not kept
//! yet.

use crate::codepage::CONTROL_GLYPHS;
use crate::consts::*;
use crate::input::KeyEvent;
use crate::screen::Screen;
use alloc::vec::Vec;

/// The line being typed.
#[derive(Clone, Debug, Default)]
pub struct LineEditor {
    buf: Vec<u16>,
    cursor: usize,
    overwrite: bool,
}

/// What the editor shows for one character.
fn shown(u: u16) -> u16 {
    if u < 0x20 && u != 0x09 { CONTROL_GLYPHS[usize::from(u)] } else { u }
}

impl LineEditor {
    /// The text typed so far.
    pub fn text(&self) -> &[u16] {
        &self.buf
    }

    /// One key press. `insert`: `ENABLE_INSERT_MODE`. `echo`: the screen
    /// to echo on (`ENABLE_ECHO_INPUT`). Returns the line once Enter is
    /// pressed.
    pub fn key(&mut self, k: &KeyEvent, insert: bool, mut echo: Option<&mut Screen>) -> Option<Vec<u16>> {
        if !k.down {
            return None;
        }
        let insert = insert != self.overwrite;
        let mut done = None;
        for _ in 0..k.repeat.max(1) {
            if let Some(line) = self.one(k, insert, echo.as_deref_mut()) {
                done = Some(line);
                break;
            }
        }
        done
    }

    fn one(&mut self, k: &KeyEvent, insert: bool, mut echo: Option<&mut Screen>) -> Option<Vec<u16>> {
        let is_enter = k.vk == VK_RETURN || k.ch == 0x0D || k.ch == 0x0A;
        if is_enter {
            if let Some(s) = echo.as_deref_mut() {
                s.move_cursor((self.buf.len() - self.cursor) as isize);
                s.echo(&[0x0D, 0x0A]);
            }
            let mut line = core::mem::take(&mut self.buf);
            line.extend_from_slice(&[0x0D, 0x0A]);
            self.cursor = 0;
            return Some(line);
        }
        match (k.vk, k.ch) {
            (VK_BACK, _) | (_, 0x08) => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.buf.remove(self.cursor);
                    if let Some(s) = echo {
                        s.move_cursor(-1);
                        self.redraw_tail(s, 1);
                    }
                }
            }
            (VK_DELETE, 0) => {
                if self.cursor < self.buf.len() {
                    self.buf.remove(self.cursor);
                    if let Some(s) = echo {
                        self.redraw_tail(s, 1);
                    }
                }
            }
            (VK_LEFT, 0) => self.move_to(self.cursor.saturating_sub(1), echo),
            (VK_RIGHT, 0) => self.move_to((self.cursor + 1).min(self.buf.len()), echo),
            (VK_HOME, 0) => self.move_to(0, echo),
            (VK_END, 0) => self.move_to(self.buf.len(), echo),
            (VK_INSERT, 0) => self.overwrite = !self.overwrite,
            (VK_ESCAPE, _) => {
                let len = self.buf.len();
                if let Some(s) = echo {
                    s.move_cursor(-(self.cursor as isize));
                    let blanks = alloc::vec![0x20u16; len];
                    s.echo(&blanks);
                    s.move_cursor(-(len as isize));
                }
                self.buf.clear();
                self.cursor = 0;
            }
            (_, 0) => {}
            (_, ch) => {
                if insert || self.cursor == self.buf.len() {
                    self.buf.insert(self.cursor, ch);
                } else {
                    self.buf[self.cursor] = ch;
                }
                self.cursor += 1;
                if let Some(s) = echo {
                    s.echo(&[shown(ch)]);
                    if insert {
                        self.redraw_tail(s, 0);
                    }
                }
            }
        }
        None
    }

    /// Write the text after the cursor (and `blanks` spaces where removed
    /// characters were), then put the screen's cursor back.
    fn redraw_tail(&self, s: &mut Screen, blanks: usize) {
        let tail: Vec<u16> =
            self.buf[self.cursor..].iter().map(|&u| shown(u)).chain(core::iter::repeat_n(0x20, blanks)).collect();
        if tail.is_empty() {
            return;
        }
        s.echo(&tail);
        s.move_cursor(-(tail.len() as isize));
    }

    fn move_to(&mut self, to: usize, echo: Option<&mut Screen>) {
        if let Some(s) = echo {
            s.move_cursor(to as isize - self.cursor as isize);
        }
        self.cursor = to;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::TermDecoder;
    use std::string::String;

    fn type_in(e: &mut LineEditor, s: &mut Screen, bytes: &[u8]) -> Option<String> {
        let mut d = TermDecoder::default();
        for k in d.decode(bytes) {
            if let Some(line) = e.key(&k, true, Some(s)) {
                return Some(String::from_utf16(&line).unwrap());
            }
        }
        None
    }

    #[test]
    fn a_line_with_edits() {
        let mut s = Screen::new(40, 5);
        let mut e = LineEditor::default();
        assert_eq!(type_in(&mut e, &mut s, b"helo"), None);
        assert_eq!(s.row_text(0), "helo");
        // Left, insert an l, End, backspace twice, retype.
        assert_eq!(type_in(&mut e, &mut s, b"\x1b[Dl\x1b[F\x7f\x7flo!\r").as_deref(), Some("hello!\r\n"));
        assert_eq!(s.row_text(0), "hello!");
        assert_eq!(s.cursor(), (0, 1));
    }

    #[test]
    fn delete_home_escape_overwrite() {
        let mut s = Screen::new(40, 5);
        let mut e = LineEditor::default();
        type_in(&mut e, &mut s, b"abcd\x1b[H\x1b[3~");
        assert_eq!(String::from_utf16(e.text()).unwrap(), "bcd");
        assert_eq!(s.row_text(0), "bcd");
        type_in(&mut e, &mut s, b"\x1b");
        assert!(e.text().is_empty());
        assert_eq!(s.row_text(0), "");
        type_in(&mut e, &mut s, b"xyz\x1b[H\x1b[2~Q");
        assert_eq!(String::from_utf16(e.text()).unwrap(), "Qyz");
    }

    #[test]
    fn without_echo_nothing_is_shown() {
        let mut e = LineEditor::default();
        let mut d = TermDecoder::default();
        let mut line = None;
        for k in d.decode(b"pw\r") {
            line = line.or(e.key(&k, true, None));
        }
        assert_eq!(String::from_utf16(&line.unwrap()).unwrap(), "pw\r\n");
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut e = LineEditor::default();
        let k = KeyEvent::press(0x41, 0x61, 0);
        assert_eq!(e.key(&k.released(), true, None), None);
        assert!(e.text().is_empty());
    }
}
