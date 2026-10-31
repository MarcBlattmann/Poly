//! Signature verification of signed X.509 data: `sha*WithRSAEncryption`
//! (RFC 3279 2.2.1, RFC 4055 5), RSASSA-PSS (RFC 4055 3.1) and
//! `ecdsa-with-SHA*` (RFC 3279 2.2.3, RFC 5758 3.2).

use crate::cert::{AlgorithmId, Certificate, PublicKey};
use crate::der::{self, Reader, tag};
use crate::oid;
use poly_crypto::HashAlg;
use poly_crypto::bignum::BigUint;

/// Why a signature was not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigError {
    /// An algorithm (or key type, or curve) Poly does not implement.
    Unsupported,
    /// The key does not fit the algorithm (an EC key for RSA ...).
    WrongKey,
    /// Malformed parameters, key or signature encoding.
    Malformed,
    /// Well-formed, and wrong.
    Bad,
}

/// What a signature algorithm identifier asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigScheme {
    RsaPkcs1(HashAlg),
    RsaPss { hash: HashAlg, salt_len: usize },
    Ecdsa(HashAlg),
}

/// The scheme an `AlgorithmIdentifier` names.
pub fn scheme(alg: &AlgorithmId) -> Result<SigScheme, SigError> {
    Ok(match alg.oid.as_str() {
        oid::MD5_WITH_RSA => SigScheme::RsaPkcs1(HashAlg::Md5),
        oid::SHA1_WITH_RSA => SigScheme::RsaPkcs1(HashAlg::Sha1),
        oid::SHA256_WITH_RSA => SigScheme::RsaPkcs1(HashAlg::Sha256),
        oid::SHA384_WITH_RSA => SigScheme::RsaPkcs1(HashAlg::Sha384),
        oid::SHA512_WITH_RSA => SigScheme::RsaPkcs1(HashAlg::Sha512),
        oid::ECDSA_WITH_SHA1 => SigScheme::Ecdsa(HashAlg::Sha1),
        oid::ECDSA_WITH_SHA256 => SigScheme::Ecdsa(HashAlg::Sha256),
        oid::ECDSA_WITH_SHA384 => SigScheme::Ecdsa(HashAlg::Sha384),
        oid::ECDSA_WITH_SHA512 => SigScheme::Ecdsa(HashAlg::Sha512),
        oid::RSASSA_PSS => pss_params(&alg.params)?,
        _ => return Err(SigError::Unsupported),
    })
}

/// `RSASSA-PSS-params` (RFC 4055 3.1): hash [0] (default SHA-1), mask
/// generation [1] (default MGF1 with SHA-1), salt length [2] (default 20),
/// trailer field [3] (must be 1). Poly's PSS uses MGF1 with the same hash,
/// so another MGF hash is unsupported.
fn pss_params(params: &[u8]) -> Result<SigScheme, SigError> {
    let mut hash = HashAlg::Sha1;
    let mut mgf_hash = HashAlg::Sha1;
    let mut salt_len = 20usize;
    if !params.is_empty() {
        let seq = der::parse_one(params).map_err(|_| SigError::Malformed)?;
        let mut r = Reader::new(seq.expect(tag::SEQUENCE).map_err(|_| SigError::Malformed)?);
        let m = |_| SigError::Malformed;
        if let Some(e) = r.read_optional(tag::context_cons(0)).map_err(m)? {
            hash = hash_of(e.value)?;
        }
        if let Some(e) = r.read_optional(tag::context_cons(1)).map_err(m)? {
            let mut s = e.reader().sequence().map_err(m)?;
            if oid::to_dotted(s.oid().map_err(m)?).as_deref() != Some(oid::MGF1) {
                return Err(SigError::Unsupported);
            }
            mgf_hash = hash_of(s.read().map_err(m)?.raw)?;
        }
        if let Some(e) = r.read_optional(tag::context_cons(2)).map_err(m)? {
            salt_len = usize::try_from(e.reader().small_uint().map_err(m)?).map_err(|_| SigError::Malformed)?;
        }
        if let Some(e) = r.read_optional(tag::context_cons(3)).map_err(m)?
            && e.reader().small_uint().map_err(m)? != 1
        {
            return Err(SigError::Unsupported);
        }
        r.finish().map_err(m)?;
    }
    if mgf_hash != hash {
        return Err(SigError::Unsupported);
    }
    Ok(SigScheme::RsaPss { hash, salt_len })
}

/// The hash an encoded hash `AlgorithmIdentifier` names.
fn hash_of(encoded: &[u8]) -> Result<HashAlg, SigError> {
    let mut r = Reader::new(encoded);
    let mut s = r.sequence().map_err(|_| SigError::Malformed)?;
    let o = oid::to_dotted(s.oid().map_err(|_| SigError::Malformed)?).ok_or(SigError::Malformed)?;
    oid::hash_alg(&o).ok_or(SigError::Unsupported)
}

