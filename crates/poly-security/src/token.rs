//! The single user's access token and what `NtQueryInformationToken` /
//! `GetTokenInformation` return for it.
//!
//! Poly has one user (ADR 0066, 0083), so every process has the same
//! primary token, [`Token::poly_user`]: the user `S-1-5-21-0-0-0-1000`, a
//! member of Administrators and Users, **elevated** (the owner of the
//! machine; there is no UAC split token, so the elevation type is
//! `TokenElevationTypeDefault`), at the high integrity level. Nothing is
//! enforced: the token only answers questions programs ask (am I an
//! administrator, elevated, which user am I).
//!
//! Each `TOKEN_INFORMATION_CLASS` structure is laid out as `winnt.h`
//! declares it (x64): the fixed part first, then the SIDs and ACLs its
//! pointers point at, inside the caller's buffer, so the bytes are built
//! for the buffer's address ([`Token::query`]). Learn ("GetTokenInformation",
//! `ReturnLength`): when the buffer is smaller than the structure "the
//! function fails and stores no data in the buffer" (our
//! `STATUS_BUFFER_TOO_SMALL`, `ERROR_INSUFFICIENT_BUFFER`); the length
//! needed is reported either way.
//!
//! To be confirmed by the spec agent against real Windows: the group list
//! and attributes, the privileges and which are enabled, the owner (ours:
//! the user), the primary group (ours: `S-1-5-21-0-0-0-513`), the default
//! DACL (ours: `GENERIC_ALL` for the user and for SYSTEM), the LUIDs, the
//! session (1), the statistics' dynamic sizes, a too-small buffer for a
//! fixed-size class (ours: `STATUS_BUFFER_TOO_SMALL` like the others), and
//! the classes Poly does not serve (`STATUS_INVALID_INFO_CLASS`), among
//! them `TokenImpersonationLevel` of a primary token; `TokenLinkedToken`
//! of a token without one is `STATUS_NO_SUCH_LOGON_SESSION`.

use crate::acl;
use crate::sid::{self, Sid, rid};
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::status::{STATUS_BUFFER_TOO_SMALL, STATUS_INVALID_INFO_CLASS, STATUS_SUCCESS};

/// `STATUS_NO_SUCH_LOGON_SESSION` (`ntstatus.h`).
pub const STATUS_NO_SUCH_LOGON_SESSION: NtStatus = NtStatus(0xC000_005F);
/// `STATUS_NO_TOKEN` (`ntstatus.h`): the thread is not impersonating.
pub const STATUS_NO_TOKEN: NtStatus = NtStatus(0xC000_007C);

/// Token access rights (`winnt.h`).
pub const TOKEN_ASSIGN_PRIMARY: u32 = 0x0001;
pub const TOKEN_DUPLICATE: u32 = 0x0002;
pub const TOKEN_IMPERSONATE: u32 = 0x0004;
pub const TOKEN_QUERY: u32 = 0x0008;
pub const TOKEN_QUERY_SOURCE: u32 = 0x0010;
pub const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
pub const TOKEN_ADJUST_GROUPS: u32 = 0x0040;
pub const TOKEN_ADJUST_DEFAULT: u32 = 0x0080;
pub const TOKEN_ADJUST_SESSIONID: u32 = 0x0100;

/// The token pseudo-handles (`processthreadsapi.h`:
/// `GetCurrentProcessToken`, `GetCurrentThreadToken`,
/// `GetCurrentThreadEffectiveToken`).
pub const CURRENT_PROCESS_TOKEN: i64 = -4;
pub const CURRENT_THREAD_TOKEN: i64 = -5;
pub const CURRENT_THREAD_EFFECTIVE_TOKEN: i64 = -6;

