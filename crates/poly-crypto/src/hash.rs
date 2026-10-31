//! One interface over the hash functions: [`HashAlg`] names them, [`Hasher`]
//! runs one.

use crate::md5::Md5;
use crate::sha1::Sha1;
use crate::sha256::Sha256;
use crate::sha512::{Sha384, Sha512};
use alloc::vec::Vec;

/// A hash function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HashAlg {
    /// MD5 (RFC 1321).
    Md5,
    /// SHA-1 (FIPS 180-4).
    Sha1,
    /// SHA-256 (FIPS 180-4).
    Sha256,
    /// SHA-384 (FIPS 180-4).
    Sha384,
    /// SHA-512 (FIPS 180-4).
    Sha512,
}

impl HashAlg {
    /// The digest length in bytes.
    pub const fn output_len(self) -> usize {
        match self {
            HashAlg::Md5 => 16,
            HashAlg::Sha1 => 20,
            HashAlg::Sha256 => 32,
            HashAlg::Sha384 => 48,
            HashAlg::Sha512 => 64,
        }
    }

    /// The input block length in bytes (HMAC's `B`, RFC 2104 2).
    pub const fn block_len(self) -> usize {
        match self {
            HashAlg::Md5 | HashAlg::Sha1 | HashAlg::Sha256 => 64,
            HashAlg::Sha384 | HashAlg::Sha512 => 128,
        }
    }
}

/// The running state of one of the hash functions.
#[derive(Clone, Debug)]
enum State {
    Md5(Md5),
    Sha1(Sha1),
    Sha256(Sha256),
    Sha384(Sha384),
    Sha512(Sha512),
}

/// A hash computation in progress; `Clone` duplicates it mid-way.
#[derive(Clone, Debug)]
pub struct Hasher(State);

impl Hasher {
    /// A fresh computation of `alg`.
    pub fn new(alg: HashAlg) -> Self {
        Hasher(match alg {
            HashAlg::Md5 => State::Md5(Md5::new()),
            HashAlg::Sha1 => State::Sha1(Sha1::new()),
            HashAlg::Sha256 => State::Sha256(Sha256::new()),
            HashAlg::Sha384 => State::Sha384(Sha384::new()),
            HashAlg::Sha512 => State::Sha512(Sha512::new()),
        })
    }

    /// Which function this is.
    pub fn alg(&self) -> HashAlg {
        match self.0 {
            State::Md5(_) => HashAlg::Md5,
            State::Sha1(_) => HashAlg::Sha1,
            State::Sha256(_) => HashAlg::Sha256,
            State::Sha384(_) => HashAlg::Sha384,
            State::Sha512(_) => HashAlg::Sha512,
        }
    }

    /// Absorb `data`.
    pub fn update(&mut self, data: &[u8]) {
        match &mut self.0 {
            State::Md5(h) => h.update(data),
            State::Sha1(h) => h.update(data),
            State::Sha256(h) => h.update(data),
            State::Sha384(h) => h.update(data),
            State::Sha512(h) => h.update(data),
        }
    }

    /// The digest ([`HashAlg::output_len`] bytes).
    pub fn finish(self) -> Vec<u8> {
        match self.0 {
            State::Md5(h) => h.finish().to_vec(),
            State::Sha1(h) => h.finish().to_vec(),
            State::Sha256(h) => h.finish().to_vec(),
            State::Sha384(h) => h.finish().to_vec(),
            State::Sha512(h) => h.finish().to_vec(),
        }
    }
}

