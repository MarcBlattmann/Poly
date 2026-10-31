//! Physical keys ([`Key`]) and evdev key codes (`KEY_*`), both ways.
//!
//! The scancode decoders ([`crate::set1`], [`crate::set2`]) turn bytes
//! into a [`Key`]; this table turns that into the key code the event layer
//! and `/dev/input/event*` carry, and a key code from a virtio keyboard
//! back into a [`Key`] for the kernel's own console. The pairs follow the
//! key names of `linux/input-event-codes.h`; the PC Application key is
//! `KEY_COMPOSE` (as PC keyboards report it), and `KEY_MENU` maps to it as
//! well.

use crate::Key;
use crate::codes::key as k;

/// Every key and its code, in [`Key`] order.
const TABLE: [(Key, u16); Key::COUNT] = [
    (Key::A, k::A),
    (Key::B, k::B),
    (Key::C, k::C),
    (Key::D, k::D),
    (Key::E, k::E),
    (Key::F, k::F),
    (Key::G, k::G),
    (Key::H, k::H),
    (Key::I, k::I),
    (Key::J, k::J),
    (Key::K, k::K),
    (Key::L, k::L),
    (Key::M, k::M),
    (Key::N, k::N),
    (Key::O, k::O),
    (Key::P, k::P),
    (Key::Q, k::Q),
    (Key::R, k::R),
    (Key::S, k::S),
    (Key::T, k::T),
    (Key::U, k::U),
    (Key::V, k::V),
    (Key::W, k::W),
    (Key::X, k::X),
    (Key::Y, k::Y),
    (Key::Z, k::Z),
    (Key::Digit1, k::N1),
    (Key::Digit2, k::N2),
    (Key::Digit3, k::N3),
    (Key::Digit4, k::N4),
    (Key::Digit5, k::N5),
    (Key::Digit6, k::N6),
    (Key::Digit7, k::N7),
    (Key::Digit8, k::N8),
    (Key::Digit9, k::N9),
    (Key::Digit0, k::N0),
    (Key::Backquote, k::GRAVE),
    (Key::Minus, k::MINUS),
    (Key::Equal, k::EQUAL),
    (Key::BracketLeft, k::LEFTBRACE),
    (Key::BracketRight, k::RIGHTBRACE),
    (Key::Backslash, k::BACKSLASH),
    (Key::Semicolon, k::SEMICOLON),
    (Key::Quote, k::APOSTROPHE),
    (Key::Comma, k::COMMA),
    (Key::Period, k::DOT),
    (Key::Slash, k::SLASH),
    (Key::IntlBackslash, k::KEY_102ND),
    (Key::Space, k::SPACE),
    (Key::Enter, k::ENTER),
    (Key::Backspace, k::BACKSPACE),
    (Key::Tab, k::TAB),
    (Key::Escape, k::ESC),
    (Key::CapsLock, k::CAPSLOCK),
    (Key::ShiftLeft, k::LEFTSHIFT),
    (Key::ShiftRight, k::RIGHTSHIFT),
    (Key::ControlLeft, k::LEFTCTRL),
    (Key::ControlRight, k::RIGHTCTRL),
    (Key::AltLeft, k::LEFTALT),
    (Key::AltRight, k::RIGHTALT),
    (Key::MetaLeft, k::LEFTMETA),
    (Key::MetaRight, k::RIGHTMETA),
    (Key::ContextMenu, k::COMPOSE),
    (Key::F1, k::F1),
    (Key::F2, k::F2),
    (Key::F3, k::F3),
    (Key::F4, k::F4),
    (Key::F5, k::F5),
    (Key::F6, k::F6),
    (Key::F7, k::F7),
    (Key::F8, k::F8),
    (Key::F9, k::F9),
    (Key::F10, k::F10),
    (Key::F11, k::F11),
    (Key::F12, k::F12),
    (Key::PrintScreen, k::SYSRQ),
    (Key::ScrollLock, k::SCROLLLOCK),
    (Key::Pause, k::PAUSE),
    (Key::Insert, k::INSERT),
    (Key::Delete, k::DELETE),
    (Key::Home, k::HOME),
    (Key::End, k::END),
    (Key::PageUp, k::PAGEUP),
    (Key::PageDown, k::PAGEDOWN),
    (Key::ArrowUp, k::UP),
    (Key::ArrowDown, k::DOWN),
    (Key::ArrowLeft, k::LEFT),
    (Key::ArrowRight, k::RIGHT),
    (Key::NumLock, k::NUMLOCK),
    (Key::NumpadDivide, k::KPSLASH),
    (Key::NumpadMultiply, k::KPASTERISK),
    (Key::NumpadSubtract, k::KPMINUS),
    (Key::NumpadAdd, k::KPPLUS),
    (Key::NumpadEnter, k::KPENTER),
    (Key::NumpadDecimal, k::KPDOT),
    (Key::Numpad0, k::KP0),
    (Key::Numpad1, k::KP1),
    (Key::Numpad2, k::KP2),
    (Key::Numpad3, k::KP3),
    (Key::Numpad4, k::KP4),
    (Key::Numpad5, k::KP5),
    (Key::Numpad6, k::KP6),
    (Key::Numpad7, k::KP7),
    (Key::Numpad8, k::KP8),
    (Key::Numpad9, k::KP9),
];

/// The evdev key code of `key`.
pub const fn code(key: Key) -> u16 {
    TABLE[key as usize].1
}

/// The key with evdev code `code`, if it is one of the keys [`Key`] names.
pub fn key(code: u16) -> Option<Key> {
    if code == k::MENU {
        return Some(Key::ContextMenu);
    }
    TABLE.iter().find(|(_, c)| *c == code).map(|&(key, _)| key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_in_key_order_and_codes_are_unique() {
        for (i, (key, _)) in TABLE.iter().enumerate() {
            assert_eq!(*key as usize, i, "{key:?} out of order");
        }
        let mut seen = [false; crate::codes::KEY_CNT];
        for (key, c) in TABLE {
            assert!(!seen[c as usize], "{key:?}: code {c} twice");
            seen[c as usize] = true;
            assert!(c < k::BTN_MISC, "{key:?} is a keyboard key");
        }
    }

    #[test]
    fn codes_round_trip() {
        for key in Key::ALL {
            assert_eq!(super::key(code(key)), Some(key));
        }
        assert_eq!(super::key(k::MENU), Some(Key::ContextMenu));
        assert_eq!(super::key(k::BTN_LEFT), None);
        assert_eq!(super::key(0), None);
    }

    #[test]
    fn leds_convert_both_ways() {
        use crate::Leds;
        for m in 0..8u8 {
            assert_eq!(Leds::from_evdev(Leds(m).to_evdev()), Leds(m));
        }
        assert_eq!(Leds::CAPS_LOCK.to_evdev(), 1 << crate::codes::led::CAPSL);
        assert_eq!(Leds::from_evdev(1 << crate::codes::led::NUML | 1 << 7), Leds::NUM_LOCK);
    }

    #[test]
    fn the_main_block_matches_set_1() {
        // The key codes of the original 83-key block are its set 1 make
        // codes; a cross-check of both tables.
        for key in Key::ALL {
            let bytes = crate::set1::encode(key, true);
            if bytes.len() == 1 && bytes[0] < 0x54 {
                assert_eq!(code(key), u16::from(bytes[0]), "{key:?}");
            }
        }
    }
}
