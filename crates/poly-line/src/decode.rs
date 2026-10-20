//! Terminal input: the bytes a terminal emulator (or QEMU's serial console)
//! sends for the keys it is given, turned into [`Edit`]s.
//!
//! What a terminal sends (ECMA-48 and xterm conventions):
//!
//! | Bytes | Key | Edit |
//! |---|---|---|
//! | UTF-8 text | characters | `Insert` |
//! | `CR`, `LF`, `CR LF` | Enter | `Enter` (one for `CR LF`) |
//! | `DEL` (0x7f), `BS` (0x08) | Backspace | `Backspace` |
//! | `HT` (0x09) | Tab | `Complete` |
//! | 0x01-0x1a | Ctrl-A to Ctrl-Z | [`Edit::control`] |
//! | `ESC [ A` / `B` / `C` / `D` (or `ESC O` ...) | Up / Down / Right / Left | `HistoryPrev` / `HistoryNext` / `Right` / `Left` |
//! | `ESC [ 1 ; 5 C` / `D` (also `; 3`, Alt) | Ctrl-Right / Ctrl-Left | `WordRight` / `WordLeft` |
//! | `ESC b`, `ESC f` | Alt-B, Alt-F | `WordLeft`, `WordRight` |
//! | `ESC [ H`, `ESC [ 1 ~`, `ESC [ 7 ~`, `ESC O H` | Home | `Home` |
//! | `ESC [ F`, `ESC [ 4 ~`, `ESC [ 8 ~`, `ESC O F` | End | `End` |
//! | `ESC [ 3 ~` | Delete | `Delete` |
//!
//! Anything else (function keys, Insert, Page Up, malformed UTF-8, other
//! control bytes) is dropped. A lone `ESC` cannot be told apart from the
//! start of a sequence without a timeout, so it waits for the next byte; if
//! that does not continue a sequence, the `ESC` is dropped and the byte is
//! taken on its own.

use crate::Edit;

/// Longest parameter string kept in a control sequence; longer ones are
/// read to their end and dropped.
const MAX_PARAMS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    /// Inside a UTF-8 sequence: the code point so far and the continuation
    /// bytes still missing, and the smallest value it may encode (to refuse
    /// overlong forms).
    Utf8 {
        value: u32,
        missing: u8,
        min: u32,
    },
    Escape,
    /// `ESC [`: parameter bytes so far.
    Csi {
        params: [u8; MAX_PARAMS],
        len: u8,
        overflow: bool,
    },
    /// `ESC O`.
    Ss3,
}

/// Decodes terminal input one byte at a time. See the module documentation
/// for what it understands.
#[derive(Clone, Copy, Debug)]
pub struct TermDecoder {
    state: State,
    /// The previous byte was CR, so an LF right after it is not a second Enter.
    after_cr: bool,
}

impl Default for TermDecoder {
    fn default() -> Self {
        TermDecoder::new()
    }
}

impl TermDecoder {
    pub const fn new() -> TermDecoder {
        TermDecoder { state: State::Ground, after_cr: false }
    }

