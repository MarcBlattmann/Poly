//! Object names: UTF-16 code units, `\` separated, compared with or without
//! case.
//!
//! # Case folding (to be confirmed by the spec agent against real Windows)
//!
//! Windows compares names case-insensitively by upcasing each UTF-16 code
//! unit through a system upcase table. That table is not public, so
//! [`upcase`] uses this rule instead:
//!
//! - ASCII `a`–`z` map to `A`–`Z`;
//! - any other code unit outside the surrogate range is treated as a
//!   Basic Multilingual Plane character and mapped to its Unicode uppercase
//!   (`char::to_uppercase`, Unicode's full mapping as shipped with Rust's
//!   `core`) **only when that mapping is a single BMP character**; a mapping
//!   that expands (`ß` → `SS`, `ŉ` → `ʼN`, Greek letters with
//!   ypogegrammeni) leaves the code unit unchanged;
//! - surrogate code units (U+D800–U+DFFF) are never changed, so characters
//!   outside the BMP compare exactly.
//!
//! Known places where this may differ from Windows and that the spec agent
//! should check: `ı` (U+0131) and `ſ` (U+017F), which this rule maps to `I`
//! and `S`; `ÿ` (U+00FF) → `Ÿ` (U+0178); characters whose Unicode *simple*
//! mapping differs from the single-character full mapping; and characters
//! added to Unicode after the version Windows' table was built from.

use alloc::vec::Vec;

/// `\`, the separator between name components.
pub const SEPARATOR: u16 = b'\\' as u16;

/// Encode a Rust string as UTF-16, for names built in code and in tests.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Upcase one UTF-16 code unit (see the module documentation for the rule).
pub fn upcase(c: u16) -> u16 {
    if c < 0x80 {
        return (c as u8).to_ascii_uppercase() as u16;
    }
    let Some(ch) = char::from_u32(c as u32) else { return c };
    let mut up = ch.to_uppercase();
    match (up.next(), up.next()) {
        (Some(u), None) if (u as u32) <= 0xFFFF => u as u16,
        _ => c,
    }
}

/// Compare two names ignoring case, by [`upcase`].
pub fn eq_ignore_case(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| x == y || upcase(x) == upcase(y))
}

/// A name with every code unit upcased: the key directories sort by.
pub(crate) fn fold(name: &[u16]) -> Vec<u16> {
    name.iter().map(|&c| upcase(c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_latin1() {
        assert_eq!(upcase(b'a' as u16), b'A' as u16);
        assert_eq!(upcase(b'Z' as u16), b'Z' as u16);
        assert_eq!(upcase(b'\\' as u16), b'\\' as u16);
        assert_eq!(upcase(0xE9), 0xC9); // é → É
        assert_eq!(upcase(0xFF), 0x178); // ÿ → Ÿ
        assert_eq!(upcase(0xDF), 0xDF); // ß expands to "SS": unchanged
    }

    #[test]
    fn other_scripts() {
        assert_eq!(upcase(0x3B1), 0x391); // α → Α
        assert_eq!(upcase(0x436), 0x416); // ж → Ж
        assert_eq!(upcase(0xFF41), 0xFF21); // fullwidth a → A
        assert_eq!(upcase(0x4E2D), 0x4E2D); // 中 has no case
        assert_eq!(upcase(0x149), 0x149); // ŉ expands: unchanged
    }

    #[test]
    fn surrogates_are_left_alone() {
        // U+10428 DESERET SMALL LETTER LONG I has an uppercase (U+10400),
        // but outside the BMP names compare exactly.
        let small = wide("\u{10428}");
        let capital = wide("\u{10400}");
        assert_eq!(fold(&small), small);
        assert!(!eq_ignore_case(&small, &capital));
        assert_eq!(upcase(0xD800), 0xD800);
    }

    #[test]
    fn comparisons() {
        assert!(eq_ignore_case(&wide("BaseNamedObjects"), &wide("BASENAMEDOBJECTS")));
        assert!(eq_ignore_case(&wide("Événement"), &wide("éVÉNEMENT")));
        assert!(!eq_ignore_case(&wide("abc"), &wide("abcd")));
        assert!(!eq_ignore_case(&wide("abc"), &wide("abd")));
        assert_eq!(fold(&wide("a\\b")), wide("A\\B"));
    }
}
