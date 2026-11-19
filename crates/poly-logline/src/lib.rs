//! Whole lines on a shared serial console (ADR 0228).
//!
//! The kernel log and the programs' terminal (`/dev/ttyS0`, which the NT
//! console draws on with VT sequences) share one serial port. The boot tests
//! read results from that port, so a line must reach it whole. Two things
//! tore lines apart:
//!
//! - A kernel log line written in several calls (`kprint!` pieces), each
//!   taking the port's lock on its own: another CPU's line fell between
//!   the pieces.
//! - A program writing one row of its terminal in several system calls (the
//!   Chrome launcher wrote a SHA-256 two hex digits at a time): a kernel log
//!   line from any CPU fell between the calls, into the middle of the row.
//!
//! A [`Gate`] sits in front of the port, under the port's lock, and fixes
//! both:
//!
//! - Kernel text is collected per writer ([`Gate::kernel`] names it: a
//!   thread, or a CPU's interrupt handlers) and goes out only as whole
//!   lines. A writer with an unfinished line has a slot of its own; text
//!   without its line end goes out after the window ([`Gate::poll`]), when
//!   its slot is full, or when every slot is taken and it is the oldest.
//! - While a program's terminal row is open (it printed text since the last
//!   line feed or cursor-position sequence, [`Row`]) and the program wrote
//!   within the window, whole kernel lines wait. They go out at the
//!   terminal's next row boundary, inside the write that reaches it, or
//!   once the terminal has been quiet for the window.
//!
//! Nothing is lost: when the waiting room is full, what waits goes out
//! first (tearing that one row, as before). Kernel lines keep their order
//! among each other; only their place relative to the terminal's rows moves.
//!
//! Everything is fixed-size (no heap: the kernel logs before it has one).
#![no_std]

/// Where a [`Gate`] sends what it lets through.
pub trait Port {
    /// Kernel log text with `\n` line ends (the port adds a `\r` if it needs one).
    fn kernel(&mut self, bytes: &[u8]);
    /// A terminal's output, as it is.
    fn tty(&mut self, bytes: &[u8]);
}

/// How long the gate holds kernel text for an open terminal row or an
/// unfinished kernel line: 250 ms. Long enough for a program to finish a
/// row it writes in pieces on a busy machine, short enough that a log line
/// never seems lost on an interactive terminal.
pub const WINDOW_NS: u64 = 250_000_000;

/// Whether a terminal's current row has text on it, followed through the
/// VT sequences of its output (ECMA-48: `ESC [` … final byte is a control
/// sequence, `ESC ]` … BEL or `ESC \` an operating system command).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    state: VtState,
    open: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum VtState {
    #[default]
    Ground,
    Escape,
    Csi,
    Osc,
    OscEscape,
}

impl Row {
    pub const fn new() -> Row {
        Row { state: VtState::Ground, open: false }
    }

    /// Whether the row has text on it.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Take one byte of output; true when it ends at a row boundary: a line
    /// feed, or the final byte of a cursor-position sequence (`CSI … H`,
    /// `CSI … f`, with which the NT console starts each row it draws).
    pub fn feed(&mut self, b: u8) -> bool {
        match self.state {
            VtState::Ground => match b {
                b'\n' => {
                    self.open = false;
                    return true;
                }
                0x1b => self.state = VtState::Escape,
                // Other controls (CR, BS, TAB, BEL) neither print nor end the row.
                0x00..=0x1f | 0x7f => {}
                _ => self.open = true,
            },
            VtState::Escape => {
                self.state = match b {
                    b'[' => VtState::Csi,
                    b']' => VtState::Osc,
                    _ => VtState::Ground,
                }
            }
            VtState::Csi => {
                if (0x40..=0x7e).contains(&b) {
                    self.state = VtState::Ground;
                    if b == b'H' || b == b'f' {
                        self.open = false;
                        return true;
                    }
                }
            }
            VtState::Osc => match b {
                0x07 => self.state = VtState::Ground,
                0x1b => self.state = VtState::OscEscape,
                _ => {}
            },
            VtState::OscEscape => self.state = VtState::Ground,
        }
        false
    }
}

