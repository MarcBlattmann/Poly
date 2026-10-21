//! Character classes and case in the "C" locale (C11 7.4, 7.30), as the
//! `_pctype` table MSVC's `ctype.h` macros index (`__pctype_func()[c]` for
//! `c` in −1 (`EOF`) … 255), with `ctype.h`'s bits.
//!
//! The "C" locale's narrow classes are ASCII's; bytes 0x80–0xFF have none.
//! Wide characters (`_wctype`, `iswalpha` …) are classified by Unicode
//! for U+0000–U+00FF (Latin-1: letters, the no-break space, controls,
//! punctuation); characters above U+00FF have no class (Poly's choice, to
//! be confirmed by the spec agent: Windows classifies them with
//! `GetStringTypeW`). `toupper`/`tolower` map ASCII letters,
//! `towupper`/`towlower` ASCII and Latin-1 letters.

/// `ctype.h`.
pub const UPPER: u16 = 0x1;
pub const LOWER: u16 = 0x2;
pub const DIGIT: u16 = 0x4;
pub const SPACE: u16 = 0x8;
pub const PUNCT: u16 = 0x10;
pub const CONTROL: u16 = 0x20;
pub const BLANK: u16 = 0x40;
pub const HEX: u16 = 0x80;
pub const LEADBYTE: u16 = 0x8000;
pub const ALPHA: u16 = 0x0100 | UPPER | LOWER;

/// The class bits of an ASCII byte.
const fn ascii_class(c: u8) -> u16 {
    match c {
        b'\t'..=b'\r' => CONTROL | SPACE,
        0x00..=0x1F | 0x7F => CONTROL,
        b' ' => SPACE | BLANK,
        b'0'..=b'9' => DIGIT | HEX,
        b'A'..=b'F' => UPPER | HEX | 0x100,
        b'G'..=b'Z' => UPPER | 0x100,
        b'a'..=b'f' => LOWER | HEX | 0x100,
        b'g'..=b'z' => LOWER | 0x100,
        0x21..=0x7E => PUNCT,
        _ => 0,
    }
}

/// The class bits of a Latin-1 character (U+0000–U+00FF).
const fn latin1_class(c: u8) -> u16 {
    match c {
        0x00..=0x7F => ascii_class(c),
        0x80..=0x9F => CONTROL,
        0xA0 => SPACE | BLANK,
        0xAA | 0xB5 | 0xBA | 0xDF => LOWER | 0x100,
        0xC0..=0xD6 | 0xD8..=0xDE => UPPER | 0x100,
        0xE0..=0xF6 | 0xF8..=0xFF => LOWER | 0x100,
        _ => PUNCT,
    }
}

const fn table(wide: bool) -> [u16; 257] {
    let mut t = [0u16; 257];
    let mut c = 0;
    while c < 256 {
        t[c + 1] = if wide {
            latin1_class(c as u8)
        } else if c < 0x80 {
            ascii_class(c as u8)
        } else {
            0
        };
        c += 1;
    }
    t
}

/// The narrow table, entry 0 for `EOF` (`_pctype` points at entry 1).
pub const NARROW_TABLE: [u16; 257] = table(false);
/// The wide table for U+0000–U+00FF, entry 0 for `WEOF`'s place.
pub const WIDE_TABLE: [u16; 257] = table(true);
pub static NARROW: [u16; 257] = NARROW_TABLE;
pub static WIDE: [u16; 257] = WIDE_TABLE;

/// `_isctype(c, mask)` in the "C" locale: `c` is `EOF` or an `unsigned
/// char` value; anything else has no class (MSVC's debug CRT asserts; Poly
/// answers 0).
pub fn class(c: i32) -> u16 {
    match c {
        -1..=255 => NARROW[(c + 1) as usize],
        _ => 0,
    }
}

/// `iswctype(c, mask)`'s class bits.
pub fn wide_class(c: u32) -> u16 {
    if c < 256 { WIDE[c as usize + 1] } else { 0 }
}

