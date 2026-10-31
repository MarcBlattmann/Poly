//! ASN.1 Distinguished Encoding Rules (ITU-T X.690 section 10, with the
//! basic rules of section 8 it restricts): a reader of tag-length-value
//! elements and a small writer.
//!
//! Only what X.509 and its neighbours use: single-byte identifiers (tag
//! numbers 0..=30, X.690 8.1.2.2), definite lengths (8.1.3; the
//! indefinite form 8.1.3.6 is refused, as DER requires, 10.1), lengths up
//! to four length octets. The reader does not insist on minimal length
//! encodings (some deployed certificates have them), but refuses
//! everything ambiguous.

use alloc::vec::Vec;

/// Universal class tags (X.690 8.1.2, X.680 8.4).
pub mod tag {
    pub const BOOLEAN: u8 = 0x01;
    pub const INTEGER: u8 = 0x02;
    pub const BIT_STRING: u8 = 0x03;
    pub const OCTET_STRING: u8 = 0x04;
    pub const NULL: u8 = 0x05;
    pub const OID: u8 = 0x06;
    pub const ENUMERATED: u8 = 0x0a;
    pub const UTF8_STRING: u8 = 0x0c;
    pub const NUMERIC_STRING: u8 = 0x12;
    pub const PRINTABLE_STRING: u8 = 0x13;
    pub const T61_STRING: u8 = 0x14;
    pub const VIDEOTEX_STRING: u8 = 0x15;
    pub const IA5_STRING: u8 = 0x16;
    pub const UTC_TIME: u8 = 0x17;
    pub const GENERALIZED_TIME: u8 = 0x18;
    pub const GRAPHIC_STRING: u8 = 0x19;
    pub const VISIBLE_STRING: u8 = 0x1a;
    pub const GENERAL_STRING: u8 = 0x1b;
    pub const UNIVERSAL_STRING: u8 = 0x1c;
    pub const BMP_STRING: u8 = 0x1e;
    pub const SEQUENCE: u8 = 0x30;
    pub const SET: u8 = 0x31;
    /// The constructed bit (X.690 8.1.2.5).
    pub const CONSTRUCTED: u8 = 0x20;
    /// Context-specific class (X.690 8.1.2.2, bits 8-7 = 10).
    pub const CONTEXT: u8 = 0x80;

    /// `[n]` constructed (an explicit tag or an implicitly tagged SEQUENCE).
    pub const fn context_cons(n: u8) -> u8 {
        CONTEXT | CONSTRUCTED | n
    }

    /// `[n]` primitive (an implicitly tagged primitive type).
    pub const fn context_prim(n: u8) -> u8 {
        CONTEXT | n
    }
}

/// Why an encoding was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The input ended inside an element.
    Truncated,
    /// A tag other than the one expected.
    UnexpectedTag { expected: u8, found: u8 },
    /// Something the rules do not allow (indefinite length, high tag
    /// numbers, a bad BOOLEAN, an empty INTEGER ...).
    Invalid,
    /// Bytes left after the element that should have ended the input.
    TrailingData,
}

/// One element: its identifier octet, its contents and its whole encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tlv<'a> {
    pub tag: u8,
    pub value: &'a [u8],
    /// Identifier, length and contents.
    pub raw: &'a [u8],
}

impl<'a> Tlv<'a> {
    /// A reader over the contents (for constructed elements).
    pub fn reader(&self) -> Reader<'a> {
        Reader::new(self.value)
    }

    /// The contents if the tag is `t`.
    pub fn expect(&self, t: u8) -> Result<&'a [u8], Error> {
        if self.tag == t { Ok(self.value) } else { Err(Error::UnexpectedTag { expected: t, found: self.tag }) }
    }
}

