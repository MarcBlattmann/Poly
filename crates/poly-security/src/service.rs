//! Who a Windows service runs as (ADR 0221): the tokens of the service accounts and the service
//! SID.
//!
//! From `specs/windows/advapi32.dll/SERVICES.md` section 4 (observed through a service's own
//! token): a LocalSystem service's process has the user `S-1-5-18`, session 0, the System
//! integrity level `S-1-16-16384` and the groups Everyone and Authenticated Users, without
//! `S-1-5-6` (SERVICE); LocalService and NetworkService are `S-1-5-19` and `S-1-5-20` the same
//! way. A service whose `ServiceSidType` is 1 (unrestricted) also has the group
//! `S-1-5-80-a-b-c-d-e` with attributes `0xE` and the group `S-1-5-6`, where `a` to `e` are the
//! five little-endian 32-bit words of the SHA-1 of the upper-cased service name in UTF-16LE.
//!
//! Kept apart from the interactive user's token ([`Token::poly_user`]) on purpose: the user's
//! token follows the elevation model (ADR 0171 and its successors), a service's does not.

use crate::sid::{self, Sid, rid};
use crate::token::*;
use crate::acl;
use alloc::vec;
use poly_crypto::sha1::Sha1;

/// The accounts a service can run as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Account {
    LocalSystem,
    LocalService,
    NetworkService,
}

impl Account {
    /// The account's SID.
    pub fn sid(self) -> Sid {
        sid::nt(match self {
            Account::LocalSystem => rid::SECURITY_LOCAL_SYSTEM_RID,
            Account::LocalService => rid::SECURITY_LOCAL_SERVICE_RID,
            Account::NetworkService => rid::SECURITY_NETWORK_SERVICE_RID,
        })
    }

    /// The account's logon session (`SYSTEM_LUID` 0x3E7, `LOCALSERVICE_LUID` 0x3E5,
    /// `NETWORKSERVICE_LUID` 0x3E4, `winnt.h`): its service window station is
    /// `Service-0x0-<luid>$`.
    pub fn logon_luid(self) -> u64 {
        match self {
            Account::LocalSystem => 0x3E7,
            Account::LocalService => 0x3E5,
            Account::NetworkService => 0x3E4,
        }
    }

    /// The name of the window station its services run on (`Service-0x0-3e7$`).
    pub fn station_name(self) -> alloc::string::String {
        alloc::format!("Service-0x0-{:x}$", self.logon_luid())
    }
}

/// `S-1-5-80-a-b-c-d-e` for service `name`: the SHA-1 of the upper-cased name in UTF-16LE, as five
/// little-endian 32-bit words.
pub fn service_sid(name: &[u16]) -> Sid {
    let mut h = Sha1::new();
    for &c in name {
        h.update(&upcase(c).to_le_bytes());
    }
    let d = h.finish();
    let w = |i: usize| u32::from_le_bytes([d[4 * i], d[4 * i + 1], d[4 * i + 2], d[4 * i + 3]]);
    Sid::new(sid::SECURITY_NT_AUTHORITY, &[80, w(0), w(1), w(2), w(3), w(4)])
}

/// One UTF-16 unit in upper case (the units of the Basic Multilingual Plane that have a one-unit
/// upper case; surrogates and the rest stay).
pub fn upcase(c: u16) -> u16 {
    if (0xD800..0xE000).contains(&c) {
        return c;
    }
    let Some(ch) = char::from_u32(u32::from(c)) else { return c };
    let mut up = ch.to_uppercase();
    match (up.next(), up.next()) {
        (Some(u), None) if (u as u32) < 0x1_0000 => u as u16,
        _ => c,
    }
}

/// The attributes of the service SID group (observed: `0xE`).
pub const SERVICE_SID_ATTRIBUTES: u32 = SE_GROUP_ENABLED_BY_DEFAULT | SE_GROUP_ENABLED | SE_GROUP_OWNER;

