//! A console screen buffer (Learn: "Console Screen Buffers", "High-Level
//! Console Modes", "Console Virtual Terminal Sequences") and the bytes that
//! make a VT terminal show it.
//!
//! The buffer is a grid of cells (a character and an attribute), a cursor
//! and the attribute new text gets. Poly's buffer is exactly the terminal's
//! size, and its window is the whole buffer (no scroll-back: the terminal
//! keeps its own). A terminal of unknown size (0×0) counts as 80×25.
//!
//! # Output
//!
//! [`Screen::write`] follows the output mode:
//!
//! - `ENABLE_PROCESSED_OUTPUT`: backspace moves the cursor back (not past
//!   the start of the line), tab moves to the next multiple of 8 writing
//!   spaces, line feed moves down (and to the start of the line unless
//!   `DISABLE_NEWLINE_AUTO_RETURN`), carriage return to the start of the
//!   line, and bell rings. Without it, and for the other control
//!   characters, the IBM PC glyph of the character is shown
//!   ([`crate::codepage::CONTROL_GLYPHS`]).
//! - `ENABLE_WRAP_AT_EOL_OUTPUT`: writing in the last column moves the
//!   cursor to the next line (scrolling at the bottom); without it the
//!   cursor stays in the last column and later characters overwrite it.
//! - `ENABLE_VIRTUAL_TERMINAL_PROCESSING`: the sequences of
//!   [`crate::vt`] act on the buffer: SGR (`m`), the cursor moves (`A` `B`
//!   `C` `D` `E` `F` `G` `d` `H` `f`), erase in display and line (`J`
//!   `K`), erase characters (`X`), save and restore (`s` `u`, `ESC 7`
//!   `ESC 8`), cursor visibility (`?25h` / `?25l`) and the title (`OSC 0`
//!   / `OSC 2`). Other sequences are ignored. Without the mode, ESC is a
//!   character like any other control.
//!
//! # The terminal
//!
//! Every change is mirrored to the terminal as VT bytes, collected until
//! [`Screen::take_output`]. The screen tracks where the terminal's cursor
//! is and which rendition it draws with, and sends a cursor position
//! (`CUP`) or rendition (`SGR`) only when they differ. A console takes the
//! terminal over on its first output: it resets the rendition, limits
//! scrolling to its own rows (`DECSTBM`) and clears the screen, so the
//! buffer and the terminal start the same (Poly's choice: Windows makes a
//! new window for a new console). [`Screen::release`] gives the terminal
//! back below the console's text.

use crate::codepage::CONTROL_GLYPHS;
use crate::consts::*;
use crate::mode::ERROR_INVALID_PARAMETER;
use crate::vt::{self, Action, Parser};
use alloc::vec::Vec;

/// One character cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub attr: u16,
}

/// The size a terminal of unknown size counts as.
pub const DEFAULT_SIZE: (u16, u16) = (80, 25);

/// A screen buffer and its terminal mirror.
#[derive(Clone, Debug)]
pub struct Screen {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
    cursor: (u16, u16),
    attr: u16,
    cursor_visible: bool,
    cursor_size: u32,
    saved: (u16, u16),
    parser: Parser,
    high_surrogate: Option<u16>,
    out: Vec<u8>,
    term_cursor: Option<(u16, u16)>,
    term_attr: Option<u16>,
    taken_over: bool,
    /// A title a VT `OSC 0` / `OSC 2` set (the console picks it up).
    pub title: Option<Vec<u16>>,
}

