//! Keysyms: what a key means on the current level, as a number a
//! compositor or toolkit understands.
//!
//! The values are the X11 keysyms of `X11/keysymdef.h` (the X.Org
//! protocol headers, MIT licensed), which Wayland's keymaps and
//! toolkits use as well: Latin-1 characters are their own code points,
//! other Unicode characters are `0x0100_0000 + code point`, and the
//! function, modifier, keypad and dead keys have values of their own.
//!
//! [`keysym`] is the keysym of a key on the level the modifiers select,
//! without dead-key composition (that is text, which
//! [`Composer`](crate::Composer) produces).

use crate::layout::{DeadKey, Layout, Sym};
use crate::{Key, Modifiers};

/// An X11 keysym.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Keysym(pub u32);

impl Keysym {
    pub const NO_SYMBOL: Keysym = Keysym(0);
    pub const BACKSPACE: Keysym = Keysym(0xff08);
    pub const TAB: Keysym = Keysym(0xff09);
    pub const RETURN: Keysym = Keysym(0xff0d);
    pub const PAUSE: Keysym = Keysym(0xff13);
    pub const SCROLL_LOCK: Keysym = Keysym(0xff14);
    pub const SYS_REQ: Keysym = Keysym(0xff15);
    pub const ESCAPE: Keysym = Keysym(0xff1b);
    pub const HOME: Keysym = Keysym(0xff50);
    pub const LEFT: Keysym = Keysym(0xff51);
    pub const UP: Keysym = Keysym(0xff52);
    pub const RIGHT: Keysym = Keysym(0xff53);
    pub const DOWN: Keysym = Keysym(0xff54);
    pub const PAGE_UP: Keysym = Keysym(0xff55);
    pub const PAGE_DOWN: Keysym = Keysym(0xff56);
    pub const END: Keysym = Keysym(0xff57);
    pub const PRINT: Keysym = Keysym(0xff61);
    pub const INSERT: Keysym = Keysym(0xff63);
    pub const MENU: Keysym = Keysym(0xff67);
    pub const BREAK: Keysym = Keysym(0xff6b);
    pub const NUM_LOCK: Keysym = Keysym(0xff7f);
    pub const KP_ENTER: Keysym = Keysym(0xff8d);
    pub const KP_HOME: Keysym = Keysym(0xff95);
    pub const KP_LEFT: Keysym = Keysym(0xff96);
    pub const KP_UP: Keysym = Keysym(0xff97);
    pub const KP_RIGHT: Keysym = Keysym(0xff98);
    pub const KP_DOWN: Keysym = Keysym(0xff99);
    pub const KP_PAGE_UP: Keysym = Keysym(0xff9a);
    pub const KP_PAGE_DOWN: Keysym = Keysym(0xff9b);
    pub const KP_END: Keysym = Keysym(0xff9c);
    pub const KP_BEGIN: Keysym = Keysym(0xff9d);
    pub const KP_INSERT: Keysym = Keysym(0xff9e);
    pub const KP_DELETE: Keysym = Keysym(0xff9f);
    pub const KP_MULTIPLY: Keysym = Keysym(0xffaa);
    pub const KP_ADD: Keysym = Keysym(0xffab);
    pub const KP_SUBTRACT: Keysym = Keysym(0xffad);
    pub const KP_DECIMAL: Keysym = Keysym(0xffae);
    pub const KP_DIVIDE: Keysym = Keysym(0xffaf);
    pub const KP_0: Keysym = Keysym(0xffb0);
    pub const F1: Keysym = Keysym(0xffbe);
    pub const SHIFT_L: Keysym = Keysym(0xffe1);
    pub const SHIFT_R: Keysym = Keysym(0xffe2);
    pub const CONTROL_L: Keysym = Keysym(0xffe3);
    pub const CONTROL_R: Keysym = Keysym(0xffe4);
    pub const CAPS_LOCK: Keysym = Keysym(0xffe5);
    pub const ALT_L: Keysym = Keysym(0xffe9);
    pub const ALT_R: Keysym = Keysym(0xffea);
    pub const SUPER_L: Keysym = Keysym(0xffeb);
    pub const SUPER_R: Keysym = Keysym(0xffec);
    pub const DELETE: Keysym = Keysym(0xffff);
    /// AltGr.
    pub const ISO_LEVEL3_SHIFT: Keysym = Keysym(0xfe03);
    pub const DEAD_GRAVE: Keysym = Keysym(0xfe50);
    pub const DEAD_ACUTE: Keysym = Keysym(0xfe51);
    pub const DEAD_CIRCUMFLEX: Keysym = Keysym(0xfe52);
    pub const DEAD_TILDE: Keysym = Keysym(0xfe53);
    pub const DEAD_DIAERESIS: Keysym = Keysym(0xfe57);

