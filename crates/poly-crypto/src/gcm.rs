//! Galois/Counter Mode (SP 800-38D) over AES: GCTR encryption (6.5) and
//! the GHASH authentication tag (6.4, 7.1, 7.2).

use crate::aes::Aes;
use alloc::vec::Vec;

/// `R = 11100001 || 0^120` (SP 800-38D 6.3).
const R: u128 = 0xe1 << 120;

/// The product of two blocks in GF(2^128) (SP 800-38D 6.3, Algorithm 1);
/// blocks are big-endian `u128`s, so bit 0 of the standard is the most
/// significant bit.
fn gf_mul(x: u128, y: u128) -> u128 {
    let mut z = 0u128;
    let mut v = y;
    for i in 0..128 {
        if (x >> (127 - i)) & 1 == 1 {
            z ^= v;
        }
        v = if v & 1 == 0 { v >> 1 } else { (v >> 1) ^ R };
    }
    z
}

/// GHASH in progress (SP 800-38D 6.4): `Y_i = (Y_(i-1) ^ X_i) · H`, with a
/// partial block buffered and zero-padded when flushed.
#[derive(Clone)]
struct Ghash {
    h: u128,
    y: u128,
    buf: [u8; 16],
    len: usize,
}

impl Ghash {
    fn new(h: u128) -> Self {
        Ghash { h, y: 0, buf: [0; 16], len: 0 }
    }

    fn block(&mut self, b: &[u8; 16]) {
        self.y = gf_mul(self.y ^ u128::from_be_bytes(*b), self.h);
    }

    fn update(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            let n = (16 - self.len).min(data.len());
            self.buf[self.len..self.len + n].copy_from_slice(&data[..n]);
            self.len += n;
            data = &data[n..];
            if self.len == 16 {
                let b = self.buf;
                self.block(&b);
                self.len = 0;
            }
        }
    }

    /// Zero-pad a partial block (the `0^v` / `0^u` of 7.1 step 5).
    fn flush(&mut self) {
        if self.len > 0 {
            self.buf[self.len..].fill(0);
            let b = self.buf;
            self.block(&b);
            self.len = 0;
        }
    }
}

/// An AES-GCM key: the cipher and the hash subkey `H = CIPH_K(0^128)`.
#[derive(Clone)]
pub struct Gcm {
    aes: Aes,
    h: u128,
}

impl core::fmt::Debug for Gcm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Gcm").field("aes", &self.aes).finish_non_exhaustive()
    }
}

impl Gcm {
    /// Set up GCM with `aes` (SP 800-38D 7.1 step 1).
    pub fn new(aes: Aes) -> Gcm {
        let mut z = [0u8; 16];
        aes.encrypt_block(&mut z);
        Gcm { aes, h: u128::from_be_bytes(z) }
    }

    /// Start an encryption or decryption with `nonce` (any length; an
    /// empty nonce is treated as GHASH of nothing, which SP 800-38D does not
    /// allow and callers should refuse).
    pub fn stream(&self, nonce: &[u8]) -> GcmStream {
        // 7.1 step 2: J0 = IV || 0^31 || 1 for a 96-bit IV, else
        // GHASH(IV || 0^(s+64) || [len(IV)]_64).
        let j0 = if nonce.len() == 12 {
            let mut b = [0u8; 16];
            b[..12].copy_from_slice(nonce);
            b[15] = 1;
            u128::from_be_bytes(b)
        } else {
            let mut g = Ghash::new(self.h);
            g.update(nonce);
            g.flush();
            let mut len = [0u8; 16];
            len[8..].copy_from_slice(&((nonce.len() as u64) * 8).to_be_bytes());
            g.block(&len);
            g.y
        };
        let mut ej0 = j0.to_be_bytes();
        self.aes.encrypt_block(&mut ej0);
        GcmStream {
            aes: self.aes.clone(),
            ghash: Ghash::new(self.h),
            ej0,
            counter: j0,
            keystream: [0; 16],
            used: 16,
            aad_len: 0,
            data_len: 0,
            in_data: false,
        }
    }

    /// Encrypt `data` in place and write the tag (its length: 1..=16 bytes,
    /// the leftmost bits of the full tag; SP 800-38D 7.1).
    pub fn seal(&self, nonce: &[u8], aad: &[u8], data: &mut [u8], tag: &mut [u8]) {
        let mut s = self.stream(nonce);
        s.aad(aad);
        s.encrypt(data);
        s.tag(tag);
    }

    /// Check the tag and decrypt `data` in place (SP 800-38D 7.2); false,
    /// with `data` untouched, if the tag does not match. Tags of 1..=16
    /// bytes are accepted.
    pub fn open(&self, nonce: &[u8], aad: &[u8], data: &mut [u8], tag: &[u8]) -> bool {
        if tag.is_empty() || tag.len() > 16 {
            return false;
        }
        let mut s = self.stream(nonce);
        s.aad(aad);
        let mut copy: Vec<u8> = data.to_vec();
        s.decrypt(&mut copy);
        let mut want = [0u8; 16];
        s.tag(&mut want[..tag.len()]);
        if !crate::ct_eq(&want[..tag.len()], tag) {
            return false;
        }
        data.copy_from_slice(&copy);
        true
    }
}

