//! SID strings (Learn: "SID Components", "SID Strings";
//! `ConvertSidToStringSidW`, `ConvertStringSidToSidW`).
//!
//! The standard form is `S-R-I-S-S...`: `S`, the revision, the identifier
//! authority, then each sub-authority, separated by hyphens. Learn's SID
//! components page writes the authority in decimal; an authority of 2^32
//! or more (whose top two bytes are not zero) is written as `0x` and twelve
//! hexadecimal digits (to be confirmed by the spec agent: case and width).
//!
//! `ConvertStringSidToSidW` also takes the two-letter SID string constants
//! of `sddl.h` ("BA" for built-in administrators). Poly knows those whose
//! SID does not depend on a domain controller ([`ALIASES`]), plus "LA" and
//! "LG", relative to the machine's account domain.
//!
//! Choices to be confirmed by the spec agent: the leading `S` and the hex
//! digits are accepted in either case; the aliases only in upper case; a
//! SID with no sub-authority (`S-1-5`) is accepted; a sub-authority above
//! `u32::MAX`, an empty component, a sign or a space is `ERROR_INVALID_SID`.

use crate::sid::{self, Sid, rid};
use alloc::string::String;
use alloc::vec::Vec;

/// The SID string constants Poly knows and their SIDs' (authority,
/// sub-authorities) (Learn: "SID Strings"; the RIDs from `winnt.h`).
pub const ALIASES: &[(&str, u64, &[u32])] = &[
    (
        "AC",
        sid::SECURITY_APP_PACKAGE_AUTHORITY,
        &[rid::SECURITY_APP_PACKAGE_BASE_RID, rid::SECURITY_BUILTIN_PACKAGE_ANY_PACKAGE],
    ),
    ("AN", 5, &[rid::SECURITY_ANONYMOUS_LOGON_RID]),
    ("AO", 5, &[32, rid::DOMAIN_ALIAS_RID_ACCOUNT_OPS]),
    ("AU", 5, &[rid::SECURITY_AUTHENTICATED_USER_RID]),
    ("BA", 5, &[32, rid::DOMAIN_ALIAS_RID_ADMINS]),
    ("BG", 5, &[32, rid::DOMAIN_ALIAS_RID_GUESTS]),
    ("BO", 5, &[32, rid::DOMAIN_ALIAS_RID_BACKUP_OPS]),
    ("BU", 5, &[32, rid::DOMAIN_ALIAS_RID_USERS]),
    ("CG", 3, &[rid::SECURITY_CREATOR_GROUP_RID]),
    ("CO", 3, &[rid::SECURITY_CREATOR_OWNER_RID]),
    ("ED", 5, &[rid::SECURITY_SERVER_LOGON_RID]),
    ("HI", 16, &[rid::SECURITY_MANDATORY_HIGH_RID]),
    ("IU", 5, &[rid::SECURITY_INTERACTIVE_RID]),
    ("LS", 5, &[rid::SECURITY_LOCAL_SERVICE_RID]),
    ("LU", 5, &[32, rid::DOMAIN_ALIAS_RID_LOGGING_USERS]),
    ("LW", 16, &[rid::SECURITY_MANDATORY_LOW_RID]),
    ("ME", 16, &[rid::SECURITY_MANDATORY_MEDIUM_RID]),
    ("MP", 16, &[rid::SECURITY_MANDATORY_MEDIUM_PLUS_RID]),
    ("MU", 5, &[32, rid::DOMAIN_ALIAS_RID_MONITORING_USERS]),
    ("NO", 5, &[32, rid::DOMAIN_ALIAS_RID_NETWORK_CONFIGURATION_OPS]),
    ("NS", 5, &[rid::SECURITY_NETWORK_SERVICE_RID]),
    ("NU", 5, &[rid::SECURITY_NETWORK_RID]),
    ("OW", 3, &[rid::SECURITY_CREATOR_OWNER_RIGHTS_RID]),
    ("PO", 5, &[32, rid::DOMAIN_ALIAS_RID_PRINT_OPS]),
    ("PS", 5, &[rid::SECURITY_PRINCIPAL_SELF_RID]),
    ("PU", 5, &[32, rid::DOMAIN_ALIAS_RID_POWER_USERS]),
    ("RC", 5, &[rid::SECURITY_RESTRICTED_CODE_RID]),
    ("RD", 5, &[32, rid::DOMAIN_ALIAS_RID_REMOTE_DESKTOP_USERS]),
    ("RE", 5, &[32, rid::DOMAIN_ALIAS_RID_REPLICATOR]),
    ("RU", 5, &[32, rid::DOMAIN_ALIAS_RID_PREW2KCOMPACCESS]),
    ("SI", 16, &[rid::SECURITY_MANDATORY_SYSTEM_RID]),
    ("SO", 5, &[32, rid::DOMAIN_ALIAS_RID_SYSTEM_OPS]),
    ("SU", 5, &[rid::SECURITY_SERVICE_RID]),
    ("SY", 5, &[rid::SECURITY_LOCAL_SYSTEM_RID]),
    ("WD", 1, &[rid::SECURITY_WORLD_RID]),
];

/// The standard string form of a SID.
pub fn to_string(s: &Sid) -> String {
    use core::fmt::Write;
    let mut out = String::from("S-1-");
    if s.authority >> 32 == 0 {
        let _ = write!(out, "{}", s.authority);
    } else {
        let _ = write!(out, "0x{:012X}", s.authority);
    }
    for sub in &s.subs {
        let _ = write!(out, "-{sub}");
    }
    out
}