impl Screen {
    /// A blank buffer of `width` × `height` (0 counts as unknown).
    pub fn new(width: u16, height: u16) -> Screen {
        let (width, height) =
            if width == 0 || height == 0 { DEFAULT_SIZE } else { (width.min(1000), height.min(1000)) };
        Screen {
            width,
            height,
            cells: alloc::vec![Cell { ch: ' ', attr: DEFAULT_ATTRIBUTE }; usize::from(width) * usize::from(height)],
            cursor: (0, 0),
            attr: DEFAULT_ATTRIBUTE,
            cursor_visible: true,
            cursor_size: 25,
            saved: (0, 0),
            parser: Parser::default(),
            high_surrogate: None,
            out: Vec::new(),
            term_cursor: None,
            term_attr: None,
            taken_over: false,
            title: None,
        }
    }

    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    pub fn cursor(&self) -> (u16, u16) {
        self.cursor
    }

    pub fn attribute(&self) -> u16 {
        self.attr
    }

    /// `SetConsoleTextAttribute`.
    pub fn set_attribute(&mut self, attr: u16) {
        self.attr = attr;
    }

    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    pub fn cursor_size(&self) -> u32 {
        self.cursor_size
    }

    /// The cell at (`x`, `y`).
    pub fn cell(&self, x: u16, y: u16) -> Option<Cell> {
        (x < self.width && y < self.height).then(|| self.cells[self.index(x, y)])
    }

    /// The characters of row `y`, trailing spaces removed (tests).
    pub fn row_text(&self, y: u16) -> alloc::string::String {
        let start = usize::from(y) * usize::from(self.width);
        let s: alloc::string::String =
            self.cells[start..start + usize::from(self.width)].iter().map(|c| c.ch).collect();
        s.trim_end().into()
    }

    fn index(&self, x: u16, y: u16) -> usize {
        usize::from(y) * usize::from(self.width) + usize::from(x)
    }