impl Token {
    /// The primary token of a service process running as `account`, with the group of its service
    /// SID when it has one (`ServiceSidType` 1 or 3). `id` is the token's LUID.
    pub fn service(account: Account, service: Option<Sid>, id: u64) -> Token {
        const G: u32 = SE_GROUP_MANDATORY | SE_GROUP_ENABLED_BY_DEFAULT | SE_GROUP_ENABLED;
        let admins = sid::builtin(rid::DOMAIN_ALIAS_RID_ADMINS);
        let user = account.sid();
        let mut groups = vec![];
        if account == Account::LocalSystem {
            groups.push((admins.clone(), G | SE_GROUP_OWNER));
        }
        groups.push((sid::world(), G));
        groups.push((sid::nt(rid::SECURITY_AUTHENTICATED_USER_RID), G));
        if let Some(s) = service {
            groups.push((sid::nt(rid::SECURITY_SERVICE_RID), G));
            groups.push((s, SERVICE_SID_ATTRIBUTES));
        }
        groups.push((
            sid::mandatory(rid::SECURITY_MANDATORY_SYSTEM_RID),
            SE_GROUP_INTEGRITY | SE_GROUP_INTEGRITY_ENABLED,
        ));
        let on = SE_PRIVILEGE_ENABLED | SE_PRIVILEGE_ENABLED_BY_DEFAULT;
        // LocalSystem holds every privilege (Learn: "LocalSystem Account"); the two service
        // accounts the few Learn lists for them.
        let luids: &[u32] = match account {
            Account::LocalSystem => &[
                2, 3, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 28, 29, 30, 31,
                33, 34, 35, 36,
            ],
            Account::LocalService | Account::NetworkService => &[3, 5, 21, 23, 29, 30, 34, 36],
        };
        let privileges = luids.iter().map(|&l| (l, on)).collect();
        let owner = if account == Account::LocalSystem { admins.clone() } else { user.clone() };
        let default_dacl = acl::build(
            acl::ACL_REVISION,
            &[(0, 0, GENERIC_ALL, &sid::local_system()), (0, 0, 0xA002_0000, &admins), (0, 0, GENERIC_ALL, &user)],
        );
        let mut t = Token::poly_user();
        t.id = id;
        t.authentication_id = account.logon_luid();
        t.modified_id = id;
        t.user = user.clone();
        t.user_attrs = 0;
        t.groups = groups;
        t.restricted = vec![];
        t.privileges = privileges;
        t.owner = owner;
        t.primary_group = user;
        t.object_dacl = Some(default_dacl.clone());
        t.default_dacl = Some(default_dacl);
        t.elevated = true;
        t.elevation_type = TOKEN_ELEVATION_TYPE_DEFAULT;
        t.session = 0;
        t.origin = 0;
        t.app_container = None;
        t
    }
}

/// The account a service's `ObjectName` names: `LocalSystem` (and an empty or missing name),
/// `NT AUTHORITY\LocalService` and `NT AUTHORITY\NetworkService` (also without the domain, in any
/// case, and as their SID strings' names `.\LocalSystem`). `None` for any other account.
pub fn account_of(name: &[u16]) -> Option<Account> {
    let text: alloc::string::String =
        char::decode_utf16(name.iter().copied()).map(|c| c.unwrap_or('?').to_ascii_lowercase()).collect();
    let bare = text.rsplit('\\').next().unwrap_or("");
    let domain = text.rsplit_once('\\').map(|(d, _)| d);
    match (domain, bare) {
        (None | Some(".") | Some("nt authority"), "localsystem" | "system") | (None, "") => Some(Account::LocalSystem),
        (None | Some("nt authority"), "localservice" | "local service") => Some(Account::LocalService),
        (None | Some("nt authority"), "networkservice" | "network service") => Some(Account::NetworkService),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sddl;

    fn w(s: &str) -> alloc::vec::Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn the_service_sid_of_a_name_is_the_one_windows_gives() {
        // SERVICES.md / the visible test svc_service_sid: PolySpecSvcSid2.
        let s = service_sid(&w("PolySpecSvcSid2"));
        assert_eq!(sddl::to_string(&s), "S-1-5-80-1277672605-383501034-2443229130-2253393169-603946302");
        // The name is upper-cased first.
        assert_eq!(service_sid(&w("polyspecsvcsid2")), s);
    }

    #[test]
    fn a_local_system_token() {
        let t = Token::service(Account::LocalSystem, None, 0x1234);
        assert_eq!(t.user, sid::local_system());
        assert_eq!(t.session, 0);
        assert_eq!(t.integrity_rid(), rid::SECURITY_MANDATORY_SYSTEM_RID);
        assert!(t.groups.iter().any(|(s, _)| *s == sid::world()));
        assert!(!t.groups.iter().any(|(s, _)| *s == sid::nt(rid::SECURITY_SERVICE_RID)));
        let with = Token::service(Account::LocalSystem, Some(service_sid(&w("x"))), 1);
        assert!(with.groups.iter().any(|(s, a)| s.subs.first() == Some(&80) && *a == 0xE));
        assert!(with.groups.iter().any(|(s, _)| *s == sid::nt(rid::SECURITY_SERVICE_RID)));
        assert_eq!(Account::LocalSystem.station_name(), "Service-0x0-3e7$");
    }

    #[test]
    fn account_names() {
        assert_eq!(account_of(&w("LocalSystem")), Some(Account::LocalSystem));
        assert_eq!(account_of(&w("")), Some(Account::LocalSystem));
        assert_eq!(account_of(&w("NT AUTHORITY\\LocalService")), Some(Account::LocalService));
        assert_eq!(account_of(&w("nt authority\\networkservice")), Some(Account::NetworkService));
        assert_eq!(account_of(&w("NoSuchAccountPolySpec")), None);
    }
}
