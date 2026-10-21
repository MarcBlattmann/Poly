//! `SID` (`winnt.h`): a revision byte (`SID_REVISION`, 1), the number of
//! sub-authorities (at most `SID_MAX_SUB_AUTHORITIES`, 15), a 48-bit
//! identifier authority stored as six bytes, most significant first
//! (`SID_IDENTIFIER_AUTHORITY.Value[6]`), and the sub-authorities as
//! little-endian `DWORD`s. Its length is `8 + 4 * count` bytes
//! (`GetSidLengthRequired`, `GetLengthSid`).

use alloc::vec::Vec;

/// `SID_REVISION`.
pub const SID_REVISION: u8 = 1;
/// `SID_MAX_SUB_AUTHORITIES`.
pub const SID_MAX_SUB_AUTHORITIES: usize = 15;
/// The fixed part: revision, count, authority.
pub const SID_HEADER: usize = 8;

/// Identifier authorities (`winnt.h`, `SECURITY_*_AUTHORITY`).
pub const SECURITY_NULL_SID_AUTHORITY: u64 = 0;
pub const SECURITY_WORLD_SID_AUTHORITY: u64 = 1;
pub const SECURITY_LOCAL_SID_AUTHORITY: u64 = 2;
pub const SECURITY_CREATOR_SID_AUTHORITY: u64 = 3;
pub const SECURITY_NT_AUTHORITY: u64 = 5;
pub const SECURITY_APP_PACKAGE_AUTHORITY: u64 = 15;
pub const SECURITY_MANDATORY_LABEL_AUTHORITY: u64 = 16;

/// Relative identifiers (`winnt.h`).
pub mod rid {
    pub const SECURITY_WORLD_RID: u32 = 0;
    pub const SECURITY_LOCAL_RID: u32 = 0;
    pub const SECURITY_LOCAL_LOGON_RID: u32 = 1;
    pub const SECURITY_CREATOR_OWNER_RID: u32 = 0;
    pub const SECURITY_CREATOR_GROUP_RID: u32 = 1;
    pub const SECURITY_CREATOR_OWNER_RIGHTS_RID: u32 = 4;
    pub const SECURITY_NETWORK_RID: u32 = 2;
    pub const SECURITY_INTERACTIVE_RID: u32 = 4;
    pub const SECURITY_LOGON_IDS_RID: u32 = 5;
    pub const SECURITY_SERVICE_RID: u32 = 6;
    pub const SECURITY_ANONYMOUS_LOGON_RID: u32 = 7;
    pub const SECURITY_SERVER_LOGON_RID: u32 = 9;
    pub const SECURITY_PRINCIPAL_SELF_RID: u32 = 10;
    pub const SECURITY_AUTHENTICATED_USER_RID: u32 = 11;
    pub const SECURITY_RESTRICTED_CODE_RID: u32 = 12;
    pub const SECURITY_THIS_ORGANIZATION_RID: u32 = 15;
    pub const SECURITY_LOCAL_SYSTEM_RID: u32 = 18;
    pub const SECURITY_LOCAL_SERVICE_RID: u32 = 19;
    pub const SECURITY_NETWORK_SERVICE_RID: u32 = 20;
    pub const SECURITY_NT_NON_UNIQUE: u32 = 21;
    pub const SECURITY_BUILTIN_DOMAIN_RID: u32 = 32;
    pub const DOMAIN_USER_RID_ADMIN: u32 = 500;
    pub const DOMAIN_USER_RID_GUEST: u32 = 501;
    pub const DOMAIN_GROUP_RID_USERS: u32 = 513;
    pub const DOMAIN_ALIAS_RID_ADMINS: u32 = 544;
    pub const DOMAIN_ALIAS_RID_USERS: u32 = 545;
    pub const DOMAIN_ALIAS_RID_GUESTS: u32 = 546;
    pub const DOMAIN_ALIAS_RID_POWER_USERS: u32 = 547;
    pub const DOMAIN_ALIAS_RID_ACCOUNT_OPS: u32 = 548;
    pub const DOMAIN_ALIAS_RID_SYSTEM_OPS: u32 = 549;
    pub const DOMAIN_ALIAS_RID_PRINT_OPS: u32 = 550;
    pub const DOMAIN_ALIAS_RID_BACKUP_OPS: u32 = 551;
    pub const DOMAIN_ALIAS_RID_REPLICATOR: u32 = 552;
    pub const DOMAIN_ALIAS_RID_PREW2KCOMPACCESS: u32 = 554;
    pub const DOMAIN_ALIAS_RID_REMOTE_DESKTOP_USERS: u32 = 555;
    pub const DOMAIN_ALIAS_RID_NETWORK_CONFIGURATION_OPS: u32 = 556;
    pub const DOMAIN_ALIAS_RID_MONITORING_USERS: u32 = 558;
    pub const DOMAIN_ALIAS_RID_LOGGING_USERS: u32 = 559;
    pub const SECURITY_APP_PACKAGE_BASE_RID: u32 = 2;
    pub const SECURITY_BUILTIN_PACKAGE_ANY_PACKAGE: u32 = 1;
    pub const SECURITY_MANDATORY_UNTRUSTED_RID: u32 = 0;
    pub const SECURITY_MANDATORY_LOW_RID: u32 = 0x1000;
    pub const SECURITY_MANDATORY_MEDIUM_RID: u32 = 0x2000;
    pub const SECURITY_MANDATORY_MEDIUM_PLUS_RID: u32 = 0x2100;
    pub const SECURITY_MANDATORY_HIGH_RID: u32 = 0x3000;
    pub const SECURITY_MANDATORY_SYSTEM_RID: u32 = 0x4000;
}

