//! The account database of a machine with one user (ADR 0171): what
//! `LookupAccountSidW` and `LookupAccountNameW` (and the name trustees of
//! `SetEntriesInAclW`) know. Well-known SIDs have fixed names and
//! domains (the sheets of `LookupAccountSidW` / `LookupAccountNameW`);
//! Poly's own user and its primary group live in the machine's domain.

use crate::sid::{self, Sid};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// `SID_NAME_USE` (`winnt.h`).
pub mod name_use {
    pub const USER: u32 = 1;
    pub const GROUP: u32 = 2;
    pub const DOMAIN: u32 = 3;
    pub const ALIAS: u32 = 4;
    pub const WELL_KNOWN_GROUP: u32 = 5;
    pub const LABEL: u32 = 10;
    pub const LOGON_SESSION: u32 = 11;
}

/// The machine's name (the domain of its accounts).
pub const COMPUTER: &str = "POLY";

/// What a SID is called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub domain: String,
    pub kind: u32,
}

/// `(authority, subs, domain, name, use)` of the fixed accounts.
fn table() -> Vec<(u64, Vec<u32>, &'static str, &'static str, u32)> {
    use name_use::*;
    let nt = "NT AUTHORITY";
    let mut t: Vec<(u64, Vec<u32>, &'static str, &'static str, u32)> = alloc::vec![
        (0, alloc::vec![0], "", "NULL SID", WELL_KNOWN_GROUP),
        (1, alloc::vec![0], "", "Everyone", WELL_KNOWN_GROUP),
        (2, alloc::vec![0], "", "LOCAL", WELL_KNOWN_GROUP),
        (2, alloc::vec![1], "", "CONSOLE LOGON", WELL_KNOWN_GROUP),
        (3, alloc::vec![0], "", "CREATOR OWNER", WELL_KNOWN_GROUP),
        (3, alloc::vec![1], "", "CREATOR GROUP", WELL_KNOWN_GROUP),
        (3, alloc::vec![4], "", "OWNER RIGHTS", WELL_KNOWN_GROUP),
    ];
    for (rid, name) in [
        (1, "DIALUP"),
        (2, "NETWORK"),
        (3, "BATCH"),
        (4, "INTERACTIVE"),
        (6, "SERVICE"),
        (7, "ANONYMOUS LOGON"),
        (9, "ENTERPRISE DOMAIN CONTROLLERS"),
        (10, "SELF"),
        (11, "Authenticated Users"),
        (12, "RESTRICTED"),
        (13, "TERMINAL SERVER USER"),
        (14, "REMOTE INTERACTIVE LOGON"),
        (15, "This Organization"),
        (18, "SYSTEM"),
        (19, "LOCAL SERVICE"),
        (20, "NETWORK SERVICE"),
        (113, "Local account"),
        (114, "Local account and member of Administrators group"),
    ] {
        t.push((5, alloc::vec![rid], nt, name, WELL_KNOWN_GROUP));
    }
    t.push((5, alloc::vec![64, 10], nt, "NTLM Authentication", WELL_KNOWN_GROUP));
    for (rid, name) in [(544, "Administrators"), (545, "Users"), (546, "Guests"), (547, "Power Users")] {
        // Power Users (547) is not an account of this machine (the sheets).
        if rid != 547 {
            t.push((5, alloc::vec![32, rid], "BUILTIN", name, ALIAS));
        }
    }
    t.push((5, alloc::vec![80, 0], "NT SERVICE", "ALL SERVICES", WELL_KNOWN_GROUP));
    let pkg = "APPLICATION PACKAGE AUTHORITY";
    t.push((15, alloc::vec![2, 1], pkg, "ALL APPLICATION PACKAGES", WELL_KNOWN_GROUP));
    t.push((15, alloc::vec![2, 2], pkg, "ALL RESTRICTED APP PACKAGES", WELL_KNOWN_GROUP));
    t.push((15, alloc::vec![3, 1], pkg, "Your Internet connection", WELL_KNOWN_GROUP));
    for (rid, name) in [
        (0, "Untrusted Mandatory Level"),
        (0x1000, "Low Mandatory Level"),
        (0x2000, "Medium Mandatory Level"),
        (0x3000, "High Mandatory Level"),
        (0x4000, "System Mandatory Level"),
    ] {
        t.push((16, alloc::vec![rid], "Mandatory Label", name, LABEL));
    }
    t
}

