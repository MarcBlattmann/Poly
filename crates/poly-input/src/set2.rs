//! PS/2 scancode set 2: what a PS/2 keyboard sends when the controller
//! does not translate (and every keyboard's default after a reset).
//!
//! Source: Microsoft, "Keyboard Scan Code Specification", revision 1.3a
//! (2000), Appendix C (scan code tables, set 2 column).
//!
//! A key's make code is one byte, or 0xE0 and one byte for the keys of the
//! 101-key keyboard; its break code is the make code preceded by 0xF0
//! (0xE0 0xF0 xx for the extended keys). As in set 1, the editing block
//! and arrows wrap their codes in "fake shifts" (`E0 12` / `E0 59` and
//! their breaks), which are dropped; Print Screen sends `E0 12 E0 7C`
//! (`84` as SysRq with Alt held), and Pause sends `E1 14 77 E1 F0 14 F0 77`
//! when pressed and nothing when released (`E0 7E E0 F0 7E` as Break with
//! Ctrl held). As for set 1, each half of the Pause sequence is decoded as
//! an event, so Pause reports a press and an immediate release.
//!
//! [`Set2Decoder`] turns bytes into [`ScanEvent`]s like
//! [`Set1Decoder`](crate::Set1Decoder): it never panics, drops a byte that
//! breaks the sequence it is in (with the sequence), and ignores unknown
//! codes and keyboard replies (0xFA, 0xAA, 0xEE, 0xFE, 0x00, 0xFF).

use crate::Key;
use crate::set1::ScanEvent;

/// Prefix of the extended (101-key) codes.
pub const EXTENDED: u8 = 0xe0;
/// Prefix of the Pause sequence.
pub const PAUSE_PREFIX: u8 = 0xe1;
/// Precedes a break code.
pub const BREAK: u8 = 0xf0;

/// Codes without a prefix (Appendix C).
const PLAIN: &[(u8, Key)] = &[
    (0x01, Key::F9),
    (0x03, Key::F5),
    (0x04, Key::F3),
    (0x05, Key::F1),
    (0x06, Key::F2),
    (0x07, Key::F12),
    (0x09, Key::F10),
    (0x0a, Key::F8),
    (0x0b, Key::F6),
    (0x0c, Key::F4),
    (0x0d, Key::Tab),
    (0x0e, Key::Backquote),
    (0x11, Key::AltLeft),
    (0x12, Key::ShiftLeft),
    (0x14, Key::ControlLeft),
    (0x15, Key::Q),
    (0x16, Key::Digit1),
    (0x1a, Key::Z),
    (0x1b, Key::S),
    (0x1c, Key::A),
    (0x1d, Key::W),
    (0x1e, Key::Digit2),
    (0x21, Key::C),
    (0x22, Key::X),
    (0x23, Key::D),
    (0x24, Key::E),
    (0x25, Key::Digit4),
    (0x26, Key::Digit3),
    (0x29, Key::Space),
    (0x2a, Key::V),
    (0x2b, Key::F),
    (0x2c, Key::T),
    (0x2d, Key::R),
    (0x2e, Key::Digit5),
    (0x31, Key::N),
    (0x32, Key::B),
    (0x33, Key::H),
    (0x34, Key::G),
    (0x35, Key::Y),
    (0x36, Key::Digit6),
    (0x3a, Key::M),
    (0x3b, Key::J),
    (0x3c, Key::U),
    (0x3d, Key::Digit7),
    (0x3e, Key::Digit8),
    (0x41, Key::Comma),
    (0x42, Key::K),
    (0x43, Key::I),
    (0x44, Key::O),
    (0x45, Key::Digit0),
    (0x46, Key::Digit9),
    (0x49, Key::Period),
    (0x4a, Key::Slash),
    (0x4b, Key::L),
    (0x4c, Key::Semicolon),
    (0x4d, Key::P),
    (0x4e, Key::Minus),
    (0x52, Key::Quote),
    (0x54, Key::BracketLeft),
    (0x55, Key::Equal),
    (0x58, Key::CapsLock),
    (0x59, Key::ShiftRight),
    (0x5a, Key::Enter),
    (0x5b, Key::BracketRight),
    // US \ and |; on ISO keyboards the key left of Enter ("Europe 1").
    (0x5d, Key::Backslash),
    // The ISO key next to the left Shift ("Europe 2").
    (0x61, Key::IntlBackslash),
    (0x66, Key::Backspace),
    (0x69, Key::Numpad1),
    (0x6b, Key::Numpad4),
    (0x6c, Key::Numpad7),
    (0x70, Key::Numpad0),
    (0x71, Key::NumpadDecimal),
    (0x72, Key::Numpad2),
    (0x73, Key::Numpad5),
    (0x74, Key::Numpad6),
    (0x75, Key::Numpad8),
    (0x76, Key::Escape),
    (0x77, Key::NumLock),
    (0x78, Key::F11),
    (0x79, Key::NumpadAdd),
    (0x7a, Key::Numpad3),
    (0x7b, Key::NumpadSubtract),
    (0x7c, Key::NumpadMultiply),
    (0x7d, Key::Numpad9),
    (0x7e, Key::ScrollLock),
    (0x83, Key::F7),
    // SysRq: Print Screen with Alt held.
    (0x84, Key::PrintScreen),
];

