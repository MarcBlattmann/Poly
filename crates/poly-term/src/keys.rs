//! Key events as terminal input.
//!
//! A terminal turns key presses into bytes for the program it runs; the
//! kernel shell's line editor (and later a TTY) reads those bytes, through
//! `poly_line::TermDecoder`, exactly as it reads a serial line. The bytes
//! follow xterm in its default modes (ECMA-48 control sequences, "normal"
//! cursor keys):
//!
//! | Key | Bytes |
//! |---|---|
//! | characters | UTF-8; Ctrl + letter is already a control character (`poly_input`) |
//! | Alt + character | `ESC`, then the character |
//! | Enter, Tab, Shift+Tab, Backspace, Escape | `CR`, `HT`, `ESC [ Z`, `DEL` (Ctrl: `BS`), `ESC` |
//! | Up, Down, Right, Left | `ESC [ A` / `B` / `C` / `D` |
//! | Home, End | `ESC [ H` / `F` |
//! | Insert, Delete, Page Up, Page Down | `ESC [ 2 ~` / `3 ~` / `5 ~` / `6 ~` |
//! | F1-F4 | `ESC O P` / `Q` / `R` / `S` |
//! | F5-F12 | `ESC [ 15 ~`, `17`, `18`, `19`, `20`, `21`, `23`, `24` `~` |
//!
//! With Shift, Alt or Ctrl, the cursor keys, Home, End and F1-F4 become
//! `ESC [ 1 ; m X` and the `~` keys `ESC [ n ; m ~`, where `m` is 1 plus 1
//! for Shift, 2 for Alt and 4 for Ctrl (xterm's "PC-style function keys").
//! Print Screen, Pause, Menu, dead keys and releases send nothing.
//!
//! Shift+Page Up and Shift+Page Down are the terminal's own: they scroll
//! the view a page into the scrollback and back ([`KeyAction::ScrollPages`]),
//! as in most terminal emulators.

use core::fmt;
use poly_input::{Input, Modifiers, Output, Special};

/// The most bytes one key sends (Alt + a character is 5; the longest
/// sequence, `ESC [ 2 4 ; 8 ~`, is 7).
pub const MAX_KEY_BYTES: usize = 8;

/// What one key sends.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct KeyBytes {
    buf: [u8; MAX_KEY_BYTES],
    len: u8,
}

impl fmt::Debug for KeyBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyBytes({:?})", self.as_bytes().escape_ascii().collect::<Bytes>())
    }
}

/// `escape_ascii` output, collected without allocating (for `Debug`).
struct Bytes([u8; 4 * MAX_KEY_BYTES], usize);

impl FromIterator<u8> for Bytes {
    fn from_iter<I: IntoIterator<Item = u8>>(iter: I) -> Bytes {
        let mut b = Bytes([0; 4 * MAX_KEY_BYTES], 0);
        for byte in iter.into_iter().take(b.0.len()) {
            b.0[b.1] = byte;
            b.1 += 1;
        }
        b
    }
}

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(core::str::from_utf8(&self.0[..self.1]).unwrap_or("?"))
    }
}

impl KeyBytes {
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len as usize]
    }

    fn push(&mut self, bytes: &[u8]) {
        let at = self.len as usize;
        let n = bytes.len().min(MAX_KEY_BYTES - at);
        self.buf[at..at + n].copy_from_slice(&bytes[..n]);
        self.len += n as u8;
    }

    fn push_char(&mut self, c: char) {
        let mut b = [0u8; 4];
        self.push(c.encode_utf8(&mut b).as_bytes());
    }

    /// A decimal number (at most 255).
    fn push_number(&mut self, n: u8) {
        if n >= 100 {
            self.push(&[b'0' + n / 100]);
        }
        if n >= 10 {
            self.push(&[b'0' + n / 10 % 10]);
        }
        self.push(&[b'0' + n % 10]);
    }

    fn from(bytes: &[u8]) -> KeyBytes {
        let mut k = KeyBytes::default();
        k.push(bytes);
        k
    }
}

/// What the terminal does with a key event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyAction {
    /// Nothing (a release, a modifier, a dead key, a key without a code).
    None,
    /// Send these bytes to the program reading the terminal.
    Send(KeyBytes),
    /// Scroll the view this many pages back into the scrollback (negative:
    /// toward the screen).
    ScrollPages(i32),
}

