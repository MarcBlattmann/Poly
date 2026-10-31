//! Certificate stores: where Poly keeps the system stores, how
//! `CertFindCertificateInStore` matches, what `CertAdd*ToStore`'s
//! dispositions do, and `CertVerifyCertificateChainPolicy`'s verdicts.
//!
//! Sources: Learn's pages of `CertOpenStore` (providers, location flags),
//! `CertOpenSystemStoreW`, `CertFindCertificateInStore` (find types),
//! `CertAddEncodedCertificateToStore` (dispositions),
//! `CertVerifyCertificateChainPolicy` and `CERT_CHAIN_POLICY_PARA`,
//! `HTTPSPolicyCallbackData`, "System Store Locations"; constants from
//! `wincrypt.h` and `wininet.h` (`SECURITY_FLAG_*`).

use crate::status::*;
use alloc::string::String;
use alloc::vec::Vec;
use poly_crypto::HashAlg;
use poly_x509::Certificate;
use poly_x509::chain::{self, error as trust};

// `CertOpenStore` providers (`wincrypt.h`; small integers or strings).
pub const CERT_STORE_PROV_MEMORY: usize = 2;
pub const CERT_STORE_PROV_SYSTEM_A: usize = 9;
pub const CERT_STORE_PROV_SYSTEM_W: usize = 10;
pub const CERT_STORE_PROV_COLLECTION: usize = 11;
pub const CERT_STORE_PROV_SYSTEM_REGISTRY_W: usize = 13;

/// The provider a `CertOpenStore` provider argument names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Memory,
    /// `pvPara` is a wide store name (SYSTEM_W, SYSTEM_REGISTRY_W).
    SystemW,
    /// `pvPara` is an ANSI store name.
    SystemA,
    Collection,
}

/// The provider for a numbered provider or a provider string (Learn:
/// `sz_CERT_STORE_PROV_MEMORY` "Memory", "System", "Collection").
pub fn provider(number: Option<usize>, name: Option<&str>) -> Option<Provider> {
    match (number, name) {
        (Some(CERT_STORE_PROV_MEMORY), _) => Some(Provider::Memory),
        (Some(CERT_STORE_PROV_SYSTEM_W | CERT_STORE_PROV_SYSTEM_REGISTRY_W), _) => Some(Provider::SystemW),
        (Some(CERT_STORE_PROV_SYSTEM_A), _) => Some(Provider::SystemA),
        (Some(CERT_STORE_PROV_COLLECTION), _) => Some(Provider::Collection),
        (None, Some(n)) if n.eq_ignore_ascii_case("Memory") => Some(Provider::Memory),
        (None, Some(n)) if n.eq_ignore_ascii_case("System") || n.eq_ignore_ascii_case("SystemRegistry") => {
            Some(Provider::SystemW)
        }
        (None, Some(n)) if n.eq_ignore_ascii_case("Collection") => Some(Provider::Collection),
        _ => None,
    }
}

// `CertOpenStore` flags.
pub const CERT_STORE_NO_CRYPT_RELEASE_FLAG: u32 = 0x1;
pub const CERT_STORE_DELETE_FLAG: u32 = 0x10;
pub const CERT_STORE_CREATE_NEW_FLAG: u32 = 0x2000;
pub const CERT_STORE_OPEN_EXISTING_FLAG: u32 = 0x4000;
pub const CERT_STORE_READONLY_FLAG: u32 = 0x8000;
pub const CERT_SYSTEM_STORE_LOCATION_MASK: u32 = 0x00FF_0000;
pub const CERT_SYSTEM_STORE_CURRENT_USER: u32 = 1 << 16;
pub const CERT_SYSTEM_STORE_LOCAL_MACHINE: u32 = 2 << 16;

/// A system store location Poly has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    CurrentUser,
    LocalMachine,
}

/// The location in `CertOpenStore`'s flags (none given: the current user,
/// as `CertOpenSystemStore` uses). Service and group-policy locations map
/// to their machine or user counterpart.
pub fn location(flags: u32) -> Location {
    match (flags & CERT_SYSTEM_STORE_LOCATION_MASK) >> 16 {
        // LOCAL_MACHINE, CURRENT_SERVICE, SERVICES, LOCAL_MACHINE_GROUP_POLICY, LOCAL_MACHINE_ENTERPRISE.
        2 | 4 | 5 | 8 | 9 => Location::LocalMachine,
        _ => Location::CurrentUser,
    }
}