    /// The VT bytes for the terminal since the last call.
    pub fn take_output(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.out)
    }

    /// Whether the console has drawn on its terminal.
    pub fn taken_over(&self) -> bool {
        self.taken_over
    }

    // ----- the terminal mirror

    fn begin(&mut self) {
        if self.taken_over {
            return;
        }
        self.taken_over = true;
        self.out.extend_from_slice(b"\x1b[0m\x1b[1;");
        vt::push_num(&mut self.out, 0, self.height);
        self.out.extend_from_slice(b"r\x1b[H\x1b[2J");
        self.term_cursor = Some((0, 0));
        self.term_attr = Some(DEFAULT_ATTRIBUTE);
    }

    fn term_goto(&mut self, x: u16, y: u16) {
        if self.term_cursor != Some((x, y)) {
            self.out.extend_from_slice(b"\x1b[");
            vt::push_num(&mut self.out, 0, y + 1);
            vt::push_num(&mut self.out, b';', x + 1);
            self.out.push(b'H');
            self.term_cursor = Some((x, y));
        }
    }

    fn term_attr(&mut self, attr: u16) {
        if self.term_attr != Some(attr) {
            self.out.extend_from_slice(&vt::sgr_for(attr));
            self.term_attr = Some(attr);
        }
    }

    /// Draw cell (`x`, `y`) on the terminal.
    fn draw(&mut self, x: u16, y: u16) {
        self.begin();
        let cell = self.cells[self.index(x, y)];
        self.term_goto(x, y);
        self.term_attr(cell.attr);
        let mut buf = [0u8; 4];
        self.out.extend_from_slice(cell.ch.encode_utf8(&mut buf).as_bytes());
        // After the last column a VT terminal's cursor is in its
        // "pending wrap" state: position it explicitly next time.
        self.term_cursor = if x + 1 < self.width { Some((x + 1, y)) } else { None };
    }

    /// Put the terminal's cursor where the buffer's is.
    pub fn sync_cursor(&mut self) {
        if self.taken_over {
            let (x, y) = self.cursor;
            self.term_goto(x, y);
        }
    }

    /// Give the terminal back: default rendition, full scrolling region,
    /// the cursor at the start of the line after the console's last text.
    pub fn release(&mut self) {
        if !self.taken_over {
            return;
        }
        self.out.extend_from_slice(b"\x1b[0m\x1b[r");
        self.term_cursor = Some((0, 0));
        // On the line after the console's cursor, so whatever the terminal
        // shows next starts a line of its own.
        let (x, y) = self.cursor;
        self.term_goto(x, y);
        self.out.extend_from_slice(b"\r\n");
        if !self.cursor_visible {
            self.out.extend_from_slice(b"\x1b[?25h");
        }
        self.taken_over = false;
        self.term_cursor = None;
        self.term_attr = None;
    }

    // ----- the buffer

    /// Scroll the buffer up one line (new line in the current attribute).
    fn scroll_up(&mut self) {
        let w = usize::from(self.width);
        self.cells.drain(..w);
        let blank = Cell { ch: ' ', attr: self.attr };
        self.cells.extend(core::iter::repeat_n(blank, w));
        self.begin();
        // From the first column, so the line feed lands there whether or
        // not the terminal adds a carriage return (`ONLCR`).
        let bottom = self.height - 1;
        self.term_goto(0, bottom);
        self.term_attr(self.attr);
        self.out.push(b'\n');
        self.term_cursor = Some((0, bottom));
    }

    fn line_feed(&mut self) {
        if self.cursor.1 + 1 < self.height {
            self.cursor.1 += 1;
        } else {
            self.scroll_up();
        }
    }

    /// Put one character at the cursor and advance it.
    fn put(&mut self, ch: char, wrap: bool) {
        let (x, y) = self.cursor;
        let i = self.index(x, y);
        self.cells[i] = Cell { ch, attr: self.attr };
        self.draw(x, y);
        if x + 1 < self.width {
            self.cursor.0 += 1;
        } else if wrap {
            self.cursor.0 = 0;
            self.line_feed();
        }
    }

    /// One UTF-16 unit of text to show (surrogate pairs joined).
    fn put_unit(&mut self, u: u16, wrap: bool) {
        if (0xD800..0xDC00).contains(&u) {
            if self.high_surrogate.replace(u).is_some() {
                self.put('\u{FFFD}', wrap);
            }
            return;
        }
        let ch = match self.high_surrogate.take() {
            Some(high) if (0xDC00..0xE000).contains(&u) => {
                char::decode_utf16([high, u]).next().and_then(Result::ok).unwrap_or('\u{FFFD}')
            }
            Some(_) => {
                self.put('\u{FFFD}', wrap);
                char::from_u32(u32::from(u)).unwrap_or('\u{FFFD}')
            }
            None => char::from_u32(u32::from(u)).unwrap_or('\u{FFFD}'),
        };
        self.put(ch, wrap);
    }

    /// A control character, processed or shown.
    fn control(&mut self, c: u16, mode: u32) {
        let wrap = mode & ENABLE_WRAP_AT_EOL_OUTPUT != 0;
        if mode & ENABLE_PROCESSED_OUTPUT != 0 {
            match c {
                0x08 => {
                    self.cursor.0 = self.cursor.0.saturating_sub(1);
                    return;
                }
                0x09 => {
                    let n = 8 - self.cursor.0 % 8;
                    let room = self.width - self.cursor.0;
                    for _ in 0..n.min(room) {
                        self.put(' ', wrap);
                    }
                    return;
                }
                0x0A => {
                    self.line_feed();
                    if mode & DISABLE_NEWLINE_AUTO_RETURN == 0 {
                        self.cursor.0 = 0;
                    }
                    return;
                }
                0x0D => {
                    self.cursor.0 = 0;
                    return;
                }
                0x07 => {
                    self.begin();
                    self.out.push(0x07);
                    return;
                }
                _ => {}
            }
        }
        let glyph = if c == 0x7F { 0x2302 } else { CONTROL_GLYPHS[usize::from(c & 0x1F)] };
        self.put_unit(glyph, wrap);
    }

    /// `WriteConsoleW` / `WriteFile` text in output mode `mode`.
    pub fn write(&mut self, text: &[u16], mode: u32) {
        let wrap = mode & ENABLE_WRAP_AT_EOL_OUTPUT != 0;
        let vt_on = mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0;
        for &u in text {
            if !vt_on {
                if u < 0x20 || u == 0x7F {
                    self.control(u, mode);
                } else {
                    self.put_unit(u, wrap);
                }
                continue;
            }
            match self.parser.feed(u) {
                None => {}
                Some(Action::Print(p)) => self.put_unit(p, wrap),
                Some(Action::Control(c)) => self.control(c, mode),
                Some(Action::Esc(b'7')) => self.saved = self.cursor,
                Some(Action::Esc(b'8')) => self.cursor = self.saved,
                Some(Action::Esc(_)) => {}
                Some(Action::Osc { number: 0 | 2, text }) => self.title = Some(text),
                Some(Action::Osc { .. }) => {}
                Some(Action::Csi { private, params, final_byte }) => self.csi(private, &params, final_byte),
            }
        }
        self.sync_cursor();
    }

    /// Cooked-mode echo: always processed and wrapping, never VT.
    pub fn echo(&mut self, text: &[u16]) {
        self.write(text, ENABLE_PROCESSED_OUTPUT | ENABLE_WRAP_AT_EOL_OUTPUT);
    }

    fn csi(&mut self, private: Option<u8>, params: &[u16], final_byte: u8) {
        let p = |i: usize| params.get(i).copied().unwrap_or(0);
        let n = |i: usize| p(i).max(1);
        let (w, h) = (self.width, self.height);
        let (x, y) = self.cursor;
        if private == Some(b'?') {
            if params == [25] && matches!(final_byte, b'h' | b'l') {
                self.cursor_visible = final_byte == b'h';
                self.begin();
                self.out.extend_from_slice(if self.cursor_visible { b"\x1b[?25h" } else { b"\x1b[?25l" });
            }
            return;
        }
        if private.is_some() {
            return;
        }
        match final_byte {
            b'm' => self.attr = vt::apply_sgr(self.attr, params),
            b'H' | b'f' => self.cursor = ((n(1) - 1).min(w - 1), (n(0) - 1).min(h - 1)),
            b'A' => self.cursor.1 = y.saturating_sub(n(0)),
            b'B' => self.cursor.1 = (y.saturating_add(n(0))).min(h - 1),
            b'C' => self.cursor.0 = (x.saturating_add(n(0))).min(w - 1),
            b'D' => self.cursor.0 = x.saturating_sub(n(0)),
            b'E' => self.cursor = (0, (y.saturating_add(n(0))).min(h - 1)),
            b'F' => self.cursor = (0, y.saturating_sub(n(0))),
            b'G' => self.cursor.0 = (n(0) - 1).min(w - 1),
            b'd' => self.cursor.1 = (n(0) - 1).min(h - 1),
            b's' => self.saved = self.cursor,
            b'u' => self.cursor = self.saved,
            b'J' | b'K' | b'X' => {
                let here = self.index(x, y);
                let row = usize::from(y) * usize::from(w);
                let all = self.cells.len();
                let range = match (final_byte, p(0)) {
                    (b'J', 0) => here..all,
                    (b'J', 1) => 0..here + 1,
                    (b'J', _) => 0..all,
                    (b'K', 0) => here..row + usize::from(w),
                    (b'K', 1) => row..here + 1,
                    (b'K', _) => row..row + usize::from(w),
                    _ => here..(here + usize::from(n(0))).min(row + usize::from(w)),
                };
                let blank = Cell { ch: ' ', attr: self.attr };
                self.cells[range].fill(blank);
                // The terminal erases the same cells itself.
                self.begin();
                self.term_goto(x, y);
                self.term_attr(self.attr);
                self.out.extend_from_slice(b"\x1b[");
                vt::push_num(&mut self.out, 0, if final_byte == b'X' { n(0) } else { p(0) });
                self.out.push(final_byte);
            }
            _ => {}
        }
    }

    /// `SetConsoleCursorPosition`: outside the buffer is
    /// `ERROR_INVALID_PARAMETER` (Learn: "The coordinates must be within
    /// the boundaries of the console screen buffer").
    pub fn set_cursor(&mut self, x: i16, y: i16) -> Result<(), u32> {
        if x < 0 || y < 0 || x as u16 >= self.width || y as u16 >= self.height {
            return Err(ERROR_INVALID_PARAMETER);
        }
        self.cursor = (x as u16, y as u16);
        self.begin();
        self.sync_cursor();
        Ok(())
    }

    /// Move the cursor `delta` cells along the buffer (rows wrap), staying
    /// inside it (cooked-mode editing).
    pub fn move_cursor(&mut self, delta: isize) {
        let w = self.width as isize;
        let max = self.height as isize * w - 1;
        let at = (self.cursor.1 as isize * w + self.cursor.0 as isize + delta).clamp(0, max);
        self.cursor = ((at % w) as u16, (at / w) as u16);
        self.sync_cursor();
    }

    /// The cells `FillConsoleOutput*` covers from (`x`, `y`): `count` of
    /// them, continuing on the next rows, cut at the end of the buffer.
    /// A start outside the buffer is `ERROR_INVALID_PARAMETER` (Poly's
    /// choice, to be confirmed by the spec agent).
    fn fill_range(&self, count: u32, x: i16, y: i16) -> Result<core::ops::Range<usize>, u32> {
        if x < 0 || y < 0 || x as u16 >= self.width || y as u16 >= self.height {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let start = self.index(x as u16, y as u16);
        Ok(start..(start + count as usize).min(self.cells.len()))
    }

    fn redraw(&mut self, range: core::ops::Range<usize>) {
        let w = usize::from(self.width);
        for i in range {
            self.draw((i % w) as u16, (i / w) as u16);
        }
        self.sync_cursor();
    }

    /// `FillConsoleOutputCharacterW`: the cells written (the cursor and
    /// the attributes stay).
    pub fn fill_char(&mut self, ch: u16, count: u32, x: i16, y: i16) -> Result<u32, u32> {
        let range = self.fill_range(count, x, y)?;
        let ch = char::from_u32(u32::from(ch)).unwrap_or('\u{FFFD}');
        for c in &mut self.cells[range.clone()] {
            c.ch = ch;
        }
        let n = range.len() as u32;
        self.redraw(range);
        Ok(n)
    }

    /// `FillConsoleOutputAttribute`.
    pub fn fill_attr(&mut self, attr: u16, count: u32, x: i16, y: i16) -> Result<u32, u32> {
        let range = self.fill_range(count, x, y)?;
        for c in &mut self.cells[range.clone()] {
            c.attr = attr;
        }
        let n = range.len() as u32;
        self.redraw(range);
        Ok(n)
    }

    /// `CONSOLE_SCREEN_BUFFER_INFOEX` (wincon.h, 96 bytes): `cbSize`,
    /// `dwSize`, `dwCursorPosition`, `wAttributes`, `srWindow` (the whole
    /// buffer), `dwMaximumWindowSize` (the buffer's size),
    /// `wPopupAttributes`, `bFullscreenSupported` (FALSE) and the colour
    /// table. The popup attribute and the colour table (the classic
    /// 16-colour palette, as `COLORREF` 0x00BBGGRR) are Poly's choice, to be
    /// confirmed by the spec agent. [`Self::info`] is the first 22 bytes
    /// after `cbSize`: `CONSOLE_SCREEN_BUFFER_INFO`.
    pub fn info_ex(&self) -> [u8; SCREEN_BUFFER_INFOEX_SIZE] {
        let mut b = [0u8; SCREEN_BUFFER_INFOEX_SIZE];
        let mut put16 = |at: usize, v: u16| b[at..at + 2].copy_from_slice(&v.to_le_bytes());
        put16(4, self.width);
        put16(6, self.height);
        put16(8, self.cursor.0);
        put16(10, self.cursor.1);
        put16(12, self.attr);
        put16(14, 0);
        put16(16, 0);
        put16(18, self.width - 1);
        put16(20, self.height - 1);
        put16(22, self.width);
        put16(24, self.height);
        put16(26, POPUP_ATTRIBUTE);
        b[0..4].copy_from_slice(&(SCREEN_BUFFER_INFOEX_SIZE as u32).to_le_bytes());
        for (i, c) in COLOR_TABLE.iter().enumerate() {
            b[32 + 4 * i..36 + 4 * i].copy_from_slice(&c.to_le_bytes());
        }
        b
    }

    /// `CONSOLE_SCREEN_BUFFER_INFO` (22 bytes).
    pub fn info(&self) -> [u8; SCREEN_BUFFER_INFO_SIZE] {
        let ex = self.info_ex();
        let mut b = [0u8; SCREEN_BUFFER_INFO_SIZE];
        b.copy_from_slice(&ex[4..4 + SCREEN_BUFFER_INFO_SIZE]);
        b
    }
}