/// `TOKEN_INFORMATION_CLASS` (`winnt.h`).
pub mod class {
    pub const TOKEN_USER: u32 = 1;
    pub const TOKEN_GROUPS: u32 = 2;
    pub const TOKEN_PRIVILEGES: u32 = 3;
    pub const TOKEN_OWNER: u32 = 4;
    pub const TOKEN_PRIMARY_GROUP: u32 = 5;
    pub const TOKEN_DEFAULT_DACL: u32 = 6;
    pub const TOKEN_SOURCE: u32 = 7;
    pub const TOKEN_TYPE: u32 = 8;
    pub const TOKEN_IMPERSONATION_LEVEL: u32 = 9;
    pub const TOKEN_STATISTICS: u32 = 10;
    pub const TOKEN_RESTRICTED_SIDS: u32 = 11;
    pub const TOKEN_SESSION_ID: u32 = 12;
    pub const TOKEN_SANDBOX_INERT: u32 = 15;
    pub const TOKEN_ORIGIN: u32 = 17;
    pub const TOKEN_ELEVATION_TYPE: u32 = 18;
    pub const TOKEN_LINKED_TOKEN: u32 = 19;
    pub const TOKEN_ELEVATION: u32 = 20;
    pub const TOKEN_HAS_RESTRICTIONS: u32 = 21;
    pub const TOKEN_VIRTUALIZATION_ALLOWED: u32 = 23;
    pub const TOKEN_VIRTUALIZATION_ENABLED: u32 = 24;
    pub const TOKEN_INTEGRITY_LEVEL: u32 = 25;
    pub const TOKEN_UI_ACCESS: u32 = 26;
    pub const TOKEN_MANDATORY_POLICY: u32 = 27;
    pub const TOKEN_LOGON_SID: u32 = 28;
    pub const TOKEN_IS_APP_CONTAINER: u32 = 29;
}

/// Group attributes (`winnt.h`, `SE_GROUP_*`).
pub const SE_GROUP_MANDATORY: u32 = 0x1;
pub const SE_GROUP_ENABLED_BY_DEFAULT: u32 = 0x2;
pub const SE_GROUP_ENABLED: u32 = 0x4;
pub const SE_GROUP_OWNER: u32 = 0x8;
pub const SE_GROUP_USE_FOR_DENY_ONLY: u32 = 0x10;
pub const SE_GROUP_INTEGRITY: u32 = 0x20;
pub const SE_GROUP_INTEGRITY_ENABLED: u32 = 0x40;
pub const SE_GROUP_LOGON_ID: u32 = 0xC000_0000;

/// Privilege attributes (`winnt.h`, `SE_PRIVILEGE_*`).
pub const SE_PRIVILEGE_ENABLED_BY_DEFAULT: u32 = 0x1;
pub const SE_PRIVILEGE_ENABLED: u32 = 0x2;

/// `TOKEN_TYPE`: `TokenPrimary`.
pub const TOKEN_PRIMARY: u32 = 1;
/// `TOKEN_ELEVATION_TYPE` (`winnt.h`).
pub const TOKEN_ELEVATION_TYPE_DEFAULT: u32 = 1;
pub const TOKEN_ELEVATION_TYPE_FULL: u32 = 2;
pub const TOKEN_ELEVATION_TYPE_LIMITED: u32 = 3;
/// `TOKEN_MANDATORY_POLICY_NO_WRITE_UP | TOKEN_MANDATORY_POLICY_NEW_PROCESS_MIN`
/// (`winnt.h`), the default policy.
pub const TOKEN_MANDATORY_POLICY_VALID: u32 = 0x3;

/// `GENERIC_ALL` (`winnt.h`).
pub const GENERIC_ALL: u32 = 0x1000_0000;