/// One GCM encryption or decryption fed in pieces: additional data first
/// ([`aad`](Self::aad)), then the text in chunks of any length, then
/// [`tag`](Self::tag).
#[derive(Clone)]
pub struct GcmStream {
    aes: Aes,
    ghash: Ghash,
    /// `CIPH_K(J0)`, masked onto the GHASH result for the tag.
    ej0: [u8; 16],
    /// The last counter block used (`inc_32` increments its low word).
    counter: u128,
    keystream: [u8; 16],
    /// Bytes of `keystream` already used.
    used: usize,
    aad_len: u64,
    data_len: u64,
    in_data: bool,
}

impl core::fmt::Debug for GcmStream {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GcmStream")
            .field("aad_len", &self.aad_len)
            .field("data_len", &self.data_len)
            .finish_non_exhaustive()
    }
}

impl GcmStream {
    /// Absorb additional authenticated data. Ignored once text has been
    /// processed (the AAD comes first in the GHASH input).
    pub fn aad(&mut self, aad: &[u8]) {
        if self.in_data {
            return;
        }
        self.ghash.update(aad);
        self.aad_len += aad.len() as u64;
    }

    fn start_data(&mut self) {
        if !self.in_data {
            self.ghash.flush();
            self.in_data = true;
        }
    }

    /// XOR the GCTR keystream (6.5, starting at `inc_32(J0)`) onto `data`.
    fn gctr(&mut self, data: &mut [u8]) {
        for x in data.iter_mut() {
            if self.used == 16 {
                // inc_32: the low 32 bits count modulo 2^32.
                let low = (self.counter as u32).wrapping_add(1);
                self.counter = (self.counter & !0xffff_ffffu128) | u128::from(low);
                self.keystream = self.counter.to_be_bytes();
                self.aes.encrypt_block(&mut self.keystream);
                self.used = 0;
            }
            *x ^= self.keystream[self.used];
            self.used += 1;
        }
        self.data_len += data.len() as u64;
    }

    /// Encrypt a chunk in place.
    pub fn encrypt(&mut self, data: &mut [u8]) {
        self.start_data();
        self.gctr(data);
        self.ghash.update(data);
    }

    /// Decrypt a chunk in place (the tag is checked by the caller with
    /// [`tag`](Self::tag)).
    pub fn decrypt(&mut self, data: &mut [u8]) {
        self.start_data();
        self.ghash.update(data);
        self.gctr(data);
    }