    /// Take one byte; the edit it completes, if any.
    pub fn feed(&mut self, byte: u8) -> Option<Edit> {
        let after_cr = core::mem::replace(&mut self.after_cr, false);
        match self.state {
            State::Ground => self.ground(byte, after_cr),
            State::Utf8 { value, missing, min } => {
                if byte & 0xc0 != 0x80 {
                    // Truncated sequence: drop it, start over with this byte.
                    self.state = State::Ground;
                    return self.ground(byte, false);
                }
                let value = value << 6 | u32::from(byte & 0x3f);
                if missing > 1 {
                    self.state = State::Utf8 { value, missing: missing - 1, min };
                    return None;
                }
                self.state = State::Ground;
                // Surrogates and values past U+10FFFF are refused here.
                char::from_u32(value).filter(|_| value >= min).filter(|c| !c.is_control()).map(Edit::Insert)
            }
            State::Escape => {
                self.state = State::Ground;
                match byte {
                    b'[' => self.state = State::Csi { params: [0; MAX_PARAMS], len: 0, overflow: false },
                    b'O' => self.state = State::Ss3,
                    b'b' => return Some(Edit::WordLeft),
                    b'f' => return Some(Edit::WordRight),
                    _ => return self.ground(byte, false),
                }
                None
            }
            State::Csi { mut params, len, overflow } => match byte {
                // Parameter and intermediate bytes.
                0x20..=0x3f => {
                    let (len, overflow) = if usize::from(len) < MAX_PARAMS {
                        params[usize::from(len)] = byte;
                        (len + 1, overflow)
                    } else {
                        (len, true)
                    };
                    self.state = State::Csi { params, len, overflow };
                    None
                }
                // The final byte.
                0x40..=0x7e => {
                    self.state = State::Ground;
                    if overflow { None } else { csi(&params[..usize::from(len)], byte) }
                }
                // Not part of a control sequence: give up on it.
                _ => {
                    self.state = State::Ground;
                    self.ground(byte, false)
                }
            },
            State::Ss3 => {
                self.state = State::Ground;
                match byte {
                    b'A' => Some(Edit::HistoryPrev),
                    b'B' => Some(Edit::HistoryNext),
                    b'C' => Some(Edit::Right),
                    b'D' => Some(Edit::Left),
                    b'H' => Some(Edit::Home),
                    b'F' => Some(Edit::End),
                    _ => None,
                }
            }
        }
    }

    fn ground(&mut self, byte: u8, after_cr: bool) -> Option<Edit> {
        match byte {
            b'\r' => {
                self.after_cr = true;
                Some(Edit::Enter)
            }
            b'\n' if after_cr => None,
            b'\n' => Some(Edit::Enter),
            0x1b => {
                self.state = State::Escape;
                None
            }
            0x7f | 0x08 => Some(Edit::Backspace),
            0x01..=0x1a => Edit::control(char::from(b'a' + byte - 1)),
            0x20..=0x7e => Some(Edit::Insert(char::from(byte))),
            // Lead bytes of 2-, 3- and 4-byte sequences (RFC 3629): C0, C1
            // and F5-FF never appear in UTF-8.
            0xc2..=0xdf => self.start_utf8(byte & 0x1f, 1, 0x80),
            0xe0..=0xef => self.start_utf8(byte & 0x0f, 2, 0x800),
            0xf0..=0xf4 => self.start_utf8(byte & 0x07, 3, 0x1_0000),
            _ => None,
        }
    }

    fn start_utf8(&mut self, bits: u8, missing: u8, min: u32) -> Option<Edit> {
        self.state = State::Utf8 { value: u32::from(bits), missing, min };
        None
    }
}

/// The edit for a complete control sequence `ESC [ <params> <last>`.
fn csi(params: &[u8], last: u8) -> Option<Edit> {
    // A modifier parameter (`1;5` for Ctrl, `1;3` for Alt) turns the
    // horizontal arrows into word movement.
    let modified = matches!(params, [b'1', b';', b'3' | b'5'] | [b'3' | b'5']);
    match (params, last) {
        (_, b'A') => Some(Edit::HistoryPrev),
        (_, b'B') => Some(Edit::HistoryNext),
        (_, b'C') => Some(if modified { Edit::WordRight } else { Edit::Right }),
        (_, b'D') => Some(if modified { Edit::WordLeft } else { Edit::Left }),
        ([], b'H') => Some(Edit::Home),
        ([], b'F') => Some(Edit::End),
        ([b'1' | b'7'], b'~') => Some(Edit::Home),
        ([b'4' | b'8'], b'~') => Some(Edit::End),
        ([b'3'], b'~') => Some(Edit::Delete),
        _ => None,
    }
}