/// The privileges by LUID (`wdm.h`, `SE_*_PRIVILEGE`) with their names
/// (`winnt.h`, `SE_*_NAME`), for `LookupPrivilegeValueW` /
/// `LookupPrivilegeNameW`.
pub const PRIVILEGES: &[(u32, &str)] = &[
    (2, "SeCreateTokenPrivilege"),
    (3, "SeAssignPrimaryTokenPrivilege"),
    (4, "SeLockMemoryPrivilege"),
    (5, "SeIncreaseQuotaPrivilege"),
    (6, "SeMachineAccountPrivilege"),
    (7, "SeTcbPrivilege"),
    (8, "SeSecurityPrivilege"),
    (9, "SeTakeOwnershipPrivilege"),
    (10, "SeLoadDriverPrivilege"),
    (11, "SeSystemProfilePrivilege"),
    (12, "SeSystemtimePrivilege"),
    (13, "SeProfileSingleProcessPrivilege"),
    (14, "SeIncreaseBasePriorityPrivilege"),
    (15, "SeCreatePagefilePrivilege"),
    (16, "SeCreatePermanentPrivilege"),
    (17, "SeBackupPrivilege"),
    (18, "SeRestorePrivilege"),
    (19, "SeShutdownPrivilege"),
    (20, "SeDebugPrivilege"),
    (21, "SeAuditPrivilege"),
    (22, "SeSystemEnvironmentPrivilege"),
    (23, "SeChangeNotifyPrivilege"),
    (24, "SeRemoteShutdownPrivilege"),
    (25, "SeUndockPrivilege"),
    (26, "SeSyncAgentPrivilege"),
    (27, "SeEnableDelegationPrivilege"),
    (28, "SeManageVolumePrivilege"),
    (29, "SeImpersonatePrivilege"),
    (30, "SeCreateGlobalPrivilege"),
    (31, "SeTrustedCredManAccessPrivilege"),
    (32, "SeRelabelPrivilege"),
    (33, "SeIncreaseWorkingSetPrivilege"),
    (34, "SeTimeZonePrivilege"),
    (35, "SeCreateSymbolicLinkPrivilege"),
    (36, "SeDelegateSessionUserImpersonatePrivilege"),
];

/// The LUID of a privilege name (ASCII case ignored).
pub fn privilege_value(name: &[u16]) -> Option<u32> {
    PRIVILEGES
        .iter()
        .find(|(_, n)| {
            n.len() == name.len() && n.bytes().zip(name).all(|(a, &b)| b < 0x80 && a.eq_ignore_ascii_case(&(b as u8)))
        })
        .map(|&(luid, _)| luid)
}

/// The name of a privilege LUID.
pub fn privilege_name(luid: u32) -> Option<&'static str> {
    PRIVILEGES.iter().find(|&&(l, _)| l == luid).map(|&(_, n)| n)
}

/// An access token's contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub user: Sid,
    /// Groups and their `SE_GROUP_*` attributes.
    pub groups: Vec<(Sid, u32)>,
    /// Privilege LUIDs and their `SE_PRIVILEGE_*` attributes.
    pub privileges: Vec<(u32, u32)>,
    pub owner: Sid,
    pub primary_group: Sid,
    pub default_dacl: Vec<u8>,
    pub integrity: Sid,
    pub elevated: bool,
    pub elevation_type: u32,
    pub session: u32,
    /// The token's LUID and its logon session's.
    pub id: u64,
    pub authentication_id: u64,
}

/// The single user's logon session LUID (to be confirmed: any unique value).
pub const POLY_LOGON_LUID: u64 = 0x3_E800;
/// The token's LUID.
pub const POLY_TOKEN_LUID: u64 = 0x3_E801;

impl Token {
    /// Poly's single user's token.
    pub fn poly_user() -> Token {
        let user = crate::poly_user_sid();
        const G: u32 = SE_GROUP_MANDATORY | SE_GROUP_ENABLED_BY_DEFAULT | SE_GROUP_ENABLED;
        let integrity = sid::mandatory(rid::SECURITY_MANDATORY_HIGH_RID);
        let groups = alloc::vec![
            (sid::domain_relative(rid::DOMAIN_GROUP_RID_USERS), G),
            (sid::world(), G),
            (sid::builtin(rid::DOMAIN_ALIAS_RID_ADMINS), G | SE_GROUP_OWNER),
            (sid::builtin(rid::DOMAIN_ALIAS_RID_USERS), G),
            (sid::nt(rid::SECURITY_INTERACTIVE_RID), G),
            (Sid::new(sid::SECURITY_LOCAL_SID_AUTHORITY, &[rid::SECURITY_LOCAL_LOGON_RID]), G),
            (sid::nt(rid::SECURITY_AUTHENTICATED_USER_RID), G),
            (sid::nt(rid::SECURITY_THIS_ORGANIZATION_RID), G),
            (logon_sid(POLY_LOGON_LUID), G | SE_GROUP_LOGON_ID),
            (Sid::new(sid::SECURITY_LOCAL_SID_AUTHORITY, &[rid::SECURITY_LOCAL_RID]), G),
            (integrity.clone(), SE_GROUP_INTEGRITY | SE_GROUP_INTEGRITY_ENABLED),
        ];
        let on = SE_PRIVILEGE_ENABLED | SE_PRIVILEGE_ENABLED_BY_DEFAULT;
        let privileges = [5, 8, 9, 10, 11, 12, 13, 14, 15, 17, 18, 19, 20, 22, 23, 24, 25, 28, 29, 30, 33, 34, 35, 36]
            .into_iter()
            .map(|l| (l, if matches!(l, 23 | 29 | 30) { on } else { 0 }))
            .collect();
        let default_dacl =
            acl::build(acl::ACL_REVISION, &[(0, 0, GENERIC_ALL, &user), (0, 0, GENERIC_ALL, &sid::local_system())]);
        Token {
            owner: user.clone(),
            primary_group: sid::domain_relative(rid::DOMAIN_GROUP_RID_USERS),
            user,
            groups,
            privileges,
            default_dacl,
            integrity,
            elevated: true,
            elevation_type: TOKEN_ELEVATION_TYPE_DEFAULT,
            session: 1,
            id: POLY_TOKEN_LUID,
            authentication_id: POLY_LOGON_LUID,
        }
    }