    /// The tag so far (7.1 steps 5–6), truncated to `out.len()` (at most
    /// 16 bytes are written).
    pub fn tag(&self, out: &mut [u8]) {
        let mut g = self.ghash.clone();
        g.flush();
        let mut len = [0u8; 16];
        len[..8].copy_from_slice(&(self.aad_len * 8).to_be_bytes());
        len[8..].copy_from_slice(&(self.data_len * 8).to_be_bytes());
        g.block(&len);
        let s = g.y.to_be_bytes();
        for (i, o) in out.iter_mut().take(16).enumerate() {
            *o = s[i] ^ self.ej0[i];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::hex;

    struct Case {
        key: &'static str,
        iv: &'static str,
        pt: &'static str,
        aad: &'static str,
        ct: &'static str,
        tag: &'static str,
    }

    const P60: &str = "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525\
                       b16aedf5aa0de657ba637b39";
    const P_LAST4: &str = "1aafd255";
    const A: &str = "feedfacedeadbeeffeedfacedeadbeefabaddad2";
    const K1: &str = "feffe9928665731c6d6a8f9467308308";

    /// "The Galois/Counter Mode of Operation (GCM)" (McGrew, Viega), test
    /// cases 1–4, 6 (60-byte IV), 13–16 (AES-256), each checked with OpenSSL.
    fn cases() -> alloc::vec::Vec<Case> {
        alloc::vec![
            Case {
                key: "00000000000000000000000000000000",
                iv: "000000000000000000000000",
                pt: "",
                aad: "",
                ct: "",
                tag: "58e2fccefa7e3061367f1d57a4e7455a",
            },
            Case {
                key: "00000000000000000000000000000000",
                iv: "000000000000000000000000",
                pt: "00000000000000000000000000000000",
                aad: "",
                ct: "0388dace60b6a392f328c2b971b2fe78",
                tag: "ab6e47d42cec13bdf53a67b21257bddf",
            },
            Case {
                key: K1,
                iv: "cafebabefacedbaddecaf888",
                pt: "",
                aad: "",
                ct: "42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa05\
                     1ba30b396a0aac973d58e091473f5985",
                tag: "4d5c2af327cd64a62cf35abd2ba6fab4",
            },
            Case {
                key: K1,
                iv: "cafebabefacedbaddecaf888",
                pt: "",
                aad: A,
                ct: "42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa05\
                     1ba30b396a0aac973d58e091",
                tag: "5bc94fbc3221a5db94fae95ae7121a47",
            },
            Case {
                key: K1,
                iv: "9313225df88406e555909c5aff5269aa6a7a9538534f7da1e4c303d2a318a728c3c0c95156809539fcf0e2429a6b5254\
                     16aedbf5a0de6a57a637b39b",
                pt: "",
                aad: A,
                ct: "8ce24998625615b603a033aca13fb894be9112a5c3a211a8ba262a3cca7e2ca701e4a9a4fba43c90ccdcb281d48c7c6f\
                     d62875d2aca417034c34aee5",
                tag: "619cc5aefffe0bfa462af43c1699d050",
            },
            Case {
                key: "0000000000000000000000000000000000000000000000000000000000000000",
                iv: "000000000000000000000000",
                pt: "",
                aad: "",
                ct: "",
                tag: "530f8afbc74536b9a963b4f1c4cb738b",
            },
            Case {
                key: "0000000000000000000000000000000000000000000000000000000000000000",
                iv: "000000000000000000000000",
                pt: "00000000000000000000000000000000",
                aad: "",
                ct: "cea7403d4d606b6e074ec5d3baf39d18",
                tag: "d0d1c8a799996bf0265b98b5d48ab919",
            },
            Case {
                key: "feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308",
                iv: "cafebabefacedbaddecaf888",
                pt: "",
                aad: A,
                ct: "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838\
                     c5f61e6393ba7a0abcc9f662",
                tag: "76fc6ece0f4e1768cddf8853bb2d551b",
            },
        ]
    }

    /// The plaintext of cases 3–4, 6 and 16 (60 bytes for those with AAD).
    fn plaintext(c: &Case) -> alloc::vec::Vec<u8> {
        if !c.pt.is_empty() || c.ct.is_empty() {
            return hex(c.pt);
        }
        let mut p = hex(P60);
        p.extend(hex(P_LAST4));
        p.truncate(hex(c.ct).len());
        p
    }

    #[test]
    fn mcgrew_viega_vectors() {
        for (i, c) in cases().iter().enumerate() {
            let gcm = Gcm::new(Aes::new(&hex(c.key)).unwrap());
            let pt = plaintext(c);
            let mut d = pt.clone();
            let mut tag = [0u8; 16];
            gcm.seal(&hex(c.iv), &hex(c.aad), &mut d, &mut tag);
            assert_eq!(d, hex(c.ct), "case {i} ciphertext");
            assert_eq!(tag.to_vec(), hex(c.tag), "case {i} tag");
            assert!(gcm.open(&hex(c.iv), &hex(c.aad), &mut d, &tag));
            assert_eq!(d, pt);
            // A truncated tag (12 bytes) still verifies; a changed one does not.
            assert!(gcm.open(&hex(c.iv), &hex(c.aad), &mut hex(c.ct), &tag[..12]));
            let mut bad = tag;
            bad[0] ^= 1;
            let mut ct = hex(c.ct);
            assert!(!gcm.open(&hex(c.iv), &hex(c.aad), &mut ct, &bad));
            assert_eq!(ct, hex(c.ct), "data untouched on failure");
            assert!(!gcm.open(&hex(c.iv), b"other", &mut ct, &tag));
        }
    }

    /// Streaming in odd-sized pieces gives the one-shot result.
    #[test]
    fn streaming_matches_one_shot() {
        let gcm = Gcm::new(Aes::new(&hex(K1)).unwrap());
        let pt: alloc::vec::Vec<u8> = (0..200u32).map(|i| i as u8).collect();
        let aad = hex(A);
        let nonce = hex("cafebabefacedbaddecaf888");
        let mut one = pt.clone();
        let mut tag = [0u8; 16];
        gcm.seal(&nonce, &aad, &mut one, &mut tag);
        for piece in [1, 7, 16, 33] {
            let mut s = gcm.stream(&nonce);
            for a in aad.chunks(3) {
                s.aad(a);
            }
            let mut d = pt.clone();
            for c in d.chunks_mut(piece) {
                s.encrypt(c);
            }
            assert_eq!(d, one);
            let mut t = [0u8; 16];
            s.tag(&mut t);
            assert_eq!(t, tag);
            let mut s = gcm.stream(&nonce);
            s.aad(&aad);
            for c in d.chunks_mut(piece) {
                s.decrypt(c);
            }
            assert_eq!(d, pt);
            s.tag(&mut t);
            assert_eq!(t, tag);
        }
    }
}
