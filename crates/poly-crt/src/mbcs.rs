//! Multibyte-character strings (ADR 0094): the multibyte code page of
//! `_setmbcp`, its `_mbctype` and `_mbcasemap` tables, and the logic of
//! the `_mbs*`, `_mbc*`, `_mbb*` and `_ismb*` functions.
//!
//! Sources (clean room): Learn's pages of those functions, "Code Pages",
//! "Interpretation of multibyte-character sequences", "_setmbcp",
//! "_ismbb routines", "_ismbc routines" (with the code page 932 ranges of
//! `_ismbchira`, `_ismbckata`, `_ismbcsymbol`, `_ismbcl0/1/2`,
//! `_mbctohira`, `_mbctokata`), `mbctype.h` for the bits and the `_MB_CP_*`
//! values, `mbstring.h` for `_MBC_SINGLE` … `_MBC_ILLEGAL`; JIS X 0208's
//! published row/cell ↔ Shift-JIS arithmetic for `_mbcjistojms`.
//!
//! A multibyte character is one byte, or a lead byte and a trail byte,
//! written as the value `lead << 8 | trail` where a function takes or
//! returns one. A lead byte followed by the terminating NUL counts as a
//! character of its own (one byte). Poly's choices where Learn is silent
//! (to be confirmed by the spec agent): comparisons are byte by byte (the
//! case-insensitive ones fold single-byte letters with `_mbcasemap`);
//! `_mbsncpy` pads with one NUL per character short; a double-byte
//! character that would be split at the end of `_mbsnbcpy`'s or
//! `_mbsnbcat`'s count is left out.

use alloc::vec::Vec;

/// `mbctype.h`: `_MS` single-byte katakana symbol, `_MP` punctuation, `_M1`
/// lead byte, `_M2` trail byte, `_SBUP` / `_SBLOW` single-byte upper and
/// lower case.
pub const MS: u8 = 0x01;
pub const MP: u8 = 0x02;
pub const M1: u8 = 0x04;
pub const M2: u8 = 0x08;
pub const SBUP: u8 = 0x10;
pub const SBLOW: u8 = 0x20;

/// `_MB_CP_SBCS`, `_MB_CP_OEM`, `_MB_CP_ANSI`, `_MB_CP_LOCALE`
/// (`mbctype.h`).
pub const MB_CP_SBCS: i32 = 0;
pub const MB_CP_OEM: i32 = -2;
pub const MB_CP_ANSI: i32 = -3;
pub const MB_CP_LOCALE: i32 = -4;

/// `_MBC_SINGLE`, `_MBC_LEAD`, `_MBC_TRAIL`, `_MBC_ILLEGAL` (`mbstring.h`).
pub const MBC_SINGLE: i32 = 0;
pub const MBC_LEAD: i32 = 1;
pub const MBC_TRAIL: i32 = 2;
pub const MBC_ILLEGAL: i32 = -1;

/// A code page's characters as the multibyte functions need them.
pub trait Chars {
    /// The UTF-16 unit of a character (`c` a byte or `lead << 8 | trail`).
    fn to_wide(&self, c: u32) -> Option<u16>;
    /// The character of a UTF-16 unit, if the code page has it.
    fn char_of(&self, w: u16) -> Option<u32>;
}

/// The multibyte code page's tables (`_mbctype`, `_mbcasemap`).
#[derive(Clone, Debug)]
pub struct Mbc {
    /// `_getmbcp`: 0 for the single-byte "C" setting.
    pub code_page: u32,
    /// Whether the code page has lead bytes.
    pub is_mb: bool,
    /// `_mbctype`: entry 0 for `EOF`, then bytes 0-255.
    pub ctype: [u8; 257],
    /// `_mbcasemap`: the other case of a single-byte letter, else 0.
    pub casemap: [u8; 256],
}

impl Mbc {
    /// The single-byte setting (`_setmbcp(_MB_CP_SBCS)`): ASCII letters.
    pub fn sbcs() -> Mbc {
        let mut m = Mbc { code_page: 0, is_mb: false, ctype: [0; 257], casemap: [0; 256] };
        for b in b'A'..=b'Z' {
            m.ctype[usize::from(b) + 1] = SBUP;
            m.ctype[usize::from(b) + 33] = SBLOW;
            m.casemap[usize::from(b)] = b + 32;
            m.casemap[usize::from(b) + 32] = b;
        }
        m
    }