    /// The logon SID (`S-1-5-5-X-Y`): the group with `SE_GROUP_LOGON_ID`.
    pub fn logon_sid(&self) -> Option<&(Sid, u32)> {
        self.groups.iter().find(|(_, a)| a & SE_GROUP_LOGON_ID == SE_GROUP_LOGON_ID)
    }

    /// The structure of `class` for a buffer at address `base`, or the
    /// status for a class Poly does not serve.
    pub fn query(&self, class: u32, base: u64) -> Result<Vec<u8>, NtStatus> {
        use class::*;
        let mut out = Vec::new();
        match class {
            TOKEN_USER => sid_and_attributes(&mut out, base, &[(&self.user, 0)]),
            TOKEN_GROUPS => token_groups(&mut out, base, self.groups.iter().map(|(s, a)| (s, *a))),
            TOKEN_LOGON_SID => token_groups(&mut out, base, self.logon_sid().into_iter().map(|(s, a)| (s, *a))),
            TOKEN_PRIVILEGES => {
                u32s(&mut out, &[self.privileges.len() as u32]);
                for &(luid, attrs) in &self.privileges {
                    u32s(&mut out, &[luid, 0, attrs]);
                }
            }
            TOKEN_OWNER => pointer_then(&mut out, base, &self.owner.to_bytes()),
            TOKEN_PRIMARY_GROUP => pointer_then(&mut out, base, &self.primary_group.to_bytes()),
            TOKEN_DEFAULT_DACL => pointer_then(&mut out, base, &self.default_dacl),
            TOKEN_SOURCE => {
                out.extend_from_slice(b"Poly\0\0\0\0");
                out.extend_from_slice(&self.authentication_id.to_le_bytes());
            }
            TOKEN_TYPE => u32s(&mut out, &[TOKEN_PRIMARY]),
            TOKEN_STATISTICS => {
                out.extend_from_slice(&self.id.to_le_bytes());
                out.extend_from_slice(&self.authentication_id.to_le_bytes());
                out.extend_from_slice(&i64::MAX.to_le_bytes());
                let dynamic = (self.default_dacl.len() + self.primary_group.len()) as u32;
                u32s(&mut out, &[TOKEN_PRIMARY, 0, dynamic, 0, self.groups.len() as u32, self.privileges.len() as u32]);
                out.extend_from_slice(&self.id.to_le_bytes());
            }
            TOKEN_SESSION_ID => u32s(&mut out, &[self.session]),
            TOKEN_ELEVATION_TYPE => u32s(&mut out, &[self.elevation_type]),
            TOKEN_ELEVATION => u32s(&mut out, &[u32::from(self.elevated)]),
            TOKEN_LINKED_TOKEN => return Err(STATUS_NO_SUCH_LOGON_SESSION),
            TOKEN_INTEGRITY_LEVEL => sid_and_attributes(
                &mut out,
                base,
                &[(&self.integrity, SE_GROUP_INTEGRITY | SE_GROUP_INTEGRITY_ENABLED)],
            ),
            TOKEN_MANDATORY_POLICY => u32s(&mut out, &[TOKEN_MANDATORY_POLICY_VALID]),
            TOKEN_SANDBOX_INERT
            | TOKEN_HAS_RESTRICTIONS
            | TOKEN_VIRTUALIZATION_ALLOWED
            | TOKEN_VIRTUALIZATION_ENABLED
            | TOKEN_UI_ACCESS
            | TOKEN_IS_APP_CONTAINER => u32s(&mut out, &[0]),
            TOKEN_ORIGIN => out.extend_from_slice(&self.authentication_id.to_le_bytes()),
            _ => return Err(STATUS_INVALID_INFO_CLASS),
        }
        Ok(out)
    }
}

