//! Which keys are down, the modifiers they make and the three locks.

use crate::set1::ScanEvent;
use crate::{Key, KeyEvent, Leds, Modifiers};

/// Tracks the keyboard's state across [`ScanEvent`]s and turns each into a
/// [`KeyEvent`] with the modifiers that apply.
///
/// - A key is down from its press to its release. A press of a key that is
///   already down is the keyboard's typematic repeat: reported with
///   `repeat` set, and a lock key does not toggle on it.
/// - A modifier bit is set while either key of its pair is down, so
///   releasing one Shift while the other is held keeps `SHIFT`.
/// - Each (non-repeated) press of Caps Lock, Num Lock or Scroll Lock
///   toggles that lock.
/// - The right Alt key is `ALT_GR` or `ALT`, as [`set_alt_gr`](Self::set_alt_gr)
///   says (the layout decides).
/// - A release of a key that is not down (its press was lost) is reported
///   anyway, so nothing stays stuck.
#[derive(Clone, Debug)]
pub struct KeyState {
    down: [u64; 2],
    locks: Modifiers,
    alt_gr: bool,
}

impl Default for KeyState {
    fn default() -> KeyState {
        KeyState::new()
    }
}

/// The lock bits of [`Modifiers`].
const LOCKS: Modifiers = Modifiers(Modifiers::CAPS_LOCK.0 | Modifiers::NUM_LOCK.0 | Modifiers::SCROLL_LOCK.0);

impl KeyState {
    /// No key down, every lock off, the right Alt key is AltGr.
    pub const fn new() -> KeyState {
        KeyState::with_alt_gr(true)
    }

    /// No key down, every lock off; the right Alt key is AltGr if `alt_gr`.
    pub const fn with_alt_gr(alt_gr: bool) -> KeyState {
        KeyState { down: [0; 2], locks: Modifiers::NONE, alt_gr }
    }

    /// Whether the right Alt key is AltGr (`ALT_GR`) or a second Alt key.
    pub fn set_alt_gr(&mut self, alt_gr: bool) {
        self.alt_gr = alt_gr;
    }

    pub fn is_down(&self, key: Key) -> bool {
        let i = key as usize;
        self.down[i / 64] & (1 << (i % 64)) != 0
    }

    fn set_down(&mut self, key: Key, down: bool) {
        let i = key as usize;
        if down {
            self.down[i / 64] |= 1 << (i % 64);
        } else {
            self.down[i / 64] &= !(1 << (i % 64));
        }
    }

    /// Set the locks (only the lock bits of `locks` are used), e.g. to
    /// start with Num Lock on.
    pub fn set_locks(&mut self, locks: Modifiers) {
        self.locks = Modifiers(locks.0 & LOCKS.0);
    }

    /// Forget every key that is down (the keyboard was reset); the locks
    /// stay.
    pub fn release_all(&mut self) {
        self.down = [0; 2];
    }

    /// The modifiers of the keys down now, and the locks.
    pub fn modifiers(&self) -> Modifiers {
        let mut m = self.locks;
        let either = |a, b| self.is_down(a) || self.is_down(b);
        if either(Key::ShiftLeft, Key::ShiftRight) {
            m = m | Modifiers::SHIFT;
        }
        if either(Key::ControlLeft, Key::ControlRight) {
            m = m | Modifiers::CONTROL;
        }
        if self.is_down(Key::AltLeft) || (!self.alt_gr && self.is_down(Key::AltRight)) {
            m = m | Modifiers::ALT;
        }
        if self.alt_gr && self.is_down(Key::AltRight) {
            m = m | Modifiers::ALT_GR;
        }
        if either(Key::MetaLeft, Key::MetaRight) {
            m = m | Modifiers::META;
        }
        m
    }

    /// The LEDs that show the locks.
    pub fn leds(&self) -> Leds {
        self.locks.leds()
    }