/// One writer's kernel text that has no line end yet.
#[derive(Clone, Copy, Debug)]
struct Slot<const CAP: usize> {
    buf: [u8; CAP],
    len: usize,
    /// The writer, while `len > 0`.
    owner: u64,
    /// When its first byte came.
    since: u64,
}

impl<const CAP: usize> Slot<CAP> {
    const EMPTY: Slot<CAP> = Slot { buf: [0; CAP], len: 0, owner: 0, since: 0 };
}

/// The terminal's state and the whole kernel lines waiting for its row.
#[derive(Debug)]
struct Out<const PENDING: usize> {
    window_ns: u64,
    row: Row,
    /// When the terminal last wrote.
    tty_at: u64,
    pending: [u8; PENDING],
    pending_len: usize,
}

impl<const PENDING: usize> Out<PENDING> {
    /// Whether kernel lines wait now: the terminal's row is open and it
    /// wrote within the window.
    fn holding(&self, now: u64) -> bool {
        self.row.is_open() && now.saturating_sub(self.tty_at) < self.window_ns
    }

    fn flush(&mut self, port: &mut impl Port) {
        if self.pending_len > 0 {
            port.kernel(&self.pending[..self.pending_len]);
            self.pending_len = 0;
        }
    }

    fn hold(&mut self, bytes: &[u8]) -> bool {
        let end = self.pending_len + bytes.len();
        if end > PENDING {
            return false;
        }
        self.pending[self.pending_len..end].copy_from_slice(bytes);
        self.pending_len = end;
        true
    }

    /// Send kernel text out, or keep it while the terminal's row is open.
    fn deliver(&mut self, bytes: &[u8], now: u64, port: &mut impl Port) {
        if self.holding(now) {
            if self.hold(bytes) {
                return;
            }
            // No room: what waits goes out first, in order.
            self.flush(port);
            if self.hold(bytes) {
                return;
            }
            port.kernel(bytes);
            return;
        }
        self.flush(port);
        port.kernel(bytes);
    }

    fn tty(&mut self, bytes: &[u8], now: u64, port: &mut impl Port) {
        let mut start = 0;
        for (i, &b) in bytes.iter().enumerate() {
            if self.row.feed(b) && self.pending_len > 0 {
                port.tty(&bytes[start..=i]);
                start = i + 1;
                self.flush(port);
            }
        }
        if start < bytes.len() {
            port.tty(&bytes[start..]);
        }
        self.tty_at = now;
    }
}

/// The gate in front of a shared serial port (see the crate documentation).
/// `SLOTS` writers at a time can each have `SLOT_CAP` bytes of an unfinished
/// kernel line; `PENDING` bytes of whole kernel lines can wait for the terminal.
#[derive(Debug)]
pub struct Gate<const SLOTS: usize, const SLOT_CAP: usize, const PENDING: usize> {
    slots: [Slot<SLOT_CAP>; SLOTS],
    /// Slots with text in them.
    busy: usize,
    out: Out<PENDING>,
}

impl<const SLOTS: usize, const SLOT_CAP: usize, const PENDING: usize> Gate<SLOTS, SLOT_CAP, PENDING> {
    pub const fn new(window_ns: u64) -> Self {
        Gate {
            slots: [Slot::EMPTY; SLOTS],
            busy: 0,
            out: Out { window_ns, row: Row::new(), tty_at: 0, pending: [0; PENDING], pending_len: 0 },
        }
    }

    /// Whether nothing waits: no unfinished kernel line, no held line.
    pub fn is_idle(&self) -> bool {
        self.busy == 0 && self.out.pending_len == 0
    }

    /// Kernel log text from `writer` (any number that tells writers apart).
    pub fn kernel(&mut self, writer: u64, text: &[u8], now: u64, port: &mut impl Port) {
        self.poll(now, port);
        let mut rest = text;
        while !rest.is_empty() {
            match rest.iter().position(|&b| b == b'\n') {
                Some(nl) => {
                    let (line, after) = rest.split_at(nl + 1);
                    self.push(writer, line, true, now, port);
                    rest = after;
                }
                None => {
                    self.push(writer, rest, false, now, port);
                    break;
                }
            }
        }
    }

    /// A program's terminal output.
    pub fn tty(&mut self, bytes: &[u8], now: u64, port: &mut impl Port) {
        self.poll(now, port);
        self.out.tty(bytes, now, port);
    }