    /// The tables of code page `cp`: its lead and trail byte ranges (empty
    /// for a single-byte code page) and its characters.
    pub fn build(cp: u32, leads: &[(u8, u8)], trails: &[(u8, u8)], chars: &dyn Chars) -> Mbc {
        let mut m = Mbc::sbcs();
        m.code_page = cp;
        m.is_mb = !leads.is_empty();
        let in_ranges = |r: &[(u8, u8)], b: u8| r.iter().any(|&(lo, hi)| (lo..=hi).contains(&b));
        for b in 0x80..=0xFFu8 {
            let i = usize::from(b) + 1;
            if in_ranges(leads, b) {
                m.ctype[i] |= M1;
            } else if let Some(w) = chars.to_wide(u32::from(b)) {
                let class = poly_nls::unicode::class(w);
                if class & poly_nls::unicode::UPPER != 0 {
                    m.ctype[i] |= SBUP;
                } else if class & poly_nls::unicode::LOWER != 0 {
                    m.ctype[i] |= SBLOW;
                }
                let other = if class & poly_nls::unicode::UPPER != 0 {
                    poly_nls::unicode::to_lower(w)
                } else {
                    poly_nls::unicode::to_upper(w)
                };
                if other != w
                    && let Some(o) = chars.char_of(other).filter(|&o| o < 0x100)
                {
                    m.casemap[usize::from(b)] = o as u8;
                }
            }
            if in_ranges(trails, b) {
                m.ctype[i] |= M2;
            }
        }
        for b in 0x40..0x80u8 {
            if in_ranges(trails, b) {
                m.ctype[usize::from(b) + 1] |= M2;
            }
        }
        if cp == 932 {
            for b in 0xA1..=0xDFu8 {
                m.ctype[usize::from(b) + 1] |= if b <= 0xA5 { MP } else { MS };
            }
        }
        m
    }

    fn bits(&self, b: u8) -> u8 {
        self.ctype[usize::from(b) + 1]
    }

    /// `_ismbblead`.
    pub fn is_lead(&self, b: u8) -> bool {
        self.bits(b) & M1 != 0
    }

    /// `_ismbbtrail`.
    pub fn is_trail(&self, b: u8) -> bool {
        self.bits(b) & M2 != 0
    }

    /// `_ismbbkana` (code page 932 only: 0xA1-0xDF).
    pub fn is_kana(&self, b: u8) -> bool {
        self.code_page == 932 && (0xA1..=0xDF).contains(&b)
    }

    /// `_ismbbkalnum` (`_MS`), `_ismbbkpunct` (`_MP`), `_ismbbkprint`.
    pub fn has(&self, b: u8, mask: u8) -> bool {
        self.bits(b) & mask != 0
    }

    /// The character at `s[i]` and its length (see the module text).
    pub fn char_at(&self, s: &[u8], i: usize) -> (u32, usize) {
        let b = s[i];
        match s.get(i + 1) {
            Some(&t) if t != 0 && self.is_lead(b) => (u32::from(b) << 8 | u32::from(t), 2),
            _ => (u32::from(b), 1),
        }
    }

    /// The length of the character at `s[i]` (`_mbclen` looks only at the
    /// lead byte: [`Mbc::clen`]).
    fn len_at(&self, s: &[u8], i: usize) -> usize {
        self.char_at(s, i).1
    }

    /// `_mbclen`: 2 for a lead byte, else 1.
    pub fn clen(&self, b: u8) -> usize {
        if self.is_lead(b) { 2 } else { 1 }
    }

    /// The start of every character of `s` (no NUL), and `s.len()`.
    fn starts(&self, s: &[u8]) -> Vec<usize> {
        let mut v = Vec::with_capacity(s.len() + 1);
        let mut i = 0;
        while i < s.len() {
            v.push(i);
            i += self.len_at(s, i);
        }
        v.push(s.len().min(i));
        v
    }

    /// `_mbslen`: the characters of `s`.
    pub fn len(&self, s: &[u8]) -> usize {
        self.starts(s).len() - 1
    }