/// White space as `strtol`/`strtod` skip it (`isspace` / `iswspace`).
pub fn is_space_unit(c: u32, wide: bool) -> bool {
    let bits = if wide {
        wide_class(c)
    } else if c < 256 {
        class(c as i32)
    } else {
        0
    };
    bits & SPACE != 0
}

/// `toupper` in the "C" locale.
pub fn to_upper(c: i32) -> i32 {
    if (i32::from(b'a')..=i32::from(b'z')).contains(&c) { c - 32 } else { c }
}

/// `tolower` in the "C" locale.
pub fn to_lower(c: i32) -> i32 {
    if (i32::from(b'A')..=i32::from(b'Z')).contains(&c) { c + 32 } else { c }
}

/// `towupper`.
pub fn to_upper_wide(c: u16) -> u16 {
    match c {
        0x61..=0x7A | 0xE0..=0xF6 | 0xF8..=0xFE => c - 32,
        0xFF => 0x178,
        _ => c,
    }
}

/// `towlower`.
pub fn to_lower_wide(c: u16) -> u16 {
    match c {
        0x41..=0x5A | 0xC0..=0xD6 | 0xD8..=0xDE => c + 32,
        0x178 => 0xFF,
        _ => c,
    }
}

/// `isalpha` … in terms of [`class`] and the bits; `isblank` (C99) and
/// `isgraph`/`isprint` derive from them as `ctype.h` defines.
pub fn is(c: i32, mask: u16) -> bool {
    class(c) & mask != 0
}

/// `isprint`: `_BLANK | _PUNCT | _ALPHA | _DIGIT` (`ctype.h`).
pub const PRINT: u16 = BLANK | PUNCT | ALPHA | DIGIT;
/// `isgraph`: `_PUNCT | _ALPHA | _DIGIT`.
pub const GRAPH: u16 = PUNCT | ALPHA | DIGIT;
/// `isalnum`: `_ALPHA | _DIGIT`.
pub const ALNUM: u16 = ALPHA | DIGIT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_c_locale_table() {
        assert_eq!(class(-1), 0);
        assert_eq!(class(i32::from(b'A')), 0x181);
        assert_eq!(class(i32::from(b'z')), 0x102);
        assert_eq!(class(i32::from(b'7')), 0x84);
        assert_eq!(class(i32::from(b' ')), 0x48);
        assert_eq!(class(i32::from(b'\t')), 0x28);
        assert_eq!(class(i32::from(b'\n')), 0x28);
        assert_eq!(class(i32::from(b'!')), PUNCT);
        assert_eq!(class(0x7F), CONTROL);
        assert_eq!(class(0xE9), 0);
        assert_eq!(class(256), 0);
        assert!(is(i32::from(b'q'), ALPHA) && !is(i32::from(b'1'), ALPHA));
        assert!(is(i32::from(b' '), PRINT) && !is(i32::from(b' '), GRAPH) && !is(i32::from(b'\t'), PRINT));
        assert_eq!(NARROW.len(), 257);
    }

    #[test]
    fn wide_classes_and_case() {
        assert_eq!(wide_class(0xE9), LOWER | 0x100);
        assert_eq!(wide_class(0xC9), UPPER | 0x100);
        assert_eq!(wide_class(0xA0), SPACE | BLANK);
        assert_eq!(wide_class(0xD7), PUNCT);
        assert_eq!(wide_class(0x85), CONTROL);
        assert_eq!(wide_class(0x3B1), 0);
        assert_eq!(to_upper_wide(0xE9), 0xC9);
        assert_eq!(to_lower_wide(0xC9), 0xE9);
        assert_eq!(to_upper_wide(0xF7), 0xF7);
        assert_eq!(to_upper_wide(0xFF), 0x178);
        assert_eq!(to_upper(i32::from(b'a')), i32::from(b'A'));
        assert_eq!(to_upper(0xE9), 0xE9);
        assert_eq!(to_lower(i32::from(b'Q')), i32::from(b'q'));
        assert_eq!(to_lower(-1), -1);
        assert!(is_space_unit(0x0B, false) && is_space_unit(0xA0, true) && !is_space_unit(0xA0, false));
    }
}
