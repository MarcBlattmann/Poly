//! Console input: `INPUT_RECORD`s (Learn: "INPUT_RECORD structure",
//! "KEY_EVENT_RECORD structure") and the bytes of a terminal decoded into
//! key events.
//!
//! A console on Poly reads a terminal in raw mode, so what arrives are the
//! bytes a VT terminal sends: characters (UTF-8), control characters, and
//! ECMA-48 sequences for the cursor and function keys. [`TermDecoder`]
//! turns them into the key events a Windows console reports:
//!
//! | Bytes | Key | Character | State |
//! |---|---|---|---|
//! | printable | the letter's or digit's `VK_*`, else 0 | the character | `SHIFT_PRESSED` for capitals |
//! | CR | `VK_RETURN` | `\r` | |
//! | LF | `VK_RETURN` | `\n` | `LEFT_CTRL_PRESSED` (Ctrl+Enter's character) |
//! | DEL, BS | `VK_BACK` | `\b` | |
//! | TAB | `VK_TAB` | `\t` | |
//! | `^A`–`^Z` | `'A'`–`'Z'` | 0x01–0x1A | `LEFT_CTRL_PRESSED` |
//! | ESC alone | `VK_ESCAPE` | 0x1B | |
//! | ESC + character | that key | the character | `LEFT_ALT_PRESSED` |
//! | `ESC [ A`/`B`/`C`/`D` | `VK_UP`/`DOWN`/`RIGHT`/`LEFT` | 0 | `ENHANCED_KEY` |
//! | `ESC [ H`/`F`, `ESC [ 1~`/`4~` | `VK_HOME`/`VK_END` | 0 | `ENHANCED_KEY` |
//! | `ESC [ 2~`/`3~`/`5~`/`6~` | `VK_INSERT`/`DELETE`/`PRIOR`/`NEXT` | 0 | `ENHANCED_KEY` |
//! | `ESC O P`–`S`, `ESC [ 15~`... | `VK_F1`... | 0 | |
//!
//! Each key gives a key-down and a key-up record (a terminal does not
//! report releases; Poly makes one so programs that wait for the release
//! see it). Scan codes are 0. A character outside the BMP is two records
//! pairs, one per surrogate. All of this is Poly's mapping, to be confirmed
//! by the spec agent where Windows Terminal's own input mapping differs.

use crate::consts::*;
use alloc::vec::Vec;

/// `KEY_EVENT_RECORD`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyEvent {
    pub down: bool,
    pub repeat: u16,
    pub vk: u16,
    pub scan: u16,
    pub ch: u16,
    pub state: u32,
}

impl KeyEvent {
    /// A key press (repeat count 1).
    pub const fn press(vk: u16, ch: u16, state: u32) -> KeyEvent {
        KeyEvent { down: true, repeat: 1, vk, scan: 0, ch, state }
    }

    /// The same key released.
    pub fn released(self) -> KeyEvent {
        KeyEvent { down: false, ..self }
    }
}

/// One `INPUT_RECORD`. Only key events come from a terminal; the others
/// are kept as their raw 16 bytes (`WriteConsoleInputW` may put any).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputRecord {
    Key(KeyEvent),
    Other { kind: u16, raw: [u8; 16] },
}

impl InputRecord {
    /// The record's bytes (`INPUT_RECORD`, 20 bytes: `EventType` at 0, the
    /// event at 4; `KEY_EVENT_RECORD`: `bKeyDown` at 4, `wRepeatCount` at
    /// 8, `wVirtualKeyCode` at 10, `wVirtualScanCode` at 12, `UnicodeChar`
    /// at 14, `dwControlKeyState` at 16).
    pub fn to_bytes(&self) -> [u8; INPUT_RECORD_SIZE] {
        let mut b = [0u8; INPUT_RECORD_SIZE];
        match self {
            InputRecord::Key(k) => {
                b[0..2].copy_from_slice(&KEY_EVENT.to_le_bytes());
                b[4..8].copy_from_slice(&u32::from(k.down).to_le_bytes());
                b[8..10].copy_from_slice(&k.repeat.to_le_bytes());
                b[10..12].copy_from_slice(&k.vk.to_le_bytes());
                b[12..14].copy_from_slice(&k.scan.to_le_bytes());
                b[14..16].copy_from_slice(&k.ch.to_le_bytes());
                b[16..20].copy_from_slice(&k.state.to_le_bytes());
            }
            InputRecord::Other { kind, raw } => {
                b[0..2].copy_from_slice(&kind.to_le_bytes());
                b[4..20].copy_from_slice(raw);
            }
        }
        b
    }