/// `S-1-5-5-<high>-<low>`: the logon SID of a logon session LUID.
pub fn logon_sid(luid: u64) -> Sid {
    sid::Sid::new(sid::SECURITY_NT_AUTHORITY, &[rid::SECURITY_LOGON_IDS_RID, (luid >> 32) as u32, luid as u32])
}

fn u32s(out: &mut Vec<u8>, v: &[u32]) {
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

/// A pointer to the bytes that follow it (`TOKEN_OWNER`, ...).
fn pointer_then(out: &mut Vec<u8>, base: u64, data: &[u8]) {
    out.extend_from_slice(&(base + 8).to_le_bytes());
    out.extend_from_slice(data);
}

/// `SID_AND_ATTRIBUTES[n]` (16 bytes each: pointer, attributes, padding)
/// at the current end of `out`, then the SIDs.
fn sid_and_attributes(out: &mut Vec<u8>, base: u64, entries: &[(&Sid, u32)]) {
    let start = out.len();
    let mut sid_at = start + 16 * entries.len();
    for (s, a) in entries {
        out.extend_from_slice(&(base + sid_at as u64).to_le_bytes());
        u32s(out, &[*a, 0]);
        sid_at += s.len();
    }
    for (s, _) in entries {
        s.write(out);
    }
}

/// `TOKEN_GROUPS`: `GroupCount`, padding to 8, the array, the SIDs.
fn token_groups<'a>(out: &mut Vec<u8>, base: u64, groups: impl Iterator<Item = (&'a Sid, u32)>) {
    let groups: Vec<(&Sid, u32)> = groups.collect();
    u32s(out, &[groups.len() as u32, 0]);
    sid_and_attributes(out, base, &groups);
}

