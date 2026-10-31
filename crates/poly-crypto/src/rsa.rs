//! RSA (RFC 8017): the primitives (5.1, 5.2), RSAES-OAEP (7.1),
//! RSAES-PKCS1-v1_5 (7.2), RSASSA-PSS (8.1, EMSA-PSS 9.1) and
//! RSASSA-PKCS1-v1_5 (8.2, EMSA-PKCS1-v1_5 9.2), with MGF1 (B.2.1).
//!
//! Private-key operations use the Chinese remainder theorem (5.1.2 2.b)
//! and are not blinded: not hardened against timing attacks. Decryption
//! failures all look the same to the caller (None).

use crate::bignum::BigUint;
use crate::hash::{HashAlg, Hasher, digest};
use alloc::vec;
use alloc::vec::Vec;

/// An RSA public key `(n, e)` (3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RsaPublicKey {
    /// The modulus.
    pub n: BigUint,
    /// The public exponent.
    pub e: BigUint,
}

impl RsaPublicKey {
    /// A public key; None for an even or tiny modulus (under 512 bits) or
    /// an exponent outside `3..n` that is even.
    pub fn new(n: BigUint, e: BigUint) -> Option<Self> {
        if n.bits() < 512 || !n.is_odd() || !e.is_odd() || e <= BigUint::from_u64(1) || e >= n {
            return None;
        }
        Some(RsaPublicKey { n, e })
    }

    /// The modulus length in bytes (`k`).
    pub fn size(&self) -> usize {
        self.n.bits().div_ceil(8)
    }

    /// The modulus length in bits.
    pub fn bits(&self) -> usize {
        self.n.bits()
    }
}

/// An RSA private key with its CRT values (3.2, second representation).
#[derive(Clone)]
pub struct RsaPrivateKey {
    /// The public half.
    pub public: RsaPublicKey,
    /// The private exponent.
    pub d: BigUint,
    /// The first prime.
    pub p: BigUint,
    /// The second prime.
    pub q: BigUint,
    /// `d mod (p - 1)`.
    pub dp: BigUint,
    /// `d mod (q - 1)`.
    pub dq: BigUint,
    /// `q^-1 mod p`.
    pub qinv: BigUint,
}

impl core::fmt::Debug for RsaPrivateKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RsaPrivateKey").field("bits", &self.public.bits()).finish_non_exhaustive()
    }
}

impl RsaPrivateKey {
    /// The key from `n`, `e` and the primes: `d = e^-1 mod lcm(p-1, q-1)`
    /// (3.2), the CRT exponents and coefficient. None if `p · q != n` or
    /// `e` is not invertible.
    pub fn from_primes(n: BigUint, e: BigUint, p: BigUint, q: BigUint) -> Option<Self> {
        let public = RsaPublicKey::new(n, e)?;
        if p.mul(&q) != public.n {
            return None;
        }
        let one = BigUint::from_u64(1);
        let p1 = p.sub(&one)?;
        let q1 = q.sub(&one)?;
        let lambda = p1.mul(&q1).div_rem(&p1.gcd(&q1))?.0;
        let d = public.e.mod_inverse(&lambda)?;
        let dp = d.rem(&p1)?;
        let dq = d.rem(&q1)?;
        let qinv = q.mod_inverse(&p)?;
        Some(RsaPrivateKey { public, d, p, q, dp, dq, qinv })
    }

    /// The key from all of its values (as a full private key blob carries
    /// them); None if `p · q != n`, a value is out of range, or `d` does
    /// not invert `e`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        n: BigUint,
        e: BigUint,
        d: BigUint,
        p: BigUint,
        q: BigUint,
        dp: BigUint,
        dq: BigUint,
        qinv: BigUint,
    ) -> Option<Self> {
        let public = RsaPublicKey::new(n, e)?;
        if p.mul(&q) != public.n || d.is_zero() || d >= public.n || dp >= p || dq >= q || qinv >= p {
            return None;
        }
        let key = RsaPrivateKey { public, d, p, q, dp, dq, qinv };
        // e·d ≡ 1 (mod p - 1): a cheap consistency check of the parts.
        let p1 = key.p.sub(&BigUint::from_u64(1))?;
        if key.public.e.mul(&key.dp).rem(&p1)? != BigUint::from_u64(1) {
            return None;
        }
        Some(key)
    }
}