/// Verify `sig` over `signed` with `key` as `alg` says.
pub fn verify(alg: &AlgorithmId, signed: &[u8], sig: &[u8], key: &PublicKey) -> Result<(), SigError> {
    let ok = match (scheme(alg)?, key) {
        (SigScheme::RsaPkcs1(h), PublicKey::Rsa { n, e }) => {
            let k = rsa_key(n, e)?;
            poly_crypto::rsa::verify_pkcs1v15(&k, Some(h), &poly_crypto::digest(h, signed), sig)
        }
        (SigScheme::RsaPss { hash, salt_len }, PublicKey::Rsa { n, e }) => {
            let k = rsa_key(n, e)?;
            poly_crypto::rsa::verify_pss(&k, hash, &poly_crypto::digest(hash, signed), sig, salt_len)
        }
        (SigScheme::Ecdsa(h), PublicKey::Ec { curve, x, y }) => {
            // Ecdsa-Sig-Value ::= SEQUENCE { r INTEGER, s INTEGER }.
            let mut outer = Reader::new(sig);
            let mut s = outer.sequence().map_err(|_| SigError::Malformed)?;
            let r_ = s.unsigned_integer().map_err(|_| SigError::Malformed)?;
            let s_ = s.unsigned_integer().map_err(|_| SigError::Malformed)?;
            s.finish().map_err(|_| SigError::Malformed)?;
            outer.finish().map_err(|_| SigError::Malformed)?;
            if r_.len() > curve.field_len() || s_.len() > curve.field_len() {
                return Err(SigError::Bad);
            }
            poly_crypto::ec::ecdsa_verify(*curve, x, y, &poly_crypto::digest(h, signed), r_, s_)
        }
        _ => return Err(SigError::WrongKey),
    };
    if ok { Ok(()) } else { Err(SigError::Bad) }
}

fn rsa_key(n: &[u8], e: &[u8]) -> Result<poly_crypto::rsa::RsaPublicKey, SigError> {
    poly_crypto::rsa::RsaPublicKey::new(BigUint::from_be_bytes(n), BigUint::from_be_bytes(e)).ok_or(SigError::Malformed)
}

/// Verify `cert`'s signature with `issuer`'s public key. The signature
/// BIT STRING must have no unused bits.
pub fn verify_certificate(cert: &Certificate, issuer: &PublicKey) -> Result<(), SigError> {
    if cert.signature_unused_bits != 0 {
        return Err(SigError::Malformed);
    }
    verify(&cert.signature_algorithm, &cert.tbs, &cert.signature, issuer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cert::fixtures::*;

    fn cert(pem: &str) -> Certificate {
        Certificate::parse(&der(pem)).unwrap()
    }

    #[test]
    fn verifies_the_openssl_chain() {
        let (root, int, leaf) = (cert(ROOT), cert(INTERMEDIATE), cert(LEAF));
        // RSA PKCS#1 v1.5 SHA-256 (self and intermediate), ECDSA P-256 SHA-384 (leaf).
        assert_eq!(verify_certificate(&root, &root.public_key().unwrap()), Ok(()));
        assert_eq!(verify_certificate(&int, &root.public_key().unwrap()), Ok(()));
        assert_eq!(verify_certificate(&leaf, &int.public_key().unwrap()), Ok(()));
        assert_eq!(verify_certificate(&leaf, &root.public_key().unwrap()), Err(SigError::WrongKey));
        let self_signed = cert(SELF_SIGNED);
        assert_eq!(verify_certificate(&self_signed, &self_signed.public_key().unwrap()), Ok(()));
        assert_eq!(verify_certificate(&self_signed, &int.public_key().unwrap()), Err(SigError::Bad));
    }

    #[test]
    fn verifies_pss() {
        let (root, old) = (cert(ROOT), cert(EXPIRED));
        assert_eq!(scheme(&old.signature_algorithm), Ok(SigScheme::RsaPss { hash: HashAlg::Sha256, salt_len: 32 }));
        assert_eq!(verify_certificate(&old, &root.public_key().unwrap()), Ok(()));
    }

    #[test]
    fn tampering_is_caught() {
        let (root, mut int) = (cert(ROOT), cert(INTERMEDIATE));
        let n = int.tbs.len();
        int.tbs[n - 3] ^= 1;
        assert_eq!(verify_certificate(&int, &root.public_key().unwrap()), Err(SigError::Bad));
        let mut leaf = cert(LEAF);
        leaf.signature[10] ^= 1;
        let int = cert(INTERMEDIATE);
        assert!(verify_certificate(&leaf, &int.public_key().unwrap()).is_err());
    }
}
