//! The console's code pages (`GetConsoleCP`, `SetConsoleOutputCP`, Learn:
//! "Code Page Identifiers"): what `ReadFile`, `ReadConsoleA`, `WriteFile`
//! and `WriteConsoleA` convert through.
//!
//! Poly serves four: UTF-8 (65001), the IBM PC set (437, the OEM code page
//! of US Windows and so a new console's code page), Windows-1252 (the ANSI
//! code page) and US-ASCII (20127). Others are refused by
//! `SetConsoleCP` / `SetConsoleOutputCP` with `ERROR_INVALID_PARAMETER`
//! (Poly's choice: Learn only says the identifier must be valid; to be
//! confirmed by the spec agent).
//!
//! Single-byte pages map bytes 0x00–0x7F to themselves (the control
//! characters too: as `MultiByteToWideChar` does without
//! `MB_USEGLYPHCHARS`); a character a page cannot hold becomes `?`.
//! Malformed UTF-8 becomes U+FFFD; a sequence split between two writes is
//! kept by [`Utf8Decoder`] for the next one.

use crate::consts::*;
use alloc::vec::Vec;

/// Code page 437, bytes 0x80–0xFF (the IBM PC character set).
const CP437_HIGH: [u16; 128] = [
    0x00C7, 0x00FC, 0x00E9, 0x00E2, 0x00E4, 0x00E0, 0x00E5, 0x00E7, 0x00EA, 0x00EB, 0x00E8, 0x00EF, 0x00EE, 0x00EC,
    0x00C4, 0x00C5, 0x00C9, 0x00E6, 0x00C6, 0x00F4, 0x00F6, 0x00F2, 0x00FB, 0x00F9, 0x00FF, 0x00D6, 0x00DC, 0x00A2,
    0x00A3, 0x00A5, 0x20A7, 0x0192, 0x00E1, 0x00ED, 0x00F3, 0x00FA, 0x00F1, 0x00D1, 0x00AA, 0x00BA, 0x00BF, 0x2310,
    0x00AC, 0x00BD, 0x00BC, 0x00A1, 0x00AB, 0x00BB, 0x2591, 0x2592, 0x2593, 0x2502, 0x2524, 0x2561, 0x2562, 0x2556,
    0x2555, 0x2563, 0x2551, 0x2557, 0x255D, 0x255C, 0x255B, 0x2510, 0x2514, 0x2534, 0x252C, 0x251C, 0x2500, 0x253C,
    0x255E, 0x255F, 0x255A, 0x2554, 0x2569, 0x2566, 0x2560, 0x2550, 0x256C, 0x2567, 0x2568, 0x2564, 0x2565, 0x2559,
    0x2558, 0x2552, 0x2553, 0x256B, 0x256A, 0x2518, 0x250C, 0x2588, 0x2584, 0x258C, 0x2590, 0x2580, 0x03B1, 0x00DF,
    0x0393, 0x03C0, 0x03A3, 0x03C3, 0x00B5, 0x03C4, 0x03A6, 0x0398, 0x03A9, 0x03B4, 0x221E, 0x03C6, 0x03B5, 0x2229,
    0x2261, 0x00B1, 0x2265, 0x2264, 0x2320, 0x2321, 0x00F7, 0x2248, 0x00B0, 0x2219, 0x00B7, 0x221A, 0x207F, 0x00B2,
    0x25A0, 0x00A0,
];

/// Windows-1252, bytes 0x80–0x9F (0xA0–0xFF are Latin-1). The five bytes
/// the set leaves undefined map to the C1 control of the same value.
const CP1252_C1: [u16; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D,
    0x017D, 0x008F, 0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A,
    0x0153, 0x009D, 0x017E, 0x0178,
];

/// The glyphs the IBM PC set shows for the control characters 0x01–0x1F,
/// which the console shows for a control character it does not process.
pub const CONTROL_GLYPHS: [u16; 32] = [
    0x0020, 0x263A, 0x263B, 0x2665, 0x2666, 0x2663, 0x2660, 0x2022, 0x25D8, 0x25CB, 0x25D9, 0x2642, 0x2640, 0x266A,
    0x266B, 0x263C, 0x25BA, 0x25C4, 0x2195, 0x203C, 0x00B6, 0x00A7, 0x25AC, 0x21A8, 0x2191, 0x2193, 0x2192, 0x2190,
    0x221F, 0x2194, 0x25B2, 0x25BC,
];

/// Whether Poly's console serves code page `cp`.
pub fn is_supported(cp: u32) -> bool {
    matches!(cp, CP_437 | CP_1252 | CP_US_ASCII | CP_UTF8)
}

/// One byte of a single-byte page as UTF-16.
fn single_byte(cp: u32, b: u8) -> u16 {
    match (cp, b) {
        (_, 0..=0x7F) => u16::from(b),
        (CP_437, _) => CP437_HIGH[usize::from(b - 0x80)],
        (CP_1252, 0x80..=0x9F) => CP1252_C1[usize::from(b - 0x80)],
        (CP_1252, _) => u16::from(b),
        _ => u16::from(b'?'),
    }
}

/// `c` in single-byte page `cp`, if the page has it.
fn to_single_byte(cp: u32, c: u16) -> Option<u8> {
    if c < 0x80 {
        return Some(c as u8);
    }
    match cp {
        CP_437 => CP437_HIGH.iter().position(|&x| x == c).map(|i| 0x80 + i as u8),
        CP_1252 => {
            if (0xA0..=0xFF).contains(&c) {
                Some(c as u8)
            } else {
                CP1252_C1.iter().position(|&x| x == c).map(|i| 0x80 + i as u8)
            }
        }
        _ => None,
    }
}