/// RSAEP / RSAVP1 (5.1.1, 5.2.2): `input^e mod n`, padded to `k` bytes;
/// None unless `input < n`.
pub fn public_op(key: &RsaPublicKey, input: &[u8]) -> Option<Vec<u8>> {
    let m = BigUint::from_be_bytes(input);
    if m >= key.n {
        return None;
    }
    m.mod_pow(&key.e, &key.n)?.to_be_bytes_padded(key.size())
}

/// RSADP / RSASP1 (5.1.2, 5.2.1) with the CRT: `m1 = c^dP mod p`,
/// `m2 = c^dQ mod q`, `h = qInv (m1 - m2) mod p`, `m = m2 + q h`.
pub fn private_op(key: &RsaPrivateKey, input: &[u8]) -> Option<Vec<u8>> {
    let c = BigUint::from_be_bytes(input);
    if c >= key.public.n {
        return None;
    }
    let m1 = c.mod_pow(&key.dp, &key.p)?;
    let m2 = c.mod_pow(&key.dq, &key.q)?;
    let m2p = m2.rem(&key.p)?;
    let diff = if m1 >= m2p { m1.sub(&m2p)? } else { m1.add(&key.p).sub(&m2p)? };
    let h = key.qinv.mul(&diff).rem(&key.p)?;
    let m = m2.add(&key.q.mul(&h));
    m.to_be_bytes_padded(key.public.size())
}

/// The DER `DigestInfo` prefix for each hash (RFC 8017 9.2, note 1).
fn digest_info_prefix(alg: HashAlg) -> &'static [u8] {
    match alg {
        HashAlg::Md5 => &[
            0x30, 0x20, 0x30, 0x0c, 0x06, 0x08, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x02, 0x05, 0x05, 0x00, 0x04, 0x10,
        ],
        HashAlg::Sha1 => &[0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04, 0x14],
        HashAlg::Sha256 => &[
            0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00, 0x04,
            0x20,
        ],
        HashAlg::Sha384 => &[
            0x30, 0x41, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02, 0x05, 0x00, 0x04,
            0x30,
        ],
        HashAlg::Sha512 => &[
            0x30, 0x51, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03, 0x05, 0x00, 0x04,
            0x40,
        ],
    }
}

/// EMSA-PKCS1-v1_5 (9.2): `0x00 || 0x01 || PS (0xff...) || 0x00 || T`, with
/// `T` the `DigestInfo` (or the bare hash without an algorithm).
fn emsa_pkcs1v15(alg: Option<HashAlg>, hash: &[u8], em_len: usize) -> Option<Vec<u8>> {
    let mut t = Vec::new();
    if let Some(alg) = alg {
        if hash.len() != alg.output_len() {
            return None;
        }
        t.extend_from_slice(digest_info_prefix(alg));
    }
    t.extend_from_slice(hash);
    if em_len < t.len() + 11 {
        return None;
    }
    let mut em = vec![0xffu8; em_len];
    em[0] = 0;
    em[1] = 1;
    em[em_len - t.len() - 1] = 0;
    em[em_len - t.len()..].copy_from_slice(&t);
    Some(em)
}

/// RSASSA-PKCS1-v1_5 verification (8.2.2).
pub fn verify_pkcs1v15(key: &RsaPublicKey, alg: Option<HashAlg>, hash: &[u8], sig: &[u8]) -> bool {
    if sig.len() != key.size() {
        return false;
    }
    let (Some(em), Some(want)) = (public_op(key, sig), emsa_pkcs1v15(alg, hash, key.size())) else {
        return false;
    };
    crate::ct_eq(&em, &want)
}

/// RSASSA-PKCS1-v1_5 signature generation (8.2.1).
pub fn sign_pkcs1v15(key: &RsaPrivateKey, alg: Option<HashAlg>, hash: &[u8]) -> Option<Vec<u8>> {
    let em = emsa_pkcs1v15(alg, hash, key.public.size())?;
    private_op(key, &em)
}

