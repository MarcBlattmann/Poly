//! PBKDF2 (RFC 8018 5.2) with HMAC as the pseudorandom function.

use crate::hash::HashAlg;
use crate::hmac::Hmac;

/// Fill `out` with `PBKDF2(HMAC-alg, password, salt, iterations)`. An
/// iteration count of 0 is treated as 1.
pub fn pbkdf2_hmac(alg: HashAlg, password: &[u8], salt: &[u8], iterations: u64, out: &mut [u8]) {
    let prf = Hmac::new(alg, password);
    for (i, chunk) in out.chunks_mut(alg.output_len()).enumerate() {
        // F(P, S, c, i) = U_1 ^ U_2 ^ ... ^ U_c, U_1 = PRF(P, S || INT(i)).
        let mut h = prf.clone();
        h.update(salt);
        h.update(&(i as u32 + 1).to_be_bytes());
        let mut u = h.finish();
        let mut t = u.clone();
        for _ in 1..iterations.max(1) {
            let mut h = prf.clone();
            h.update(&u);
            u = h.finish();
            for (a, b) in t.iter_mut().zip(&u) {
                *a ^= b;
            }
        }
        chunk.copy_from_slice(&t[..chunk.len()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::hex;
    use alloc::vec;

    fn run(alg: HashAlg, p: &[u8], s: &[u8], c: u64, len: usize) -> alloc::vec::Vec<u8> {
        let mut out = vec![0; len];
        pbkdf2_hmac(alg, p, s, c, &mut out);
        out
    }

    /// RFC 6070 (PBKDF2-HMAC-SHA1), all but the 16777216-iteration case.
    #[test]
    fn rfc_6070() {
        assert_eq!(run(HashAlg::Sha1, b"password", b"salt", 1, 20), hex("0c60c80f961f0e71f3a9b524af6012062fe037a6"));
        assert_eq!(run(HashAlg::Sha1, b"password", b"salt", 2, 20), hex("ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957"));
        assert_eq!(run(HashAlg::Sha1, b"password", b"salt", 4096, 20), hex("4b007901b765489abead49d926f721d065a429c1"));
        assert_eq!(
            run(HashAlg::Sha1, b"passwordPASSWORDpassword", b"saltSALTsaltSALTsaltSALTsaltSALTsalt", 4096, 25),
            hex("3d2eec4fe41c849b80c8d83662c0e44a8b291a964cf2f07038")
        );
        assert_eq!(run(HashAlg::Sha1, b"pass\0word", b"sa\0lt", 4096, 16), hex("56fa6aa75548099dcc37d7f03425e0c3"));
    }

    /// PBKDF2-HMAC-SHA256 and -SHA512, checked with Python's
    /// `hashlib.pbkdf2_hmac`.
    #[test]
    fn sha2_vectors() {
        assert_eq!(
            run(HashAlg::Sha256, b"password", b"salt", 1, 32),
            hex("120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b")
        );
        assert_eq!(
            run(HashAlg::Sha256, b"password", b"salt", 4096, 32),
            hex("c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a")
        );
        assert_eq!(
            run(HashAlg::Sha256, b"passwordPASSWORDpassword", b"saltSALTsaltSALTsaltSALTsaltSALTsalt", 4096, 40),
            hex("348c89dbcbd32b2f32d814b8116e84cf2b17347ebc1800181c4e2a1fb8dd53e1c635518c7dac47e9")
        );
        assert_eq!(run(HashAlg::Sha512, b"password", b"salt", 2, 16), hex("e1d9c16aa681708a45f5c7c4e215ceb6"));
    }
}
