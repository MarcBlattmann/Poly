//! The Internet checksum (RFC 1071): the ones' complement of the ones'
//! complement sum of 16-bit big-endian words, an odd last byte padded with
//! zero. TCP, UDP and ICMPv6 add a pseudo-header (RFC 9293 3.1, RFC 768,
//! RFC 8200 8.1).

use core::net::{Ipv4Addr, Ipv6Addr};

/// A running sum. Adding never overflows for anything shorter than 2^48 bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sum(u64);

impl Sum {
    /// Add `data` as big-endian 16-bit words. `data` must start on an even
    /// offset of the checksummed region, which holds for every use here
    /// (headers have even lengths), except for the final odd byte.
    pub fn add_bytes(mut self, data: &[u8]) -> Sum {
        let mut chunks = data.chunks_exact(2);
        for c in &mut chunks {
            self.0 += u64::from(u16::from_be_bytes([c[0], c[1]]));
        }
        if let [last] = chunks.remainder() {
            self.0 += u64::from(*last) << 8;
        }
        self
    }

    /// Add one 16-bit word.
    pub fn add16(mut self, v: u16) -> Sum {
        self.0 += u64::from(v);
        self
    }

    /// Add one 32-bit value as two words.
    pub fn add32(self, v: u32) -> Sum {
        self.add16((v >> 16) as u16).add16(v as u16)
    }

    /// The folded ones' complement sum.
    pub fn fold(self) -> u16 {
        let mut s = self.0;
        while s > 0xffff {
            s = (s & 0xffff) + (s >> 16);
        }
        s as u16
    }

    /// The checksum to put in a header: the complement of the folded sum.
    pub fn finish(self) -> u16 {
        !self.fold()
    }
}

/// Checksum of `data` alone (IPv4 header, ICMPv4).
pub fn checksum(data: &[u8]) -> u16 {
    Sum::default().add_bytes(data).finish()
}

/// True if `data`, checksum field included, sums to all ones.
pub fn verify(data: &[u8]) -> bool {
    Sum::default().add_bytes(data).fold() == 0xffff
}

/// The IPv4 pseudo-header for TCP and UDP (RFC 9293 3.1, RFC 768).
pub fn pseudo_v4(src: Ipv4Addr, dst: Ipv4Addr, protocol: u8, len: usize) -> Sum {
    Sum::default().add_bytes(&src.octets()).add_bytes(&dst.octets()).add16(u16::from(protocol)).add16(len as u16)
}

/// The IPv6 pseudo-header (RFC 8200 8.1); `len` is the upper-layer length.
pub fn pseudo_v6(src: Ipv6Addr, dst: Ipv6Addr, next_header: u8, len: usize) -> Sum {
    Sum::default().add_bytes(&src.octets()).add_bytes(&dst.octets()).add32(len as u32).add16(u16::from(next_header))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc1071_example() {
        // RFC 1071 section 3: the words 0x0001 0xf203 0xf4f5 0xf6f7 sum to 0xddf2.
        let data = [0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7];
        assert_eq!(Sum::default().add_bytes(&data).fold(), 0xddf2);
        assert_eq!(checksum(&data), !0xddf2);
    }

    #[test]
    fn odd_length_pads_with_zero() {
        assert_eq!(Sum::default().add_bytes(&[0x12, 0x34, 0x56]).fold(), 0x1234 + 0x5600);
    }

    #[test]
    fn verify_accepts_own_checksum() {
        let mut data = [0x45, 0x00, 0x00, 0x1c, 0x12, 0x34, 0x00, 0x00, 0x40, 0x11, 0, 0, 10, 0, 0, 1, 10, 0, 0, 2];
        let c = checksum(&data);
        data[10..12].copy_from_slice(&c.to_be_bytes());
        assert!(verify(&data));
        data[3] ^= 1;
        assert!(!verify(&data));
    }

    #[test]
    fn large_inputs_do_not_overflow() {
        let data = [0xffu8; 200_000];
        assert_eq!(Sum::default().add_bytes(&data).fold(), 0xffff);
    }
}