/// The directory of a system store, relative to the location's root:
/// `<root>\Poly\SystemCertificates\<NAME>` where the root is
/// `%ProgramData%` for the machine and `%APPDATA%` for the user (ADR
/// 0096: files, one DER certificate per `<SHA-1>.cer`, not the registry
/// format Windows uses). Store names are case-insensitive (upper-cased);
/// names with path characters are refused.
pub fn store_subdir(name: &str) -> Option<String> {
    if name.is_empty() || name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) || name.contains("..") {
        return None;
    }
    Some(alloc::format!("Poly\\SystemCertificates\\{}", name.to_ascii_uppercase()))
}

/// Whether a current-user store also shows the machine store of the same
/// name (Learn, "System Store Locations": the user's ROOT and CA include
/// the machine's; Poly does this for every name, read-only).
pub fn user_store_includes_machine(_name: &str) -> bool {
    true
}

/// The file name of a certificate in a store directory.
pub fn cert_file_name(c: &Certificate) -> String {
    let mut s: String = poly_x509::text::hex_encode(&c.thumbprint(HashAlg::Sha1)).to_ascii_uppercase();
    s.push_str(".cer");
    s
}

/// The certificate in a store file: DER, or PEM (so a `.crt`/`.pem` file
/// dropped into the directory counts too).
pub fn parse_store_file(bytes: &[u8]) -> Option<Certificate> {
    Certificate::parse(bytes)
        .ok()
        .or_else(|| poly_x509::text::pem_decode(bytes, Some("CERTIFICATE")).and_then(|d| Certificate::parse(&d).ok()))
}

// `CertAdd*ToStore` dispositions.
pub const CERT_STORE_ADD_NEW: u32 = 1;
pub const CERT_STORE_ADD_USE_EXISTING: u32 = 2;
pub const CERT_STORE_ADD_REPLACE_EXISTING: u32 = 3;
pub const CERT_STORE_ADD_ALWAYS: u32 = 4;
pub const CERT_STORE_ADD_REPLACE_EXISTING_INHERIT_PROPERTIES: u32 = 5;
pub const CERT_STORE_ADD_NEWER: u32 = 6;
pub const CERT_STORE_ADD_NEWER_INHERIT_PROPERTIES: u32 = 7;

/// What an add does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddAction {
    /// Add a new entry.
    Add,
    /// Keep the existing entry and return it.
    UseExisting,
    /// Replace the existing entry (keeping its properties when inherited).
    Replace { inherit: bool },
}

/// The action for a disposition, given whether the store already has the
/// same certificate (the same encoding; and for the NEWER dispositions
/// whether the new one's `NotBefore` is later). Learn: ADD_NEW fails
/// with `CRYPT_E_EXISTS`; NEWER fails with `CRYPT_E_EXISTS` unless newer.
pub fn add_action(disposition: u32, exists: bool, newer: bool) -> Result<AddAction, u32> {
    Ok(match (disposition, exists) {
        (CERT_STORE_ADD_NEW..=CERT_STORE_ADD_NEWER_INHERIT_PROPERTIES, false) => AddAction::Add,
        (CERT_STORE_ADD_ALWAYS, true) => AddAction::Add,
        (CERT_STORE_ADD_NEW, true) => return Err(CRYPT_E_EXISTS),
        (CERT_STORE_ADD_USE_EXISTING, true) => AddAction::UseExisting,
        (CERT_STORE_ADD_REPLACE_EXISTING, true) => AddAction::Replace { inherit: false },
        (CERT_STORE_ADD_REPLACE_EXISTING_INHERIT_PROPERTIES, true) => AddAction::Replace { inherit: true },
        (CERT_STORE_ADD_NEWER | CERT_STORE_ADD_NEWER_INHERIT_PROPERTIES, true) if newer => {
            AddAction::Replace { inherit: disposition == CERT_STORE_ADD_NEWER_INHERIT_PROPERTIES }
        }
        (CERT_STORE_ADD_NEWER | CERT_STORE_ADD_NEWER_INHERIT_PROPERTIES, true) => return Err(CRYPT_E_EXISTS),
        _ => return Err(E_INVALIDARG),
    })
}

