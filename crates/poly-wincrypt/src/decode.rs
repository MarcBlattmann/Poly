//! `CryptDecodeObjectEx`'s structures (x64 layouts of `wincrypt.h`) built
//! with [`Flat`], and `CERT_INFO` for certificate contexts.
//!
//! CryptoAPI integers (`CRYPT_INTEGER_BLOB`, `CRYPT_UINT_BLOB`) are
//! little-endian (Learn: "CRYPT_INTEGER_BLOB": "the integer ... in
//! little-endian order"), and `X509_CERT`'s signature is byte-reversed
//! unless `CRYPT_DECODE_NO_SIGNATURE_BYTE_REVERSAL_FLAG` is given (Learn,
//! `CryptDecodeObjectEx`).

use crate::flat::Flat;
use crate::status::*;
use alloc::string::String;
use alloc::vec::Vec;
use poly_x509::cert::{self, AlgorithmId, Certificate, Extension, PublicKeyInfo};
use poly_x509::der::{self, Error, Reader, tag};
use poly_x509::name::{GeneralName, Name};
use poly_x509::oid;

// `lpszStructType` values (`wincrypt.h`).
pub const X509_CERT: usize = 1;
pub const X509_CERT_TO_BE_SIGNED: usize = 2;
pub const X509_EXTENSIONS: usize = 5;
pub const X509_NAME: usize = 7;
pub const X509_PUBLIC_KEY_INFO: usize = 8;
pub const X509_AUTHORITY_KEY_ID: usize = 9;
pub const X509_ALTERNATE_NAME: usize = 12;
pub const X509_KEY_USAGE: usize = 14;
pub const X509_BASIC_CONSTRAINTS2: usize = 15;
pub const RSA_CSP_PUBLICKEYBLOB: usize = 19;
pub const X509_UNICODE_NAME: usize = 20;
pub const X509_OCTET_STRING: usize = 25;
pub const X509_BITS: usize = 26;
pub const X509_INTEGER: usize = 27;
pub const X509_MULTI_BYTE_INTEGER: usize = 28;
pub const X509_AUTHORITY_KEY_ID2: usize = 31;
pub const X509_SEQUENCE_OF_ANY: usize = 34;
pub const X509_ENHANCED_KEY_USAGE: usize = 36;
pub const X509_ECC_SIGNATURE: usize = 47;

// Flags (`wincrypt.h`).
pub const CRYPT_DECODE_NOCOPY_FLAG: u32 = 0x1;
pub const CRYPT_DECODE_TO_BE_SIGNED_FLAG: u32 = 0x2;
pub const CRYPT_DECODE_SHARE_OID_STRING_FLAG: u32 = 0x4;
pub const CRYPT_DECODE_NO_SIGNATURE_BYTE_REVERSAL_FLAG: u32 = 0x8;
pub const CRYPT_DECODE_ALLOC_FLAG: u32 = 0x8000;
/// `X509_ASN_ENCODING`, `PKCS_7_ASN_ENCODING`.
pub const X509_ASN_ENCODING: u32 = 0x1;
pub const PKCS_7_ASN_ENCODING: u32 = 0x1_0000;

/// A structure type given as an OID string (`szOID_*`), mapped to the
/// numbered type that decodes it.
pub fn struct_type_by_oid(o: &str) -> Option<usize> {
    Some(match o {
        oid::BASIC_CONSTRAINTS => X509_BASIC_CONSTRAINTS2,
        oid::KEY_USAGE => X509_KEY_USAGE,
        oid::SUBJECT_ALT_NAME | oid::ISSUER_ALT_NAME => X509_ALTERNATE_NAME,
        oid::EXT_KEY_USAGE => X509_ENHANCED_KEY_USAGE,
        oid::SUBJECT_KEY_IDENTIFIER => X509_OCTET_STRING,
        oid::AUTHORITY_KEY_IDENTIFIER => X509_AUTHORITY_KEY_ID2,
        oid::RSA_ENCRYPTION => RSA_CSP_PUBLICKEYBLOB,
        _ => return None,
    })
}

/// The last error for a refused encoding.
pub fn asn1_error(e: Error) -> u32 {
    match e {
        Error::Truncated => CRYPT_E_ASN1_EOD,
        Error::UnexpectedTag { .. } => CRYPT_E_ASN1_BADTAG,
        Error::Invalid | Error::TrailingData => CRYPT_E_ASN1_CORRUPT,
    }
}

