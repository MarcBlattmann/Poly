//! Character classes and simple case mapping of UTF-16 units, as the C
//! runtime's wide classification (`iswalpha`, `_wctype`) and a locale's
//! narrow tables need them (ADR 0094).
//!
//! The classes are `ctype.h`'s bits (`_UPPER` … `_ALPHA`), assigned from
//! the Unicode Character Database's general categories (public data) by
//! block: letters are `_ALPHA` (with `_UPPER`/`_LOWER` where the letter
//! has case), decimal digits `_DIGIT`, space separators `_SPACE | _BLANK`,
//! controls `_CONTROL`, and other graphic characters (punctuation,
//! symbols, marks) `_PUNCT`. Poly covers the scripts of its code pages:
//! Latin (with Latin Extended-A/B and Additional), IPA, Greek, Cyrillic,
//! Armenian, Hebrew, Arabic, Thai, general punctuation and symbols, CJK
//! (ideographs, kana, Hangul, Bopomofo) and the halfwidth and fullwidth
//! forms. Other characters have no class (Poly's choice; Windows answers
//! from `GetStringTypeW`'s full tables: to be confirmed by the spec agent).
//!
//! Case: ASCII, Latin-1 (ÿ ↔ Ÿ), Latin Extended-A, Latin Extended
//! Additional, Greek, Cyrillic, Armenian, fullwidth Latin; one unit to one
//! unit (no `ß` → `SS`). `ı` uppercases to `I` and `İ` lowercases to `i`
//! (the default, non-Turkish mapping).

/// `ctype.h`'s class bits.
pub const UPPER: u16 = 0x1;
pub const LOWER: u16 = 0x2;
pub const DIGIT: u16 = 0x4;
pub const SPACE: u16 = 0x8;
pub const PUNCT: u16 = 0x10;
pub const CONTROL: u16 = 0x20;
pub const BLANK: u16 = 0x40;
pub const HEX: u16 = 0x80;
/// `_ALPHA` without the case bits (`ctype.h`'s `_ALPHA` is `0x0100 |
/// _UPPER | _LOWER`).
pub const LETTER: u16 = 0x100;

const UP: u16 = LETTER | UPPER;
const LO: u16 = LETTER | LOWER;

/// The class bits of an ASCII character.
const fn ascii(c: u16) -> u16 {
    match c {
        0x09..=0x0D => CONTROL | SPACE,
        0x00..=0x1F | 0x7F => CONTROL,
        0x20 => SPACE | BLANK,
        0x30..=0x39 => DIGIT | HEX,
        0x41..=0x46 => UP | HEX,
        0x47..=0x5A => UP,
        0x61..=0x66 => LO | HEX,
        0x67..=0x7A => LO,
        _ => PUNCT,
    }
}

/// Upper (even) / lower (odd) alternation from `start`.
fn alternating(c: u16, start: u16) -> u16 {
    if (c - start).is_multiple_of(2) { UP } else { LO }
}

