//! Console I/O (Learn: "Console and port I/O", `_getch`, `_kbhit`,
//! `_cgets`): which key events `_getch` returns and as what.
//!
//! `_getch` reads key presses from the console's input buffer without
//! echo. A press with a character gives that character; "When reading a
//! function key or an arrow key, each function must be called twice; the
//! first call returns 0 or 0xE0, and the second call returns the actual
//! key code" (Learn). Releases, modifier keys alone and other events give
//! nothing.
//!
//! The codes are the PC BIOS keyboard service's extended codes (IBM
//! Personal Computer AT Technical Reference, keyboard BIOS: F1–F10
//! 0x3B–0x44 and their Shift / Ctrl / Alt forms 0x54–0x71, F11 / F12
//! 0x85 / 0x86 and theirs up to 0x8C, the cursor keys 0x47–0x53 and their
//! Ctrl / Alt forms), keyed by the event's virtual-key code (`winuser.h`),
//! so a console that leaves the scan code 0 still gives them. The prefix
//! is 0xE0 for a key of the separate cursor block (`ENHANCED_KEY` set,
//! `wincontypes.h`) and 0 otherwise. Which modified keys have codes is
//! Poly's reading of those tables (to be confirmed by the spec agent).

/// `INPUT_RECORD.EventType` `KEY_EVENT` and the `KEY_EVENT_RECORD` offsets
/// (`wincontypes.h`): `bKeyDown` 4, `wRepeatCount` 8, `wVirtualKeyCode`
/// 10, `wVirtualScanCode` 12, `uChar` 14, `dwControlKeyState` 16; 20 bytes.
pub const KEY_EVENT: u16 = 1;
pub const RECORD_SIZE: usize = 20;

/// `dwControlKeyState` bits (`wincontypes.h`).
pub const RIGHT_ALT_PRESSED: u32 = 0x1;
pub const LEFT_ALT_PRESSED: u32 = 0x2;
pub const RIGHT_CTRL_PRESSED: u32 = 0x4;
pub const LEFT_CTRL_PRESSED: u32 = 0x8;
pub const SHIFT_PRESSED: u32 = 0x10;
pub const ENHANCED_KEY: u32 = 0x100;

/// Console input modes (`consoleapi.h`).
pub const ENABLE_PROCESSED_INPUT: u32 = 0x1;
pub const ENABLE_LINE_INPUT: u32 = 0x2;
pub const ENABLE_ECHO_INPUT: u32 = 0x4;

/// What one input record gives `_getch`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A character (a UTF-16 unit).
    Char(u16),
    /// A function or cursor key: the prefix (0 or 0xE0), then the code.
    Extended(u8, u8),
}

/// A key event of an `INPUT_RECORD`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyEvent {
    pub down: bool,
    pub repeat: u16,
    pub vk: u16,
    pub scan: u16,
    pub ch: u16,
    pub state: u32,
}

/// The key event of a raw `INPUT_RECORD`, if it is one.
pub fn key_event(rec: &[u8; RECORD_SIZE]) -> Option<KeyEvent> {
    let u16_at = |o: usize| u16::from_le_bytes([rec[o], rec[o + 1]]);
    if u16_at(0) != KEY_EVENT {
        return None;
    }
    Some(KeyEvent {
        down: u32::from_le_bytes([rec[4], rec[5], rec[6], rec[7]]) != 0,
        repeat: u16_at(8),
        vk: u16_at(10),
        scan: u16_at(12),
        ch: u16_at(14),
        state: u32::from_le_bytes([rec[16], rec[17], rec[18], rec[19]]),
    })
}

/// A raw `INPUT_RECORD` holding a key event (tests, `_ungetch` has none).
pub fn key_record(k: &KeyEvent) -> [u8; RECORD_SIZE] {
    let mut r = [0u8; RECORD_SIZE];
    r[0..2].copy_from_slice(&KEY_EVENT.to_le_bytes());
    r[4..8].copy_from_slice(&u32::from(k.down).to_le_bytes());
    r[8..10].copy_from_slice(&k.repeat.to_le_bytes());
    r[10..12].copy_from_slice(&k.vk.to_le_bytes());
    r[12..14].copy_from_slice(&k.scan.to_le_bytes());
    r[14..16].copy_from_slice(&k.ch.to_le_bytes());
    r[16..20].copy_from_slice(&k.state.to_le_bytes());
    r
}

/// `VK_F1` (`winuser.h`); F1–F12 follow it.
const VK_F1: u16 = 0x70;

/// The cursor keys: virtual-key code (`winuser.h`), then the BIOS code
/// alone, with Ctrl, with Alt.
const CURSOR: [(u16, u8, u8, u8); 10] = [
    (0x24, 0x47, 0x77, 0x97), // VK_HOME
    (0x26, 0x48, 0x8D, 0x98), // VK_UP
    (0x21, 0x49, 0x84, 0x99), // VK_PRIOR
    (0x25, 0x4B, 0x73, 0x9B), // VK_LEFT
    (0x27, 0x4D, 0x74, 0x9D), // VK_RIGHT
    (0x23, 0x4F, 0x75, 0x9F), // VK_END
    (0x28, 0x50, 0x91, 0xA0), // VK_DOWN
    (0x22, 0x51, 0x76, 0xA1), // VK_NEXT
    (0x2D, 0x52, 0x92, 0xA2), // VK_INSERT
    (0x2E, 0x53, 0x93, 0xA3), // VK_DELETE
];