    /// Let out what waited long enough: held lines once the terminal has
    /// been quiet for the window, unfinished lines older than the window.
    pub fn poll(&mut self, now: u64, port: &mut impl Port) {
        if self.busy > 0 {
            for i in 0..SLOTS {
                let s = &self.slots[i];
                if s.len > 0 && now.saturating_sub(s.since) >= self.out.window_ns {
                    self.spill(i, now, port);
                }
            }
        }
        if self.out.pending_len > 0 && !self.out.holding(now) {
            self.out.flush(port);
        }
    }

    /// Let out everything, whatever the terminal does (before the machine
    /// stops, or on a panic).
    pub fn flush(&mut self, port: &mut impl Port) {
        self.out.flush(port);
        for i in 0..SLOTS {
            let s = &mut self.slots[i];
            if s.len > 0 {
                port.kernel(&s.buf[..s.len]);
                s.len = 0;
            }
        }
        self.busy = 0;
    }

    /// Send what slot `i` holds on, as it is.
    fn spill(&mut self, i: usize, now: u64, port: &mut impl Port) {
        let s = &mut self.slots[i];
        if s.len == 0 {
            return;
        }
        self.out.deliver(&s.buf[..s.len], now, port);
        s.len = 0;
        self.busy -= 1;
    }

    /// The slot holding `writer`'s unfinished line.
    fn find(&self, writer: u64) -> Option<usize> {
        if self.busy == 0 {
            return None;
        }
        (0..SLOTS).find(|&i| self.slots[i].len > 0 && self.slots[i].owner == writer)
    }

    /// A slot for `writer`: a free one, or the oldest one's text goes on first.
    fn claim(&mut self, writer: u64, now: u64, port: &mut impl Port) -> usize {
        let i = match (0..SLOTS).find(|&i| self.slots[i].len == 0) {
            Some(i) => i,
            None => {
                let oldest = (0..SLOTS).min_by_key(|&i| self.slots[i].since).unwrap_or(0);
                self.spill(oldest, now, port);
                oldest
            }
        };
        self.slots[i].owner = writer;
        i
    }

    /// Add `writer`'s `bytes`; `complete` when they end with the line end.
    fn push(&mut self, writer: u64, bytes: &[u8], complete: bool, now: u64, port: &mut impl Port) {
        let mut held = self.find(writer);
        if let Some(i) = held
            && self.slots[i].len + bytes.len() > SLOT_CAP
        {
            self.spill(i, now, port);
            held = None;
        }
        if held.is_none() && (complete || bytes.len() > SLOT_CAP) {
            // A whole line, or more than a slot holds: on as it is.
            self.out.deliver(bytes, now, port);
            return;
        }
        let i = match held {
            Some(i) => i,
            None => self.claim(writer, now, port),
        };
        let s = &mut self.slots[i];
        if s.len == 0 {
            s.since = now;
            self.busy += 1;
        }
        s.buf[s.len..s.len + bytes.len()].copy_from_slice(bytes);
        s.len += bytes.len();
        if complete {
            self.spill(i, now, port);
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::string::String;
    use std::vec::Vec;

    /// The port's byte stream, kernel and terminal output as they came.
    #[derive(Default)]
    struct Wire(Vec<u8>);

    impl Port for Wire {
        fn kernel(&mut self, bytes: &[u8]) {
            self.0.extend_from_slice(bytes);
        }
        fn tty(&mut self, bytes: &[u8]) {
            self.0.extend_from_slice(bytes);
        }
    }

    impl Wire {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0).into_owned()
        }
    }

    type Small = Gate<4, 32, 256>;
    const MS: u64 = 1_000_000;

    #[test]
    fn row_follows_line_feeds_and_cursor_positions() {
        let mut r = Row::new();
        assert!(!r.is_open());
        for &b in b"abc" {
            assert!(!r.feed(b));
        }
        assert!(r.is_open());
        assert!(r.feed(b'\n'));
        assert!(!r.is_open());
        // A colour change and a CR do not open the row; text does.
        for &b in b"\x1b[0m\r" {
            assert!(!r.feed(b));
        }
        assert!(!r.is_open());
        for &b in b"x\x1b[7;1" {
            assert!(!r.feed(b));
        }
        assert!(r.is_open());
        // The NT console starts each row with `CSI row ; 1 H`.
        assert!(r.feed(b'H'));
        assert!(!r.is_open());
        // An OSC's text is not on the row.
        for &b in b"\x1b]0;title\x07" {
            r.feed(b);
        }
        assert!(!r.is_open());
        for &b in b"\x1b]0;t\x1b\\" {
            r.feed(b);
        }
        assert!(!r.is_open());
    }

