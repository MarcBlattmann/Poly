//! Block cipher confidentiality modes of SP 800-38A for AES: ECB (6.1),
//! CBC (6.2) and CFB with 8-bit or 128-bit segments (6.3); and PKCS#7
//! padding (RFC 5652 6.3).
//!
//! The IV is updated in place, so calling a function twice on consecutive
//! pieces gives the same result as one call over both: the chaining a CNG
//! key keeps between `BCryptEncrypt` calls.

use crate::aes::Aes;
use alloc::vec::Vec;

const B: usize = 16;

fn block_mut(c: &mut [u8]) -> &mut [u8; 16] {
    // `chunks_exact_mut(16)` only yields 16-byte chunks.
    c.try_into().expect("a 16-byte chunk")
}

/// ECB encryption (SP 800-38A 6.1); false (data unchanged) unless the length
/// is a multiple of 16.
pub fn ecb_encrypt(aes: &Aes, data: &mut [u8]) -> bool {
    if !data.len().is_multiple_of(B) {
        return false;
    }
    data.chunks_exact_mut(B).for_each(|c| aes.encrypt_block(block_mut(c)));
    true
}

/// ECB decryption (SP 800-38A 6.1).
pub fn ecb_decrypt(aes: &Aes, data: &mut [u8]) -> bool {
    if !data.len().is_multiple_of(B) {
        return false;
    }
    data.chunks_exact_mut(B).for_each(|c| aes.decrypt_block(block_mut(c)));
    true
}

/// CBC encryption (SP 800-38A 6.2): `C_j = CIPH(P_j ^ C_(j-1))`, `C_0 = IV`.
/// The IV becomes the last ciphertext block.
pub fn cbc_encrypt(aes: &Aes, iv: &mut [u8; 16], data: &mut [u8]) -> bool {
    if !data.len().is_multiple_of(B) {
        return false;
    }
    for c in data.chunks_exact_mut(B) {
        let c = block_mut(c);
        for (x, v) in c.iter_mut().zip(iv.iter()) {
            *x ^= v;
        }
        aes.encrypt_block(c);
        *iv = *c;
    }
    true
}

/// CBC decryption (SP 800-38A 6.2): `P_j = CIPH^-1(C_j) ^ C_(j-1)`.
pub fn cbc_decrypt(aes: &Aes, iv: &mut [u8; 16], data: &mut [u8]) -> bool {
    if !data.len().is_multiple_of(B) {
        return false;
    }
    for c in data.chunks_exact_mut(B) {
        let c = block_mut(c);
        let next = *c;
        aes.decrypt_block(c);
        for (x, v) in c.iter_mut().zip(iv.iter()) {
            *x ^= v;
        }
        *iv = next;
    }
    true
}

/// CFB (SP 800-38A 6.3) with an `s`-byte segment; the input block is shifted
/// left by `s` bytes and the ciphertext segment shifted in.
fn cfb(aes: &Aes, iv: &mut [u8; 16], segment: usize, data: &mut [u8], encrypt: bool) -> bool {
    if !(segment == 1 || segment == B) || !data.len().is_multiple_of(segment) {
        return false;
    }
    for seg in data.chunks_exact_mut(segment) {
        let mut o = *iv;
        aes.encrypt_block(&mut o);
        let mut ct = [0u8; B];
        for (i, x) in seg.iter_mut().enumerate() {
            if encrypt {
                *x ^= o[i];
                ct[i] = *x;
            } else {
                ct[i] = *x;
                *x ^= o[i];
            }
        }
        iv.copy_within(segment.., 0);
        iv[B - segment..].copy_from_slice(&ct[..segment]);
    }
    true
}

/// CFB encryption with 1-byte (CFB8) or 16-byte (CFB128) segments; false
/// (data unchanged) for another segment size or a length that is not a
/// multiple of it.
pub fn cfb_encrypt(aes: &Aes, iv: &mut [u8; 16], segment: usize, data: &mut [u8]) -> bool {
    cfb(aes, iv, segment, data, true)
}

/// CFB decryption, as [`cfb_encrypt`].
pub fn cfb_decrypt(aes: &Aes, iv: &mut [u8; 16], segment: usize, data: &mut [u8]) -> bool {
    cfb(aes, iv, segment, data, false)
}

/// Append PKCS#7 padding (RFC 5652 6.3): `k - (len mod k)` bytes, each of
/// that value (a whole block when the length is already a multiple).
/// `block` must be 1..=255.
pub fn pkcs7_pad(data: &mut Vec<u8>, block: usize) {
    let block = block.clamp(1, 255);
    let n = block - data.len() % block;
    data.resize(data.len() + n, n as u8);
}