// `CertFindCertificateInStore` find types (`CERT_COMPARE_* << 16 | CERT_INFO_*_FLAG`).
pub const CERT_FIND_ANY: u32 = 0;
pub const CERT_FIND_SHA1_HASH: u32 = 1 << 16;
pub const CERT_FIND_ISSUER_NAME: u32 = (2 << 16) | 4;
pub const CERT_FIND_SUBJECT_NAME: u32 = (2 << 16) | 7;
pub const CERT_FIND_MD5_HASH: u32 = 4 << 16;
pub const CERT_FIND_PUBLIC_KEY: u32 = 6 << 16;
pub const CERT_FIND_ISSUER_STR_A: u32 = (7 << 16) | 4;
pub const CERT_FIND_SUBJECT_STR_A: u32 = (7 << 16) | 7;
pub const CERT_FIND_ISSUER_STR_W: u32 = (8 << 16) | 4;
pub const CERT_FIND_SUBJECT_STR_W: u32 = (8 << 16) | 7;
pub const CERT_FIND_ENHKEY_USAGE: u32 = 10 << 16;
pub const CERT_FIND_SUBJECT_CERT: u32 = 11 << 16;
pub const CERT_FIND_ISSUER_OF: u32 = 12 << 16;
pub const CERT_FIND_EXISTING: u32 = 13 << 16;
pub const CERT_FIND_SIGNATURE_HASH: u32 = 14 << 16;
pub const CERT_FIND_KEY_IDENTIFIER: u32 = 15 << 16;
pub const CERT_FIND_SHA256_HASH: u32 = 21 << 16;

/// `pvFindPara`, read by the caller per find type.
#[derive(Clone, Debug)]
pub enum FindPara {
    Any,
    /// SHA-1, MD5 or SHA-256 thumbprint (by find type).
    Hash(HashAlg, Vec<u8>),
    /// An encoded name (`CERT_NAME_BLOB`), issuer or subject.
    Name {
        issuer: bool,
        encoded: Vec<u8>,
    },
    /// A substring of the name (case-insensitive), issuer or subject.
    Str {
        issuer: bool,
        text: String,
    },
    /// A `CERT_PUBLIC_KEY_INFO`: algorithm and key bits.
    PublicKey {
        oid: String,
        key: Vec<u8>,
    },
    /// The certificate itself (`CERT_FIND_EXISTING`), by encoding.
    Existing(Vec<u8>),
    /// Issuer name and serial of a `CERT_INFO` (`CERT_FIND_SUBJECT_CERT`).
    SubjectCert {
        issuer: Vec<u8>,
        serial_le: Vec<u8>,
    },
    /// Certificates that may have issued this one (`CERT_FIND_ISSUER_OF`).
    IssuerOf(Vec<u8>),
    /// A subject key identifier (`CERT_FIND_KEY_IDENTIFIER`).
    KeyId(Vec<u8>),
    /// Enhanced key usages that must all be present (none: any).
    Usage(Vec<String>),
}

/// Whether `c` matches a find.
pub fn find_matches(c: &Certificate, p: &FindPara) -> bool {
    match p {
        FindPara::Any => true,
        FindPara::Hash(h, want) => c.thumbprint(*h) == *want,
        FindPara::Name { issuer, encoded } => {
            let n = if *issuer { &c.issuer } else { &c.subject };
            n.encoded == *encoded
        }
        FindPara::Str { issuer, text } => {
            let n = if *issuer { &c.issuer } else { &c.subject };
            let hay = n.to_str(&poly_x509::name::NameStrOptions::new(poly_x509::name::NameForm::Simple));
            hay.to_lowercase().contains(&text.to_lowercase())
        }
        FindPara::PublicKey { oid, key } => c.public_key_info.algorithm.oid == *oid && c.public_key_info.key == *key,
        FindPara::Existing(enc) => c.encoded == *enc,
        FindPara::SubjectCert { issuer, serial_le } => {
            let be: Vec<u8> = serial_le.iter().rev().copied().collect();
            c.issuer.encoded == *issuer && strip(&c.serial) == strip(&be)
        }
        FindPara::IssuerOf(enc) => Certificate::parse(enc).is_ok_and(|sub| {
            poly_x509::name::names_match(&c.subject, &sub.issuer)
                && match (sub.authority_key_id(), c.subject_key_id()) {
                    (Some(a), Some(s)) => a == s,
                    _ => true,
                }
        }),
        FindPara::KeyId(id) => {
            c.subject_key_id().is_some_and(|s| s == *id)
                || poly_crypto::digest(HashAlg::Sha1, &c.public_key_info.key) == *id
        }
        FindPara::Usage(u) => match c.extended_key_usage() {
            None => true,
            Some(e) => u.iter().all(|x| e.contains(x)),
        },
    }
}

