extern crate std;

use super::*;
use std::vec::Vec;

/// Every key's make and break bytes, written out from the specification's
/// set 1 column (not derived from the decoder's tables).
const EVERY_KEY: &[(Key, &[u8], &[u8])] = &[
    (Key::Escape, &[0x01], &[0x81]),
    (Key::F1, &[0x3b], &[0xbb]),
    (Key::F2, &[0x3c], &[0xbc]),
    (Key::F3, &[0x3d], &[0xbd]),
    (Key::F4, &[0x3e], &[0xbe]),
    (Key::F5, &[0x3f], &[0xbf]),
    (Key::F6, &[0x40], &[0xc0]),
    (Key::F7, &[0x41], &[0xc1]),
    (Key::F8, &[0x42], &[0xc2]),
    (Key::F9, &[0x43], &[0xc3]),
    (Key::F10, &[0x44], &[0xc4]),
    (Key::F11, &[0x57], &[0xd7]),
    (Key::F12, &[0x58], &[0xd8]),
    (Key::PrintScreen, &[0xe0, 0x2a, 0xe0, 0x37], &[0xe0, 0xb7, 0xe0, 0xaa]),
    (Key::ScrollLock, &[0x46], &[0xc6]),
    (Key::Pause, &[0xe1, 0x1d, 0x45], &[0xe1, 0x9d, 0xc5]),
    (Key::Backquote, &[0x29], &[0xa9]),
    (Key::Digit1, &[0x02], &[0x82]),
    (Key::Digit2, &[0x03], &[0x83]),
    (Key::Digit3, &[0x04], &[0x84]),
    (Key::Digit4, &[0x05], &[0x85]),
    (Key::Digit5, &[0x06], &[0x86]),
    (Key::Digit6, &[0x07], &[0x87]),
    (Key::Digit7, &[0x08], &[0x88]),
    (Key::Digit8, &[0x09], &[0x89]),
    (Key::Digit9, &[0x0a], &[0x8a]),
    (Key::Digit0, &[0x0b], &[0x8b]),
    (Key::Minus, &[0x0c], &[0x8c]),
    (Key::Equal, &[0x0d], &[0x8d]),
    (Key::Backspace, &[0x0e], &[0x8e]),
    (Key::Tab, &[0x0f], &[0x8f]),
    (Key::Q, &[0x10], &[0x90]),
    (Key::W, &[0x11], &[0x91]),
    (Key::E, &[0x12], &[0x92]),
    (Key::R, &[0x13], &[0x93]),
    (Key::T, &[0x14], &[0x94]),
    (Key::Y, &[0x15], &[0x95]),
    (Key::U, &[0x16], &[0x96]),
    (Key::I, &[0x17], &[0x97]),
    (Key::O, &[0x18], &[0x98]),
    (Key::P, &[0x19], &[0x99]),
    (Key::BracketLeft, &[0x1a], &[0x9a]),
    (Key::BracketRight, &[0x1b], &[0x9b]),
    (Key::Enter, &[0x1c], &[0x9c]),
    (Key::CapsLock, &[0x3a], &[0xba]),
    (Key::A, &[0x1e], &[0x9e]),
    (Key::S, &[0x1f], &[0x9f]),
    (Key::D, &[0x20], &[0xa0]),
    (Key::F, &[0x21], &[0xa1]),
    (Key::G, &[0x22], &[0xa2]),
    (Key::H, &[0x23], &[0xa3]),
    (Key::J, &[0x24], &[0xa4]),
    (Key::K, &[0x25], &[0xa5]),
    (Key::L, &[0x26], &[0xa6]),
    (Key::Semicolon, &[0x27], &[0xa7]),
    (Key::Quote, &[0x28], &[0xa8]),
    (Key::Backslash, &[0x2b], &[0xab]),
    (Key::ShiftLeft, &[0x2a], &[0xaa]),
    (Key::IntlBackslash, &[0x56], &[0xd6]),
    (Key::Z, &[0x2c], &[0xac]),
    (Key::X, &[0x2d], &[0xad]),
    (Key::C, &[0x2e], &[0xae]),
    (Key::V, &[0x2f], &[0xaf]),
    (Key::B, &[0x30], &[0xb0]),
    (Key::N, &[0x31], &[0xb1]),
    (Key::M, &[0x32], &[0xb2]),
    (Key::Comma, &[0x33], &[0xb3]),
    (Key::Period, &[0x34], &[0xb4]),
    (Key::Slash, &[0x35], &[0xb5]),
    (Key::ShiftRight, &[0x36], &[0xb6]),
    (Key::ControlLeft, &[0x1d], &[0x9d]),
    (Key::MetaLeft, &[0xe0, 0x5b], &[0xe0, 0xdb]),
    (Key::AltLeft, &[0x38], &[0xb8]),
    (Key::Space, &[0x39], &[0xb9]),
    (Key::AltRight, &[0xe0, 0x38], &[0xe0, 0xb8]),
    (Key::MetaRight, &[0xe0, 0x5c], &[0xe0, 0xdc]),
    (Key::ContextMenu, &[0xe0, 0x5d], &[0xe0, 0xdd]),
    (Key::ControlRight, &[0xe0, 0x1d], &[0xe0, 0x9d]),
    (Key::Insert, &[0xe0, 0x52], &[0xe0, 0xd2]),
    (Key::Home, &[0xe0, 0x47], &[0xe0, 0xc7]),
    (Key::PageUp, &[0xe0, 0x49], &[0xe0, 0xc9]),
    (Key::Delete, &[0xe0, 0x53], &[0xe0, 0xd3]),
    (Key::End, &[0xe0, 0x4f], &[0xe0, 0xcf]),
    (Key::PageDown, &[0xe0, 0x51], &[0xe0, 0xd1]),
    (Key::ArrowUp, &[0xe0, 0x48], &[0xe0, 0xc8]),
    (Key::ArrowLeft, &[0xe0, 0x4b], &[0xe0, 0xcb]),
    (Key::ArrowDown, &[0xe0, 0x50], &[0xe0, 0xd0]),
    (Key::ArrowRight, &[0xe0, 0x4d], &[0xe0, 0xcd]),
    (Key::NumLock, &[0x45], &[0xc5]),
    (Key::NumpadDivide, &[0xe0, 0x35], &[0xe0, 0xb5]),
    (Key::NumpadMultiply, &[0x37], &[0xb7]),
    (Key::NumpadSubtract, &[0x4a], &[0xca]),
    (Key::Numpad7, &[0x47], &[0xc7]),
    (Key::Numpad8, &[0x48], &[0xc8]),
    (Key::Numpad9, &[0x49], &[0xc9]),
    (Key::NumpadAdd, &[0x4e], &[0xce]),
    (Key::Numpad4, &[0x4b], &[0xcb]),
    (Key::Numpad5, &[0x4c], &[0xcc]),
    (Key::Numpad6, &[0x4d], &[0xcd]),
    (Key::Numpad1, &[0x4f], &[0xcf]),
    (Key::Numpad2, &[0x50], &[0xd0]),
    (Key::Numpad3, &[0x51], &[0xd1]),
    (Key::NumpadEnter, &[0xe0, 0x1c], &[0xe0, 0x9c]),
    (Key::Numpad0, &[0x52], &[0xd2]),
    (Key::NumpadDecimal, &[0x53], &[0xd3]),
];

