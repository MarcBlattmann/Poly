//! Windows font names on Poly (ADR 0170).
//!
//! Poly ships DejaVu Sans, DejaVu Serif and DejaVu Sans Mono (ADR 0105).
//! Windows programs ask for Arial, Segoe UI, Times New Roman, Courier New and
//! so on, and expect them to exist with four standard faces; the system
//! collection therefore holds each DejaVu family under its own name and
//! under the Windows names that stand for it: a *family alias* groups the
//! same font files under another family name, with the face names
//! `Regular`, `Bold`, `Italic` and `Bold Italic` Windows families use (in a
//! few languages, as Windows' do). The 78 names are the families of a
//! default Windows 11 installation, so the collection has 81 families with
//! DejaVu's three, like the reference machine.
//!
//! Coverage follows the DejaVu fonts, except that the families other than
//! the emoji and symbol ones do not cover the emoji planes, and the emoji
//! family covers only Latin-1, the euro sign, symbols and the emoji planes.
//! `Arial` also has a `Black` face (Windows' Arial Black is part of the
//! family); it is DejaVu Sans Bold at weight 900.

use crate::face::{STYLE_ITALIC, STYLE_OBLIQUE};
use crate::names::{Localized, utf16};

/// A Windows family name that Poly serves with a DejaVu family.
#[derive(Clone, Copy, Debug)]
pub struct Alias {
    pub name: &'static str,
    /// The DejaVu family whose fonts it uses.
    pub target: &'static str,
    /// `IsSymbolFont` is true for its fonts (Symbol, Wingdings).
    pub symbol: bool,
    pub charset: Option<&'static [(u32, u32)]>,
    /// The family also has a Black face (weight 900, the bold font).
    pub black: bool,
}

pub const SANS: &str = "DejaVu Sans";
pub const SERIF: &str = "DejaVu Serif";
pub const MONO: &str = "DejaVu Sans Mono";

/// Latin-1, the euro sign, symbols, dingbats and the emoji planes.
pub static EMOJI_SET: [(u32, u32); 6] =
    [(0x20, 0x7E), (0xA0, 0xFF), (0x20AC, 0x20AC), (0x2190, 0x2BFF), (0x1F000, 0x1FAFF), (0xFE00, 0xFE0F)];

/// Everything but the emoji planes and what lies beyond.
pub static NO_EMOJI: [(u32, u32); 2] = [(0, 0xFFFF), (0x10000, 0x1EFFF)];

const fn a(name: &'static str, target: &'static str) -> Alias {
    Alias { name, target, symbol: false, charset: Some(&NO_EMOJI), black: false }
}

const fn sym(name: &'static str) -> Alias {
    Alias { name, target: SANS, symbol: true, charset: Some(&NO_EMOJI), black: false }
}

/// Windows 11's default font families, each served by a DejaVu family.
pub static ALIASES: &[Alias] = &[
    Alias { name: "Arial", target: SANS, symbol: false, charset: Some(&NO_EMOJI), black: true },
    a("Arial Unicode MS", SANS),
    a("Bahnschrift", SANS),
    a("Calibri", SANS),
    a("Candara", SANS),
    a("Comic Sans MS", SANS),
    a("Corbel", SANS),
    a("Ebrima", SANS),
    a("Franklin Gothic Medium", SANS),
    a("Gabriola", SANS),
    a("Gadugi", SANS),
    a("HoloLens MDL2 Assets", SANS),
    a("Impact", SANS),
    a("Ink Free", SANS),
    a("Javanese Text", SANS),
    a("Leelawadee UI", SANS),
    a("Lucida Sans Unicode", SANS),
    a("Malgun Gothic", SANS),
    sym("Marlett"),
    a("Microsoft Himalaya", SANS),
    a("Microsoft JhengHei", SANS),
    a("Microsoft JhengHei UI", SANS),
    a("Microsoft New Tai Lue", SANS),
    a("Microsoft PhagsPa", SANS),
    a("Microsoft Sans Serif", SANS),
    a("Microsoft Tai Le", SANS),
    a("Microsoft YaHei", SANS),
    a("Microsoft YaHei UI", SANS),
    a("Microsoft Yi Baiti", SANS),
    a("MS Gothic", SANS),
    a("MS PGothic", SANS),
    a("MS UI Gothic", SANS),
    a("Mongolian Baiti", SANS),
    a("MV Boli", SANS),
    a("Myanmar Text", SANS),
    a("Nirmala UI", SANS),
    a("Segoe Fluent Icons", SANS),
    a("Segoe MDL2 Assets", SANS),
    a("Segoe Print", SANS),
    a("Segoe Script", SANS),
    a("Segoe UI", SANS),
    Alias { name: "Segoe UI Emoji", target: SANS, symbol: false, charset: Some(&EMOJI_SET), black: false },
    a("Segoe UI Historic", SANS),
    Alias { name: "Segoe UI Symbol", target: SANS, symbol: false, charset: None, black: false },
    a("Segoe UI Variable Display", SANS),
    a("Segoe UI Variable Small", SANS),
    a("Segoe UI Variable Text", SANS),
    a("Sitka Banner", SANS),
    a("Sitka Display", SANS),
    a("Sitka Heading", SANS),
    a("Sitka Small", SANS),
    a("Sitka Subheading", SANS),
    a("Sitka Text", SANS),
    sym("Symbol"),
    a("Tahoma", SANS),
    a("Trebuchet MS", SANS),
    a("Verdana", SANS),
    sym("Webdings"),
    sym("Wingdings"),
    a("Yu Gothic", SANS),
    a("Yu Gothic UI", SANS),
    a("Cambria", SERIF),
    a("Cambria Math", SERIF),
    a("Constantia", SERIF),
    a("Georgia", SERIF),
    a("MingLiU", SERIF),
    a("MS Mincho", SERIF),
    a("NSimSun", SERIF),
    a("Palatino Linotype", SERIF),
    a("PMingLiU", SERIF),
    a("SimSun", SERIF),
    a("Sylfaen", SERIF),
    a("Times New Roman", SERIF),
    a("Cascadia Code", MONO),
    a("Cascadia Mono", MONO),
    a("Consolas", MONO),
    a("Courier New", MONO),
    a("Lucida Console", MONO),
];

