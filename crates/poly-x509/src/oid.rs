//! OBJECT IDENTIFIERs (X.690 8.19) as dotted strings, the form Windows'
//! CryptoAPI uses for them (`szOID_*` in `wincrypt.h`), and the ones X.509
//! needs (RFC 5280, RFC 3279, RFC 4055, RFC 5480, RFC 5758, RFC 8017
//! appendix C, PKCS#9).

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

/// The dotted form of an OBJECT IDENTIFIER's contents; None if malformed.
pub fn to_dotted(contents: &[u8]) -> Option<String> {
    let mut arcs: Vec<u64> = Vec::new();
    let mut v: u64 = 0;
    let mut started = false;
    for &b in contents {
        // A leading 0x80 octet is not minimal (X.690 8.19.2).
        if !started && b == 0x80 {
            return None;
        }
        started = b & 0x80 != 0;
        v = v.checked_mul(128)? | u64::from(b & 0x7f);
        if b & 0x80 == 0 {
            arcs.push(v);
            v = 0;
        }
    }
    if started || arcs.is_empty() {
        return None;
    }
    let first = arcs[0];
    let (a, b) = if first < 40 {
        (0, first)
    } else if first < 80 {
        (1, first - 40)
    } else {
        (2, first - 80)
    };
    let mut s = String::new();
    let _ = write!(s, "{a}.{b}");
    for arc in &arcs[1..] {
        let _ = write!(s, ".{arc}");
    }
    Some(s)
}

/// An OBJECT IDENTIFIER's contents from its dotted form; None if malformed.
pub fn from_dotted(s: &str) -> Option<Vec<u8>> {
    let mut arcs = Vec::new();
    for part in s.split('.') {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        arcs.push(part.parse::<u64>().ok()?);
    }
    if arcs.len() < 2 || arcs[0] > 2 || (arcs[0] < 2 && arcs[1] >= 40) {
        return None;
    }
    let mut out = Vec::new();
    let first = arcs[0].checked_mul(40)?.checked_add(arcs[1])?;
    for arc in core::iter::once(first).chain(arcs[2..].iter().copied()) {
        let mut tmp = [0u8; 10];
        let mut n = 0;
        let mut v = arc;
        loop {
            tmp[n] = (v & 0x7f) as u8;
            n += 1;
            v >>= 7;
            if v == 0 {
                break;
            }
        }
        for i in (0..n).rev() {
            out.push(tmp[i] | if i > 0 { 0x80 } else { 0 });
        }
    }
    Some(out)
}

// Attribute types (X.520, RFC 5280 appendix A; `szOID_*` in `wincrypt.h`).
pub const COMMON_NAME: &str = "2.5.4.3";
pub const SURNAME: &str = "2.5.4.4";
pub const SERIAL_NUMBER: &str = "2.5.4.5";
pub const COUNTRY_NAME: &str = "2.5.4.6";
pub const LOCALITY_NAME: &str = "2.5.4.7";
pub const STATE_OR_PROVINCE_NAME: &str = "2.5.4.8";
pub const STREET_ADDRESS: &str = "2.5.4.9";
pub const ORGANIZATION_NAME: &str = "2.5.4.10";
pub const ORGANIZATIONAL_UNIT_NAME: &str = "2.5.4.11";
pub const TITLE: &str = "2.5.4.12";
pub const DESCRIPTION: &str = "2.5.4.13";
pub const POSTAL_CODE: &str = "2.5.4.17";
pub const GIVEN_NAME: &str = "2.5.4.42";
pub const INITIALS: &str = "2.5.4.43";
pub const DN_QUALIFIER: &str = "2.5.4.46";
pub const DOMAIN_COMPONENT: &str = "0.9.2342.19200300.100.1.25";
pub const USER_ID: &str = "0.9.2342.19200300.100.1.1";
/// PKCS#9 emailAddress (`szOID_RSA_emailAddr`).
pub const EMAIL_ADDRESS: &str = "1.2.840.113549.1.9.1";