/// The status and what to copy for a buffer of `length` bytes (`ReturnLength`
/// is always the structure's size).
pub fn fit(bytes: &[u8], length: usize) -> (NtStatus, Option<&[u8]>) {
    if length >= bytes.len() { (STATUS_SUCCESS, Some(bytes)) } else { (STATUS_BUFFER_TOO_SMALL, None) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sddl;

    fn u64_at(b: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
    }
    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn token_user_points_into_the_buffer() {
        let t = Token::poly_user();
        let base = 0x1000_0000;
        let b = t.query(class::TOKEN_USER, base).unwrap();
        assert_eq!(b.len(), 16 + 28);
        assert_eq!(u64_at(&b, 0), base + 16);
        assert_eq!(u32_at(&b, 8), 0);
        assert_eq!(sddl::to_string(&Sid::from_bytes(&b[16..]).unwrap()), "S-1-5-21-0-0-0-1000");
    }

    #[test]
    fn token_groups_layout() {
        let t = Token::poly_user();
        let base = 0x2000;
        let b = t.query(class::TOKEN_GROUPS, base).unwrap();
        let n = u32_at(&b, 0) as usize;
        assert_eq!(n, t.groups.len());
        let mut names = Vec::new();
        for i in 0..n {
            let p = (u64_at(&b, 8 + 16 * i) - base) as usize;
            let attrs = u32_at(&b, 16 + 16 * i);
            let s = Sid::from_bytes(&b[p..]).unwrap();
            names.push((sddl::to_string(&s), attrs));
        }
        assert!(names.contains(&(alloc::string::String::from("S-1-5-32-544"), 0xF)));
        assert!(names.contains(&(alloc::string::String::from("S-1-1-0"), 7)));
        assert!(names.contains(&(alloc::string::String::from("S-1-16-12288"), 0x60)));
        let last = names.len() - 1;
        let end = (u64_at(&b, 8 + 16 * last) - base) as usize
            + Sid::from_bytes(&b[(u64_at(&b, 8 + 16 * last) - base) as usize..]).unwrap().len();
        assert_eq!(end, b.len());
        // The logon SID class is a TOKEN_GROUPS of one.
        let l = t.query(class::TOKEN_LOGON_SID, base).unwrap();
        assert_eq!(u32_at(&l, 0), 1);
        assert_eq!(u32_at(&l, 16), 0xC000_0007);
        assert_eq!(sddl::to_string(&Sid::from_bytes(&l[24..]).unwrap()), "S-1-5-5-0-256000");
    }

    #[test]
    fn small_classes() {
        let t = Token::poly_user();
        assert_eq!(t.query(class::TOKEN_ELEVATION, 0).unwrap(), [1, 0, 0, 0]);
        assert_eq!(t.query(class::TOKEN_ELEVATION_TYPE, 0).unwrap(), [1, 0, 0, 0]);
        assert_eq!(t.query(class::TOKEN_TYPE, 0).unwrap(), [1, 0, 0, 0]);
        assert_eq!(t.query(class::TOKEN_SESSION_ID, 0).unwrap(), [1, 0, 0, 0]);
        assert_eq!(t.query(class::TOKEN_IS_APP_CONTAINER, 0).unwrap(), [0, 0, 0, 0]);
        assert_eq!(t.query(class::TOKEN_STATISTICS, 0).unwrap().len(), 56);
        assert_eq!(t.query(class::TOKEN_SOURCE, 0).unwrap().len(), 16);
        assert_eq!(t.query(class::TOKEN_IMPERSONATION_LEVEL, 0), Err(STATUS_INVALID_INFO_CLASS));
        assert_eq!(t.query(class::TOKEN_LINKED_TOKEN, 0), Err(STATUS_NO_SUCH_LOGON_SESSION));
        assert_eq!(t.query(999, 0), Err(STATUS_INVALID_INFO_CLASS));
        let il = t.query(class::TOKEN_INTEGRITY_LEVEL, 0x100).unwrap();
        assert_eq!(u64_at(&il, 0), 0x110);
        assert_eq!(u32_at(&il, 8), 0x60);
        assert_eq!(sddl::to_string(&Sid::from_bytes(&il[16..]).unwrap()), "S-1-16-12288");
    }

    #[test]
    fn owner_group_dacl_privileges() {
        let t = Token::poly_user();
        let o = t.query(class::TOKEN_OWNER, 0x40).unwrap();
        assert_eq!(u64_at(&o, 0), 0x48);
        assert_eq!(Sid::from_bytes(&o[8..]), Some(t.user.clone()));
        let g = t.query(class::TOKEN_PRIMARY_GROUP, 0).unwrap();
        assert_eq!(sddl::to_string(&Sid::from_bytes(&g[8..]).unwrap()), "S-1-5-21-0-0-0-513");
        let d = t.query(class::TOKEN_DEFAULT_DACL, 0).unwrap();
        assert!(acl::is_valid(&d[8..]));
        let p = t.query(class::TOKEN_PRIVILEGES, 0).unwrap();
        let n = u32_at(&p, 0) as usize;
        assert_eq!(p.len(), 4 + 12 * n);
        let change_notify = (0..n).find(|i| u32_at(&p, 4 + 12 * i) == 23).unwrap();
        assert_eq!(u32_at(&p, 4 + 12 * change_notify + 8), 3);
        let debug = (0..n).find(|i| u32_at(&p, 4 + 12 * i) == 20).unwrap();
        assert_eq!(u32_at(&p, 4 + 12 * debug + 8), 0);
    }

    #[test]
    fn privilege_names() {
        let w: Vec<u16> = "sedebugprivilege".encode_utf16().collect();
        assert_eq!(privilege_value(&w), Some(20));
        assert_eq!(privilege_name(23), Some("SeChangeNotifyPrivilege"));
        assert_eq!(privilege_value(&[0x53]), None);
        assert_eq!(privilege_name(1), None);
    }

    #[test]
    fn buffer_rule() {
        let b = [1u8, 2, 3, 4];
        assert_eq!(fit(&b, 4), (STATUS_SUCCESS, Some(&b[..])));
        assert_eq!(fit(&b, 3), (STATUS_BUFFER_TOO_SMALL, None));
    }
}
