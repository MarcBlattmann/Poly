//! Hashes and description text of CoreFoundation's value types, as the
//! spec sheets `CFHash` and `CFCopyDescription` describe them.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

/// `CFHash` of a string's units: the same for equal strings, 0 for the
/// empty one (FNV-1a over the units).
pub fn hash_units(units: &[u16]) -> u64 {
    if units.is_empty() {
        return 0;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &u in units {
        h ^= u64::from(u);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// `CFHash` of data bytes: 0 for empty data.
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The `bytes = 0x...` part of a data description: every byte up to 24,
/// else the first 16 and the last 8 with ` ... ` between.
pub fn data_bytes(bytes: &[u8]) -> String {
    let mut s = String::from("0x");
    let hex = |s: &mut String, b: &[u8]| {
        for x in b {
            let _ = write!(s, "{x:02x}");
        }
    };
    if bytes.len() <= 24 {
        hex(&mut s, bytes);
    } else {
        hex(&mut s, &bytes[..16]);
        s.push_str(" ... ");
        hex(&mut s, &bytes[bytes.len() - 8..]);
    }
    s
}

/// Indent every line after the first by four spaces (a nested
/// collection's description inside another).
pub fn indent_inner(text: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(text.len() + 16);
    for &u in text {
        out.push(u);
        if u == u16::from(b'\n') {
            out.extend(core::iter::repeat_n(u16::from(b' '), 4));
        }
    }
    out
}

/// The shortest text of a double that reads back the same (`2.5`, `1`,
/// `1e+30`), for numbers inside collections and `%@`.
pub fn short_double(d: f64) -> String {
    if d.is_nan() {
        return String::from("nan");
    }
    if d.is_infinite() {
        return String::from(if d > 0.0 { "inf" } else { "-inf" });
    }
    let mut s = String::new();
    let _ = write!(s, "{d}");
    if s.len() > 17 {
        s.clear();
        let _ = write!(s, "{d:e}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes() {
        assert_eq!(hash_units(&[]), 0);
        assert_eq!(hash_bytes(&[]), 0);
        let a: Vec<u16> = "alpha".encode_utf16().collect();
        let b: Vec<u16> = "alphb".encode_utf16().collect();
        assert_ne!(hash_units(&a), hash_units(&b));
        let abc: Vec<u16> = "abc".encode_utf16().collect();
        let cba: Vec<u16> = "cba".encode_utf16().collect();
        assert_ne!(hash_units(&abc), hash_units(&cba));
    }

    #[test]
    fn data_text() {
        assert_eq!(data_bytes(&[0, 1, 2]), "0x000102");
        let long: Vec<u8> = (0..25).collect();
        assert_eq!(data_bytes(&long), "0x000102030405060708090a0b0c0d0e0f ... 1112131415161718");
        let x: Vec<u16> = "(\n    a\n)".encode_utf16().collect();
        assert_eq!(String::from_utf16(&indent_inner(&x)).unwrap(), "(\n        a\n    )");
        assert_eq!(short_double(2.5), "2.5");
        assert_eq!(short_double(1.0), "1");
    }
}
