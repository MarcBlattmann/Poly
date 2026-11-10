//! SID strings (Learn: "SID Components", "SID Strings";
//! `ConvertSidToStringSidW`, `ConvertStringSidToSidW`).
//!
//! The standard form is `S-R-I-S-S...`: `S`, the revision, the identifier
//! authority, then each sub-authority, separated by hyphens. The authority
//! is decimal below 2^32 and `0x` and uppercase hexadecimal digits without
//! leading zeros above (the spec sheets).
//!
//! `ConvertStringSidToSidW` (observed, spec sheets): the `S` in either case;
//! numbers in decimal or with a `0x`/`0X` prefix in hexadecimal (a leading
//! zero stays decimal); any revision number is stored; the authority up to
//! 2^48 - 1; sub-authorities saturate at 2^32 - 1; at least one and at most
//! [`MAX_PARSED_SUBS`] sub-authorities; no sign, no space.
//!
//! Besides the standard form it takes the two-letter SID string constants
//! of `sddl.h` ([`ALIASES`]), in either case; the ones that need a domain
//! controller fail (`ERROR_INVALID_SID`), `LA` and `LG` are relative to the
//! machine domain.

use crate::sid::{self, Sid, rid};
use alloc::string::String;
use alloc::vec::Vec;

/// The most sub-authorities a SID string may carry (the SID is then invalid
/// above 15 but still built; spec sheets: 16 and 17 are accepted).
pub const MAX_PARSED_SUBS: usize = 17;

/// The SID string constants Poly knows and their SIDs' (authority,
/// sub-authorities) (Learn: "SID Strings"; the RIDs from `winnt.h`).
pub const ALIASES: &[(&str, u64, &[u32])] = &[
    ("AA", 5, &[32, 579]),
    (
        "AC",
        sid::SECURITY_APP_PACKAGE_AUTHORITY,
        &[rid::SECURITY_APP_PACKAGE_BASE_RID, rid::SECURITY_BUILTIN_PACKAGE_ANY_PACKAGE],
    ),
    ("AN", 5, &[rid::SECURITY_ANONYMOUS_LOGON_RID]),
    ("AO", 5, &[32, rid::DOMAIN_ALIAS_RID_ACCOUNT_OPS]),
    ("AS", 18, &[1]),
    ("AU", 5, &[rid::SECURITY_AUTHENTICATED_USER_RID]),
    ("BA", 5, &[32, rid::DOMAIN_ALIAS_RID_ADMINS]),
    ("BG", 5, &[32, rid::DOMAIN_ALIAS_RID_GUESTS]),
    ("BO", 5, &[32, rid::DOMAIN_ALIAS_RID_BACKUP_OPS]),
    ("BU", 5, &[32, rid::DOMAIN_ALIAS_RID_USERS]),
    ("CD", 5, &[32, 574]),
    ("CG", 3, &[rid::SECURITY_CREATOR_GROUP_RID]),
    ("CO", 3, &[rid::SECURITY_CREATOR_OWNER_RID]),
    ("CY", 5, &[32, 569]),
    ("ED", 5, &[rid::SECURITY_SERVER_LOGON_RID]),
    ("ER", 5, &[32, 573]),
    ("ES", 5, &[32, 576]),
    ("HA", 5, &[32, 578]),
    ("HI", 16, &[rid::SECURITY_MANDATORY_HIGH_RID]),
    ("IS", 5, &[32, 568]),
    ("IU", 5, &[rid::SECURITY_INTERACTIVE_RID]),
    ("LS", 5, &[rid::SECURITY_LOCAL_SERVICE_RID]),
    ("LU", 5, &[32, rid::DOMAIN_ALIAS_RID_LOGGING_USERS]),
    ("LW", 16, &[rid::SECURITY_MANDATORY_LOW_RID]),
    ("ME", 16, &[rid::SECURITY_MANDATORY_MEDIUM_RID]),
    ("MP", 16, &[rid::SECURITY_MANDATORY_MEDIUM_PLUS_RID]),
    ("MS", 5, &[32, 577]),
    ("MU", 5, &[32, rid::DOMAIN_ALIAS_RID_MONITORING_USERS]),
    ("NO", 5, &[32, rid::DOMAIN_ALIAS_RID_NETWORK_CONFIGURATION_OPS]),
    ("NS", 5, &[rid::SECURITY_NETWORK_SERVICE_RID]),
    ("NU", 5, &[rid::SECURITY_NETWORK_RID]),
    ("OW", 3, &[rid::SECURITY_CREATOR_OWNER_RIGHTS_RID]),
    ("PO", 5, &[32, rid::DOMAIN_ALIAS_RID_PRINT_OPS]),
    ("PS", 5, &[rid::SECURITY_PRINCIPAL_SELF_RID]),
    ("PU", 5, &[32, rid::DOMAIN_ALIAS_RID_POWER_USERS]),
    ("RA", 5, &[32, 575]),
    ("RC", 5, &[rid::SECURITY_RESTRICTED_CODE_RID]),
    ("RD", 5, &[32, rid::DOMAIN_ALIAS_RID_REMOTE_DESKTOP_USERS]),
    ("RE", 5, &[32, rid::DOMAIN_ALIAS_RID_REPLICATOR]),
    ("RM", 5, &[32, 580]),
    ("RU", 5, &[32, rid::DOMAIN_ALIAS_RID_PREW2KCOMPACCESS]),
    ("SI", 16, &[rid::SECURITY_MANDATORY_SYSTEM_RID]),
    ("SO", 5, &[32, rid::DOMAIN_ALIAS_RID_SYSTEM_OPS]),
    ("SS", 18, &[2]),
    ("SU", 5, &[rid::SECURITY_SERVICE_RID]),
    ("SY", 5, &[rid::SECURITY_LOCAL_SYSTEM_RID]),
    ("UD", 5, &[84, 0, 0, 0, 0, 0]),
    ("WD", 1, &[rid::SECURITY_WORLD_RID]),
    ("WR", 5, &[33]),
];