    /// `_mbsnbcnt`: the bytes of the first `n` characters.
    pub fn bytes_of_chars(&self, s: &[u8], n: usize) -> usize {
        let st = self.starts(s);
        st[n.min(st.len() - 1)]
    }

    /// `_mbsnccnt`: the characters in the first `n` bytes (a character
    /// the count cuts in two is not counted).
    pub fn chars_in_bytes(&self, s: &[u8], n: usize) -> usize {
        let mut i = 0;
        let mut k = 0;
        while i < s.len() && i < n {
            let l = self.len_at(s, i);
            if i + l > n {
                break;
            }
            i += l;
            k += 1;
        }
        k
    }

    /// `_mbsinc`: the start of the next character.
    pub fn inc(&self, s: &[u8], i: usize) -> usize {
        if i >= s.len() { i + 1 } else { i + self.len_at(s, i) }
    }

    /// `_mbsninc`: `n` characters on, at most to the NUL.
    pub fn ninc(&self, s: &[u8], mut i: usize, n: usize) -> usize {
        for _ in 0..n {
            if i >= s.len() {
                break;
            }
            i += self.len_at(s, i);
        }
        i.min(s.len())
    }

    /// `_mbsdec`: the start of the character before `cur`, `None` when
    /// `cur` is at the start.
    pub fn dec(&self, s: &[u8], cur: usize) -> Option<usize> {
        if cur == 0 {
            return None;
        }
        let mut i = 0;
        let mut prev = 0;
        while i < cur && i < s.len() {
            prev = i;
            i += self.len_at(s, i);
        }
        Some(if cur > s.len() { cur - 1 } else { prev })
    }

    /// `_mbsbtype(s, i)`: the type of byte `i`, `_MBC_ILLEGAL` past a NUL.
    pub fn byte_type(&self, s: &[u8], i: usize) -> i32 {
        if i >= s.len() {
            return MBC_ILLEGAL;
        }
        let mut k = 0;
        loop {
            let l = self.len_at(s, k);
            if i < k + l {
                return if l == 1 {
                    if self.is_lead(s[k]) { MBC_ILLEGAL } else { MBC_SINGLE }
                } else if i == k {
                    MBC_LEAD
                } else if self.is_trail(s[i]) {
                    MBC_TRAIL
                } else {
                    MBC_ILLEGAL
                };
            }
            k += l;
        }
    }

    /// `_mbbtype(c, previous)`: previous 1 means the byte before was a lead
    /// byte.
    pub fn btype(&self, c: u8, previous: i32) -> i32 {
        if previous == 1 {
            return if self.is_trail(c) { MBC_TRAIL } else { MBC_ILLEGAL };
        }
        if self.is_lead(c) { MBC_LEAD } else { MBC_SINGLE }
    }

    /// `_ismbslead(s, i)` / `_ismbstrail(s, i)`.
    pub fn is_lead_at(&self, s: &[u8], i: usize) -> bool {
        self.byte_type(s, i) == MBC_LEAD
    }
    pub fn is_trail_at(&self, s: &[u8], i: usize) -> bool {
        self.byte_type(s, i) == MBC_TRAIL
    }

    /// `_mbsnextc`.
    pub fn next_char(&self, s: &[u8]) -> u32 {
        if s.is_empty() { 0 } else { self.char_at(s, 0).0 }
    }

    /// `_mbschr`: the first `c` (`c == 0`: the NUL at `s.len()`).
    pub fn find_char(&self, s: &[u8], c: u32) -> Option<usize> {
        if c == 0 {
            return Some(s.len());
        }
        let mut i = 0;
        while i < s.len() {
            let (v, l) = self.char_at(s, i);
            if v == c {
                return Some(i);
            }
            i += l;
        }
        None
    }

    /// `_mbsrchr`.
    pub fn rfind_char(&self, s: &[u8], c: u32) -> Option<usize> {
        if c == 0 {
            return Some(s.len());
        }
        let mut found = None;
        let mut i = 0;
        while i < s.len() {
            let (v, l) = self.char_at(s, i);
            if v == c {
                found = Some(i);
            }
            i += l;
        }
        found
    }

    /// `_mbsstr`: `needle` starting at a character of `hay`.
    pub fn find_str(&self, hay: &[u8], needle: &[u8]) -> Option<usize> {
        if needle.is_empty() {
            return Some(0);
        }
        let mut i = 0;
        while i < hay.len() {
            if hay[i..].starts_with(needle) {
                return Some(i);
            }
            i += self.len_at(hay, i);
        }
        None
    }

