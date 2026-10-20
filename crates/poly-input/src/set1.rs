//! PS/2 scancode set 1: what an i8042 controller delivers with translation
//! on (QEMU's default, and what the kernel configures).
//!
//! Source: Microsoft, "Keyboard Scan Code Specification", revision 1.3a
//! (2000), Appendix C (scan code tables, set 1 column).
//!
//! A key's make code is one byte, or 0xE0 and one byte for the keys added by
//! the 101-key keyboard (right Ctrl and Alt, the editing block, the arrows,
//! keypad Enter and /, the Windows keys). Its break code is the same with
//! bit 7 set. Some keys send more:
//!
//! - **"Fake shifts".** The editing block and the arrows wrap their codes in
//!   `E0 2A` / `E0 AA` (or `E0 36` / `E0 B6`...) depending on Num Lock and
//!   Shift, so that an old keyboard driver would treat them like keypad
//!   keys. They carry no information and are dropped.
//! - **Print Screen** sends `E0 2A E0 37` when pressed and `E0 B7 E0 AA`
//!   when released (the fake shift again), `E0 37` / `E0 B7` with Shift or
//!   Ctrl held, and `54` / `D4` (SysRq) with Alt held.
//! - **Pause** sends `E1 1D 45 E1 9D C5` when pressed and nothing when
//!   released; with Ctrl held (Break), `E0 46 E0 C6`. Each half of the
//!   sequence is decoded as an event, so Pause reports a press and an
//!   immediate release.
//!
//! [`Set1Decoder`] turns bytes into [`ScanEvent`]s and never panics: a byte
//! that does not fit the sequence it is in is dropped along with the
//! sequence, and unknown codes (multimedia keys, keyboard replies such as
//! 0xFA) are ignored. [`encode`] is the reverse, for tests and for code that
//! injects key presses.

use crate::Key;

/// A key went down (`pressed`) or came up, as the scancodes say. Turned
/// into a [`KeyEvent`](crate::KeyEvent) by [`KeyState`](crate::KeyState).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScanEvent {
    pub key: Key,
    pub pressed: bool,
}

/// Prefix of the extended (101-key) codes.
pub const EXTENDED: u8 = 0xe0;
/// Prefix of the Pause sequence.
pub const PAUSE_PREFIX: u8 = 0xe1;
/// Set in a break code.
pub const BREAK: u8 = 0x80;

/// Codes without a prefix (Appendix C).
const PLAIN: &[(u8, Key)] = &[
    (0x01, Key::Escape),
    (0x02, Key::Digit1),
    (0x03, Key::Digit2),
    (0x04, Key::Digit3),
    (0x05, Key::Digit4),
    (0x06, Key::Digit5),
    (0x07, Key::Digit6),
    (0x08, Key::Digit7),
    (0x09, Key::Digit8),
    (0x0a, Key::Digit9),
    (0x0b, Key::Digit0),
    (0x0c, Key::Minus),
    (0x0d, Key::Equal),
    (0x0e, Key::Backspace),
    (0x0f, Key::Tab),
    (0x10, Key::Q),
    (0x11, Key::W),
    (0x12, Key::E),
    (0x13, Key::R),
    (0x14, Key::T),
    (0x15, Key::Y),
    (0x16, Key::U),
    (0x17, Key::I),
    (0x18, Key::O),
    (0x19, Key::P),
    (0x1a, Key::BracketLeft),
    (0x1b, Key::BracketRight),
    (0x1c, Key::Enter),
    (0x1d, Key::ControlLeft),
    (0x1e, Key::A),
    (0x1f, Key::S),
    (0x20, Key::D),
    (0x21, Key::F),
    (0x22, Key::G),
    (0x23, Key::H),
    (0x24, Key::J),
    (0x25, Key::K),
    (0x26, Key::L),
    (0x27, Key::Semicolon),
    (0x28, Key::Quote),
    (0x29, Key::Backquote),
    (0x2a, Key::ShiftLeft),
    (0x2b, Key::Backslash),
    (0x2c, Key::Z),
    (0x2d, Key::X),
    (0x2e, Key::C),
    (0x2f, Key::V),
    (0x30, Key::B),
    (0x31, Key::N),
    (0x32, Key::M),
    (0x33, Key::Comma),
    (0x34, Key::Period),
    (0x35, Key::Slash),
    (0x36, Key::ShiftRight),
    (0x37, Key::NumpadMultiply),
    (0x38, Key::AltLeft),
    (0x39, Key::Space),
    (0x3a, Key::CapsLock),
    (0x3b, Key::F1),
    (0x3c, Key::F2),
    (0x3d, Key::F3),
    (0x3e, Key::F4),
    (0x3f, Key::F5),
    (0x40, Key::F6),
    (0x41, Key::F7),
    (0x42, Key::F8),
    (0x43, Key::F9),
    (0x44, Key::F10),
    (0x45, Key::NumLock),
    (0x46, Key::ScrollLock),
    (0x47, Key::Numpad7),
    (0x48, Key::Numpad8),
    (0x49, Key::Numpad9),
    (0x4a, Key::NumpadSubtract),
    (0x4b, Key::Numpad4),
    (0x4c, Key::Numpad5),
    (0x4d, Key::Numpad6),
    (0x4e, Key::NumpadAdd),
    (0x4f, Key::Numpad1),
    (0x50, Key::Numpad2),
    (0x51, Key::Numpad3),
    (0x52, Key::Numpad0),
    (0x53, Key::NumpadDecimal),
    // SysRq: Print Screen with Alt held.
    (0x54, Key::PrintScreen),
    (0x56, Key::IntlBackslash),
    (0x57, Key::F11),
    (0x58, Key::F12),
];

