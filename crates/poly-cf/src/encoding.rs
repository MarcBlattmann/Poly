//! String encodings: bytes to UTF-16 code units and back.
//!
//! The encoding numbers are those of `<CoreFoundation/CFString.h>` and
//! `<CoreFoundation/CFStringEncodingExt.h>`. The single-byte tables are the
//! public mapping tables of the Unicode Consortium
//! (`MAPPINGS/VENDORS/APPLE/ROMAN.TXT`, `MICSFT/WINDOWS/CP1252.TXT`,
//! `NEXT/NEXTSTEP.TXT`). What the functions accept and refuse follows the
//! CoreFoundation spec sheets (`CFStringCreateWithCString`,
//! `CFStringCreateWithBytes`, `CFStringGetCString`).

use alloc::vec::Vec;

pub const MAC_ROMAN: u32 = 0;
pub const WINDOWS_LATIN1: u32 = 0x0500;
pub const ISO_LATIN1: u32 = 0x0201;
pub const NEXTSTEP_LATIN: u32 = 0x0B01;
pub const ASCII: u32 = 0x0600;
/// `kCFStringEncodingUnicode`, also `kCFStringEncodingUTF16`.
pub const UTF16: u32 = 0x0100;
pub const UTF8: u32 = 0x0800_0100;
pub const NON_LOSSY_ASCII: u32 = 0x0BFF;
pub const UTF16BE: u32 = 0x1000_0100;
pub const UTF16LE: u32 = 0x1400_0100;
pub const UTF32: u32 = 0x0c00_0100;
pub const UTF32BE: u32 = 0x1800_0100;
pub const UTF32LE: u32 = 0x1c00_0100;
/// `kCFStringEncodingInvalidId`.
pub const INVALID: u32 = 0xffff_ffff;

/// Mac OS Roman, bytes 0x80 to 0xFF.
const MAC_ROMAN_HIGH: [u16; 128] = [
    0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1, 0x00E0, 0x00E2, 0x00E4, 0x00E3, 0x00E5, 0x00E7,
    0x00E9, 0x00E8, 0x00EA, 0x00EB, 0x00ED, 0x00EC, 0x00EE, 0x00EF, 0x00F1, 0x00F3, 0x00F2, 0x00F4, 0x00F6, 0x00F5,
    0x00FA, 0x00F9, 0x00FB, 0x00FC, 0x2020, 0x00B0, 0x00A2, 0x00A3, 0x00A7, 0x2022, 0x00B6, 0x00DF, 0x00AE, 0x00A9,
    0x2122, 0x00B4, 0x00A8, 0x2260, 0x00C6, 0x00D8, 0x221E, 0x00B1, 0x2264, 0x2265, 0x00A5, 0x00B5, 0x2202, 0x2211,
    0x220F, 0x03C0, 0x222B, 0x00AA, 0x00BA, 0x03A9, 0x00E6, 0x00F8, 0x00BF, 0x00A1, 0x00AC, 0x221A, 0x0192, 0x2248,
    0x2206, 0x00AB, 0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3, 0x00D5, 0x0152, 0x0153, 0x2013, 0x2014, 0x201C, 0x201D,
    0x2018, 0x2019, 0x00F7, 0x25CA, 0x00FF, 0x0178, 0x2044, 0x20AC, 0x2039, 0x203A, 0xFB01, 0xFB02, 0x2021, 0x00B7,
    0x201A, 0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1, 0x00CB, 0x00C8, 0x00CD, 0x00CE, 0x00CF, 0x00CC, 0x00D3, 0x00D4,
    0xF8FF, 0x00D2, 0x00DA, 0x00DB, 0x00D9, 0x0131, 0x02C6, 0x02DC, 0x00AF, 0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD,
    0x02DB, 0x02C7,
];

/// Windows code page 1252, bytes 0x80 to 0x9F (the rest is Latin 1). The
/// five bytes the code page leaves undefined map to the C1 control of the
/// same value.
const CP1252_C1: [u16; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D,
    0x017D, 0x008F, 0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A,
    0x0153, 0x009D, 0x017E, 0x0178,
];