    fn set_chars(&self, set: &[u8]) -> Vec<u32> {
        let mut v = Vec::new();
        let mut i = 0;
        while i < set.len() {
            let (c, l) = self.char_at(set, i);
            v.push(c);
            i += l;
        }
        v
    }

    /// `_mbscspn` (the bytes before the first character of `set`),
    /// `_mbsspn` (the bytes of the leading characters in `set`).
    pub fn span(&self, s: &[u8], set: &[u8], inside: bool) -> usize {
        let set = self.set_chars(set);
        let mut i = 0;
        while i < s.len() {
            let (c, l) = self.char_at(s, i);
            if set.contains(&c) != inside {
                break;
            }
            i += l;
        }
        i
    }

    /// `_mbspbrk`: the first character of `s` in `set`.
    pub fn pbrk(&self, s: &[u8], set: &[u8]) -> Option<usize> {
        let i = self.span(s, set, false);
        (i < s.len()).then_some(i)
    }

    /// `_mbsspnp`: the first character not in `set`, `None` at the end.
    pub fn spnp(&self, s: &[u8], set: &[u8]) -> Option<usize> {
        let i = self.span(s, set, true);
        (i < s.len()).then_some(i)
    }

    /// Fold a byte for the case-insensitive comparisons.
    fn fold(&self, b: u8) -> u8 {
        if self.bits(b) & SBUP != 0 && self.casemap[usize::from(b)] != 0 { self.casemap[usize::from(b)] } else { b }
    }

    /// Byte-by-byte comparison of at most `n` bytes (`_mbsnbcmp`,
    /// `_mbsnbicmp`, and `_mbscmp`/`_mbsicmp` with `usize::MAX`).
    pub fn compare_bytes(&self, a: &[u8], b: &[u8], n: usize, ignore_case: bool) -> i32 {
        let at = |s: &[u8], i: usize| s.get(i).copied().unwrap_or(0);
        for i in 0..n {
            let (mut x, mut y) = (at(a, i), at(b, i));
            if ignore_case {
                x = self.fold(x);
                y = self.fold(y);
            }
            if x != y {
                return if x < y { -1 } else { 1 };
            }
            if x == 0 {
                break;
            }
        }
        0
    }

    /// `_mbsncmp` / `_mbsnicmp`: at most `n` characters.
    pub fn compare_chars(&self, a: &[u8], b: &[u8], n: usize, ignore_case: bool) -> i32 {
        let bytes = self.bytes_of_chars(a, n).max(self.bytes_of_chars(b, n));
        self.compare_bytes(a, b, bytes, ignore_case)
    }

    /// The other case of a character, `upper` choosing the direction
    /// (`_mbctoupper`, `_mbctolower`): single bytes with `_mbcasemap`,
    /// double-byte characters through UTF-16.
    pub fn to_case(&self, c: u32, upper: bool, chars: &dyn Chars) -> u32 {
        if c < 0x100 {
            let b = c as u8;
            let want = if upper { SBLOW } else { SBUP };
            return if self.bits(b) & want != 0 && self.casemap[usize::from(b)] != 0 {
                u32::from(self.casemap[usize::from(b)])
            } else {
                c
            };
        }
        let Some(w) = chars.to_wide(c) else { return c };
        let o = if upper { poly_nls::unicode::to_upper(w) } else { poly_nls::unicode::to_lower(w) };
        if o == w {
            return c;
        }
        chars.char_of(o).filter(|&v| v >= 0x100).unwrap_or(c)
    }

    /// `_mbslwr` / `_mbsupr` in place.
    pub fn convert_case(&self, s: &mut [u8], upper: bool, chars: &dyn Chars) {
        let mut i = 0;
        while i < s.len() {
            let (c, l) = self.char_at(s, i);
            let o = self.to_case(c, upper, chars);
            if l == 2 {
                s[i] = (o >> 8) as u8;
                s[i + 1] = o as u8;
            } else {
                s[i] = o as u8;
            }
            i += l;
        }
    }