/// MGF1 (B.2.1): `mask_len` bytes of `Hash(seed || C)` for `C = 0, 1, ...`.
pub fn mgf1(alg: HashAlg, seed: &[u8], mask_len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(mask_len + alg.output_len());
    let mut counter = 0u32;
    while out.len() < mask_len {
        let mut h = Hasher::new(alg);
        h.update(seed);
        h.update(&counter.to_be_bytes());
        out.extend_from_slice(&h.finish());
        counter += 1;
    }
    out.truncate(mask_len);
    out
}

/// `H = Hash(0x00 * 8 || mHash || salt)` (9.1.1 steps 5–6).
fn pss_hash(alg: HashAlg, m_hash: &[u8], salt: &[u8]) -> Vec<u8> {
    let mut h = Hasher::new(alg);
    h.update(&[0u8; 8]);
    h.update(m_hash);
    h.update(salt);
    h.finish()
}

/// EMSA-PSS encoding (9.1.1) into `em_len = ceil(em_bits / 8)` bytes.
fn emsa_pss_encode(alg: HashAlg, m_hash: &[u8], salt: &[u8], em_bits: usize) -> Option<Vec<u8>> {
    let h_len = alg.output_len();
    let em_len = em_bits.div_ceil(8);
    if m_hash.len() != h_len || em_len < h_len + salt.len() + 2 {
        return None;
    }
    let h = pss_hash(alg, m_hash, salt);
    let db_len = em_len - h_len - 1;
    // DB = PS || 0x01 || salt.
    let mut db = vec![0u8; db_len];
    db[db_len - salt.len() - 1] = 1;
    db[db_len - salt.len()..].copy_from_slice(salt);
    for (d, m) in db.iter_mut().zip(mgf1(alg, &h, db_len)) {
        *d ^= m;
    }
    db[0] &= 0xff >> (8 * em_len - em_bits);
    let mut em = db;
    em.extend_from_slice(&h);
    em.push(0xbc);
    Some(em)
}

/// EMSA-PSS verification (9.1.2).
fn emsa_pss_verify(alg: HashAlg, m_hash: &[u8], em: &[u8], em_bits: usize, salt_len: usize) -> bool {
    let h_len = alg.output_len();
    let em_len = em_bits.div_ceil(8);
    if m_hash.len() != h_len || em.len() != em_len || em_len < h_len + salt_len + 2 || em[em_len - 1] != 0xbc {
        return false;
    }
    let db_len = em_len - h_len - 1;
    let (masked, h) = (&em[..db_len], &em[db_len..em_len - 1]);
    let top_mask = !(0xffu8 >> (8 * em_len - em_bits));
    if masked[0] & top_mask != 0 {
        return false;
    }
    let mut db: Vec<u8> = masked.iter().zip(mgf1(alg, h, db_len)).map(|(a, b)| a ^ b).collect();
    db[0] &= !top_mask;
    let ps_len = db_len - salt_len - 1;
    if db[..ps_len].iter().any(|&b| b != 0) || db[ps_len] != 1 {
        return false;
    }
    crate::ct_eq(&pss_hash(alg, m_hash, &db[ps_len + 1..]), h)
}

/// RSASSA-PSS verification (8.1.2) with MGF1 of the same hash and a salt of
/// `salt_len` bytes.
pub fn verify_pss(key: &RsaPublicKey, alg: HashAlg, hash: &[u8], sig: &[u8], salt_len: usize) -> bool {
    if sig.len() != key.size() {
        return false;
    }
    let Some(m) = public_op(key, sig) else { return false };
    let em_bits = key.bits() - 1;
    let em_len = em_bits.div_ceil(8);
    // I2OSP(m, emLen): the leading bytes beyond emLen must be zero.
    let extra = m.len() - em_len;
    if m[..extra].iter().any(|&b| b != 0) {
        return false;
    }
    emsa_pss_verify(alg, hash, &m[extra..], em_bits, salt_len)
}

