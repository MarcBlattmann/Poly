//! The code pages Poly converts (ADR 0094): their bytes to UTF-16 and back.
//!
//! - **Single-byte**: 1252 (`poly_rtl::codepage`, ADR 0071), and the
//!   generated tables ([`crate::tables`]) of 437, 737, 775, 850, 852, 855,
//!   857, 860, 861, 862, 863, 865, 866, 869, 874 and 1250-1258 (1252
//!   excepted). Bytes 0x00-0x7F are ASCII.
//! - **Double-byte** (DBCS): 932 (Japanese Shift-JIS), 936 (Simplified
//!   Chinese GBK), 949 (Korean Unified Hangul Code), 950 (Traditional
//!   Chinese Big5): a lead byte and a trail byte make one character.
//! - **UTF-8** (65001).
//!
//! Sources (clean room): Learn's "Code Page Identifiers", "Code Pages",
//! `MultiByteToWideChar` / `WideCharToMultiByte` / `GetCPInfo` pages; the
//! tables are the Unicode Consortium's public mapping files (see
//! [`crate::tables`]); Unicode's table of well-formed UTF-8 byte sequences.
//!
//! **Choices Learn leaves open** (to be confirmed by the spec agent):
//! - A byte (or double byte) the mapping file leaves unassigned decodes to
//!   `?` (U+003F) and is invalid under `MB_ERR_INVALID_CHARS`, except in
//!   1252, whose five unassigned bytes are the C1 controls of the same
//!   value (ADR 0079). A lead byte followed by a byte that is not one of
//!   the code page's trail bytes (or by nothing) is one `?`, and the
//!   second byte is decoded on its own; a lead and a trail byte that make
//!   no character are one `?`.
//! - No best-fit mapping: a character the code page lacks becomes the
//!   default character `?`.
//! - When a code page maps several codes to one character (932's NEC and
//!   IBM extensions), the character converts back to the lowest code,
//!   except that codes with lead byte 0x87, 0xED or 0xEE lose to any other.

use crate::tables::{self, Dbcs};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicPtr, Ordering};

/// `CP_UTF8` (`winnls.h`).
pub const CP_UTF8: u32 = 65001;
/// The default character of every code page Poly has (`CPINFO.DefaultChar`).
pub const DEFAULT_CHAR: u8 = b'?';
/// What an unassigned byte decodes to.
pub const UNICODE_DEFAULT: u16 = 0x003F;

/// Code page 1252's bytes 0x80-0xFF, from `poly_rtl::codepage`.
static CP1252: [u16; 128] = {
    let mut t = [0u16; 128];
    let mut i = 0;
    while i < 128 {
        t[i] = poly_rtl::codepage::to_unicode(0x80 + i as u8);
        i += 1;
    }
    t
};

#[derive(Clone, Copy, Debug)]
enum Kind {
    Sbcs(&'static [u16; 128]),
    Dbcs(usize),
    Utf8,
}

/// A code page Poly can convert.
#[derive(Clone, Copy, Debug)]
pub struct CodePage {
    id: u32,
    kind: Kind,
}

impl PartialEq for CodePage {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id
    }
}
impl Eq for CodePage {}

/// One character decoded from the front of a byte string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decoded {
    /// A character (one UTF-16 unit, or a surrogate pair for UTF-8 above
    /// U+FFFF: `(unit, second unit or 0)`) and the bytes it took.
    Char { units: [u16; 2], len: usize },
    /// The bytes are the start of a character that needs more.
    Incomplete,
    /// The bytes make no character (the length of the bad part).
    Invalid { len: usize },
}

impl CodePage {
    /// The code page with this identifier, if Poly has it. (`CP_ACP` and
    /// friends are resolved by the caller.)
    pub fn get(id: u32) -> Option<CodePage> {
        if id == 1252 {
            return Some(CodePage { id, kind: Kind::Sbcs(&CP1252) });
        }
        if id == CP_UTF8 {
            return Some(CodePage { id, kind: Kind::Utf8 });
        }
        if let Some(&(_, t)) = tables::SBCS.iter().find(|(i, _)| u32::from(*i) == id) {
            return Some(CodePage { id, kind: Kind::Sbcs(t) });
        }
        tables::DBCS.iter().position(|d| u32::from(d.info.id) == id).map(|i| CodePage { id, kind: Kind::Dbcs(i) })
    }

