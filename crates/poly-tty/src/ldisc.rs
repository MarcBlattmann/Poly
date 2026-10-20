//! The line discipline: what happens to bytes between the terminal (keys,
//! a serial line, a pty master) and the programs reading and writing it,
//! as termios(3) and POSIX's "General Terminal Interface" describe it.
//!
//! - **Input** ([`LineDiscipline::receive`]): `ISTRIP`, `IUCLC`, literal
//!   next (`LNEXT`), flow control (`IXON`: `STOP` / `START`, `IXANY`),
//!   signals (`ISIG`: `INTR`, `QUIT`, `SUSP`, with the queues flushed
//!   unless `NOFLSH`), `IGNCR` / `ICRNL` / `INLCR`, then either canonical
//!   line editing (`ICANON`: `ERASE`, `KILL`, `WERASE`, `REPRINT`, `EOF`,
//!   `EOL`, `EOL2`, lines of at most [`MAX_LINE`] characters) or the raw
//!   byte stream. Echo (`ECHO`, `ECHOE`, `ECHOK`, `ECHONL`, `ECHOCTL`,
//!   `ECHOKE`) goes through output processing like program output.
//! - **Reading** ([`LineDiscipline::read`]): a whole line at most in
//!   canonical mode (end of file for `EOF` at the start of a line);
//!   otherwise `MIN` / `TIME` decide when a read is satisfied, and the
//!   caller gets a deadline to wait for.
//! - **Output** ([`LineDiscipline::output`]): `OPOST` with `ONLCR`,
//!   `OCRNL`, `ONOCR`, `ONLRET`, `OLCUC` and tab expansion (`TAB3`),
//!   keeping track of the cursor's column (for erasing tabs).
//! - The input buffer holds [`INPUT_BUFFER`] bytes; input that does not
//!   fit is dropped (a bell is echoed with `IMAXBEL`), and
//!   [`LineDiscipline::room`] tells a pty master how much it may write.
//!   With `IXOFF` the discipline sends `STOP` when the buffer is nearly
//!   full and `START` once readers drained it.
//!
//! Everything here is plain data: no locks, no time source, no signals
//! sent. Callers pass the time in and act on the [`Effects`].
//!
//! Points the documents leave open are Poly's choices, marked "to be
//! confirmed by the diff runner": `EOF` is not echoed; `LNEXT` echoes
//! `^` and a backspace (with `ECHO` and `ECHOCTL`), which the quoted
//! byte's echo overwrites; a literal byte skips flow control, signals and
//! the CR / NL mappings and never ends a line; switching to canonical mode
//! makes unread input one line; `ECHOPRT` and `XCASE` are not
//! implemented.

use crate::sig::{SIGINT, SIGQUIT, SIGTSTP};
use crate::termios::{Termios, cc, iflag, lflag, oflag};
use alloc::collections::VecDeque;
use alloc::vec::Vec;

/// Bytes of input the discipline holds (termios(3): "The maximum line
/// length is 4096 chars (including the terminating newline character)").
pub const INPUT_BUFFER: usize = 4096;
/// Characters of a canonical line before its delimiter; more are
/// discarded (termios(3): "any input data after 4095 characters up to
/// (but not including) any terminating newline is discarded").
pub const MAX_LINE: usize = INPUT_BUFFER - 1;
/// With `IXOFF`, `STOP` is sent once this many bytes wait to be read...
const HIGH_WATER: usize = INPUT_BUFFER - 512;
/// ... and `START` once readers took the buffer below this.
const LOW_WATER: usize = 128;
/// Nanoseconds per unit of `TIME` (tenths of a second).
const TIME_UNIT_NS: u64 = 100_000_000;
/// The terminal bell, echoed for input that does not fit (`IMAXBEL`).
const BEL: u8 = 0x07;

/// What processing input asks the terminal to do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// Bytes for the terminal (echo, flow-control characters), already
    /// through output processing. They go out even while output is
    /// stopped.
    pub output: Vec<u8>,
    /// Signals for the foreground process group, in order.
    pub signals: Vec<u32>,
    /// Something became readable.
    pub readable: bool,
    /// Output was started again (`START`, `IXANY`): wake writers.
    pub output_started: bool,
    /// Throw away output not sent yet (a signal character without
    /// `NOFLSH`).
    pub flush_output: bool,
}

