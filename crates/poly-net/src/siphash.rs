//! SipHash-2-4, written from the algorithm description in Aumasson and
//! Bernstein, "SipHash: a fast short-input PRF" (2012). Used as the keyed
//! function F of RFC 6528 for initial sequence numbers and for choosing
//! ephemeral ports (RFC 6056).

#[derive(Clone, Copy, Debug)]
pub(crate) struct SipHasher {
    v: [u64; 4],
    tail: u64,
    ntail: usize,
    len: usize,
}

impl SipHasher {
    pub fn new(k0: u64, k1: u64) -> SipHasher {
        SipHasher {
            v: [
                k0 ^ 0x736f_6d65_7073_6575,
                k1 ^ 0x646f_7261_6e64_6f6d,
                k0 ^ 0x6c79_6765_6e65_7261,
                k1 ^ 0x7465_6462_7974_6573,
            ],
            tail: 0,
            ntail: 0,
            len: 0,
        }
    }

    fn round(v: &mut [u64; 4]) {
        v[0] = v[0].wrapping_add(v[1]);
        v[1] = v[1].rotate_left(13) ^ v[0];
        v[0] = v[0].rotate_left(32);
        v[2] = v[2].wrapping_add(v[3]);
        v[3] = v[3].rotate_left(16) ^ v[2];
        v[0] = v[0].wrapping_add(v[3]);
        v[3] = v[3].rotate_left(21) ^ v[0];
        v[2] = v[2].wrapping_add(v[1]);
        v[1] = v[1].rotate_left(17) ^ v[2];
        v[2] = v[2].rotate_left(32);
    }

    fn compress(&mut self, m: u64) {
        self.v[3] ^= m;
        Self::round(&mut self.v);
        Self::round(&mut self.v);
        self.v[0] ^= m;
    }

    pub fn write(&mut self, bytes: &[u8]) -> &mut Self {
        for &b in bytes {
            self.tail |= u64::from(b) << (8 * self.ntail);
            self.ntail += 1;
            self.len += 1;
            if self.ntail == 8 {
                let m = self.tail;
                self.compress(m);
                self.tail = 0;
                self.ntail = 0;
            }
        }
        self
    }

    pub fn finish(&self) -> u64 {
        let mut s = *self;
        let b = ((s.len as u64 & 0xff) << 56) | s.tail;
        s.compress(b);
        s.v[2] ^= 0xff;
        for _ in 0..4 {
            Self::round(&mut s.v);
        }
        s.v[0] ^ s.v[1] ^ s.v[2] ^ s.v[3]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_test_vector() {
        // Appendix A of the paper: key 00..0f, message 00..0e.
        let k0 = u64::from_le_bytes([0, 1, 2, 3, 4, 5, 6, 7]);
        let k1 = u64::from_le_bytes([8, 9, 10, 11, 12, 13, 14, 15]);
        let msg: [u8; 15] = core::array::from_fn(|i| i as u8);
        assert_eq!(SipHasher::new(k0, k1).write(&msg).finish(), 0xa129_ca61_49be_45e5);
        // Split writes give the same result.
        let mut h = SipHasher::new(k0, k1);
        h.write(&msg[..3]).write(&msg[3..]);
        assert_eq!(h.finish(), 0xa129_ca61_49be_45e5);
    }
}