    /// Every code page Poly has, in increasing order.
    pub fn ids() -> Vec<u32> {
        let mut v: Vec<u32> = tables::SBCS.iter().map(|(i, _)| u32::from(*i)).collect();
        v.extend(tables::DBCS.iter().map(|d| u32::from(d.info.id)));
        v.push(1252);
        v.push(CP_UTF8);
        v.sort_unstable();
        v
    }

    pub fn id(self) -> u32 {
        self.id
    }

    pub fn is_utf8(self) -> bool {
        matches!(self.kind, Kind::Utf8)
    }

    pub fn is_dbcs(self) -> bool {
        matches!(self.kind, Kind::Dbcs(_))
    }

    fn dbcs(self) -> Option<&'static Dbcs> {
        match self.kind {
            Kind::Dbcs(i) => Some(tables::DBCS[i]),
            _ => None,
        }
    }

    /// `CPINFO.MaxCharSize`.
    pub fn max_char_size(self) -> u32 {
        match self.kind {
            Kind::Sbcs(_) => 1,
            Kind::Dbcs(_) => 2,
            Kind::Utf8 => 4,
        }
    }

    /// The lead byte ranges (`CPINFO.LeadByte`), empty but for DBCS.
    pub fn lead_ranges(self) -> &'static [(u8, u8)] {
        dbcs_ranges(self.id).map_or(&[], |(l, _)| l)
    }

    /// Whether `b` is a lead byte (`IsDBCSLeadByteEx`).
    pub fn is_lead(self, b: u8) -> bool {
        self.dbcs().is_some_and(|d| d.rows[usize::from(b)] != 0)
    }

    /// A byte that stands alone: its character, `None` if unassigned or a
    /// lead byte (or, in UTF-8, not ASCII).
    pub fn byte(self, b: u8) -> Option<u16> {
        if b < 0x80 {
            return Some(u16::from(b));
        }
        let c = match self.kind {
            Kind::Sbcs(t) => t[usize::from(b - 0x80)],
            Kind::Dbcs(i) => tables::DBCS[i].single[usize::from(b - 0x80)],
            Kind::Utf8 => 0,
        };
        (c != 0).then_some(c)
    }

    /// A double byte's character, if assigned.
    pub fn double(self, lead: u8, trail: u8) -> Option<u16> {
        let d = self.dbcs()?;
        let row = usize::from(d.rows[usize::from(lead)]);
        if row == 0 || trail < d.lo || trail > d.hi {
            return None;
        }
        let width = usize::from(d.hi - d.lo) + 1;
        let at = ((row - 1) * width + usize::from(trail - d.lo)) * 2;
        let c = u16::from_le_bytes([d.cells[at], d.cells[at + 1]]);
        (c != 0).then_some(c)
    }

    /// Decode the character at the front of `bytes` (not empty).
    pub fn decode_one(self, bytes: &[u8]) -> Decoded {
        let b = bytes[0];
        match self.kind {
            Kind::Utf8 => decode_utf8(bytes),
            Kind::Dbcs(_) if self.is_lead(b) => {
                let Some(&t) = bytes.get(1) else { return Decoded::Incomplete };
                match self.double(b, t) {
                    Some(c) => Decoded::Char { units: [c, 0], len: 2 },
                    None if is_trail(self.id, t) => Decoded::Invalid { len: 2 },
                    None => Decoded::Invalid { len: 1 },
                }
            }
            _ => match self.byte(b) {
                Some(c) => Decoded::Char { units: [c, 0], len: 1 },
                None => Decoded::Invalid { len: 1 },
            },
        }
    }

    /// `MultiByteToWideChar`'s conversion: `Err(())` is invalid input when
    /// `strict` (`MB_ERR_INVALID_CHARS`).
    #[allow(clippy::result_unit_err)]
    pub fn decode(self, bytes: &[u8], strict: bool) -> Result<Vec<u16>, ()> {
        if self.is_utf8() {
            return utf8_to_utf16(bytes, strict);
        }
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match self.decode_one(&bytes[i..]) {
                Decoded::Char { units, len } => {
                    out.push(units[0]);
                    i += len;
                }
                Decoded::Incomplete if !strict => {
                    out.push(UNICODE_DEFAULT);
                    i += 1;
                }
                Decoded::Invalid { len } if !strict => {
                    out.push(UNICODE_DEFAULT);
                    i += len;
                }
                _ => return Err(()),
            }
        }
        Ok(out)
    }

    /// One UTF-16 unit in this code page (not UTF-8): a byte (< 0x100) or
    /// `lead << 8 | trail`, if the code page has the character.
    pub fn encode_unit(self, c: u16) -> Option<u16> {
        if c < 0x80 {
            return Some(c);
        }
        match self.kind {
            Kind::Sbcs(t) => t.iter().position(|&h| h == c).map(|i| 0x80 + i as u16),
            Kind::Dbcs(i) => {
                let r = reverse(i);
                r.binary_search_by_key(&c, |&(u, _)| u).ok().map(|k| r[k].1)
            }
            Kind::Utf8 => None,
        }
    }

    /// `WideCharToMultiByte`'s conversion (not UTF-8: see
    /// [`utf16_to_utf8`]): the bytes and whether `default` stood in for a
    /// character the code page lacks (a surrogate pair is one character).
    pub fn encode(self, units: &[u16], default: u8) -> (Vec<u8>, bool) {
        let mut out = Vec::with_capacity(units.len());
        let mut used = false;
        for (c, n) in chars(units) {
            let code = if n == 1 { c.and_then(|c| self.encode_unit(c as u16)) } else { None };
            match code {
                Some(code) if code >= 0x100 => out.extend_from_slice(&code.to_be_bytes()),
                Some(code) => out.push(code as u8),
                None => {
                    out.push(default);
                    used = true;
                }
            }
        }
        (out, used)
    }
}

