//! How many cells a character takes: 0, 1 or 2.
//!
//! A compact approximation of Unicode's East Asian Width property
//! (UAX #11, `EastAsianWidth.txt`) and of the emoji that are shown wide
//! (`Emoji_Presentation`), kept as two sorted range tables rather than the
//! full data:
//!
//! - **2**: Hangul Jamo leading consonants, CJK radicals, kana, CJK
//!   ideographs (BMP and the supplementary planes 2 and 3), Hangul
//!   syllables, Yi, compatibility ideographs, fullwidth forms, and the
//!   emoji blocks that default to emoji presentation.
//! - **0**: combining marks (general category Mn) of the common scripts
//!   (Latin/Greek/Cyrillic diacritics, Hebrew, Arabic, Devanagari, Thai,
//!   the combining-mark blocks), zero-width spaces and joiners, bidi
//!   controls, variation selectors, emoji skin-tone modifiers and tag
//!   characters, and the C0/C1 control code points.
//! - **1**: everything else, including ambiguous-width characters (as
//!   xterm does outside CJK locales).
//!
//! The terminal drops zero-width characters instead of attaching them to
//! the previous cell (a cell holds one `char`), so a decomposed "e" +
//! U+0301 shows as a plain "e". Programs on Poly use precomposed text, as
//! nearly all UTF-8 text is.

/// Ranges (inclusive) of wide characters, sorted.
const WIDE: &[(u32, u32)] = &[
    (0x1100, 0x115F),
    (0x231A, 0x231B),
    (0x2329, 0x232A),
    (0x23E9, 0x23EC),
    (0x23F0, 0x23F0),
    (0x23F3, 0x23F3),
    (0x25FD, 0x25FE),
    (0x2614, 0x2615),
    (0x2648, 0x2653),
    (0x267F, 0x267F),
    (0x2693, 0x2693),
    (0x26A1, 0x26A1),
    (0x26AA, 0x26AB),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CE),
    (0x26D4, 0x26D4),
    (0x26EA, 0x26EA),
    (0x26F2, 0x26F3),
    (0x26F5, 0x26F5),
    (0x26FA, 0x26FA),
    (0x26FD, 0x26FD),
    (0x2705, 0x2705),
    (0x270A, 0x270B),
    (0x2728, 0x2728),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x2E80, 0x303E),
    (0x3041, 0x3247),
    (0x3250, 0x4DBF),
    (0x4E00, 0xA4CF),
    (0xA960, 0xA97F),
    (0xAC00, 0xD7A3),
    (0xF900, 0xFAFF),
    (0xFE10, 0xFE19),
    (0xFE30, 0xFE6F),
    (0xFF00, 0xFF60),
    (0xFFE0, 0xFFE6),
    (0x16FE0, 0x16FE4),
    (0x17000, 0x18CFF),
    (0x1B000, 0x1B2FF),
    (0x1F004, 0x1F004),
    (0x1F0CF, 0x1F0CF),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F200, 0x1F202),
    (0x1F210, 0x1F23B),
    (0x1F240, 0x1F248),
    (0x1F250, 0x1F251),
    (0x1F260, 0x1F265),
    (0x1F300, 0x1F320),
    (0x1F32D, 0x1F335),
    (0x1F337, 0x1F37C),
    (0x1F37E, 0x1F393),
    (0x1F3A0, 0x1F3CA),
    (0x1F3CF, 0x1F3D3),
    (0x1F3E0, 0x1F3F0),
    (0x1F3F4, 0x1F3F4),
    (0x1F3F8, 0x1F3FA),
    (0x1F400, 0x1F43E),
    (0x1F440, 0x1F440),
    (0x1F442, 0x1F4FC),
    (0x1F4FF, 0x1F53D),
    (0x1F54B, 0x1F54E),
    (0x1F550, 0x1F567),
    (0x1F57A, 0x1F57A),
    (0x1F595, 0x1F596),
    (0x1F5A4, 0x1F5A4),
    (0x1F5FB, 0x1F64F),
    (0x1F680, 0x1F6C5),
    (0x1F6CC, 0x1F6CC),
    (0x1F6D0, 0x1F6D2),
    (0x1F6D5, 0x1F6D7),
    (0x1F6DC, 0x1F6DF),
    (0x1F6EB, 0x1F6EC),
    (0x1F6F4, 0x1F6FC),
    (0x1F7E0, 0x1F7EB),
    (0x1F7F0, 0x1F7F0),
    (0x1F90C, 0x1F93A),
    (0x1F93C, 0x1F945),
    (0x1F947, 0x1F9FF),
    (0x1FA70, 0x1FAFF),
    (0x20000, 0x2FFFD),
    (0x30000, 0x3FFFD),
];

