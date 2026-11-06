//! The kernel log buffer behind syslog(2) (ADR 0135): the text dmesg(1)
//! shows, as lines `<level>[seconds.micros] message\n`.
//!
//! The text is kept formatted, in a ring of [`BUFFER_SIZE`] bytes of which
//! the oldest whole lines go first. Positions are absolute byte counts
//! (never reused), so the readers' places stay valid while old text
//! leaves: the place `READ` (action 2) has reached, and the place
//! `CLEAR` (action 5) or `READ_CLEAR` (action 4) set for `READ_ALL`
//! (action 3).

use crate::ns::lock::Mutex;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::vec::Vec;

/// What syslog(2) reports as the buffer's size (`SYSLOG_ACTION_SIZE_BUFFER`).
pub const BUFFER_SIZE: usize = 262_144;

/// Default console log level that `SYSLOG_ACTION_CONSOLE_ON` restores.
const DEFAULT_CONSOLE_LEVEL: u8 = 7;

/// The log: text, readers' positions and the console log level.
#[derive(Debug)]
pub struct Klog {
    text: VecDeque<u8>,
    /// Absolute position of `text[0]`.
    base: u64,
    /// Where `READ` goes on.
    read_pos: u64,
    /// Where `READ_ALL` starts (after `CLEAR`).
    clear_pos: u64,
    /// The console log level (1 to 8).
    console_level: u8,
    /// The level `CONSOLE_OFF` replaced, for `CONSOLE_ON`.
    saved_level: u8,
    seeded: bool,
}

impl Klog {
    pub const fn new() -> Klog {
        Klog {
            text: VecDeque::new(),
            base: 0,
            read_pos: 0,
            clear_pos: 0,
            console_level: DEFAULT_CONSOLE_LEVEL,
            saved_level: DEFAULT_CONSOLE_LEVEL,
            seeded: false,
        }
    }

    /// Absolute position of the end of the text.
    fn end(&self) -> u64 {
        self.base + self.text.len() as u64
    }

    /// Append one message (`level` 0 to 7, `micros` since boot). A newline
    /// inside the text starts a record of its own.
    pub fn append(&mut self, level: u8, micros: u64, message: &str) {
        for line in message.split('\n').filter(|l| !l.is_empty()) {
            let stamp = format!("<{}>[{:>5}.{:06}] {}\n", level.min(7), micros / 1_000_000, micros % 1_000_000, line);
            self.text.extend(stamp.bytes());
        }
        // The oldest whole lines leave when the buffer is over full.
        while self.text.len() > BUFFER_SIZE {
            let cut = self.text.iter().position(|&b| b == b'\n').map_or(self.text.len(), |p| p + 1);
            self.text.drain(..cut);
            self.base += cut as u64;
        }
    }

    /// The boot messages, once.
    fn seed(&mut self) {
        if !self.seeded {
            self.seeded = true;
            self.append(6, 0, "Poly: Linux personality started");
            self.append(6, 0, "Poly: kernel log buffer ready");
        }
    }

    /// The text from `from` (clamped to what is still there) to the end.
    fn since(&self, from: u64) -> Vec<u8> {
        let from = from.max(self.base);
        self.text.iter().skip((from - self.base) as usize).copied().collect()
    }

    /// `SYSLOG_ACTION_SIZE_UNREAD`.
    pub fn unread(&mut self) -> usize {
        self.seed();
        (self.end() - self.read_pos.max(self.base)) as usize
    }

    /// `SYSLOG_ACTION_READ`: up to `len` bytes of unread text, consumed.
    /// Empty when there is none.
    pub fn read(&mut self, len: usize) -> Vec<u8> {
        self.seed();
        let mut out = self.since(self.read_pos);
        out.truncate(len);
        self.read_pos = self.read_pos.max(self.base) + out.len() as u64;
        out
    }

    /// `SYSLOG_ACTION_READ_ALL`: the newest whole records after the last
    /// clear that fit in `len` bytes, oldest first. Nothing is consumed.
    pub fn read_all(&mut self, len: usize) -> Vec<u8> {
        self.seed();
        let all = self.since(self.clear_pos);
        // Walk back over whole lines while they fit.
        let mut start = all.len();
        let mut line_end = all.len();
        while line_end > 0 {
            let line_start = all[..line_end - 1].iter().rposition(|&b| b == b'\n').map_or(0, |p| p + 1);
            if all.len() - line_start > len {
                break;
            }
            start = line_start;
            line_end = line_start;
        }
        all[start..].to_vec()
    }

