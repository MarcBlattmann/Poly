//! Keyboard layouts as data: what each key types on each shift level.
//!
//! A [`Layout`] maps every [`Key`] to a [`KeyDef`], six [`Sym`]s: without
//! modifiers, with Shift, with Caps Lock, with Caps Lock and Shift, with
//! AltGr, and with AltGr and Shift. A symbol is a character, a dead key
//! (an accent that combines with the next character) or nothing. The
//! [`Composer`](crate::Composer) picks the level and handles dead keys; keys
//! that never type text (Enter, the arrows, the keypad's navigation keys)
//! are not part of a layout.
//!
//! Adding a layout means writing one table of `(Key, KeyDef)` entries with
//! the helpers below ([`letter`], [`keys`], [`KeyDef::alt_gr`],
//! [`KeyDef::caps`]) and listing it in [`LAYOUTS`].
//!
//! Sources for the two layouts here: the keyboard layout descriptions in
//! the Microsoft "Windows keyboard layouts" reference ("US" and "Swiss
//! German"), and the W3C "UI Events KeyboardEvent code Values" key
//! positions.

use crate::Key;

/// An accent typed with a dead key: it types nothing by itself and changes
/// the next character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeadKey {
    /// \` (è)
    Grave,
    /// ´ (é)
    Acute,
    /// ^ (ê)
    Circumflex,
    /// ~ (ñ)
    Tilde,
    /// ¨ (ë)
    Diaeresis,
}

impl DeadKey {
    /// The accent as a character of its own: what the dead key types when
    /// followed by Space or by a character it does not combine with.
    pub const fn spacing(self) -> char {
        match self {
            DeadKey::Grave => '`',
            DeadKey::Acute => '\u{b4}',
            DeadKey::Circumflex => '^',
            DeadKey::Tilde => '~',
            DeadKey::Diaeresis => '\u{a8}',
        }
    }

    /// `base` with this accent (Latin-1 and Latin Extended-A), or `None` if
    /// the pair does not combine.
    pub fn compose(self, base: char) -> Option<char> {
        let table: &[(char, char)] = match self {
            DeadKey::Grave => &[
                ('a', 'à'),
                ('e', 'è'),
                ('i', 'ì'),
                ('o', 'ò'),
                ('u', 'ù'),
                ('A', 'À'),
                ('E', 'È'),
                ('I', 'Ì'),
                ('O', 'Ò'),
                ('U', 'Ù'),
            ],
            DeadKey::Acute => &[
                ('a', 'á'),
                ('e', 'é'),
                ('i', 'í'),
                ('o', 'ó'),
                ('u', 'ú'),
                ('y', 'ý'),
                ('A', 'Á'),
                ('E', 'É'),
                ('I', 'Í'),
                ('O', 'Ó'),
                ('U', 'Ú'),
                ('Y', 'Ý'),
            ],
            DeadKey::Circumflex => &[
                ('a', 'â'),
                ('e', 'ê'),
                ('i', 'î'),
                ('o', 'ô'),
                ('u', 'û'),
                ('A', 'Â'),
                ('E', 'Ê'),
                ('I', 'Î'),
                ('O', 'Ô'),
                ('U', 'Û'),
            ],
            DeadKey::Tilde => &[('a', 'ã'), ('n', 'ñ'), ('o', 'õ'), ('A', 'Ã'), ('N', 'Ñ'), ('O', 'Õ')],
            DeadKey::Diaeresis => &[
                ('a', 'ä'),
                ('e', 'ë'),
                ('i', 'ï'),
                ('o', 'ö'),
                ('u', 'ü'),
                ('y', 'ÿ'),
                ('A', 'Ä'),
                ('E', 'Ë'),
                ('I', 'Ï'),
                ('O', 'Ö'),
                ('U', 'Ü'),
                ('Y', 'Ÿ'),
            ],
        };
        table.iter().find(|(b, _)| *b == base).map(|&(_, c)| c)
    }
}

/// What a key types on one level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sym {
    None,
    Char(char),
    Dead(DeadKey),
}

/// Shorthands for the tables.
const fn c(ch: char) -> Sym {
    Sym::Char(ch)
}

const fn dead(d: DeadKey) -> Sym {
    Sym::Dead(d)
}

/// What one key types on each level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyDef {
    pub base: Sym,
    pub shift: Sym,
    /// With Caps Lock on.
    pub caps: Sym,
    /// With Caps Lock on and Shift.
    pub caps_shift: Sym,
    pub alt_gr: Sym,
    pub alt_gr_shift: Sym,
}

impl KeyDef {
    /// A key that types nothing.
    pub const NONE: KeyDef = keys(Sym::None, Sym::None);

    /// The same key, with `sym` on the AltGr level.
    pub const fn alt_gr(self, sym: Sym) -> KeyDef {
        KeyDef { alt_gr: sym, ..self }
    }