/// NeXTSTEP Latin, bytes 0x80 to 0xFF (0xFE and 0xFF are unassigned).
const NEXTSTEP_HIGH: [u16; 128] = [
    0x00A0, 0x00C0, 0x00C1, 0x00C2, 0x00C3, 0x00C4, 0x00C5, 0x00C7, 0x00C8, 0x00C9, 0x00CA, 0x00CB, 0x00CC, 0x00CD,
    0x00CE, 0x00CF, 0x00D0, 0x00D1, 0x00D2, 0x00D3, 0x00D4, 0x00D5, 0x00D6, 0x00D9, 0x00DA, 0x00DB, 0x00DC, 0x00DD,
    0x00DE, 0x00B5, 0x00D7, 0x00F7, 0x00A9, 0x00A1, 0x00A2, 0x00A3, 0x2044, 0x00A5, 0x0192, 0x00A7, 0x00A4, 0x2019,
    0x201C, 0x00AB, 0x2039, 0x203A, 0xFB01, 0xFB02, 0x00AE, 0x2013, 0x2020, 0x2021, 0x00B7, 0x00A6, 0x00B6, 0x2022,
    0x201A, 0x201E, 0x201D, 0x00BB, 0x2026, 0x2030, 0x00AC, 0x00BF, 0x00B9, 0x02CB, 0x00B4, 0x02C6, 0x02DC, 0x00AF,
    0x02D8, 0x02D9, 0x00A8, 0x00B2, 0x02DA, 0x00B8, 0x00B3, 0x02DD, 0x02DB, 0x02C7, 0x2014, 0x00B1, 0x00BC, 0x00BD,
    0x00BE, 0x00E0, 0x00E1, 0x00E2, 0x00E3, 0x00E4, 0x00E5, 0x00E7, 0x00E8, 0x00E9, 0x00EA, 0x00EB, 0x00EC, 0x00C6,
    0x00ED, 0x00AA, 0x00EE, 0x00EF, 0x00F0, 0x00F1, 0x0141, 0x00D8, 0x0152, 0x00BA, 0x00F2, 0x00F3, 0x00F4, 0x00F5,
    0x00F6, 0x00E6, 0x00F9, 0x00FA, 0x00FB, 0x0131, 0x00FC, 0x00FD, 0x0142, 0x00F8, 0x0153, 0x00DF, 0x00FE, 0x00FF,
    0xFFFD, 0xFFFD,
];

/// The table of a single-byte encoding for bytes 0x80 and above, or `None`
/// when the encoding is not a table one.
fn high_table(enc: u32) -> Option<&'static [u16; 128]> {
    match enc {
        MAC_ROMAN => Some(&MAC_ROMAN_HIGH),
        NEXTSTEP_LATIN => Some(&NEXTSTEP_HIGH),
        _ => None,
    }
}

/// Whether CoreFoundation knows `enc` (what `CFStringIsEncodingAvailable`
/// answers).
pub fn is_available(enc: u32) -> bool {
    matches!(
        enc,
        MAC_ROMAN
            | WINDOWS_LATIN1
            | ISO_LATIN1
            | NEXTSTEP_LATIN
            | ASCII
            | UTF16
            | UTF8
            | NON_LOSSY_ASCII
            | UTF16BE
            | UTF16LE
            | UTF32
            | UTF32BE
            | UTF32LE
    )
}

/// One byte of a single-byte encoding as a code unit.
pub fn byte_to_unit(b: u8, enc: u32) -> u16 {
    if b < 0x80 {
        return u16::from(b);
    }
    match enc {
        WINDOWS_LATIN1 if b < 0xA0 => CP1252_C1[usize::from(b - 0x80)],
        _ => high_table(enc).map_or(u16::from(b), |t| t[usize::from(b - 0x80)]),
    }
}

/// One code unit as a byte of a single-byte encoding, if it has one.
pub fn unit_to_byte(u: u16, enc: u32) -> Option<u8> {
    if u < 0x80 {
        return Some(u as u8);
    }
    match enc {
        ASCII | NON_LOSSY_ASCII => None,
        ISO_LATIN1 => (u <= 0xFF).then_some(u as u8),
        WINDOWS_LATIN1 => {
            if let Some(i) = CP1252_C1.iter().position(|&c| c == u) {
                return Some(0x80 + i as u8);
            }
            (0xA0..=0xFF).contains(&u).then_some(u as u8)
        }
        _ => {
            let t = high_table(enc)?;
            if u == 0xFFFD {
                return None;
            }
            t.iter().position(|&c| c == u).map(|i| 0x80 + i as u8)
        }
    }
}