/// Ranges (inclusive) of zero-width characters, sorted.
const ZERO: &[(u32, u32)] = &[
    (0x0000, 0x001F),
    (0x007F, 0x009F),
    (0x00AD, 0x00AD),
    (0x0300, 0x036F),
    (0x0483, 0x0489),
    (0x0591, 0x05BD),
    (0x05BF, 0x05BF),
    (0x05C1, 0x05C2),
    (0x05C4, 0x05C5),
    (0x05C7, 0x05C7),
    (0x0610, 0x061A),
    (0x064B, 0x065F),
    (0x0670, 0x0670),
    (0x06D6, 0x06DC),
    (0x06DF, 0x06E4),
    (0x06E7, 0x06E8),
    (0x06EA, 0x06ED),
    (0x0711, 0x0711),
    (0x0730, 0x074A),
    (0x0900, 0x0902),
    (0x093A, 0x093A),
    (0x093C, 0x093C),
    (0x0941, 0x0948),
    (0x094D, 0x094D),
    (0x0951, 0x0957),
    (0x0962, 0x0963),
    (0x0E31, 0x0E31),
    (0x0E34, 0x0E3A),
    (0x0E47, 0x0E4E),
    (0x1160, 0x11FF),
    (0x1AB0, 0x1AFF),
    (0x1DC0, 0x1DFF),
    (0x200B, 0x200F),
    (0x2028, 0x202E),
    (0x2060, 0x2064),
    (0x20D0, 0x20FF),
    (0x302A, 0x302D),
    (0x3099, 0x309A),
    (0xFE00, 0xFE0F),
    (0xFE20, 0xFE2F),
    (0xFEFF, 0xFEFF),
    (0x1F3FB, 0x1F3FF),
    (0xE0000, 0xE0FFF),
];

fn in_table(table: &[(u32, u32)], c: u32) -> bool {
    table
        .binary_search_by(|&(lo, hi)| {
            if hi < c {
                core::cmp::Ordering::Less
            } else if lo > c {
                core::cmp::Ordering::Greater
            } else {
                core::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// The number of cells `c` takes on the screen.
pub fn char_width(c: char) -> u8 {
    let c = c as u32;
    if (0x20..0x7f).contains(&c) {
        return 1;
    }
    if in_table(ZERO, c) {
        0
    } else if in_table(WIDE, c) {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_and_disjoint() {
        for t in [WIDE, ZERO] {
            for w in t.windows(2) {
                assert!(w[0].0 <= w[0].1 && w[0].1 < w[1].0, "{w:x?}");
            }
        }
    }

    #[test]
    fn widths() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('é'), 1);
        assert_eq!(char_width('─'), 1);
        assert_eq!(char_width('中'), 2);
        assert_eq!(char_width('あ'), 2);
        assert_eq!(char_width('한'), 2);
        assert_eq!(char_width('Ａ'), 2);
        assert_eq!(char_width('😀'), 2);
        assert_eq!(char_width('🚀'), 2);
        assert_eq!(char_width('\u{0301}'), 0);
        assert_eq!(char_width('\u{200D}'), 0);
        assert_eq!(char_width('\u{FE0F}'), 0);
        assert_eq!(char_width('\u{1F3FB}'), 0);
        assert_eq!(char_width('\u{9B}'), 0);
        assert_eq!(char_width('\u{10FFFF}'), 1);
    }
}