/// The digest of `data` under `alg`.
pub fn digest(alg: HashAlg, data: &[u8]) -> Vec<u8> {
    let mut h = Hasher::new(alg);
    h.update(data);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::hex;

    const TWO_BLOCK: &[u8] = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
    const LONG: &[u8] = b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu";

    /// The examples of FIPS 180-4 (NIST "Cryptographic Standards and
    /// Guidelines: Examples with Intermediate Values"): "abc", the 448-bit
    /// and 896-bit messages, one million "a"; and the empty message. Each
    /// value also checked with Python's `hashlib`.
    #[test]
    fn fips_180_4_examples() {
        let million = alloc::vec![b'a'; 1_000_000];
        let cases: [(HashAlg, &[u8], &str); 19] = [
            (HashAlg::Sha1, b"", "da39a3ee5e6b4b0d3255bfef95601890afd80709"),
            (HashAlg::Sha1, b"abc", "a9993e364706816aba3e25717850c26c9cd0d89d"),
            (HashAlg::Sha1, TWO_BLOCK, "84983e441c3bd26ebaae4aa1f95129e5e54670f1"),
            (HashAlg::Sha1, LONG, "a49b2446a02c645bf419f995b67091253a04a259"),
            (HashAlg::Sha1, &million, "34aa973cd4c4daa4f61eeb2bdbad27316534016f"),
            (HashAlg::Sha256, b"", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            (HashAlg::Sha256, b"abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
            (HashAlg::Sha256, TWO_BLOCK, "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"),
            (HashAlg::Sha256, LONG, "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1"),
            (HashAlg::Sha256, &million, "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"),
            (
                HashAlg::Sha384,
                b"",
                "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b",
            ),
            (
                HashAlg::Sha384,
                b"abc",
                "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7",
            ),
            (
                HashAlg::Sha384,
                LONG,
                "09330c33f71147e83d192fc782cd1b4753111b173b3b05d22fa08086e3b0f712fcc7c71a557e2db966c3e9fa91746039",
            ),
            (
                HashAlg::Sha384,
                &million,
                "9d0e1809716474cb086e834e310a4a1ced149e9c00f248527972cec5704c2a5b07b8b3dc38ecc4ebae97ddd87f3d8985",
            ),
            (
                HashAlg::Sha512,
                b"",
                "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce\
                 47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e",
            ),
            (
                HashAlg::Sha512,
                b"abc",
                "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
                 2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            ),
            (
                HashAlg::Sha512,
                TWO_BLOCK,
                "204a8fc6dda82f0a0ced7beb8e08a41657c16ef468b228a8279be331a703c335\
                 96fd15c13b1b07f9aa1d3bea57789ca031ad85c7a71dd70354ec631238ca3445",
            ),
            (
                HashAlg::Sha512,
                LONG,
                "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018\
                 501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909",
            ),
            (
                HashAlg::Sha512,
                &million,
                "e718483d0ce769644e2e42c7bc15b4638e1f98b13b2044285632a803afa973eb\
                 de0ff244877ea60a4cb0432ce577c31beb009c5c2c49aa2e4eadb217ad8cc09b",
            ),
        ];
        for (alg, m, d) in cases {
            let got = digest(alg, m);
            assert_eq!(got.len(), alg.output_len());
            assert_eq!(got, hex(d), "{alg:?} of {} bytes", m.len());
        }
    }

    /// Feeding the input in pieces (across block and padding boundaries) or
    /// cloning mid-way gives the same digest.
    #[test]
    fn pieces_and_clones_agree() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 7) as u8).collect();
        for alg in [HashAlg::Md5, HashAlg::Sha1, HashAlg::Sha256, HashAlg::Sha384, HashAlg::Sha512] {
            let whole = digest(alg, &data);
            for split in [0, 1, 55, 56, 63, 64, 111, 112, 127, 128, 129, 999] {
                let mut h = Hasher::new(alg);
                h.update(&data[..split]);
                let copy = h.clone();
                h.update(&data[split..]);
                assert_eq!(h.finish(), whole, "{alg:?} split at {split}");
                let mut c = copy;
                c.update(&data[split..]);
                assert_eq!(c.finish(), whole);
            }
            // Messages whose padding needs one more block.
            for len in 50..140 {
                let mut a = Hasher::new(alg);
                for b in &data[..len] {
                    a.update(core::slice::from_ref(b));
                }
                assert_eq!(a.finish(), digest(alg, &data[..len]));
            }
            assert_eq!(Hasher::new(alg).alg(), alg);
        }
    }
}