    #[test]
    fn a_line_in_pieces_goes_out_whole() {
        let (mut g, mut w) = (Small::new(WINDOW_NS), Wire::default());
        g.kernel(0, b"test a ... ", 0, &mut w);
        g.kernel(1, b"other line\n", 0, &mut w);
        g.kernel(0, b"ok", 0, &mut w);
        g.kernel(0, b"\n", 0, &mut w);
        assert_eq!(w.text(), "other line\ntest a ... ok\n");
        assert!(g.is_idle());
    }

    #[test]
    fn writers_never_share_a_slot() {
        let (mut g, mut w) = (Gate::<2, 32, 256>::new(WINDOW_NS), Wire::default());
        // Writers 0 and 4 (the same slot modulo anything) both have half a line.
        g.kernel(0, b"zero ", 0, &mut w);
        g.kernel(4, b"four ", 1, &mut w);
        g.kernel(
            4, b"done
", 2, &mut w,
        );
        g.kernel(
            0, b"done
", 3, &mut w,
        );
        assert_eq!(
            w.text(),
            "four done
zero done
"
        );
        // Every slot taken: the oldest unfinished line goes on first, whole so far.
        let mut w = Wire::default();
        g.kernel(1, b"a", 10, &mut w);
        g.kernel(2, b"b", 11, &mut w);
        g.kernel(3, b"c", 12, &mut w);
        assert_eq!(w.text(), "a");
        g.kernel(
            2, b"!
", 13, &mut w,
        );
        g.kernel(
            3, b"!
", 14, &mut w,
        );
        assert_eq!(
            w.text(),
            "ab!
c!
"
        );
        assert!(g.is_idle());
    }

    #[test]
    fn an_unfinished_line_goes_out_after_the_window() {
        let (mut g, mut w) = (Small::new(WINDOW_NS), Wire::default());
        g.kernel(0, b"prompt: ", 0, &mut w);
        g.poll(WINDOW_NS - 1, &mut w);
        assert_eq!(w.text(), "");
        assert!(!g.is_idle());
        g.poll(WINDOW_NS, &mut w);
        assert_eq!(w.text(), "prompt: ");
        assert!(g.is_idle());
    }

    #[test]
    fn a_line_longer_than_a_slot_still_goes_out_in_order() {
        let (mut g, mut w) = (Small::new(WINDOW_NS), Wire::default());
        let long = "x".repeat(100);
        g.kernel(0, b"head ", 0, &mut w);
        g.kernel(0, long.as_bytes(), 0, &mut w);
        g.kernel(0, b" tail\n", 0, &mut w);
        assert_eq!(w.text(), std::format!("head {long} tail\n"));
        // And as one write.
        let mut w = Wire::default();
        g.kernel(2, std::format!("{long}\n").as_bytes(), 0, &mut w);
        assert_eq!(w.text(), std::format!("{long}\n"));
    }

    #[test]
    fn kernel_lines_wait_for_an_open_terminal_row() {
        let (mut g, mut w) = (Small::new(WINDOW_NS), Wire::default());
        // The launcher's digest line, two hex digits per system call.
        g.tty(b"\x1b[6;1HLAUNCH-SHA256 stdout ", 0, &mut w);
        g.kernel(1, b"nt: pid 3: noise\n", MS, &mut w);
        g.tty(b"e3", 2 * MS, &mut w);
        g.kernel(2, b"fs: more noise\n", 3 * MS, &mut w);
        g.tty(b"b0", 4 * MS, &mut w);
        g.tty(b"\x1b[7;1Hnext", 5 * MS, &mut w);
        assert_eq!(w.text(), "\x1b[6;1HLAUNCH-SHA256 stdout e3b0\x1b[7;1Hnt: pid 3: noise\nfs: more noise\nnext");
    }