/// Inclusive byte ranges.
pub type Ranges = &'static [(u8, u8)];

/// A double-byte code page's lead and trail byte ranges, from the small
/// table (no cells are linked by this).
pub fn dbcs_ranges(id: u32) -> Option<(Ranges, Ranges)> {
    tables::DBCS_INFO.iter().find(|d| u32::from(d.id) == id).map(|d| (d.leads, d.trails))
}

/// Whether `b` is a trail byte of DBCS code page `id`.
pub fn is_trail(id: u32, b: u8) -> bool {
    dbcs_ranges(id).is_some_and(|(_, t)| t.iter().any(|&(lo, hi)| (lo..=hi).contains(&b)))
}

/// Whether `b` is a lead byte of DBCS code page `id` (from the small table).
pub fn is_lead_byte(id: u32, b: u8) -> bool {
    dbcs_ranges(id).is_some_and(|(l, _)| l.iter().any(|&(lo, hi)| (lo..=hi).contains(&b)))
}

/// A double-byte code page's reverse index.
type Reverse = Box<[(u16, u16)]>;

/// The reverse index of DBCS table `i`: `(character, code)` sorted by
/// character, built on first use and kept.
fn reverse(i: usize) -> &'static [(u16, u16)] {
    static REVERSE: [AtomicPtr<Reverse>; 4] = [
        AtomicPtr::new(core::ptr::null_mut()),
        AtomicPtr::new(core::ptr::null_mut()),
        AtomicPtr::new(core::ptr::null_mut()),
        AtomicPtr::new(core::ptr::null_mut()),
    ];
    let slot = &REVERSE[i];
    let p = slot.load(Ordering::Acquire);
    if !p.is_null() {
        // SAFETY: a published index is never freed.
        return unsafe { &*p };
    }
    let built = Box::into_raw(Box::new(build_reverse(tables::DBCS[i])));
    match slot.compare_exchange(core::ptr::null_mut(), built, Ordering::AcqRel, Ordering::Acquire) {
        // SAFETY: just published, never freed.
        Ok(_) => unsafe { &*built },
        Err(other) => {
            // SAFETY: ours, never published.
            drop(unsafe { Box::from_raw(built) });
            // SAFETY: as above.
            unsafe { &*other }
        }
    }
}