/// Codes after 0xE0 (Appendix C).
const EXTENDED_CODES: &[(u8, Key)] = &[
    (0x11, Key::AltRight),
    (0x14, Key::ControlRight),
    (0x1f, Key::MetaLeft),
    (0x27, Key::MetaRight),
    (0x2f, Key::ContextMenu),
    (0x4a, Key::NumpadDivide),
    (0x5a, Key::NumpadEnter),
    (0x69, Key::End),
    (0x6b, Key::ArrowLeft),
    (0x6c, Key::Home),
    (0x70, Key::Insert),
    (0x71, Key::Delete),
    (0x72, Key::ArrowDown),
    (0x74, Key::ArrowRight),
    (0x75, Key::ArrowUp),
    (0x7a, Key::PageDown),
    (0x7c, Key::PrintScreen),
    (0x7d, Key::PageUp),
    // Break: Pause with Ctrl held.
    (0x7e, Key::Pause),
];

/// The fake shifts' codes after 0xE0 (left and right Shift).
const FAKE_SHIFTS: [u8; 2] = [0x12, 0x59];

/// The rest of the Pause sequence after its first 0xE1: the press half,
/// and the release half.
const PAUSE_PRESS: [u8; 2] = [0x14, 0x77];
const PAUSE_RELEASE: [u8; 4] = [0xf0, 0x14, 0xf0, 0x77];

type CodeTable = [Option<Key>; 256];

const fn table(entries: &[(u8, Key)]) -> CodeTable {
    let mut t = [None; 256];
    let mut i = 0;
    while i < entries.len() {
        let (code, key) = entries[i];
        assert!(t[code as usize].is_none(), "duplicate scancode");
        t[code as usize] = Some(key);
        i += 1;
    }
    t
}

static PLAIN_TABLE: CodeTable = table(PLAIN);
static EXTENDED_TABLE: CodeTable = table(EXTENDED_CODES);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Idle,
    /// After 0xF0.
    Break,
    /// After 0xE0.
    Extended,
    /// After 0xE0 0xF0.
    ExtendedBreak,
    /// After 0xE1: expect 0x14 (the press half) or 0xF0 (the release half).
    Pause,
    /// Inside the press (`pressed`) or release half, `at` bytes matched.
    PauseRest { pressed: bool, at: u8 },
}

/// Decodes a stream of set 2 scancode bytes into key presses and releases.
#[derive(Clone, Debug, Default)]
pub struct Set2Decoder {
    state: State,
}