fn reversed(b: &[u8]) -> Vec<u8> {
    b.iter().rev().copied().collect()
}

/// `CRYPT_ALGORITHM_IDENTIFIER` (24 bytes) at `at`.
fn put_alg(f: &mut Flat, at: usize, a: &AlgorithmId) {
    f.put_cstr(at, &a.oid);
    f.put_blob(at + 8, &a.params);
}

/// `CERT_PUBLIC_KEY_INFO` (48 bytes) at `at`.
fn put_spki(f: &mut Flat, at: usize, k: &PublicKeyInfo) {
    put_alg(f, at, &k.algorithm);
    f.put_bit_blob(at + 24, k.unused_bits, &k.key);
}

/// `CERT_EXTENSION` array (32 bytes each); its offset.
fn put_extensions(f: &mut Flat, exts: &[Extension]) -> usize {
    let arr = f.reserve(32 * exts.len());
    for (i, e) in exts.iter().enumerate() {
        let at = arr + 32 * i;
        f.put_cstr(at, &e.oid);
        f.put_u32(at + 8, u32::from(e.critical));
        f.put_blob(at + 16, &e.value);
    }
    arr
}

/// `CERT_INFO` (208 bytes) at `at`: `dwVersion` 0, `SerialNumber` 8,
/// `SignatureAlgorithm` 24, `Issuer` 48, `NotBefore` 64, `NotAfter` 72,
/// `Subject` 80, `SubjectPublicKeyInfo` 96, `IssuerUniqueId` 144,
/// `SubjectUniqueId` 168, `cExtension` 192, `rgExtension` 200.
pub const CERT_INFO_SIZE: usize = 208;

pub fn put_cert_info(f: &mut Flat, at: usize, c: &Certificate) {
    f.put_u32(at, c.version);
    f.put_blob(at + 8, &reversed(&c.serial));
    put_alg(f, at + 24, &c.tbs_signature_algorithm);
    f.put_blob(at + 48, &c.issuer.encoded);
    f.put_u64(at + 64, c.not_before);
    f.put_u64(at + 72, c.not_after);
    f.put_blob(at + 80, &c.subject.encoded);
    put_spki(f, at + 96, &c.public_key_info);
    if let Some((u, b)) = &c.issuer_unique_id {
        f.put_bit_blob(at + 144, *u, b);
    }
    if let Some((u, b)) = &c.subject_unique_id {
        f.put_bit_blob(at + 168, *u, b);
    }
    f.put_u32(at + 192, c.extensions.len() as u32);
    if !c.extensions.is_empty() {
        let arr = put_extensions(f, &c.extensions);
        f.put_ptr(at + 200, arr);
    }
}

/// `CERT_RDN_*` value types (`wincrypt.h`) of a string tag.
fn rdn_value_type(t: u8) -> u32 {
    match t {
        tag::OCTET_STRING => 2,
        tag::NUMERIC_STRING => 3,
        tag::PRINTABLE_STRING => 4,
        tag::T61_STRING => 5,
        tag::VIDEOTEX_STRING => 6,
        tag::IA5_STRING => 7,
        tag::GRAPHIC_STRING => 8,
        tag::VISIBLE_STRING => 9,
        tag::GENERAL_STRING => 10,
        tag::UNIVERSAL_STRING => 11,
        tag::BMP_STRING => 12,
        tag::UTF8_STRING => 13,
        // CERT_RDN_ENCODED_BLOB: the value is the whole encoding.
        _ => 1,
    }
}

