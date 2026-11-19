//! The publisher of a signed Windows program (ADR 0222: the consent prompt names it).
//!
//! The PE format specification ("The Attribute Certificate Table (Image Only)"): data directory
//! 4 of an image is the certificate table, whose "virtual address" is a FILE offset; it holds
//! `WIN_CERTIFICATE` entries (`dwLength`, `wRevision`, `wCertificateType`, then the bytes,
//! each entry padded to 8 bytes). `WIN_CERT_TYPE_PKCS_SIGNED_DATA` (2, `wintrust.h`) is a
//! PKCS #7 `SignedData` (RFC 2315, RFC 5652): `ContentInfo { contentType signedData,
//! content [0] EXPLICIT SignedData }`, `SignedData { version, digestAlgorithms, contentInfo,
//! certificates [0] IMPLICIT, crls [1] IMPLICIT, signerInfos SET }`, each `SignerInfo` naming
//! its certificate by `issuerAndSerialNumber`. The publisher is the subject of the first
//! signer's certificate: its common name, else its organisation.
//!
//! This reads the name only. It does not check the signature, the digest of the image or the
//! chain to a trusted root; the prompt says so ("not verified") until it does.

use crate::cert::Certificate;
use crate::der::{Reader, tag};
use crate::name::names_match;
use crate::oid;
use alloc::string::String;
use alloc::vec::Vec;

/// `signedData` (1.2.840.113549.1.7.2).
const SIGNED_DATA: &str = "1.2.840.113549.1.7.2";
/// `WIN_CERT_TYPE_PKCS_SIGNED_DATA` (`wintrust.h`).
const PKCS_SIGNED_DATA: u16 = 2;
/// `IMAGE_DIRECTORY_ENTRY_SECURITY` (`winnt.h`).
const SECURITY_DIRECTORY: usize = 4;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The certificate table of a PE image whose first bytes are `headers`: `(file offset, size)`,
/// `None` when the image has none (or the bytes are not a PE image).
pub fn certificate_table(headers: &[u8]) -> Option<(u64, u32)> {
    if headers.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(headers, 0x3C)? as usize;
    if headers.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let optional = pe + 24;
    let directories = match u16_at(headers, optional)? {
        0x10B => optional + 96,
        0x20B => optional + 112,
        _ => return None,
    };
    let count = u32_at(headers, optional + if u16_at(headers, optional)? == 0x10B { 92 } else { 108 })? as usize;
    if count <= SECURITY_DIRECTORY {
        return None;
    }
    let at = directories + 8 * SECURITY_DIRECTORY;
    let (offset, size) = (u32_at(headers, at)?, u32_at(headers, at + 4)?);
    (offset != 0 && size >= 8).then_some((u64::from(offset), size))
}

/// The publisher named by the first `PKCS_SIGNED_DATA` entry of a certificate table.
pub fn publisher(table: &[u8]) -> Option<String> {
    let mut at = 0usize;
    while at + 8 <= table.len() {
        let len = u32_at(table, at)? as usize;
        let kind = u16_at(table, at + 6)?;
        if len < 8 || at + len > table.len() {
            return None;
        }
        if kind == PKCS_SIGNED_DATA
            && let Some(name) = signer_name(&table[at + 8..at + len])
        {
            return Some(name);
        }
        at += (len + 7) & !7;
    }
    None
}

