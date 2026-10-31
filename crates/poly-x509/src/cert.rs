//! X.509 v1-v3 certificates (RFC 5280 4.1) and the extensions CryptoAPI
//! programs and chain building read (RFC 5280 4.2.1).

use crate::der::{self, Error, Reader, tag};
use crate::name::{self, GeneralName, Name};
use crate::oid;
use crate::time;
use alloc::string::String;
use alloc::vec::Vec;
use poly_crypto::HashAlg;

/// An `AlgorithmIdentifier` (RFC 5280 4.1.1.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlgorithmId {
    pub oid: String,
    /// The parameters' whole encoding (empty when absent).
    pub params: Vec<u8>,
}

impl AlgorithmId {
    fn parse(r: &mut Reader<'_>) -> Result<AlgorithmId, Error> {
        let mut s = r.sequence()?;
        let oid = oid::to_dotted(s.oid()?).ok_or(Error::Invalid)?;
        let params = if s.is_empty() { Vec::new() } else { s.read()?.raw.to_vec() };
        s.finish()?;
        Ok(AlgorithmId { oid, params })
    }
}

/// A `SubjectPublicKeyInfo` (RFC 5280 4.1.2.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicKeyInfo {
    pub encoded: Vec<u8>,
    pub algorithm: AlgorithmId,
    pub unused_bits: u8,
    /// The BIT STRING's octets.
    pub key: Vec<u8>,
}

impl PublicKeyInfo {
    /// Parse a whole `SubjectPublicKeyInfo` encoding.
    pub fn parse(encoded: &[u8]) -> Result<PublicKeyInfo, Error> {
        let mut outer = Reader::new(encoded);
        let mut r = outer.sequence()?;
        outer.finish()?;
        let algorithm = AlgorithmId::parse(&mut r)?;
        let (unused_bits, key) = r.bit_string()?;
        r.finish()?;
        Ok(PublicKeyInfo { encoded: encoded.to_vec(), algorithm, unused_bits, key: key.to_vec() })
    }

    /// The key it holds, if it is a kind Poly understands.
    pub fn public_key(&self) -> Option<PublicKey> {
        match self.algorithm.oid.as_str() {
            oid::RSA_ENCRYPTION | oid::RSASSA_PSS => {
                // RFC 3279 2.3.1: RSAPublicKey ::= SEQUENCE { modulus, publicExponent }.
                let mut outer = Reader::new(&self.key);
                let mut s = outer.sequence().ok()?;
                let n = s.unsigned_integer().ok()?.to_vec();
                let e = s.unsigned_integer().ok()?.to_vec();
                s.finish().ok()?;
                Some(PublicKey::Rsa { n, e })
            }
            oid::EC_PUBLIC_KEY => {
                // RFC 5480 2.1.1: namedCurve parameters; 2.2: the
                // uncompressed point 04 || X || Y.
                let curve_oid = der::parse_one(&self.algorithm.params).ok()?;
                let curve = match oid::to_dotted(curve_oid.expect(tag::OID).ok()?)?.as_str() {
                    oid::PRIME256V1 => poly_crypto::ec::Curve::P256,
                    oid::SECP384R1 => poly_crypto::ec::Curve::P384,
                    _ => return None,
                };
                let l = curve.field_len();
                if self.key.len() != 1 + 2 * l || self.key[0] != 4 {
                    return None;
                }
                Some(PublicKey::Ec { curve, x: self.key[1..1 + l].to_vec(), y: self.key[1 + l..].to_vec() })
            }
            _ => None,
        }
    }
}

/// A public key in a form the algorithms take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublicKey {
    /// Modulus and exponent, big-endian magnitudes.
    Rsa { n: Vec<u8>, e: Vec<u8> },
    /// A point on a NIST curve.
    Ec { curve: poly_crypto::ec::Curve, x: Vec<u8>, y: Vec<u8> },
}

impl PublicKey {
    /// The key's size in bits (`CertGetPublicKeyLength`).
    pub fn bits(&self) -> usize {
        match self {
            PublicKey::Rsa { n, .. } => poly_crypto::bignum::BigUint::from_be_bytes(n).bits(),
            PublicKey::Ec { curve, .. } => curve.field_len() * 8,
        }
    }
}

/// One extension (RFC 5280 4.1.2.9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extension {
    pub oid: String,
    pub critical: bool,
    /// The OCTET STRING's contents: the extension value's encoding.
    pub value: Vec<u8>,
}