/// A cursor over consecutive DER elements.
#[derive(Clone, Copy, Debug)]
pub struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data }
    }

    /// Whether every element has been read.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// What is left.
    pub fn rest(&self) -> &'a [u8] {
        self.data
    }

    /// The tag of the next element, if any.
    pub fn peek_tag(&self) -> Option<u8> {
        self.data.first().copied()
    }

    /// The next element.
    pub fn read(&mut self) -> Result<Tlv<'a>, Error> {
        let d = self.data;
        let &t = d.first().ok_or(Error::Truncated)?;
        // High tag numbers (X.690 8.1.2.4) do not occur in X.509.
        if t & 0x1f == 0x1f {
            return Err(Error::Invalid);
        }
        let &l0 = d.get(1).ok_or(Error::Truncated)?;
        let (len, hdr) = if l0 < 0x80 {
            (usize::from(l0), 2)
        } else {
            let n = usize::from(l0 & 0x7f);
            // 0x80 is the indefinite form (not DER); more than 4 octets is
            // more than anything here can hold.
            if n == 0 || n > 4 {
                return Err(Error::Invalid);
            }
            let bytes = d.get(2..2 + n).ok_or(Error::Truncated)?;
            let len = bytes.iter().fold(0usize, |a, &b| (a << 8) | usize::from(b));
            (len, 2 + n)
        };
        let end = hdr.checked_add(len).ok_or(Error::Invalid)?;
        if d.len() < end {
            return Err(Error::Truncated);
        }
        self.data = &d[end..];
        Ok(Tlv { tag: t, value: &d[hdr..end], raw: &d[..end] })
    }

    /// The next element, which must have tag `t`.
    pub fn read_tag(&mut self, t: u8) -> Result<Tlv<'a>, Error> {
        let e = self.read()?;
        if e.tag != t {
            return Err(Error::UnexpectedTag { expected: t, found: e.tag });
        }
        Ok(e)
    }

    /// The next element if it has tag `t`; nothing is consumed otherwise.
    pub fn read_optional(&mut self, t: u8) -> Result<Option<Tlv<'a>>, Error> {
        if self.peek_tag() == Some(t) { self.read().map(Some) } else { Ok(None) }
    }

    /// A SEQUENCE's contents as a reader.
    pub fn sequence(&mut self) -> Result<Reader<'a>, Error> {
        Ok(self.read_tag(tag::SEQUENCE)?.reader())
    }

    /// An INTEGER's contents (two's complement, big-endian, X.690 8.3).
    pub fn integer_bytes(&mut self) -> Result<&'a [u8], Error> {
        let v = self.read_tag(tag::INTEGER)?.value;
        if v.is_empty() {
            return Err(Error::Invalid);
        }
        Ok(v)
    }

    /// A non-negative INTEGER's magnitude without the sign octet
    /// (big-endian, possibly empty for zero).
    pub fn unsigned_integer(&mut self) -> Result<&'a [u8], Error> {
        let v = self.integer_bytes()?;
        if v[0] & 0x80 != 0 {
            return Err(Error::Invalid);
        }
        let mut s = v;
        while let [0, rest @ ..] = s {
            s = rest;
        }
        Ok(s)
    }

    /// A small non-negative INTEGER.
    pub fn small_uint(&mut self) -> Result<u64, Error> {
        let v = self.unsigned_integer()?;
        if v.len() > 8 {
            return Err(Error::Invalid);
        }
        Ok(v.iter().fold(0u64, |a, &b| (a << 8) | u64::from(b)))
    }

    /// A BOOLEAN (DER: 0x00 or 0xff, X.690 11.1).
    pub fn boolean(&mut self) -> Result<bool, Error> {
        match self.read_tag(tag::BOOLEAN)?.value {
            [0] => Ok(false),
            // BER allows any non-zero value; accept them.
            [_] => Ok(true),
            _ => Err(Error::Invalid),
        }
    }

    /// An OBJECT IDENTIFIER's contents.
    pub fn oid(&mut self) -> Result<&'a [u8], Error> {
        let v = self.read_tag(tag::OID)?.value;
        if v.is_empty() || v[v.len() - 1] & 0x80 != 0 {
            return Err(Error::Invalid);
        }
        Ok(v)
    }

    /// A BIT STRING: the number of unused bits and the octets (X.690 8.6).
    pub fn bit_string(&mut self) -> Result<(u8, &'a [u8]), Error> {
        let v = self.read_tag(tag::BIT_STRING)?.value;
        bit_string_contents(v)
    }

    /// An OCTET STRING's contents.
    pub fn octet_string(&mut self) -> Result<&'a [u8], Error> {
        Ok(self.read_tag(tag::OCTET_STRING)?.value)
    }

    /// Fails unless everything was read.
    pub fn finish(&self) -> Result<(), Error> {
        if self.data.is_empty() { Ok(()) } else { Err(Error::TrailingData) }
    }
}