/// Whether `enc` is one byte per character.
pub fn is_single_byte(enc: u32) -> bool {
    matches!(enc, MAC_ROMAN | WINDOWS_LATIN1 | ISO_LATIN1 | NEXTSTEP_LATIN | ASCII | NON_LOSSY_ASCII)
}

/// Strict UTF-8: no overlong forms, no encoded surrogates, nothing past
/// U+10FFFF, no truncated sequence. A byte order mark at the start is
/// dropped; one elsewhere stays U+FEFF.
pub fn decode_utf8(bytes: &[u8]) -> Option<Vec<u16>> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x80 {
            out.push(u16::from(b));
            i += 1;
            continue;
        }
        let (len, min, init) = match b {
            0xC2..=0xDF => (2, 0x80, u32::from(b & 0x1F)),
            0xE0..=0xEF => (3, 0x800, u32::from(b & 0x0F)),
            0xF0..=0xF4 => (4, 0x10000, u32::from(b & 0x07)),
            _ => return None,
        };
        if i + len > bytes.len() {
            return None;
        }
        let mut c = init;
        for &cont in &bytes[i + 1..i + len] {
            if cont & 0xC0 != 0x80 {
                return None;
            }
            c = (c << 6) | u32::from(cont & 0x3F);
        }
        if c < min || c > 0x10FFFF || (0xD800..=0xDFFF).contains(&c) {
            return None;
        }
        push_scalar(&mut out, c);
        i += len;
    }
    Some(out)
}

/// Append code point `c` as one unit or a surrogate pair.
pub fn push_scalar(out: &mut Vec<u16>, c: u32) {
    if c >= 0x10000 {
        let v = c - 0x10000;
        out.push(0xD800 | (v >> 10) as u16);
        out.push(0xDC00 | (v & 0x3FF) as u16);
    } else {
        out.push(c as u16);
    }
}

/// Decode `bytes` in `enc` (`CFStringCreateWithBytes`); `None` when they are
/// not valid in it or the encoding is unknown.
pub fn decode(bytes: &[u8], enc: u32, external: bool) -> Option<Vec<u16>> {
    match enc {
        UTF8 => decode_utf8(bytes),
        UTF16 | UTF16BE | UTF16LE => {
            if bytes.len() == 1 {
                return None;
            }
            let (mut big, mut body) = (enc == UTF16BE, bytes);
            if enc == UTF16 && external {
                // A byte order mark decides and is consumed; without one an
                // external representation is big endian.
                big = true;
                if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
                    big = false;
                    body = rest;
                } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
                    body = rest;
                }
            }
            Some(
                body.chunks_exact(2)
                    .map(|p| if big { u16::from_be_bytes([p[0], p[1]]) } else { u16::from_le_bytes([p[0], p[1]]) })
                    .collect(),
            )
        }
        UTF32 | UTF32BE | UTF32LE => {
            let (mut big, mut body) = (enc == UTF32BE, bytes);
            if enc == UTF32 && external {
                big = true;
                if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE, 0, 0]) {
                    big = false;
                    body = rest;
                } else if let Some(rest) = bytes.strip_prefix(&[0, 0, 0xFE, 0xFF]) {
                    body = rest;
                }
            }
            if body.len() % 4 != 0 {
                return None;
            }
            let mut out = Vec::with_capacity(body.len() / 4);
            for q in body.chunks_exact(4) {
                let c = if big {
                    u32::from_be_bytes([q[0], q[1], q[2], q[3]])
                } else {
                    u32::from_le_bytes([q[0], q[1], q[2], q[3]])
                };
                if c > 0x10FFFF || (0xD800..=0xDFFF).contains(&c) {
                    return None;
                }
                push_scalar(&mut out, c);
            }
            Some(out)
        }
        _ if is_single_byte(enc) => Some(bytes.iter().map(|&b| byte_to_unit(b, enc)).collect()),
        _ => None,
    }
}

