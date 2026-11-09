//! Windows font names on Poly (ADR 0170).
//!
//! Poly ships DejaVu Sans, DejaVu Serif and DejaVu Sans Mono (ADR 0105).
//! Windows programs ask for Arial, Segoe UI, Times New Roman, Courier New and
//! so on, and expect them to exist with four standard faces; the system
//! collection therefore holds each DejaVu family under its own name and
//! under the Windows names that stand for it: a *family alias* groups the
//! same font files under another family name, with the face names
//! `Regular`, `Bold`, `Italic` and `Bold Italic` Windows families use.
//! Coverage follows the DejaVu fonts, except that the emoji family covers
//! only Latin-1, the euro sign and the symbol and emoji blocks.

use crate::face::{STYLE_ITALIC, STYLE_OBLIQUE};
use crate::names::Localized;

/// A Windows family name that Poly serves with a DejaVu family.
#[derive(Clone, Copy, Debug)]
pub struct Alias {
    pub name: &'static str,
    /// The DejaVu family whose fonts it uses.
    pub target: &'static str,
    /// `IsSymbolFont` is true for its fonts (Symbol, Wingdings).
    pub symbol: bool,
    pub charset: Option<&'static [(u32, u32)]>,
}

const SANS: &str = "DejaVu Sans";
const SERIF: &str = "DejaVu Serif";
const MONO: &str = "DejaVu Sans Mono";

/// Latin-1, the euro sign, symbols, dingbats and the emoji planes.
pub static EMOJI_SET: [(u32, u32); 6] =
    [(0x20, 0x7E), (0xA0, 0xFF), (0x20AC, 0x20AC), (0x2190, 0x2BFF), (0x1F000, 0x1FAFF), (0xFE00, 0xFE0F)];

const fn a(name: &'static str, target: &'static str) -> Alias {
    Alias { name, target, symbol: false, charset: None }
}

pub static ALIASES: &[Alias] = &[
    a("Arial", SANS),
    a("Arial Unicode MS", SANS),
    a("Calibri", SANS),
    a("Candara", SANS),
    a("Corbel", SANS),
    a("Franklin Gothic Medium", SANS),
    a("Lucida Sans Unicode", SANS),
    a("Malgun Gothic", SANS),
    a("Microsoft Sans Serif", SANS),
    a("Microsoft YaHei UI", SANS),
    a("Nirmala UI", SANS),
    a("Leelawadee UI", SANS),
    a("Segoe UI", SANS),
    a("Segoe UI Symbol", SANS),
    Alias { name: "Segoe UI Emoji", target: SANS, symbol: false, charset: Some(&EMOJI_SET) },
    Alias { name: "Symbol", target: SANS, symbol: true, charset: None },
    a("Tahoma", SANS),
    a("Trebuchet MS", SANS),
    a("Verdana", SANS),
    Alias { name: "Webdings", target: SANS, symbol: true, charset: None },
    Alias { name: "Wingdings", target: SANS, symbol: true, charset: None },
    a("Yu Gothic UI", SANS),
    a("Book Antiqua", SERIF),
    a("Cambria", SERIF),
    a("Constantia", SERIF),
    a("Georgia", SERIF),
    a("Palatino Linotype", SERIF),
    a("Times New Roman", SERIF),
    a("Cascadia Code", MONO),
    a("Cascadia Mono", MONO),
    a("Consolas", MONO),
    a("Courier New", MONO),
    a("Lucida Console", MONO),
    a("Lucida Sans Typewriter", MONO),
];

/// The face name Windows families use for a weight and style.
pub fn standard_face_name(weight: u32, style: u32) -> Localized {
    let bold = weight >= 600;
    let italic = style == STYLE_ITALIC || style == STYLE_OBLIQUE;
    Localized::en_us(match (bold, italic) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "Bold Italic",
    })
}

/// Is `cp` inside the ranges (all code points when `None`)?
pub fn in_charset(set: Option<&[(u32, u32)]>, cp: u32) -> bool {
    set.is_none_or(|s| s.iter().any(|&(a, b)| (a..=b).contains(&cp)))
}

/// The font families (by name) the fallback tries for a code point, in
/// order: the first that covers it wins.
pub fn fallback_families(cp: u32) -> &'static [&'static str] {
    match cp {
        0x1F000..=0x1FAFF | 0x2600..=0x27BF => &["Segoe UI Emoji", "Segoe UI Symbol", "Segoe UI"],
        0x2190..=0x2BFF => &["Segoe UI Symbol", "Segoe UI"],
        _ => &["Segoe UI", "Segoe UI Symbol", "Arial", "DejaVu Sans", "DejaVu Serif", "DejaVu Sans Mono"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_names() {
        assert_eq!(standard_face_name(400, 0), Localized::en_us("Regular"));
        assert_eq!(standard_face_name(700, 2), Localized::en_us("Bold Italic"));
    }

    #[test]
    fn emoji_charset() {
        assert!(in_charset(Some(&EMOJI_SET), 0x1F600));
        assert!(in_charset(Some(&EMOJI_SET), 0x41));
        assert!(!in_charset(Some(&EMOJI_SET), 0x3B1));
        assert!(!in_charset(Some(&EMOJI_SET), 0x416));
        assert!(in_charset(None, 0x416));
    }
}