    /// The keysym of a character: Latin-1 printable characters are their
    /// own code point, other characters `0x0100_0000 + code point`.
    /// Control characters have none.
    pub const fn of_char(c: char) -> Keysym {
        let cp = c as u32;
        match cp {
            0x20..=0x7e | 0xa0..=0xff => Keysym(cp),
            0..=0x1f | 0x7f..=0x9f => Keysym::NO_SYMBOL,
            _ => Keysym(0x0100_0000 + cp),
        }
    }

    /// The character a keysym stands for, if it is one.
    pub fn to_char(self) -> Option<char> {
        match self.0 {
            0x20..=0x7e | 0xa0..=0xff => char::from_u32(self.0),
            0x0100_0100..=0x0110_ffff => char::from_u32(self.0 - 0x0100_0000),
            _ => None,
        }
    }

    pub const fn of_dead(d: DeadKey) -> Keysym {
        match d {
            DeadKey::Grave => Keysym::DEAD_GRAVE,
            DeadKey::Acute => Keysym::DEAD_ACUTE,
            DeadKey::Circumflex => Keysym::DEAD_CIRCUMFLEX,
            DeadKey::Tilde => Keysym::DEAD_TILDE,
            DeadKey::Diaeresis => Keysym::DEAD_DIAERESIS,
        }
    }
}

