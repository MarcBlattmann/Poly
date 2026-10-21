//! Poly's ANSI code page: Windows-1252, the default ANSI code page of
//! Western-European and US English Windows (Learn: "Code Page
//! Identifiers", 1252 "ANSI Latin 1; Western European (Windows)").
//!
//! The mapping is the public one (Unicode Consortium's `CP1252.TXT`,
//! Microsoft's "Windows-1252" code chart): bytes 0x00-0x7F and 0xA0-0xFF are
//! the same code points; 0x80-0x9F hold the characters below. The five
//! bytes that chart leaves undefined (0x81, 0x8D, 0x8F, 0x90, 0x9D) map to
//! the C1 control of the same value, and a character the code page cannot
//! hold becomes `?` (0x3F), the code page's default character; whether
//! Windows' `Rtl*` conversions use a best-fit mapping instead is for the
//! spec agent to confirm (ADR 0071).

/// Code points of bytes 0x80-0x9F.
const HIGH: [u16; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, // 0x80
    0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D, 0x017D, 0x008F, // 0x88
    0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, // 0x90
    0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178, // 0x98
];

/// The code page's default character.
pub const DEFAULT_CHAR: u8 = b'?';

/// The code page Poly reports as the ANSI code page (`GetACP`).
pub const ANSI_CODE_PAGE: u32 = 1252;

/// One byte to UTF-16.
pub fn to_unicode(b: u8) -> u16 {
    match b {
        0x80..=0x9F => HIGH[usize::from(b - 0x80)],
        _ => u16::from(b),
    }
}

/// One UTF-16 code unit to a byte, or [`DEFAULT_CHAR`].
pub fn from_unicode(c: u16) -> u8 {
    match c {
        0x00..=0x7F | 0xA0..=0xFF => c as u8,
        _ => HIGH.iter().position(|&h| h == c).map_or(DEFAULT_CHAR, |i| 0x80 + i as u8),
    }
}

/// Upcase one byte of the code page (`RtlUpperChar`): through UTF-16 and
/// the object manager's upcase rule, kept only if the result is a byte of
/// the code page again.
pub fn upcase(b: u8) -> u8 {
    let up = poly_ntobj::name::upcase(to_unicode(b));
    let back = from_unicode(up);
    if back == DEFAULT_CHAR && up != u16::from(DEFAULT_CHAR) { b } else { back }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_round_trips() {
        for b in 0..=255u8 {
            assert_eq!(from_unicode(to_unicode(b)), b, "{b:#x}");
        }
    }

    #[test]
    fn the_high_range_and_the_default_character() {
        assert_eq!(to_unicode(0x80), 0x20AC);
        assert_eq!(to_unicode(0x9F), 0x0178);
        assert_eq!(to_unicode(0xE9), 0xE9);
        assert_eq!(from_unicode(0x20AC), 0x80);
        assert_eq!(from_unicode(0x4E2D), b'?');
        assert_eq!(from_unicode(0xD800), b'?');
    }

    #[test]
    fn upcase_stays_in_the_code_page() {
        assert_eq!(upcase(b'a'), b'A');
        assert_eq!(upcase(0xE9), 0xC9); // é → É
        assert_eq!(upcase(0x9A), 0x8A); // š → Š
        assert_eq!(upcase(0xFF), 0x9F); // ÿ → Ÿ, which is 0x9F here
        assert_eq!(upcase(0xB5), 0xB5); // µ → Μ (Greek) is not in 1252: kept
        assert_eq!(upcase(b'?'), b'?');
    }
}