    /// `_mbsrev` in place: the characters reversed, each pair kept in
    /// order.
    pub fn reverse(&self, s: &mut [u8]) {
        let st = self.starts(s);
        let copy: Vec<u8> = s.to_vec();
        let mut o = 0;
        for w in st.windows(2).rev() {
            for &b in &copy[w[0]..w[1]] {
                s[o] = b;
                o += 1;
            }
        }
    }

    /// `_mbsset` / `_mbsnset` / `_mbsnbset`: fill the first `limit` bytes
    /// of `s` with `c`; a double-byte `c` that does not fit leaves a blank
    /// in the last byte (Learn, `_mbsnbset`).
    pub fn fill(&self, s: &mut [u8], c: u32, limit: usize) {
        let n = limit.min(s.len());
        if c < 0x100 {
            s[..n].fill(c as u8);
            return;
        }
        let mut i = 0;
        while i + 1 < n {
            s[i] = (c >> 8) as u8;
            s[i + 1] = c as u8;
            i += 2;
        }
        if i < n {
            s[i] = b' ';
        }
    }

    /// `_mbsnbcpy`/`_mbsnbcat`'s byte count: at most `n` bytes of `s`
    /// without cutting a character in two.
    pub fn whole_bytes(&self, s: &[u8], n: usize) -> usize {
        let mut i = 0;
        while i < s.len() {
            let l = self.len_at(s, i);
            if i + l > n {
                break;
            }
            i += l;
        }
        i
    }

    /// `_mbstok`: the next token of `s` starting at `from`: `(start, end)`
    /// of the token (end at a delimiter or the NUL), or `None`.
    pub fn token(&self, s: &[u8], from: usize, delims: &[u8]) -> Option<(usize, usize)> {
        let rest = s.get(from..)?;
        let start = from + self.span(rest, delims, true);
        if start >= s.len() {
            return None;
        }
        let end = start + self.span(&s[start..], delims, false);
        Some((start, end))
    }
}

/// `_ismbclegal`: a lead and a trail byte.
pub fn is_legal(m: &Mbc, c: u32) -> bool {
    c > 0xFF && c <= 0xFFFF && m.is_lead((c >> 8) as u8) && m.is_trail(c as u8)
}

/// The class bits (`ctype.h`) of a multibyte character, for the
/// `_ismbc*` functions: ASCII by the "C" table, others through UTF-16.
pub fn char_class(c: u32, chars: &dyn Chars) -> u16 {
    if c < 0x80 {
        return crate::ctype::class(c as i32);
    }
    chars.to_wide(c).map_or(0, poly_nls::unicode::class)
}

// ----- code page 932 ---------------------------------------------------------------------

/// Learn's code page 932 ranges.
pub fn is_hiragana(c: u32) -> bool {
    (0x829F..=0x82F1).contains(&c)
}
pub fn is_katakana(c: u32) -> bool {
    (0x8340..=0x8396).contains(&c) && c != 0x837F
}
pub fn is_symbol(c: u32) -> bool {
    (0x8141..=0x81AC).contains(&c) && c != 0x817F
}
/// `_ismbcl0`: JIS non-Kanji; `_ismbcl1`: JIS level 1; `_ismbcl2`: level 2.
pub fn jis_level(c: u32, m: &Mbc) -> Option<u8> {
    if m.code_page != 932 || !is_legal(m, c) {
        return None;
    }
    match c {
        0x8140..=0x889E => Some(0),
        0x889F..=0x9872 => Some(1),
        0x989F..=0xEAA4 => Some(2),
        _ => None,
    }
}

/// `_mbctohira`: katakana to hiragana (others unchanged).
pub fn to_hiragana(c: u32) -> u32 {
    match c {
        0x8340..=0x837E => c - 0xA1,
        0x8380..=0x8393 => c - 0xA2,
        _ => c,
    }
}

/// `_mbctokata`: hiragana to katakana.
pub fn to_katakana(c: u32) -> u32 {
    match c {
        0x829F..=0x82DD => c + 0xA1,
        0x82DE..=0x82F1 => c + 0xA2,
        _ => c,
    }
}