/// The keysym of `key` with `modifiers` held and locked, on `layout`:
/// the symbol of the selected level for keys that type, the keypad's digit
/// or navigation keysym as Num Lock and Shift say, and the key's own
/// keysym for the rest.
pub fn keysym(layout: &Layout, key: Key, modifiers: Modifiers) -> Keysym {
    use Key::*;
    let def = layout.get(key);
    if *def != crate::layout::KeyDef::NONE {
        let alt_gr = modifiers.alt_gr() && layout.has_alt_gr;
        return match def.level(modifiers.shift(), modifiers.caps_lock(), alt_gr) {
            Sym::Char(c) => Keysym::of_char(c),
            Sym::Dead(d) => Keysym::of_dead(d),
            Sym::None => Keysym::NO_SYMBOL,
        };
    }
    // Num Lock gives the digits, Shift reverses it.
    let digits = modifiers.num_lock() != modifiers.shift();
    let keypad = |digit: u32, nav: Keysym| if digits { Keysym(Keysym::KP_0.0 + digit) } else { nav };
    match key {
        Enter => Keysym::RETURN,
        Backspace => Keysym::BACKSPACE,
        Tab => Keysym::TAB,
        Escape => Keysym::ESCAPE,
        CapsLock => Keysym::CAPS_LOCK,
        ShiftLeft => Keysym::SHIFT_L,
        ShiftRight => Keysym::SHIFT_R,
        ControlLeft => Keysym::CONTROL_L,
        ControlRight => Keysym::CONTROL_R,
        AltLeft => Keysym::ALT_L,
        AltRight if layout.has_alt_gr => Keysym::ISO_LEVEL3_SHIFT,
        AltRight => Keysym::ALT_R,
        MetaLeft => Keysym::SUPER_L,
        MetaRight => Keysym::SUPER_R,
        ContextMenu => Keysym::MENU,
        F1 | F2 | F3 | F4 | F5 | F6 | F7 | F8 | F9 | F10 | F11 | F12 => Keysym(Keysym::F1.0 + (key as u32 - F1 as u32)),
        PrintScreen if modifiers.alt() => Keysym::SYS_REQ,
        PrintScreen => Keysym::PRINT,
        ScrollLock => Keysym::SCROLL_LOCK,
        Pause if modifiers.control() => Keysym::BREAK,
        Pause => Keysym::PAUSE,
        Insert => Keysym::INSERT,
        Delete => Keysym::DELETE,
        Home => Keysym::HOME,
        End => Keysym::END,
        PageUp => Keysym::PAGE_UP,
        PageDown => Keysym::PAGE_DOWN,
        ArrowUp => Keysym::UP,
        ArrowDown => Keysym::DOWN,
        ArrowLeft => Keysym::LEFT,
        ArrowRight => Keysym::RIGHT,
        NumLock => Keysym::NUM_LOCK,
        NumpadDivide => Keysym::KP_DIVIDE,
        NumpadMultiply => Keysym::KP_MULTIPLY,
        NumpadSubtract => Keysym::KP_SUBTRACT,
        NumpadAdd => Keysym::KP_ADD,
        NumpadEnter => Keysym::KP_ENTER,
        NumpadDecimal => {
            if digits {
                Keysym::KP_DECIMAL
            } else {
                Keysym::KP_DELETE
            }
        }
        Numpad0 => keypad(0, Keysym::KP_INSERT),
        Numpad1 => keypad(1, Keysym::KP_END),
        Numpad2 => keypad(2, Keysym::KP_DOWN),
        Numpad3 => keypad(3, Keysym::KP_PAGE_DOWN),
        Numpad4 => keypad(4, Keysym::KP_LEFT),
        Numpad5 => keypad(5, Keysym::KP_BEGIN),
        Numpad6 => keypad(6, Keysym::KP_RIGHT),
        Numpad7 => keypad(7, Keysym::KP_HOME),
        Numpad8 => keypad(8, Keysym::KP_UP),
        Numpad9 => keypad(9, Keysym::KP_PAGE_UP),
        // Keys that type, without anything on this layout.
        _ => Keysym::NO_SYMBOL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{DE, DE_CH, US};

    #[test]
    fn characters_and_levels() {
        let none = Modifiers::NONE;
        assert_eq!(keysym(&US, Key::A, none), Keysym(u32::from(b'a')));
        assert_eq!(keysym(&US, Key::A, Modifiers::SHIFT), Keysym(u32::from(b'A')));
        assert_eq!(keysym(&US, Key::A, Modifiers::CAPS_LOCK), Keysym(u32::from(b'A')));
        assert_eq!(keysym(&DE_CH, Key::Y, none), Keysym(u32::from(b'z')));
        assert_eq!(keysym(&DE_CH, Key::BracketLeft, none), Keysym(0xfc), "ü is Latin-1");
        assert_eq!(keysym(&DE_CH, Key::E, Modifiers::ALT_GR), Keysym(0x0100_20ac), "€ is Unicode");
        assert_eq!(keysym(&DE, Key::Minus, none), Keysym(0xdf), "ß");
        assert_eq!(keysym(&DE_CH, Key::Equal, none), Keysym::DEAD_CIRCUMFLEX);
        assert_eq!(keysym(&DE, Key::Equal, Modifiers::SHIFT), Keysym::DEAD_GRAVE);
        assert_eq!(keysym(&US, Key::Space, none), Keysym(0x20));
    }

    #[test]
    fn special_and_keypad_keys() {
        let none = Modifiers::NONE;
        assert_eq!(keysym(&US, Key::Enter, none), Keysym::RETURN);
        assert_eq!(keysym(&US, Key::F5, none), Keysym(0xffc2));
        assert_eq!(keysym(&US, Key::F12, none), Keysym(0xffc9));
        assert_eq!(keysym(&US, Key::AltRight, none), Keysym::ALT_R);
        assert_eq!(keysym(&DE_CH, Key::AltRight, none), Keysym::ISO_LEVEL3_SHIFT);
        assert_eq!(keysym(&US, Key::Numpad7, Modifiers::NUM_LOCK), Keysym(0xffb7));
        assert_eq!(keysym(&US, Key::Numpad7, none), Keysym::KP_HOME);
        assert_eq!(keysym(&US, Key::Numpad7, Modifiers::NUM_LOCK | Modifiers::SHIFT), Keysym::KP_HOME);
        assert_eq!(keysym(&US, Key::NumpadDecimal, Modifiers::NUM_LOCK), Keysym::KP_DECIMAL);
        assert_eq!(keysym(&US, Key::Pause, Modifiers::CONTROL), Keysym::BREAK);
        assert_eq!(keysym(&US, Key::IntlBackslash, none), Keysym(u32::from(b'\\')));
        // Every key has a keysym on some level of some layout.
        for key in Key::ALL {
            let any = [&US, &DE_CH, &DE].iter().any(|l| keysym(l, key, Modifiers::NONE) != Keysym::NO_SYMBOL);
            assert!(any, "{key:?}");
        }
    }

    #[test]
    fn character_keysyms_round_trip() {
        for c in [' ', 'a', '~', '\u{a0}', 'ÿ', '€', 'ő', '\u{1f600}'] {
            assert_eq!(Keysym::of_char(c).to_char(), Some(c));
        }
        assert_eq!(Keysym::of_char('\u{3}'), Keysym::NO_SYMBOL);
        assert_eq!(Keysym::RETURN.to_char(), None);
    }
}
