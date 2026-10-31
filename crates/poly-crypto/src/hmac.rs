//! HMAC (RFC 2104, FIPS 198-1): `H((K ^ opad) || H((K ^ ipad) || text))`.

use crate::hash::{HashAlg, Hasher, digest};
use alloc::vec::Vec;

/// An HMAC computation in progress; `Clone` duplicates it mid-way.
#[derive(Clone)]
pub struct Hmac {
    inner: Hasher,
    /// The outer hash with `K ^ opad` already absorbed.
    outer: Hasher,
}

impl core::fmt::Debug for Hmac {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Hmac").field("alg", &self.inner.alg()).finish_non_exhaustive()
    }
}

impl Hmac {
    /// Start an HMAC with `key`: a key longer than the block is hashed
    /// first, a shorter one padded with zeros (RFC 2104 2).
    pub fn new(alg: HashAlg, key: &[u8]) -> Self {
        let b = alg.block_len();
        let mut k = if key.len() > b { digest(alg, key) } else { key.to_vec() };
        k.resize(b, 0);
        let ipad: Vec<u8> = k.iter().map(|x| x ^ 0x36).collect();
        let opad: Vec<u8> = k.iter().map(|x| x ^ 0x5c).collect();
        let mut inner = Hasher::new(alg);
        inner.update(&ipad);
        let mut outer = Hasher::new(alg);
        outer.update(&opad);
        Hmac { inner, outer }
    }

    /// The hash function.
    pub fn alg(&self) -> HashAlg {
        self.inner.alg()
    }

    /// Absorb `data`.
    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    /// The MAC (the hash's full output length).
    pub fn finish(self) -> Vec<u8> {
        let inner = self.inner.finish();
        let mut outer = self.outer;
        outer.update(&inner);
        outer.finish()
    }
}