    /// `SYSLOG_ACTION_CLEAR`: `READ_ALL` shows nothing older than now.
    pub fn clear(&mut self) {
        self.seed();
        self.clear_pos = self.end();
    }

    /// `SYSLOG_ACTION_CONSOLE_OFF`.
    pub fn console_off(&mut self) {
        if self.console_level > 1 {
            self.saved_level = self.console_level;
        }
        self.console_level = 1;
    }

    /// `SYSLOG_ACTION_CONSOLE_ON`.
    pub fn console_on(&mut self) {
        self.console_level = self.saved_level;
    }

    /// `SYSLOG_ACTION_CONSOLE_LEVEL`: `level` 1 to 8.
    pub fn set_console_level(&mut self, level: u8) {
        self.console_level = level;
        self.saved_level = level;
    }

    pub fn console_level(&self) -> u8 {
        self.console_level
    }
}

impl Default for Klog {
    fn default() -> Self {
        Klog::new()
    }
}

/// The system's log.
static LOG: Mutex<Klog> = Mutex::new(Klog::new());

/// Run `f` on the system's log.
pub fn with<R>(f: impl FnOnce(&mut Klog) -> R) -> R {
    f(&mut LOG.lock())
}

/// Add a message to the system's log (for the kernel to report events).
pub fn log(level: u8, micros: u64, message: &str) {
    with(|l| {
        l.seed();
        l.append(level, micros, message);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A log without the boot lines.
    fn empty() -> Klog {
        let mut l = Klog::new();
        l.seeded = true;
        l
    }

    fn lines(b: &[u8]) -> Vec<&str> {
        core::str::from_utf8(b).unwrap().lines().collect()
    }

    #[test]
    fn records_have_the_dmesg_shape() {
        let mut l = empty();
        l.append(4, 12_345_678, "hello");
        assert_eq!(l.read_all(1000), b"<4>[   12.345678] hello\n");
    }

    #[test]
    fn boot_lines_exist_and_read_consumes() {
        let mut l = Klog::new();
        let size = l.unread();
        assert!(size > 0);
        let first = l.read(5);
        assert_eq!(first.len(), 5);
        assert_eq!(l.unread(), size - 5);
        let rest = l.read(1 << 20);
        assert_eq!(rest.len(), size - 5);
        assert_eq!(l.unread(), 0);
        assert!(l.read(10).is_empty());
        // READ_ALL is unaffected by reading.
        assert_eq!(l.read_all(1 << 20).len(), size);
        l.append(6, 1_000_000, "more");
        assert_eq!(l.unread(), b"<6>[    1.000000] more\n".len());
    }

    #[test]
    fn read_all_takes_the_newest_whole_records_that_fit() {
        let mut l = empty();
        l.append(6, 0, "aaaa");
        l.append(6, 0, "bbbb");
        l.append(6, 0, "cccc");
        let line = b"<6>[    0.000000] cccc\n".len();
        assert!(l.read_all(line - 1).is_empty(), "no record fits");
        let two = l.read_all(2 * line + 3);
        assert_eq!(lines(&two), ["<6>[    0.000000] bbbb", "<6>[    0.000000] cccc"]);
        // READ_ALL starts after the clear.
        l.clear();
        assert!(l.read_all(1 << 20).is_empty());
        l.append(6, 0, "dddd");
        assert_eq!(lines(&l.read_all(1 << 20)), ["<6>[    0.000000] dddd"]);
    }

    #[test]
    fn old_lines_leave_the_full_buffer_whole() {
        let mut l = empty();
        let big = "x".repeat(1000);
        for i in 0..400 {
            l.append(6, i, &big);
        }
        let all = l.read_all(usize::MAX);
        assert!(all.len() <= BUFFER_SIZE + 64);
        assert!(all.starts_with(b"<6>["), "cut at a record");
        assert!(all.ends_with(b"\n"));
        // A reader that fell behind starts at what is left.
        assert_eq!(l.unread(), all.len());
    }

    #[test]
    fn console_levels() {
        let mut l = Klog::new();
        assert_eq!(l.console_level(), 7);
        l.console_off();
        assert_eq!(l.console_level(), 1);
        l.console_on();
        assert_eq!(l.console_level(), 7);
        l.set_console_level(3);
        l.console_off();
        l.console_on();
        assert_eq!(l.console_level(), 3);
    }
}