fn build_reverse(d: &Dbcs) -> Reverse {
    let mut v: Vec<(u16, u8, u16)> = Vec::new();
    for b in 0x80..=0xFFu8 {
        let c = d.single[usize::from(b - 0x80)];
        if c != 0 {
            v.push((c, 0, u16::from(b)));
        }
    }
    let width = usize::from(d.hi - d.lo) + 1;
    for lead in 0..=255u8 {
        let row = usize::from(d.rows[usize::from(lead)]);
        if row == 0 {
            continue;
        }
        for k in 0..width {
            let at = ((row - 1) * width + k) * 2;
            let c = u16::from_le_bytes([d.cells[at], d.cells[at + 1]]);
            if c != 0 {
                let penalty = u8::from(d.info.id == 932 && matches!(lead, 0x87 | 0xED | 0xEE));
                v.push((c, penalty, u16::from(lead) << 8 | (u16::from(d.lo) + k as u16)));
            }
        }
    }
    v.sort_unstable();
    v.dedup_by_key(|e| e.0);
    v.into_iter().map(|(c, _, code)| (c, code)).collect()
}

// ----- UTF-8 ---------------------------------------------------------------------------

/// The length and valid second-byte range of a UTF-8 lead byte (Unicode's
/// table of well-formed UTF-8 byte sequences); length 0: not a lead.
fn utf8_lead(b: u8) -> (usize, u8, u8) {
    match b {
        0xC2..=0xDF => (2, 0x80, 0xBF),
        0xE0 => (3, 0xA0, 0xBF),
        0xE1..=0xEC | 0xEE..=0xEF => (3, 0x80, 0xBF),
        0xED => (3, 0x80, 0x9F),
        0xF0 => (4, 0x90, 0xBF),
        0xF1..=0xF3 => (4, 0x80, 0xBF),
        0xF4 => (4, 0x80, 0x8F),
        _ => (0, 0, 0),
    }
}

/// How many bytes of a UTF-8 sequence starting at `bytes[0]` are well
/// formed so far, and the full length (0: `bytes[0]` is no lead byte).
fn utf8_prefix(bytes: &[u8]) -> (usize, usize) {
    let (len, lo, hi) = utf8_lead(bytes[0]);
    if len == 0 {
        return (0, 0);
    }
    let mut good = 1;
    while good < len && good < bytes.len() {
        let c = bytes[good];
        let ok = if good == 1 { (lo..=hi).contains(&c) } else { (0x80..=0xBF).contains(&c) };
        if !ok {
            break;
        }
        good += 1;
    }
    (good, len)
}

fn utf8_value(bytes: &[u8]) -> u32 {
    let len = bytes.len();
    let mut cp = u32::from(bytes[0]) & (0x7F >> len);
    for &c in &bytes[1..] {
        cp = (cp << 6) | u32::from(c & 0x3F);
    }
    cp
}

fn units_of(cp: u32) -> [u16; 2] {
    if cp >= 0x1_0000 {
        let v = cp - 0x1_0000;
        [0xD800 | (v >> 10) as u16, 0xDC00 | (v & 0x3FF) as u16]
    } else {
        [cp as u16, 0]
    }
}

/// Decode the UTF-8 character at the front of `bytes` (not empty).
pub fn decode_utf8(bytes: &[u8]) -> Decoded {
    let b = bytes[0];
    if b < 0x80 {
        return Decoded::Char { units: [u16::from(b), 0], len: 1 };
    }
    let (good, len) = utf8_prefix(bytes);
    if len == 0 {
        return Decoded::Invalid { len: 1 };
    }
    if good == len {
        return Decoded::Char { units: units_of(utf8_value(&bytes[..len])), len };
    }
    if good == bytes.len() { Decoded::Incomplete } else { Decoded::Invalid { len: good.max(1) } }
}

