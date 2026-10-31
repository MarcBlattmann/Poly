//! SHA-1 (FIPS 180-4 6.1). Broken for collision resistance; certificates,
//! thumbprints and old protocols still use it.

use crate::block::BlockBuf;

/// A SHA-1 computation in progress.
#[derive(Clone, Debug)]
pub struct Sha1 {
    state: [u32; 5],
    buf: BlockBuf<64>,
}

impl Default for Sha1 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha1 {
    /// The initial hash value (FIPS 180-4 5.3.1).
    pub fn new() -> Self {
        Sha1 { state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0], buf: BlockBuf::new() }
    }

    /// Absorb `data`.
    pub fn update(&mut self, data: &[u8]) {
        self.buf.update(data, |b| compress(&mut self.state, b));
    }

    /// Pad (FIPS 180-4 5.1.1) and return the digest.
    pub fn finish(mut self) -> [u8; 20] {
        let bits = (self.buf.bit_len() as u64).to_be_bytes();
        self.buf.pad(&bits, |b| compress(&mut self.state, b));
        let mut out = [0u8; 20];
        for (o, w) in out.chunks_exact_mut(4).zip(self.state) {
            o.copy_from_slice(&w.to_be_bytes());
        }
        out
    }
}

/// The SHA-1 compression of one block (FIPS 180-4 6.1.2, functions and
/// constants of 4.1.1 and 4.2.1).
fn compress(state: &mut [u32; 5], block: &[u8; 64]) {
    let mut w = [0u32; 80];
    for (t, c) in block.chunks_exact(4).enumerate() {
        w[t] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
    }
    for t in 16..80 {
        w[t] = (w[t - 3] ^ w[t - 8] ^ w[t - 14] ^ w[t - 16]).rotate_left(1);
    }
    let [mut a, mut b, mut c, mut d, mut e] = *state;
    for (t, &wt) in w.iter().enumerate() {
        let (f, k) = match t / 20 {
            0 => ((b & c) ^ (!b & d), 0x5a827999),
            1 => (b ^ c ^ d, 0x6ed9eba1),
            2 => ((b & c) ^ (b & d) ^ (c & d), 0x8f1bbcdc),
            _ => (b ^ c ^ d, 0xca62c1d6),
        };
        let tmp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wt);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = tmp;
    }
    for (s, v) in state.iter_mut().zip([a, b, c, d, e]) {
        *s = s.wrapping_add(v);
    }
}