/// The length of `data` without its PKCS#7 padding, or None when the
/// padding is malformed (or the length is not a positive multiple of
/// `block`).
pub fn pkcs7_unpad(data: &[u8], block: usize) -> Option<usize> {
    if block == 0 || block > 255 || data.is_empty() || !data.len().is_multiple_of(block) {
        return None;
    }
    let n = usize::from(*data.last()?);
    if n == 0 || n > block || data[data.len() - n..].iter().any(|&b| usize::from(b) != n) {
        return None;
    }
    Some(data.len() - n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::hex;

    const K128: &str = "2b7e151628aed2a6abf7158809cf4f3c";
    const K256: &str = "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4";
    const IV: &str = "000102030405060708090a0b0c0d0e0f";
    const PT: &str = "6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51\
                      30c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710";

    fn iv() -> [u8; 16] {
        hex(IV).try_into().unwrap()
    }

    type Enc = fn(&Aes, &mut [u8; 16], &mut [u8]) -> bool;

    /// Encrypt in one call and in pieces, then decrypt.
    fn check(key: &str, pt: &[u8], ct: &str, enc: Enc, dec: Enc, piece: usize) {
        let aes = Aes::new(&hex(key)).unwrap();
        let mut d = pt.to_vec();
        let mut v = iv();
        assert!(enc(&aes, &mut v, &mut d));
        assert_eq!(d, hex(ct));
        let mut d2 = pt.to_vec();
        let mut v = iv();
        for c in d2.chunks_mut(piece) {
            assert!(enc(&aes, &mut v, c));
        }
        assert_eq!(d2, d);
        let mut v = iv();
        assert!(dec(&aes, &mut v, &mut d));
        assert_eq!(d, pt);
    }

    /// SP 800-38A F.1.1/F.1.2 and F.1.5/F.1.6 (ECB-AES128, ECB-AES256).
    #[test]
    fn sp800_38a_ecb() {
        for (key, ct) in [
            (
                K128,
                "3ad77bb40d7a3660a89ecaf32466ef97f5d3d58503b9699de785895a96fdbaaf\
                 43b1cd7f598ece23881b00e3ed0306887b0c785e27e8ad3f8223207104725dd4",
            ),
            (
                K256,
                "f3eed1bdb5d2a03c064b5a7e3db181f8591ccb10d410ed26dc5ba74a31362870\
                 b6ed21b99ca6f4f9f153e7b1beafed1d23304b7a39f9f3ff067d8d8f9e24ecc7",
            ),
        ] {
            let aes = Aes::new(&hex(key)).unwrap();
            let mut d = hex(PT);
            assert!(ecb_encrypt(&aes, &mut d));
            assert_eq!(d, hex(ct));
            assert!(ecb_decrypt(&aes, &mut d));
            assert_eq!(d, hex(PT));
        }
        let aes = Aes::new(&hex(K128)).unwrap();
        assert!(!ecb_encrypt(&aes, &mut [0u8; 15]));
    }

    /// SP 800-38A F.2.1–F.2.6 (CBC-AES128, CBC-AES256).
    #[test]
    fn sp800_38a_cbc() {
        let ct128 = "7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b2\
                     73bed6b8e3c1743b7116e69e222295163ff1caa1681fac09120eca307586e1a7";
        let ct256 = "f58c4c04d6e5f1ba779eabfb5f7bfbd69cfc4e967edb808d679f777bc6702c7d\
                     39f23369a9d9bacfa530e26304231461b2eb05e2c39be9fcda6c19078c6a9d1b";
        check(K128, &hex(PT), ct128, cbc_encrypt, cbc_decrypt, 16);
        check(K256, &hex(PT), ct256, cbc_encrypt, cbc_decrypt, 32);
        let aes = Aes::new(&hex(K128)).unwrap();
        assert!(!cbc_encrypt(&aes, &mut iv(), &mut [0u8; 17]));
    }

    /// SP 800-38A F.3.7–F.3.12 (CFB8, 18 bytes) and F.3.13–F.3.18 (CFB128).
    #[test]
    fn sp800_38a_cfb() {
        let pt = hex(PT);
        check(
            K128,
            &pt[..18],
            "3b79424c9c0dd436bace9e0ed4586a4f32b9",
            |a, v, d| cfb_encrypt(a, v, 1, d),
            |a, v, d| cfb_decrypt(a, v, 1, d),
            5,
        );
        check(
            K256,
            &pt[..18],
            "dc1f1a8520a64db55fcc8ac554844e889700",
            |a, v, d| cfb_encrypt(a, v, 1, d),
            |a, v, d| cfb_decrypt(a, v, 1, d),
            7,
        );
        check(
            K128,
            &pt,
            "3b3fd92eb72dad20333449f8e83cfb4ac8a64537a0b3a93fcde3cdad9f1ce58b\
             26751f67a3cbb140b1808cf187a4f4dfc04b05357c5d1c0eeac4c66f9ff7f2e6",
            |a, v, d| cfb_encrypt(a, v, 16, d),
            |a, v, d| cfb_decrypt(a, v, 16, d),
            16,
        );
        check(
            K256,
            &pt,
            "dc7e84bfda79164b7ecd8486985d386039ffed143b28b1c832113c6331e5407b\
             df10132415e54b92a13ed0a8267ae2f975a385741ab9cef82031623d55b1e471",
            |a, v, d| cfb_encrypt(a, v, 16, d),
            |a, v, d| cfb_decrypt(a, v, 16, d),
            48,
        );
        let aes = Aes::new(&hex(K128)).unwrap();
        assert!(!cfb_encrypt(&aes, &mut iv(), 8, &mut [0u8; 16]));
        assert!(!cfb_encrypt(&aes, &mut iv(), 16, &mut [0u8; 15]));
    }

    #[test]
    fn pkcs7() {
        let mut v = b"YELLOW SUBMARINE".to_vec();
        pkcs7_pad(&mut v, 16);
        assert_eq!(v.len(), 32);
        assert!(v[16..].iter().all(|&b| b == 16));
        assert_eq!(pkcs7_unpad(&v, 16), Some(16));
        let mut v = b"abc".to_vec();
        pkcs7_pad(&mut v, 16);
        assert_eq!(&v[3..], &[13u8; 13]);
        assert_eq!(pkcs7_unpad(&v, 16), Some(3));
        let mut bad = v.clone();
        bad[5] = 12;
        assert_eq!(pkcs7_unpad(&bad, 16), None);
        bad[15] = 0;
        assert_eq!(pkcs7_unpad(&bad, 16), None);
        bad[15] = 17;
        assert_eq!(pkcs7_unpad(&bad, 16), None);
        assert_eq!(pkcs7_unpad(&v[..15], 16), None);
        assert_eq!(pkcs7_unpad(&[], 16), None);
    }
}