/// The subject name (common name, else organisation) of the certificate of the first signer of
/// a DER `ContentInfo` holding a `SignedData`.
pub fn signer_name(content_info: &[u8]) -> Option<String> {
    // The first element: a certificate table pads each entry to 8 bytes after it.
    let ci = Reader::new(content_info).read_tag(tag::SEQUENCE).ok()?;
    let mut ci = ci.reader();
    if oid::to_dotted(ci.oid().ok()?)? != SIGNED_DATA {
        return None;
    }
    let explicit = ci.read_tag(tag::context_cons(0)).ok()?;
    let mut sd = explicit.reader().sequence().ok()?;
    sd.integer_bytes().ok()?;
    sd.read_tag(tag::SET).ok()?;
    sd.read_tag(tag::SEQUENCE).ok()?;
    let mut certificates: Vec<Certificate> = Vec::new();
    if let Some(set) = sd.read_optional(tag::context_cons(0)).ok()? {
        let mut r = set.reader();
        while !r.is_empty() {
            let c = r.read().ok()?;
            if let Ok(cert) = Certificate::parse(c.raw) {
                certificates.push(cert);
            }
        }
    }
    sd.read_optional(tag::context_cons(1)).ok()?;
    let signers = sd.read_tag(tag::SET).ok()?;
    let mut signer = signers.reader().sequence().ok()?;
    signer.integer_bytes().ok()?;
    let mut id = signer.sequence().ok()?;
    let issuer = crate::name::Name::parse(id.read_tag(tag::SEQUENCE).ok()?.raw).ok()?;
    let serial = id.integer_bytes().ok()?;
    let cert = certificates.iter().find(|c| c.serial == serial && names_match(&c.issuer, &issuer))?;
    cert.subject.first(oid::COMMON_NAME).or_else(|| cert.subject.first(oid::ORGANIZATION_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::der::{encode, encode_sequence};
    use crate::text::pem_decode;

    const LEAF: &str = include_str!("testdata/leaf.pem");
    const ROOT: &str = include_str!("testdata/root.pem");

    /// A `ContentInfo` with `certs` and one signer naming `signer`.
    fn signed_data(certs: &[&Certificate], signer: &Certificate) -> Vec<u8> {
        let oid_signed = encode(tag::OID, &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x02]);
        let version = encode(tag::INTEGER, &[1]);
        let digests = encode(tag::SET, &[]);
        let inner = encode_sequence(&[&encode(tag::OID, &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x01])]);
        let all: Vec<u8> = certs.iter().flat_map(|c| c.encoded.clone()).collect();
        let certificates = encode(tag::context_cons(0), &all);
        let sid = encode_sequence(&[&signer.issuer.encoded, &encode(tag::INTEGER, &signer.serial)]);
        let info = encode_sequence(&[&version, &sid]);
        let signers = encode(tag::SET, &info);
        let sd = encode_sequence(&[&version, &digests, &inner, &certificates, &signers]);
        encode_sequence(&[&oid_signed, &encode(tag::context_cons(0), &sd)])
    }

    fn cert(pem: &str) -> Certificate {
        Certificate::parse(&pem_decode(pem.as_bytes(), Some("CERTIFICATE")).unwrap()).unwrap()
    }

    #[test]
    fn the_signer_s_certificate_names_the_publisher() {
        let (leaf, root) = (cert(LEAF), cert(ROOT));
        let ci = signed_data(&[&root, &leaf], &leaf);
        let want = leaf.subject.first(oid::COMMON_NAME).or_else(|| leaf.subject.first(oid::ORGANIZATION_NAME));
        assert!(want.is_some());
        assert_eq!(signer_name(&ci), want);
        // Padding after the element (the table pads each entry to 8 bytes) is no obstacle.
        let mut padded = ci.clone();
        padded.extend_from_slice(&[0; 5]);
        assert_eq!(signer_name(&padded), want);
        // In a certificate table: one WIN_CERTIFICATE entry, padded to 8.
        let mut table = Vec::new();
        table.extend_from_slice(&((8 + ci.len()) as u32).to_le_bytes());
        table.extend_from_slice(&0x200u16.to_le_bytes());
        table.extend_from_slice(&PKCS_SIGNED_DATA.to_le_bytes());
        table.extend_from_slice(&ci);
        while table.len() % 8 != 0 {
            table.push(0);
        }
        assert_eq!(publisher(&table), want);
        // A signer whose certificate is not there: no name.
        assert_eq!(signer_name(&signed_data(&[&root], &leaf)), None);
        assert_eq!(publisher(&[]), None);
        assert_eq!(signer_name(b"\x30\x00"), None);
    }

    #[test]
    fn the_table_is_found_in_the_headers() {
        // A PE32+ header with the security directory at 0x1200, 0x300 bytes.
        let mut h = alloc::vec![0u8; 0x400];
        h[..2].copy_from_slice(b"MZ");
        h[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        h[0x80..0x84].copy_from_slice(b"PE\0\0");
        let opt = 0x80 + 24;
        h[opt..opt + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
        h[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        let d = opt + 112 + 8 * 4;
        h[d..d + 4].copy_from_slice(&0x1200u32.to_le_bytes());
        h[d + 4..d + 8].copy_from_slice(&0x300u32.to_le_bytes());
        assert_eq!(certificate_table(&h), Some((0x1200, 0x300)));
        // PE32: the directories 16 bytes earlier.
        h[opt..opt + 2].copy_from_slice(&0x10Bu16.to_le_bytes());
        h[opt + 92..opt + 96].copy_from_slice(&16u32.to_le_bytes());
        let d = opt + 96 + 8 * 4;
        h[d..d + 4].copy_from_slice(&0x2000u32.to_le_bytes());
        h[d + 4..d + 8].copy_from_slice(&0x100u32.to_le_bytes());
        assert_eq!(certificate_table(&h), Some((0x2000, 0x100)));
        // Not signed, not a PE image.
        h[d..d + 8].fill(0);
        assert_eq!(certificate_table(&h), None);
        assert_eq!(certificate_table(b"MZ"), None);
    }
}


