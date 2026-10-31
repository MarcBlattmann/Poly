//! Text forms of binary data: Base64 (RFC 4648 section 4), hexadecimal and
//! PEM's textual encoding (RFC 7468).

use alloc::string::String;
use alloc::vec::Vec;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 of `data`, padded, without line breaks.
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(B64[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn b64_value(c: u8) -> Option<u32> {
    Some(match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    } as u32)
}

/// Decode Base64, skipping white space (RFC 4648 3.3 lets applications
/// ignore line breaks). Padding is optional at the end; anything else
/// that is not in the alphabet fails.
pub fn base64_decode(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0u32;
    let mut n = 0;
    let mut pad = 0;
    for &c in text {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == b'=' {
            pad += 1;
            continue;
        }
        if pad > 0 {
            return None;
        }
        acc = (acc << 6) | b64_value(c)?;
        n += 1;
        if n == 4 {
            out.extend_from_slice(&[(acc >> 16) as u8, (acc >> 8) as u8, acc as u8]);
            acc = 0;
            n = 0;
        }
    }
    match n {
        0 => {}
        2 => out.push((acc >> 4) as u8),
        3 => out.extend_from_slice(&[(acc >> 10) as u8, (acc >> 2) as u8]),
        _ => return None,
    }
    if pad > 2 || (pad > 0 && (n + pad) % 4 != 0) {
        return None;
    }
    Some(out)
}

/// Lowercase hex digits of `data`.
pub fn hex_encode(data: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(data.len() * 2);
    for &b in data {
        s.push(char::from(D[usize::from(b >> 4)]));
        s.push(char::from(D[usize::from(b & 15)]));
    }
    s
}

/// Bytes from hex digit pairs, skipping white space; None for anything
/// else or an odd number of digits.
pub fn hex_decode(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut hi: Option<u8> = None;
    for &c in text {
        if c.is_ascii_whitespace() {
            continue;
        }
        let v = (c as char).to_digit(16)? as u8;
        match hi.take() {
            Some(h) => out.push((h << 4) | v),
            None => hi = Some(v),
        }
    }
    if hi.is_some() { None } else { Some(out) }
}

/// One PEM block found in text: its label and where its Base64 body is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PemBlock<'a> {
    pub label: &'a str,
    pub body: &'a [u8],
    /// The index just past the `-----END ...-----` line.
    pub end: usize,
}

/// The first `-----BEGIN label-----` ... `-----END label-----` block in
/// `text` (RFC 7468 2), with any label.
pub fn pem_find(text: &[u8]) -> Option<PemBlock<'_>> {
    const BEGIN: &[u8] = b"-----BEGIN ";
    const DASHES: &[u8] = b"-----";
    let start = find(text, BEGIN)?;
    let label_start = start + BEGIN.len();
    let label_len = find(&text[label_start..], DASHES)?;
    let label = core::str::from_utf8(&text[label_start..label_start + label_len]).ok()?;
    let body_start = label_start + label_len + DASHES.len();
    let mut end_marker = Vec::from(&b"-----END "[..]);
    end_marker.extend_from_slice(label.as_bytes());
    end_marker.extend_from_slice(DASHES);
    let body_len = find(&text[body_start..], &end_marker)?;
    Some(PemBlock {
        label,
        body: &text[body_start..body_start + body_len],
        end: body_start + body_len + end_marker.len(),
    })
}

/// The bytes of the first PEM block (with the given label, if any).
pub fn pem_decode(text: &[u8], label: Option<&str>) -> Option<Vec<u8>> {
    let mut rest = text;
    loop {
        let b = pem_find(rest)?;
        if label.is_none_or(|l| l == b.label) {
            return base64_decode(b.body);
        }
        rest = &rest[b.end..];
    }
}

/// PEM text for `data`: header, Base64 in 64-character lines, footer, each
/// line ended by `eol`.
pub fn pem_encode(label: &str, data: &[u8], eol: &str) -> String {
    let b64 = base64_encode(data);
    let mut s = String::new();
    s.push_str("-----BEGIN ");
    s.push_str(label);
    s.push_str("-----");
    s.push_str(eol);
    for line in b64.as_bytes().chunks(64) {
        s.push_str(core::str::from_utf8(line).unwrap_or_default());
        s.push_str(eol);
    }
    s.push_str("-----END ");
    s.push_str(label);
    s.push_str("-----");
    s.push_str(eol);
    s
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors() {
        // RFC 4648 section 10.
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (plain, enc) in cases {
            assert_eq!(base64_encode(plain.as_bytes()), enc);
            assert_eq!(base64_decode(enc.as_bytes()).unwrap(), plain.as_bytes());
        }
        assert_eq!(base64_decode(b"Zm9v\r\nYmFy").unwrap(), b"foobar");
        assert_eq!(base64_decode(b"Zg").unwrap(), b"f");
        assert_eq!(base64_decode(b"Z"), None);
        assert_eq!(base64_decode(b"Zg==Zg=="), None);
        assert_eq!(base64_decode(b"Zm9v!"), None);
    }

    #[test]
    fn hex() {
        assert_eq!(hex_encode(&[0, 0xab, 0x10]), "00ab10");
        assert_eq!(hex_decode(b"00 AB\n10").unwrap(), [0, 0xab, 0x10]);
        assert_eq!(hex_decode(b"0"), None);
        assert_eq!(hex_decode(b"0g"), None);
    }

    #[test]
    fn pem() {
        let text = pem_encode("CERTIFICATE", &[1u8; 100], "\n");
        assert!(text.starts_with("-----BEGIN CERTIFICATE-----\n"));
        assert_eq!(text.lines().nth(1).unwrap().len(), 64);
        let both = alloc::format!("junk\n{}{}", pem_encode("X", b"x", "\r\n"), text);
        assert_eq!(pem_decode(both.as_bytes(), Some("CERTIFICATE")).unwrap(), [1u8; 100]);
        assert_eq!(pem_decode(both.as_bytes(), None).unwrap(), b"x");
        assert_eq!(pem_find(b"-----BEGIN X-----abc").map(|b| b.label), None);
    }
}