/// `CERT_NAME_INFO` (16 bytes) at `at`: `cRDN`, `rgRDN` -> `CERT_RDN`
/// (16: `cRDNAttr`, `rgRDNAttr`) -> `CERT_RDN_ATTR` (32: `pszObjId`,
/// `dwValueType`, `Value`). `unicode` (`X509_UNICODE_NAME`): string values
/// as UTF-16 (`cbData` without the NUL).
fn put_name(f: &mut Flat, at: usize, n: &Name, unicode: bool) {
    f.put_u32(at, n.rdns.len() as u32);
    if n.rdns.is_empty() {
        return;
    }
    let rdns = f.reserve(16 * n.rdns.len());
    f.put_ptr(at + 8, rdns);
    for (i, rdn) in n.rdns.iter().enumerate() {
        let r = rdns + 16 * i;
        f.put_u32(r, rdn.len() as u32);
        let attrs = f.reserve(32 * rdn.len());
        f.put_ptr(r + 8, attrs);
        for (j, a) in rdn.iter().enumerate() {
            let p = attrs + 32 * j;
            f.put_cstr(p, &a.oid);
            let vt = rdn_value_type(a.tag);
            f.put_u32(p + 8, vt);
            match (unicode, a.text(), vt) {
                (true, Some(t), 2..) => {
                    let n = f.put_wstr(p + 24, &t);
                    f.put_u32(p + 16, (n * 2) as u32);
                }
                (_, _, 1) => f.put_blob(p + 16, &a.encoded),
                _ => f.put_blob(p + 16, &a.value),
            }
        }
    }
}

/// `CERT_ALT_NAME_INFO` (16 bytes) at `at`; entries of 24 bytes:
/// `dwAltNameChoice`, then the union at +8.
fn put_alt_names(f: &mut Flat, at: usize, names: &[GeneralName]) {
    f.put_u32(at, names.len() as u32);
    if names.is_empty() {
        return;
    }
    let arr = f.reserve(24 * names.len());
    f.put_ptr(at + 8, arr);
    for (i, g) in names.iter().enumerate() {
        let e = arr + 24 * i;
        match g {
            GeneralName::Other { oid, value } => {
                f.put_u32(e, 1);
                // CERT_OTHER_NAME: pszObjId, Value (the explicit value's contents).
                let o = f.reserve(24);
                f.put_cstr(o, oid);
                f.put_blob(o + 8, value);
                f.put_ptr(e + 8, o);
            }
            GeneralName::Email(s) => {
                f.put_u32(e, 2);
                f.put_wstr(e + 8, s);
            }
            GeneralName::Dns(s) => {
                f.put_u32(e, 3);
                f.put_wstr(e + 8, s);
            }
            GeneralName::Directory(n) => {
                f.put_u32(e, 5);
                f.put_blob(e + 8, &n.encoded);
            }
            GeneralName::Uri(s) => {
                f.put_u32(e, 7);
                f.put_wstr(e + 8, s);
            }
            GeneralName::Ip(b) => {
                f.put_u32(e, 8);
                f.put_blob(e + 8, b);
            }
            GeneralName::Registered(o) => {
                f.put_u32(e, 9);
                f.put_cstr(e + 8, o);
            }
            GeneralName::Unsupported(t, raw) => {
                // x400Address (4) and ediPartyName (6) as their encoding.
                f.put_u32(e, if *t & 0x1f == 3 { 4 } else { 6 });
                f.put_blob(e + 8, raw);
            }
        }
    }
}

