//! Certificate path building and validation: a basic form of RFC 5280
//! section 6 that is enough for a trust decision against a set of trusted
//! roots.
//!
//! From the end certificate, each next certificate is an issuer from the
//! candidate pool whose subject matches the current issuer (RFC 5280
//! 4.1.2.4) and, when both carry them, whose subjectKeyIdentifier equals
//! the authorityKeyIdentifier (4.2.1.1); among several, one whose key
//! verifies the signature wins, then a trusted one. Building stops at a
//! self-signed certificate, at a trusted one, or when nothing issues the
//! current one (a partial chain). Every element's problems are recorded,
//! not just the first: signature, validity period, basic constraints and
//! path length (4.2.1.9) of the issuers, and the requested extended key
//! usages (4.2.1.12). Revocation is not checked (Poly has no network
//! fetching of CRLs or OCSP yet); name constraints and policies are not
//! evaluated.
//!
//! The status bits are CryptoAPI's `CERT_TRUST_*` values (`wincrypt.h`),
//! so `crypt32` hands them out unchanged.

use crate::cert::Certificate;
use crate::name;
use crate::verify;
use alloc::string::String;
use alloc::vec::Vec;

/// `dwErrorStatus` bits (`wincrypt.h`, `CERT_TRUST_*`).
pub mod error {
    pub const IS_NOT_TIME_VALID: u32 = 0x0000_0001;
    pub const IS_NOT_SIGNATURE_VALID: u32 = 0x0000_0008;
    pub const IS_NOT_VALID_FOR_USAGE: u32 = 0x0000_0010;
    pub const IS_UNTRUSTED_ROOT: u32 = 0x0000_0020;
    pub const REVOCATION_STATUS_UNKNOWN: u32 = 0x0000_0040;
    pub const IS_CYCLIC: u32 = 0x0000_0080;
    pub const INVALID_EXTENSION: u32 = 0x0000_0100;
    pub const INVALID_BASIC_CONSTRAINTS: u32 = 0x0000_0400;
    pub const IS_PARTIAL_CHAIN: u32 = 0x0001_0000;
    pub const IS_OFFLINE_REVOCATION: u32 = 0x0100_0000;
}

/// `dwInfoStatus` bits (`wincrypt.h`, `CERT_TRUST_*`).
pub mod info {
    pub const HAS_EXACT_MATCH_ISSUER: u32 = 0x0000_0001;
    pub const HAS_KEY_MATCH_ISSUER: u32 = 0x0000_0002;
    pub const HAS_NAME_MATCH_ISSUER: u32 = 0x0000_0004;
    pub const IS_SELF_SIGNED: u32 = 0x0000_0008;
    pub const HAS_PREFERRED_ISSUER: u32 = 0x0000_0100;
}

/// The longest chain built (a guard against loops through cross-signed
/// pools).
pub const MAX_DEPTH: usize = 16;

/// What to build a chain from.
#[derive(Debug)]
pub struct Request<'a> {
    /// The certificate to build a chain for.
    pub end: &'a Certificate,
    /// Candidate issuers (intermediates and roots, in preference order).
    pub pool: &'a [&'a Certificate],
    /// Whether `pool[i]` is a trusted root.
    pub trusted: &'a [bool],
    /// The time to check validity at (FILETIME).
    pub time: u64,
    /// Extended key usages every certificate must allow (absent extension
    /// = any usage).
    pub usages: &'a [String],
}

/// One element of a built chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Element {
    /// `None` for the end certificate, else its index in the pool.
    pub pool_index: Option<usize>,
    pub error: u32,
    pub info: u32,
}

/// A built chain, end certificate first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chain {
    pub elements: Vec<Element>,
    /// The OR of the elements' errors.
    pub error: u32,
    pub info: u32,
}

fn signs(issuer: &Certificate, subject: &Certificate) -> bool {
    issuer.public_key().is_some_and(|k| verify::verify_certificate(subject, &k).is_ok())
}