/// UTF-8 to UTF-16: one U+FFFD per maximal invalid subpart, or `Err(())`
/// when `strict`.
#[allow(clippy::result_unit_err)]
pub fn utf8_to_utf16(bytes: &[u8], strict: bool) -> Result<Vec<u16>, ()> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x80 {
            out.push(u16::from(b));
            i += 1;
            continue;
        }
        let (good, len) = utf8_prefix(&bytes[i..]);
        if len == 0 || good < len {
            if strict {
                return Err(());
            }
            out.push(0xFFFD);
            i += good.max(1);
            continue;
        }
        let u = units_of(utf8_value(&bytes[i..i + len]));
        out.push(u[0]);
        if u[1] != 0 {
            out.push(u[1]);
        }
        i += len;
    }
    Ok(out)
}

/// The characters (code points, `None` for a lone surrogate) of UTF-16
/// and how many units each took.
pub fn chars(units: &[u16]) -> impl Iterator<Item = (Option<u32>, usize)> + '_ {
    let mut i = 0;
    core::iter::from_fn(move || {
        let c = *units.get(i)?;
        let next = units.get(i + 1).copied();
        if (0xD800..0xDC00).contains(&c)
            && let Some(n) = next.filter(|n| (0xDC00..0xE000).contains(n))
        {
            i += 2;
            return Some((Some(0x1_0000 + ((u32::from(c) - 0xD800) << 10) + (u32::from(n) - 0xDC00)), 2));
        }
        i += 1;
        if (0xD800..0xE000).contains(&c) { Some((None, 1)) } else { Some((Some(u32::from(c)), 1)) }
    })
}