/// Parse `Extensions ::= SEQUENCE SIZE (1..MAX) OF Extension` (the whole
/// encoding).
pub fn parse_extensions(encoded: &[u8]) -> Result<Vec<Extension>, Error> {
    let seq = der::parse_one(encoded)?;
    let mut r = Reader::new(seq.expect(tag::SEQUENCE)?);
    let mut out = Vec::new();
    while !r.is_empty() {
        let mut e = r.sequence()?;
        let oid = oid::to_dotted(e.oid()?).ok_or(Error::Invalid)?;
        let critical = if e.peek_tag() == Some(tag::BOOLEAN) { e.boolean()? } else { false };
        let value = e.octet_string()?.to_vec();
        e.finish()?;
        out.push(Extension { oid, critical, value });
    }
    Ok(out)
}

/// The basicConstraints extension (RFC 5280 4.2.1.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BasicConstraints {
    pub ca: bool,
    pub path_len: Option<u32>,
}

/// Parse a basicConstraints value.
pub fn parse_basic_constraints(v: &[u8]) -> Result<BasicConstraints, Error> {
    let seq = der::parse_one(v)?;
    let mut r = Reader::new(seq.expect(tag::SEQUENCE)?);
    let ca = if r.peek_tag() == Some(tag::BOOLEAN) { r.boolean()? } else { false };
    let path_len = if r.is_empty() { None } else { Some(u32::try_from(r.small_uint()?).map_err(|_| Error::Invalid)?) };
    r.finish()?;
    Ok(BasicConstraints { ca, path_len })
}

/// keyUsage bits (RFC 5280 4.2.1.3), as a number with bit 0 =
/// digitalSignature at the top of the first octet (the BIT STRING as
/// CryptoAPI's `CERT_*_KEY_USAGE` byte values use it: 0x80 digital
/// signature ... 0x04 keyCertSign, 0x02 cRLSign).
pub mod key_usage {
    pub const DIGITAL_SIGNATURE: u16 = 0x80;
    pub const NON_REPUDIATION: u16 = 0x40;
    pub const KEY_ENCIPHERMENT: u16 = 0x20;
    pub const DATA_ENCIPHERMENT: u16 = 0x10;
    pub const KEY_AGREEMENT: u16 = 0x08;
    pub const KEY_CERT_SIGN: u16 = 0x04;
    pub const CRL_SIGN: u16 = 0x02;
    pub const ENCIPHER_ONLY: u16 = 0x01;
    /// In the second octet.
    pub const DECIPHER_ONLY: u16 = 0x8000;
}

/// A parsed certificate. Everything is copied out of the encoding, which
/// is kept as [`Certificate::encoded`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Certificate {
    pub encoded: Vec<u8>,
    /// The `tbsCertificate`'s whole encoding (what the signature covers).
    pub tbs: Vec<u8>,
    /// 0 for v1, 1 for v2, 2 for v3 (the encoded value, as `CERT_V1..3`).
    pub version: u32,
    /// The serial number's INTEGER contents (big-endian two's complement).
    pub serial: Vec<u8>,
    /// The signature algorithm inside the `tbsCertificate`.
    pub tbs_signature_algorithm: AlgorithmId,
    pub issuer: Name,
    pub not_before: u64,
    pub not_after: u64,
    pub subject: Name,
    pub public_key_info: PublicKeyInfo,
    pub issuer_unique_id: Option<(u8, Vec<u8>)>,
    pub subject_unique_id: Option<(u8, Vec<u8>)>,
    pub extensions: Vec<Extension>,
    /// The outer signature algorithm.
    pub signature_algorithm: AlgorithmId,
    pub signature_unused_bits: u8,
    pub signature: Vec<u8>,
}

