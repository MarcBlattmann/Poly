//! AES (FIPS 197) with 128-, 192- and 256-bit keys.
//!
//! The S-box is computed from its definition (FIPS 197 5.1.1: the
//! multiplicative inverse in GF(2^8) followed by the affine transformation)
//! at compile time, and the rounds use the usual 32-bit table form: each
//! table entry is a column of `MixColumns(SubBytes(x))`. Decryption is the
//! "equivalent inverse cipher" of FIPS 197 5.3.5 (`InvMixColumns` applied to
//! the middle round keys). Table lookups depend on secret data: not hardened
//! against cache-timing attacks.

/// Multiply by `x` in GF(2^8) modulo `x^8 + x^4 + x^3 + x + 1` (FIPS 197 4.2.1).
const fn xtime(a: u8) -> u8 {
    (a << 1) ^ if a & 0x80 != 0 { 0x1b } else { 0 }
}

/// Multiplication in GF(2^8) (FIPS 197 4.2).
const fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    p
}

/// The S-box (FIPS 197 5.1.1, Figure 7) and its inverse (5.3.2, Figure 14).
const fn sboxes() -> ([u8; 256], [u8; 256]) {
    // Powers of the generator {03}: every nonzero element is 3^i, and its
    // inverse is 3^(255 - i).
    let mut pow = [0u8; 255];
    let mut log = [0usize; 256];
    let mut p = 1u8;
    let mut i = 0;
    while i < 255 {
        pow[i] = p;
        log[p as usize] = i;
        p = gmul(p, 3);
        i += 1;
    }
    let mut s = [0u8; 256];
    let mut inv = [0u8; 256];
    let mut x = 0usize;
    while x < 256 {
        // The multiplicative inverse; {00} maps to {00}.
        let b = if x == 0 { 0 } else { pow[(255 - log[x]) % 255] };
        // b'_i = b_i ^ b_(i+4) ^ b_(i+5) ^ b_(i+6) ^ b_(i+7) ^ c_i, c = {63}.
        let v = b ^ b.rotate_left(1) ^ b.rotate_left(2) ^ b.rotate_left(3) ^ b.rotate_left(4) ^ 0x63;
        s[x] = v;
        inv[v as usize] = x as u8;
        x += 1;
    }
    (s, inv)
}

const BOXES: ([u8; 256], [u8; 256]) = sboxes();
const SBOX: [u8; 256] = BOXES.0;
const INV_SBOX: [u8; 256] = BOXES.1;

/// `TE[x]`: the column `MixColumns` makes of `(S[x], 0, 0, 0)`:
/// `(2·S[x], S[x], S[x], 3·S[x])`, most significant byte first.
const TE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut x = 0;
    while x < 256 {
        let s = SBOX[x];
        t[x] = u32::from_be_bytes([gmul(s, 2), s, s, gmul(s, 3)]);
        x += 1;
    }
    t
};

/// `TD[x]`: `InvMixColumns` of `(InvS[x], 0, 0, 0)`:
/// `(14·v, 9·v, 13·v, 11·v)`.
const TD: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut x = 0;
    while x < 256 {
        let v = INV_SBOX[x];
        t[x] = u32::from_be_bytes([gmul(v, 14), gmul(v, 9), gmul(v, 13), gmul(v, 11)]);
        x += 1;
    }
    t
};

/// Most rounds (AES-256: 14), so the round keys fit in `4 * 15` words.
const MAX_WORDS: usize = 60;

/// An expanded AES key.
#[derive(Clone)]
pub struct Aes {
    enc: [u32; MAX_WORDS],
    dec: [u32; MAX_WORDS],
    rounds: usize,
}

impl core::fmt::Debug for Aes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Aes").field("key_bits", &(self.key_len() * 8)).finish_non_exhaustive()
    }
}