/// A SID as a value.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Sid {
    /// The 48-bit identifier authority.
    pub authority: u64,
    /// At most [`SID_MAX_SUB_AUTHORITIES`].
    pub subs: Vec<u32>,
}

impl Sid {
    /// A SID; at most 15 sub-authorities (more are dropped).
    pub fn new(authority: u64, subs: &[u32]) -> Sid {
        Sid { authority: authority & 0xFFFF_FFFF_FFFF, subs: subs[..subs.len().min(SID_MAX_SUB_AUTHORITIES)].to_vec() }
    }

    /// Its length in bytes.
    pub fn len(&self) -> usize {
        length_required(self.subs.len() as u8) as usize
    }

    /// Never empty: the header is always there.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The last sub-authority (the relative identifier), if any.
    pub fn rid(&self) -> Option<u32> {
        self.subs.last().copied()
    }

    /// Append its bytes to `out`.
    pub fn write(&self, out: &mut Vec<u8>) {
        out.push(SID_REVISION);
        out.push(self.subs.len() as u8);
        out.extend_from_slice(&authority_bytes(self.authority));
        for s in &self.subs {
            out.extend_from_slice(&s.to_le_bytes());
        }
    }

    /// Its bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.len());
        self.write(&mut v);
        v
    }

    /// Parse a valid SID from the start of `bytes` ([`is_valid`]).
    pub fn from_bytes(bytes: &[u8]) -> Option<Sid> {
        let len = valid_length(bytes)?;
        let subs =
            bytes[SID_HEADER..len].chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        Some(Sid { authority: authority_of(&bytes[2..8]), subs })
    }
}

/// `SID_IDENTIFIER_AUTHORITY.Value` of a 48-bit authority.
pub fn authority_bytes(authority: u64) -> [u8; 6] {
    let b = authority.to_be_bytes();
    [b[2], b[3], b[4], b[5], b[6], b[7]]
}

/// The 48-bit authority of `SID_IDENTIFIER_AUTHORITY.Value` (six bytes).
pub fn authority_of(value: &[u8]) -> u64 {
    value.iter().take(6).fold(0u64, |a, &b| (a << 8) | u64::from(b))
}

/// `GetSidLengthRequired`.
pub const fn length_required(count: u8) -> u32 {
    SID_HEADER as u32 + 4 * count as u32
}

/// `GetLengthSid` of the SID starting at `bytes` (its header only is
/// trusted: Learn says the result for an invalid SID is undefined).
pub fn length_of(header: &[u8]) -> u32 {
    length_required(header.get(1).copied().unwrap_or(0))
}

/// The length of the valid SID at the start of `bytes`, if it is one.
pub fn valid_length(bytes: &[u8]) -> Option<usize> {
    let (&rev, &count) = (bytes.first()?, bytes.get(1)?);
    let len = length_required(count) as usize;
    (rev == SID_REVISION && usize::from(count) <= SID_MAX_SUB_AUTHORITIES && bytes.len() >= len).then_some(len)
}

/// `IsValidSid`: revision 1 and at most 15 sub-authorities (Learn:
/// "validates a SID structure by verifying that the revision number is
/// within a known range, and that the number of subauthorities is less
/// than the maximum").
pub fn is_valid(bytes: &[u8]) -> bool {
    valid_length(bytes).is_some()
}

