//! MD5 (RFC 1321). Broken for collision resistance; kept because CNG and
//! old file formats still name it.

use crate::block::BlockBuf;

/// The per-round shift amounts (RFC 1321 3.4).
const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15,
    21,
];

/// `T[i] = floor(2^32 * abs(sin(i + 1)))` (RFC 1321 3.4), as listed there.
const T: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501, 0x698098d8,
    0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340,
    0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87,
    0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
    0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039,
    0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92,
    0xffeff47d, 0x85845dd1, 0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
    0xeb86d391,
];

/// An MD5 computation in progress.
#[derive(Clone, Debug)]
pub struct Md5 {
    state: [u32; 4],
    buf: BlockBuf<64>,
}

impl Default for Md5 {
    fn default() -> Self {
        Self::new()
    }
}

impl Md5 {
    /// The initial state (RFC 1321 3.3).
    pub fn new() -> Self {
        Md5 { state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476], buf: BlockBuf::new() }
    }

    /// Absorb `data`.
    pub fn update(&mut self, data: &[u8]) {
        self.buf.update(data, |b| compress(&mut self.state, b));
    }

    /// Pad (RFC 1321 3.1, 3.2: the bit length little-endian) and return the
    /// digest.
    pub fn finish(mut self) -> [u8; 16] {
        let bits = (self.buf.bit_len() as u64).to_le_bytes();
        self.buf.pad(&bits, |b| compress(&mut self.state, b));
        let mut out = [0u8; 16];
        for (o, w) in out.chunks_exact_mut(4).zip(self.state) {
            o.copy_from_slice(&w.to_le_bytes());
        }
        out
    }
}

/// One 64-byte block (RFC 1321 3.4).
fn compress(state: &mut [u32; 4], block: &[u8; 64]) {
    let mut x = [0u32; 16];
    for (w, c) in x.iter_mut().zip(block.chunks_exact(4)) {
        *w = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
    }
    let [mut a, mut b, mut c, mut d] = *state;
    for i in 0..64 {
        let (f, g) = match i / 16 {
            0 => ((b & c) | (!b & d), i),
            1 => ((b & d) | (c & !d), (5 * i + 1) % 16),
            2 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let t = a.wrapping_add(f).wrapping_add(T[i]).wrapping_add(x[g]);
        a = d;
        d = c;
        c = b;
        b = b.wrapping_add(t.rotate_left(S[i]));
    }
    for (s, v) in state.iter_mut().zip([a, b, c, d]) {
        *s = s.wrapping_add(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::hex;

    fn md5(s: &[u8]) -> [u8; 16] {
        let mut h = Md5::new();
        h.update(s);
        h.finish()
    }

    /// RFC 1321 A.5 test suite.
    #[test]
    fn rfc_1321_suite() {
        let cases: [(&[u8], &str); 7] = [
            (b"", "d41d8cd98f00b204e9800998ecf8427e"),
            (b"a", "0cc175b9c0f1b6a831c399e269772661"),
            (b"abc", "900150983cd24fb0d6963f7d28e17f72"),
            (b"message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (b"abcdefghijklmnopqrstuvwxyz", "c3fcd3d76192e4007dfb496cca67e13b"),
            (b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789", "d174ab98d277d9f5a5611c2c9f419d9f"),
            (
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ];
        for (m, d) in cases {
            assert_eq!(md5(m).to_vec(), hex(d));
        }
    }

    #[test]
    fn split_updates_match_one_update() {
        let data: alloc::vec::Vec<u8> = (0..300u32).map(|i| i as u8).collect();
        for split in [0, 1, 55, 56, 63, 64, 65, 200] {
            let mut h = Md5::new();
            h.update(&data[..split]);
            h.update(&data[split..]);
            assert_eq!(h.finish(), md5(&data));
        }
    }
}
