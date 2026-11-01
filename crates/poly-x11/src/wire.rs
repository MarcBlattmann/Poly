//! The protocol's integer types on the wire.
//!
//! A client says which byte order it uses in the first byte of its
//! connection setup; this crate always says `'l'` (least significant byte
//! first), so every `CARD16`/`CARD32`/`INT16`/`INT32` here is
//! little-endian. Variable-length data (strings, lists) is padded to a
//! multiple of 4 bytes, and request lengths count 4-byte units.

use alloc::vec::Vec;

/// Bytes of padding after `n` bytes, to a multiple of 4.
pub const fn pad(n: usize) -> usize {
    (4 - n % 4) % 4
}

/// The `CARD16` at `off`, 0 if `b` is too short.
pub fn u16_at(b: &[u8], off: usize) -> u16 {
    b.get(off..off + 2).map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}

/// The `INT16` at `off`, 0 if `b` is too short.
pub fn i16_at(b: &[u8], off: usize) -> i16 {
    u16_at(b, off) as i16
}

/// The `CARD32` at `off`, 0 if `b` is too short.
pub fn u32_at(b: &[u8], off: usize) -> u32 {
    b.get(off..off + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// The byte at `off`, 0 if `b` is too short.
pub fn u8_at(b: &[u8], off: usize) -> u8 {
    b.get(off).copied().unwrap_or(0)
}

/// Appends protocol fields to a buffer.
#[derive(Debug)]
pub struct Writer<'a>(pub &'a mut Vec<u8>);

impl Writer<'_> {
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn i16(&mut self, v: i16) -> &mut Self {
        self.u16(v as u16)
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }

    /// `n` zero bytes.
    pub fn zeros(&mut self, n: usize) -> &mut Self {
        self.0.resize(self.0.len() + n, 0);
        self
    }

    /// `b`, then zeros to a multiple of 4.
    pub fn padded(&mut self, b: &[u8]) -> &mut Self {
        self.bytes(b).zeros(pad(b.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_to_four_and_reads_little_endian() {
        assert_eq!([pad(0), pad(1), pad(2), pad(3), pad(4), pad(5)], [0, 3, 2, 1, 0, 3]);
        let b = [0x34, 0x12, 0x78, 0x56, 0xff, 0xff];
        assert_eq!(u16_at(&b, 0), 0x1234);
        assert_eq!(u32_at(&b, 0), 0x5678_1234);
        assert_eq!(i16_at(&b, 4), -1);
        assert_eq!((u32_at(&b, 4), u16_at(&b, 5), u8_at(&b, 9)), (0, 0, 0), "short input reads as 0");
        let mut v = Vec::new();
        Writer(&mut v).u8(1).u16(0x0203).i16(-2).u32(0x0807_0605).padded(b"abcde");
        assert_eq!(v, [1, 3, 2, 0xfe, 0xff, 5, 6, 7, 8, b'a', b'b', b'c', b'd', b'e', 0, 0, 0]);
    }
}