/// RSASSA-PSS signature generation (8.1.1) with the given salt.
pub fn sign_pss(key: &RsaPrivateKey, alg: HashAlg, hash: &[u8], salt: &[u8]) -> Option<Vec<u8>> {
    let em = emsa_pss_encode(alg, hash, salt, key.public.bits() - 1)?;
    private_op(key, &em)
}

/// RSAES-PKCS1-v1_5 encryption (7.2.1): `EM = 0x00 || 0x02 || PS || 0x00 ||
/// M` with at least 8 nonzero random bytes of `PS`.
pub fn encrypt_pkcs1v15(key: &RsaPublicKey, msg: &[u8], random: &mut dyn FnMut(&mut [u8])) -> Option<Vec<u8>> {
    let k = key.size();
    if msg.len() + 11 > k {
        return None;
    }
    let ps_len = k - msg.len() - 3;
    let mut em = vec![0u8; k];
    em[1] = 2;
    let ps = &mut em[2..2 + ps_len];
    random(ps);
    for b in ps.iter_mut() {
        while *b == 0 {
            let mut one = [0u8; 1];
            random(&mut one);
            *b = one[0];
        }
    }
    em[3 + ps_len..].copy_from_slice(msg);
    public_op(key, &em)
}

/// RSAES-PKCS1-v1_5 decryption (7.2.2).
pub fn decrypt_pkcs1v15(key: &RsaPrivateKey, ct: &[u8]) -> Option<Vec<u8>> {
    let k = key.public.size();
    if ct.len() != k || k < 11 {
        return None;
    }
    let em = private_op(key, ct)?;
    if em[0] != 0 || em[1] != 2 {
        return None;
    }
    let sep = em[2..].iter().position(|&b| b == 0)? + 2;
    if sep < 10 {
        return None;
    }
    Some(em[sep + 1..].to_vec())
}

/// RSAES-OAEP encryption (7.1.1) with `alg` for the label hash and MGF1.
pub fn encrypt_oaep(
    key: &RsaPublicKey,
    alg: HashAlg,
    label: &[u8],
    msg: &[u8],
    random: &mut dyn FnMut(&mut [u8]),
) -> Option<Vec<u8>> {
    let k = key.size();
    let h_len = alg.output_len();
    if k < 2 * h_len + 2 || msg.len() > k - 2 * h_len - 2 {
        return None;
    }
    // DB = lHash || PS || 0x01 || M.
    let db_len = k - h_len - 1;
    let mut db = vec![0u8; db_len];
    db[..h_len].copy_from_slice(&digest(alg, label));
    db[db_len - msg.len() - 1] = 1;
    db[db_len - msg.len()..].copy_from_slice(msg);
    let mut seed = vec![0u8; h_len];
    random(&mut seed);
    for (d, m) in db.iter_mut().zip(mgf1(alg, &seed, db_len)) {
        *d ^= m;
    }
    for (s, m) in seed.iter_mut().zip(mgf1(alg, &db, h_len)) {
        *s ^= m;
    }
    let mut em = vec![0u8];
    em.extend_from_slice(&seed);
    em.extend_from_slice(&db);
    public_op(key, &em)
}