fn self_signed(c: &Certificate) -> bool {
    c.is_self_issued() && signs(c, c)
}

/// Build and check the chain for `req.end`.
pub fn build(req: &Request<'_>) -> Chain {
    let cert_at = |e: &Element| -> &Certificate { e.pool_index.map_or(req.end, |i| req.pool[i]) };
    let trusted = |i: Option<usize>| i.is_some_and(|i| req.trusted.get(i).copied().unwrap_or(false));
    let mut elements = Vec::from([Element { pool_index: None, error: 0, info: 0 }]);
    // Whether the last element ends the chain at a root.
    let mut complete = false;
    loop {
        let last = *elements.last().unwrap_or(&elements[0]);
        let cur = cert_at(&last);
        if self_signed(cur) {
            if let Some(e) = elements.last_mut() {
                e.info |= info::IS_SELF_SIGNED;
            }
            complete = true;
            break;
        }
        if trusted(last.pool_index) {
            complete = true;
            break;
        }
        if elements.len() >= MAX_DEPTH {
            break;
        }
        let aki = cur.authority_key_id();
        let mut best: Option<(usize, u32, u32)> = None; // (index, score, info)
        for (i, cand) in req.pool.iter().enumerate() {
            if !name::names_match(&cand.subject, &cur.issuer) {
                continue;
            }
            let ski = cand.subject_key_id();
            let mut inf = info::HAS_NAME_MATCH_ISSUER;
            if let (Some(a), Some(s)) = (&aki, &ski) {
                if a != s {
                    continue;
                }
                inf = info::HAS_KEY_MATCH_ISSUER;
            }
            let in_chain = elements.iter().any(|e| e.pool_index == Some(i))
                || elements.iter().any(|e| cert_at(e).encoded == cand.encoded);
            let score = u32::from(signs(cand, cur)) * 8
                + u32::from(trusted(Some(i))) * 4
                + u32::from(cand.is_time_valid(req.time)) * 2
                + u32::from(!in_chain);
            if best.is_none_or(|(_, s, _)| score > s) {
                best = Some((i, score, inf));
            }
        }
        let Some((i, score, inf)) = best else { break };
        let n = elements.len();
        if score & 1 == 0 {
            // Every issuer found is already in the chain.
            elements[n - 1].error |= error::IS_CYCLIC;
            break;
        }
        if score & 8 == 0 {
            elements[n - 1].error |= error::IS_NOT_SIGNATURE_VALID;
        }
        elements[n - 1].info |= inf;
        elements.push(Element { pool_index: Some(i), error: 0, info: 0 });
    }

    let n = elements.len();
    let root_trusted = complete && trusted(elements[n - 1].pool_index);
    if !complete {
        elements[n - 1].error |= error::IS_PARTIAL_CHAIN;
    } else if !root_trusted {
        elements[n - 1].error |= error::IS_UNTRUSTED_ROOT;
    }
    // Per-element checks.
    let mut ca_below = 0u32;
    for (k, e) in elements.iter_mut().enumerate() {
        let c = e.pool_index.map_or(req.end, |i| req.pool[i]);
        if !c.is_time_valid(req.time) {
            e.error |= error::IS_NOT_TIME_VALID;
        }
        if k > 0 {
            // An issuer: a CA if it says what it is (v3 with the extension);
            // a v3 issuer without the extension is accepted only as the root.
            match c.basic_constraints() {
                Some(bc) => {
                    if !bc.ca || bc.path_len.is_some_and(|p| ca_below > p) {
                        e.error |= error::INVALID_BASIC_CONSTRAINTS;
                    }
                }
                None if c.version >= 2 && k + 1 < n => e.error |= error::INVALID_BASIC_CONSTRAINTS,
                None => {}
            }
            if !c.is_self_issued() {
                ca_below += 1;
            }
        }
        if c.extension(crate::oid::BASIC_CONSTRAINTS).is_some() && c.basic_constraints().is_none() {
            e.error |= error::INVALID_EXTENSION;
        }
        if !req.usages.is_empty()
            && let Some(ekus) = c.extended_key_usage()
            && !ekus.iter().any(|u| u == crate::oid::ANY_EXTENDED_KEY_USAGE)
            && !req.usages.iter().all(|u| ekus.contains(u))
        {
            e.error |= error::IS_NOT_VALID_FOR_USAGE;
        }
    }
    let error = elements.iter().fold(0, |a, e| a | e.error);
    let info = if root_trusted { info::HAS_PREFERRED_ISSUER } else { 0 };
    Chain { elements, error, info }
}

