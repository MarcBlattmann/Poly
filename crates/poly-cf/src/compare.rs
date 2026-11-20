//! `CFStringCompare` and its options (`<CoreFoundation/CFString.h>`), as the
//! spec sheets `CFStringCompare` and `CFStringCompareWithOptions` describe
//! them: without options UTF-16 units in order as unsigned numbers; case
//! folding, digit runs as numbers, canonical equivalence, diacritics and
//! full-width forms on request.
//!
//! The decomposition and case tables cover Latin (Latin-1 Supplement and
//! Latin Extended-A), Greek and Cyrillic, from the Unicode Character
//! Database's public data.

use alloc::vec::Vec;

pub const CASE_INSENSITIVE: u64 = 1;
pub const BACKWARDS: u64 = 4;
pub const ANCHORED: u64 = 8;
pub const NONLITERAL: u64 = 16;
pub const LOCALIZED: u64 = 32;
pub const NUMERICALLY: u64 = 64;
pub const DIACRITIC_INSENSITIVE: u64 = 128;
pub const WIDTH_INSENSITIVE: u64 = 256;
pub const FORCED_ORDERING: u64 = 512;

/// The canonical decomposition of a precomposed Latin letter: (base, mark).
pub fn decompose(u: u16) -> Option<(u16, u16)> {
    const GRAVE: u16 = 0x300;
    const ACUTE: u16 = 0x301;
    const CIRC: u16 = 0x302;
    const TILDE: u16 = 0x303;
    const MACRON: u16 = 0x304;
    const BREVE: u16 = 0x306;
    const DOT: u16 = 0x307;
    const DIAER: u16 = 0x308;
    const RING: u16 = 0x30A;
    const DACUTE: u16 = 0x30B;
    const CARON: u16 = 0x30C;
    const CEDIL: u16 = 0x327;
    const OGONEK: u16 = 0x328;
    // Latin-1: the upper-case row; lower case is 0x20 above with the same
    // marks (and ÿ).
    const LATIN1: [(u16, u16); 32] = [
        (b'A' as u16, GRAVE),
        (b'A' as u16, ACUTE),
        (b'A' as u16, CIRC),
        (b'A' as u16, TILDE),
        (b'A' as u16, DIAER),
        (b'A' as u16, RING),
        (0, 0),
        (b'C' as u16, CEDIL),
        (b'E' as u16, GRAVE),
        (b'E' as u16, ACUTE),
        (b'E' as u16, CIRC),
        (b'E' as u16, DIAER),
        (b'I' as u16, GRAVE),
        (b'I' as u16, ACUTE),
        (b'I' as u16, CIRC),
        (b'I' as u16, DIAER),
        (0, 0),
        (b'N' as u16, TILDE),
        (b'O' as u16, GRAVE),
        (b'O' as u16, ACUTE),
        (b'O' as u16, CIRC),
        (b'O' as u16, TILDE),
        (b'O' as u16, DIAER),
        (0, 0),
        (0, 0),
        (b'U' as u16, GRAVE),
        (b'U' as u16, ACUTE),
        (b'U' as u16, CIRC),
        (b'U' as u16, DIAER),
        (b'Y' as u16, ACUTE),
        (0, 0),
        (0, 0),
    ];
    // Latin Extended-A from U+0100, in pairs (upper, lower) unless noted.
    const EXT_A: [(u8, u16); 128] = [
        (b'A', MACRON),
        (b'a', MACRON),
        (b'A', BREVE),
        (b'a', BREVE),
        (b'A', OGONEK),
        (b'a', OGONEK),
        (b'C', ACUTE),
        (b'c', ACUTE),
        (b'C', CIRC),
        (b'c', CIRC),
        (b'C', DOT),
        (b'c', DOT),
        (b'C', CARON),
        (b'c', CARON),
        (b'D', CARON),
        (b'd', CARON),
        (0, 0),
        (0, 0),
        (b'E', MACRON),
        (b'e', MACRON),
        (b'E', BREVE),
        (b'e', BREVE),
        (b'E', DOT),
        (b'e', DOT),
        (b'E', OGONEK),
        (b'e', OGONEK),
        (b'E', CARON),
        (b'e', CARON),
        (b'G', CIRC),
        (b'g', CIRC),
        (b'G', BREVE),
        (b'g', BREVE),
        (b'G', DOT),
        (b'g', DOT),
        (b'G', CEDIL),
        (b'g', CEDIL),
        (b'H', CIRC),
        (b'h', CIRC),
        (0, 0),
        (0, 0),
        (b'I', TILDE),
        (b'i', TILDE),
        (b'I', MACRON),
        (b'i', MACRON),
        (b'I', BREVE),
        (b'i', BREVE),
        (b'I', OGONEK),
        (b'i', OGONEK),
        (b'I', DOT),
        (0, 0),
        (0, 0),
        (0, 0),
        (b'J', CIRC),
        (b'j', CIRC),
        (b'K', CEDIL),
        (b'k', CEDIL),
        (0, 0),
        (b'L', ACUTE),
        (b'l', ACUTE),
        (b'L', CEDIL),
        (b'l', CEDIL),
        (b'L', CARON),
        (b'l', CARON),
        (0, 0),
        (0, 0),
        (0, 0),
        (0, 0),
        (b'N', ACUTE),
        (b'n', ACUTE),
        (b'N', CEDIL),
        (b'n', CEDIL),
        (b'N', CARON),
        (b'n', CARON),
        (0, 0),
        (0, 0),
        (0, 0),
        (b'O', MACRON),
        (b'o', MACRON),
        (b'O', BREVE),
        (b'o', BREVE),
        (b'O', DACUTE),
        (b'o', DACUTE),
        (0, 0),
        (0, 0),
        (b'R', ACUTE),
        (b'r', ACUTE),
        (b'R', CEDIL),
        (b'r', CEDIL),
        (b'R', CARON),
        (b'r', CARON),
        (b'S', ACUTE),
        (b's', ACUTE),
        (b'S', CIRC),
        (b's', CIRC),
        (b'S', CEDIL),
        (b's', CEDIL),
        (b'S', CARON),
        (b's', CARON),
        (b'T', CEDIL),
        (b't', CEDIL),
        (b'T', CARON),
        (b't', CARON),
        (0, 0),
        (0, 0),
        (b'U', TILDE),
        (b'u', TILDE),
        (b'U', MACRON),
        (b'u', MACRON),
        (b'U', BREVE),
        (b'u', BREVE),
        (b'U', RING),
        (b'u', RING),
        (b'U', DACUTE),
        (b'u', DACUTE),
        (b'U', OGONEK),
        (b'u', OGONEK),
        (b'W', CIRC),
        (b'w', CIRC),
        (b'Y', CIRC),
        (b'y', CIRC),
        (b'Y', DIAER),
        (b'Z', ACUTE),
        (b'z', ACUTE),
        (b'Z', DOT),
        (b'z', DOT),
        (b'Z', CARON),
        (b'z', CARON),
        (0, 0),
    ];
    match u {
        0xC0..=0xDF => {
            let (b, m) = LATIN1[usize::from(u - 0xC0)];
            (b != 0).then_some((b, m))
        }
        0xE0..=0xFD => {
            let (b, m) = LATIN1[usize::from(u - 0xE0)];
            (b != 0).then_some((b + 0x20, m))
        }
        0xFF => Some((u16::from(b'y'), DIAER)),
        0x100..=0x17F => {
            let (b, m) = EXT_A[usize::from(u - 0x100)];
            (b != 0).then_some((u16::from(b), m))
        }
        _ => None,
    }
}