/// The face name Windows families use for a weight and style, in the
/// languages Windows' fonts carry the standard names in.
pub fn standard_face_name(weight: u32, style: u32) -> Localized {
    let bold = weight >= 600;
    let italic = style == STYLE_ITALIC || style == STYLE_OBLIQUE;
    let k = match (bold, italic) {
        (false, false) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (true, true) => 3,
    };
    const NAMES: [(&str, [&str; 4]); 7] = [
        ("de-de", ["Standard", "Fett", "Kursiv", "Fett Kursiv"]),
        ("en-us", ["Regular", "Bold", "Italic", "Bold Italic"]),
        ("es-es", ["Normal", "Negrita", "Cursiva", "Negrita Cursiva"]),
        ("fr-fr", ["Normal", "Gras", "Italique", "Gras Italique"]),
        ("it-it", ["Normale", "Grassetto", "Corsivo", "Grassetto Corsivo"]),
        ("nl-nl", ["Standaard", "Vet", "Cursief", "Vet cursief"]),
        ("pt-br", ["Normal", "Negrito", "Itálico", "Negrito Itálico"]),
    ];
    let mut l = Localized::default();
    for (loc, names) in NAMES {
        l.insert(loc, &utf16(names[k]));
    }
    l
}

/// Is `cp` inside the ranges (all code points when `None`)?
pub fn in_charset(set: Option<&[(u32, u32)]>, cp: u32) -> bool {
    set.is_none_or(|s| s.iter().any(|&(a, b)| (a..=b).contains(&cp)))
}

/// The code points in both sets of ranges (`a` sorted and merged).
pub fn intersect(a: &[(u32, u32)], b: &[(u32, u32)]) -> alloc::vec::Vec<(u32, u32)> {
    let mut out = alloc::vec::Vec::new();
    for &(a0, a1) in a {
        for &(b0, b1) in b {
            let (lo, hi) = (a0.max(b0), a1.min(b1));
            if lo <= hi {
                out.push((lo, hi));
            }
        }
    }
    out.sort_unstable();
    out
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
        assert_eq!(standard_face_name(400, 0).primary(), Localized::en_us("Regular").primary());
        assert_eq!(standard_face_name(700, 2).primary(), Localized::en_us("Bold Italic").primary());
        assert!(standard_face_name(400, 0).entries.len() > 1);
    }

    #[test]
    fn emoji_charset() {
        assert!(in_charset(Some(&EMOJI_SET), 0x1F600));
        assert!(in_charset(Some(&EMOJI_SET), 0x41));
        assert!(!in_charset(Some(&EMOJI_SET), 0x3B1));
        assert!(!in_charset(Some(&EMOJI_SET), 0x416));
        assert!(in_charset(None, 0x416));
        assert!(!in_charset(Some(&NO_EMOJI), 0x1F600));
        assert!(in_charset(Some(&NO_EMOJI), 0x3B1));
    }

    #[test]
    fn intersecting_ranges() {
        let got = intersect(&[(0, 0x7F), (0x400, 0x4FF)], &[(0x20, 0x7E), (0x450, 0x460)]);
        assert_eq!(got, [(0x20, 0x7E), (0x450, 0x460)]);
    }

    #[test]
    fn seventy_eight_names_each_once() {
        assert_eq!(ALIASES.len(), 78);
        for (i, x) in ALIASES.iter().enumerate() {
            assert!(ALIASES[i + 1..].iter().all(|y| y.name != x.name), "{}", x.name);
        }
    }
}
