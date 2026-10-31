//! Keys, modifiers and key events, independent of how the keyboard is
//! attached and of the layout printed on the keycaps.

/// A key on a full-size PC keyboard (104 keys, plus the ISO key next to the
/// left Shift), named by its **physical position**: the legend it has on a
/// US QWERTY keyboard. The names follow the W3C "UI Events KeyboardEvent
/// code Values" specification. What a key types depends on the layout:
/// `Key::Y` types "z" on a Swiss German keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Key {
    // Letters.
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    // The digit row.
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Digit0,
    /// Left of `Digit1` (US: \` and ~; Swiss German: § and °).
    Backquote,
    /// Right of `Digit0` (US: - and _; Swiss German: ' and ?).
    Minus,
    /// Left of Backspace (US: = and +; Swiss German: the ^ \` ~ dead keys).
    Equal,
    /// Right of `P` (US: [ and {; Swiss German: ü and è).
    BracketLeft,
    /// Right of `BracketLeft` (US: ] and }; Swiss German: the ¨ dead key and !).
    BracketRight,
    /// US: \ and |, above Enter. On ISO keyboards this is the key left of
    /// Enter in the home row (Swiss German: $ and £).
    Backslash,
    /// Right of `L` (US: ; and :; Swiss German: ö and é).
    Semicolon,
    /// Right of `Semicolon` (US: ' and "; Swiss German: ä and à).
    Quote,
    Comma,
    Period,
    Slash,
    /// The ISO key between the left Shift and `Z` (Swiss German: < and >).
    /// US keyboards do not have it.
    IntlBackslash,
    Space,
    Enter,
    Backspace,
    Tab,
    Escape,
    CapsLock,
    // Modifiers.
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    /// The right Alt key: AltGr on layouts that have a third level.
    AltRight,
    /// The left Windows / Command key.
    MetaLeft,
    MetaRight,
    /// The Application (context menu) key.
    ContextMenu,
    // Function keys.
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    /// Print Screen, also SysRq (Alt + Print Screen).
    PrintScreen,
    ScrollLock,
    /// Pause, also Break (Ctrl + Pause).
    Pause,
    // The editing and navigation block.
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    // The keypad.
    NumLock,
    NumpadDivide,
    NumpadMultiply,
    NumpadSubtract,
    NumpadAdd,
    NumpadEnter,
    NumpadDecimal,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
}

impl Key {
    /// Number of keys; every `Key as usize` is below it.
    pub const COUNT: usize = Key::Numpad9 as usize + 1;

    /// Every key, in declaration order (`ALL[k as usize] == k`).
    pub const ALL: [Key; Key::COUNT] = {
        use Key::*;
        [
            A,
            B,
            C,
            D,
            E,
            F,
            G,
            H,
            I,
            J,
            K,
            L,
            M,
            N,
            O,
            P,
            Q,
            R,
            S,
            T,
            U,
            V,
            W,
            X,
            Y,
            Z,
            Digit1,
            Digit2,
            Digit3,
            Digit4,
            Digit5,
            Digit6,
            Digit7,
            Digit8,
            Digit9,
            Digit0,
            Backquote,
            Minus,
            Equal,
            BracketLeft,
            BracketRight,
            Backslash,
            Semicolon,
            Quote,
            Comma,
            Period,
            Slash,
            IntlBackslash,
            Space,
            Enter,
            Backspace,
            Tab,
            Escape,
            CapsLock,
            ShiftLeft,
            ShiftRight,
            ControlLeft,
            ControlRight,
            AltLeft,
            AltRight,
            MetaLeft,
            MetaRight,
            ContextMenu,
            F1,
            F2,
            F3,
            F4,
            F5,
            F6,
            F7,
            F8,
            F9,
            F10,
            F11,
            F12,
            PrintScreen,
            ScrollLock,
            Pause,
            Insert,
            Delete,
            Home,
            End,
            PageUp,
            PageDown,
            ArrowUp,
            ArrowDown,
            ArrowLeft,
            ArrowRight,
            NumLock,
            NumpadDivide,
            NumpadMultiply,
            NumpadSubtract,
            NumpadAdd,
            NumpadEnter,
            NumpadDecimal,
            Numpad0,
            Numpad1,
            Numpad2,
            Numpad3,
            Numpad4,
            Numpad5,
            Numpad6,
            Numpad7,
            Numpad8,
            Numpad9,
        ]
    };

    /// Shift, Control, Alt, AltGr (right Alt) or Meta.
    pub const fn is_modifier(self) -> bool {
        matches!(
            self,
            Key::ShiftLeft
                | Key::ShiftRight
                | Key::ControlLeft
                | Key::ControlRight
                | Key::AltLeft
                | Key::AltRight
                | Key::MetaLeft
                | Key::MetaRight
        )
    }

    /// Caps Lock, Num Lock or Scroll Lock: each press toggles a lock.
    pub const fn is_lock(self) -> bool {
        matches!(self, Key::CapsLock | Key::NumLock | Key::ScrollLock)
    }