/// What a read found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// This many bytes were copied (0 for a non-canonical read whose
    /// `MIN` and `TIME` allow returning nothing).
    Data(usize),
    /// End of file: `EOF` at the start of a canonical line.
    Eof,
    /// Nothing yet: wait for input, or until the monotonic clock reads
    /// `until` (then read again).
    Wait { until: Option<u64> },
}

/// The line discipline of one terminal (see the module documentation).
#[derive(Clone, Debug)]
pub struct LineDiscipline {
    termios: Termios,
    /// Bytes readers can take; in canonical mode only complete lines.
    read_buf: VecDeque<u8>,
    /// Canonical mode: the length of each complete line in `read_buf`,
    /// oldest first. 0 is an end of file.
    lines: VecDeque<usize>,
    /// Canonical mode: the line being edited.
    line: Vec<u8>,
    /// The next byte is taken literally (after `LNEXT`).
    literal: bool,
    /// Output is stopped (`STOP`, `TCOOFF`).
    stopped: bool,
    /// The output cursor's column, as far as output through the
    /// discipline tells.
    column: usize,
    /// The column at which the line being edited started.
    line_column: usize,
    /// `STOP` was sent for input flow control (`IXOFF`).
    throttled: bool,
    /// When the last input byte arrived (monotonic nanoseconds).
    last_input_ns: u64,
}

fn is_control(c: u8) -> bool {
    c < 0x20 || c == 0x7f
}