/// HMAC of `data` under `key`.
pub fn hmac(alg: HashAlg, key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut h = Hmac::new(alg, key);
    h.update(data);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::hex;
    use alloc::vec;

    /// RFC 4231 4.2–4.8: test cases 1–4, 6 and 7 for SHA-256/384/512 (case
    /// 5 truncates; its first 128 bits are checked).
    #[test]
    fn rfc_4231() {
        let long_key = vec![0xaa; 131];
        let cases: [(Vec<u8>, Vec<u8>, [&str; 3]); 6] = [
            (
                vec![0x0b; 20],
                b"Hi There".to_vec(),
                [
                    "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
                    "afd03944d84895626b0825f4ab46907f15f9dadbe4101ec682aa034c7cebc59c\
                     faea9ea9076ede7f4af152e8b2fa9cb6",
                    "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cde\
                     daa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854",
                ],
            ),
            (
                b"Jefe".to_vec(),
                b"what do ya want for nothing?".to_vec(),
                [
                    "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
                    "af45d2e376484031617f78d2b58a6b1b9c7ef464f5a01b47e42ec3736322445e\
                     8e2240ca5e69e2c78b3239ecfab21649",
                    "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea250554\
                     9758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737",
                ],
            ),
            (
                vec![0xaa; 20],
                vec![0xdd; 50],
                [
                    "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
                    "88062608d3e6ad8a0aa2ace014c8a86f0aa635d947ac9febe83ef4e55966144b\
                     2a5ab39dc13814b94e3ab6e101a34f27",
                    "fa73b0089d56a284efb0f0756c890be9b1b5dbdd8ee81a3655f83e33b2279d39\
                     bf3e848279a722c806b485a47e67c807b946a337bee8942674278859e13292fb",
                ],
            ),
            (
                (1..=25u8).collect(),
                vec![0xcd; 50],
                [
                    "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b",
                    "3e8a69b7783c25851933ab6290af6ca77a9981480850009cc5577c6e1f573b4e\
                     6801dd23c4a7d679ccf8a386c674cffb",
                    "b0ba465637458c6990e5a8c5f61d4af7e576d97ff94b872de76f8050361ee3db\
                     a91ca5c11aa25eb4d679275cc5788063a5f19741120c4f2de2adebeb10a298dd",
                ],
            ),
            (
                long_key.clone(),
                b"Test Using Larger Than Block-Size Key - Hash Key First".to_vec(),
                [
                    "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
                    "4ece084485813e9088d2c63a041bc5b44f9ef1012a2b588f3cd11f05033ac4c6\
                     0c2ef6ab4030fe8296248df163f44952",
                    "80b24263c7c1a3ebb71493c1dd7be8b49b46d1f41b4aeec1121b013783f8f352\
                     6b56d037e05f2598bd0fd2215d6a1e5295e64f73f63f0aec8b915a985d786598",
                ],
            ),
            (
                long_key,
                b"This is a test using a larger than block-size key and a larger than block-size data. \
                  The key needs to be hashed before being used by the HMAC algorithm."
                    .to_vec(),
                [
                    "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2",
                    "6617178e941f020d351e2f254e8fd32c602420feb0b8fb9adccebb82461e99c5\
                     a678cc31e799176d3860e6110c46523e",
                    "e37b6a775dc87dbaa4dfa9f96e5e3ffddebd71f8867289865df5a32d20cdc944\
                     b6022cac3c4982b10d5eeb55c3e4de15134676fb6de0446065c97440fa8c6a58",
                ],
            ),
        ];
        for (key, data, macs) in cases {
            for (alg, mac) in [HashAlg::Sha256, HashAlg::Sha384, HashAlg::Sha512].into_iter().zip(macs) {
                assert_eq!(hmac(alg, &key, &data), hex(mac), "{alg:?}");
            }
        }
        // Case 5, truncated to 128 bits.
        let t = hmac(HashAlg::Sha256, &[0x0c; 20], b"Test With Truncation");
        assert_eq!(t[..16], hex("a3b6167473100ee06e0c796c2955552b")[..]);
    }

    /// RFC 2202: HMAC-MD5 cases 1, 2, 6 and HMAC-SHA1 cases 1, 2, 6.
    #[test]
    fn rfc_2202() {
        assert_eq!(hmac(HashAlg::Md5, &[0x0b; 16], b"Hi There"), hex("9294727a3638bb1c13f48ef8158bfc9d"));
        assert_eq!(
            hmac(HashAlg::Md5, b"Jefe", b"what do ya want for nothing?"),
            hex("750c783e6ab0b503eaa86e310a5db738")
        );
        assert_eq!(
            hmac(HashAlg::Md5, &[0xaa; 80], b"Test Using Larger Than Block-Size Key - Hash Key First"),
            hex("6b1ab7fe4bd7bf8f0b62e6ce61b9d0cd")
        );
        assert_eq!(hmac(HashAlg::Sha1, &[0x0b; 20], b"Hi There"), hex("b617318655057264e28bc0b6fb378c8ef146be00"));
        assert_eq!(
            hmac(HashAlg::Sha1, b"Jefe", b"what do ya want for nothing?"),
            hex("effcdf6ae5eb2fa2d27416d5f184df9c259a7c79")
        );
        assert_eq!(
            hmac(HashAlg::Sha1, &[0xaa; 80], b"Test Using Larger Than Block-Size Key - Hash Key First"),
            hex("aa4ae5e15272d00e95705637ce8a3b55ed402112")
        );
    }

    #[test]
    fn streaming_and_clone() {
        let mut h = Hmac::new(HashAlg::Sha256, b"key");
        h.update(b"The quick brown fox ");
        let c = h.clone();
        h.update(b"jumps over the lazy dog");
        let mut c = c;
        c.update(b"jumps over the lazy dog");
        let want = hmac(HashAlg::Sha256, b"key", b"The quick brown fox jumps over the lazy dog");
        assert_eq!(h.finish(), want);
        assert_eq!(c.finish(), want);
        assert_eq!(want, hex("f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"));
    }
}