/// `_mbcjistojms`: a JIS X 0208 code (row and cell, each 0x21-0x7E) to
/// Shift-JIS; 0 if `c` is not one.
pub fn jis_to_sjis(c: u32) -> u32 {
    let (j1, j2) = ((c >> 8) & 0xFF, c & 0xFF);
    if c > 0xFFFF || !(0x21..=0x7E).contains(&j1) || !(0x21..=0x7E).contains(&j2) {
        return 0;
    }
    let s1 = j1.div_ceil(2) + if j1 <= 0x5E { 0x70 } else { 0xB0 };
    let s2 = if j1 % 2 == 1 { j2 + if j2 >= 0x60 { 0x20 } else { 0x1F } } else { j2 + 0x7E };
    s1 << 8 | s2
}

/// `_mbcjmstojis`: Shift-JIS to JIS X 0208; 0 if `c` is not a double-byte
/// Shift-JIS code of the JIS rows.
pub fn sjis_to_jis(c: u32) -> u32 {
    let (s1, s2) = ((c >> 8) & 0xFF, c & 0xFF);
    if c > 0xFFFF || !((0x81..=0x9F).contains(&s1) || (0xE0..=0xEF).contains(&s1)) {
        return 0;
    }
    if !(0x40..=0xFC).contains(&s2) || s2 == 0x7F {
        return 0;
    }
    let base = if s1 <= 0x9F { s1 - 0x70 } else { s1 - 0xB0 };
    let (j1, j2) =
        if s2 >= 0x9F { (base * 2, s2 - 0x7E) } else { (base * 2 - 1, if s2 >= 0x80 { s2 - 0x20 } else { s2 - 0x1F }) };
    j1 << 8 | j2
}

/// Halfwidth katakana (U+FF61-U+FF9F) and their fullwidth forms (the
/// Unicode Character Database's `<narrow>` decompositions; the voiced
/// marks as the spacing U+309B/U+309C).
const HALFWIDTH_KANA: [u16; 63] = [
    0x3002, 0x300C, 0x300D, 0x3001, 0x30FB, 0x30F2, 0x30A1, 0x30A3, 0x30A5, 0x30A7, 0x30A9, 0x30E3, 0x30E5, 0x30E7,
    0x30C3, 0x30FC, 0x30A2, 0x30A4, 0x30A6, 0x30A8, 0x30AA, 0x30AB, 0x30AD, 0x30AF, 0x30B1, 0x30B3, 0x30B5, 0x30B7,
    0x30B9, 0x30BB, 0x30BD, 0x30BF, 0x30C1, 0x30C4, 0x30C6, 0x30C8, 0x30CA, 0x30CB, 0x30CC, 0x30CD, 0x30CE, 0x30CF,
    0x30D2, 0x30D5, 0x30D8, 0x30DB, 0x30DE, 0x30DF, 0x30E0, 0x30E1, 0x30E2, 0x30E4, 0x30E6, 0x30E8, 0x30E9, 0x30EA,
    0x30EB, 0x30EC, 0x30ED, 0x30EF, 0x30F3, 0x309B, 0x309C,
];

/// The fullwidth form of a narrow character (ASCII, the space, halfwidth
/// katakana), if it has one.
fn widen(w: u16) -> Option<u16> {
    match w {
        0x20 => Some(0x3000),
        0x21..=0x7E => Some(w - 0x21 + 0xFF01),
        0xFF61..=0xFF9F => Some(HALFWIDTH_KANA[usize::from(w - 0xFF61)]),
        _ => None,
    }
}

/// The narrow form of a fullwidth character.
fn narrow(w: u16) -> Option<u16> {
    match w {
        0x3000 => Some(0x20),
        0xFF01..=0xFF5E => Some(w - 0xFF01 + 0x21),
        _ => HALFWIDTH_KANA.iter().position(|&k| k == w).map(|i| 0xFF61 + i as u16),
    }
}

/// `_mbbtombc`: a single-byte character to its double-byte form (code
/// page 932; else unchanged).
pub fn to_double(m: &Mbc, c: u32, chars: &dyn Chars) -> u32 {
    if m.code_page != 932 || c > 0xFF {
        return c;
    }
    chars.to_wide(c).and_then(widen).and_then(|w| chars.char_of(w)).filter(|&d| d > 0xFF).unwrap_or(c)
}