/// What `input` does on the terminal; see the module documentation.
pub fn translate(input: &Input) -> KeyAction {
    if !input.event.pressed {
        return KeyAction::None;
    }
    let m = input.event.modifiers;
    let alt = m.alt() && !m.alt_gr();
    let mut k = KeyBytes::default();
    match input.output {
        Output::None | Output::Dead(_) => return KeyAction::None,
        Output::Char(c) => {
            if alt {
                k.push(b"\x1b");
            }
            k.push_char(c);
        }
        Output::Chars(a, b) => {
            k.push_char(a);
            k.push_char(b);
        }
        Output::Special(s) => return special(s, m),
    }
    KeyAction::Send(k)
}

/// xterm's modifier parameter: 1 + Shift + 2 Alt + 4 Ctrl; 1 is none.
fn modifier_param(m: Modifiers) -> u8 {
    1 + u8::from(m.shift()) + 2 * u8::from(m.alt() && !m.alt_gr()) + 4 * u8::from(m.control())
}

/// `ESC [ 1 ; m final` with modifiers, else `plain`.
fn letter_key(plain: &[u8], last: u8, mods: u8) -> KeyBytes {
    if mods == 1 {
        return KeyBytes::from(plain);
    }
    let mut k = KeyBytes::from(b"\x1b[1;");
    k.push_number(mods);
    k.push(&[last]);
    k
}

/// `ESC [ n ~`, or `ESC [ n ; m ~` with modifiers.
fn tilde_key(n: u8, mods: u8) -> KeyBytes {
    let mut k = KeyBytes::from(b"\x1b[");
    k.push_number(n);
    if mods != 1 {
        k.push(b";");
        k.push_number(mods);
    }
    k.push(b"~");
    k
}