impl Set2Decoder {
    pub const fn new() -> Set2Decoder {
        Set2Decoder { state: State::Idle }
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
        match self.state {
            State::Idle => match byte {
                EXTENDED => {
                    self.state = State::Extended;
                    None
                }
                BREAK => {
                    self.state = State::Break;
                    None
                }
                PAUSE_PREFIX => {
                    self.state = State::Pause;
                    None
                }
                _ => PLAIN_TABLE[usize::from(byte)].map(|key| ScanEvent { key, pressed: true }),
            },
            State::Break => self.code(byte, &PLAIN_TABLE, false, false),
            State::Extended if byte == BREAK => {
                self.state = State::ExtendedBreak;
                None
            }
            State::Extended => self.code(byte, &EXTENDED_TABLE, true, true),
            State::ExtendedBreak => self.code(byte, &EXTENDED_TABLE, true, false),
            State::Pause => match byte {
                0x14 => {
                    self.state = State::PauseRest { pressed: true, at: 1 };
                    None
                }
                BREAK => {
                    self.state = State::PauseRest { pressed: false, at: 1 };
                    None
                }
                _ => self.restart(byte),
            },
            State::PauseRest { pressed, at } => {
                let rest: &[u8] = if pressed { &PAUSE_PRESS } else { &PAUSE_RELEASE };
                if rest.get(usize::from(at)) != Some(&byte) {
                    return self.restart(byte);
                }
                if usize::from(at) + 1 == rest.len() {
                    self.state = State::Idle;
                    Some(ScanEvent { key: Key::Pause, pressed })
                } else {
                    self.state = State::PauseRest { pressed, at: at + 1 };
                    None
                }
            }
        }
    }

    /// The code byte of a sequence: the key it names, unless it is a fake
    /// shift or unknown. A prefix in its place starts a new sequence.
    fn code(&mut self, byte: u8, table: &CodeTable, extended: bool, pressed: bool) -> Option<ScanEvent> {
        if matches!(byte, EXTENDED | BREAK | PAUSE_PREFIX) {
            return self.restart(byte);
        }
        self.state = State::Idle;
        if extended && FAKE_SHIFTS.contains(&byte) {
            return None;
        }
        table[usize::from(byte)].map(|key| ScanEvent { key, pressed })
    }

    /// `byte` broke the sequence: drop it and take the byte on its own.
    fn restart(&mut self, byte: u8) -> Option<ScanEvent> {
        self.state = State::Idle;
        self.feed(byte)
    }
}

/// The set 2 bytes for one key event, at most eight.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sequence {
    bytes: [u8; 8],
    len: u8,
}

