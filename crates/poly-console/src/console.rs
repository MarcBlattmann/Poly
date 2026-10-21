//! One console: its input buffer and modes, its code pages and title, and
//! its screen buffer (Learn: "Consoles", "Console Input Buffer", "Console
//! Screen Buffers").
//!
//! The console host feeds it the terminal's bytes ([`Console::feed`]) and
//! runs the programs' requests on it. A read that cannot finish yet
//! returns `None`; the host keeps the request waiting and retries it after
//! the next input.
//!
//! # Control events
//!
//! With `ENABLE_PROCESSED_INPUT`, Ctrl+C is "processed by the system and is
//! not placed in the input buffer" (Learn: "SetConsoleMode"): it becomes a
//! `CTRL_C_EVENT` for every process attached ([`Console::take_ctrl`]). A
//! cooked line being typed is thrown away, and a read waiting at that
//! moment ends with nothing read and [`ReadOutcome::Aborted`] (Poly's
//! choice, to be confirmed by the spec agent: `ReadConsoleW` then succeeds
//! with 0 characters and `ERROR_OPERATION_ABORTED` as the last error).
//! Without the mode, Ctrl+C is the character 0x03 like any other key.

use crate::codepage::{self, Utf8Decoder};
use crate::consts::*;
use crate::cooked::LineEditor;
use crate::input::{self, InputRecord, KeyEvent, TermDecoder};
use crate::mode;
use crate::screen::Screen;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

/// What a text read produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    /// UTF-16 units (`ReadConsoleW`).
    Units(Vec<u16>),
    /// Bytes in the input code page (`ReadConsoleA`, `ReadFile`).
    Bytes(Vec<u8>),
    /// Ctrl+C ended the read.
    Aborted,
}

/// The most input records kept; the oldest go first (Poly's bound).
pub const MAX_INPUT_RECORDS: usize = 4096;

/// A console.
#[derive(Clone, Debug)]
pub struct Console {
    pub input_mode: u32,
    pub output_mode: u32,
    pub input_cp: u32,
    pub output_cp: u32,
    pub title: Vec<u16>,
    pub screen: Screen,
    input: VecDeque<InputRecord>,
    decoder: TermDecoder,
    editor: Option<LineEditor>,
    /// A finished cooked line not read yet (the caller's buffer was
    /// smaller).
    pending: VecDeque<u16>,
    /// Bytes of an ANSI read's last character that did not fit.
    pending_bytes: VecDeque<u8>,
    /// A read is waiting for input.
    reading: bool,
    /// Ctrl+C came while a read waited: the next read try ends it.
    abort: bool,
    ctrl: VecDeque<u32>,
    out_decoder: Utf8Decoder,
    /// Terminal bytes that do not change the buffer (the title).
    extra: Vec<u8>,
}

impl Console {
    /// A new console on a terminal of `width` × `height` (0: unknown),
    /// with the default modes and code page 437 (the OEM code page of US
    /// Windows, which a new console uses; Poly's choice of OEM page).
    pub fn new(width: u16, height: u16) -> Console {
        Console {
            input_mode: mode::DEFAULT_INPUT_MODE,
            output_mode: mode::DEFAULT_OUTPUT_MODE,
            input_cp: CP_437,
            output_cp: CP_437,
            title: Vec::new(),
            screen: Screen::new(width, height),
            input: VecDeque::new(),
            decoder: TermDecoder::default(),
            editor: None,
            pending: VecDeque::new(),
            pending_bytes: VecDeque::new(),
            reading: false,
            abort: false,
            ctrl: VecDeque::new(),
            out_decoder: Utf8Decoder::default(),
            extra: Vec::new(),
        }
    }

    // ----- input

    /// Bytes from the terminal.
    pub fn feed(&mut self, bytes: &[u8]) {
        let keys = self.decoder.decode(bytes);
        self.push_keys(&keys);
    }