    #[test]
    fn a_line_feed_inside_a_write_lets_held_lines_out_there() {
        let (mut g, mut w) = (Small::new(WINDOW_NS), Wire::default());
        g.tty(b"result ", 0, &mut w);
        g.kernel(0, b"k\n", 0, &mut w);
        g.tty(b"42\nmore", 0, &mut w);
        assert_eq!(w.text(), "result 42\nk\nmore");
    }

    #[test]
    fn a_quiet_terminal_does_not_hold_lines_for_long() {
        let (mut g, mut w) = (Small::new(WINDOW_NS), Wire::default());
        g.tty(b"Password: ", 0, &mut w);
        g.kernel(0, b"held\n", MS, &mut w);
        assert_eq!(w.text(), "Password: ");
        g.poll(WINDOW_NS, &mut w);
        assert_eq!(w.text(), "Password: held\n");
        // Once the window has passed, lines go straight out.
        g.kernel(0, b"direct\n", WINDOW_NS + MS, &mut w);
        assert_eq!(w.text(), "Password: held\ndirect\n");
    }

    #[test]
    fn a_closed_row_holds_nothing() {
        let (mut g, mut w) = (Small::new(WINDOW_NS), Wire::default());
        g.tty(b"done\n", 0, &mut w);
        g.kernel(0, b"k\n", 0, &mut w);
        assert_eq!(w.text(), "done\nk\n");
    }

    #[test]
    fn a_full_waiting_room_lets_out_in_order() {
        let (mut g, mut w) = (Gate::<2, 16, 16>::new(WINDOW_NS), Wire::default());
        g.tty(b"row", 0, &mut w);
        g.kernel(0, b"0123456789\n", 0, &mut w);
        g.kernel(0, b"abcdefghij\n", 0, &mut w);
        assert_eq!(w.text(), "row0123456789\n");
        g.tty(b"\n", 0, &mut w);
        assert_eq!(w.text(), "row0123456789\n\nabcdefghij\n");
    }

    #[test]
    fn flush_lets_everything_out() {
        let (mut g, mut w) = (Small::new(WINDOW_NS), Wire::default());
        g.tty(b"row", 0, &mut w);
        g.kernel(0, b"whole\n", 0, &mut w);
        g.kernel(1, b"part", 0, &mut w);
        g.flush(&mut w);
        assert_eq!(w.text(), "rowwhole\npart");
        assert!(g.is_idle());
    }

    /// Many writers, many pieces: every kernel line and every terminal row
    /// reaches the port whole, kernel lines in the order they were finished.
    #[test]
    fn interleaved_writers_never_tear_a_line() {
        let (mut g, mut w) = (Gate::<8, 64, 4096>::new(WINDOW_NS), Wire::default());
        let mut want_kernel = Vec::new();
        let mut t = 0u64;
        for i in 0..200usize {
            let row = std::format!("RESULT {i} {:08x}", i * 2654435761usize % (1 << 32));
            // The row in 3-byte pieces, kernel text from three writers between them.
            for (n, piece) in row.as_bytes().chunks(3).enumerate() {
                g.tty(piece, t, &mut w);
                let writer = (n % 3) as u64;
                let line = std::format!("k{writer} {i} {n}\n");
                let (a, b) = line.as_bytes().split_at(2);
                g.kernel(writer, a, t, &mut w);
                g.kernel(writer, b, t, &mut w);
                want_kernel.push(line);
                t += 10_000;
            }
            g.tty(b"\n", t, &mut w);
        }
        g.flush(&mut w);
        let text = w.text();
        let lines: Vec<&str> = text.lines().collect();
        let results: Vec<&str> = lines.iter().copied().filter(|l| l.starts_with("RESULT ")).collect();
        assert_eq!(results.len(), 200);
        for (i, l) in results.iter().enumerate() {
            assert_eq!(*l, std::format!("RESULT {i} {:08x}", i * 2654435761usize % (1 << 32)));
        }
        let kernel: Vec<String> = lines.iter().filter(|l| l.starts_with('k')).map(|l| std::format!("{l}\n")).collect();
        assert_eq!(kernel, want_kernel);
        assert_eq!(results.len() + kernel.len(), lines.len());
    }
}