    /// One of the 17 keys of the numeric keypad, Num Lock included.
    pub const fn is_keypad(self) -> bool {
        (self as u8) >= (Key::NumLock as u8)
    }
}

/// Modifier keys held and locks engaged, as a set of bits.
///
/// `SHIFT`, `CONTROL`, `ALT` and `META` are set while either key of the
/// pair is down; `ALT_GR` while the right Alt key is down on a layout with
/// AltGr (on others the right Alt key is `ALT`). The lock bits are set while
/// the lock is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers(pub u16);

impl Modifiers {
    pub const NONE: Modifiers = Modifiers(0);
    pub const SHIFT: Modifiers = Modifiers(1 << 0);
    pub const CONTROL: Modifiers = Modifiers(1 << 1);
    pub const ALT: Modifiers = Modifiers(1 << 2);
    pub const ALT_GR: Modifiers = Modifiers(1 << 3);
    pub const META: Modifiers = Modifiers(1 << 4);
    pub const CAPS_LOCK: Modifiers = Modifiers(1 << 5);
    pub const NUM_LOCK: Modifiers = Modifiers(1 << 6);
    pub const SCROLL_LOCK: Modifiers = Modifiers(1 << 7);

    /// Every bit in `other` is set in `self`.
    pub const fn contains(self, other: Modifiers) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Modifiers) -> Modifiers {
        Modifiers(self.0 | other.0)
    }

    pub const fn without(self, other: Modifiers) -> Modifiers {
        Modifiers(self.0 & !other.0)
    }

    pub const fn shift(self) -> bool {
        self.contains(Modifiers::SHIFT)
    }

    pub const fn control(self) -> bool {
        self.contains(Modifiers::CONTROL)
    }

    pub const fn alt(self) -> bool {
        self.contains(Modifiers::ALT)
    }

    pub const fn alt_gr(self) -> bool {
        self.contains(Modifiers::ALT_GR)
    }

    pub const fn meta(self) -> bool {
        self.contains(Modifiers::META)
    }

    pub const fn caps_lock(self) -> bool {
        self.contains(Modifiers::CAPS_LOCK)
    }

    pub const fn num_lock(self) -> bool {
        self.contains(Modifiers::NUM_LOCK)
    }

    pub const fn scroll_lock(self) -> bool {
        self.contains(Modifiers::SCROLL_LOCK)
    }

    /// The keyboard LEDs that show these locks.
    pub const fn leds(self) -> Leds {
        let mut l = 0;
        if self.scroll_lock() {
            l |= Leds::SCROLL_LOCK.0;
        }
        if self.num_lock() {
            l |= Leds::NUM_LOCK.0;
        }
        if self.caps_lock() {
            l |= Leds::CAPS_LOCK.0;
        }
        Leds(l)
    }
}

impl core::ops::BitOr for Modifiers {
    type Output = Modifiers;

    fn bitor(self, rhs: Modifiers) -> Modifiers {
        self.union(rhs)
    }
}

/// The three keyboard LEDs, in the bit layout of the PS/2 "set LEDs"
/// command (0xED): bit 0 Scroll Lock, bit 1 Num Lock, bit 2 Caps Lock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Leds(pub u8);

impl Leds {
    pub const SCROLL_LOCK: Leds = Leds(1 << 0);
    pub const NUM_LOCK: Leds = Leds(1 << 1);
    pub const CAPS_LOCK: Leds = Leds(1 << 2);

    /// The evdev LED bits (bit `n` for `LED_*` code `n`: Num Lock 0, Caps
    /// Lock 1, Scroll Lock 2).
    pub const fn to_evdev(self) -> u64 {
        use crate::codes::led;
        let mut b = 0;
        if self.0 & Leds::NUM_LOCK.0 != 0 {
            b |= 1 << led::NUML;
        }
        if self.0 & Leds::CAPS_LOCK.0 != 0 {
            b |= 1 << led::CAPSL;
        }
        if self.0 & Leds::SCROLL_LOCK.0 != 0 {
            b |= 1 << led::SCROLLL;
        }
        b
    }

    /// The PS/2 LED mask for evdev LED bits (others are dropped).
    pub const fn from_evdev(bits: u64) -> Leds {
        use crate::codes::led;
        let mut l = 0;
        if bits & (1 << led::NUML) != 0 {
            l |= Leds::NUM_LOCK.0;
        }
        if bits & (1 << led::CAPSL) != 0 {
            l |= Leds::CAPS_LOCK.0;
        }
        if bits & (1 << led::SCROLLL) != 0 {
            l |= Leds::SCROLL_LOCK.0;
        }
        Leds(l)
    }
}

/// A key went down or came up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub key: Key,
    /// True for a press (also an auto-repeated one), false for a release.
    pub pressed: bool,
    /// A press of a key that was already down: the keyboard's typematic
    /// repeat. Locks do not toggle on repeats.
    pub repeat: bool,
    /// Modifiers and locks after this event: pressing Shift reports
    /// `SHIFT`, releasing it no longer does.
    pub modifiers: Modifiers,
}