/// The popup attribute Poly reports: magenta on white.
pub const POPUP_ATTRIBUTE: u16 = 0xF5;

/// The colour table Poly reports.
pub const COLOR_TABLE: [u32; 16] = [
    0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xC0C0C0, 0x808080, 0xFF0000, 0x00FF00,
    0xFFFF00, 0x0000FF, 0xFF00FF, 0x00FFFF, 0xFFFFFF,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::String;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    const DEFAULT: u32 = ENABLE_PROCESSED_OUTPUT | ENABLE_WRAP_AT_EOL_OUTPUT;
    const VT: u32 = DEFAULT | ENABLE_VIRTUAL_TERMINAL_PROCESSING;

    fn out(s: &mut Screen) -> String {
        String::from_utf8(s.take_output()).unwrap()
    }

    #[test]
    fn first_output_takes_the_terminal_over() {
        let mut s = Screen::new(10, 3);
        s.write(&w("hi"), DEFAULT);
        assert_eq!(out(&mut s), "\x1b[0m\x1b[1;3r\x1b[H\x1b[2Jhi");
        assert_eq!(s.cursor(), (2, 0));
        s.write(&w("\r\nx"), DEFAULT);
        assert_eq!(out(&mut s), "\x1b[2;1Hx");
        s.release();
        assert_eq!(out(&mut s), "\x1b[0m\x1b[r\x1b[2;2H\r\n");
    }

    #[test]
    fn processed_controls() {
        let mut s = Screen::new(20, 3);
        s.write(&w("ab\x08c\tX\rY"), DEFAULT);
        assert_eq!(s.row_text(0), "Yc      X");
        assert_eq!(s.cursor(), (1, 0));
        s.write(&w("\n"), DEFAULT | DISABLE_NEWLINE_AUTO_RETURN);
        assert_eq!(s.cursor(), (1, 1));
        s.write(&w("\n"), DEFAULT);
        assert_eq!(s.cursor(), (0, 2));
        // Unprocessed: glyphs.
        let mut s = Screen::new(20, 3);
        s.write(&w("a\r\n\x1b"), ENABLE_WRAP_AT_EOL_OUTPUT);
        assert_eq!(s.row_text(0), "a♪◙←");
    }

    #[test]
    fn wrap_and_scroll() {
        let mut s = Screen::new(4, 2);
        s.write(&w("abcdefghij"), DEFAULT);
        assert_eq!(s.row_text(0), "efgh");
        assert_eq!(s.row_text(1), "ij");
        assert_eq!(s.cursor(), (2, 1));
        let o = out(&mut s);
        // One scroll: one line feed on the terminal.
        assert_eq!(o.matches('\n').count(), 1, "{o:?}");
        // Without wrapping the last column is overwritten.
        let mut s = Screen::new(4, 2);
        s.write(&w("abcdef"), ENABLE_PROCESSED_OUTPUT);
        assert_eq!(s.row_text(0), "abcf");
        assert_eq!(s.cursor(), (3, 0));
    }

    #[test]
    fn vt_sequences_act_on_the_buffer() {
        let mut s = Screen::new(10, 4);
        s.write(&w("\x1b[31mR\x1b[0m\x1b[3;4HZ\x1b[2;1H\x1b[1;44mB"), VT);
        assert_eq!(s.cell(0, 0).unwrap().attr, FOREGROUND_RED);
        assert_eq!(s.cell(3, 2).unwrap().ch, 'Z');
        assert_eq!(s.cell(0, 1).unwrap().attr, DEFAULT_ATTRIBUTE | FOREGROUND_INTENSITY | BACKGROUND_BLUE);
        s.write(&w("\x1b[3;1Habcdef\x1b[3;3H\x1b[K"), VT);
        assert_eq!(s.row_text(2), "ab");
        s.write(&w("\x1b[2J"), VT);
        assert!((0..4).all(|y| s.row_text(y).is_empty()));
        s.write(&w("\x1b]0;Title\x07"), VT);
        assert_eq!(s.title.as_deref(), Some(&w("Title")[..]));
        s.write(&w("\x1b[?25l"), VT);
        assert!(!s.cursor_visible());
        // Without the mode the sequence is text.
        let mut s = Screen::new(10, 2);
        s.write(&w("\x1b[1m"), DEFAULT);
        assert_eq!(s.row_text(0), "←[1m");
    }

    #[test]
    fn attributes_reach_the_terminal() {
        let mut s = Screen::new(10, 2);
        s.set_attribute(FOREGROUND_GREEN | FOREGROUND_INTENSITY);
        s.write(&w("ok"), DEFAULT);
        let o = out(&mut s);
        assert!(o.ends_with("\x1b[0;92;40mok"), "{o:?}");
        s.set_attribute(DEFAULT_ATTRIBUTE);
        s.write(&w("."), DEFAULT);
        assert_eq!(out(&mut s), "\x1b[0m.");
    }

    #[test]
    fn cursor_and_fills() {
        let mut s = Screen::new(5, 3);
        assert_eq!(s.set_cursor(5, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(s.set_cursor(-1, 0), Err(ERROR_INVALID_PARAMETER));
        s.set_cursor(2, 1).unwrap();
        let o = out(&mut s);
        assert!(o.ends_with("\x1b[2;3H"), "{o:?}");
        assert_eq!(s.fill_char(u16::from(b'#'), 4, 3, 0), Ok(4));
        assert_eq!(s.row_text(0), "   ##");
        assert_eq!(s.row_text(1), "##");
        assert_eq!(s.fill_char(u16::from(b'*'), 100, 0, 2), Ok(5));
        assert_eq!(s.fill_attr(0x1E, 2, 0, 0), Ok(2));
        assert_eq!(s.cell(1, 0).unwrap().attr, 0x1E);
        assert_eq!(s.cursor(), (2, 1));
        assert_eq!(s.fill_char(32, 1, 0, 3), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn info_layout() {
        let mut s = Screen::new(80, 25);
        s.write(&w("abc"), DEFAULT);
        let b = s.info();
        let h = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        assert_eq!((h(0), h(2), h(4), h(6), h(8)), (80, 25, 3, 0, 7));
        assert_eq!((h(10), h(12), h(14), h(16)), (0, 0, 79, 24));
        assert_eq!((h(18), h(20)), (80, 25));
        let ex = s.info_ex();
        assert_eq!(u32::from_le_bytes([ex[0], ex[1], ex[2], ex[3]]), 96);
        assert_eq!(Screen::new(0, 0).size(), DEFAULT_SIZE);
    }

    #[test]
    fn surrogate_pairs_are_one_cell() {
        let mut s = Screen::new(10, 2);
        s.write(&w("a😀b"), DEFAULT);
        assert_eq!(s.row_text(0), "a😀b");
        assert_eq!(s.cursor(), (3, 0));
    }
}