fn strip(b: &[u8]) -> &[u8] {
    let n = b.iter().take_while(|&&x| x == 0).count();
    &b[n..]
}

// Chain policies (`wincrypt.h`: `CERT_CHAIN_POLICY_*`, small integers).
pub const CERT_CHAIN_POLICY_BASE: usize = 1;
pub const CERT_CHAIN_POLICY_AUTHENTICODE: usize = 2;
pub const CERT_CHAIN_POLICY_SSL: usize = 4;
pub const CERT_CHAIN_POLICY_BASIC_CONSTRAINTS: usize = 5;
pub const CERT_CHAIN_POLICY_NT_AUTH: usize = 6;
// `CERT_CHAIN_POLICY_PARA` flags.
pub const IGNORE_NOT_TIME_VALID_FLAG: u32 = 0x1;
pub const IGNORE_INVALID_BASIC_CONSTRAINTS_FLAG: u32 = 0x8;
pub const ALLOW_UNKNOWN_CA_FLAG: u32 = 0x10;
pub const IGNORE_WRONG_USAGE_FLAG: u32 = 0x20;
pub const IGNORE_INVALID_NAME_FLAG: u32 = 0x40;
pub const IGNORE_ALL_REV_UNKNOWN_FLAGS: u32 = 0xF00;
// `HTTPSPolicyCallbackData.fdwChecks` (`wininet.h`).
pub const SECURITY_FLAG_IGNORE_UNKNOWN_CA: u32 = 0x100;
pub const SECURITY_FLAG_IGNORE_WRONG_USAGE: u32 = 0x200;
pub const SECURITY_FLAG_IGNORE_CERT_CN_INVALID: u32 = 0x1000;
pub const SECURITY_FLAG_IGNORE_CERT_DATE_INVALID: u32 = 0x2000;

/// A policy's verdict: `dwError`, `lChainIndex`, `lElementIndex` (-1
/// where none applies; to be confirmed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyStatus {
    pub error: u32,
    pub chain_index: i32,
    pub element_index: i32,
}

/// The SSL policy's extra parameters.
#[derive(Clone, Debug)]
pub struct SslPara {
    pub checks: u32,
    pub server_name: Option<String>,
}