    /// Key presses (each gets its release).
    pub fn push_keys(&mut self, keys: &[KeyEvent]) {
        for k in keys {
            let ctrl_c = k.ch == 0x03 && k.state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0;
            if ctrl_c && self.input_mode & ENABLE_PROCESSED_INPUT != 0 {
                self.ctrl.push_back(CTRL_C_EVENT);
                self.editor = None;
                if self.reading {
                    self.abort = true;
                }
                continue;
            }
            self.push_records(&input::records(core::slice::from_ref(k)));
        }
    }

    /// Records as `WriteConsoleInputW` puts them.
    pub fn push_records(&mut self, records: &[InputRecord]) {
        for r in records {
            if self.input.len() == MAX_INPUT_RECORDS {
                self.input.pop_front();
            }
            self.input.push_back(*r);
        }
    }

    /// The next control event raised, oldest first.
    pub fn take_ctrl(&mut self) -> Option<u32> {
        self.ctrl.pop_front()
    }

    /// `GetNumberOfConsoleInputEvents`.
    pub fn input_count(&self) -> usize {
        self.input.len()
    }

    /// `FlushConsoleInputBuffer` (a line being typed goes too).
    pub fn flush_input(&mut self) {
        self.input.clear();
        self.editor = None;
    }

    /// `PeekConsoleInputW` (`remove` false) or `ReadConsoleInputW`: up to
    /// `max` records. An empty result for a read means it must wait.
    pub fn read_input(&mut self, max: usize, remove: bool) -> Vec<InputRecord> {
        let n = max.min(self.input.len());
        if remove { self.input.drain(..n).collect() } else { self.input.iter().take(n).copied().collect() }
    }

    /// `ReadConsoleW` (`unicode`) or `ReadConsoleA` / `ReadFile`: up to
    /// `max` UTF-16 units or bytes. `None`: wait for more input.
    pub fn read_text(&mut self, max: usize, unicode: bool) -> Option<ReadOutcome> {
        if core::mem::take(&mut self.abort) {
            self.reading = false;
            return Some(ReadOutcome::Aborted);
        }
        if max == 0 {
            return Some(self.done(if unicode {
                ReadOutcome::Units(Vec::new())
            } else {
                ReadOutcome::Bytes(Vec::new())
            }));
        }
        if !unicode && !self.pending_bytes.is_empty() {
            let bytes = self.take_bytes(max);
            return Some(self.done(ReadOutcome::Bytes(bytes)));
        }
        if self.pending.is_empty() {
            if self.input_mode & ENABLE_LINE_INPUT != 0 {
                let line = self.cooked()?;
                self.pending.extend(line);
            } else {
                let chars = self.raw(if unicode { max } else { max.max(1) });
                if chars.is_empty() {
                    self.reading = true;
                    return None;
                }
                self.pending.extend(chars);
            }
        }
        let out = if unicode {
            let n = max.min(self.pending.len());
            // Never split a surrogate pair when the buffer holds more.
            let n = if n > 0 && n < self.pending.len() && (0xD800..0xDC00).contains(&self.pending[n - 1]) && n > 1 {
                n - 1
            } else {
                n
            };
            ReadOutcome::Units(self.pending.drain(..n).collect())
        } else {
            let units: Vec<u16> = self.pending.drain(..).collect();
            self.pending_bytes.extend(codepage::encode(self.input_cp, &units));
            ReadOutcome::Bytes(self.take_bytes(max))
        };
        Some(self.done(out))
    }

    fn done(&mut self, out: ReadOutcome) -> ReadOutcome {
        self.reading = false;
        out
    }

    fn take_bytes(&mut self, max: usize) -> Vec<u8> {
        let n = max.min(self.pending_bytes.len());
        self.pending_bytes.drain(..n).collect()
    }

