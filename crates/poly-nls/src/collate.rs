//! Poly's collation (ADR 0094): what `strcoll`, `wcscoll`, `strxfrm` and
//! `wcsxfrm` use in a locale other than "C".
//!
//! Windows collates with `CompareStringEx` and the sort tables of the
//! locale; Poly has no such tables and does not pretend to. Its order is
//! a simple, documented multi-level one, the same in every locale:
//!
//! 1. **Primary**: characters by class — white space and punctuation and
//!    symbols first (by code point), then digits (by value), then Latin
//!    letters by their base letter (`a` … `z`, case and diacritics
//!    ignored; `æ` is `ae`, `œ` `oe`, `ß` `ss`, `ĳ` `ij`, `þ` `th`), then
//!    every other letter by its lower case code point.
//! 2. **Secondary**: diacritics (none before grave, acute, circumflex,
//!    tilde, diaeresis, ring, cedilla, stroke, macron, breve, ogonek, dot,
//!    caron, double acute, other).
//! 3. **Tertiary**: case, lower before upper.
//!
//! Strings are compared level by level, so "resume" < "résumé" <
//! "Resume" < "Résumé" < "resumes". No locale tailoring (Swedish `å ä ö`
//! sort as `a a o`). To be confirmed by the spec agent against Windows'
//! order, which differs (word sort ignores hyphens and apostrophes; its
//! tables cover every script).
//!
//! A sort key ([`sort_key`]) is the three levels' weights as bytes (each
//! at least 2), separated by 1 and ended by 0, so comparing two keys with
//! `strcmp` orders the strings as [`compare`] does.

use crate::unicode;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// Base letter and diacritic of U+00C0-U+017F: two bytes per character,
/// the base (`a`-`z`, `0` for none, or an expansion code below) and the
/// diacritic (`0`-`f`: none, grave, acute, circumflex, tilde, diaeresis,
/// ring, cedilla, stroke, macron, breve, ogonek, dot, caron, double acute,
/// other). Expansions: `1` ae, `2` th, `3` ss, `4` oe, `5` ij.
const LATIN: &[u8; 2 * 0xC0] = b"\
a1a2a3a4a5a610c7e1e2e3e5i1i2i3i5d8n4o1o2o3o4o500o8u1u2u3u5y22030\
a1a2a3a4a5a610c7e1e2e3e5i1i2i3i5d8n4o1o2o3o4o500o8u1u2u3u5y220y5\
a9a9aaaaababc2c2c3c3cccccdcdddddd8d8e9e9eaeaececebebededg3g3gaga\
gcgcg7g7h3h3h8h8i4i4i9i9iaiaibibicif5050j3j3k7k7kfl2l2l7l7ldldlc\
lcl8l8n2n2n7n7ndndnfnfnfo9o9oaoaoeoe4040r2r2r7r7rdrds2s2s3s3s7s7\
sdsdt7t7tdtdt8t8u4u4u9u9uauau6u6ueueububw3w3y3y3y5z2z2zczczdzdsf";

/// The expansions' letters.
const EXPANSIONS: [&[u8]; 5] = [b"ae", b"th", b"ss", b"oe", b"ij"];

/// One character's weights: up to two primary weights, the diacritic, the
/// case.
struct Weights {
    primary: [u32; 2],
    count: usize,
    secondary: u8,
    tertiary: u8,
}

fn weights(c: u16) -> Weights {
    let class = unicode::class(c);
    let tertiary = if class & unicode::UPPER != 0 { 1 } else { 0 };
    let letter = |b: u8| 0x13_0000 + u32::from(b - b'a');
    let lower = unicode::to_lower(c);
    if (0x61..=0x7A).contains(&lower) {
        return Weights { primary: [letter(lower as u8), 0], count: 1, secondary: 0, tertiary };
    }
    if (0xC0..0x180).contains(&c) {
        let i = usize::from(c - 0xC0) * 2;
        let (base, mark) = (LATIN[i], LATIN[i + 1]);
        let secondary = (mark as char).to_digit(16).unwrap_or(0) as u8;
        match base {
            b'a'..=b'z' => return Weights { primary: [letter(base), 0], count: 1, secondary, tertiary },
            b'1'..=b'5' => {
                let e = EXPANSIONS[usize::from(base - b'1')];
                return Weights { primary: [letter(e[0]), letter(e[1])], count: 2, secondary, tertiary };
            }
            _ => {}
        }
    }
    if class & unicode::DIGIT != 0 {
        let v = match c {
            0x30..=0x39 => c - 0x30,
            0xFF10..=0xFF19 => c - 0xFF10,
            0x0660..=0x0669 => c - 0x0660,
            0x06F0..=0x06F9 => c - 0x06F0,
            0x0E50..=0x0E59 => c - 0x0E50,
            _ => 0,
        };
        return Weights { primary: [0x12_0000 + u32::from(v), 0], count: 1, secondary: 0, tertiary: 0 };
    }
    if class & unicode::LETTER != 0 {
        // Fullwidth Latin letters sort with their ASCII forms.
        if (0xFF41..=0xFF5A).contains(&lower) {
            let b = (lower - 0xFF41) as u8 + b'a';
            return Weights { primary: [letter(b), 0], count: 1, secondary: 15, tertiary };
        }
        return Weights { primary: [0x14_0000 + u32::from(lower), 0], count: 1, secondary: 0, tertiary };
    }
    Weights { primary: [0x100 + u32::from(c), 0], count: 1, secondary: 0, tertiary: 0 }
}