fn sub_word(w: u32) -> u32 {
    let b = w.to_be_bytes();
    u32::from_be_bytes([SBOX[b[0] as usize], SBOX[b[1] as usize], SBOX[b[2] as usize], SBOX[b[3] as usize]])
}

/// `InvMixColumns` of one column, through the tables: `TD[S[b]]` is
/// `InvMixColumns` of `(b, 0, 0, 0)`.
fn inv_mix_column(w: u32) -> u32 {
    let b = w.to_be_bytes();
    TD[SBOX[b[0] as usize] as usize]
        ^ TD[SBOX[b[1] as usize] as usize].rotate_right(8)
        ^ TD[SBOX[b[2] as usize] as usize].rotate_right(16)
        ^ TD[SBOX[b[3] as usize] as usize].rotate_right(24)
}

impl Aes {
    /// Expand a 16-, 24- or 32-byte key (FIPS 197 5.2); None for another
    /// length.
    pub fn new(key: &[u8]) -> Option<Aes> {
        let nk = match key.len() {
            16 | 24 | 32 => key.len() / 4,
            _ => return None,
        };
        let rounds = nk + 6;
        let words = 4 * (rounds + 1);
        let mut w = [0u32; MAX_WORDS];
        for (i, c) in key.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        let mut rcon: u8 = 1;
        for i in nk..words {
            let mut t = w[i - 1];
            if i % nk == 0 {
                t = sub_word(t.rotate_left(8)) ^ (u32::from(rcon) << 24);
                rcon = xtime(rcon);
            } else if nk > 6 && i % nk == 4 {
                t = sub_word(t);
            }
            w[i] = w[i - nk] ^ t;
        }
        // The equivalent inverse cipher's schedule (FIPS 197 5.3.5): round
        // keys in reverse order, InvMixColumns on all but the first and last.
        let mut dec = [0u32; MAX_WORDS];
        for r in 0..=rounds {
            for c in 0..4 {
                let k = w[4 * (rounds - r) + c];
                dec[4 * r + c] = if r == 0 || r == rounds { k } else { inv_mix_column(k) };
            }
        }
        Some(Aes { enc: w, dec, rounds })
    }

    /// The key length in bytes.
    pub fn key_len(&self) -> usize {
        (self.rounds - 6) * 4
    }

    /// Encrypt one block in place (FIPS 197 5.1).
    pub fn encrypt_block(&self, b: &mut [u8; 16]) {
        let k = &self.enc;
        let mut s = load(b);
        for (c, v) in s.iter_mut().enumerate() {
            *v ^= k[c];
        }
        for r in 1..self.rounds {
            let mut t = [0u32; 4];
            for (c, v) in t.iter_mut().enumerate() {
                // ShiftRows: row j of column c comes from column c + j.
                *v = TE[(s[c] >> 24) as usize]
                    ^ TE[((s[(c + 1) % 4] >> 16) & 0xff) as usize].rotate_right(8)
                    ^ TE[((s[(c + 2) % 4] >> 8) & 0xff) as usize].rotate_right(16)
                    ^ TE[(s[(c + 3) % 4] & 0xff) as usize].rotate_right(24)
                    ^ k[4 * r + c];
            }
            s = t;
        }
        let mut out = [0u32; 4];
        for (c, v) in out.iter_mut().enumerate() {
            *v = u32::from_be_bytes([
                SBOX[(s[c] >> 24) as usize],
                SBOX[((s[(c + 1) % 4] >> 16) & 0xff) as usize],
                SBOX[((s[(c + 2) % 4] >> 8) & 0xff) as usize],
                SBOX[(s[(c + 3) % 4] & 0xff) as usize],
            ]) ^ k[4 * self.rounds + c];
        }
        store(&out, b);
    }