    /// The same key, typing `caps` with Caps Lock on and `caps_shift` with
    /// Caps Lock and Shift.
    pub const fn caps(self, caps: Sym, caps_shift: Sym) -> KeyDef {
        KeyDef { caps, caps_shift, ..self }
    }

    /// The symbol for these levels.
    pub const fn level(&self, shift: bool, caps_lock: bool, alt_gr: bool) -> Sym {
        match (alt_gr, caps_lock, shift) {
            (true, _, false) => self.alt_gr,
            (true, _, true) => self.alt_gr_shift,
            (false, false, false) => self.base,
            (false, false, true) => self.shift,
            (false, true, false) => self.caps,
            (false, true, true) => self.caps_shift,
        }
    }
}

/// A key that types `base`, and `shift` with Shift; Caps Lock does not
/// change it.
pub const fn keys(base: Sym, shift: Sym) -> KeyDef {
    KeyDef { base, shift, caps: base, caps_shift: shift, alt_gr: Sym::None, alt_gr_shift: Sym::None }
}

/// A key that types the character `base`, and `shift` with Shift.
pub const fn chars(base: char, shift: char) -> KeyDef {
    keys(c(base), c(shift))
}

/// A letter: Caps Lock inverts Shift.
pub const fn letter(lower: char, upper: char) -> KeyDef {
    KeyDef {
        base: c(lower),
        shift: c(upper),
        caps: c(upper),
        caps_shift: c(lower),
        alt_gr: Sym::None,
        alt_gr_shift: Sym::None,
    }
}

/// The space bar: a space on every level.
const SPACE: KeyDef =
    KeyDef { base: c(' '), shift: c(' '), caps: c(' '), caps_shift: c(' '), alt_gr: c(' '), alt_gr_shift: c(' ') };

/// A keyboard layout.
#[derive(Debug)]
pub struct Layout {
    /// Short name, as on the kernel command line (`poly.keymap=de-ch`).
    pub name: &'static str,
    pub description: &'static str,
    /// The right Alt key is AltGr (selects the `alt_gr` levels) rather than
    /// a second Alt key.
    pub has_alt_gr: bool,
    /// What the keypad's decimal key types with Num Lock on.
    pub keypad_decimal: char,
    keys: [KeyDef; Key::COUNT],
}

impl Layout {
    /// A layout from `(key, definition)` entries; keys without an entry
    /// type nothing. Panics (at compile time, for a `static`) on a key
    /// listed twice.
    pub const fn new(
        name: &'static str,
        description: &'static str,
        has_alt_gr: bool,
        keypad_decimal: char,
        entries: &[(Key, KeyDef)],
    ) -> Layout {
        let mut keys = [KeyDef::NONE; Key::COUNT];
        let mut listed = [false; Key::COUNT];
        let mut i = 0;
        while i < entries.len() {
            let (key, def) = entries[i];
            assert!(!listed[key as usize], "key listed twice in a layout");
            listed[key as usize] = true;
            keys[key as usize] = def;
            i += 1;
        }
        Layout { name, description, has_alt_gr, keypad_decimal, keys }
    }

    pub fn get(&self, key: Key) -> &KeyDef {
        &self.keys[key as usize]
    }
}

/// US English (QWERTY), the 104-key ANSI keyboard. The ISO key next to the
/// left Shift, which US keyboards lack, types \ and | as on the 105-key US
/// variant.
pub static US: Layout = Layout::new(
    "us",
    "US English (QWERTY)",
    false,
    '.',
    &[
        (Key::Backquote, chars('`', '~')),
        (Key::Digit1, chars('1', '!')),
        (Key::Digit2, chars('2', '@')),
        (Key::Digit3, chars('3', '#')),
        (Key::Digit4, chars('4', '$')),
        (Key::Digit5, chars('5', '%')),
        (Key::Digit6, chars('6', '^')),
        (Key::Digit7, chars('7', '&')),
        (Key::Digit8, chars('8', '*')),
        (Key::Digit9, chars('9', '(')),
        (Key::Digit0, chars('0', ')')),
        (Key::Minus, chars('-', '_')),
        (Key::Equal, chars('=', '+')),
        (Key::Q, letter('q', 'Q')),
        (Key::W, letter('w', 'W')),
        (Key::E, letter('e', 'E')),
        (Key::R, letter('r', 'R')),
        (Key::T, letter('t', 'T')),
        (Key::Y, letter('y', 'Y')),
        (Key::U, letter('u', 'U')),
        (Key::I, letter('i', 'I')),
        (Key::O, letter('o', 'O')),
        (Key::P, letter('p', 'P')),
        (Key::BracketLeft, chars('[', '{')),
        (Key::BracketRight, chars(']', '}')),
        (Key::Backslash, chars('\\', '|')),
        (Key::A, letter('a', 'A')),
        (Key::S, letter('s', 'S')),
        (Key::D, letter('d', 'D')),
        (Key::F, letter('f', 'F')),
        (Key::G, letter('g', 'G')),
        (Key::H, letter('h', 'H')),
        (Key::J, letter('j', 'J')),
        (Key::K, letter('k', 'K')),
        (Key::L, letter('l', 'L')),
        (Key::Semicolon, chars(';', ':')),
        (Key::Quote, chars('\'', '"')),
        (Key::IntlBackslash, chars('\\', '|')),
        (Key::Z, letter('z', 'Z')),
        (Key::X, letter('x', 'X')),
        (Key::C, letter('c', 'C')),
        (Key::V, letter('v', 'V')),
        (Key::B, letter('b', 'B')),
        (Key::N, letter('n', 'N')),
        (Key::M, letter('m', 'M')),
        (Key::Comma, chars(',', '<')),
        (Key::Period, chars('.', '>')),
        (Key::Slash, chars('/', '?')),
        (Key::Space, SPACE),
    ],
);