fn decode(bytes: &[u8]) -> Vec<ScanEvent> {
    let mut d = Set1Decoder::new();
    let events = bytes.iter().filter_map(|&b| d.feed(b)).collect();
    assert!(!d.in_sequence(), "a complete sequence left the decoder mid-sequence");
    events
}

fn ev(key: Key, pressed: bool) -> ScanEvent {
    ScanEvent { key, pressed }
}

#[test]
fn the_table_covers_every_key_once() {
    let mut seen = [false; Key::COUNT];
    for &(key, _, _) in EVERY_KEY {
        assert!(!seen[key as usize], "{key:?} listed twice");
        seen[key as usize] = true;
    }
    let missing: Vec<Key> = Key::ALL.iter().copied().filter(|&k| !seen[k as usize]).collect();
    assert!(missing.is_empty(), "keys without a sequence: {missing:?}");
    for (i, k) in Key::ALL.iter().enumerate() {
        assert_eq!(*k as usize, i);
    }
}

#[test]
fn every_key_decodes_from_its_make_and_break_sequence() {
    for &(key, make, brk) in EVERY_KEY {
        assert_eq!(decode(make), [ev(key, true)], "make of {key:?}");
        assert_eq!(decode(brk), [ev(key, false)], "break of {key:?}");
    }
}

#[test]
fn every_key_encodes_to_its_sequence() {
    for &(key, make, brk) in EVERY_KEY {
        assert_eq!(encode(key, true).as_slice(), make, "make of {key:?}");
        assert_eq!(encode(key, false).as_slice(), brk, "break of {key:?}");
    }
}

#[test]
fn a_stream_of_every_key_decodes_in_order() {
    // All presses, then all releases in reverse, as one byte stream.
    let mut bytes = Vec::new();
    let mut expected = Vec::new();
    for &(key, make, _) in EVERY_KEY {
        bytes.extend_from_slice(make);
        expected.push(ev(key, true));
    }
    for &(key, _, brk) in EVERY_KEY.iter().rev() {
        bytes.extend_from_slice(brk);
        expected.push(ev(key, false));
    }
    assert_eq!(decode(&bytes), expected);
}