fn is_blank(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

impl LineDiscipline {
    pub fn new(termios: Termios) -> LineDiscipline {
        LineDiscipline {
            termios,
            read_buf: VecDeque::new(),
            lines: VecDeque::new(),
            line: Vec::new(),
            literal: false,
            stopped: false,
            column: 0,
            line_column: 0,
            throttled: false,
            last_input_ns: 0,
        }
    }

    pub fn termios(&self) -> &Termios {
        &self.termios
    }

    /// Whether output is stopped (by `STOP` or `TCOOFF`).
    pub fn output_stopped(&self) -> bool {
        self.stopped
    }

    /// `tcflow(TCOOFF / TCOON)`: stop or restart output. Returns whether
    /// output was started again.
    pub fn set_output_stopped(&mut self, stopped: bool) -> bool {
        let started = self.stopped && !stopped;
        self.stopped = stopped;
        started
    }

    /// Bytes a read could take now (`FIONREAD`): in canonical mode the
    /// complete lines.
    pub fn readable_bytes(&self) -> usize {
        self.read_buf.len()
    }

    /// Whether a read would return without waiting for more input (for
    /// `poll`): a complete line or end of file in canonical mode, any
    /// byte otherwise.
    pub fn readable(&self) -> bool {
        if self.termios.canonical() { !self.lines.is_empty() } else { !self.read_buf.is_empty() }
    }

    /// How many bytes of input the discipline can take now. A pty master
    /// writes at most this much and waits for the rest. In canonical mode
    /// past [`MAX_LINE`] characters, input is discarded rather than held,
    /// so there is always room.
    pub fn room(&self) -> usize {
        if self.termios.canonical() && self.line.len() >= MAX_LINE {
            return usize::MAX;
        }
        INPUT_BUFFER.saturating_sub(self.read_buf.len() + self.line.len())
    }

    /// Change the attributes (`TCSETS`); `flush` throws away pending input
    /// first (`TCSETSF`). Leaving canonical mode makes the line being
    /// edited readable; entering it makes unread input one line.
    pub fn set_termios(&mut self, new: Termios, flush: bool, out: &mut Effects) {
        if flush {
            self.flush_input(out);
        }
        let was = self.termios.canonical();
        let now = new.canonical();
        if was && !now {
            self.read_buf.extend(self.line.drain(..));
            self.lines.clear();
            out.readable |= !self.read_buf.is_empty();
        } else if !was && now {
            self.lines.clear();
            if !self.read_buf.is_empty() {
                self.lines.push_back(self.read_buf.len());
                out.readable = true;
            }
            self.line_column = self.column;
        }
        if self.stopped && new.iflag & iflag::IXON == 0 && self.termios.has_i(iflag::IXON) {
            self.stopped = false;
            out.output_started = true;
        }
        self.termios = new;
    }

    /// Throw away all input (`TCIFLUSH`).
    pub fn flush_input(&mut self, out: &mut Effects) {
        self.read_buf.clear();
        self.lines.clear();
        self.line.clear();
        self.literal = false;
        self.line_column = self.column;
        self.unthrottle(out);
    }

    // ---- input -----------------------------------------------------------

    /// Bytes arrived from the terminal at `now_ns`.
    pub fn receive(&mut self, bytes: &[u8], now_ns: u64, out: &mut Effects) {
        for &b in bytes {
            self.receive_byte(b, now_ns, out);
        }
    }

    fn receive_byte(&mut self, byte: u8, now_ns: u64, out: &mut Effects) {
        let t = self.termios;
        let mut c = byte;
        if t.has_i(iflag::ISTRIP) {
            c &= 0x7f;
        }
        if t.has_i(iflag::IUCLC) && t.has_l(lflag::IEXTEN) {
            c = c.to_ascii_lowercase();
        }
        self.last_input_ns = now_ns;
        if self.literal {
            // The echo overwrites the "^" shown for LNEXT.
            self.literal = false;
            self.insert(c, true, out);
            return;
        }
        if t.has_l(lflag::IEXTEN) && t.is(cc::VLNEXT, c) {
            self.literal = true;
            if t.canonical() && t.has_l(lflag::ECHO) && t.has_l(lflag::ECHOCTL) {
                self.emit(b'^', &mut out.output);
                self.emit(b'\x08', &mut out.output);
            }
            return;
        }
        if t.has_i(iflag::IXON) {
            let (stop, start) = (t.is(cc::VSTOP, c), t.is(cc::VSTART, c));
            if stop && start {
                // The same character for both toggles.
                self.stopped = !self.stopped;
                out.output_started |= !self.stopped;
                return;
            }
            if stop {
                self.stopped = true;
                return;
            }
            if start {
                if self.stopped {
                    self.stopped = false;
                    out.output_started = true;
                }
                return;
            }
            if self.stopped && t.has_i(iflag::IXANY) {
                self.stopped = false;
                out.output_started = true;
            }
        }
        if t.has_l(lflag::ISIG) {
            let signal = if t.is(cc::VINTR, c) {
                Some(SIGINT)
            } else if t.is(cc::VQUIT, c) {
                Some(SIGQUIT)
            } else if t.is(cc::VSUSP, c) {
                Some(SIGTSTP)
            } else {
                None
            };
            if let Some(signal) = signal {
                if !t.has_l(lflag::NOFLSH) {
                    self.flush_input(out);
                    out.flush_output = true;
                }
                if t.has_l(lflag::ECHO) {
                    self.echo(c, &mut out.output);
                }
                // Typed after the echo, a new line starts where it ends.
                self.line_column = self.column;
                out.signals.push(signal);
                return;
            }
        }
        if c == b'\r' {
            if t.has_i(iflag::IGNCR) {
                return;
            }
            if t.has_i(iflag::ICRNL) {
                c = b'\n';
            }
        } else if c == b'\n' && t.has_i(iflag::INLCR) {
            c = b'\r';
        }
        if t.canonical() {
            self.canonical(c, out);
        } else {
            self.insert(c, false, out);
        }
    }

    /// One byte of canonical input (after the mappings).
    fn canonical(&mut self, c: u8, out: &mut Effects) {
        let t = self.termios;
        let iexten = t.has_l(lflag::IEXTEN);
        if t.is(cc::VERASE, c) {
            self.erase(Erase::Char, &mut out.output);
        } else if t.is(cc::VKILL, c) {
            self.kill(c, &mut out.output);
        } else if iexten && t.is(cc::VWERASE, c) {
            self.erase(Erase::Word, &mut out.output);
        } else if iexten && t.is(cc::VREPRINT, c) {
            self.reprint(c, &mut out.output);
        } else if t.is(cc::VEOF, c) {
            self.finish_line(out);
        } else {
            self.insert(c, false, out);
        }
    }

    /// Put `c` into the input as data: the line being edited (a delimiter
    /// finishes it, unless it is `literal`) or the read buffer.
    fn insert(&mut self, c: u8, literal: bool, out: &mut Effects) {
        let t = self.termios;
        let canonical = t.canonical();
        let delimiter = canonical
            && !literal
            && (c == b'\n' || t.is(cc::VEOL, c) || (t.has_l(lflag::IEXTEN) && t.is(cc::VEOL2, c)));
        let held = self.read_buf.len() + self.line.len();
        let fits = held < INPUT_BUFFER && (!canonical || delimiter || self.line.len() < MAX_LINE);
        if !fits {
            if t.has_i(iflag::IMAXBEL) && t.has_l(lflag::ECHO) {
                out.output.push(BEL);
            }
            return;
        }
        if canonical {
            self.line.push(c);
        } else {
            self.read_buf.push_back(c);
            out.readable = true;
        }
        let echo = t.has_l(lflag::ECHO) || (delimiter && c == b'\n' && t.has_l(lflag::ECHONL));
        if echo {
            self.echo(c, &mut out.output);
        }
        if delimiter {
            self.finish_line(out);
        }
        self.throttle(out);
    }

    /// The line being edited is complete (a delimiter, or `EOF`, which
    /// sends what was typed so far; on an empty line it is end of file).
    fn finish_line(&mut self, out: &mut Effects) {
        self.lines.push_back(self.line.len());
        self.read_buf.extend(self.line.drain(..));
        self.line_column = self.column;
        out.readable = true;
    }

    /// The columns the characters of the line being edited take on the
    /// screen as echoed: the column after each byte.
    fn line_columns(&self) -> Vec<usize> {
        let mut col = self.line_column;
        self.line
            .iter()
            .map(|&c| {
                col = match c {
                    b'\t' => (col / 8 + 1) * 8,
                    c if is_control(c) && self.echoed_as_caret(c) => col + 2,
                    c if is_control(c) => col,
                    // UTF-8 continuation bytes take no column of their own.
                    c if c & 0xc0 == 0x80 => col,
                    _ => col + 1,
                };
                col
            })
            .collect()
    }

    /// Whether control character `c` is echoed as `^X` (termios(3)
    /// `ECHOCTL`: "terminal special characters other than TAB, NL, START,
    /// and STOP are echoed as ^X").
    fn echoed_as_caret(&self, c: u8) -> bool {
        let t = &self.termios;
        is_control(c)
            && t.has_l(lflag::ECHOCTL)
            && !(c == b'\t' || c == b'\n' || t.is(cc::VSTART, c) || t.is(cc::VSTOP, c))
    }

    /// `ERASE` (the character before the cursor) or `WERASE` (the word
    /// before it, after any blanks), never past the start of the line.
    fn erase(&mut self, what: Erase, output: &mut Vec<u8>) {
        if self.line.is_empty() {
            return;
        }
        let t = self.termios;
        let cols = self.line_columns();
        let end_col = *cols.last().expect("not empty");
        let mut keep = self.line.len();
        match what {
            Erase::Char => {
                keep -= 1;
                if t.has_i(iflag::IUTF8) {
                    while keep > 0 && self.line[keep] & 0xc0 == 0x80 {
                        keep -= 1;
                    }
                }
            }
            Erase::Word => {
                while keep > 0 && is_blank(self.line[keep - 1]) {
                    keep -= 1;
                }
                while keep > 0 && !is_blank(self.line[keep - 1]) {
                    keep -= 1;
                }
            }
        }
        let start_col = if keep == 0 { self.line_column } else { cols[keep - 1] };
        if t.has_l(lflag::ECHO) {
            if t.has_l(lflag::ECHOE) {
                for _ in start_col..end_col {
                    output.extend_from_slice(b"\x08 \x08");
                }
                self.column = self.column.saturating_sub(end_col - start_col);
            } else {
                let c = if what == Erase::Char { t.cc[cc::VERASE] } else { t.cc[cc::VWERASE] };
                self.echo(c, output);
            }
        }
        self.line.truncate(keep);
    }

    /// `KILL`: drop the line being edited.
    fn kill(&mut self, c: u8, output: &mut Vec<u8>) {
        let t = self.termios;
        if t.has_l(lflag::ECHO) {
            if t.has_l(lflag::ECHOKE) && t.has_l(lflag::ECHOE) {
                if let Some(&end_col) = self.line_columns().last() {
                    for _ in self.line_column..end_col {
                        output.extend_from_slice(b"\x08 \x08");
                    }
                    self.column = self.column.saturating_sub(end_col - self.line_column);
                }
            } else {
                self.echo(c, output);
                if t.has_l(lflag::ECHOK) {
                    self.emit(b'\n', output);
                }
                self.line_column = self.column;
            }
        }
        self.line.clear();
    }

    /// `REPRINT`: show the character, a new line, and the line being
    /// edited again.
    fn reprint(&mut self, c: u8, output: &mut Vec<u8>) {
        if !self.termios.has_l(lflag::ECHO) {
            return;
        }
        self.echo(c, output);
        self.emit(b'\n', output);
        self.line_column = self.column;
        let line = core::mem::take(&mut self.line);
        for &b in &line {
            self.echo(b, output);
        }
        self.line = line;
    }

    /// Show input character `c` (with `ECHOCTL`, control characters other
    /// than TAB, NL, START and STOP as `^X`, termios(3)).
    fn echo(&mut self, c: u8, output: &mut Vec<u8>) {
        if self.echoed_as_caret(c) {
            self.emit(b'^', output);
            self.emit(c ^ 0x40, output);
        } else {
            self.emit(c, output);
        }
    }

    /// Send `STOP` (`IXOFF`) when the buffer is nearly full.
    fn throttle(&mut self, out: &mut Effects) {
        let t = self.termios;
        if t.has_i(iflag::IXOFF) && !self.throttled && self.read_buf.len() + self.line.len() >= HIGH_WATER {
            if let Some(stop) = t.special(cc::VSTOP) {
                out.output.push(stop);
            }
            self.throttled = true;
        }
    }

    /// Send `START` once the buffer drained.
    fn unthrottle(&mut self, out: &mut Effects) {
        if self.throttled && self.read_buf.len() + self.line.len() < LOW_WATER {
            self.throttled = false;
            if self.termios.has_i(iflag::IXOFF)
                && let Some(start) = self.termios.special(cc::VSTART)
            {
                out.output.push(start);
            }
        }
    }

    // ---- reading -----------------------------------------------------------

    /// A read of up to `buf.len()` bytes that started at `start_ns`, tried
    /// at `now_ns`. In canonical mode it returns at most one line; in
    /// non-canonical mode `MIN` and `TIME` decide (termios(3)): with both
    /// 0 whatever is there; `MIN` alone waits for that many bytes (or
    /// `buf.len()`, if fewer); `TIME` alone waits until a byte arrives or
    /// `TIME` tenths of a second passed since the read started; both make
    /// `TIME` an inter-byte timer that runs from the latest byte once one
    /// arrived.
    pub fn read(&mut self, buf: &mut [u8], start_ns: u64, now_ns: u64, out: &mut Effects) -> Read {
        let r = self.try_read(buf, start_ns, now_ns);
        if matches!(r, Read::Data(1..)) {
            self.unthrottle(out);
        }
        r
    }

    fn try_read(&mut self, buf: &mut [u8], start_ns: u64, now_ns: u64) -> Read {
        if self.termios.canonical() {
            let Some(&len) = self.lines.front() else { return Read::Wait { until: None } };
            if len == 0 {
                self.lines.pop_front();
                return Read::Eof;
            }
            let n = len.min(buf.len());
            self.take(&mut buf[..n]);
            if n == len {
                self.lines.pop_front();
            } else if let Some(front) = self.lines.front_mut() {
                *front -= n;
            }
            return Read::Data(n);
        }
        let avail = self.read_buf.len();
        let (min, time) = self.termios.min_time();
        let (min, time) = (usize::from(min), u64::from(time));
        let wanted = min.min(buf.len());
        if avail > 0 && avail >= wanted {
            return Read::Data(self.take_up_to(buf));
        }
        match (min, time) {
            (0, 0) => Read::Data(0),
            (0, t) => {
                let deadline = start_ns.saturating_add(t * TIME_UNIT_NS);
                if now_ns >= deadline { Read::Data(0) } else { Read::Wait { until: Some(deadline) } }
            }
            (_, 0) => Read::Wait { until: None },
            (_, t) if avail > 0 => {
                let deadline = self.last_input_ns.saturating_add(t * TIME_UNIT_NS);
                if now_ns >= deadline { Read::Data(self.take_up_to(buf)) } else { Read::Wait { until: Some(deadline) } }
            }
            _ => Read::Wait { until: None },
        }
    }

    fn take_up_to(&mut self, buf: &mut [u8]) -> usize {
        let n = self.read_buf.len().min(buf.len());
        self.take(&mut buf[..n]);
        n
    }

    fn take(&mut self, buf: &mut [u8]) {
        let n = buf.len();
        for (dst, src) in buf.iter_mut().zip(self.read_buf.drain(..n)) {
            *dst = src;
        }
    }

    // ---- output --------------------------------------------------------------

    /// Process program output `input` into `out`, with at most `room`
    /// bytes of result (one input byte becomes at most 8). Returns how
    /// many input bytes were taken.
    pub fn output(&mut self, input: &[u8], room: usize, out: &mut Vec<u8>) -> usize {
        let start = out.len();
        let mut scratch = Vec::with_capacity(8);
        for (i, &c) in input.iter().enumerate() {
            scratch.clear();
            let column = self.column;
            self.emit(c, &mut scratch);
            if out.len() - start + scratch.len() > room {
                self.column = column;
                return i;
            }
            out.extend_from_slice(&scratch);
        }
        input.len()
    }

    /// One byte through output processing (`OPOST`), tracking the column.
    fn emit(&mut self, c: u8, out: &mut Vec<u8>) {
        let t = self.termios;
        if !t.has_o(oflag::OPOST) {
            self.advance(c);
            out.push(c);
            return;
        }
        match c {
            b'\n' => {
                if t.has_o(oflag::ONLCR) {
                    out.extend_from_slice(b"\r\n");
                    self.column = 0;
                } else {
                    out.push(b'\n');
                    if t.has_o(oflag::ONLRET) {
                        self.column = 0;
                    }
                }
            }
            b'\r' => {
                if t.has_o(oflag::ONOCR) && self.column == 0 {
                    return;
                }
                if t.has_o(oflag::OCRNL) {
                    out.push(b'\n');
                    if t.has_o(oflag::ONLRET) {
                        self.column = 0;
                    }
                } else {
                    out.push(b'\r');
                    self.column = 0;
                }
            }
            b'\t' => {
                let next = (self.column / 8 + 1) * 8;
                if t.oflag & oflag::TABDLY == oflag::TAB3 {
                    out.extend(core::iter::repeat_n(b' ', next - self.column));
                } else {
                    out.push(b'\t');
                }
                self.column = next;
            }
            c => {
                let c = if t.has_o(oflag::OLCUC) { c.to_ascii_uppercase() } else { c };
                self.advance(c);
                out.push(c);
            }
        }
    }

    /// Move the column over `c` as a terminal shows it.
    fn advance(&mut self, c: u8) {
        match c {
            b'\x08' => self.column = self.column.saturating_sub(1),
            b'\r' | b'\n' => self.column = 0,
            b'\t' => self.column = (self.column / 8 + 1) * 8,
            c if is_control(c) => {}
            c if c & 0xc0 == 0x80 => {}
            _ => self.column += 1,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Erase {
    Char,
    Word,
}