/// Codes after 0xE0 (Appendix C).
const EXTENDED_CODES: &[(u8, Key)] = &[
    (0x1c, Key::NumpadEnter),
    (0x1d, Key::ControlRight),
    (0x35, Key::NumpadDivide),
    (0x37, Key::PrintScreen),
    (0x38, Key::AltRight),
    // Break: Pause with Ctrl held.
    (0x46, Key::Pause),
    (0x47, Key::Home),
    (0x48, Key::ArrowUp),
    (0x49, Key::PageUp),
    (0x4b, Key::ArrowLeft),
    (0x4d, Key::ArrowRight),
    (0x4f, Key::End),
    (0x50, Key::ArrowDown),
    (0x51, Key::PageDown),
    (0x52, Key::Insert),
    (0x53, Key::Delete),
    (0x5b, Key::MetaLeft),
    (0x5c, Key::MetaRight),
    (0x5d, Key::ContextMenu),
];

/// The fake shifts' make codes after 0xE0 (left and right Shift); their
/// breaks have bit 7 set.
const FAKE_SHIFTS: [u8; 2] = [0x2a, 0x36];

/// Make code (bits 6:0) to key.
type CodeTable = [Option<Key>; 128];

const fn table(entries: &[(u8, Key)]) -> CodeTable {
    let mut t = [None; 128];
    let mut i = 0;
    while i < entries.len() {
        let (code, key) = entries[i];
        assert!(code < BREAK && t[code as usize].is_none(), "bad or duplicate scancode");
        t[code as usize] = Some(key);
        i += 1;
    }
    t
}

static PLAIN_TABLE: CodeTable = table(PLAIN);
static EXTENDED_TABLE: CodeTable = table(EXTENDED_CODES);

/// Where the decoder is within a multi-byte sequence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Idle,
    /// After 0xE0.
    Extended,
    /// After 0xE1: expect 1D (Pause pressed) or 9D (released).
    Pause,
    /// After E1 1D or E1 9D: expect 45 or C5 to match.
    PauseSecond { pressed: bool },
}

/// Decodes a stream of set 1 scancode bytes into key presses and releases.
#[derive(Clone, Debug, Default)]
pub struct Set1Decoder {
    state: State,
}

impl Set1Decoder {
    pub const fn new() -> Set1Decoder {
        Set1Decoder { state: State::Idle }
    }

    /// Forget a partly received sequence.
    pub fn reset(&mut self) {
        self.state = State::Idle;
    }

    /// Whether the decoder is in the middle of a multi-byte sequence.
    pub fn in_sequence(&self) -> bool {
        self.state != State::Idle
    }

    /// Feed one byte; returns the key event it completes, if any.
    pub fn feed(&mut self, byte: u8) -> Option<ScanEvent> {
        let pressed = byte & BREAK == 0;
        let code = byte & !BREAK;
        match self.state {
            State::Idle | State::Extended if byte == EXTENDED => {
                self.state = State::Extended;
                None
            }
            State::Idle | State::Extended if byte == PAUSE_PREFIX => {
                self.state = State::Pause;
                None
            }
            State::Idle => PLAIN_TABLE[usize::from(code)].map(|key| ScanEvent { key, pressed }),
            State::Extended => {
                self.state = State::Idle;
                if FAKE_SHIFTS.contains(&code) {
                    return None;
                }
                EXTENDED_TABLE[usize::from(code)].map(|key| ScanEvent { key, pressed })
            }
            State::Pause => {
                if code == 0x1d {
                    self.state = State::PauseSecond { pressed };
                    None
                } else {
                    // Not a Pause sequence after all: drop the prefix and
                    // take the byte on its own.
                    self.state = State::Idle;
                    self.feed(byte)
                }
            }
            State::PauseSecond { pressed: first } => {
                self.state = State::Idle;
                if code == 0x45 && pressed == first {
                    Some(ScanEvent { key: Key::Pause, pressed })
                } else {
                    self.feed(byte)
                }
            }
        }
    }
}

/// The bytes a keyboard sends for one key event, at most six.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sequence {
    bytes: [u8; 6],
    len: u8,
}

impl Sequence {
    const fn new(bytes: &[u8]) -> Sequence {
        let mut s = Sequence { bytes: [0; 6], len: bytes.len() as u8 };
        let mut i = 0;
        while i < bytes.len() {
            s.bytes[i] = bytes[i];
            i += 1;
        }
        s
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }
}

impl core::ops::Deref for Sequence {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl core::fmt::Debug for Sequence {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:02x?}", self.as_slice())
    }
}

/// The set 1 bytes a keyboard sends when `key` is pressed or released, with
/// no other key held: Print Screen with its fake shifts, Pause as the
/// first or second half of its sequence.
pub fn encode(key: Key, pressed: bool) -> Sequence {
    let brk = if pressed { 0 } else { BREAK };
    match key {
        Key::PrintScreen if pressed => Sequence::new(&[EXTENDED, 0x2a, EXTENDED, 0x37]),
        Key::PrintScreen => Sequence::new(&[EXTENDED, 0xb7, EXTENDED, 0xaa]),
        Key::Pause => Sequence::new(&[PAUSE_PREFIX, 0x1d | brk, 0x45 | brk]),
        _ => {
            if let Some(&(code, _)) = PLAIN.iter().find(|(_, k)| *k == key) {
                Sequence::new(&[code | brk])
            } else if let Some(&(code, _)) = EXTENDED_CODES.iter().find(|(_, k)| *k == key) {
                Sequence::new(&[EXTENDED, code | brk])
            } else {
                unreachable!("every key except Print Screen and Pause has a code in the tables")
            }
        }
    }
}

#[cfg(test)]
mod tests;