    /// Run the line editor over the input buffer; the line once Enter came.
    fn cooked(&mut self) -> Option<Vec<u16>> {
        let mut editor = self.editor.take().unwrap_or_default();
        let insert = self.input_mode & ENABLE_INSERT_MODE != 0;
        let echo = self.input_mode & ENABLE_ECHO_INPUT != 0;
        while let Some(r) = self.input.pop_front() {
            let Some(k) = r.key() else { continue };
            let screen = if echo { Some(&mut self.screen) } else { None };
            if let Some(line) = editor.key(k, insert, screen) {
                return Some(line);
            }
        }
        self.editor = Some(editor);
        self.reading = true;
        None
    }

    /// Characters typed, for a read without `ENABLE_LINE_INPUT`.
    fn raw(&mut self, max: usize) -> Vec<u16> {
        let mut out = Vec::new();
        while out.len() < max {
            let Some(r) = self.input.front() else { break };
            if let Some(k) = r.key()
                && k.down
                && k.ch != 0
            {
                for _ in 0..k.repeat.max(1) {
                    out.push(k.ch);
                }
            }
            self.input.pop_front();
        }
        out
    }

    // ----- output

    /// `WriteConsoleW`: the units written (all of them).
    pub fn write_units(&mut self, text: &[u16]) -> usize {
        self.screen.write(text, self.output_mode);
        self.take_title();
        text.len()
    }

    /// `WriteConsoleA` / `WriteFile`: bytes in the output code page; all
    /// are taken (an unfinished UTF-8 sequence waits for the next write).
    pub fn write_bytes(&mut self, bytes: &[u8]) -> usize {
        let units = self.out_decoder.decode(self.output_cp, bytes);
        self.write_units(&units);
        bytes.len()
    }

    fn take_title(&mut self) {
        if let Some(t) = self.screen.title.take() {
            self.title = t;
        }
    }

    /// `SetConsoleTitleW`: the title, also sent to the terminal (`OSC 0`).
    pub fn set_title(&mut self, title: &[u16]) {
        self.title = title.to_vec();
        let shown: Vec<u16> = title.iter().copied().filter(|&u| u >= 0x20).collect();
        self.extra.extend_from_slice(b"\x1b]0;");
        self.extra.extend(codepage::encode(CP_UTF8, &shown));
        self.extra.push(0x07);
    }

    /// The bytes to send to the terminal.
    pub fn take_output(&mut self) -> Vec<u8> {
        let mut out = self.screen.take_output();
        out.extend(core::mem::take(&mut self.extra));
        out
    }

    // ----- modes and code pages

    pub fn set_input_mode(&mut self, m: u32) -> Result<(), u32> {
        self.input_mode = mode::set_input(self.input_mode, m)?;
        Ok(())
    }

    pub fn set_output_mode(&mut self, m: u32) -> Result<(), u32> {
        self.output_mode = mode::set_output(m)?;
        Ok(())
    }

    /// `SetConsoleCP` (`output` false) / `SetConsoleOutputCP`.
    pub fn set_cp(&mut self, cp: u32, output: bool) -> Result<(), u32> {
        if !codepage::is_supported(cp) {
            return Err(mode::ERROR_INVALID_PARAMETER);
        }
        if output {
            self.output_cp = cp;
        } else {
            self.input_cp = cp;
        }
        Ok(())
    }