/// The class bits of a UTF-16 unit.
pub fn class(c: u16) -> u16 {
    match c {
        0x0000..=0x007F => ascii(c),
        0x0080..=0x009F => CONTROL,
        0x00A0 => SPACE | BLANK,
        0x00AA | 0x00B5 | 0x00BA | 0x00DF => LO,
        0x00C0..=0x00D6 | 0x00D8..=0x00DE => UP,
        0x00E0..=0x00F6 | 0x00F8..=0x00FF => LO,
        0x00A1..=0x00BF | 0x00D7 | 0x00F7 => PUNCT,
        // Latin Extended-A.
        0x0130 => UP,
        0x0131 | 0x0138 | 0x0149 | 0x017F => LO,
        0x0139..=0x0148 | 0x0179..=0x017E => {
            if c % 2 == 1 {
                UP
            } else {
                LO
            }
        }
        0x0178 => UP,
        0x0100..=0x017F => alternating(c, 0x0100),
        // Latin Extended-B, IPA: letters, mostly without simple case here.
        0x0180..=0x024F => LETTER,
        0x0250..=0x02AF => LO,
        0x02B0..=0x036F => PUNCT,
        // Greek.
        0x0386 | 0x0388..=0x038A | 0x038C | 0x038E | 0x038F | 0x0391..=0x03A1 | 0x03A3..=0x03AB => UP,
        0x0390 | 0x03AC..=0x03CE => LO,
        0x0370..=0x03FF => LETTER,
        // Cyrillic.
        0x0400..=0x042F => UP,
        0x0430..=0x045F => LO,
        0x0460..=0x0481 | 0x048A..=0x04BF | 0x04D0..=0x04FF => alternating(c, 0x0460),
        0x04C0 => UP,
        0x04C1..=0x04CE => {
            if c % 2 == 1 {
                UP
            } else {
                LO
            }
        }
        0x04CF => LO,
        0x0482..=0x0489 => PUNCT,
        0x0500..=0x052F => alternating(c, 0x0500),
        // Armenian.
        0x0531..=0x0556 => UP,
        0x0561..=0x0587 => LO,
        0x0559..=0x055F | 0x0589 | 0x058A => PUNCT,
        // Hebrew.
        0x0591..=0x05C7 => PUNCT,
        0x05D0..=0x05EA | 0x05F0..=0x05F2 => LETTER,
        0x05F3 | 0x05F4 => PUNCT,
        // Arabic.
        0x0660..=0x0669 | 0x06F0..=0x06F9 => DIGIT,
        0x0621..=0x063A | 0x0641..=0x064A | 0x0671..=0x06D3 | 0x06D5 => LETTER,
        0x0600..=0x06FF => PUNCT,
        // Thai.
        0x0E01..=0x0E30 | 0x0E32 | 0x0E33 | 0x0E40..=0x0E46 => LETTER,
        0x0E50..=0x0E59 => DIGIT,
        0x0E31 | 0x0E34..=0x0E3A | 0x0E3F | 0x0E47..=0x0E4F | 0x0E5A | 0x0E5B => PUNCT,
        // Latin Extended Additional, Greek Extended.
        0x1E96..=0x1E9D | 0x1E9F => LO,
        0x1E9E => UP,
        0x1E00..=0x1EFF => alternating(c, 0x1E00),
        0x1F00..=0x1FFF => LETTER,
        // General punctuation, symbols.
        0x2000..=0x200A | 0x202F | 0x205F => SPACE | BLANK,
        0x2028 | 0x2029 => SPACE,
        0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F => CONTROL,
        0x2010..=0x2027 | 0x2030..=0x205E => PUNCT,
        0x2070..=0x2BFF => PUNCT,
        // CJK.
        0x3000 => SPACE | BLANK,
        0x3001..=0x3004 | 0x3008..=0x3020 | 0x3030 | 0x303D..=0x303F | 0x309B | 0x309C | 0x30A0 | 0x30FB => PUNCT,
        0x3005..=0x3007 | 0x3021..=0x3029 | 0x3031..=0x3035 | 0x3038..=0x303C => LETTER,
        0x3041..=0x3096 | 0x309D..=0x309F | 0x30A1..=0x30FA | 0x30FC..=0x30FF => LETTER,
        0x3105..=0x312F | 0x3131..=0x318E | 0x31A0..=0x31BF | 0x31F0..=0x31FF => LETTER,
        0x3200..=0x33FF => PUNCT,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF => LETTER,
        // Halfwidth and fullwidth forms.
        0xFF10..=0xFF19 => DIGIT | HEX,
        0xFF21..=0xFF26 => UP | HEX,
        0xFF27..=0xFF3A => UP,
        0xFF41..=0xFF46 => LO | HEX,
        0xFF47..=0xFF5A => LO,
        0xFF01..=0xFF0F | 0xFF1A..=0xFF20 | 0xFF3B..=0xFF40 | 0xFF5B..=0xFF65 => PUNCT,
        0xFF66..=0xFFDC => LETTER,
        0xFFE0..=0xFFEE => PUNCT,
        0xFEFF => CONTROL,
        _ => 0,
    }
}

/// `towupper` (simple, one unit to one unit).
pub fn to_upper(c: u16) -> u16 {
    match c {
        0x0061..=0x007A | 0x00E0..=0x00F6 | 0x00F8..=0x00FE => c - 0x20,
        0x00FF => 0x0178,
        0x0131 => 0x0049,
        0x0139..=0x0148 | 0x0179..=0x017E if c.is_multiple_of(2) => c - 1,
        0x0100..=0x0137 | 0x014A..=0x0177 if c % 2 == 1 => c - 1,
        0x017F => 0x0053,
        0x03AC => 0x0386,
        0x03AD..=0x03AF => c - 0x25,
        0x03B1..=0x03C1 | 0x03C3..=0x03CB => c - 0x20,
        0x03C2 => 0x03A3,
        0x03CC => 0x038C,
        0x03CD | 0x03CE => c - 0x3F,
        0x0430..=0x044F => c - 0x20,
        0x0450..=0x045F => c - 0x50,
        0x0460..=0x0481 | 0x048A..=0x04BF | 0x04D0..=0x052F if c % 2 == 1 => c - 1,
        0x04C1..=0x04CE if c.is_multiple_of(2) => c - 1,
        0x04CF => 0x04C0,
        0x0561..=0x0586 => c - 0x30,
        0x1E00..=0x1E95 | 0x1EA0..=0x1EFF if c % 2 == 1 => c - 1,
        0xFF41..=0xFF5A => c - 0x20,
        _ => c,
    }
}

