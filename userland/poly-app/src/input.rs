//! Keys as an app sees them: the evdev code of `wl_keyboard.key` with the
//! seat's XKB modifier bits, turned into Poly's [`Key`] (the physical key)
//! and the character it types on the desktop's keyboard layout (the
//! settings' `keyboard`, which is also the layout of the keymap the
//! compositor hands out), with `poly-input`'s tables, as the compositor
//! does for its launcher. Dead keys type nothing (no composing yet).
//!
//! Key repeat is the client's under Wayland (`wl_keyboard.repeat_info`):
//! [`Repeat`] keeps the key held and when it fires next.

use poly_input::key::{Key, Modifiers};
use poly_input::keysym::keysym;
use poly_input::layout::Layout;
use poly_input::xkb::mods;

/// Which modifiers are held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// The Windows key.
    pub meta: bool,
}

impl Mods {
    /// From the seat's XKB modifier bits.
    pub fn from_xkb(depressed: u32) -> Mods {
        Mods {
            shift: depressed & mods::SHIFT != 0,
            ctrl: depressed & mods::CONTROL != 0,
            alt: depressed & mods::MOD1 != 0,
            meta: depressed & mods::MOD4 != 0,
        }
    }

    /// No Ctrl, Alt or Windows key (Shift may be held).
    pub fn plain(self) -> bool {
        !self.ctrl && !self.alt && !self.meta
    }
}

/// A key going down (or repeating) or up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyInput {
    /// The evdev code.
    pub code: u32,
    /// The physical key, if Poly knows it.
    pub key: Option<Key>,
    /// What it types on the layout with these modifiers (not a control
    /// character; also when Ctrl or Alt is held, so Ctrl+C is `c`).
    pub ch: Option<char>,
    pub mods: Mods,
    pub pressed: bool,
    /// A repeat of a held key.
    pub repeat: bool,
}

/// `poly-input`'s modifiers for the seat's bits (depressed and locked).
pub fn modifiers(depressed: u32, locked: u32) -> Modifiers {
    let all = depressed | locked;
    let mut m = Modifiers::NONE;
    for (bit, modifier) in [
        (mods::SHIFT, Modifiers::SHIFT),
        (mods::LOCK, Modifiers::CAPS_LOCK),
        (mods::CONTROL, Modifiers::CONTROL),
        (mods::MOD1, Modifiers::ALT),
        (mods::MOD2, Modifiers::NUM_LOCK),
        (mods::MOD4, Modifiers::META),
        (mods::MOD5, Modifiers::ALT_GR),
    ] {
        if all & bit != 0 {
            m = m.union(modifier);
        }
    }
    m
}

/// Key `code` with the seat's modifiers on `layout`.
pub fn translate(code: u32, pressed: bool, depressed: u32, locked: u32, layout: &Layout) -> KeyInput {
    let key = u16::try_from(code).ok().and_then(poly_input::keycode::key);
    // AltGr is MOD5 in Poly's keymaps; Alt is not AltGr.
    let ch = key.and_then(|k| keysym(layout, k, modifiers(depressed, locked)).to_char()).filter(|c| !c.is_control());
    KeyInput { code, key, ch, mods: Mods::from_xkb(depressed), pressed, repeat: false }
}

/// Whether `key` is a modifier or a lock (they do not repeat).
pub fn is_modifier(key: Option<Key>) -> bool {
    use Key::*;
    matches!(
        key,
        Some(
            ShiftLeft
                | ShiftRight
                | ControlLeft
                | ControlRight
                | AltLeft
                | AltRight
                | MetaLeft
                | MetaRight
                | CapsLock
                | NumLock
                | ScrollLock
        )
    )
}

/// The key being held, for repeating it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Repeat {
    held: Option<(u32, u64)>,
    /// Repeats per second (0: none) and the delay before the first, ms.
    pub rate: i32,
    pub delay: i32,
}

impl Repeat {
    /// Key `code` went down or up at `now_ms`.
    pub fn key(&mut self, code: u32, pressed: bool, modifier: bool, now_ms: u64) {
        if pressed && !modifier && self.rate > 0 {
            self.held = Some((code, now_ms + self.delay.max(0) as u64));
        } else if self.held.is_some_and(|(c, _)| c == code) || !pressed && modifier {
            // Letting go of the key, or of a modifier (Ctrl+C held, Ctrl
            // let go: the repeat would type a plain c).
            self.held = None;
        }
    }

    pub fn stop(&mut self) {
        self.held = None;
    }

    /// When the next repeat is due.
    pub fn deadline(&self) -> Option<u64> {
        self.held.map(|(_, at)| at)
    }

    /// The key to repeat now, if one is due at `now_ms`.
    pub fn fire(&mut self, now_ms: u64) -> Option<u32> {
        let (code, at) = self.held?;
        if now_ms < at {
            return None;
        }
        let step = (1000 / self.rate.max(1)) as u64;
        // Behind by more than a step (the loop was busy): do not burst.
        let next = if now_ms - at > step { now_ms + step } else { at + step };
        self.held = Some((code, next));
        Some(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_input::codes::key as k;
    use poly_input::layout::{DE_CH, US};

    #[test]
    fn characters_follow_the_layout_and_shift() {
        let t = |code: u16, d: u32, l: &Layout| translate(u32::from(code), true, d, 0, l).ch;
        assert_eq!(t(k::A, 0, &US), Some('a'));
        assert_eq!(t(k::A, mods::SHIFT, &US), Some('A'));
        assert_eq!(t(k::Y, 0, &US), Some('y'));
        assert_eq!(t(k::Y, 0, &DE_CH), Some('z'), "QWERTZ");
        assert_eq!(t(k::N1, mods::SHIFT, &US), Some('!'));
        assert_eq!(t(k::ENTER, 0, &US), None, "Enter is a key, not a character");
        let c = translate(u32::from(k::C), true, mods::CONTROL, 0, &US);
        assert_eq!((c.ch, c.mods.ctrl, c.key), (Some('c'), true, Some(Key::C)));
        assert!(!c.mods.plain());
    }

    #[test]
    fn caps_lock_and_altgr() {
        assert_eq!(translate(u32::from(k::A), true, 0, mods::LOCK, &US).ch, Some('A'));
        // de-ch: AltGr+2 is @.
        assert_eq!(translate(u32::from(k::N2), true, mods::MOD5, 0, &DE_CH).ch, Some('@'));
    }

    #[test]
    fn repeat_waits_the_delay_then_fires_at_the_rate() {
        let mut r = Repeat { rate: 25, delay: 600, ..Repeat::default() };
        r.key(30, true, false, 1000);
        assert_eq!(r.deadline(), Some(1600));
        assert_eq!(r.fire(1599), None);
        assert_eq!(r.fire(1600), Some(30));
        assert_eq!(r.deadline(), Some(1640));
        assert_eq!(r.fire(1640), Some(30));
        // Far behind: the next is a step from now, not a burst.
        assert_eq!(r.fire(5000), Some(30));
        assert_eq!(r.deadline(), Some(5040));
        r.key(30, false, false, 5001);
        assert_eq!(r.deadline(), None);
        // Modifiers never repeat; letting one go stops a repeat.
        r.key(42, true, true, 0);
        assert_eq!(r.deadline(), None);
        r.key(46, true, false, 0);
        r.key(29, false, true, 10);
        assert_eq!(r.deadline(), None);
        // No repeat at rate 0.
        let mut none = Repeat::default();
        none.key(30, true, false, 0);
        assert_eq!(none.deadline(), None);
    }
}