/// Whether `name` (a host name) matches a certificate's `pattern` (a
/// dNSName or common name): case-insensitive, with a `*` allowed as the
/// whole leftmost label matching exactly one label (RFC 6125 6.4.3).
pub fn host_matches(pattern: &str, name: &str) -> bool {
    let p = pattern.trim_end_matches('.');
    let n = name.trim_end_matches('.');
    if let Some(rest) = p.strip_prefix("*.") {
        let Some((first, tail)) = n.split_once('.') else { return false };
        // Not across the public suffix boundary in the simple sense: the
        // wildcard needs at least two labels after it.
        !first.is_empty() && rest.contains('.') && tail.eq_ignore_ascii_case(rest)
    } else {
        p.eq_ignore_ascii_case(n)
    }
}

/// Whether the certificate is for host `name` (RFC 6125 6.4): its dNSName
/// SANs if it has any, else its subject's common names; an IP address
/// literal matches an iPAddress SAN.
pub fn certificate_matches_host(c: &Certificate, name: &str) -> bool {
    use crate::name::GeneralName;
    let sans = c.subject_alt_names();
    if let Some(ip) = parse_ipv4(name) {
        return sans.iter().any(|g| matches!(g, GeneralName::Ip(a) if a[..] == ip[..]));
    }
    let dns: Vec<&String> = sans
        .iter()
        .filter_map(|g| match g {
            GeneralName::Dns(d) => Some(d),
            _ => None,
        })
        .collect();
    if !dns.is_empty() {
        return dns.iter().any(|d| host_matches(d, name));
    }
    c.subject
        .attributes()
        .filter(|a| a.oid == crate::oid::COMMON_NAME)
        .filter_map(|a| a.text())
        .any(|cn| host_matches(&cn, name))
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for o in &mut out {
        *o = parts.next()?.parse().ok()?;
    }
    if parts.next().is_some() { None } else { Some(out) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cert::fixtures::*;
    use crate::time;

    fn cert(pem: &str) -> Certificate {
        Certificate::parse(&der(pem)).unwrap()
    }

    fn now() -> u64 {
        time::filetime(2026, 10, 30, 12, 0, 0).unwrap()
    }

    #[test]
    fn a_good_chain_to_a_trusted_root() {
        let (root, int, leaf) = (cert(ROOT), cert(INTERMEDIATE), cert(LEAF));
        let pool = [&int, &root];
        let req = Request { end: &leaf, pool: &pool, trusted: &[false, true], time: now(), usages: &[] };
        let c = build(&req);
        assert_eq!(c.error, 0, "{c:?}");
        assert_eq!(c.elements.iter().map(|e| e.pool_index).collect::<Vec<_>>(), [None, Some(0), Some(1)]);
        assert_eq!(c.elements[0].info, info::HAS_KEY_MATCH_ISSUER);
        assert_eq!(c.elements[2].info, info::IS_SELF_SIGNED);
        assert_eq!(c.info, info::HAS_PREFERRED_ISSUER);
        let usages = [String::from(crate::oid::KP_SERVER_AUTH)];
        assert_eq!(build(&Request { usages: &usages, ..req }).error, 0);
        let usages = [String::from(crate::oid::KP_CODE_SIGNING)];
        assert_eq!(build(&Request { usages: &usages, ..req }).error, error::IS_NOT_VALID_FOR_USAGE);
    }

    #[test]
    fn untrusted_partial_expired() {
        let (root, int, leaf, old) = (cert(ROOT), cert(INTERMEDIATE), cert(LEAF), cert(EXPIRED));
        let pool = [&int, &root];
        let c = build(&Request { end: &leaf, pool: &pool, trusted: &[false, false], time: now(), usages: &[] });
        assert_eq!(c.error, error::IS_UNTRUSTED_ROOT);
        assert_eq!(c.elements[2].error, error::IS_UNTRUSTED_ROOT);
        let pool = [&int];
        let c = build(&Request { end: &leaf, pool: &pool, trusted: &[false], time: now(), usages: &[] });
        assert_eq!(c.error, error::IS_PARTIAL_CHAIN);
        assert_eq!(c.elements.len(), 2);
        let pool = [&root];
        let c = build(&Request { end: &old, pool: &pool, trusted: &[true], time: now(), usages: &[] });
        assert_eq!(c.error, error::IS_NOT_TIME_VALID);
        assert_eq!(c.elements[0].error, error::IS_NOT_TIME_VALID);
    }

    #[test]
    fn a_bad_signature_and_a_trusted_self_signed_end() {
        let (root, int, leaf) = (cert(ROOT), cert(INTERMEDIATE), cert(LEAF));
        let mut forged = leaf.clone();
        forged.tbs[20] ^= 1;
        let pool = [&int, &root];
        let c = build(&Request { end: &forged, pool: &pool, trusted: &[false, true], time: now(), usages: &[] });
        assert_eq!(c.error, error::IS_NOT_SIGNATURE_VALID);
        assert_eq!(c.elements[0].error, error::IS_NOT_SIGNATURE_VALID);
        let ss = cert(SELF_SIGNED);
        let c = build(&Request { end: &ss, pool: &[], trusted: &[], time: now(), usages: &[] });
        assert_eq!((c.error, c.elements[0].info), (error::IS_UNTRUSTED_ROOT, info::IS_SELF_SIGNED));
    }

    #[test]
    fn path_length_is_enforced() {
        let (root, int, sub, deep) = (cert(ROOT), cert(INTERMEDIATE), cert(TOO_DEEP_CA), cert(TOO_DEEP_LEAF));
        let pool = [&sub, &int, &root];
        let c = build(&Request { end: &deep, pool: &pool, trusted: &[false, false, true], time: now(), usages: &[] });
        assert_eq!(c.elements.len(), 4);
        assert_eq!(c.error, error::INVALID_BASIC_CONSTRAINTS);
        assert_eq!(c.elements[2].error, error::INVALID_BASIC_CONSTRAINTS);
        // The intermediate itself, under the root, is fine.
        let pool = [&root];
        let c = build(&Request { end: &int, pool: &pool, trusted: &[true], time: now(), usages: &[] });
        assert_eq!(c.error, 0);
    }

    #[test]
    fn host_names() {
        assert!(host_matches("www.example.test", "WWW.Example.Test"));
        assert!(host_matches("*.api.example.test", "a.api.example.test"));
        assert!(!host_matches("*.api.example.test", "a.b.api.example.test"));
        assert!(!host_matches("*.test", "example.test"));
        let leaf = cert(LEAF);
        assert!(certificate_matches_host(&leaf, "www.example.test"));
        assert!(certificate_matches_host(&leaf, "x.api.example.test"));
        assert!(certificate_matches_host(&leaf, "192.0.2.1"));
        assert!(!certificate_matches_host(&leaf, "example.test"));
        // The self-signed certificate has no SANs: its CN counts.
        let ss = cert(SELF_SIGNED);
        assert!(certificate_matches_host(&ss, "Self Signed, Inc."));
    }
}