/// `towlower` (simple).
pub fn to_lower(c: u16) -> u16 {
    match c {
        0x0041..=0x005A | 0x00C0..=0x00D6 | 0x00D8..=0x00DE => c + 0x20,
        0x0178 => 0x00FF,
        0x0130 => 0x0069,
        0x0139..=0x0148 | 0x0179..=0x017E if c % 2 == 1 => c + 1,
        0x0100..=0x0137 | 0x014A..=0x0177 if c.is_multiple_of(2) => c + 1,
        0x0386 => 0x03AC,
        0x0388..=0x038A => c + 0x25,
        0x038C => 0x03CC,
        0x038E | 0x038F => c + 0x3F,
        0x0391..=0x03A1 | 0x03A3..=0x03AB => c + 0x20,
        0x0400..=0x040F => c + 0x50,
        0x0410..=0x042F => c + 0x20,
        0x0460..=0x0481 | 0x048A..=0x04BF | 0x04D0..=0x052F if c.is_multiple_of(2) => c + 1,
        0x04C1..=0x04CE if c % 2 == 1 => c + 1,
        0x04C0 => 0x04CF,
        0x0531..=0x0556 => c + 0x30,
        0x1E00..=0x1E95 | 0x1EA0..=0x1EFF if c.is_multiple_of(2) => c + 1,
        0xFF21..=0xFF3A => c + 0x20,
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes() {
        assert_eq!(class(u16::from(b'A')), UP | HEX);
        assert_eq!(class(u16::from(b' ')), SPACE | BLANK);
        assert_eq!(class(0xE9), LO);
        assert_eq!(class(0x0141), UP); // Ł
        assert_eq!(class(0x0142), LO); // ł
        assert_eq!(class(0x0160), UP); // Š
        assert_eq!(class(0x03A9), UP); // Ω
        assert_eq!(class(0x03C9), LO); // ω
        assert_eq!(class(0x0416), UP); // Ж
        assert_eq!(class(0x0436), LO); // ж
        assert_eq!(class(0x05D0), LETTER); // א
        assert_eq!(class(0x0663), DIGIT);
        assert_eq!(class(0x3042), LETTER); // あ
        assert_eq!(class(0x4E2D), LETTER); // 中
        assert_eq!(class(0xAC00), LETTER); // 가
        assert_eq!(class(0x3000), SPACE | BLANK);
        assert_eq!(class(0x3002), PUNCT); // 。
        assert_eq!(class(0xFF21), UP | HEX); // Ａ
        assert_eq!(class(0xFF15), DIGIT | HEX); // ５
        assert_eq!(class(0x20AC), PUNCT); // €
        assert_eq!(class(0xE000), 0);
    }

    #[test]
    fn case_round_trips() {
        for c in 0..=0xFFFFu16 {
            let up = to_upper(c);
            let lo = to_lower(c);
            if up != c && c != 0x0131 && c != 0x017F && c != 0x03C2 {
                assert_eq!(to_lower(up), c, "{c:#x} → {up:#x}");
                assert!(class(c) & LOWER != 0, "{c:#x} has an upper case but is not lower case");
            }
            if lo != c && c != 0x0130 {
                assert_eq!(to_upper(lo), c, "{c:#x} → {lo:#x}");
                assert!(class(c) & UPPER != 0, "{c:#x} has a lower case but is not upper case");
            }
        }
        assert_eq!(to_upper(0x00E9), 0x00C9);
        assert_eq!(to_upper(0x0142), 0x0141);
        assert_eq!(to_upper(0x017E), 0x017D);
        assert_eq!(to_upper(0x03C2), 0x03A3);
        assert_eq!(to_upper(0x0451), 0x0401);
        assert_eq!(to_lower(0x0416), 0x0436);
        assert_eq!(to_lower(0xFF21), 0xFF41);
        assert_eq!(to_upper(0x00DF), 0x00DF);
        assert_eq!(to_upper(0x0131), u16::from(b'I'));
    }
}
