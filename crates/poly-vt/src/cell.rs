//! What one character cell holds: a character, its colours and its style.
//!
//! Colours stay symbolic ([`Color::Default`], an index into the 256-colour
//! palette, or a 24-bit value); the renderer resolves them with its theme
//! and [`crate::palette::xterm_256`]. A wide character (CJK, most emoji)
//! takes two cells: the first holds the character with `width == 2`, the
//! second is a continuation cell with `width == 0` that draws nothing.

use core::ops::{BitOr, BitOrAssign};

/// A colour as the program asked for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Color {
    /// The theme's default foreground or background.
    #[default]
    Default,
    /// One of the 256 palette colours: 0-15 the theme's ANSI colours,
    /// 16-255 the xterm cube and grey ramp ([`crate::palette::xterm_256`]).
    Indexed(u8),
    /// A direct 24-bit colour.
    Rgb(u8, u8, u8),
}

/// A set of text style flags (SGR attributes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Attrs(u16);

impl Attrs {
    /// SGR 1.
    pub const BOLD: Attrs = Attrs(1);
    /// SGR 2 (dim).
    pub const FAINT: Attrs = Attrs(1 << 1);
    /// SGR 3.
    pub const ITALIC: Attrs = Attrs(1 << 2);
    /// SGR 4 (any underline style, `4:1`..`4:5`, and SGR 21).
    pub const UNDERLINE: Attrs = Attrs(1 << 3);
    /// SGR 5 / 6.
    pub const BLINK: Attrs = Attrs(1 << 4);
    /// SGR 7: swap foreground and background when drawing.
    pub const INVERSE: Attrs = Attrs(1 << 5);
    /// SGR 8 (concealed): draw the background only.
    pub const HIDDEN: Attrs = Attrs(1 << 6);
    /// SGR 9 (crossed out).
    pub const STRIKE: Attrs = Attrs(1 << 7);

    /// No flags.
    pub const fn empty() -> Attrs {
        Attrs(0)
    }

    /// The raw flag bits.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Flags from raw bits (unknown bits are kept and ignored).
    pub const fn from_bits(bits: u16) -> Attrs {
        Attrs(bits)
    }

    /// Whether every flag in `other` is set.
    pub const fn contains(self, other: Attrs) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether no flag is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Set the flags in `other`.
    pub fn insert(&mut self, other: Attrs) {
        self.0 |= other.0;
    }

    /// Clear the flags in `other`.
    pub fn remove(&mut self, other: Attrs) {
        self.0 &= !other.0;
    }

    /// Set or clear the flags in `other`.
    pub fn set(&mut self, other: Attrs, on: bool) {
        if on {
            self.insert(other);
        } else {
            self.remove(other);
        }
    }
}

impl BitOr for Attrs {
    type Output = Attrs;
    fn bitor(self, rhs: Attrs) -> Attrs {
        Attrs(self.0 | rhs.0)
    }
}

impl BitOrAssign for Attrs {
    fn bitor_assign(&mut self, rhs: Attrs) {
        self.0 |= rhs.0;
    }
}

/// One character cell of the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    /// The character (a space when blank; ignore it when `width == 0`).
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub attrs: Attrs,
    /// 1 for a normal cell, 2 for the first cell of a wide character, 0
    /// for the second cell of a wide character.
    pub width: u8,
}

impl Cell {
    /// A space in the default colours.
    pub const BLANK: Cell = Cell { ch: ' ', fg: Color::Default, bg: Color::Default, attrs: Attrs::empty(), width: 1 };

    /// A space with background `bg` (what erasing leaves: xterm's
    /// "background colour erase").
    pub const fn blank(bg: Color) -> Cell {
        Cell { ch: ' ', fg: Color::Default, bg, attrs: Attrs::empty(), width: 1 }
    }
}

impl Default for Cell {
    fn default() -> Cell {
        Cell::BLANK
    }
}

/// The style new characters are written with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Pen {
    pub fg: Color,
    pub bg: Color,
    pub attrs: Attrs,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attrs_set_and_contains() {
        let mut a = Attrs::BOLD | Attrs::ITALIC;
        assert!(a.contains(Attrs::BOLD));
        assert!(a.contains(Attrs::BOLD | Attrs::ITALIC));
        assert!(!a.contains(Attrs::UNDERLINE));
        a.remove(Attrs::BOLD);
        assert!(!a.contains(Attrs::BOLD));
        a.set(Attrs::STRIKE, true);
        assert_eq!(a, Attrs::ITALIC | Attrs::STRIKE);
        a.set(Attrs::ITALIC, false);
        a.set(Attrs::STRIKE, false);
        assert!(a.is_empty());
        assert_eq!(Attrs::from_bits(Attrs::HIDDEN.bits()), Attrs::HIDDEN);
    }

    #[test]
    fn blank_cells() {
        assert_eq!(Cell::default(), Cell::BLANK);
        assert_eq!(Cell::blank(Color::Indexed(4)).bg, Color::Indexed(4));
        assert_eq!(Cell::blank(Color::Indexed(4)).ch, ' ');
    }
}