/// Whether `u` is a combining diacritical mark.
pub fn is_mark(u: u16) -> bool {
    (0x300..=0x36F).contains(&u)
}

/// Simple lower-case mapping (`None`: the unit maps to itself).
pub fn to_lower(u: u16) -> u16 {
    match u {
        0x41..=0x5A => u + 0x20,
        0xC0..=0xDE if u != 0xD7 => u + 0x20,
        0x100..=0x137 | 0x14A..=0x177 if u.is_multiple_of(2) => u + 1,
        0x139..=0x148 | 0x179..=0x17E if u % 2 == 1 => u + 1,
        0x178 => 0xFF,
        0x391..=0x3A9 if u != 0x3A2 => u + 0x20,
        0x410..=0x42F => u + 0x20,
        0x400..=0x40F => u + 0x50,
        0xFF21..=0xFF3A => u + 0x20,
        _ => u,
    }
}

/// Simple upper-case mapping.
pub fn to_upper(u: u16) -> u16 {
    match u {
        0x61..=0x7A => u - 0x20,
        0xE0..=0xFE if u != 0xF7 => u - 0x20,
        0xFF => 0x178,
        0x101..=0x138 | 0x14B..=0x178 if u % 2 == 1 => u - 1,
        0x13A..=0x149 | 0x17A..=0x17F if u.is_multiple_of(2) => u - 1,
        0x3B1..=0x3C9 if u != 0x3C2 => u - 0x20,
        0x430..=0x44F => u - 0x20,
        0x450..=0x45F => u - 0x50,
        0xFF41..=0xFF5A => u - 0x20,
        _ => u,
    }
}

