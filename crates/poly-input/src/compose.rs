//! From key events to text: shift levels, Caps Lock, Num Lock, Ctrl and
//! dead keys.

use crate::layout::{DeadKey, Layout, Sym};
use crate::{Key, KeyEvent};

/// A key that does not type text but that a line editor acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Special {
    /// Enter or keypad Enter.
    Enter,
    Tab,
    Backspace,
    Escape,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    /// F1 to F12.
    Function(u8),
    PrintScreen,
    Pause,
    /// The Application (context menu) key.
    Menu,
}

/// What a key event amounts to. The modifiers are in the event (Shift-Tab
/// is `Special(Tab)` from an event with `SHIFT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Output {
    /// Nothing to type: a release, a modifier or lock key, a key the layout
    /// leaves empty on this level, a repeated dead key.
    None,
    /// One character. With Ctrl held (and not AltGr), a letter types its
    /// control character: Ctrl-C is U+0003.
    Char(char),
    /// Two characters: a dead key's accent that did not combine with what
    /// followed, then that character (or a second accent).
    Chars(char, char),
    /// A dead key is now pending; it changes the next character.
    Dead(DeadKey),
    Special(Special),
}

impl Output {
    /// The characters typed, in order (none, one or two).
    pub fn chars(self) -> impl Iterator<Item = char> {
        let (a, b) = match self {
            Output::Char(a) => (Some(a), None),
            Output::Chars(a, b) => (Some(a), Some(b)),
            _ => (None, None),
        };
        a.into_iter().chain(b)
    }
}

/// Applies a [`Layout`] to key events, keeping the pending dead key.
///
/// - Releases type nothing. Repeats type again, except a repeated dead key.
/// - Keys that do not type text become [`Special`]s and drop a pending dead
///   key (Escape or an arrow cancels an accent). Modifiers and locks leave
///   it pending.
/// - The keypad types digits and its decimal character when Num Lock is on
///   and Shift is not held (or the other way round), and acts as the
///   navigation keys otherwise (7 Home, 8 Up, 9 Page Up, 4 Left, 6 Right,
///   1 End, 2 Down, 3 Page Down, 0 Insert, decimal Delete; 5 nothing).
///   Its / * - + always type.
/// - A dead key followed by Space types the accent alone; by a character it
///   combines with, the accented character; by any other character, the
///   accent and then that character; by another dead key, both accents.
/// - Ctrl (without AltGr) with a letter types the letter's control
///   character (from the letter the layout puts there, so Ctrl-Z is the
///   key labelled Z on every layout) and drops a pending dead key.
#[derive(Clone, Debug, Default)]
pub struct Composer {
    pending: Option<DeadKey>,
}

impl Composer {
    pub const fn new() -> Composer {
        Composer { pending: None }
    }

    /// The dead key waiting for the next character.
    pub fn pending(&self) -> Option<DeadKey> {
        self.pending
    }

    /// Drop a pending dead key.
    pub fn reset(&mut self) {
        self.pending = None;
    }

    pub fn translate(&mut self, layout: &Layout, event: &KeyEvent) -> Output {
        if !event.pressed {
            return Output::None;
        }
        let m = event.modifiers;
        let keypad_digits = m.num_lock() != m.shift();
        if let Some(special) = special(event.key, keypad_digits) {
            self.pending = None;
            return Output::Special(special);
        }
        let sym = match event.key {
            k if k.is_modifier() || k.is_lock() => return Output::None,
            Key::NumpadDivide => Sym::Char('/'),
            Key::NumpadMultiply => Sym::Char('*'),
            Key::NumpadSubtract => Sym::Char('-'),
            Key::NumpadAdd => Sym::Char('+'),
            Key::NumpadDecimal => Sym::Char(layout.keypad_decimal),
            k if k.is_keypad() => match keypad_digit(k) {
                Some(d) if keypad_digits => Sym::Char(d),
                _ => Sym::None, // keypad 5 without Num Lock
            },
            k => layout.get(k).level(m.shift(), m.caps_lock(), layout.has_alt_gr && m.alt_gr()),
        };
        match sym {
            Sym::None => Output::None,
            Sym::Char(ch) if m.control() && !m.alt_gr() && ch.is_ascii_alphabetic() => {
                self.pending = None;
                Output::Char(char::from(ch.to_ascii_uppercase() as u8 & 0x1f))
            }
            Sym::Char(ch) => match self.pending.take() {
                None => Output::Char(ch),
                Some(d) if ch == ' ' => Output::Char(d.spacing()),
                Some(d) => d.compose(ch).map_or(Output::Chars(d.spacing(), ch), Output::Char),
            },
            Sym::Dead(_) if event.repeat => Output::None,
            Sym::Dead(d) => match self.pending.take() {
                Some(first) => Output::Chars(first.spacing(), d.spacing()),
                None => {
                    self.pending = Some(d);
                    Output::Dead(d)
                }
            },
        }
    }
}

/// The special key `key` is, if it is one. Keypad keys are navigation keys
/// unless `keypad_digits`.
fn special(key: Key, keypad_digits: bool) -> Option<Special> {
    let s = match key {
        Key::Enter | Key::NumpadEnter => Special::Enter,
        Key::Tab => Special::Tab,
        Key::Backspace => Special::Backspace,
        Key::Escape => Special::Escape,
        Key::Insert => Special::Insert,
        Key::Delete => Special::Delete,
        Key::Home => Special::Home,
        Key::End => Special::End,
        Key::PageUp => Special::PageUp,
        Key::PageDown => Special::PageDown,
        Key::ArrowUp => Special::Up,
        Key::ArrowDown => Special::Down,
        Key::ArrowLeft => Special::Left,
        Key::ArrowRight => Special::Right,
        Key::PrintScreen => Special::PrintScreen,
        Key::Pause => Special::Pause,
        Key::ContextMenu => Special::Menu,
        Key::F1 => Special::Function(1),
        Key::F2 => Special::Function(2),
        Key::F3 => Special::Function(3),
        Key::F4 => Special::Function(4),
        Key::F5 => Special::Function(5),
        Key::F6 => Special::Function(6),
        Key::F7 => Special::Function(7),
        Key::F8 => Special::Function(8),
        Key::F9 => Special::Function(9),
        Key::F10 => Special::Function(10),
        Key::F11 => Special::Function(11),
        Key::F12 => Special::Function(12),
        _ if keypad_digits => return None,
        Key::Numpad7 => Special::Home,
        Key::Numpad8 => Special::Up,
        Key::Numpad9 => Special::PageUp,
        Key::Numpad4 => Special::Left,
        Key::Numpad6 => Special::Right,
        Key::Numpad1 => Special::End,
        Key::Numpad2 => Special::Down,
        Key::Numpad3 => Special::PageDown,
        Key::Numpad0 => Special::Insert,
        Key::NumpadDecimal => Special::Delete,
        _ => return None,
    };
    Some(s)
}

fn keypad_digit(key: Key) -> Option<char> {
    let d = match key {
        Key::Numpad0 => '0',
        Key::Numpad1 => '1',
        Key::Numpad2 => '2',
        Key::Numpad3 => '3',
        Key::Numpad4 => '4',
        Key::Numpad5 => '5',
        Key::Numpad6 => '6',
        Key::Numpad7 => '7',
        Key::Numpad8 => '8',
        Key::Numpad9 => '9',
        _ => return None,
    };
    Some(d)
}