/// Encode all of `units` in `enc` into `out` (no terminator); `false` when
/// some character has no form in the encoding (`out` may then hold part).
/// `bom`: the plain UTF-16 and UTF-32 forms start with a byte order mark
/// (little endian, the host order).
pub fn encode(units: &[u16], enc: u32, bom: bool, out: &mut Vec<u8>) -> bool {
    match enc {
        UTF8 => {
            let mut i = 0;
            while i < units.len() {
                let Some((c, n)) = scalar_at(units, i) else { return false };
                i += n;
                let mut buf = [0u8; 4];
                out.extend_from_slice(char::from_u32(c).map_or(&[][..], |ch| ch.encode_utf8(&mut buf).as_bytes()));
            }
            true
        }
        UTF16 | UTF16LE | UTF16BE => {
            if enc == UTF16 && bom {
                out.extend_from_slice(&[0xFF, 0xFE]);
            }
            for &u in units {
                out.extend_from_slice(&if enc == UTF16BE { u.to_be_bytes() } else { u.to_le_bytes() });
            }
            true
        }
        UTF32 | UTF32LE | UTF32BE => {
            if enc == UTF32 && bom {
                out.extend_from_slice(&[0xFF, 0xFE, 0, 0]);
            }
            let mut i = 0;
            while i < units.len() {
                let Some((c, n)) = scalar_at(units, i) else { return false };
                i += n;
                out.extend_from_slice(&if enc == UTF32BE { c.to_be_bytes() } else { c.to_le_bytes() });
            }
            true
        }
        _ if is_single_byte(enc) => {
            for &u in units {
                match unit_to_byte(u, enc) {
                    Some(b) => out.push(b),
                    None => return false,
                }
            }
            true
        }
        _ => false,
    }
}

/// The code point at `i` and how many units it takes; `None` for a lone
/// surrogate.
pub fn scalar_at(units: &[u16], i: usize) -> Option<(u32, usize)> {
    let u = units[i];
    match u {
        0xD800..=0xDBFF => {
            let lo = *units.get(i + 1)?;
            if !(0xDC00..=0xDFFF).contains(&lo) {
                return None;
            }
            Some((0x10000 + ((u32::from(u) - 0xD800) << 10) + (u32::from(lo) - 0xDC00), 2))
        }
        0xDC00..=0xDFFF => None,
        _ => Some((u32::from(u), 1)),
    }
}

/// Lossy UTF-8 for messages and paths: a lone surrogate becomes U+FFFD.
pub fn to_utf8_lossy(units: &[u16], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < units.len() {
        let (c, n) = scalar_at(units, i).unwrap_or((0xFFFD, 1));
        i += n;
        let mut buf = [0u8; 4];
        out.extend_from_slice(char::from_u32(c).unwrap_or('\u{FFFD}').encode_utf8(&mut buf).as_bytes());
    }
}