/// Swiss German (de-CH, QWERTZ), the 105-key ISO keyboard. Shift on ü, ö
/// and ä types the French è, é and à; the capitals Ü, Ö and Ä come with
/// Caps Lock. ^, \` (Shift), ~ (AltGr), ´ (AltGr) and ¨ are dead keys.
pub static DE_CH: Layout = Layout::new(
    "de-ch",
    "Swiss German (QWERTZ)",
    true,
    '.',
    &[
        (Key::Backquote, chars('§', '°')),
        (Key::Digit1, chars('1', '+').alt_gr(c('¦'))),
        (Key::Digit2, chars('2', '"').alt_gr(c('@'))),
        (Key::Digit3, chars('3', '*').alt_gr(c('#'))),
        (Key::Digit4, chars('4', 'ç')),
        (Key::Digit5, chars('5', '%')),
        (Key::Digit6, chars('6', '&').alt_gr(c('¬'))),
        (Key::Digit7, chars('7', '/').alt_gr(c('|'))),
        (Key::Digit8, chars('8', '(').alt_gr(c('¢'))),
        (Key::Digit9, chars('9', ')')),
        (Key::Digit0, chars('0', '=')),
        (Key::Minus, chars('\'', '?').alt_gr(dead(DeadKey::Acute))),
        (Key::Equal, keys(dead(DeadKey::Circumflex), dead(DeadKey::Grave)).alt_gr(dead(DeadKey::Tilde))),
        (Key::Q, letter('q', 'Q')),
        (Key::W, letter('w', 'W')),
        (Key::E, letter('e', 'E').alt_gr(c('€'))),
        (Key::R, letter('r', 'R')),
        (Key::T, letter('t', 'T')),
        (Key::Y, letter('z', 'Z')),
        (Key::U, letter('u', 'U')),
        (Key::I, letter('i', 'I')),
        (Key::O, letter('o', 'O')),
        (Key::P, letter('p', 'P')),
        (Key::BracketLeft, chars('ü', 'è').caps(c('Ü'), c('è')).alt_gr(c('['))),
        (Key::BracketRight, keys(dead(DeadKey::Diaeresis), c('!')).alt_gr(c(']'))),
        (Key::A, letter('a', 'A')),
        (Key::S, letter('s', 'S')),
        (Key::D, letter('d', 'D')),
        (Key::F, letter('f', 'F')),
        (Key::G, letter('g', 'G')),
        (Key::H, letter('h', 'H')),
        (Key::J, letter('j', 'J')),
        (Key::K, letter('k', 'K')),
        (Key::L, letter('l', 'L')),
        (Key::Semicolon, chars('ö', 'é').caps(c('Ö'), c('é'))),
        (Key::Quote, chars('ä', 'à').caps(c('Ä'), c('à')).alt_gr(c('{'))),
        (Key::Backslash, chars('$', '£').alt_gr(c('}'))),
        (Key::IntlBackslash, chars('<', '>').alt_gr(c('\\'))),
        (Key::Z, letter('y', 'Y')),
        (Key::X, letter('x', 'X')),
        (Key::C, letter('c', 'C')),
        (Key::V, letter('v', 'V')),
        (Key::B, letter('b', 'B')),
        (Key::N, letter('n', 'N')),
        (Key::M, letter('m', 'M')),
        (Key::Comma, chars(',', ';')),
        (Key::Period, chars('.', ':')),
        (Key::Slash, chars('-', '_')),
        (Key::Space, SPACE),
    ],
);

/// Every layout, the default ([`US`]) first.
pub static LAYOUTS: [&Layout; 2] = [&US, &DE_CH];

/// The layout called `name` (ASCII case-insensitive): "us", "de-ch".
pub fn by_name(name: &str) -> Option<&'static Layout> {
    LAYOUTS.iter().copied().find(|l| l.name.eq_ignore_ascii_case(name))
}