    /// Apply one key press or release.
    pub fn apply(&mut self, scan: ScanEvent) -> KeyEvent {
        let repeat = scan.pressed && self.is_down(scan.key);
        self.set_down(scan.key, scan.pressed);
        if scan.pressed && !repeat {
            let lock = match scan.key {
                Key::CapsLock => Modifiers::CAPS_LOCK,
                Key::NumLock => Modifiers::NUM_LOCK,
                Key::ScrollLock => Modifiers::SCROLL_LOCK,
                _ => Modifiers::NONE,
            };
            self.locks = Modifiers(self.locks.0 ^ lock.0);
        }
        KeyEvent { key: scan.key, pressed: scan.pressed, repeat, modifiers: self.modifiers() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(s: &mut KeyState, key: Key) -> KeyEvent {
        s.apply(ScanEvent { key, pressed: true })
    }

    fn release(s: &mut KeyState, key: Key) -> KeyEvent {
        s.apply(ScanEvent { key, pressed: false })
    }

    #[test]
    fn modifiers_follow_either_key_of_a_pair() {
        let mut s = KeyState::new();
        assert_eq!(press(&mut s, Key::ShiftLeft).modifiers, Modifiers::SHIFT);
        assert_eq!(press(&mut s, Key::ShiftRight).modifiers, Modifiers::SHIFT);
        assert_eq!(release(&mut s, Key::ShiftLeft).modifiers, Modifiers::SHIFT);
        assert_eq!(release(&mut s, Key::ShiftRight).modifiers, Modifiers::NONE);
        for (key, bit) in [
            (Key::ControlLeft, Modifiers::CONTROL),
            (Key::ControlRight, Modifiers::CONTROL),
            (Key::AltLeft, Modifiers::ALT),
            (Key::AltRight, Modifiers::ALT_GR),
            (Key::MetaLeft, Modifiers::META),
            (Key::MetaRight, Modifiers::META),
        ] {
            let e = press(&mut s, key);
            assert_eq!((e.key, e.pressed, e.repeat, e.modifiers), (key, true, false, bit), "{key:?}");
            assert_eq!(release(&mut s, key).modifiers, Modifiers::NONE, "{key:?}");
        }
        assert_eq!(press(&mut s, Key::A).modifiers, Modifiers::NONE);
    }

    #[test]
    fn right_alt_is_alt_without_alt_gr() {
        let mut s = KeyState::new();
        s.set_alt_gr(false);
        assert_eq!(press(&mut s, Key::AltRight).modifiers, Modifiers::ALT);
        assert_eq!(press(&mut s, Key::AltLeft).modifiers, Modifiers::ALT);
        release(&mut s, Key::AltLeft);
        assert_eq!(s.modifiers(), Modifiers::ALT, "the right Alt key still holds ALT");
        s.set_alt_gr(true);
        assert_eq!(s.modifiers(), Modifiers::ALT_GR);
    }

    #[test]
    fn locks_toggle_on_press_but_not_on_repeat_or_release() {
        let mut s = KeyState::new();
        for (key, bit, led) in [
            (Key::CapsLock, Modifiers::CAPS_LOCK, Leds::CAPS_LOCK),
            (Key::NumLock, Modifiers::NUM_LOCK, Leds::NUM_LOCK),
            (Key::ScrollLock, Modifiers::SCROLL_LOCK, Leds::SCROLL_LOCK),
        ] {
            assert_eq!(press(&mut s, key).modifiers, bit);
            assert_eq!(s.leds(), led);
            let r = press(&mut s, key);
            assert!(r.repeat, "{key:?}");
            assert_eq!(r.modifiers, bit, "a repeat must not toggle {key:?}");
            assert_eq!(release(&mut s, key).modifiers, bit);
            assert_eq!(press(&mut s, key).modifiers, Modifiers::NONE, "a second press turns {key:?} off");
            release(&mut s, key);
            assert_eq!(s.leds(), Leds::default());
        }
    }

    #[test]
    fn repeats_are_flagged_until_the_release() {
        let mut s = KeyState::new();
        assert!(!press(&mut s, Key::A).repeat);
        assert!(press(&mut s, Key::A).repeat);
        assert!(press(&mut s, Key::A).repeat);
        let r = release(&mut s, Key::A);
        assert!(!r.pressed && !r.repeat);
        assert!(!press(&mut s, Key::A).repeat);
    }

    #[test]
    fn a_release_without_press_is_reported_and_nothing_sticks() {
        let mut s = KeyState::new();
        let e = release(&mut s, Key::ShiftLeft);
        assert_eq!((e.key, e.pressed, e.modifiers), (Key::ShiftLeft, false, Modifiers::NONE));
        press(&mut s, Key::ControlLeft);
        press(&mut s, Key::Z);
        s.release_all();
        assert_eq!(s.modifiers(), Modifiers::NONE);
        assert!(!s.is_down(Key::Z));
    }

    #[test]
    fn set_locks_keeps_only_lock_bits() {
        let mut s = KeyState::new();
        s.set_locks(Modifiers::NUM_LOCK | Modifiers::SHIFT);
        assert_eq!(s.modifiers(), Modifiers::NUM_LOCK);
        assert_eq!(s.leds(), Leds::NUM_LOCK);
        s.release_all();
        assert_eq!(s.modifiers(), Modifiers::NUM_LOCK, "locks survive a release of all keys");
    }
}