/// The BIOS code of a function or cursor key with its modifiers.
fn extended_code(vk: u16, state: u32) -> Option<u8> {
    let alt = state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0;
    let ctrl = state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0;
    let shift = state & SHIFT_PRESSED != 0;
    if (VK_F1..VK_F1 + 10).contains(&vk) {
        let n = (vk - VK_F1) as u8;
        return Some(
            n + if alt {
                0x68
            } else if ctrl {
                0x5E
            } else if shift {
                0x54
            } else {
                0x3B
            },
        );
    }
    if vk == VK_F1 + 10 || vk == VK_F1 + 11 {
        let n = (vk - VK_F1 - 10) as u8;
        return Some(
            n + if alt {
                0x8B
            } else if ctrl {
                0x89
            } else if shift {
                0x87
            } else {
                0x85
            },
        );
    }
    CURSOR.iter().find(|c| c.0 == vk).map(|&(_, plain, with_ctrl, with_alt)| {
        if alt {
            with_alt
        } else if ctrl {
            with_ctrl
        } else {
            plain
        }
    })
}

/// What a key event gives `_getch`, if anything.
pub fn key(k: &KeyEvent) -> Option<Key> {
    if !k.down {
        return None;
    }
    if k.ch != 0 {
        return Some(Key::Char(k.ch));
    }
    let code = extended_code(k.vk, k.state)?;
    Some(Key::Extended(if k.state & ENHANCED_KEY != 0 { 0xE0 } else { 0 }, code))
}

/// `_cgets`' line from what a cooked console read returned: up to `max`
/// units, the line's `\r\n` (or a lone `\r` / `\n`) left out.
pub fn line<C: Copy + PartialEq + From<u8>>(read: &[C], max: usize) -> &[C] {
    let end = read.iter().position(|&c| c == C::from(b'\r') || c == C::from(b'\n')).unwrap_or(read.len());
    &read[..end.min(max)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(vk: u16, ch: u16, state: u32) -> KeyEvent {
        KeyEvent { down: true, repeat: 1, vk, scan: 0, ch, state }
    }

    #[test]
    fn characters_and_releases() {
        assert_eq!(key(&press(0x41, u16::from(b'a'), 0)), Some(Key::Char(u16::from(b'a'))));
        assert_eq!(key(&press(0x41, 1, LEFT_CTRL_PRESSED)), Some(Key::Char(1)));
        assert_eq!(key(&KeyEvent { down: false, ..press(0x41, u16::from(b'a'), 0) }), None);
        // Shift alone.
        assert_eq!(key(&press(0x10, 0, SHIFT_PRESSED)), None);
    }

    #[test]
    fn function_and_cursor_keys() {
        assert_eq!(key(&press(VK_F1, 0, 0)), Some(Key::Extended(0, 0x3B)));
        assert_eq!(key(&press(VK_F1 + 9, 0, 0)), Some(Key::Extended(0, 0x44)));
        assert_eq!(key(&press(VK_F1, 0, SHIFT_PRESSED)), Some(Key::Extended(0, 0x54)));
        assert_eq!(key(&press(VK_F1 + 1, 0, LEFT_CTRL_PRESSED)), Some(Key::Extended(0, 0x5F)));
        assert_eq!(key(&press(VK_F1 + 2, 0, RIGHT_ALT_PRESSED)), Some(Key::Extended(0, 0x6A)));
        assert_eq!(key(&press(VK_F1 + 10, 0, 0)), Some(Key::Extended(0, 0x85)));
        assert_eq!(key(&press(VK_F1 + 11, 0, SHIFT_PRESSED)), Some(Key::Extended(0, 0x88)));
        assert_eq!(key(&press(0x26, 0, ENHANCED_KEY)), Some(Key::Extended(0xE0, 0x48)));
        assert_eq!(key(&press(0x25, 0, ENHANCED_KEY | LEFT_CTRL_PRESSED)), Some(Key::Extended(0xE0, 0x73)));
        assert_eq!(key(&press(0x2E, 0, ENHANCED_KEY | LEFT_ALT_PRESSED)), Some(Key::Extended(0xE0, 0xA3)));
        // The keypad's Home (no ENHANCED_KEY) has the prefix 0.
        assert_eq!(key(&press(0x24, 0, 0)), Some(Key::Extended(0, 0x47)));
    }

    #[test]
    fn records_round_trip() {
        let k = KeyEvent { down: true, repeat: 1, vk: 0x26, scan: 0x48, ch: 0, state: ENHANCED_KEY };
        let r = key_record(&k);
        assert_eq!(key_event(&r), Some(k));
        let mut other = r;
        other[0] = 2; // MOUSE_EVENT
        assert_eq!(key_event(&other), None);
    }

    #[test]
    fn lines() {
        assert_eq!(line(b"hello\r\n", 80), b"hello");
        assert_eq!(line(b"hello\r\n", 3), b"hel");
        assert_eq!(line(b"part", 80), b"part");
        let w: [u16; 3] = [0x41, 0x0D, 0x0A];
        assert_eq!(line(&w, 10), &[0x41]);
    }
}