    /// A record from its bytes.
    pub fn from_bytes(b: &[u8; INPUT_RECORD_SIZE]) -> InputRecord {
        let kind = u16::from_le_bytes([b[0], b[1]]);
        let h = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        if kind == KEY_EVENT {
            InputRecord::Key(KeyEvent {
                down: u32::from_le_bytes([b[4], b[5], b[6], b[7]]) != 0,
                repeat: h(8),
                vk: h(10),
                scan: h(12),
                ch: h(14),
                state: u32::from_le_bytes([b[16], b[17], b[18], b[19]]),
            })
        } else {
            let mut raw = [0u8; 16];
            raw.copy_from_slice(&b[4..20]);
            InputRecord::Other { kind, raw }
        }
    }

    pub fn key(&self) -> Option<&KeyEvent> {
        match self {
            InputRecord::Key(k) => Some(k),
            InputRecord::Other { .. } => None,
        }
    }
}

/// The key a printable character is typed with (US layout, Poly's choice):
/// its virtual-key code and shift state.
fn key_of_char(c: u16) -> (u16, u32) {
    match c {
        0x61..=0x7A => (c - 0x20, 0),
        0x41..=0x5A => (c, SHIFT_PRESSED),
        0x30..=0x39 => (c, 0),
        0x20 => (VK_SPACE, 0),
        _ => (0, 0),
    }
}

/// Terminal bytes → key events, keeping a sequence cut between two reads.
#[derive(Clone, Debug, Default)]
pub struct TermDecoder {
    pending: Vec<u8>,
    /// The last byte was a CR: an LF right after it is the same Enter.
    after_cr: bool,
}

/// One decoded key (a press; the caller adds the release).
fn press(out: &mut Vec<KeyEvent>, vk: u16, ch: u16, state: u32) {
    out.push(KeyEvent::press(vk, ch, state));
}

impl TermDecoder {
    /// Decode `bytes` into key presses. A lone ESC at the very end of
    /// `bytes` is the Escape key; an unfinished sequence or UTF-8 character
    /// after ESC `[`, ESC `O` or a lead byte waits for the next call.
    pub fn decode(&mut self, bytes: &[u8]) -> Vec<KeyEvent> {
        let mut data = core::mem::take(&mut self.pending);
        data.extend_from_slice(bytes);
        let mut out = Vec::new();
        let mut i = 0;
        while i < data.len() {
            let b = data[i];
            let after_cr = core::mem::replace(&mut self.after_cr, b == b'\r');
            match b {
                b'\r' => press(&mut out, VK_RETURN, u16::from(b'\r'), 0),
                b'\n' if after_cr => {}
                b'\n' => press(&mut out, VK_RETURN, u16::from(b'\n'), LEFT_CTRL_PRESSED),
                0x7F | 0x08 => press(&mut out, VK_BACK, 0x08, 0),
                b'\t' => press(&mut out, VK_TAB, u16::from(b'\t'), 0),
                0x00 => press(&mut out, VK_SPACE, 0, LEFT_CTRL_PRESSED),
                0x1B => match self.escape(&data[i + 1..], &mut out) {
                    Some(used) => i += used,
                    None => {
                        self.pending = data[i..].to_vec();
                        return out;
                    }
                },
                0x01..=0x1A => press(&mut out, u16::from(b'A' + b - 1), u16::from(b), LEFT_CTRL_PRESSED),
                0x1C..=0x1F => press(&mut out, 0, u16::from(b), LEFT_CTRL_PRESSED),
                0x20..=0x7E => {
                    let (vk, state) = key_of_char(u16::from(b));
                    press(&mut out, vk, u16::from(b), state);
                }
                _ => {
                    let need = match b {
                        0xC2..=0xDF => 2,
                        0xE0..=0xEF => 3,
                        0xF0..=0xF4 => 4,
                        _ => 0,
                    };
                    if need == 0 {
                        press(&mut out, 0, 0xFFFD, 0);
                    } else if i + need > data.len() && data[i + 1..].iter().all(|&c| c & 0xC0 == 0x80) {
                        self.pending = data[i..].to_vec();
                        return out;
                    } else if let Ok(s) = core::str::from_utf8(&data[i..(i + need).min(data.len())]) {
                        for c in s.chars() {
                            let mut buf = [0u16; 2];
                            for &u in c.encode_utf16(&mut buf).iter() {
                                press(&mut out, 0, u, 0);
                            }
                        }
                        i += need - 1;
                    } else {
                        press(&mut out, 0, 0xFFFD, 0);
                    }
                }
            }
            i += 1;
        }
        out
    }