/// `CertVerifyCertificateChainPolicy` over one simple chain: its
/// elements' error bits (end certificate first) and the end certificate.
/// The checks run in this order, the first failing one deciding (to be
/// confirmed): signature, untrusted root, partial chain, revocation
/// (Poly never revokes), usage, time, basic constraints, then the SSL
/// server name. Unknown policies are refused by
/// the caller.
pub fn verify_policy(
    policy: usize,
    elements: &[u32],
    end: &Certificate,
    flags: u32,
    ssl: Option<&SslPara>,
) -> PolicyStatus {
    let ok = PolicyStatus { error: 0, chain_index: -1, element_index: -1 };
    let at = |bit: u32| elements.iter().position(|e| e & bit != 0).map(|i| i as i32);
    let fail = |error: u32, bit: u32| at(bit).map(|i| PolicyStatus { error, chain_index: 0, element_index: i });
    if policy == CERT_CHAIN_POLICY_BASIC_CONSTRAINTS {
        return fail(TRUST_E_BASIC_CONSTRAINTS, trust::INVALID_BASIC_CONSTRAINTS).unwrap_or(ok);
    }
    let checks = ssl.map_or(0, |s| s.checks);
    let mut steps: Vec<(u32, u32)> = Vec::from([(TRUST_E_CERT_SIGNATURE, trust::IS_NOT_SIGNATURE_VALID)]);
    if flags & ALLOW_UNKNOWN_CA_FLAG == 0 && checks & SECURITY_FLAG_IGNORE_UNKNOWN_CA == 0 {
        steps.push((CERT_E_UNTRUSTEDROOT, trust::IS_UNTRUSTED_ROOT));
    }
    steps.push((CERT_E_CHAINING, trust::IS_PARTIAL_CHAIN));
    steps.push((CERT_E_CHAINING, trust::IS_CYCLIC));
    if flags & IGNORE_WRONG_USAGE_FLAG == 0 && checks & SECURITY_FLAG_IGNORE_WRONG_USAGE == 0 {
        steps.push((CERT_E_WRONG_USAGE, trust::IS_NOT_VALID_FOR_USAGE));
    }
    if flags & IGNORE_NOT_TIME_VALID_FLAG == 0 && checks & SECURITY_FLAG_IGNORE_CERT_DATE_INVALID == 0 {
        steps.push((CERT_E_EXPIRED, trust::IS_NOT_TIME_VALID));
    }
    if flags & IGNORE_INVALID_BASIC_CONSTRAINTS_FLAG == 0 {
        steps.push((TRUST_E_BASIC_CONSTRAINTS, trust::INVALID_BASIC_CONSTRAINTS));
    }
    if flags & IGNORE_ALL_REV_UNKNOWN_FLAGS != IGNORE_ALL_REV_UNKNOWN_FLAGS {
        steps.push((CRYPT_E_NO_REVOCATION_CHECK, trust::REVOCATION_STATUS_UNKNOWN));
    }
    for (error, bit) in steps {
        if let Some(s) = fail(error, bit) {
            return s;
        }
    }
    if policy == CERT_CHAIN_POLICY_SSL
        && let Some(SslPara { checks, server_name: Some(name) }) = ssl
        && checks & SECURITY_FLAG_IGNORE_CERT_CN_INVALID == 0
        && flags & IGNORE_INVALID_NAME_FLAG == 0
        && !chain::certificate_matches_host(end, name)
    {
        return PolicyStatus { error: CERT_E_CN_NO_MATCH, chain_index: 0, element_index: 0 };
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEAF: &str = include_str!("../../poly-x509/src/testdata/leaf.pem");
    const ROOT: &str = include_str!("../../poly-x509/src/testdata/root.pem");
    const INT: &str = include_str!("../../poly-x509/src/testdata/int.pem");

    fn cert(pem: &str) -> Certificate {
        Certificate::parse(&poly_x509::text::pem_decode(pem.as_bytes(), None).unwrap()).unwrap()
    }

    #[test]
    fn providers_locations_files() {
        assert_eq!(provider(Some(2), None), Some(Provider::Memory));
        assert_eq!(provider(None, Some("system")), Some(Provider::SystemW));
        assert_eq!(provider(Some(5), None), None);
        assert_eq!(location(CERT_SYSTEM_STORE_LOCAL_MACHINE | CERT_STORE_READONLY_FLAG), Location::LocalMachine);
        assert_eq!(location(0), Location::CurrentUser);
        assert_eq!(store_subdir("root").as_deref(), Some("Poly\\SystemCertificates\\ROOT"));
        assert_eq!(store_subdir("..\\x"), None);
        let c = cert(ROOT);
        assert_eq!(cert_file_name(&c).len(), 44);
        assert!(parse_store_file(ROOT.as_bytes()).is_some());
        assert!(parse_store_file(&c.encoded).is_some());
        assert!(parse_store_file(b"junk").is_none());
    }

    #[test]
    fn dispositions() {
        assert_eq!(add_action(CERT_STORE_ADD_NEW, false, false), Ok(AddAction::Add));
        assert_eq!(add_action(CERT_STORE_ADD_NEW, true, false), Err(CRYPT_E_EXISTS));
        assert_eq!(add_action(CERT_STORE_ADD_USE_EXISTING, true, false), Ok(AddAction::UseExisting));
        assert_eq!(add_action(CERT_STORE_ADD_REPLACE_EXISTING, true, false), Ok(AddAction::Replace { inherit: false }));
        assert_eq!(add_action(CERT_STORE_ADD_ALWAYS, true, false), Ok(AddAction::Add));
        assert_eq!(add_action(CERT_STORE_ADD_NEWER, true, false), Err(CRYPT_E_EXISTS));
        assert_eq!(
            add_action(CERT_STORE_ADD_NEWER_INHERIT_PROPERTIES, true, true),
            Ok(AddAction::Replace { inherit: true })
        );
        assert_eq!(add_action(9, false, false), Err(E_INVALIDARG));
    }

    #[test]
    fn finds() {
        let (leaf, int, root) = (cert(LEAF), cert(INT), cert(ROOT));
        assert!(find_matches(&leaf, &FindPara::Str { issuer: false, text: "EXAMPLE.test".into() }));
        assert!(!find_matches(&leaf, &FindPara::Str { issuer: true, text: "example".into() }));
        assert!(find_matches(&leaf, &FindPara::Hash(HashAlg::Sha1, leaf.thumbprint(HashAlg::Sha1))));
        assert!(find_matches(&int, &FindPara::Name { issuer: true, encoded: root.subject.encoded.clone() }));
        assert!(find_matches(&int, &FindPara::IssuerOf(leaf.encoded.clone())));
        assert!(!find_matches(&root, &FindPara::IssuerOf(leaf.encoded.clone())));
        let serial_le: Vec<u8> = leaf.serial.iter().rev().copied().collect();
        assert!(find_matches(&leaf, &FindPara::SubjectCert { issuer: leaf.issuer.encoded.clone(), serial_le }));
        assert!(find_matches(&int, &FindPara::KeyId(int.subject_key_id().unwrap())));
        assert!(find_matches(&leaf, &FindPara::Usage(Vec::from([String::from(poly_x509::oid::KP_SERVER_AUTH)]))));
        assert!(find_matches(&root, &FindPara::Usage(Vec::from([String::from(poly_x509::oid::KP_CODE_SIGNING)]))));
    }

    #[test]
    fn policies() {
        let leaf = cert(LEAF);
        let good = [0, 0, 0];
        assert_eq!(verify_policy(CERT_CHAIN_POLICY_BASE, &good, &leaf, 0, None).error, 0);
        let untrusted = [0, 0, trust::IS_UNTRUSTED_ROOT];
        let s = verify_policy(CERT_CHAIN_POLICY_BASE, &untrusted, &leaf, 0, None);
        assert_eq!(s, PolicyStatus { error: CERT_E_UNTRUSTEDROOT, chain_index: 0, element_index: 2 });
        assert_eq!(verify_policy(CERT_CHAIN_POLICY_BASE, &untrusted, &leaf, ALLOW_UNKNOWN_CA_FLAG, None).error, 0);
        let expired = [trust::IS_NOT_TIME_VALID, 0, trust::IS_UNTRUSTED_ROOT];
        assert_eq!(
            verify_policy(CERT_CHAIN_POLICY_BASE, &expired, &leaf, ALLOW_UNKNOWN_CA_FLAG, None).error,
            CERT_E_EXPIRED
        );
        let ssl = SslPara { checks: 0, server_name: Some("www.example.test".into()) };
        assert_eq!(verify_policy(CERT_CHAIN_POLICY_SSL, &good, &leaf, 0, Some(&ssl)).error, 0);
        let wrong = SslPara { checks: 0, server_name: Some("evil.test".into()) };
        assert_eq!(verify_policy(CERT_CHAIN_POLICY_SSL, &good, &leaf, 0, Some(&wrong)).error, CERT_E_CN_NO_MATCH);
        let ignored = SslPara { checks: SECURITY_FLAG_IGNORE_CERT_CN_INVALID, ..wrong };
        assert_eq!(verify_policy(CERT_CHAIN_POLICY_SSL, &good, &leaf, 0, Some(&ignored)).error, 0);
        let bc = [0, trust::INVALID_BASIC_CONSTRAINTS, 0];
        assert_eq!(
            verify_policy(CERT_CHAIN_POLICY_BASIC_CONSTRAINTS, &bc, &leaf, 0, None).error,
            TRUST_E_BASIC_CONSTRAINTS
        );
    }
}