// Extensions (RFC 5280 4.2).
pub const SUBJECT_KEY_IDENTIFIER: &str = "2.5.29.14";
pub const KEY_USAGE: &str = "2.5.29.15";
pub const SUBJECT_ALT_NAME: &str = "2.5.29.17";
pub const ISSUER_ALT_NAME: &str = "2.5.29.18";
pub const BASIC_CONSTRAINTS: &str = "2.5.29.19";
pub const NAME_CONSTRAINTS: &str = "2.5.29.30";
pub const CRL_DISTRIBUTION_POINTS: &str = "2.5.29.31";
pub const CERTIFICATE_POLICIES: &str = "2.5.29.32";
pub const AUTHORITY_KEY_IDENTIFIER: &str = "2.5.29.35";
pub const EXT_KEY_USAGE: &str = "2.5.29.37";
pub const AUTHORITY_INFO_ACCESS: &str = "1.3.6.1.5.5.7.1.1";

// Extended key usages (RFC 5280 4.2.1.12).
pub const KP_SERVER_AUTH: &str = "1.3.6.1.5.5.7.3.1";
pub const KP_CLIENT_AUTH: &str = "1.3.6.1.5.5.7.3.2";
pub const KP_CODE_SIGNING: &str = "1.3.6.1.5.5.7.3.3";
pub const KP_EMAIL_PROTECTION: &str = "1.3.6.1.5.5.7.3.4";
pub const KP_TIME_STAMPING: &str = "1.3.6.1.5.5.7.3.8";
pub const ANY_EXTENDED_KEY_USAGE: &str = "2.5.29.37.0";

// Public keys (RFC 3279 2.3, RFC 5480 2.1.1).
pub const RSA_ENCRYPTION: &str = "1.2.840.113549.1.1.1";
pub const RSASSA_PSS: &str = "1.2.840.113549.1.1.10";
pub const EC_PUBLIC_KEY: &str = "1.2.840.10045.2.1";
pub const PRIME256V1: &str = "1.2.840.10045.3.1.7";
pub const SECP384R1: &str = "1.3.132.0.34";
pub const SECP521R1: &str = "1.3.132.0.35";

// Signature algorithms (RFC 3279 2.2, RFC 4055 5, RFC 5758 3.2, RFC 8017 C).
pub const MD5_WITH_RSA: &str = "1.2.840.113549.1.1.4";
pub const SHA1_WITH_RSA: &str = "1.2.840.113549.1.1.5";
pub const SHA256_WITH_RSA: &str = "1.2.840.113549.1.1.11";
pub const SHA384_WITH_RSA: &str = "1.2.840.113549.1.1.12";
pub const SHA512_WITH_RSA: &str = "1.2.840.113549.1.1.13";
pub const ECDSA_WITH_SHA1: &str = "1.2.840.10045.4.1";
pub const ECDSA_WITH_SHA256: &str = "1.2.840.10045.4.3.2";
pub const ECDSA_WITH_SHA384: &str = "1.2.840.10045.4.3.3";
pub const ECDSA_WITH_SHA512: &str = "1.2.840.10045.4.3.4";
pub const MGF1: &str = "1.2.840.113549.1.1.8";

// Hash algorithms (RFC 3279 2.1, RFC 4055 2.1, NIST CSOR).
pub const MD5: &str = "1.2.840.113549.2.5";
pub const SHA1: &str = "1.3.14.3.2.26";
pub const SHA256: &str = "2.16.840.1.101.3.4.2.1";
pub const SHA384: &str = "2.16.840.1.101.3.4.2.2";
pub const SHA512: &str = "2.16.840.1.101.3.4.2.3";