/// UTF-16 to UTF-8: a lone surrogate is U+FFFD, or `Err(())` when `strict`.
#[allow(clippy::result_unit_err)]
pub fn utf16_to_utf8(units: &[u16], strict: bool) -> Result<Vec<u8>, ()> {
    let mut out = Vec::with_capacity(units.len());
    for (c, _) in chars(units) {
        let cp = match c {
            Some(c) => c,
            None if strict => return Err(()),
            None => 0xFFFD,
        };
        let mut buf = [0u8; 4];
        let s = char::from_u32(cp).unwrap_or('\u{FFFD}').encode_utf8(&mut buf);
        out.extend_from_slice(s.as_bytes());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn the_code_pages() {
        let ids = CodePage::ids();
        for id in [437, 850, 852, 866, 874, 932, 936, 949, 950, 1250, 1251, 1252, 1253, 1257, 1258, 65001] {
            assert!(ids.contains(&id), "{id}");
            assert_eq!(CodePage::get(id).unwrap().id(), id);
        }
        assert!(CodePage::get(0).is_none() && CodePage::get(12345).is_none() && CodePage::get(65000).is_none());
        assert_eq!(CodePage::get(932).unwrap().max_char_size(), 2);
        assert_eq!(CodePage::get(850).unwrap().max_char_size(), 1);
        assert_eq!(CodePage::get(65001).unwrap().max_char_size(), 4);
        assert_eq!(CodePage::get(932).unwrap().lead_ranges(), &[(0x81, 0x9F), (0xE0, 0xFC)]);
        assert_eq!(CodePage::get(936).unwrap().lead_ranges(), &[(0x81, 0xFE)]);
        assert!(CodePage::get(1252).unwrap().lead_ranges().is_empty());
    }

    #[test]
    fn single_byte_pages_round_trip() {
        for id in CodePage::ids() {
            let cp = CodePage::get(id).unwrap();
            if cp.is_dbcs() || cp.is_utf8() {
                continue;
            }
            for b in 0..=255u8 {
                if let Some(c) = cp.byte(b) {
                    assert_eq!(cp.encode_unit(c), Some(u16::from(b)), "{id}: {b:#x}");
                }
            }
        }
        let cp850 = CodePage::get(850).unwrap();
        assert_eq!(cp850.byte(0x82), Some(0xE9)); // é
        assert_eq!(cp850.byte(0xD5), Some(0x0131)); // ı
        assert_eq!(cp850.encode(&w("\u{E4}\u{20AC}"), b'?'), (vec![0x84, b'?'], true));
        let cp1251 = CodePage::get(1251).unwrap();
        assert_eq!(cp1251.decode(b"\xCF\xF0\xE8", false).unwrap(), w("При"));
        // 1253 leaves 0xAA unassigned.
        let cp1253 = CodePage::get(1253).unwrap();
        assert_eq!(cp1253.decode(b"\xAA", false).unwrap(), [0x3F]);
        assert_eq!(cp1253.decode(b"\xAA", true), Err(()));
        // 1252's unassigned bytes are C1 controls (ADR 0079).
        assert_eq!(CodePage::get(1252).unwrap().decode(b"\x81\x80", true).unwrap(), [0x81, 0x20AC]);
    }

    #[test]
    fn double_byte_pages() {
        let sjis = CodePage::get(932).unwrap();
        // "日本" in Shift-JIS, halfwidth katakana ｱ, ASCII.
        let bytes = b"\x93\xFA\x96\x7B\xB1A";
        let u = sjis.decode(bytes, true).unwrap();
        assert_eq!(u, w("日本\u{FF71}A"));
        assert_eq!(sjis.encode(&u, b'?'), (bytes.to_vec(), false));
        assert!(sjis.is_lead(0x93) && !sjis.is_lead(0xB1) && !sjis.is_lead(b'A'));
        // A lead byte at the end, or before a byte that is no trail byte.
        assert_eq!(sjis.decode(b"A\x93", false).unwrap(), w("A?"));
        assert_eq!(sjis.decode(b"\x93\x0A", false).unwrap(), w("?\n"));
        assert_eq!(sjis.decode(b"\x93", true), Err(()));
        assert_eq!(sjis.decode_one(b"\x93"), Decoded::Incomplete);
        assert_eq!(sjis.decode_one(b"\x93\xFA"), Decoded::Char { units: [0x65E5, 0], len: 2 });
        // NEC row 13's ≒ (0x8790) duplicates JIS 0x81E0: back to 0x81E0.
        assert_eq!(sjis.double(0x87, 0x90), Some(0x2252));
        assert_eq!(sjis.encode_unit(0x2252), Some(0x81E0));
        let gbk = CodePage::get(936).unwrap();
        assert_eq!(gbk.decode(b"\xC4\xE3\xBA\xC3\x80", true).unwrap(), w("你好€"));
        assert_eq!(gbk.encode(&w("你好"), b'?').0, b"\xC4\xE3\xBA\xC3");
        let uhc = CodePage::get(949).unwrap();
        assert_eq!(uhc.decode(b"\xC7\xD1\xB1\xDB", true).unwrap(), w("한글"));
        let big5 = CodePage::get(950).unwrap();
        assert_eq!(big5.decode(b"\xA4\xA4\xA4\xE5", true).unwrap(), w("中文"));
        assert_eq!(big5.encode(&w("中\u{1F600}"), b'#'), (b"\xA4\xA4#".to_vec(), true));
        assert!(is_trail(949, 0x41) && !is_trail(949, 0x5B) && is_lead_byte(950, 0xA4) && !is_lead_byte(950, 0x80));
    }

    #[test]
    fn utf8() {
        let s = "a\u{E9}\u{20AC}\u{1F600}";
        let u = utf8_to_utf16(s.as_bytes(), true).unwrap();
        assert_eq!(u, w(s));
        assert_eq!(utf16_to_utf8(&u, true).unwrap(), s.as_bytes());
        let bad = [b'x', 0xC0, 0x80, 0xE2, 0x82, b'y'];
        assert_eq!(utf8_to_utf16(&bad, false).unwrap(), [0x78, 0xFFFD, 0xFFFD, 0xFFFD, 0x79]);
        let cp = CodePage::get(CP_UTF8).unwrap();
        assert_eq!(cp.decode_one(b"\xE2\x82"), Decoded::Incomplete);
        assert_eq!(cp.decode_one(b"\xE2\x82\xAC"), Decoded::Char { units: [0x20AC, 0], len: 3 });
        assert_eq!(cp.decode_one(b"\xF0\x9F\x98\x80"), Decoded::Char { units: [0xD83D, 0xDE00], len: 4 });
        assert_eq!(cp.decode_one(b"\xE2\x41"), Decoded::Invalid { len: 1 });
        assert_eq!(utf16_to_utf8(&[0xD800], true), Err(()));
    }
}