/// RSAES-OAEP decryption (7.1.2).
pub fn decrypt_oaep(key: &RsaPrivateKey, alg: HashAlg, label: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
    let k = key.public.size();
    let h_len = alg.output_len();
    if ct.len() != k || k < 2 * h_len + 2 {
        return None;
    }
    let em = private_op(key, ct)?;
    let (y, rest) = (em[0], &em[1..]);
    let (masked_seed, masked_db) = rest.split_at(h_len);
    let seed: Vec<u8> = masked_seed.iter().zip(mgf1(alg, masked_db, h_len)).map(|(a, b)| a ^ b).collect();
    let db: Vec<u8> = masked_db.iter().zip(mgf1(alg, &seed, masked_db.len())).map(|(a, b)| a ^ b).collect();
    let l_hash_ok = crate::ct_eq(&db[..h_len], &digest(alg, label));
    let sep = db[h_len..].iter().position(|&b| b != 0).map(|i| i + h_len);
    match sep {
        Some(i) if y == 0 && l_hash_ok && db[i] == 1 => Some(db[i + 1..].to_vec()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rsa_vectors as v;
    use crate::testutil::hex;

    fn big(s: &str) -> BigUint {
        BigUint::from_be_bytes(&hex(s))
    }

    fn public() -> RsaPublicKey {
        RsaPublicKey::new(big(v::N), BigUint::from_u64(v::E)).unwrap()
    }

    fn private() -> RsaPrivateKey {
        RsaPrivateKey::from_primes(big(v::N), BigUint::from_u64(v::E), big(v::P), big(v::Q)).unwrap()
    }

    /// A counter as the "random" source, so runs repeat.
    fn counter() -> impl FnMut(&mut [u8]) {
        let mut c = 0u8;
        move |b: &mut [u8]| {
            for x in b {
                c = c.wrapping_add(1);
                *x = c;
            }
        }
    }

    /// The CRT values computed from the primes match OpenSSL's, and
    /// `from_parts` takes them.
    #[test]
    fn key_from_primes_matches_openssl() {
        let k = private();
        assert_eq!(k.dp, big(v::DP));
        assert_eq!(k.dq, big(v::DQ));
        assert_eq!(k.qinv, big(v::QINV));
        // OpenSSL's d is modulo φ(n) or λ(n); both give the same dP/dQ.
        let full = RsaPrivateKey::from_parts(
            big(v::N),
            BigUint::from_u64(v::E),
            big(v::D),
            big(v::P),
            big(v::Q),
            big(v::DP),
            big(v::DQ),
            big(v::QINV),
        )
        .unwrap();
        assert_eq!(full.public, public());
        assert!(
            RsaPrivateKey::from_parts(
                big(v::N),
                BigUint::from_u64(v::E),
                big(v::D),
                big(v::P),
                big(v::Q),
                big(v::DQ),
                big(v::DQ),
                big(v::QINV),
            )
            .is_none()
        );
        assert!(RsaPrivateKey::from_primes(big(v::N), BigUint::from_u64(v::E), big(v::P), big(v::P)).is_none());
        assert_eq!(public().size(), 256);
        assert_eq!(public().bits(), 2048);
    }

    #[test]
    fn primitives_invert() {
        let (pubk, k) = (public(), private());
        let m = hex("00112233445566778899");
        let c = public_op(&pubk, &m).unwrap();
        let back = private_op(&k, &c).unwrap();
        assert_eq!(BigUint::from_be_bytes(&back), BigUint::from_be_bytes(&m));
        assert!(public_op(&pubk, &hex(v::N)).is_none());
    }

    /// OpenSSL's PKCS#1 v1.5 SHA-256 signature verifies, and ours is the
    /// same bytes (the scheme is deterministic).
    #[test]
    fn pkcs1v15_signature() {
        let (pubk, k) = (public(), private());
        let h = crate::digest(HashAlg::Sha256, v::MSG);
        assert_eq!(h, hex(v::MSG_SHA256));
        let sig = hex(v::SIG_V15);
        assert!(verify_pkcs1v15(&pubk, Some(HashAlg::Sha256), &h, &sig));
        assert_eq!(sign_pkcs1v15(&k, Some(HashAlg::Sha256), &h).unwrap(), sig);
        assert!(!verify_pkcs1v15(&pubk, Some(HashAlg::Sha1), &h[..20], &sig));
        assert!(!verify_pkcs1v15(&pubk, None, &h, &sig));
        let mut bad = sig.clone();
        bad[100] ^= 1;
        assert!(!verify_pkcs1v15(&pubk, Some(HashAlg::Sha256), &h, &bad));
        // Without a DigestInfo (BCrypt's NULL algorithm): sign and verify.
        let raw = sign_pkcs1v15(&k, None, &h).unwrap();
        assert!(verify_pkcs1v15(&pubk, None, &h, &raw));
        for alg in [HashAlg::Md5, HashAlg::Sha1, HashAlg::Sha384, HashAlg::Sha512] {
            let h = crate::digest(alg, v::MSG);
            let s = sign_pkcs1v15(&k, Some(alg), &h).unwrap();
            assert!(verify_pkcs1v15(&pubk, Some(alg), &h, &s));
        }
    }

    /// OpenSSL's PSS signature (SHA-256, MGF1-SHA-256, 32-byte salt)
    /// verifies; ours verifies with any salt length.
    #[test]
    fn pss_signature() {
        let (pubk, k) = (public(), private());
        let h = hex(v::MSG_SHA256);
        let sig = hex(v::SIG_PSS);
        assert!(verify_pss(&pubk, HashAlg::Sha256, &h, &sig, 32));
        assert!(!verify_pss(&pubk, HashAlg::Sha256, &h, &sig, 20));
        assert!(!verify_pss(&pubk, HashAlg::Sha256, &crate::digest(HashAlg::Sha256, b"x"), &sig, 32));
        for salt_len in [0, 20, 32, 64] {
            let salt: Vec<u8> = (0..salt_len as u8).collect();
            let s = sign_pss(&k, HashAlg::Sha256, &h, &salt).unwrap();
            assert!(verify_pss(&pubk, HashAlg::Sha256, &h, &s, salt_len));
        }
        let h384 = crate::digest(HashAlg::Sha384, v::MSG);
        let s = sign_pss(&k, HashAlg::Sha384, &h384, &[7; 48]).unwrap();
        assert!(verify_pss(&pubk, HashAlg::Sha384, &h384, &s, 48));
    }

    /// OpenSSL's OAEP (SHA-1; SHA-256) and PKCS#1 v1.5 ciphertexts decrypt;
    /// ours round-trip.
    #[test]
    fn encryption() {
        let (pubk, k) = (public(), private());
        assert_eq!(decrypt_oaep(&k, HashAlg::Sha1, b"", &hex(v::CT_OAEP_SHA1)).unwrap(), b"secret");
        assert_eq!(decrypt_oaep(&k, HashAlg::Sha256, b"", &hex(v::CT_OAEP_SHA256)).unwrap(), b"secret");
        assert!(decrypt_oaep(&k, HashAlg::Sha256, b"label", &hex(v::CT_OAEP_SHA256)).is_none());
        assert!(decrypt_oaep(&k, HashAlg::Sha1, b"", &hex(v::CT_OAEP_SHA256)).is_none());
        assert_eq!(decrypt_pkcs1v15(&k, &hex(v::CT_V15)).unwrap(), b"secret");
        let mut rng = counter();
        let c = encrypt_oaep(&pubk, HashAlg::Sha256, b"lbl", b"hello", &mut rng).unwrap();
        assert_eq!(decrypt_oaep(&k, HashAlg::Sha256, b"lbl", &c).unwrap(), b"hello");
        let c = encrypt_pkcs1v15(&pubk, b"hello", &mut rng).unwrap();
        assert_eq!(decrypt_pkcs1v15(&k, &c).unwrap(), b"hello");
        assert!(encrypt_pkcs1v15(&pubk, &[1; 246], &mut rng).is_none());
        assert!(encrypt_oaep(&pubk, HashAlg::Sha256, b"", &[1; 191], &mut rng).is_none());
        let c = encrypt_oaep(&pubk, HashAlg::Sha256, b"", &[1; 190], &mut rng).unwrap();
        assert_eq!(decrypt_oaep(&k, HashAlg::Sha256, b"", &c).unwrap(), [1; 190]);
        // A zero byte from the source is replaced in PKCS#1 v1.5 padding.
        let mut c = 0u8;
        let mut zeros = |b: &mut [u8]| {
            for x in b {
                c = c.wrapping_add(1);
                *x = c % 3;
            }
        };
        let c = encrypt_pkcs1v15(&pubk, b"", &mut zeros).unwrap();
        assert_eq!(decrypt_pkcs1v15(&k, &c).unwrap(), b"");
    }

    #[test]
    fn mgf1_known_value() {
        // MGF1-SHA-1 of "foo", 3 bytes: SHA-1("foo" || 00000000)[..3], checked with Python.
        assert_eq!(mgf1(HashAlg::Sha1, b"foo", 3), crate::digest(HashAlg::Sha1, b"foo\0\0\0\0")[..3].to_vec());
        assert_eq!(mgf1(HashAlg::Sha256, b"x", 40).len(), 40);
    }
}