/// The string form of the SID at the start of `bytes`, as UTF-16 (for
/// `ConvertSidToStringSidW`); `None` if it is not a valid SID.
pub fn to_wide(bytes: &[u8]) -> Option<Vec<u16>> {
    Sid::from_bytes(bytes).map(|s| to_string(&s).encode_utf16().collect())
}

fn number(digits: &[u16], hex: bool, max: u64) -> Option<u64> {
    if digits.is_empty() || (hex && digits.len() > 12) {
        return None;
    }
    let radix = if hex { 16 } else { 10 };
    let mut v: u64 = 0;
    for &c in digits {
        let d = char::from_u32(u32::from(c))?.to_digit(radix)?;
        v = v.checked_mul(u64::from(radix))?.checked_add(u64::from(d))?;
        if v > max {
            return None;
        }
    }
    Some(v)
}

/// Parse a SID string (`ConvertStringSidToSidW`): the standard form or a
/// SID string constant. `None` is `ERROR_INVALID_SID`.
pub fn parse(s: &[u16]) -> Option<Sid> {
    if s.len() == 2 {
        let name = [s[0], s[1]];
        return ALIASES
            .iter()
            .find(|(a, _, _)| a.encode_utf16().eq(name.iter().copied()))
            .map(|&(_, auth, subs)| Sid::new(auth, subs))
            .or_else(|| match (s[0], s[1]) {
                (0x4C, 0x41) => Some(sid::domain_relative(rid::DOMAIN_USER_RID_ADMIN)),
                (0x4C, 0x47) => Some(sid::domain_relative(rid::DOMAIN_USER_RID_GUEST)),
                _ => None,
            });
    }
    let mut parts = s.split(|&c| c == u16::from(b'-'));
    let head = parts.next()?;
    if head != [u16::from(b'S')] && head != [u16::from(b's')] {
        return None;
    }
    if number(parts.next()?, false, 0xFF)? != u64::from(sid::SID_REVISION) {
        return None;
    }
    let auth = parts.next()?;
    let authority = if auth.len() > 2 && auth[0] == u16::from(b'0') && (auth[1] | 0x20) == u16::from(b'x') {
        number(&auth[2..], true, 0xFFFF_FFFF_FFFF)?
    } else {
        number(auth, false, 0xFFFF_FFFF_FFFF)?
    };
    let mut subs = Vec::new();
    for p in parts {
        if subs.len() == sid::SID_MAX_SUB_AUTHORITIES {
            return None;
        }
        subs.push(number(p, false, u64::from(u32::MAX))? as u32);
    }
    Some(Sid::new(authority, &subs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sid::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn standard_form_round_trips() {
        for s in ["S-1-1-0", "S-1-5-32-544", "S-1-5-21-0-0-0-1000", "S-1-16-12288", "S-1-5", "S-1-5-4294967295"] {
            let sid = parse(&w(s)).unwrap();
            assert_eq!(to_string(&sid), s);
            assert_eq!(to_wide(&sid.to_bytes()), Some(w(s)));
        }
        assert_eq!(to_string(&Sid::new(0x0102_0304_0506, &[7])), "S-1-0x010203040506-7");
        assert_eq!(parse(&w("S-1-0x010203040506-7")), Some(Sid::new(0x0102_0304_0506, &[7])));
        assert_eq!(parse(&w("s-1-5-18")), Some(local_system()));
    }

    #[test]
    fn bad_strings() {
        for s in [
            "",
            "S",
            "S-",
            "S-1",
            "S-2-5-18",
            "X-1-5-18",
            "S-1--18",
            "S-1-5-",
            "S-1-5-4294967296",
            "S-1-5-+1",
            "S-1-5- 1",
            "S-1-281474976710656",
            "ba",
            "ZZ",
        ] {
            assert_eq!(parse(&w(s)), None, "{s}");
        }
        // At most 15 sub-authorities.
        let fifteen = alloc::format!("S-1-5{}", "-1".repeat(15));
        assert_eq!(parse(&w(&fifteen)).map(|s| s.subs.len()), Some(15));
        assert_eq!(parse(&w(&alloc::format!("{fifteen}-1"))), None);
        assert_eq!(to_wide(&[2, 0, 0, 0, 0, 0, 0, 0]), None);
    }

    #[test]
    fn constants() {
        assert_eq!(parse(&w("BA")), Some(builtin(rid::DOMAIN_ALIAS_RID_ADMINS)));
        assert_eq!(parse(&w("WD")), Some(world()));
        assert_eq!(parse(&w("SY")), Some(local_system()));
        assert_eq!(parse(&w("HI")), Some(mandatory(0x3000)));
        assert_eq!(parse(&w("AC")).map(|s| to_string(&s)).as_deref(), Some("S-1-15-2-1"));
        assert_eq!(parse(&w("LA")), Some(domain_relative(500)));
        assert_eq!(parse(&w("LG")), Some(domain_relative(501)));
        let mut seen = alloc::collections::BTreeSet::new();
        for (a, _, _) in ALIASES {
            assert!(seen.insert(*a), "{a} twice");
            assert_eq!(a.len(), 2);
        }
    }
}