fn special(s: Special, m: Modifiers) -> KeyAction {
    let mods = modifier_param(m);
    let alt = m.alt() && !m.alt_gr();
    let with_alt = |bytes: &[u8]| {
        let mut k = KeyBytes::default();
        if alt {
            k.push(b"\x1b");
        }
        k.push(bytes);
        k
    };
    let k = match s {
        Special::PageUp | Special::PageDown if m.shift() && !m.control() && !alt => {
            return KeyAction::ScrollPages(if s == Special::PageUp { 1 } else { -1 });
        }
        Special::Enter => with_alt(b"\r"),
        Special::Tab if m.shift() => KeyBytes::from(b"\x1b[Z"),
        Special::Tab => with_alt(b"\t"),
        Special::Backspace if m.control() => with_alt(b"\x08"),
        Special::Backspace => with_alt(b"\x7f"),
        Special::Escape => with_alt(b"\x1b"),
        Special::Up => letter_key(b"\x1b[A", b'A', mods),
        Special::Down => letter_key(b"\x1b[B", b'B', mods),
        Special::Right => letter_key(b"\x1b[C", b'C', mods),
        Special::Left => letter_key(b"\x1b[D", b'D', mods),
        Special::Home => letter_key(b"\x1b[H", b'H', mods),
        Special::End => letter_key(b"\x1b[F", b'F', mods),
        Special::Insert => tilde_key(2, mods),
        Special::Delete => tilde_key(3, mods),
        Special::PageUp => tilde_key(5, mods),
        Special::PageDown => tilde_key(6, mods),
        Special::Function(n @ 1..=4) => {
            let last = b'P' + (n - 1);
            letter_key(&[0x1b, b'O', last], last, mods)
        }
        Special::Function(n @ 5..=12) => {
            const CODES: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
            tilde_key(CODES[n as usize - 5], mods)
        }
        Special::Function(_) | Special::PrintScreen | Special::Pause | Special::Menu => return KeyAction::None,
    };
    KeyAction::Send(k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_input::layout::{self, Layout};
    use poly_input::set1::encode;
    use poly_input::{Key, Keyboard};
    use std::vec::Vec;

    /// What pressing `key` with `held` down (and Num Lock on) sends, on
    /// `layout`, through the real keyboard pipeline.
    fn press_on(layout: &'static Layout, held: &[Key], key: Key) -> KeyAction {
        let mut kb = Keyboard::new(layout);
        kb.set_locks(Modifiers::NUM_LOCK);
        let mut bytes = Vec::new();
        for &h in held {
            bytes.extend_from_slice(&encode(h, true));
        }
        bytes.extend_from_slice(&encode(key, true));
        let mut last = None;
        for b in bytes {
            if let Some(input) = kb.feed(b) {
                last = Some(input);
            }
        }
        translate(&last.expect("the key made an event"))
    }

    fn press(held: &[Key], key: Key) -> KeyAction {
        press_on(&layout::US, held, key)
    }

    fn sends(held: &[Key], key: Key) -> Vec<u8> {
        match press(held, key) {
            KeyAction::Send(k) => k.as_bytes().to_vec(),
            other => panic!("{key:?} with {held:?} did {other:?}"),
        }
    }

    #[test]
    fn characters_are_utf8_and_ctrl_letters_control_characters() {
        assert_eq!(sends(&[], Key::A), b"a");
        assert_eq!(sends(&[Key::ShiftLeft], Key::A), b"A");
        assert_eq!(sends(&[Key::ControlLeft], Key::C), b"\x03");
        assert_eq!(sends(&[Key::ControlLeft], Key::L), b"\x0c");
        assert_eq!(sends(&[Key::AltLeft], Key::B), b"\x1bb");
        assert_eq!(sends(&[], Key::Numpad7), b"7");
        // de-CH: AltGr is not Alt, so no ESC; dead key then a vowel.
        let de = |held: &[Key], key| match press_on(&layout::DE_CH, held, key) {
            KeyAction::Send(k) => k.as_bytes().to_vec(),
            other => panic!("{other:?}"),
        };
        assert_eq!(de(&[Key::AltRight], Key::Digit2), "@".as_bytes());
        assert_eq!(de(&[], Key::BracketLeft), "ü".as_bytes());
        assert_eq!(press_on(&layout::DE_CH, &[], Key::Equal), KeyAction::None, "a dead key alone");
    }

    #[test]
    fn editing_keys_send_what_the_line_editor_decodes() {
        assert_eq!(sends(&[], Key::Enter), b"\r");
        assert_eq!(sends(&[], Key::NumpadEnter), b"\r");
        assert_eq!(sends(&[], Key::Backspace), b"\x7f");
        assert_eq!(sends(&[Key::ControlLeft], Key::Backspace), b"\x08");
        assert_eq!(sends(&[], Key::Tab), b"\t");
        assert_eq!(sends(&[Key::ShiftLeft], Key::Tab), b"\x1b[Z");
        assert_eq!(sends(&[], Key::Escape), b"\x1b");
        assert_eq!(sends(&[], Key::ArrowUp), b"\x1b[A");
        assert_eq!(sends(&[], Key::ArrowDown), b"\x1b[B");
        assert_eq!(sends(&[], Key::ArrowRight), b"\x1b[C");
        assert_eq!(sends(&[], Key::ArrowLeft), b"\x1b[D");
        assert_eq!(sends(&[Key::ControlLeft], Key::ArrowLeft), b"\x1b[1;5D");
        assert_eq!(sends(&[Key::ShiftLeft, Key::AltLeft], Key::ArrowUp), b"\x1b[1;4A");
        assert_eq!(sends(&[], Key::Home), b"\x1b[H");
        assert_eq!(sends(&[], Key::End), b"\x1b[F");
        assert_eq!(sends(&[], Key::Delete), b"\x1b[3~");
        assert_eq!(sends(&[], Key::Insert), b"\x1b[2~");
        assert_eq!(sends(&[Key::ControlLeft], Key::Delete), b"\x1b[3;5~");
        assert_eq!(sends(&[], Key::PageUp), b"\x1b[5~");
        assert_eq!(sends(&[Key::ControlLeft], Key::PageDown), b"\x1b[6;5~");
        // The keypad without Num Lock (Shift reverses it) is navigation.
        assert_eq!(sends(&[Key::ShiftLeft], Key::Numpad4), b"\x1b[1;2D");
    }

    #[test]
    fn function_keys() {
        assert_eq!(sends(&[], Key::F1), b"\x1bOP");
        assert_eq!(sends(&[], Key::F4), b"\x1bOS");
        assert_eq!(sends(&[Key::ControlLeft], Key::F2), b"\x1b[1;5Q");
        assert_eq!(sends(&[], Key::F5), b"\x1b[15~");
        assert_eq!(sends(&[], Key::F6), b"\x1b[17~");
        assert_eq!(sends(&[], Key::F11), b"\x1b[23~");
        assert_eq!(sends(&[Key::ControlLeft, Key::ShiftLeft, Key::AltLeft], Key::F12), b"\x1b[24;8~");
        assert_eq!(press(&[], Key::PrintScreen), KeyAction::None);
        assert_eq!(press(&[], Key::ContextMenu), KeyAction::None);
    }

    #[test]
    fn shift_page_keys_scroll_the_view() {
        assert_eq!(press(&[Key::ShiftLeft], Key::PageUp), KeyAction::ScrollPages(1));
        assert_eq!(press(&[Key::ShiftRight], Key::PageDown), KeyAction::ScrollPages(-1));
    }

    #[test]
    fn releases_and_modifiers_send_nothing() {
        assert_eq!(press(&[], Key::ShiftLeft), KeyAction::None);
        assert_eq!(press(&[], Key::CapsLock), KeyAction::None);
        let mut kb = Keyboard::new(&layout::US);
        let mut events = encode(Key::A, true).as_slice().to_vec();
        events.extend_from_slice(&encode(Key::A, false));
        let actions: Vec<_> = events.into_iter().filter_map(|b| kb.feed(b)).map(|i| translate(&i)).collect();
        assert_eq!(actions, [KeyAction::Send(KeyBytes::from(b"a")), KeyAction::None]);
    }

    #[test]
    fn every_key_with_every_modifier_fits_and_decodes() {
        // Whatever is sent is short enough, and nothing panics.
        let mods = [Key::ShiftLeft, Key::ControlLeft, Key::AltLeft, Key::AltRight];
        for layout in [&layout::US, &layout::DE_CH] {
            for &key in Key::ALL.iter() {
                for mask in 0..16u8 {
                    let held: Vec<Key> =
                        mods.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, &k)| k).collect();
                    if held.contains(&key) {
                        continue;
                    }
                    let mut kb = Keyboard::new(layout);
                    let mut last = None;
                    for &h in &held {
                        for &b in encode(h, true).as_slice() {
                            last = kb.feed(b).or(last);
                        }
                    }
                    for &b in encode(key, true).as_slice() {
                        last = kb.feed(b).or(last);
                    }
                    if let Some(input) = last
                        && let KeyAction::Send(k) = translate(&input)
                    {
                        assert!(!k.as_bytes().is_empty() && k.as_bytes().len() <= MAX_KEY_BYTES);
                        let _ = std::format!("{k:?}");
                    }
                }
            }
        }
    }

    /// What the line editor makes of a key: through the same decoder the
    /// serial line uses.
    #[test]
    fn the_line_editor_decodes_every_editing_key() {
        use poly_line::{Edit, TermDecoder};
        let edit = |held: &[Key], key: Key| {
            let mut d = TermDecoder::new();
            let edits: Vec<Edit> = sends(held, key).into_iter().filter_map(|b| d.feed(b)).collect();
            edits
        };
        let cases: [(&[Key], Key, Edit); 22] = [
            (&[], Key::A, Edit::Insert('a')),
            (&[Key::ShiftLeft], Key::Digit1, Edit::Insert('!')),
            (&[], Key::Enter, Edit::Enter),
            (&[], Key::NumpadEnter, Edit::Enter),
            (&[], Key::Backspace, Edit::Backspace),
            (&[Key::ControlLeft], Key::Backspace, Edit::Backspace),
            (&[], Key::Delete, Edit::Delete),
            (&[], Key::Tab, Edit::Complete),
            (&[], Key::ArrowLeft, Edit::Left),
            (&[], Key::ArrowRight, Edit::Right),
            (&[], Key::ArrowUp, Edit::HistoryPrev),
            (&[], Key::ArrowDown, Edit::HistoryNext),
            (&[], Key::Home, Edit::Home),
            (&[], Key::End, Edit::End),
            (&[Key::ControlLeft], Key::ArrowLeft, Edit::WordLeft),
            (&[Key::ControlLeft], Key::ArrowRight, Edit::WordRight),
            (&[Key::AltLeft], Key::B, Edit::WordLeft),
            (&[Key::AltLeft], Key::F, Edit::WordRight),
            (&[Key::ControlLeft], Key::C, Edit::Cancel),
            (&[Key::ControlLeft], Key::L, Edit::ClearScreen),
            (&[Key::ControlLeft], Key::U, Edit::KillToStart),
            (&[], Key::Numpad1, Edit::Insert('1')),
        ];
        for (held, key, want) in cases {
            assert_eq!(edit(held, key), [want], "{key:?} with {held:?}");
        }
        // Keys the editor has no use for decode to nothing (and leave the
        // decoder ready for the next key).
        for key in [Key::PageUp, Key::Insert, Key::F1, Key::F5] {
            assert_eq!(edit(&[], key), [], "{key:?}");
        }
        let mut d = TermDecoder::new();
        let mut bytes = sends(&[], Key::F12);
        bytes.extend(sends(&[], Key::A));
        assert_eq!(bytes.into_iter().filter_map(|b| d.feed(b)).collect::<Vec<_>>(), [Edit::Insert('a')]);
    }

    #[test]
    fn numbers_are_written_in_decimal() {
        let mut k = KeyBytes::default();
        k.push_number(7);
        k.push_number(24);
        k.push_number(255);
        assert_eq!(k.as_bytes(), b"724255");
        let mut full = KeyBytes::from(b"12345678");
        full.push(b"9");
        assert_eq!(full.as_bytes(), b"12345678", "extra bytes are dropped");
    }
}