/// Decode `data` as structure type `t` into a block.
pub fn decode(t: usize, data: &[u8], flags: u32) -> Result<Flat, u32> {
    let e = asn1_error;
    let mut f;
    match t {
        X509_CERT_TO_BE_SIGNED => {
            let c = if flags & CRYPT_DECODE_TO_BE_SIGNED_FLAG != 0 {
                // A bare TBSCertificate: wrap it as a certificate with an
                // empty signature to reuse the parser.
                let alg = der::encode_sequence(&[&der::encode(tag::OID, &[0x2a])]);
                let sig = der::encode(tag::BIT_STRING, &[0]);
                Certificate::parse(&der::encode_sequence(&[data, &alg, &sig])).map_err(e)?
            } else {
                Certificate::parse(data).map_err(e)?
            };
            f = Flat::new(CERT_INFO_SIZE);
            put_cert_info(&mut f, 0, &c);
        }
        X509_CERT => {
            // CERT_SIGNED_CONTENT_INFO: ToBeSigned (DER blob), SignatureAlgorithm, Signature.
            let mut outer = Reader::new(data);
            let mut r = outer.sequence().map_err(e)?;
            outer.finish().map_err(e)?;
            let tbs = r.read().map_err(e)?;
            let mut a = r.sequence().map_err(e)?;
            let alg_oid = oid::to_dotted(a.oid().map_err(e)?).ok_or(CRYPT_E_ASN1_CORRUPT)?;
            let params = if a.is_empty() { Vec::new() } else { a.read().map_err(e)?.raw.to_vec() };
            let (unused, sig) = r.bit_string().map_err(e)?;
            f = Flat::new(64);
            f.put_blob(0, tbs.raw);
            put_alg(&mut f, 16, &AlgorithmId { oid: alg_oid, params });
            let sig =
                if flags & CRYPT_DECODE_NO_SIGNATURE_BYTE_REVERSAL_FLAG != 0 { sig.to_vec() } else { reversed(sig) };
            f.put_bit_blob(40, unused, &sig);
        }
        X509_NAME | X509_UNICODE_NAME => {
            let n = Name::parse(data).map_err(e)?;
            f = Flat::new(16);
            put_name(&mut f, 0, &n, t == X509_UNICODE_NAME);
        }
        X509_PUBLIC_KEY_INFO => {
            let k = PublicKeyInfo::parse(data).map_err(e)?;
            f = Flat::new(48);
            put_spki(&mut f, 0, &k);
        }
        X509_EXTENSIONS => {
            let exts = cert::parse_extensions(data).map_err(e)?;
            f = Flat::new(16);
            f.put_u32(0, exts.len() as u32);
            if !exts.is_empty() {
                let arr = put_extensions(&mut f, &exts);
                f.put_ptr(8, arr);
            }
        }
        X509_BASIC_CONSTRAINTS2 => {
            let bc = cert::parse_basic_constraints(data).map_err(e)?;
            f = Flat::new(12);
            f.put_u32(0, u32::from(bc.ca));
            f.put_u32(4, u32::from(bc.path_len.is_some()));
            f.put_u32(8, bc.path_len.unwrap_or(0));
        }
        X509_KEY_USAGE | X509_BITS => {
            let el = der::parse_one(data).map_err(e)?;
            let (unused, bits) = der::bit_string_contents(el.expect(tag::BIT_STRING).map_err(e)?).map_err(e)?;
            f = Flat::new(24);
            f.put_bit_blob(0, unused, bits);
        }
        X509_OCTET_STRING => {
            let el = der::parse_one(data).map_err(e)?;
            f = Flat::new(16);
            f.put_blob(0, el.expect(tag::OCTET_STRING).map_err(e)?);
        }
        X509_INTEGER => {
            let el = der::parse_one(data).map_err(e)?;
            let v = el.expect(tag::INTEGER).map_err(e)?;
            if v.is_empty() || v.len() > 4 {
                return Err(if v.is_empty() { CRYPT_E_ASN1_CORRUPT } else { CRYPT_E_ASN1_LARGE });
            }
            let mut n: i32 = if v[0] & 0x80 != 0 { -1 } else { 0 };
            for &b in v {
                n = (n << 8) | i32::from(b);
            }
            f = Flat::new(4);
            f.put_u32(0, n as u32);
        }
        X509_MULTI_BYTE_INTEGER => {
            let el = der::parse_one(data).map_err(e)?;
            f = Flat::new(16);
            f.put_blob(0, &reversed(el.expect(tag::INTEGER).map_err(e)?));
        }
        X509_SEQUENCE_OF_ANY => {
            let el = der::parse_one(data).map_err(e)?;
            let mut r = Reader::new(el.expect(tag::SEQUENCE).map_err(e)?);
            let mut items = Vec::new();
            while !r.is_empty() {
                items.push(r.read().map_err(e)?.raw);
            }
            f = Flat::new(16);
            f.put_u32(0, items.len() as u32);
            if !items.is_empty() {
                let arr = f.reserve(16 * items.len());
                for (i, it) in items.iter().enumerate() {
                    f.put_blob(arr + 16 * i, it);
                }
                f.put_ptr(8, arr);
            }
        }
        X509_ALTERNATE_NAME => {
            let names = poly_x509::name::parse_general_names(data).map_err(e)?;
            f = Flat::new(16);
            put_alt_names(&mut f, 0, &names);
        }
        X509_ENHANCED_KEY_USAGE => {
            let el = der::parse_one(data).map_err(e)?;
            let mut r = Reader::new(el.expect(tag::SEQUENCE).map_err(e)?);
            let mut oids: Vec<String> = Vec::new();
            while !r.is_empty() {
                oids.push(oid::to_dotted(r.oid().map_err(e)?).ok_or(CRYPT_E_ASN1_CORRUPT)?);
            }
            f = Flat::new(16);
            f.put_u32(0, oids.len() as u32);
            if !oids.is_empty() {
                let arr = f.reserve(8 * oids.len());
                for (i, o) in oids.iter().enumerate() {
                    f.put_cstr(arr + 8 * i, o);
                }
                f.put_ptr(8, arr);
            }
        }
        X509_AUTHORITY_KEY_ID2 => {
            // CERT_AUTHORITY_KEY_ID2_INFO: KeyId (16), AuthorityCertIssuer
            // (CERT_ALT_NAME_INFO, 16), AuthorityCertSerialNumber (16).
            let el = der::parse_one(data).map_err(e)?;
            let mut r = Reader::new(el.expect(tag::SEQUENCE).map_err(e)?);
            f = Flat::new(48);
            if let Some(k) = r.read_optional(tag::context_prim(0)).map_err(e)? {
                f.put_blob(0, k.value);
            }
            if let Some(i) = r.read_optional(tag::context_cons(1)).map_err(e)? {
                let mut names = Vec::new();
                let mut ir = i.reader();
                while !ir.is_empty() {
                    names.push(poly_x509::name::parse_general_name(ir.read().map_err(e)?).map_err(e)?);
                }
                put_alt_names(&mut f, 16, &names);
            }
            if let Some(s) = r.read_optional(tag::context_prim(2)).map_err(e)? {
                f.put_blob(32, &reversed(s.value));
            }
            r.finish().map_err(e)?;
        }
        X509_ECC_SIGNATURE => {
            let el = der::parse_one(data).map_err(e)?;
            let mut r = Reader::new(el.expect(tag::SEQUENCE).map_err(e)?);
            let rr = r.unsigned_integer().map_err(e)?;
            let ss = r.unsigned_integer().map_err(e)?;
            r.finish().map_err(e)?;
            f = Flat::new(32);
            f.put_blob(0, &reversed(rr));
            f.put_blob(16, &reversed(ss));
        }
        RSA_CSP_PUBLICKEYBLOB => {
            // PUBLICKEYSTRUC (bType PUBLICKEYBLOB 6, bVersion 2, reserved,
            // aiKeyAlg CALG_RSA_KEYX 0xa400), RSAPUBKEY ('RSA1', bitlen,
            // pubexp), the modulus little-endian (`wincrypt.h`).
            let el = der::parse_one(data).map_err(e)?;
            let mut r = Reader::new(el.expect(tag::SEQUENCE).map_err(e)?);
            let n = r.unsigned_integer().map_err(e)?;
            let ex = r.unsigned_integer().map_err(e)?;
            r.finish().map_err(e)?;
            if ex.len() > 4 {
                return Err(CRYPT_E_ASN1_LARGE);
            }
            let bits = poly_crypto::bignum::BigUint::from_be_bytes(n).bits() as u32;
            let exp = ex.iter().fold(0u32, |a, &b| (a << 8) | u32::from(b));
            let mut v = Vec::from([6u8, 2, 0, 0]);
            v.extend_from_slice(&0xa400u32.to_le_bytes());
            v.extend_from_slice(&0x3141_5352u32.to_le_bytes());
            v.extend_from_slice(&bits.to_le_bytes());
            v.extend_from_slice(&exp.to_le_bytes());
            v.extend_from_slice(&reversed(n));
            f = Flat::new(0);
            f.data(&v);
        }
        _ => return Err(ERROR_FILE_NOT_FOUND),
    }
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEAF: &str = include_str!("../../poly-x509/src/testdata/leaf.pem");

    fn der_of(pem: &str) -> Vec<u8> {
        poly_x509::text::pem_decode(pem.as_bytes(), Some("CERTIFICATE")).unwrap()
    }

    fn u32_at(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    }

    fn ptr_at(b: &[u8], o: usize, base: usize) -> usize {
        u64::from_le_bytes(b[o..o + 8].try_into().unwrap()) as usize - base
    }

    #[test]
    fn cert_info_layout() {
        let der = der_of(LEAF);
        let out = decode(X509_CERT_TO_BE_SIGNED, &der, 0).unwrap().finish(0x10000);
        assert_eq!(u32_at(&out, 0), 2);
        // Serial 00 ab cd ef 01 23, little-endian.
        assert_eq!(u32_at(&out, 8), 6);
        let s = ptr_at(&out, 16, 0x10000);
        assert_eq!(&out[s..s + 6], &[0x23, 0x01, 0xef, 0xcd, 0xab, 0x00]);
        let o = ptr_at(&out, 24, 0x10000);
        assert_eq!(&out[o..o + 20], b"1.2.840.10045.4.3.3\0");
        let c = Certificate::parse(&der).unwrap();
        assert_eq!(u64::from_le_bytes(out[64..72].try_into().unwrap()), c.not_before);
        assert_eq!(u32_at(&out, 192) as usize, c.extensions.len());
        // The same through the TBS-only form.
        let tbs = decode(X509_CERT_TO_BE_SIGNED, &c.tbs, CRYPT_DECODE_TO_BE_SIGNED_FLAG).unwrap().finish(0x10000);
        assert_eq!(tbs.len(), out.len());
    }

    #[test]
    fn names_and_extensions() {
        let c = Certificate::parse(&der_of(LEAF)).unwrap();
        let out = decode(X509_UNICODE_NAME, &c.subject.encoded, 0).unwrap().finish(0);
        assert_eq!(u32_at(&out, 0), 5);
        let rdns = ptr_at(&out, 8, 0);
        let attrs = ptr_at(&out, rdns + 8, 0);
        assert_eq!(u32_at(&out, attrs + 8), 4); // CERT_RDN_PRINTABLE_STRING "CH"
        assert_eq!(u32_at(&out, attrs + 16), 4); // two UTF-16 characters
        let bc = c.extension(oid::BASIC_CONSTRAINTS).unwrap();
        let out = decode(X509_BASIC_CONSTRAINTS2, &bc.value, 0).unwrap().finish(0);
        assert_eq!(out, [0u8; 12]);
        let san = c.extension(oid::SUBJECT_ALT_NAME).unwrap();
        let out = decode(struct_type_by_oid(oid::SUBJECT_ALT_NAME).unwrap(), &san.value, 0).unwrap().finish(0);
        assert_eq!(u32_at(&out, 0), 5);
        let arr = ptr_at(&out, 8, 0);
        assert_eq!(u32_at(&out, arr), 3); // CERT_ALT_NAME_DNS_NAME
        assert_eq!(u32_at(&out, arr + 24 * 4), 8); // CERT_ALT_NAME_IP_ADDRESS
        let eku = c.extension(oid::EXT_KEY_USAGE).unwrap();
        let out = decode(X509_ENHANCED_KEY_USAGE, &eku.value, 0).unwrap().finish(0);
        assert_eq!(u32_at(&out, 0), 2);
        assert_eq!(decode(9999, &[], 0).err(), Some(ERROR_FILE_NOT_FOUND));
        assert_eq!(decode(X509_NAME, &[0x30, 0x05], 0).err(), Some(CRYPT_E_ASN1_EOD));
        assert_eq!(decode(X509_OCTET_STRING, &[0x05, 0x00], 0).err(), Some(CRYPT_E_ASN1_BADTAG));
    }

    #[test]
    fn integers_and_signatures() {
        let out = decode(X509_INTEGER, &[2, 2, 0xff, 0x7f], 0).unwrap().finish(0);
        assert_eq!(i32::from_le_bytes(out[..4].try_into().unwrap()), -129);
        let out = decode(X509_MULTI_BYTE_INTEGER, &[2, 3, 1, 2, 3], 0).unwrap().finish(0);
        let p = ptr_at(&out, 8, 0);
        assert_eq!(&out[p..p + 3], &[3, 2, 1]);
        let sig = der::encode_sequence(&[&der::encode_unsigned(&[1, 2]), &der::encode_unsigned(&[0x80])]);
        let out = decode(X509_ECC_SIGNATURE, &sig, 0).unwrap().finish(0);
        let r = ptr_at(&out, 8, 0);
        assert_eq!((&out[r..r + 2], u32_at(&out, 16)), (&[2u8, 1][..], 1));
        let der = der_of(LEAF);
        let a = decode(X509_CERT, &der, 0).unwrap().finish(0);
        let b = decode(X509_CERT, &der, CRYPT_DECODE_NO_SIGNATURE_BYTE_REVERSAL_FLAG).unwrap().finish(0);
        let (pa, pb, n) = (ptr_at(&a, 48, 0), ptr_at(&b, 48, 0), u32_at(&a, 40) as usize);
        assert_eq!(a[pa..pa + n].iter().rev().copied().collect::<Vec<_>>(), b[pb..pb + n]);
    }
}