/// `EqualSid`: the same bytes.
pub fn equal(a: &[u8], b: &[u8]) -> bool {
    match (valid_length(a), valid_length(b)) {
        (Some(la), Some(lb)) => la == lb && a[..la] == b[..lb],
        _ => false,
    }
}

/// `EqualPrefixSid`: the same revision, authority and sub-authorities but
/// the last.
pub fn equal_prefix(a: &[u8], b: &[u8]) -> bool {
    match (Sid::from_bytes(a), Sid::from_bytes(b)) {
        (Some(x), Some(y)) => {
            x.authority == y.authority
                && x.subs.len() == y.subs.len()
                && x.subs[..x.subs.len().saturating_sub(1)] == y.subs[..y.subs.len().saturating_sub(1)]
        }
        _ => false,
    }
}

// ----- well-known SIDs ---------------------------------------------------------------

/// `S-1-1-0`, Everyone.
pub fn world() -> Sid {
    Sid::new(SECURITY_WORLD_SID_AUTHORITY, &[rid::SECURITY_WORLD_RID])
}

/// `S-1-5-<rid>`: an NT authority SID.
pub fn nt(r: u32) -> Sid {
    Sid::new(SECURITY_NT_AUTHORITY, &[r])
}

/// `S-1-5-32-<rid>`: a built-in alias.
pub fn builtin(r: u32) -> Sid {
    Sid::new(SECURITY_NT_AUTHORITY, &[rid::SECURITY_BUILTIN_DOMAIN_RID, r])
}

/// `S-1-5-21-0-0-0-<rid>`: relative to the machine's account domain.
pub fn domain_relative(r: u32) -> Sid {
    let d = crate::MACHINE_DOMAIN;
    Sid::new(SECURITY_NT_AUTHORITY, &[d[0], d[1], d[2], d[3], r])
}

/// `S-1-16-<rid>`: an integrity level.
pub fn mandatory(r: u32) -> Sid {
    Sid::new(SECURITY_MANDATORY_LABEL_AUTHORITY, &[r])
}

/// `S-1-5-18`, LocalSystem.
pub fn local_system() -> Sid {
    nt(rid::SECURITY_LOCAL_SYSTEM_RID)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_and_lengths() {
        let s = builtin(rid::DOMAIN_ALIAS_RID_ADMINS);
        let b = s.to_bytes();
        assert_eq!(b, [1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 0x20, 2, 0, 0]);
        assert_eq!(s.len(), 16);
        assert_eq!(length_required(0), 8);
        assert_eq!(length_required(15), 68);
        assert_eq!(length_of(&b), 16);
        assert_eq!(Sid::from_bytes(&b), Some(s.clone()));
        assert_eq!(s.rid(), Some(544));
        // A 48-bit authority keeps its bytes big-endian.
        assert_eq!(authority_bytes(0x0102_0304_0506), [1, 2, 3, 4, 5, 6]);
        assert_eq!(authority_of(&[1, 2, 3, 4, 5, 6]), 0x0102_0304_0506);
    }

    #[test]
    fn validity() {
        let b = world().to_bytes();
        assert!(is_valid(&b));
        assert!(!is_valid(&b[..10]));
        let mut bad = b.clone();
        bad[0] = 2;
        assert!(!is_valid(&bad));
        let mut many = [0u8; 8 + 4 * 16];
        many[0] = 1;
        many[1] = 16;
        assert!(!is_valid(&many));
        many[1] = 15;
        assert!(is_valid(&many));
        assert_eq!(Sid::from_bytes(&many).map(|s| s.subs.len()), Some(15));
    }

    #[test]
    fn equality() {
        let a = builtin(544).to_bytes();
        let b = builtin(545).to_bytes();
        assert!(equal(&a, &a));
        assert!(!equal(&a, &b));
        assert!(equal_prefix(&a, &b));
        assert!(!equal_prefix(&a, &world().to_bytes()));
        // Trailing bytes after the SID do not count.
        let mut c = a.clone();
        c.extend_from_slice(&[9, 9]);
        assert!(equal(&a, &c));
    }

    #[test]
    fn well_known() {
        assert_eq!(local_system(), Sid::new(5, &[18]));
        assert_eq!(domain_relative(500), Sid::new(5, &[21, 0, 0, 0, 500]));
        assert_eq!(mandatory(rid::SECURITY_MANDATORY_HIGH_RID).subs, [0x3000]);
        assert_eq!(Sid::new(5, &[1; 20]).subs.len(), 15);
    }
}