/// The aliases that `ConvertSecurityDescriptorToStringSecurityDescriptor`
/// prints for a SID (the others, `AS`, `SS`, `UD`, print as numbers).
const PRINT_ALIASES: &[&str] = &[
    "AA", "AC", "AN", "AO", "AU", "BA", "BG", "BO", "BU", "CD", "CG", "CO", "CY", "ED", "ER", "ES", "HA", "HI", "IS",
    "IU", "LS", "LU", "LW", "ME", "MP", "MS", "MU", "NO", "NS", "NU", "OW", "PO", "PS", "PU", "RA", "RC", "RD", "RE",
    "RM", "RU", "SI", "SO", "SU", "SY", "WD", "WR",
];

/// The SID of a two-letter constant (either case); `LA` and `LG` are the
/// machine's administrator and guest.
pub fn alias_sid(a: u8, b: u8) -> Option<Sid> {
    let name = [a.to_ascii_uppercase(), b.to_ascii_uppercase()];
    if let Some(&(_, auth, subs)) = ALIASES.iter().find(|(n, _, _)| n.as_bytes() == name) {
        return Some(Sid::new(auth, subs));
    }
    match &name {
        b"LA" => Some(sid::domain_relative(rid::DOMAIN_USER_RID_ADMIN)),
        b"LG" => Some(sid::domain_relative(rid::DOMAIN_USER_RID_GUEST)),
        _ => None,
    }
}

/// The alias a SID is printed as, if it has one.
pub fn alias_of(s: &Sid) -> Option<&'static str> {
    ALIASES
        .iter()
        .find(|(n, auth, subs)| *auth == s.authority && **subs == s.subs[..] && PRINT_ALIASES.contains(n))
        .map(|&(n, _, _)| n)
}