/// A BIT STRING's contents split into unused bits and octets.
pub fn bit_string_contents(v: &[u8]) -> Result<(u8, &[u8]), Error> {
    let (&unused, bits) = v.split_first().ok_or(Error::Invalid)?;
    if unused > 7 || (bits.is_empty() && unused != 0) {
        return Err(Error::Invalid);
    }
    Ok((unused, bits))
}

/// The single element that makes up `data`.
pub fn parse_one(data: &[u8]) -> Result<Tlv<'_>, Error> {
    let mut r = Reader::new(data);
    let e = r.read()?;
    r.finish()?;
    Ok(e)
}

// ----- Writing ------------------------------------------------------------------------

/// Append an element with tag `t` and contents `value`.
pub fn write(out: &mut Vec<u8>, t: u8, value: &[u8]) {
    out.push(t);
    write_len(out, value.len());
    out.extend_from_slice(value);
}

/// An element as a new vector.
pub fn encode(t: u8, value: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(value.len() + 6);
    write(&mut v, t, value);
    v
}

/// The length octets (X.690 10.1: the shortest form).
pub fn write_len(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = (len as u64).to_be_bytes();
        let skip = bytes.iter().take_while(|&&b| b == 0).count();
        out.push(0x80 | (8 - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
}

/// A non-negative INTEGER from big-endian magnitude bytes (a leading zero
/// octet added when the top bit is set; leading zeros removed).
pub fn encode_unsigned(magnitude: &[u8]) -> Vec<u8> {
    let mut m = magnitude;
    while let [0, rest @ ..] = m {
        m = rest;
    }
    let mut v = Vec::with_capacity(m.len() + 1);
    if m.is_empty() || m[0] & 0x80 != 0 {
        v.push(0);
    }
    v.extend_from_slice(m);
    encode(tag::INTEGER, &v)
}

/// A SEQUENCE of already encoded elements.
pub fn encode_sequence(parts: &[&[u8]]) -> Vec<u8> {
    let body: Vec<u8> = parts.concat();
    encode(tag::SEQUENCE, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_short_and_long_lengths() {
        let mut r = Reader::new(&[0x04, 0x02, 0xaa, 0xbb, 0x30, 0x81, 0x01, 0x05]);
        let e = r.read().unwrap();
        assert_eq!((e.tag, e.value), (0x04, &[0xaa, 0xbb][..]));
        let e = r.read().unwrap();
        assert_eq!((e.tag, e.value, e.raw.len()), (0x30, &[0x05][..], 4));
        assert!(r.is_empty());
    }

    #[test]
    fn refuses_indefinite_truncated_and_high_tags() {
        assert_eq!(Reader::new(&[0x30, 0x80, 0, 0]).read(), Err(Error::Invalid));
        assert_eq!(Reader::new(&[0x04, 0x05, 1]).read(), Err(Error::Truncated));
        assert_eq!(Reader::new(&[0x1f, 0x22, 0]).read(), Err(Error::Invalid));
        assert_eq!(Reader::new(&[]).read(), Err(Error::Truncated));
        assert_eq!(parse_one(&[0x05, 0x00, 0x00]), Err(Error::TrailingData));
    }

    #[test]
    fn integers_booleans_and_bit_strings() {
        let mut r = Reader::new(&[0x02, 0x02, 0x00, 0x80, 0x02, 0x01, 0xff, 0x01, 0x01, 0xff, 0x03, 0x02, 0x07, 0x80]);
        assert_eq!(r.unsigned_integer().unwrap(), &[0x80]);
        assert_eq!(r.unsigned_integer(), Err(Error::Invalid));
        assert!(r.boolean().unwrap());
        assert_eq!(r.bit_string().unwrap(), (7, &[0x80][..]));
        assert_eq!(bit_string_contents(&[8, 0]), Err(Error::Invalid));
    }

    #[test]
    fn writes_minimal_lengths_and_unsigned_integers() {
        assert_eq!(encode(0x04, &[1; 3]), [4, 3, 1, 1, 1]);
        let long = encode(0x04, &[0; 300]);
        assert_eq!(&long[..4], &[4, 0x82, 1, 44]);
        assert_eq!(encode_unsigned(&[0x80]), [2, 2, 0, 0x80]);
        assert_eq!(encode_unsigned(&[0, 0, 5]), [2, 1, 5]);
        assert_eq!(encode_unsigned(&[]), [2, 1, 0]);
        let mut r = Reader::new(&long);
        assert_eq!(r.read().unwrap().value.len(), 300);
    }
}