/// The three levels of a string (`ignore_case`: no tertiary level).
fn levels(s: &[u16], ignore_case: bool) -> (Vec<u32>, Vec<u8>, Vec<u8>) {
    let mut p = Vec::with_capacity(s.len());
    let mut sec = Vec::with_capacity(s.len());
    let mut ter = Vec::with_capacity(s.len());
    for &c in s {
        let w = weights(c);
        // One secondary and tertiary weight per primary weight, so an
        // expansion lines up with the letters it stands for.
        for &pw in &w.primary[..w.count] {
            p.push(pw);
            sec.push(w.secondary);
            if !ignore_case {
                ter.push(w.tertiary);
            }
        }
    }
    (p, sec, ter)
}

/// Compare two strings in Poly's collation.
pub fn compare(a: &[u16], b: &[u16], ignore_case: bool) -> Ordering {
    let (pa, sa, ta) = levels(a, ignore_case);
    let (pb, sb, tb) = levels(b, ignore_case);
    pa.cmp(&pb).then_with(|| sa.cmp(&sb)).then_with(|| ta.cmp(&tb)).then_with(|| {
        // Equal at every level but different (e.g. two characters of no
        // class): code points decide, so the order is total.
        if ignore_case { Ordering::Equal } else { a.cmp(b) }
    })
}

/// The sort key of a string (without its NUL); see the module text.
pub fn sort_key(s: &[u16]) -> Vec<u8> {
    let (p, sec, ter) = levels(s, false);
    let mut key = Vec::with_capacity(p.len() * 3 + sec.len() + ter.len() + s.len() * 3 + 4);
    for w in p {
        // Three base-254 digits, each + 2: order-preserving, never 0 or 1.
        key.push((w / (254 * 254)) as u8 + 2);
        key.push((w / 254 % 254) as u8 + 2);
        key.push((w % 254) as u8 + 2);
    }
    key.push(1);
    key.extend(sec.iter().map(|&s| s + 2));
    key.push(1);
    key.extend(ter.iter().map(|&t| t + 2));
    key.push(1);
    for &c in s {
        let w = u32::from(c);
        key.push((w / (254 * 254)) as u8 + 2);
        key.push((w / 254 % 254) as u8 + 2);
        key.push((w % 254) as u8 + 2);
    }
    key.push(0);
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn the_table_is_whole() {
        assert_eq!(LATIN.len(), 2 * 0xC0);
        // The letters of Latin-1 and Latin Extended-A have a base letter.
        for c in 0xC0..0x180u16 {
            if unicode::class(c) & unicode::LETTER != 0 && ![0x138, 0x149, 0x14A, 0x14B].contains(&c) {
                let i = usize::from(c - 0xC0) * 2;
                assert!(LATIN[i] != b'0', "{c:#x} has no base letter");
            }
        }
        // Upper and lower case of a letter have the same base and mark.
        for c in 0xC0..0x180u16 {
            let l = unicode::to_lower(c);
            if l != c && (0xC0..0x180).contains(&l) {
                let (i, j) = (usize::from(c - 0xC0) * 2, usize::from(l - 0xC0) * 2);
                assert_eq!(&LATIN[i..i + 2], &LATIN[j..j + 2], "{c:#x} / {l:#x}");
            }
        }
    }

    #[test]
    fn order() {
        let mut words = vec![
            "résumé", "Resume", "resumes", "resume", "Résumé", "zebra", "Zürich", "zoo", "10", "9", " x", "Ølen",
            "ole", "straße", "strasse", "Strasse", "über", "uber",
        ];
        words.sort_by(|a, b| compare(&w(a), &w(b), false));
        assert_eq!(
            words,
            [
                " x", "10", "9", "ole", "Ølen", "resume", "Resume", "résumé", "Résumé", "resumes", "strasse", "straße",
                "Strasse", "uber", "über", "zebra", "zoo", "Zürich"
            ]
        );
        assert_eq!(compare(&w("ABC"), &w("abc"), true), Ordering::Equal);
        assert_eq!(compare(&w("abc"), &w("ABC"), false), Ordering::Less);
        assert_eq!(compare(&w("é"), &w("E"), true), Ordering::Greater);
    }

    #[test]
    fn keys_order_like_compare() {
        let words = [
            "résumé", "Resume", "resumes", "resume", "Zürich", "zoo", "10", "9", "Æble", "aeble", "", "a", "日本",
            "中文", "ж", "Ж",
        ];
        for a in words {
            for b in words {
                let by_key = sort_key(&w(a)).cmp(&sort_key(&w(b)));
                assert_eq!(by_key, compare(&w(a), &w(b), false), "{a} / {b}");
                assert!(sort_key(&w(a))[..sort_key(&w(a)).len() - 1].iter().all(|&k| k != 0));
            }
        }
    }
}