/// The units after the options' folding, in order.
fn fold(s: &[u16], options: u64) -> Vec<u16> {
    let mut out = Vec::with_capacity(s.len());
    for &u0 in s {
        let mut u = u0;
        if options & WIDTH_INSENSITIVE != 0 && (0xFF01..=0xFF5E).contains(&u) {
            u -= 0xFF01 - 0x21;
        }
        let decomposed = if options & (NONLITERAL | DIACRITIC_INSENSITIVE) != 0 { decompose(u) } else { None };
        let mut push = |mut c: u16| {
            if options & DIACRITIC_INSENSITIVE != 0 && is_mark(c) {
                return;
            }
            if options & CASE_INSENSITIVE != 0 {
                if c == 0xDF {
                    out.extend_from_slice(&[u16::from(b's'), u16::from(b's')]);
                    return;
                }
                c = to_lower(c);
            }
            out.push(c);
        };
        match decomposed {
            Some((base, mark)) => {
                push(base);
                push(mark);
            }
            None => push(u),
        }
    }
    out
}

fn is_digit(u: u16) -> bool {
    (0x30..=0x39).contains(&u)
}

fn sign(o: core::cmp::Ordering) -> i32 {
    match o {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

/// Compare the folded forms; digit runs as numbers with `NUMERICALLY`.
fn compare_folded(a: &[u16], b: &[u16], numeric: bool) -> i32 {
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if numeric && is_digit(a[i]) && is_digit(b[j]) {
            let si = i;
            while i < a.len() && is_digit(a[i]) {
                i += 1;
            }
            let sj = j;
            while j < b.len() && is_digit(b[j]) {
                j += 1;
            }
            let da = trim_zeros(&a[si..i]);
            let db = trim_zeros(&b[sj..j]);
            let c = sign(da.len().cmp(&db.len()).then_with(|| da.cmp(db)));
            if c != 0 {
                return c;
            }
            continue;
        }
        if a[i] != b[j] {
            return sign(a[i].cmp(&b[j]));
        }
        i += 1;
        j += 1;
    }
    sign((a.len() - i).cmp(&(b.len() - j)))
}

fn trim_zeros(d: &[u16]) -> &[u16] {
    let z = d.iter().take_while(|&&u| u == u16::from(b'0')).count();
    &d[z..]
}

/// `CFStringCompare(a, b, options)`: -1, 0 or 1.
pub fn compare(a: &[u16], b: &[u16], options: u64) -> i32 {
    let numeric = options & NUMERICALLY != 0;
    let folding = options & (CASE_INSENSITIVE | NONLITERAL | DIACRITIC_INSENSITIVE | WIDTH_INSENSITIVE) != 0;
    let c = if folding {
        compare_folded(&fold(a, options), &fold(b, options), numeric)
    } else {
        compare_folded(a, b, numeric)
    };
    if c == 0 && options & FORCED_ORDERING != 0 && a != b {
        return compare_folded(a, b, false);
    }
    c
}

/// Whether `hay` at `at` starts with `needle` under the folding options
/// (without numeric reading).
pub fn equal_at(hay: &[u16], needle: &[u16], options: u64) -> bool {
    if needle.len() > hay.len() {
        return false;
    }
    let opts = options & !(NUMERICALLY | FORCED_ORDERING);
    if opts & (CASE_INSENSITIVE | NONLITERAL | DIACRITIC_INSENSITIVE | WIDTH_INSENSITIVE) == 0 {
        return hay == needle;
    }
    fold(hay, opts) == fold(needle, opts)
}

/// `CFStringFind`-style search: the first (or with `BACKWARDS` the last)
/// index of `needle` in `hay`; `ANCHORED` only at the start (or the end).
pub fn find(hay: &[u16], needle: &[u16], options: u64) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    let last = hay.len() - needle.len();
    let at = |i: usize| equal_at(&hay[i..i + needle.len()], needle, options);
    match (options & BACKWARDS != 0, options & ANCHORED != 0) {
        (false, false) => (0..=last).find(|&i| at(i)),
        (true, false) => (0..=last).rev().find(|&i| at(i)),
        (false, true) => at(0).then_some(0),
        (true, true) => at(last).then_some(last),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn cmp(a: &str, b: &str, o: u64) -> i32 {
        compare(&u(a), &u(b), o)
    }

    #[test]
    fn plain_order_is_by_unit() {
        assert_eq!(cmp("a", "b", 0), -1);
        assert_eq!(cmp("a", "B", 0), 1);
        assert_eq!(cmp("Z", "a", 0), -1);
        assert_eq!(cmp("ab", "abc", 0), -1);
        assert_eq!(cmp("", "", 0), 0);
        assert_eq!(cmp("a2", "a10", 0), 1);
        assert_eq!(cmp("\u{1F600}", "\u{FF61}", 0), -1);
        assert_eq!(cmp("é", "e", 0), 1);
    }

    #[test]
    fn options() {
        assert_eq!(cmp("É", "é", CASE_INSENSITIVE), 0);
        assert_eq!(cmp("É", "é", 0), -1);
        assert_eq!(cmp("ß", "ss", CASE_INSENSITIVE), 0);
        assert_eq!(cmp("a", "B", CASE_INSENSITIVE), -1);
        assert_eq!(cmp("Z", "a", CASE_INSENSITIVE), 1);
        assert_eq!(cmp("é", "e\u{301}", 0), 1);
        assert_eq!(cmp("é", "e\u{301}", NONLITERAL), 0);
        assert_eq!(cmp("é", "e", DIACRITIC_INSENSITIVE), 0);
        assert_eq!(cmp("resume", "résumé", DIACRITIC_INSENSITIVE), 0);
        assert_eq!(cmp("\u{FF21}", "A", 0), 1);
        assert_eq!(cmp("\u{FF21}", "A", WIDTH_INSENSITIVE), 0);
        assert_eq!(cmp("a", "A", CASE_INSENSITIVE | FORCED_ORDERING), 1);
        assert_eq!(cmp("abc", "abc", 0x8000_0000), 0);
    }

    #[test]
    fn numbers() {
        let n = |a, b| cmp(a, b, NUMERICALLY);
        assert_eq!(n("a2", "a10"), -1);
        assert_eq!(n("a10", "a2"), 1);
        assert_eq!(n("a02", "a2"), 0);
        assert_eq!(n("a1", "a01"), 0);
        assert_eq!(n("file9.txt", "file10.txt"), -1);
        assert_eq!(n("1.5", "1.10"), -1);
        assert_eq!(n("100", "99"), 1);
        assert_eq!(cmp("File10.txt", "file2.txt", CASE_INSENSITIVE | NUMERICALLY), 1);
        assert_eq!(cmp("File10.txt", "file2.txt", CASE_INSENSITIVE), -1);
    }

    #[test]
    fn case_maps_are_inverse_on_letters() {
        for c in (0x41..0x5B).chain(0xC0..0xDF).chain(0x100..0x17F).chain(0x391..0x3AA).chain(0x400..0x430) {
            let l = to_lower(c);
            if l != c {
                assert_eq!(to_upper(l), c, "{c:#x}");
            }
        }
        let _ = String::new();
    }

    #[test]
    fn search() {
        assert_eq!(find(&u("hello.txt"), &u(".txt"), BACKWARDS | ANCHORED), Some(5));
        assert_eq!(find(&u("hello"), &u("L"), CASE_INSENSITIVE), Some(2));
        assert_eq!(find(&u("hello"), &u("x"), 0), None);
    }
}