    /// The console goes away: give the terminal back.
    pub fn close(&mut self) {
        self.screen.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::String;

    fn units(o: Option<ReadOutcome>) -> String {
        match o {
            Some(ReadOutcome::Units(u)) => String::from_utf16(&u).unwrap(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn cooked_read_with_typeahead_and_small_buffers() {
        let mut c = Console::new(40, 10);
        c.feed(b"hello\rworld\r");
        assert_eq!(units(c.read_text(100, true)), "hello\r\n");
        assert_eq!(units(c.read_text(3, true)), "wor");
        assert_eq!(units(c.read_text(100, true)), "ld\r\n");
        assert_eq!(c.read_text(100, true), None);
        c.feed(b"x");
        assert_eq!(c.read_text(100, true), None);
        c.feed(b"\r");
        assert_eq!(units(c.read_text(100, true)), "x\r\n");
        // The echo is on the screen.
        assert_eq!(c.screen.row_text(0), "hello");
        assert_eq!(c.screen.row_text(1), "world");
        assert_eq!(c.screen.row_text(2), "x");
    }

    #[test]
    fn ansi_reads_use_the_input_code_page() {
        let mut c = Console::new(40, 10);
        c.feed("é\r".as_bytes());
        assert_eq!(c.read_text(10, false), Some(ReadOutcome::Bytes(alloc::vec![0x82, b'\r', b'\n'])));
        c.set_cp(CP_UTF8, false).unwrap();
        c.feed("é\r".as_bytes());
        assert_eq!(c.read_text(1, false), Some(ReadOutcome::Bytes(alloc::vec![0xC3])));
        assert_eq!(c.read_text(10, false), Some(ReadOutcome::Bytes(alloc::vec![0xA9, b'\r', b'\n'])));
        assert_eq!(c.set_cp(42, true), Err(mode::ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn raw_reads_and_records() {
        let mut c = Console::new(40, 10);
        c.set_input_mode(0).unwrap();
        c.feed(b"ab");
        assert_eq!(c.input_count(), 4);
        assert_eq!(c.read_input(10, false).len(), 4);
        assert_eq!(units(c.read_text(1, true)), "a");
        assert_eq!(units(c.read_text(10, true)), "b");
        assert_eq!(c.read_text(10, true), None);
        c.feed(b"\x03");
        let r = c.read_input(10, true);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].key().unwrap().ch, 3);
        assert_eq!(c.take_ctrl(), None);
        assert!(c.screen.row_text(0).is_empty(), "no echo");
    }

    #[test]
    fn ctrl_c_is_an_event_and_aborts_a_waiting_read() {
        let mut c = Console::new(40, 10);
        c.feed(b"par");
        assert_eq!(c.read_text(10, true), None);
        c.feed(b"\x03");
        assert_eq!(c.take_ctrl(), Some(CTRL_C_EVENT));
        assert_eq!(c.read_text(10, true), Some(ReadOutcome::Aborted));
        c.feed(b"new\r");
        assert_eq!(units(c.read_text(10, true)), "new\r\n");
        // Without a read waiting, no abort is left behind.
        c.feed(b"\x03");
        assert_eq!(c.take_ctrl(), Some(CTRL_C_EVENT));
        c.feed(b"z\r");
        assert_eq!(units(c.read_text(10, true)), "z\r\n");
    }

    #[test]
    fn writes_in_the_output_code_page() {
        let mut c = Console::new(20, 5);
        c.write_bytes(&[0x82]);
        assert_eq!(c.screen.row_text(0), "é");
        c.set_cp(CP_UTF8, true).unwrap();
        let s = "ü✓".as_bytes();
        c.write_bytes(&s[..1]);
        c.write_bytes(&s[1..3]);
        c.write_bytes(&s[3..]);
        assert_eq!(c.screen.row_text(0), "éü✓");
        let out = String::from_utf8(c.take_output()).unwrap();
        assert!(out.ends_with("éü✓"), "{out:?}");
    }

    #[test]
    fn titles() {
        let mut c = Console::new(20, 5);
        let t: Vec<u16> = "Poly".encode_utf16().collect();
        c.set_title(&t);
        assert_eq!(c.title, t);
        assert_eq!(c.take_output(), b"\x1b]0;Poly\x07");
        c.output_mode |= ENABLE_VIRTUAL_TERMINAL_PROCESSING;
        let seq: Vec<u16> = "\x1b]2;Other\x07".encode_utf16().collect();
        c.write_units(&seq);
        assert_eq!(String::from_utf16(&c.title).unwrap(), "Other");
    }
}