/// `_mbctombb`: a double-byte character to its single-byte form (code
/// page 932; else unchanged).
pub fn to_single(m: &Mbc, c: u32, chars: &dyn Chars) -> u32 {
    if m.code_page != 932 || c <= 0xFF {
        return c;
    }
    chars.to_wide(c).and_then(narrow).and_then(|w| chars.char_of(w)).filter(|&d| d <= 0xFF).unwrap_or(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_nls::codepage::CodePage;

    struct Cp(CodePage);
    impl Chars for Cp {
        fn to_wide(&self, c: u32) -> Option<u16> {
            if c < 0x100 { self.0.byte(c as u8) } else { self.0.double((c >> 8) as u8, c as u8) }
        }
        fn char_of(&self, w: u16) -> Option<u32> {
            self.0.encode_unit(w).map(u32::from)
        }
    }

    fn sjis() -> (Mbc, Cp) {
        let cp = CodePage::get(932).unwrap();
        let (l, t) = poly_nls::codepage::dbcs_ranges(932).unwrap();
        let chars = Cp(cp);
        (Mbc::build(932, l, t, &chars), chars)
    }

    #[test]
    fn tables() {
        let (m, _) = sjis();
        assert!(m.is_lead(0x81) && m.is_lead(0xFC) && !m.is_lead(0xA0) && !m.is_lead(b'A'));
        assert!(m.is_trail(0x40) && m.is_trail(0x81) && !m.is_trail(0x7F) && !m.is_trail(0xFD));
        assert!(m.is_kana(0xB1) && m.has(0xB1, MS) && m.has(0xA1, MP) && !m.has(b'A', MS));
        assert_eq!(m.ctype[usize::from(b'A') + 1] & SBUP, SBUP);
        assert_eq!(m.casemap[usize::from(b'a')], b'A');
        let s = Mbc::sbcs();
        assert!(!s.is_lead(0x81) && s.code_page == 0);
        let cp1252 = Cp(CodePage::get(1252).unwrap());
        let w = Mbc::build(1252, &[], &[], &cp1252);
        assert_eq!(w.casemap[0xE9], 0xC9);
        assert_eq!(w.ctype[0xC9 + 1] & SBUP, SBUP);
        assert!(!w.is_mb);
    }

    #[test]
    fn walking_and_counting() {
        let (m, _) = sjis();
        // "a日本b" = a, 93 FA, 96 7B, b.
        let s = b"a\x93\xFA\x96\x7Bb";
        assert_eq!(m.len(s), 4);
        assert_eq!(m.bytes_of_chars(s, 2), 3);
        assert_eq!(m.bytes_of_chars(s, 9), 6);
        assert_eq!(m.chars_in_bytes(s, 2), 1);
        assert_eq!(m.chars_in_bytes(s, 3), 2);
        assert_eq!(m.inc(s, 1), 3);
        assert_eq!(m.ninc(s, 0, 3), 5);
        assert_eq!(m.dec(s, 5), Some(3));
        assert_eq!(m.dec(s, 3), Some(1));
        assert_eq!(m.dec(s, 0), None);
        assert_eq!(m.next_char(&s[1..]), 0x93FA);
        assert_eq!(m.byte_type(s, 1), MBC_LEAD);
        assert_eq!(m.byte_type(s, 2), MBC_TRAIL);
        assert_eq!(m.byte_type(s, 5), MBC_SINGLE);
        assert_eq!(m.byte_type(s, 6), MBC_ILLEGAL);
        assert_eq!(m.btype(0x93, 0), MBC_LEAD);
        assert_eq!(m.btype(0x40, 1), MBC_TRAIL);
        assert_eq!(m.btype(0x20, 1), MBC_ILLEGAL);
        // A lead byte before the NUL is a character of its own.
        assert_eq!(m.len(b"a\x93"), 2);
        assert_eq!(m.clen(0x93), 2);
    }

    #[test]
    fn searching() {
        let (m, _) = sjis();
        // 0x955C is 表, whose trail byte is '\\': not a match for '\\'.
        let s = b"\x95\x5Cx\\y";
        assert_eq!(m.find_char(s, u32::from(b'\\')), Some(3));
        assert_eq!(m.find_char(s, 0x955C), Some(0));
        assert_eq!(m.rfind_char(b"a\\b\\", u32::from(b'\\')), Some(3));
        assert_eq!(m.find_char(s, 0), Some(5));
        assert_eq!(m.find_str(s, b"\\y"), Some(3));
        assert_eq!(m.find_str(s, b"\x5Cx"), None);
        assert_eq!(m.span(s, b"\\", false), 3);
        assert_eq!(m.span(b"aab", b"a", true), 2);
        assert_eq!(m.pbrk(s, b"yx"), Some(2));
        assert_eq!(m.spnp(b"aaa", b"a"), None);
        assert_eq!(m.token(b"  a b", 0, b" "), Some((2, 3)));
        assert_eq!(m.token(b"  a b", 3, b" "), Some((4, 5)));
        assert_eq!(m.token(b"  ", 0, b" "), None);
    }

    #[test]
    fn comparing_and_case() {
        let (m, c) = sjis();
        assert_eq!(m.compare_bytes(b"abc", b"ABC", usize::MAX, true), 0);
        assert!(m.compare_bytes(b"abc", b"abd", usize::MAX, false) < 0);
        assert_eq!(m.compare_bytes(b"abX", b"abY", 2, false), 0);
        assert_eq!(m.compare_chars(b"\x93\xFAa", b"\x93\xFAb", 1, false), 0);
        assert!(m.compare_chars(b"\x93\xFAa", b"\x93\xFAb", 2, false) < 0);
        // Fullwidth Ａ (0x8260) ↔ ａ (0x8281).
        assert_eq!(m.to_case(0x8260, false, &c), 0x8281);
        assert_eq!(m.to_case(0x8281, true, &c), 0x8260);
        assert_eq!(m.to_case(u32::from(b'q'), true, &c), u32::from(b'Q'));
        let mut s = *b"ab\x82\x60c";
        m.convert_case(&mut s, false, &c);
        assert_eq!(&s, b"ab\x82\x81c");
        m.convert_case(&mut s, true, &c);
        assert_eq!(&s, b"AB\x82\x60C");
        let mut r = *b"a\x93\xFAb";
        m.reverse(&mut r);
        assert_eq!(&r, b"b\x93\xFAa");
        let mut f = [0u8; 5];
        m.fill(&mut f, 0x8140, 5);
        assert_eq!(&f, b"\x81\x40\x81\x40 ");
        assert_eq!(m.whole_bytes(b"a\x93\xFA", 2), 1);
    }

    #[test]
    fn japanese_specifics() {
        let (m, c) = sjis();
        assert!(is_hiragana(0x82A0) && is_katakana(0x8341) && !is_katakana(0x837F) && is_symbol(0x8141));
        assert_eq!(to_hiragana(0x8341), 0x82A0); // ア → あ
        assert_eq!(to_katakana(0x82A0), 0x8341);
        assert_eq!(to_hiragana(0x8380), 0x82DE); // ム → む
        assert_eq!(to_katakana(0x82DE), 0x8380);
        assert_eq!(jis_to_sjis(0x2422), 0x82A0); // あ
        assert_eq!(sjis_to_jis(0x82A0), 0x2422);
        assert_eq!(jis_to_sjis(0x3021), 0x889F); // 亜
        assert_eq!(sjis_to_jis(0x889F), 0x3021);
        for j1 in 0x21..=0x7Eu32 {
            for j2 in 0x21..=0x7Eu32 {
                assert_eq!(sjis_to_jis(jis_to_sjis(j1 << 8 | j2)), j1 << 8 | j2);
            }
        }
        assert_eq!(jis_to_sjis(0x1234), 0);
        assert_eq!(jis_level(0x82A0, &m), Some(0));
        assert_eq!(jis_level(0x889F, &m), Some(1));
        assert_eq!(jis_level(0x989F, &m), Some(2));
        assert_eq!(to_double(&m, u32::from(b'A'), &c), 0x8260);
        assert_eq!(to_single(&m, 0x8260, &c), u32::from(b'A'));
        assert_eq!(to_double(&m, 0xB1, &c), 0x8341); // ｱ → ア
        assert_eq!(to_single(&m, 0x8341, &c), 0xB1);
        assert!(is_legal(&m, 0x8140) && !is_legal(&m, 0x8120) && !is_legal(&m, 0x41));
        assert_eq!(char_class(0x8260, &c) & crate::ctype::UPPER, crate::ctype::UPPER);
        assert_eq!(char_class(0x82A0, &c) & crate::ctype::ALPHA, 0x100);
    }
}