/// The hash algorithm an OID names.
pub fn hash_alg(oid: &str) -> Option<poly_crypto::HashAlg> {
    use poly_crypto::HashAlg::*;
    Some(match oid {
        MD5 => Md5,
        SHA1 => Sha1,
        SHA256 => Sha256,
        SHA384 => Sha384,
        SHA512 => Sha512,
        _ => return None,
    })
}

/// The short name CryptoAPI's name strings use for an attribute type (the
/// X.500 keys Learn's `CertNameToStr` page lists), if it has one.
pub fn attribute_key(oid: &str) -> Option<&'static str> {
    Some(match oid {
        COMMON_NAME => "CN",
        SURNAME => "SN",
        SERIAL_NUMBER => "SERIALNUMBER",
        COUNTRY_NAME => "C",
        LOCALITY_NAME => "L",
        STATE_OR_PROVINCE_NAME => "S",
        STREET_ADDRESS => "STREET",
        ORGANIZATION_NAME => "O",
        ORGANIZATIONAL_UNIT_NAME => "OU",
        TITLE => "T",
        DESCRIPTION => "Description",
        POSTAL_CODE => "PostalCode",
        GIVEN_NAME => "G",
        INITIALS => "I",
        DN_QUALIFIER => "dnQualifier",
        DOMAIN_COMPONENT => "DC",
        EMAIL_ADDRESS => "E",
        USER_ID => "UID",
        _ => return None,
    })
}

/// The attribute type a short name (case-insensitive) stands for.
pub fn attribute_oid(key: &str) -> Option<&'static str> {
    const KEYS: [&str; 18] = [
        COMMON_NAME,
        SURNAME,
        SERIAL_NUMBER,
        COUNTRY_NAME,
        LOCALITY_NAME,
        STATE_OR_PROVINCE_NAME,
        STREET_ADDRESS,
        ORGANIZATION_NAME,
        ORGANIZATIONAL_UNIT_NAME,
        TITLE,
        DESCRIPTION,
        POSTAL_CODE,
        GIVEN_NAME,
        INITIALS,
        DN_QUALIFIER,
        DOMAIN_COMPONENT,
        EMAIL_ADDRESS,
        USER_ID,
    ];
    if key.eq_ignore_ascii_case("Email") {
        return Some(EMAIL_ADDRESS);
    }
    KEYS.iter().copied().find(|o| attribute_key(o).is_some_and(|k| k.eq_ignore_ascii_case(key)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_round_trip() {
        // X.690 8.19.5's example: {2 100 3} is 0x81 0x34 0x03.
        assert_eq!(to_dotted(&[0x81, 0x34, 0x03]).unwrap(), "2.100.3");
        assert_eq!(from_dotted("2.100.3").unwrap(), [0x81, 0x34, 0x03]);
        let rsa = [0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
        assert_eq!(to_dotted(&rsa).unwrap(), RSA_ENCRYPTION);
        assert_eq!(from_dotted(RSA_ENCRYPTION).unwrap(), rsa);
        assert_eq!(to_dotted(&[0x55, 0x04, 0x03]).unwrap(), COMMON_NAME);
    }

    #[test]
    fn malformed() {
        assert_eq!(to_dotted(&[0x80, 0x01]), None);
        assert_eq!(to_dotted(&[0x2a, 0x86]), None);
        assert_eq!(from_dotted("1"), None);
        assert_eq!(from_dotted("1.40"), None);
        assert_eq!(from_dotted("1..2"), None);
        assert_eq!(from_dotted("3.1"), None);
    }

    #[test]
    fn attribute_keys() {
        assert_eq!(attribute_key(COMMON_NAME), Some("CN"));
        assert_eq!(attribute_oid("ou"), Some(ORGANIZATIONAL_UNIT_NAME));
        assert_eq!(attribute_oid("E"), Some(EMAIL_ADDRESS));
        assert_eq!(attribute_oid("Email"), Some(EMAIL_ADDRESS));
        assert_eq!(attribute_oid("XX"), None);
    }
}