    /// Decrypt one block in place (FIPS 197 5.3.5).
    pub fn decrypt_block(&self, b: &mut [u8; 16]) {
        let k = &self.dec;
        let mut s = load(b);
        for (c, v) in s.iter_mut().enumerate() {
            *v ^= k[c];
        }
        for r in 1..self.rounds {
            let mut t = [0u32; 4];
            for (c, v) in t.iter_mut().enumerate() {
                // InvShiftRows: row j of column c comes from column c - j.
                *v = TD[(s[c] >> 24) as usize]
                    ^ TD[((s[(c + 3) % 4] >> 16) & 0xff) as usize].rotate_right(8)
                    ^ TD[((s[(c + 2) % 4] >> 8) & 0xff) as usize].rotate_right(16)
                    ^ TD[(s[(c + 1) % 4] & 0xff) as usize].rotate_right(24)
                    ^ k[4 * r + c];
            }
            s = t;
        }
        let mut out = [0u32; 4];
        for (c, v) in out.iter_mut().enumerate() {
            *v = u32::from_be_bytes([
                INV_SBOX[(s[c] >> 24) as usize],
                INV_SBOX[((s[(c + 3) % 4] >> 16) & 0xff) as usize],
                INV_SBOX[((s[(c + 2) % 4] >> 8) & 0xff) as usize],
                INV_SBOX[(s[(c + 1) % 4] & 0xff) as usize],
            ]) ^ k[4 * self.rounds + c];
        }
        store(&out, b);
    }
}

fn load(b: &[u8; 16]) -> [u32; 4] {
    core::array::from_fn(|c| u32::from_be_bytes([b[4 * c], b[4 * c + 1], b[4 * c + 2], b[4 * c + 3]]))
}

fn store(s: &[u32; 4], b: &mut [u8; 16]) {
    for (c, w) in s.iter().enumerate() {
        b[4 * c..4 * c + 4].copy_from_slice(&w.to_be_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::hex;

    #[test]
    fn sbox_matches_fips_197_figure_7_corners() {
        assert_eq!(SBOX[0x00], 0x63);
        assert_eq!(SBOX[0x53], 0xed);
        assert_eq!(SBOX[0xff], 0x16);
        assert_eq!(INV_SBOX[0x63], 0x00);
        assert_eq!(INV_SBOX[0x16], 0xff);
    }

    /// FIPS 197 Appendix C.1–C.3: plaintext 00112233...ff with keys
    /// 000102...; and Appendix A.1's key with Appendix B's example.
    #[test]
    fn fips_197_appendix_c() {
        let pt = hex("00112233445566778899aabbccddeeff");
        let cases = [
            ("000102030405060708090a0b0c0d0e0f", "69c4e0d86a7b0430d8cdb78070b4c55a"),
            ("000102030405060708090a0b0c0d0e0f1011121314151617", "dda97ca4864cdfe06eaf70a0ec0d7191"),
            ("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f", "8ea2b7ca516745bfeafc49904b496089"),
            ("2b7e151628aed2a6abf7158809cf4f3c", ""),
        ];
        for (key, ct) in cases {
            let aes = Aes::new(&hex(key)).unwrap();
            assert_eq!(aes.key_len(), hex(key).len());
            if ct.is_empty() {
                // Appendix B: 3243f6a8885a308d313198a2e0370734 -> 3925841d02dc09fbdc118597196a0b32.
                let mut b: [u8; 16] = hex("3243f6a8885a308d313198a2e0370734").try_into().unwrap();
                aes.encrypt_block(&mut b);
                assert_eq!(b.to_vec(), hex("3925841d02dc09fbdc118597196a0b32"));
                continue;
            }
            let mut b: [u8; 16] = pt.clone().try_into().unwrap();
            aes.encrypt_block(&mut b);
            assert_eq!(b.to_vec(), hex(ct));
            aes.decrypt_block(&mut b);
            assert_eq!(b.to_vec(), pt);
        }
    }

    #[test]
    fn bad_key_lengths() {
        for n in [0, 8, 15, 17, 31, 33, 64] {
            assert!(Aes::new(&alloc::vec![0; n]).is_none());
        }
    }
}