/// The account of `s`, if the machine has one.
pub fn lookup_sid(s: &Sid) -> Option<Account> {
    for (auth, subs, domain, name, kind) in table() {
        if s.authority == auth && s.subs == subs {
            return Some(Account { name: name.to_string(), domain: domain.to_string(), kind });
        }
    }
    // A logon session: S-1-5-5-<high>-<low>.
    if s.authority == 5 && s.subs.len() == 3 && s.subs[0] == 5 {
        return Some(Account {
            name: format!("LogonSessionId_{}_{}", s.subs[1], s.subs[2]),
            domain: "NT AUTHORITY".to_string(),
            kind: name_use::LOGON_SESSION,
        });
    }
    // Poly's own accounts.
    let me = crate::poly_user_sid();
    if *s == me {
        return Some(Account {
            name: crate::POLY_USER_NAME.to_string(),
            domain: COMPUTER.to_string(),
            kind: name_use::USER,
        });
    }
    if *s == sid::domain_relative(crate::sid::rid::DOMAIN_GROUP_RID_USERS) {
        return Some(Account { name: "None".to_string(), domain: COMPUTER.to_string(), kind: name_use::GROUP });
    }
    None
}

fn fold(s: &str) -> String {
    s.to_uppercase()
}

/// The SID, domain and use of the account `name` names (`DOMAIN\name` or
/// `name`, case ignored). The empty name and `BUILTIN` are the BUILTIN
/// domain itself; an authority alone is not an account.
pub fn lookup_name(name: &str) -> Option<(Sid, String, u32)> {
    let builtin_domain = || Some((Sid::new(5, &[32]), "BUILTIN".to_string(), name_use::DOMAIN));
    if name.is_empty() || name == "\\" || fold(name) == "BUILTIN" {
        return builtin_domain();
    }
    let (domain, account) = match name.split_once('\\') {
        Some((d, a)) => (Some(d), a),
        None => (None, name),
    };
    if account.is_empty() {
        return None;
    }
    let (want_domain, want_account) = (domain.map(fold), fold(account));
    for (auth, subs, d, n, kind) in table() {
        if fold(n) != want_account {
            continue;
        }
        if let Some(wd) = &want_domain {
            // A domain prefix is accepted for the authorities that have names
            // of their own, not for the package authority.
            if fold(d) != *wd || d == "APPLICATION PACKAGE AUTHORITY" {
                continue;
            }
        }
        return Some((Sid::new(auth, &subs), d.to_string(), kind));
    }
    let me = crate::POLY_USER_NAME;
    for (n, s, kind) in [
        (me, crate::poly_user_sid(), name_use::USER),
        ("None", sid::domain_relative(crate::sid::rid::DOMAIN_GROUP_RID_USERS), name_use::GROUP),
    ] {
        if fold(n) == want_account && want_domain.as_deref().is_none_or(|d| d == COMPUTER) {
            return Some((s, COMPUTER.to_string(), kind));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_sids() {
        let (s, d, k) = lookup_name("nt authority\\system").unwrap();
        assert_eq!((s.to_bytes().len(), d.as_str(), k), (12, "NT AUTHORITY", 5));
        assert_eq!(lookup_sid(&s).unwrap().name, "SYSTEM");
        assert_eq!(lookup_name("Administrators").unwrap().1, "BUILTIN");
        assert!(lookup_name("ALL APPLICATION PACKAGES").is_some());
        assert!(lookup_name("APPLICATION PACKAGE AUTHORITY\\ALL APPLICATION PACKAGES").is_none());
        assert!(lookup_name("ALL RESTRICTED APPLICATION PACKAGES").is_none());
        assert!(lookup_name("NT AUTHORITY").is_none() && lookup_name("NT AUTHORITY\\").is_none());
        assert_eq!(lookup_name("").unwrap().2, name_use::DOMAIN);
        assert!(lookup_name("SYSTEM ").is_none() && lookup_name("S-1-1-0").is_none());
        assert_eq!(lookup_sid(&Sid::new(5, &[5, 0, 12345])).unwrap().name, "LogonSessionId_0_12345");
        assert!(lookup_sid(&Sid::new(5, &[32, 547])).is_none());
        assert_eq!(lookup_sid(&crate::poly_user_sid()).unwrap().name, "poly");
    }
}