#[test]
fn print_screen_variants() {
    // With Shift or Ctrl held: no fake shifts. With Alt: SysRq (0x54).
    assert_eq!(decode(&[0xe0, 0x37, 0xe0, 0xb7]), [ev(Key::PrintScreen, true), ev(Key::PrintScreen, false)]);
    assert_eq!(decode(&[0x54, 0xd4]), [ev(Key::PrintScreen, true), ev(Key::PrintScreen, false)]);
}

#[test]
fn pause_and_break() {
    // Pause sends its press and release as one sequence.
    let pause = [0xe1, 0x1d, 0x45, 0xe1, 0x9d, 0xc5];
    assert_eq!(decode(&pause), [ev(Key::Pause, true), ev(Key::Pause, false)]);
    // Ctrl-Pause (Break).
    let brk = [0x1d, 0xe0, 0x46, 0xe0, 0xc6, 0x9d];
    assert_eq!(
        decode(&brk),
        [ev(Key::ControlLeft, true), ev(Key::Pause, true), ev(Key::Pause, false), ev(Key::ControlLeft, false)]
    );
}

#[test]
fn fake_shifts_around_navigation_keys_are_dropped() {
    // Insert with Num Lock on (E0 2A first), and Home with left Shift held
    // (E0 AA first, E0 2A after).
    assert_eq!(
        decode(&[0xe0, 0x2a, 0xe0, 0x52, 0xe0, 0xd2, 0xe0, 0xaa]),
        [ev(Key::Insert, true), ev(Key::Insert, false)]
    );
    let shifted_home = [0x2a, 0xe0, 0xaa, 0xe0, 0x47, 0xe0, 0xc7, 0xe0, 0x2a, 0xaa];
    assert_eq!(
        decode(&shifted_home),
        [ev(Key::ShiftLeft, true), ev(Key::Home, true), ev(Key::Home, false), ev(Key::ShiftLeft, false)]
    );
    assert_eq!(
        decode(&[0xe0, 0x36, 0xe0, 0x4d, 0xe0, 0xcd, 0xe0, 0xb6]),
        [ev(Key::ArrowRight, true), ev(Key::ArrowRight, false)]
    );
}

#[test]
fn typematic_repeat_is_repeated_makes() {
    assert_eq!(
        decode(&[0x1e, 0x1e, 0x1e, 0x9e]),
        [ev(Key::A, true), ev(Key::A, true), ev(Key::A, true), ev(Key::A, false)]
    );
}

#[test]
fn replies_and_unknown_codes_are_ignored() {
    // ACK, resend, echo, BAT failure, overrun (0x00, 0xFF), unused codes,
    // and multimedia keys (E0 20 mute, E0 30 volume up).
    assert_eq!(decode(&[0xfa, 0xfe, 0xee, 0xfc, 0x00, 0xff, 0x55, 0x7f, 0xe0, 0x20, 0xe0, 0xa0, 0xe0, 0x30]), []);
}

#[test]
fn broken_sequences_resynchronise() {
    let mut d = Set1Decoder::new();
    // E0 E0 1D: the second prefix restarts the sequence.
    assert_eq!(d.feed(0xe0), None);
    assert_eq!(d.feed(0xe0), None);
    assert_eq!(d.feed(0x1d), Some(ev(Key::ControlRight, true)));
    // E1 followed by a plain key: the prefix is dropped, the key decodes.
    assert_eq!(d.feed(0xe1), None);
    assert_eq!(d.feed(0x1e), Some(ev(Key::A, true)));
    // E1 1D followed by something other than 45: dropped, the byte decodes.
    assert_eq!(d.feed(0xe1), None);
    assert_eq!(d.feed(0x1d), None);
    assert_eq!(d.feed(0x9e), Some(ev(Key::A, false)));
    // Mismatched halves (E1 1D C5) are not Pause.
    assert_eq!(d.feed(0xe1), None);
    assert_eq!(d.feed(0x1d), None);
    assert_eq!(d.feed(0xc5), Some(ev(Key::NumLock, false)));
    // E1 E0 4B: the extended prefix wins.
    assert_eq!(d.feed(0xe1), None);
    assert_eq!(d.feed(0xe0), None);
    assert_eq!(d.feed(0x4b), Some(ev(Key::ArrowLeft, true)));
    assert!(!d.in_sequence());
    d.feed(0xe0);
    d.reset();
    assert_eq!(d.feed(0x4b), Some(ev(Key::Numpad4, true)));
}

#[test]
fn every_byte_after_every_prefix_is_survived() {
    for prefix in [&[][..], &[0xe0], &[0xe1], &[0xe1, 0x1d], &[0xe1, 0x9d]] {
        for b in 0..=255u8 {
            let mut d = Set1Decoder::new();
            for &p in prefix {
                d.feed(p);
            }
            d.feed(b);
            // Whatever happened, a plain key decodes after at most one
            // more byte.
            d.feed(0x9e);
            assert_eq!(d.feed(0x1e), Some(ev(Key::A, true)), "after {prefix:02x?} {b:#04x}");
        }
    }
}