/// `CFStringGetMaximumSizeForEncoding`: a safe byte count for `n` units.
pub fn max_size(n: i64, enc: u32) -> i64 {
    match enc {
        UTF8 => n.saturating_mul(3),
        UTF16 | UTF16BE | UTF16LE => n.saturating_mul(2),
        UTF32 | UTF32BE | UTF32LE => n.saturating_mul(4),
        NON_LOSSY_ASCII => n.saturating_mul(6),
        _ if is_single_byte(enc) => n,
        _ => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn utf8_is_strict() {
        assert_eq!(decode_utf8(b"h\xc3\xa9llo").unwrap(), vec![0x68, 0xe9, 0x6c, 0x6c, 0x6f]);
        assert_eq!(decode_utf8(b"\xf0\x9f\x98\x80").unwrap(), vec![0xd83d, 0xde00]);
        for bad in [
            &b"\xff\xfe"[..],
            b"\x80",
            b"ab\xc3",
            b"\xc0\x80",
            b"\xe0\x80\x80",
            b"\xed\xa0\x80",
            b"\xf4\x90\x80\x80",
            b"\xf0\x9f\x98",
        ] {
            assert!(decode_utf8(bad).is_none(), "{bad:?}");
        }
        assert_eq!(decode_utf8(b"\xef\xbb\xbfa").unwrap(), vec![0x61]);
        assert!(decode_utf8(b"\xef\xbb\xbf").unwrap().is_empty());
        assert_eq!(decode_utf8(b"a\xef\xbb\xbfb").unwrap(), vec![0x61, 0xfeff, 0x62]);
    }

    #[test]
    fn single_byte_tables() {
        assert_eq!(byte_to_unit(0x8E, MAC_ROMAN), 0xE9);
        assert_eq!(byte_to_unit(0xA5, MAC_ROMAN), 0x2022);
        assert_eq!(byte_to_unit(0xC3, MAC_ROMAN), 0x221A);
        assert_eq!(byte_to_unit(0x80, WINDOWS_LATIN1), 0x20AC);
        assert_eq!(byte_to_unit(0x93, WINDOWS_LATIN1), 0x201C);
        assert_eq!(byte_to_unit(0xE9, NEXTSTEP_LATIN), 0xD8);
        assert_eq!(byte_to_unit(0xE9, ISO_LATIN1), 0xE9);
        assert_eq!(byte_to_unit(0xFF, ASCII), 0xFF);
        assert_eq!(unit_to_byte(0xE9, MAC_ROMAN), Some(0x8E));
        assert_eq!(unit_to_byte(0x20AC, WINDOWS_LATIN1), Some(0x80));
        assert_eq!(unit_to_byte(0x20AC, ISO_LATIN1), None);
        assert_eq!(unit_to_byte(0xE9, ASCII), None);
        // Every table entry comes back.
        for b in 0x80..=0xFDu8 {
            for enc in [MAC_ROMAN, NEXTSTEP_LATIN, WINDOWS_LATIN1, ISO_LATIN1] {
                let u = byte_to_unit(b, enc);
                assert_eq!(unit_to_byte(u, enc), Some(b), "{enc:#x} {b:#x}");
            }
        }
    }

    #[test]
    fn utf16_and_utf32_forms() {
        assert_eq!(decode(&[0xff, 0xfe, 0x61, 0], UTF16, true).unwrap(), vec![0x61]);
        assert_eq!(decode(&[0xfe, 0xff, 0, 0x61], UTF16, true).unwrap(), vec![0x61]);
        assert_eq!(decode(&[0xff, 0xfe, 0x61, 0], UTF16, false).unwrap(), vec![0xfeff, 0x61]);
        assert_eq!(decode(&[0xff, 0xfe, 0x61, 0], UTF16LE, true).unwrap(), vec![0xfeff, 0x61]);
        assert_eq!(decode(&[0, 0x61, 0, 0x62], UTF16, true).unwrap(), vec![0x61, 0x62]);
        assert_eq!(decode(&[0x61, 0, 0x62], UTF16LE, false).unwrap(), vec![0x61]);
        assert!(decode(&[0x61], UTF16LE, false).is_none());
        assert_eq!(decode(&[0x41, 0, 0, 0, 0, 0xf6, 1, 0], UTF32LE, false).unwrap(), vec![0x41, 0xd83d, 0xde00]);
        assert_eq!(decode(&[0, 0, 0, 0x41, 0, 1, 0xf6, 0], UTF32BE, false).unwrap(), vec![0x41, 0xd83d, 0xde00]);
        assert!(decode(b"x", 0x1234, false).is_none());
    }

    #[test]
    fn encoding_is_all_or_nothing() {
        let mut out = Vec::new();
        assert!(encode(&[0x68, 0xe9], UTF16, true, &mut out));
        assert_eq!(out, [0xff, 0xfe, 0x68, 0, 0xe9, 0]);
        out.clear();
        assert!(!encode(&[0xd83d], UTF8, false, &mut out));
        out.clear();
        assert!(encode(&[0x41, 0, 0x42], UTF8, false, &mut out));
        assert_eq!(out, [0x41, 0, 0x42]);
        out.clear();
        assert!(encode(&[0x61], UTF32, true, &mut out));
        assert_eq!(out, [0xff, 0xfe, 0, 0, 0x61, 0, 0, 0]);
        assert_eq!(max_size(5, UTF8), 15);
        assert_eq!(max_size(5, ASCII), 5);
        assert_eq!(max_size(5, UTF16), 10);
    }
}