/// The standard string form of a SID.
pub fn to_string(s: &Sid) -> String {
    use core::fmt::Write;
    let mut out = String::from("S-1-");
    if s.authority >> 32 == 0 {
        let _ = write!(out, "{}", s.authority);
    } else {
        let _ = write!(out, "0x{:X}", s.authority);
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

/// A number: decimal digits, or `0x` or `0X` and hexadecimal digits;
/// saturating at `u64::MAX`. `None` for an empty or malformed one.
fn number(digits: &[u16]) -> Option<u64> {
    let (radix, body) = match digits {
        [0x30, x, rest @ ..] if (*x | 0x20) == u16::from(b'x') => (16u64, rest),
        _ => (10, digits),
    };
    if body.is_empty() {
        return None;
    }
    let mut v: u64 = 0;
    for &c in body {
        let d = char::from_u32(u32::from(c))?.to_digit(radix as u32)?;
        v = v.saturating_mul(radix).saturating_add(u64::from(d));
    }
    Some(v)
}

/// Parse a standard-form SID string into the bytes of a SID (`S-R-I-S...`):
/// the revision is stored as given, the count may be up to
/// [`MAX_PARSED_SUBS`]; whether the result is a valid SID is for
/// `IsValidSid` to say. `None` is `ERROR_INVALID_SID`.
pub fn parse_standard(s: &[u16]) -> Option<Vec<u8>> {
    let mut parts = s.split(|&c| c == u16::from(b'-'));
    let head = parts.next()?;
    if head != [u16::from(b'S')] && head != [u16::from(b's')] {
        return None;
    }
    let revision = number(parts.next()?)?;
    if revision > 0xFF {
        return None;
    }
    let authority = number(parts.next()?)?;
    if authority > 0xFFFF_FFFF_FFFF {
        return None;
    }
    let mut subs: Vec<u32> = Vec::new();
    for p in parts {
        if subs.len() == MAX_PARSED_SUBS {
            return None;
        }
        subs.push(number(p)?.min(u64::from(u32::MAX)) as u32);
    }
    if subs.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(8 + 4 * subs.len());
    out.push(revision as u8);
    out.push(subs.len() as u8);
    out.extend_from_slice(&sid::authority_bytes(authority));
    for v in subs {
        out.extend_from_slice(&v.to_le_bytes());
    }
    Some(out)
}

/// `ConvertStringSidToSidW`: the standard form, or a two-letter constant.
pub fn parse_bytes(s: &[u16]) -> Option<Vec<u8>> {
    if s.len() == 2 {
        let (a, b) = (u8::try_from(s[0]).ok()?, u8::try_from(s[1]).ok()?);
        return alias_sid(a, b).map(|x| x.to_bytes());
    }
    parse_standard(s)
}

/// Parse into a valid SID value (`None` for a string that is not one, or
/// whose SID is invalid, e.g. revision 2).
pub fn parse(s: &[u16]) -> Option<Sid> {
    Sid::from_bytes(&parse_bytes(s)?)
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
        for s in ["S-1-1-0", "S-1-5-32-544", "S-1-5-21-0-0-0-1000", "S-1-16-12288", "S-1-5-4294967295"] {
            let sid = parse(&w(s)).unwrap();
            assert_eq!(to_string(&sid), s);
            assert_eq!(to_wide(&sid.to_bytes()), Some(w(s)));
        }
        assert_eq!(to_string(&Sid::new(0x0102_0304_0506, &[7])), "S-1-0x10203040506-7");
        assert_eq!(to_string(&Sid::new(0x1_0000_0000, &[7])), "S-1-0x100000000-7");
        assert_eq!(parse(&w("S-1-0x10203040506-7")), Some(Sid::new(0x0102_0304_0506, &[7])));
        assert_eq!(parse(&w("s-1-5-18")), Some(local_system()));
        assert_eq!(to_string(&Sid::new(16, &[7])), "S-1-16-7");
        assert_eq!(to_string(&Sid::new(0x0100_0000, &[7])), "S-1-16777216-7");
    }

    #[test]
    fn lenient_forms() {
        assert_eq!(parse(&w("S-1-0x5-18")), Some(local_system()));
        assert_eq!(parse(&w("S-1-5-0x12")), Some(local_system()));
        assert_eq!(parse(&w("S-1-5-012")), Some(nt(12)));
        assert_eq!(parse(&w("S-1-5-4294967296")), Some(nt(u32::MAX)));
        assert_eq!(parse(&w("S-1-5-18-0")).map(|s| s.subs), Some(alloc::vec![18, 0]));
        let b = parse_bytes(&w("S-2-5-18")).unwrap();
        assert_eq!(b[0], 2);
        assert_eq!(parse(&w("S-2-5-18")), None);
        assert_eq!(parse_bytes(&w(&alloc::format!("S-1-5{}", "-1".repeat(16)))).unwrap()[1], 16);
        assert_eq!(parse_bytes(&w("S-1-0xFFFFFFFFFFFF-1")).unwrap()[2..8], [0xFF; 6]);
    }

    #[test]
    fn bad_strings() {
        for s in [
            "",
            "S",
            "S-",
            "S-1",
            "S-1-5",
            "X-1-5-18",
            "S-1--18",
            "S-1-5-",
            "S-1-5-+1",
            "S-1-5- 1",
            "S-1-5-18 ",
            " S-1-5-18",
            "S-1-5-1a",
            "S-1-281474976710656-1",
            "S-1-5--1",
            "ZZ",
            "SYS",
            "CA",
            "DA",
        ] {
            assert_eq!(parse_bytes(&w(s)), None, "{s}");
        }
        let eighteen = alloc::format!("S-1-5{}", "-1".repeat(18));
        assert_eq!(parse_bytes(&w(&eighteen)), None);
    }

    #[test]
    fn constants() {
        assert_eq!(parse(&w("BA")), Some(builtin(rid::DOMAIN_ALIAS_RID_ADMINS)));
        assert_eq!(parse(&w("ba")), Some(builtin(rid::DOMAIN_ALIAS_RID_ADMINS)));
        assert_eq!(parse(&w("Sy")), Some(local_system()));
        assert_eq!(parse(&w("WD")), Some(world()));
        assert_eq!(parse(&w("HI")), Some(mandatory(0x3000)));
        assert_eq!(parse(&w("AC")).map(|s| to_string(&s)).as_deref(), Some("S-1-15-2-1"));
        assert_eq!(parse(&w("LA")), Some(domain_relative(500)));
        assert_eq!(parse(&w("LG")), Some(domain_relative(501)));
        assert_eq!(alias_of(&builtin(544)), Some("BA"));
        assert_eq!(alias_of(&Sid::new(5, &[32, 557])), None);
        assert_eq!(alias_of(&Sid::new(18, &[2])), None);
        let mut seen = alloc::collections::BTreeSet::new();
        for (a, _, _) in ALIASES {
            assert!(seen.insert(*a), "{a} twice");
            assert_eq!(a.len(), 2);
        }
    }
}