impl Sequence {
    fn new(bytes: &[u8]) -> Sequence {
        let mut s = Sequence { bytes: [0; 8], len: bytes.len() as u8 };
        s.bytes[..bytes.len()].copy_from_slice(bytes);
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

/// The set 2 bytes a keyboard sends when `key` is pressed or released, with
/// no other key held: Print Screen with its fake shifts, Pause as the
/// first or second half of its sequence.
pub fn encode(key: Key, pressed: bool) -> Sequence {
    match (key, pressed) {
        (Key::PrintScreen, true) => Sequence::new(&[EXTENDED, 0x12, EXTENDED, 0x7c]),
        (Key::PrintScreen, false) => Sequence::new(&[EXTENDED, BREAK, 0x7c, EXTENDED, BREAK, 0x12]),
        (Key::Pause, true) => Sequence::new(&[PAUSE_PREFIX, 0x14, 0x77]),
        (Key::Pause, false) => Sequence::new(&[PAUSE_PREFIX, BREAK, 0x14, BREAK, 0x77]),
        _ => {
            if let Some(&(code, _)) = PLAIN.iter().find(|(_, k)| *k == key) {
                if pressed { Sequence::new(&[code]) } else { Sequence::new(&[BREAK, code]) }
            } else if let Some(&(code, _)) = EXTENDED_CODES.iter().find(|(_, k)| *k == key) {
                if pressed { Sequence::new(&[EXTENDED, code]) } else { Sequence::new(&[EXTENDED, BREAK, code]) }
            } else {
                unreachable!("every key except Print Screen and Pause has a code in the tables")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    fn decode(bytes: &[u8]) -> Vec<ScanEvent> {
        let mut d = Set2Decoder::new();
        bytes.iter().filter_map(|&b| d.feed(b)).collect()
    }

    fn ev(key: Key, pressed: bool) -> ScanEvent {
        ScanEvent { key, pressed }
    }

    #[test]
    fn every_key_round_trips() {
        for key in Key::ALL {
            for pressed in [true, false] {
                let bytes = encode(key, pressed);
                assert_eq!(decode(&bytes), [ev(key, pressed)], "{key:?} {pressed}: {bytes:?}");
            }
        }
    }

    #[test]
    fn both_tables_name_every_key_once() {
        let mut seen = [0u8; Key::COUNT];
        for &(_, k) in PLAIN.iter().chain(EXTENDED_CODES) {
            seen[k as usize] += 1;
        }
        for key in Key::ALL {
            // Print Screen (E0 7C, and 84 as SysRq) and Pause (E1 ...,
            // and E0 7E as Break) have two codes each.
            let want = if matches!(key, Key::PrintScreen | Key::Pause) { 1..=2 } else { 1..=1 };
            assert!(want.contains(&seen[key as usize]), "{key:?} listed {} times", seen[key as usize]);
        }
    }

    #[test]
    fn known_codes() {
        assert_eq!(decode(&[0x1c, 0xf0, 0x1c]), [ev(Key::A, true), ev(Key::A, false)]);
        assert_eq!(decode(&[0x83]), [ev(Key::F7, true)]);
        assert_eq!(decode(&[0xe0, 0x75, 0xe0, 0xf0, 0x75]), [ev(Key::ArrowUp, true), ev(Key::ArrowUp, false)]);
        assert_eq!(decode(&[0x61]), [ev(Key::IntlBackslash, true)]);
        assert_eq!(decode(&[0xe0, 0x11]), [ev(Key::AltRight, true)]);
    }

    #[test]
    fn fake_shifts_replies_and_unknown_codes_are_dropped() {
        // Insert with Num Lock on: E0 12 E0 70 ... E0 F0 70 E0 F0 12.
        let bytes = [0xe0, 0x12, 0xe0, 0x70, 0xe0, 0xf0, 0x70, 0xe0, 0xf0, 0x12];
        assert_eq!(decode(&bytes), [ev(Key::Insert, true), ev(Key::Insert, false)]);
        assert_eq!(decode(&[0xfa, 0xaa, 0xee, 0xfe, 0x00, 0xff, 0x02, 0xe0, 0x02]), []);
        // Ctrl + Pause is Break: E0 7E E0 F0 7E.
        assert_eq!(decode(&[0xe0, 0x7e, 0xe0, 0xf0, 0x7e]), [ev(Key::Pause, true), ev(Key::Pause, false)]);
        // Alt + Print Screen is SysRq: 84, F0 84.
        assert_eq!(decode(&[0x84, 0xf0, 0x84]), [ev(Key::PrintScreen, true), ev(Key::PrintScreen, false)]);
    }

    #[test]
    fn the_whole_pause_sequence_is_a_press_and_a_release() {
        let bytes = [0xe1, 0x14, 0x77, 0xe1, 0xf0, 0x14, 0xf0, 0x77];
        assert_eq!(decode(&bytes), [ev(Key::Pause, true), ev(Key::Pause, false)]);
    }

    #[test]
    fn broken_sequences_resynchronise() {
        // A prefix where a code should be starts over.
        assert_eq!(decode(&[0xe0, 0xe0, 0x75]), [ev(Key::ArrowUp, true)]);
        assert_eq!(decode(&[0xf0, 0xe0, 0x75]), [ev(Key::ArrowUp, true)]);
        // A broken Pause sequence: the byte that broke it counts on its own.
        assert_eq!(decode(&[0xe1, 0x14, 0x1c]), [ev(Key::A, true)]);
        assert_eq!(decode(&[0xe1, 0x1c]), [ev(Key::A, true)]);
        let mut d = Set2Decoder::new();
        assert_eq!(d.feed(0xe0), None);
        assert!(d.in_sequence());
        d.reset();
        assert!(!d.in_sequence());
        assert_eq!(d.feed(0x75), Some(ev(Key::Numpad8, true)));
        // Any byte stream is accepted.
        let mut d = Set2Decoder::new();
        for b in 0..=255u8 {
            for c in 0..=255u8 {
                d.feed(b);
                d.feed(c);
            }
        }
    }
}