impl Certificate {
    /// Parse a DER certificate. Trailing bytes after it are refused.
    pub fn parse(encoded: &[u8]) -> Result<Certificate, Error> {
        let mut outer = Reader::new(encoded);
        let cert = outer.read_tag(tag::SEQUENCE)?;
        outer.finish()?;
        let mut c = cert.reader();
        let tbs = c.read_tag(tag::SEQUENCE)?;
        let signature_algorithm = AlgorithmId::parse(&mut c)?;
        let (signature_unused_bits, signature) = c.bit_string()?;
        c.finish()?;

        let mut t = tbs.reader();
        let version = match t.read_optional(tag::context_cons(0))? {
            Some(v) => {
                let mut r = v.reader();
                let n = r.small_uint()?;
                r.finish()?;
                u32::try_from(n).map_err(|_| Error::Invalid)?
            }
            None => 0,
        };
        let serial = t.integer_bytes()?.to_vec();
        let tbs_signature_algorithm = AlgorithmId::parse(&mut t)?;
        let issuer = Name::parse(t.read_tag(tag::SEQUENCE)?.raw)?;
        let mut validity = t.sequence()?;
        let nb = validity.read()?;
        let na = validity.read()?;
        validity.finish()?;
        let not_before = time::parse(nb.tag, nb.value)?;
        let not_after = time::parse(na.tag, na.value)?;
        let subject = Name::parse(t.read_tag(tag::SEQUENCE)?.raw)?;
        let public_key_info = PublicKeyInfo::parse(t.read_tag(tag::SEQUENCE)?.raw)?;
        let unique = |e: Option<der::Tlv<'_>>| -> Result<Option<(u8, Vec<u8>)>, Error> {
            e.map(|e| der::bit_string_contents(e.value).map(|(u, b)| (u, b.to_vec()))).transpose()
        };
        let issuer_unique_id = unique(t.read_optional(tag::context_prim(1))?)?;
        let subject_unique_id = unique(t.read_optional(tag::context_prim(2))?)?;
        let extensions = match t.read_optional(tag::context_cons(3))? {
            Some(e) => parse_extensions(der::parse_one(e.value)?.raw)?,
            None => Vec::new(),
        };
        t.finish()?;
        Ok(Certificate {
            encoded: encoded.to_vec(),
            tbs: tbs.raw.to_vec(),
            version,
            serial,
            tbs_signature_algorithm,
            issuer,
            not_before,
            not_after,
            subject,
            public_key_info,
            issuer_unique_id,
            subject_unique_id,
            extensions,
            signature_algorithm,
            signature_unused_bits,
            signature: signature.to_vec(),
        })
    }

    /// The extension with this OID.
    pub fn extension(&self, oid: &str) -> Option<&Extension> {
        self.extensions.iter().find(|e| e.oid == oid)
    }

    /// The basicConstraints extension, if present and well-formed.
    pub fn basic_constraints(&self) -> Option<BasicConstraints> {
        parse_basic_constraints(&self.extension(oid::BASIC_CONSTRAINTS)?.value).ok()
    }

    /// The keyUsage bits ([`key_usage`]), if the extension is present.
    pub fn key_usage(&self) -> Option<u16> {
        let v = &self.extension(oid::KEY_USAGE)?.value;
        let mut r = Reader::new(v);
        let (_, bits) = r.bit_string().ok()?;
        Some(u16::from(bits.first().copied().unwrap_or(0)) | (u16::from(bits.get(1).copied().unwrap_or(0)) << 8))
    }

    /// The extendedKeyUsage purposes (RFC 5280 4.2.1.12), if present.
    pub fn extended_key_usage(&self) -> Option<Vec<String>> {
        let v = &self.extension(oid::EXT_KEY_USAGE)?.value;
        let mut outer = Reader::new(v);
        let mut r = outer.sequence().ok()?;
        let mut out = Vec::new();
        while !r.is_empty() {
            out.push(oid::to_dotted(r.oid().ok()?)?);
        }
        Some(out)
    }

    /// The subjectAltName entries (empty without the extension).
    pub fn subject_alt_names(&self) -> Vec<GeneralName> {
        self.extension(oid::SUBJECT_ALT_NAME).and_then(|e| name::parse_general_names(&e.value).ok()).unwrap_or_default()
    }

    /// The subjectKeyIdentifier (RFC 5280 4.2.1.2).
    pub fn subject_key_id(&self) -> Option<Vec<u8>> {
        let v = &self.extension(oid::SUBJECT_KEY_IDENTIFIER)?.value;
        Reader::new(v).octet_string().ok().map(<[u8]>::to_vec)
    }

    /// The authorityKeyIdentifier's keyIdentifier (RFC 5280 4.2.1.1).
    pub fn authority_key_id(&self) -> Option<Vec<u8>> {
        let v = &self.extension(oid::AUTHORITY_KEY_IDENTIFIER)?.value;
        let mut outer = Reader::new(v);
        let mut r = outer.sequence().ok()?;
        r.read_optional(tag::context_prim(0)).ok()?.map(|e| e.value.to_vec())
    }

    /// Whether issuer and subject are the same name.
    pub fn is_self_issued(&self) -> bool {
        name::names_match(&self.issuer, &self.subject)
    }

    /// Whether `ft` (a FILETIME) is inside the validity period.
    pub fn is_time_valid(&self, ft: u64) -> bool {
        self.not_before <= ft && ft <= self.not_after
    }

    /// A hash of the whole encoding (the thumbprint for SHA-1).
    pub fn thumbprint(&self, alg: HashAlg) -> Vec<u8> {
        poly_crypto::digest(alg, &self.encoded)
    }

    /// The public key, if Poly understands its kind.
    pub fn public_key(&self) -> Option<PublicKey> {
        self.public_key_info.public_key()
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Certificates made with the OpenSSL command-line tool for these
    //! tests: an RSA-2048 root (SHA-256), an ECDSA P-256 intermediate
    //! (pathlen 0) signed by it, an ECDSA P-384 leaf (signed with
    //! ECDSA-SHA384 by the intermediate, with SANs), an expired RSA leaf
    //! signed by the root with RSASSA-PSS, a self-signed P-256 cert, and a
    //! CA below the intermediate that exceeds its path length.
    pub const ROOT: &str = include_str!("testdata/root.pem");
    pub const INTERMEDIATE: &str = include_str!("testdata/int.pem");
    pub const LEAF: &str = include_str!("testdata/leaf.pem");
    pub const EXPIRED: &str = include_str!("testdata/old.pem");
    pub const SELF_SIGNED: &str = include_str!("testdata/self.pem");
    /// A CA issued by the pathlen-0 intermediate, and a leaf under it.
    pub const TOO_DEEP_CA: &str = include_str!("testdata/sub.pem");
    pub const TOO_DEEP_LEAF: &str = include_str!("testdata/deep.pem");

    pub fn der(pem: &str) -> alloc::vec::Vec<u8> {
        crate::text::pem_decode(pem.as_bytes(), Some("CERTIFICATE")).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::name::{NameForm, NameStrOptions};

    #[test]
    fn parses_the_leaf() {
        let c = Certificate::parse(&der(LEAF)).unwrap();
        assert_eq!(c.version, 2);
        assert_eq!(c.serial, [0x00, 0xab, 0xcd, 0xef, 0x01, 0x23]);
        assert_eq!(c.signature_algorithm.oid, oid::ECDSA_WITH_SHA384);
        assert_eq!(c.tbs_signature_algorithm, c.signature_algorithm);
        let x500 = NameStrOptions::new(NameForm::X500);
        assert_eq!(c.subject.to_str(&x500), "C=CH, L=Zurich, O=Poly Test, CN=www.example.test, E=admin@example.test");
        assert_eq!(c.issuer.to_str(&x500), "C=CH, O=Poly Test, OU=Issuing, CN=Poly Test Intermediate CA");
        assert_eq!(time::civil(c.not_before), (2024, 1, 1, 0, 0, 0));
        // 2054 is past UTCTime's range: a GeneralizedTime.
        assert_eq!(time::civil(c.not_after), (2054, 1, 1, 0, 0, 0));
        assert_eq!(c.basic_constraints(), Some(BasicConstraints { ca: false, path_len: None }));
        assert_eq!(c.key_usage(), Some(key_usage::DIGITAL_SIGNATURE));
        assert_eq!(c.extended_key_usage().unwrap(), [oid::KP_SERVER_AUTH, oid::KP_CLIENT_AUTH]);
        let sans = c.subject_alt_names();
        assert_eq!(sans[0], GeneralName::Dns("www.example.test".into()));
        assert_eq!(sans[1], GeneralName::Dns("*.api.example.test".into()));
        assert_eq!(sans[2], GeneralName::Email("admin@example.test".into()));
        assert_eq!(sans[3], GeneralName::Uri("https://example.test/".into()));
        assert_eq!(sans[4], GeneralName::Ip([192, 0, 2, 1].to_vec()));
        assert!(matches!(c.public_key(), Some(PublicKey::Ec { curve: poly_crypto::ec::Curve::P384, .. })));
        assert_eq!(c.public_key().unwrap().bits(), 384);
        assert!(!c.is_self_issued());
        let int = Certificate::parse(&der(INTERMEDIATE)).unwrap();
        assert_eq!(c.authority_key_id(), int.subject_key_id());
        assert_eq!(int.basic_constraints(), Some(BasicConstraints { ca: true, path_len: Some(0) }));
    }

    #[test]
    fn parses_the_root_and_its_rsa_key() {
        let c = Certificate::parse(&der(ROOT)).unwrap();
        assert!(c.is_self_issued());
        assert_eq!(c.serial, [1]);
        assert_eq!(c.key_usage(), Some(key_usage::KEY_CERT_SIGN | key_usage::CRL_SIGN));
        match c.public_key().unwrap() {
            PublicKey::Rsa { n, e } => {
                assert_eq!(n.len(), 256);
                assert_eq!(e, [1, 0, 1]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(c.thumbprint(HashAlg::Sha1).len(), 20);
    }

    #[test]
    fn refuses_damage() {
        let mut d = der(LEAF);
        assert!(Certificate::parse(&d[..d.len() - 1]).is_err());
        d.push(0);
        assert!(Certificate::parse(&d).is_err());
        assert!(Certificate::parse(&[0x30, 0x00]).is_err());
    }
}
