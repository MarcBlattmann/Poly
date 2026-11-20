//! The C locale's character classes as the macOS SDK's `<ctype.h>` reads
//! them: programs compiled against the SDK test `isalpha(c)` and friends
//! inline, by reading `_DefaultRuneLocale.__runetype[c]` and masking it
//! with the `_CTYPE_*` bits (`<_ctype.h>`, `<runetype.h>`), and call
//! `__maskrune`, `__tolower` and `__toupper` for the rest. The classes are
//! those the C standard (7.4) gives the "C" locale; a byte above 127 has
//! none and maps to itself.
//!
//! The low byte of a digit's entry is its value (`digittoint` masks with
//! 0x0f), and every printable character has a width of one column
//! (`_CTYPE_SW1`), the space included.

/// `<_ctype.h>`.
pub const A: u32 = 0x0000_0100;
pub const C: u32 = 0x0000_0200;
pub const D: u32 = 0x0000_0400;
pub const G: u32 = 0x0000_0800;
pub const L: u32 = 0x0000_1000;
pub const P: u32 = 0x0000_2000;
pub const S: u32 = 0x0000_4000;
pub const U: u32 = 0x0000_8000;
pub const X: u32 = 0x0001_0000;
pub const B: u32 = 0x0002_0000;
pub const R: u32 = 0x0004_0000;
pub const SW1: u32 = 0x4000_0000;

/// `_CACHED_RUNES` (`<runetype.h>`).
pub const CACHED: usize = 256;

/// The class bits of byte `c` in the C locale.
pub const fn class(c: u8) -> u32 {
    let mut t = 0;
    if c < 0x20 || c == 0x7f {
        t |= C;
    }
    if c == b' ' || (c >= 0x09 && c <= 0x0d) {
        t |= S;
    }
    if c == b' ' || c == b'\t' {
        t |= B;
    }
    if c >= 0x20 && c < 0x7f {
        t |= R | SW1;
    }
    if c > 0x20 && c < 0x7f {
        t |= G;
    }
    let upper = c >= b'A' && c <= b'Z';
    let lower = c >= b'a' && c <= b'z';
    let digit = c >= b'0' && c <= b'9';
    if upper {
        t |= U | A;
    }
    if lower {
        t |= L | A;
    }
    if digit {
        t |= D | X | (c - b'0') as u32;
    }
    if (c >= b'a' && c <= b'f') || (c >= b'A' && c <= b'F') {
        t |= X | ((c | 0x20) - b'a' + 10) as u32;
    }
    if c > 0x20 && c < 0x7f && !upper && !lower && !digit {
        t |= P;
    }
    t
}

/// `__runetype`: the classes of the 256 cached runes.
pub const fn runetype() -> [u32; CACHED] {
    let mut t = [0u32; CACHED];
    let mut i = 0;
    while i < CACHED {
        t[i] = class(i as u8);
        i += 1;
    }
    t
}

/// `__maplower` (`upper` false) or `__mapupper` (`upper` true).
pub const fn map(upper: bool) -> [i32; CACHED] {
    let mut t = [0i32; CACHED];
    let mut i = 0;
    while i < CACHED {
        let c = i as u8;
        t[i] = if upper && c >= b'a' && c <= b'z' {
            (c - 32) as i32
        } else if !upper && c >= b'A' && c <= b'Z' {
            (c + 32) as i32
        } else {
            i as i32
        };
        i += 1;
    }
    t
}

/// `__maskrune(c, mask)`: the class bits of `c` that are in `mask` (none
/// for a rune outside the table: the C locale has no classes there).
pub fn mask_rune(c: i32, mask: u64) -> i32 {
    match usize::try_from(c) {
        Ok(i) if i < CACHED => (class(i as u8) & mask as u32) as i32,
        _ => 0,
    }
}

/// `__tolower` / `__toupper`: a rune outside the table maps to itself.
pub fn map_rune(c: i32, upper: bool) -> i32 {
    match u8::try_from(c) {
        Ok(b) if upper && b.is_ascii_lowercase() => i32::from(b.to_ascii_uppercase()),
        Ok(b) if !upper && b.is_ascii_uppercase() => i32::from(b.to_ascii_lowercase()),
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_match_the_c_locale() {
        for c in 0..=255u8 {
            let t = class(c);
            let has = |m| t & m != 0;
            assert_eq!(has(A), c.is_ascii_alphabetic(), "{c}");
            assert_eq!(has(D), c.is_ascii_digit(), "{c}");
            assert_eq!(has(X), c.is_ascii_hexdigit(), "{c}");
            assert_eq!(has(U), c.is_ascii_uppercase(), "{c}");
            assert_eq!(has(L), c.is_ascii_lowercase(), "{c}");
            assert_eq!(has(P), c.is_ascii_punctuation(), "{c}");
            assert_eq!(has(G), c.is_ascii_graphic(), "{c}");
            assert_eq!(has(C), c.is_ascii_control(), "{c}");
            // C's isspace includes \v (0x0b), Rust's is_ascii_whitespace does not.
            assert_eq!(has(S), c.is_ascii_whitespace() || c == 0x0b, "{c}");
            assert_eq!(has(R), c.is_ascii_graphic() || c == b' ', "{c}");
            assert_eq!(has(B), c == b' ' || c == b'\t', "{c}");
        }
        // digittoint: the value in the low byte.
        assert_eq!(class(b'7') & 0x0f, 7);
        assert_eq!(class(b'c') & 0x0f, 12);
        assert_eq!(class(b'F') & 0x0f, 15);
        assert_eq!(runetype()[b'q' as usize], class(b'q'));
        assert_eq!(runetype()[200], 0);
    }

    #[test]
    fn maps_and_masks() {
        assert_eq!(map(true)[b'a' as usize], 'A' as i32);
        assert_eq!(map(false)[b'Z' as usize], 'z' as i32);
        assert_eq!(map(true)[b'5' as usize], '5' as i32);
        assert_eq!(map(false)[233], 233);
        assert_eq!(map_rune('x' as i32, true), 'X' as i32);
        assert_eq!(map_rune(-1, true), -1);
        assert_eq!(map_rune(0x263a, false), 0x263a);
        assert_eq!(mask_rune('a' as i32, u64::from(A | D)), A as i32);
        assert_eq!(mask_rune(0x263a, u64::from(A)), 0);
        assert_eq!(mask_rune(-1, u64::from(A)), 0);
    }
}