/// UTF-16 in code page `cp`. Unpaired surrogates and characters the page
/// lacks become `?` (U+FFFD's UTF-8 for an unpaired surrogate in UTF-8).
pub fn encode(cp: u32, text: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    if cp == CP_UTF8 {
        for c in char::decode_utf16(text.iter().copied()) {
            let c = c.unwrap_or(char::REPLACEMENT_CHARACTER);
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
        return out;
    }
    for &u in text {
        out.push(to_single_byte(cp, u).unwrap_or(b'?'));
    }
    out
}

/// One character (one or two UTF-16 units) in code page `cp`.
pub fn encode_char(cp: u32, units: &[u16]) -> Vec<u8> {
    encode(cp, units)
}

/// Bytes in code page `cp` as UTF-16 (a whole buffer; a UTF-8 sequence cut
/// at the end becomes U+FFFD).
pub fn decode(cp: u32, bytes: &[u8]) -> Vec<u16> {
    let mut d = Utf8Decoder::default();
    let mut out = d.decode(cp, bytes);
    out.extend(d.finish());
    out
}

/// A stream decoder: a UTF-8 sequence split across calls is completed by
/// the next call.
#[derive(Clone, Debug, Default)]
pub struct Utf8Decoder {
    pending: Vec<u8>,
}

impl Utf8Decoder {
    /// Decode `bytes` in `cp`, keeping an unfinished UTF-8 sequence.
    pub fn decode(&mut self, cp: u32, bytes: &[u8]) -> Vec<u16> {
        if cp != CP_UTF8 {
            let mut out: Vec<u16> = self.finish();
            out.extend(bytes.iter().map(|&b| single_byte(cp, b)));
            return out;
        }
        let mut data = core::mem::take(&mut self.pending);
        data.extend_from_slice(bytes);
        let mut out = Vec::with_capacity(data.len());
        let mut i = 0;
        while i < data.len() {
            let b = data[i];
            let need = match b {
                0x00..=0x7F => 1,
                0xC2..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF4 => 4,
                _ => 0,
            };
            if need == 0 {
                out.push(0xFFFD);
                i += 1;
                continue;
            }
            if i + need > data.len() {
                // Unfinished: keep it if what is there could still be valid.
                if data[i + 1..].iter().all(|&c| c & 0xC0 == 0x80) {
                    self.pending = data[i..].to_vec();
                    return out;
                }
            }
            match core::str::from_utf8(&data[i..(i + need).min(data.len())]) {
                Ok(s) => {
                    for c in s.chars() {
                        let mut buf = [0u16; 2];
                        out.extend_from_slice(c.encode_utf16(&mut buf));
                    }
                    i += need;
                }
                Err(_) => {
                    out.push(0xFFFD);
                    i += 1;
                    while i < data.len() && data[i] & 0xC0 == 0x80 {
                        i += 1;
                    }
                }
            }
        }
        out
    }

    /// Whatever is pending, as U+FFFD (nothing if nothing is).
    pub fn finish(&mut self) -> Vec<u16> {
        if core::mem::take(&mut self.pending).is_empty() { Vec::new() } else { alloc::vec![0xFFFD] }
    }

    /// Whether a sequence is waiting for its next bytes.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::String;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn cp437_round_trips_its_high_half() {
        for b in 0x80..=0xFFu8 {
            let u = single_byte(CP_437, b);
            assert_eq!(to_single_byte(CP_437, u), Some(b), "byte {b:#x}");
        }
        assert_eq!(decode(CP_437, &[0x82, 0xB0, 0xE1, 0xFE]), w("é░ß■"));
        assert_eq!(encode(CP_437, &w("Ä€")), [0x8E, b'?']);
    }

    #[test]
    fn cp1252_and_ascii() {
        assert_eq!(decode(CP_1252, &[0x80, 0xE9, 0x81]), [0x20AC, 0xE9, 0x81]);
        assert_eq!(encode(CP_1252, &w("€é─")), [0x80, 0xE9, b'?']);
        assert_eq!(encode(CP_US_ASCII, &w("aé")), [b'a', b'?']);
        assert_eq!(decode(CP_US_ASCII, b"ok"), w("ok"));
    }

    #[test]
    fn utf8_split_sequences_are_completed() {
        let s = "a€😀b";
        let bytes = s.as_bytes();
        for cut in 0..=bytes.len() {
            let mut d = Utf8Decoder::default();
            let mut out = d.decode(CP_UTF8, &bytes[..cut]);
            out.extend(d.decode(CP_UTF8, &bytes[cut..]));
            assert!(!d.has_pending());
            assert_eq!(String::from_utf16(&out).unwrap(), s, "cut at {cut}");
        }
    }

    #[test]
    fn utf8_errors_become_replacement_characters() {
        assert_eq!(decode(CP_UTF8, &[b'x', 0xFF, b'y']), [u16::from(b'x'), 0xFFFD, u16::from(b'y')]);
        assert_eq!(decode(CP_UTF8, &[0xE2, 0x82]), [0xFFFD]);
        assert_eq!(decode(CP_UTF8, &[0xE2, b'a']), [0xFFFD, u16::from(b'a')]);
        assert_eq!(encode(CP_UTF8, &[0xD800]), "\u{FFFD}".as_bytes());
        assert_eq!(encode(CP_UTF8, &w("ü😀")), "ü😀".as_bytes());
    }

    #[test]
    fn supported_pages() {
        assert!(is_supported(65001) && is_supported(437) && is_supported(1252) && is_supported(20127));
        assert!(!is_supported(0) && !is_supported(1) && !is_supported(12345));
    }
}