    /// An escape: the bytes after ESC. Returns how many of them the key
    /// used, or `None` to wait for more.
    fn escape(&self, rest: &[u8], out: &mut Vec<KeyEvent>) -> Option<usize> {
        let Some(&first) = rest.first() else {
            press(out, VK_ESCAPE, 0x1B, 0);
            return Some(0);
        };
        match first {
            b'[' => {
                // CSI: parameters (digits and ';'), then a final byte.
                let mut j = 1;
                while j < rest.len() && (rest[j].is_ascii_digit() || rest[j] == b';') {
                    j += 1;
                }
                let &fin = rest.get(j)?;
                let params = &rest[1..j];
                let mut nums = params
                    .split(|&c| c == b';')
                    .map(|p| p.iter().fold(0u32, |acc, &d| acc.saturating_mul(10).saturating_add(u32::from(d - b'0'))));
                let first_num = nums.next().unwrap_or(0);
                let modifier = nums.next().unwrap_or(1);
                let state = ENHANCED_KEY | modifier_state(modifier);
                let vk = match fin {
                    b'A' => Some(VK_UP),
                    b'B' => Some(VK_DOWN),
                    b'C' => Some(VK_RIGHT),
                    b'D' => Some(VK_LEFT),
                    b'H' => Some(VK_HOME),
                    b'F' => Some(VK_END),
                    b'~' => match first_num {
                        1 | 7 => Some(VK_HOME),
                        2 => Some(VK_INSERT),
                        3 => Some(VK_DELETE),
                        4 | 8 => Some(VK_END),
                        5 => Some(VK_PRIOR),
                        6 => Some(VK_NEXT),
                        11..=15 => Some(VK_F1 + (first_num - 11) as u16),
                        17..=21 => Some(VK_F1 + 5 + (first_num - 17) as u16),
                        23 | 24 => Some(VK_F1 + 10 + (first_num - 23) as u16),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(vk) = vk {
                    let state = if (VK_F1..VK_F1 + 12).contains(&vk) { state & !ENHANCED_KEY } else { state };
                    press(out, vk, 0, state);
                }
                Some(j + 1)
            }
            b'O' => {
                let &fin = rest.get(1)?;
                let vk = match fin {
                    b'P'..=b'S' => Some((VK_F1 + u16::from(fin - b'P'), 0)),
                    b'H' => Some((VK_HOME, ENHANCED_KEY)),
                    b'F' => Some((VK_END, ENHANCED_KEY)),
                    b'A' => Some((VK_UP, ENHANCED_KEY)),
                    b'B' => Some((VK_DOWN, ENHANCED_KEY)),
                    b'C' => Some((VK_RIGHT, ENHANCED_KEY)),
                    b'D' => Some((VK_LEFT, ENHANCED_KEY)),
                    _ => None,
                };
                if let Some((vk, state)) = vk {
                    press(out, vk, 0, state);
                }
                Some(2)
            }
            0x20..=0x7E => {
                let (vk, state) = key_of_char(u16::from(first));
                press(out, vk, u16::from(first), state | LEFT_ALT_PRESSED);
                Some(1)
            }
            _ => {
                // ESC followed by a control: the Escape key, then that byte.
                press(out, VK_ESCAPE, 0x1B, 0);
                Some(0)
            }
        }
    }
}

/// The xterm modifier parameter (1 + shift 1, alt 2, ctrl 4) as control
/// key state.
fn modifier_state(m: u32) -> u32 {
    let bits = m.saturating_sub(1);
    let mut s = 0;
    if bits & 1 != 0 {
        s |= SHIFT_PRESSED;
    }
    if bits & 2 != 0 {
        s |= LEFT_ALT_PRESSED;
    }
    if bits & 4 != 0 {
        s |= LEFT_CTRL_PRESSED;
    }
    s
}

/// Key presses as the records a console stores: each press, then its
/// release.
pub fn records(keys: &[KeyEvent]) -> Vec<InputRecord> {
    keys.iter().flat_map(|&k| [InputRecord::Key(k), InputRecord::Key(k.released())]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(bytes: &[u8]) -> Vec<(u16, u16, u32)> {
        TermDecoder::default().decode(bytes).iter().map(|k| (k.vk, k.ch, k.state)).collect()
    }

    #[test]
    fn printable_and_controls() {
        assert_eq!(keys(b"aZ1 "), [(0x41, 0x61, 0), (0x5A, 0x5A, SHIFT_PRESSED), (0x31, 0x31, 0), (VK_SPACE, 0x20, 0)]);
        assert_eq!(keys(b"\r"), [(VK_RETURN, 13, 0)]);
        assert_eq!(keys(b"\r\n"), [(VK_RETURN, 13, 0)]);
        assert_eq!(keys(b"\n"), [(VK_RETURN, 10, LEFT_CTRL_PRESSED)]);
        assert_eq!(keys(b"\x7f\x08\t"), [(VK_BACK, 8, 0), (VK_BACK, 8, 0), (VK_TAB, 9, 0)]);
        assert_eq!(keys(b"\x03"), [(0x43, 3, LEFT_CTRL_PRESSED)]);
    }

    #[test]
    fn escape_sequences() {
        assert_eq!(keys(b"\x1b[A\x1b[D"), [(VK_UP, 0, ENHANCED_KEY), (VK_LEFT, 0, ENHANCED_KEY)]);
        assert_eq!(keys(b"\x1b[3~"), [(VK_DELETE, 0, ENHANCED_KEY)]);
        assert_eq!(keys(b"\x1b[1;5C"), [(VK_RIGHT, 0, ENHANCED_KEY | LEFT_CTRL_PRESSED)]);
        assert_eq!(keys(b"\x1bOP\x1b[24~"), [(VK_F1, 0, 0), (VK_F1 + 11, 0, 0)]);
        assert_eq!(keys(b"\x1b"), [(VK_ESCAPE, 0x1b, 0)]);
        assert_eq!(keys(b"\x1bx"), [(0x58, u16::from(b'x'), LEFT_ALT_PRESSED)]);
    }

    #[test]
    fn split_input_waits() {
        let mut d = TermDecoder::default();
        assert!(d.decode(b"\x1b[").is_empty());
        assert_eq!(d.decode(b"B").len(), 1);
        let s = "é😀".as_bytes();
        assert!(d.decode(&s[..1]).is_empty());
        let k = d.decode(&s[1..]);
        assert_eq!(k.iter().map(|k| k.ch).collect::<Vec<_>>(), [0xE9, 0xD83D, 0xDE00]);
    }

    #[test]
    fn records_round_trip() {
        let r = records(&[KeyEvent::press(0x41, 0x61, 0)]);
        assert_eq!(r.len(), 2);
        for rec in r {
            assert_eq!(InputRecord::from_bytes(&rec.to_bytes()), rec);
        }
        let b = InputRecord::Key(KeyEvent::press(VK_RETURN, 13, 0)).to_bytes();
        assert_eq!(&b[..2], &[1, 0]);
        assert_eq!(&b[4..8], &[1, 0, 0, 0]);
        assert_eq!(&b[10..12], &[13, 0]);
        assert_eq!(&b[14..16], &[13, 0]);
    }
}
