//! Access tokens and what `NtQueryInformationToken` /
//! `GetTokenInformation` return for them.
//!
//! A [`Token`] is the contents of one token object: its user, groups and
//! restricting SIDs with their attributes, its privileges, owner, primary
//! group, default DACL, integrity level and the flags programs ask about.
//! Poly's processes start with [`Token::poly_user`], an elevated
//! administrator's token like the one the spec sheets were observed with
//! (12 groups, 24 privileges, High integrity); every other token is made
//! from one by [`Token::duplicate`] and [`Token::filter`]
//! (`DuplicateTokenEx`, `CreateRestrictedToken`), and changed by
//! [`adjust_privileges`], [`Token::adjust_groups`] and
//! [`Token::set_information`].
//!
//! Each `TOKEN_INFORMATION_CLASS` structure is laid out as `winnt.h`
//! declares it (x64): the fixed part first, then the SIDs and ACLs its
//! pointers point at, inside the caller's buffer, so the bytes are built
//! for the buffer's address ([`Token::query`]). Learn ("GetTokenInformation",
//! `ReturnLength`): when the buffer is smaller than the structure "the
//! function fails and stores no data in the buffer"; the length needed is
//! reported either way.

use crate::access::{self, CheckToken};
use crate::acl::{self, AclPtr};
use crate::err::SecErr;
use crate::sd::{self, Parts};
use crate::sid::{self, Sid, rid};
use crate::status::{NtStatus, STATUS_BUFFER_TOO_SMALL, STATUS_INVALID_INFO_CLASS, STATUS_SUCCESS};
use alloc::vec::Vec;

/// `STATUS_NO_SUCH_LOGON_SESSION` (`ntstatus.h`).
pub const STATUS_NO_SUCH_LOGON_SESSION: NtStatus = NtStatus(0xC000_005F);
/// `STATUS_NO_TOKEN` (`ntstatus.h`): the thread is not impersonating.
pub const STATUS_NO_TOKEN: NtStatus = NtStatus(0xC000_007C);
/// `STATUS_INFO_LENGTH_MISMATCH`.
pub const STATUS_INFO_LENGTH_MISMATCH: NtStatus = NtStatus(0xC000_0004);
/// `STATUS_CANT_DISABLE_MANDATORY`, `STATUS_CANT_ENABLE_DENY_ONLY`.
pub const STATUS_CANT_DISABLE_MANDATORY: NtStatus = NtStatus(0xC000_005D);
pub const STATUS_CANT_ENABLE_DENY_ONLY: NtStatus = NtStatus(0xC000_02B3);
/// `STATUS_INVALID_LABEL`.
pub const STATUS_INVALID_LABEL: NtStatus = NtStatus(0xC000_0446);

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
/// `TOKEN_ALL_ACCESS` (`winnt.h`).
pub const TOKEN_ALL_ACCESS: u32 = 0xF01FF;
/// The token generic mapping (`TOKEN_READ` ...).
pub const TOKEN_MAPPING: access::GenericMapping =
    access::GenericMapping { read: 0x2_001A, write: 0x2_01E0, execute: 0x2_0005, all: TOKEN_ALL_ACCESS };

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
    pub const TOKEN_GROUPS_AND_PRIVILEGES: u32 = 13;
    pub const TOKEN_SANDBOX_INERT: u32 = 15;
    pub const TOKEN_AUDIT_POLICY: u32 = 16;
    pub const TOKEN_ORIGIN: u32 = 17;
    pub const TOKEN_ELEVATION_TYPE: u32 = 18;
    pub const TOKEN_LINKED_TOKEN: u32 = 19;
    pub const TOKEN_ELEVATION: u32 = 20;
    pub const TOKEN_HAS_RESTRICTIONS: u32 = 21;
    pub const TOKEN_ACCESS_INFORMATION: u32 = 22;
    pub const TOKEN_VIRTUALIZATION_ALLOWED: u32 = 23;
    pub const TOKEN_VIRTUALIZATION_ENABLED: u32 = 24;
    pub const TOKEN_INTEGRITY_LEVEL: u32 = 25;
    pub const TOKEN_UI_ACCESS: u32 = 26;
    pub const TOKEN_MANDATORY_POLICY: u32 = 27;
    pub const TOKEN_LOGON_SID: u32 = 28;
    pub const TOKEN_IS_APP_CONTAINER: u32 = 29;
    pub const TOKEN_CAPABILITIES: u32 = 30;
    pub const TOKEN_APP_CONTAINER_SID: u32 = 31;
    pub const TOKEN_APP_CONTAINER_NUMBER: u32 = 32;
    pub const TOKEN_USER_CLAIM_ATTRIBUTES: u32 = 33;
    pub const TOKEN_DEVICE_CLAIM_ATTRIBUTES: u32 = 34;
    pub const TOKEN_DEVICE_GROUPS: u32 = 37;
    pub const TOKEN_SECURITY_ATTRIBUTES: u32 = 39;
    pub const TOKEN_IS_RESTRICTED: u32 = 40;
    pub const TOKEN_PROCESS_TRUST_LEVEL: u32 = 41;
    pub const TOKEN_PRIVATE_NAME_SPACE: u32 = 42;
    pub const TOKEN_SINGLETON_ATTRIBUTES: u32 = 43;
    pub const TOKEN_BNO_ISOLATION: u32 = 44;
    pub const TOKEN_IS_SANDBOXED: u32 = 47;
    pub const TOKEN_IS_APP_SILO: u32 = 48;
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
pub const SE_PRIVILEGE_REMOVED: u32 = 0x4;

/// `STATUS_NOT_ALL_ASSIGNED` (`ntstatus.h`): a success status, Win32's
/// `ERROR_NOT_ALL_ASSIGNED` (1300).
pub const STATUS_NOT_ALL_ASSIGNED: NtStatus = NtStatus(0x0000_0106);

/// `TOKEN_TYPE` (`winnt.h`).
pub const TOKEN_PRIMARY: u32 = 1;
pub const TOKEN_IMPERSONATION: u32 = 2;
/// `SECURITY_IMPERSONATION_LEVEL` (`winnt.h`).
pub const SECURITY_ANONYMOUS: u32 = 0;
pub const SECURITY_IDENTIFICATION: u32 = 1;
pub const SECURITY_IMPERSONATION: u32 = 2;
pub const SECURITY_DELEGATION: u32 = 3;
/// `TOKEN_ELEVATION_TYPE` (`winnt.h`).
pub const TOKEN_ELEVATION_TYPE_DEFAULT: u32 = 1;
pub const TOKEN_ELEVATION_TYPE_FULL: u32 = 2;
pub const TOKEN_ELEVATION_TYPE_LIMITED: u32 = 3;
/// `TOKEN_MANDATORY_POLICY_NO_WRITE_UP | TOKEN_MANDATORY_POLICY_NEW_PROCESS_MIN`
/// (`winnt.h`), the default policy.
pub const TOKEN_MANDATORY_POLICY_VALID: u32 = 0x3;

/// `CreateRestrictedToken` flags (`winnt.h`).
pub const DISABLE_MAX_PRIVILEGE: u32 = 0x1;
pub const SANDBOX_INERT: u32 = 0x2;
pub const LUA_TOKEN: u32 = 0x4;
pub const WRITE_RESTRICTED: u32 = 0x8;

pub use crate::access::GENERIC_ALL;

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

/// The privileges a standard user's token (and a `LUA_TOKEN`) keeps:
/// `SeChangeNotify`, `SeShutdown`, `SeUndock`, `SeIncreaseWorkingSet`,
/// `SeTimeZone`.
const LUA_PRIVILEGES: [u32; 5] = [23, 19, 25, 33, 34];

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

/// What adjusting a token's privileges did ([`adjust_privileges`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adjusted {
    /// The privileges afterwards.
    pub privileges: Vec<(u32, u32)>,
    /// The privileges the adjustment changed, with their attributes before
    /// (`PreviousState`), each once, in the order they changed.
    pub previous: Vec<(u32, u32)>,
    /// Whether the token held every privilege `NewState` named.
    pub all_assigned: bool,
}

/// `AdjustTokenPrivileges` on a token holding `privileges` (Learn): with
/// `disable_all`, every enabled privilege is disabled and `new` ignored;
/// else each `new` entry enables (`SE_PRIVILEGE_ENABLED`), disables (no
/// attribute) or removes (`SE_PRIVILEGE_REMOVED`) a privilege the token
/// holds, and one it does not hold is skipped (`ERROR_NOT_ALL_ASSIGNED`:
/// "the function may succeed with this error value even if no privileges
/// were adjusted"). `PreviousState` lists only what changed ("if a
/// privilege has been modified by this function, the privilege and its
/// previous state are contained in" it). Other attribute bits of `new`
/// are ignored.
pub fn adjust_privileges(privileges: &[(u32, u32)], disable_all: bool, new: &[(u32, u32)]) -> Adjusted {
    fn note(previous: &mut Vec<(u32, u32)>, luid: u32, attrs: u32) {
        if !previous.iter().any(|&(l, _)| l == luid) {
            previous.push((luid, attrs));
        }
    }
    let mut now = privileges.to_vec();
    let mut previous = Vec::new();
    let mut all_assigned = true;
    if disable_all {
        for p in now.iter_mut() {
            if p.1 & SE_PRIVILEGE_ENABLED != 0 {
                note(&mut previous, p.0, p.1);
                p.1 &= !SE_PRIVILEGE_ENABLED;
            }
        }
        return Adjusted { privileges: now, previous, all_assigned };
    }
    for &(luid, attrs) in new {
        let Some(i) = now.iter().position(|&(l, _)| l == luid) else {
            all_assigned = false;
            continue;
        };
        let old = now[i].1;
        if attrs & SE_PRIVILEGE_REMOVED != 0 {
            note(&mut previous, luid, old);
            now.remove(i);
        } else if (attrs & SE_PRIVILEGE_ENABLED != 0) != (old & SE_PRIVILEGE_ENABLED != 0) {
            note(&mut previous, luid, old);
            now[i].1 = old ^ SE_PRIVILEGE_ENABLED;
        }
    }
    Adjusted { privileges: now, previous, all_assigned }
}

/// `TOKEN_PRIVILEGES` (`winnt.h`): `PrivilegeCount`, then 12-byte
/// `LUID_AND_ATTRIBUTES` (LUID low, high, attributes).
pub fn token_privileges(privileges: &[(u32, u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    u32s(&mut out, &[privileges.len() as u32]);
    for &(luid, attrs) in privileges {
        u32s(&mut out, &[luid, 0, attrs]);
    }
    out
}

/// A `TOKEN_PRIVILEGES` read back; `None` if `bytes` is shorter than its
/// count says. A LUID whose high part is not 0 names none of Poly's
/// privileges (all are below 2^32, `wdm.h`): it becomes `u32::MAX`, which
/// no token holds.
pub fn parse_token_privileges(bytes: &[u8]) -> Option<Vec<(u32, u32)>> {
    let n = u32::from_le_bytes(bytes.get(0..4)?.try_into().ok()?) as usize;
    let body = bytes.get(4..4 + n.checked_mul(12)?)?;
    Some(
        body.chunks_exact(12)
            .map(|c| {
                let w = |i: usize| u32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
                (if w(4) == 0 { w(0) } else { u32::MAX }, w(8))
            })
            .collect(),
    )
}

/// The result type of the token operations that fail with an `NTSTATUS`.
pub type TokenResult<T> = Result<T, NtStatus>;

const F_HAS_RESTRICTIONS: u32 = 1;
const F_WRITE_RESTRICTED: u32 = 2;
const F_SANDBOX_INERT: u32 = 4;
const F_UI_ACCESS: u32 = 8;
const F_VIRT_ALLOWED: u32 = 16;
const F_VIRT_ENABLED: u32 = 32;

/// An access token's contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// `TokenId` and `AuthenticationId` (LUIDs).
    pub id: u64,
    pub authentication_id: u64,
    pub modified_id: u64,
    /// [`TOKEN_PRIMARY`] or [`TOKEN_IMPERSONATION`].
    pub kind: u32,
    /// The impersonation level (an impersonation token's; a primary token
    /// reports [`SECURITY_IMPERSONATION`] in its statistics).
    pub level: u32,
    pub user: Sid,
    pub user_attrs: u32,
    /// Groups and their `SE_GROUP_*` attributes, the integrity label among
    /// them.
    pub groups: Vec<(Sid, u32)>,
    pub restricted: Vec<(Sid, u32)>,
    /// Privilege LUIDs and their `SE_PRIVILEGE_*` attributes.
    pub privileges: Vec<(u32, u32)>,
    pub owner: Sid,
    pub primary_group: Sid,
    /// The default DACL; `None` is a NULL DACL.
    pub default_dacl: Option<Vec<u8>>,
    pub mandatory_policy: u32,
    pub elevated: bool,
    pub elevation_type: u32,
    pub session: u32,
    pub origin: u64,
    /// The token object's own security: the creator's default DACL.
    pub object_dacl: Option<Vec<u8>>,
    flags: u32,
}

/// The single user's logon session LUID (to be confirmed: any unique value).
pub const POLY_LOGON_LUID: u64 = 0x3_E800;
/// The first token's LUID.
pub const POLY_TOKEN_LUID: u64 = 0x3_E801;

impl Token {
    /// Poly's single user's token: an elevated local administrator's, as
    /// the spec sheets observed one (High integrity, 12 groups, 24
    /// privileges, all enabled).
    pub fn poly_user() -> Token {
        let user = crate::poly_user_sid();
        const G: u32 = SE_GROUP_MANDATORY | SE_GROUP_ENABLED_BY_DEFAULT | SE_GROUP_ENABLED;
        let admins = sid::builtin(rid::DOMAIN_ALIAS_RID_ADMINS);
        let logon = logon_sid(POLY_LOGON_LUID);
        let groups = alloc::vec![
            (sid::world(), G),
            (sid::nt(114), G),
            (admins.clone(), G | SE_GROUP_OWNER),
            (sid::builtin(rid::DOMAIN_ALIAS_RID_USERS), G),
            (sid::nt(rid::SECURITY_NETWORK_RID), G),
            (sid::nt(rid::SECURITY_AUTHENTICATED_USER_RID), G),
            (sid::nt(rid::SECURITY_THIS_ORGANIZATION_RID), G),
            (sid::nt(113), G),
            (logon.clone(), G | SE_GROUP_LOGON_ID),
            (Sid::new(sid::SECURITY_NT_AUTHORITY, &[64, 10]), G),
            (sid::domain_relative(rid::DOMAIN_GROUP_RID_USERS), G),
            (sid::mandatory(rid::SECURITY_MANDATORY_HIGH_RID), SE_GROUP_INTEGRITY | SE_GROUP_INTEGRITY_ENABLED),
        ];
        let on = SE_PRIVILEGE_ENABLED | SE_PRIVILEGE_ENABLED_BY_DEFAULT;
        let privileges = [5, 8, 9, 10, 11, 12, 13, 14, 15, 17, 18, 19, 20, 22, 23, 24, 25, 28, 29, 30, 33, 34, 35, 36]
            .into_iter()
            .map(|l| (l, on))
            .collect();
        let default_dacl = acl::build(
            acl::ACL_REVISION,
            &[(0, 0, GENERIC_ALL, &admins), (0, 0, GENERIC_ALL, &sid::local_system()), (0, 0, 0xA000_0000, &logon)],
        );
        Token {
            id: POLY_TOKEN_LUID,
            authentication_id: POLY_LOGON_LUID,
            modified_id: POLY_TOKEN_LUID,
            kind: TOKEN_PRIMARY,
            level: SECURITY_IMPERSONATION,
            owner: admins,
            primary_group: sid::domain_relative(rid::DOMAIN_GROUP_RID_USERS),
            user,
            user_attrs: 0,
            groups,
            restricted: Vec::new(),
            privileges,
            object_dacl: Some(default_dacl.clone()),
            default_dacl: Some(default_dacl),
            mandatory_policy: TOKEN_MANDATORY_POLICY_VALID,
            elevated: true,
            elevation_type: TOKEN_ELEVATION_TYPE_DEFAULT,
            session: 0,
            origin: POLY_LOGON_LUID,
            flags: 0,
        }
    }

    /// The logon SID (`S-1-5-5-X-Y`): the group with `SE_GROUP_LOGON_ID`.
    pub fn logon_sid(&self) -> Option<&(Sid, u32)> {
        self.groups.iter().find(|(_, a)| a & SE_GROUP_LOGON_ID == SE_GROUP_LOGON_ID)
    }

    /// Whether the token has restricting SIDs (`IsTokenRestricted`).
    pub fn is_restricted(&self) -> bool {
        !self.restricted.is_empty()
    }

    /// `TokenHasRestrictions`.
    pub fn has_restrictions(&self) -> bool {
        self.flags & F_HAS_RESTRICTIONS != 0
    }

    pub fn write_restricted(&self) -> bool {
        self.flags & F_WRITE_RESTRICTED != 0
    }

    /// The integrity level (the label's RID).
    pub fn integrity_rid(&self) -> u32 {
        self.groups
            .iter()
            .find(|(_, a)| a & SE_GROUP_INTEGRITY != 0)
            .and_then(|(s, _)| s.rid())
            .unwrap_or(rid::SECURITY_MANDATORY_MEDIUM_RID)
    }

    /// The part of the token the access check looks at.
    pub fn check_view(&self) -> CheckToken<'_> {
        CheckToken {
            user: &self.user,
            user_attrs: self.user_attrs,
            groups: &self.groups,
            restricted: &self.restricted,
            privileges: &self.privileges,
            integrity: self.integrity_rid(),
            write_restricted: self.write_restricted(),
        }
    }

    /// The token object's security descriptor (owner the token's, DACL the
    /// creator's default).
    pub fn object_sd(&self) -> Parts {
        Parts {
            revision: sd::SECURITY_DESCRIPTOR_REVISION,
            sbz1: 0,
            control: sd::SE_DACL_PRESENT,
            owner: Some(self.owner.to_bytes()),
            group: Some(self.primary_group.to_bytes()),
            sacl: None,
            dacl: self.object_dacl.clone(),
        }
    }

    /// A copy as a token of `kind` and `level` with a new `id`
    /// (`DuplicateTokenEx`): the contents are the same, the privileges as
    /// they are now.
    pub fn duplicate(&self, kind: u32, level: u32, id: u64) -> Token {
        let mut t = self.clone();
        t.id = id;
        t.modified_id = id;
        t.kind = kind;
        t.level = if kind == TOKEN_PRIMARY { SECURITY_IMPERSONATION } else { level };
        t
    }

    /// `CreateRestrictedToken` / `NtFilterToken`: groups made deny-only,
    /// privileges deleted, restricting SIDs added (spec sheets). The result
    /// has the source's type and level. `restrict` entries must have no
    /// attributes ([`SecErr::InvalidParameter`] otherwise), and a token that
    /// is restricted cannot be restricted again.
    pub fn filter(
        &self,
        flags: u32,
        disable: &[Sid],
        delete: &[u32],
        restrict: &[(Sid, u32)],
        id: u64,
    ) -> Result<Token, SecErr> {
        if restrict.iter().any(|&(_, a)| a != 0) || (!restrict.is_empty() && self.is_restricted()) {
            return Err(SecErr::InvalidParameter);
        }
        let mut t = self.duplicate(self.kind, self.level, id);
        t.level = self.level;
        let mut changed = false;
        if flags & DISABLE_MAX_PRIVILEGE != 0 {
            let before = t.privileges.len();
            t.privileges.retain(|&(l, _)| l == 23);
            changed |= t.privileges.len() != before;
        }
        if flags & LUA_TOKEN != 0 {
            t.privileges.retain(|&(l, _)| LUA_PRIVILEGES.contains(&l));
            changed = true;
            let admins = sid::builtin(rid::DOMAIN_ALIAS_RID_ADMINS);
            for (g, a) in t.groups.iter_mut() {
                if *g == admins {
                    *a = SE_GROUP_USE_FOR_DENY_ONLY;
                }
            }
        }
        for s in disable {
            if *s == t.user {
                t.user_attrs = SE_GROUP_USE_FOR_DENY_ONLY;
                changed = true;
            }
            for (g, a) in t.groups.iter_mut() {
                if g == s && *a & SE_GROUP_INTEGRITY == 0 && *a & SE_GROUP_LOGON_ID != SE_GROUP_LOGON_ID {
                    *a = SE_GROUP_USE_FOR_DENY_ONLY;
                    changed = true;
                }
            }
        }
        for &l in delete {
            let before = t.privileges.len();
            t.privileges.retain(|&(p, _)| p != l);
            changed |= t.privileges.len() != before;
        }
        for (s, _) in restrict {
            t.restricted.push((s.clone(), SE_GROUP_MANDATORY | SE_GROUP_ENABLED_BY_DEFAULT | SE_GROUP_ENABLED));
            changed = true;
        }
        if flags & WRITE_RESTRICTED != 0 {
            t.flags |= F_WRITE_RESTRICTED;
            changed = true;
        }
        if changed {
            t.flags |= F_HAS_RESTRICTIONS;
        }
        Ok(t)
    }

    /// `AdjustTokenGroups` / `NtAdjustGroupsToken`: `reset` restores the
    /// default (enabled-by-default) state; else each entry enables (the
    /// enabled attribute) or disables (none) a group. Mandatory groups
    /// cannot be disabled, deny-only groups cannot be enabled. Returns the
    /// groups whose state changed (with the attributes before) and whether
    /// the token held every group named. A failure changes nothing.
    pub fn adjust_groups(&mut self, reset: bool, new: &[(Vec<u8>, u32)]) -> TokenResult<(Vec<(Sid, u32)>, bool)> {
        let mut previous = Vec::new();
        if reset {
            for (g, a) in self.groups.iter_mut() {
                if *a & SE_GROUP_USE_FOR_DENY_ONLY != 0 || *a & SE_GROUP_INTEGRITY != 0 {
                    continue;
                }
                let want =
                    if *a & SE_GROUP_ENABLED_BY_DEFAULT != 0 { *a | SE_GROUP_ENABLED } else { *a & !SE_GROUP_ENABLED };
                if want != *a {
                    previous.push((g.clone(), *a));
                    *a = want;
                }
            }
            return Ok((previous, true));
        }
        let mut all = true;
        let mut plan: Vec<(usize, u32)> = Vec::new();
        for (bytes, attrs) in new {
            let Some(s) = Sid::from_bytes(bytes) else {
                all = false;
                continue;
            };
            let Some(i) = self.groups.iter().position(|(g, _)| *g == s) else {
                all = false;
                continue;
            };
            let old = self.groups[i].1;
            let enable = attrs & SE_GROUP_ENABLED != 0 && attrs & SE_GROUP_USE_FOR_DENY_ONLY == 0;
            if old & SE_GROUP_USE_FOR_DENY_ONLY != 0 {
                if enable {
                    return Err(STATUS_CANT_ENABLE_DENY_ONLY);
                }
                continue;
            }
            if !enable && old & SE_GROUP_MANDATORY != 0 {
                return Err(STATUS_CANT_DISABLE_MANDATORY);
            }
            if old & SE_GROUP_INTEGRITY != 0 {
                continue;
            }
            let want = if enable { old | SE_GROUP_ENABLED } else { old & !SE_GROUP_ENABLED };
            if want != old {
                plan.push((i, want));
            }
        }
        for (i, want) in plan {
            previous.push((self.groups[i].0.clone(), self.groups[i].1));
            self.groups[i].1 = want;
        }
        Ok((previous, all))
    }

    /// `SetTokenInformation` / `NtSetInformationToken` of a class the
    /// caller's buffer carries resolved in `data`.
    pub fn set_information(&mut self, class: u32, data: &SetData) -> TokenResult<()> {
        use class::*;
        match (class, data) {
            (TOKEN_OWNER, SetData::Sid(Some(s))) => {
                let ok = *s == self.user
                    || self
                        .groups
                        .iter()
                        .any(|(g, a)| g == s && a & SE_GROUP_OWNER != 0 && a & SE_GROUP_USE_FOR_DENY_ONLY == 0);
                if !ok {
                    return Err(SecErr::InvalidOwner.nt());
                }
                self.owner = s.clone();
            }
            (TOKEN_PRIMARY_GROUP, SetData::Sid(Some(s))) => {
                if !self.groups.iter().any(|(g, _)| g == s) {
                    return Err(SecErr::InvalidGroup.nt());
                }
                self.primary_group = s.clone();
            }
            (TOKEN_OWNER | TOKEN_PRIMARY_GROUP, SetData::Sid(None)) => return Err(SecErr::InvalidParameter.nt()),
            (TOKEN_DEFAULT_DACL, SetData::Acl(a)) => {
                if let Some(a) = a {
                    // SAFETY: the bytes are a whole ACL as far as its header says.
                    if !unsafe { acl::is_valid(AclPtr(a.as_ptr() as *mut u8)) } {
                        return Err(SecErr::InvalidAcl.nt());
                    }
                }
                self.default_dacl = a.clone();
            }
            (TOKEN_INTEGRITY_LEVEL, SetData::Sid(Some(s))) => {
                if s.authority != sid::SECURITY_MANDATORY_LABEL_AUTHORITY || s.subs.len() != 1 {
                    return Err(STATUS_INVALID_LABEL);
                }
                let new = s.subs[0];
                if new > self.integrity_rid() {
                    return Err(SecErr::PrivilegeNotHeld.nt());
                }
                for (g, a) in self.groups.iter_mut() {
                    if *a & SE_GROUP_INTEGRITY != 0 {
                        *g = sid::mandatory(new);
                    }
                }
            }
            (TOKEN_INTEGRITY_LEVEL, SetData::Sid(None)) => return Err(SecErr::InvalidParameter.nt()),
            (TOKEN_VIRTUALIZATION_ALLOWED, SetData::Value(v)) => self.set_flag(F_VIRT_ALLOWED, *v != 0),
            (TOKEN_VIRTUALIZATION_ENABLED, SetData::Value(v)) => self.set_flag(F_VIRT_ENABLED, *v != 0),
            (TOKEN_UI_ACCESS | TOKEN_SESSION_ID | TOKEN_MANDATORY_POLICY | TOKEN_SANDBOX_INERT | TOKEN_ORIGIN, _) => {
                return Err(SecErr::PrivilegeNotHeld.nt());
            }
            _ => return Err(STATUS_INVALID_INFO_CLASS),
        }
        self.modified_id = self.modified_id.wrapping_add(1);
        Ok(())
    }

    fn set_flag(&mut self, bit: u32, on: bool) {
        if on {
            self.flags |= bit;
        } else {
            self.flags &= !bit;
        }
    }

    /// `NtCompareTokens`: the same security context (user, groups,
    /// privileges and restrictions with their attributes), whatever the
    /// ids, types and levels.
    pub fn same_context(&self, other: &Token) -> bool {
        self.user == other.user
            && self.user_attrs == other.user_attrs
            && self.groups == other.groups
            && self.restricted == other.restricted
            && self.privileges == other.privileges
            && self.flags & (F_HAS_RESTRICTIONS | F_WRITE_RESTRICTED)
                == other.flags & (F_HAS_RESTRICTIONS | F_WRITE_RESTRICTED)
    }

    /// `PrivilegeCheck`: marks the entries found enabled with
    /// `SE_PRIVILEGE_USED_FOR_ACCESS`; the result is all of them (`all`) or
    /// any one. An empty set is true for "all", false for "any".
    pub fn privilege_check(&self, set: &mut [(u32, u32)], all: bool) -> bool {
        let mut found = 0;
        for (luid, attrs) in set.iter_mut() {
            if self.privileges.iter().any(|&(l, a)| l == *luid && a & SE_PRIVILEGE_ENABLED != 0) {
                *attrs |= access::SE_PRIVILEGE_USED_FOR_ACCESS;
                found += 1;
            }
        }
        if all { found == set.len() } else { found > 0 }
    }

    /// The structure of `class` for a buffer at address `base`, or the
    /// status for a class that cannot be answered.
    pub fn query(&self, class: u32, base: u64) -> Result<Vec<u8>, NtStatus> {
        use class::*;
        let mut out = Vec::new();
        match class {
            TOKEN_USER => sid_and_attributes(&mut out, base, &[(&self.user, self.user_attrs)]),
            TOKEN_GROUPS => token_groups(&mut out, base, self.groups.iter().map(|(s, a)| (s, *a))),
            TOKEN_RESTRICTED_SIDS => token_groups(&mut out, base, self.restricted.iter().map(|(s, a)| (s, *a))),
            TOKEN_LOGON_SID => token_groups(&mut out, base, self.logon_sid().into_iter().map(|(s, a)| (s, *a))),
            TOKEN_CAPABILITIES | TOKEN_DEVICE_GROUPS => {
                token_groups(&mut out, base, core::iter::empty());
                out.resize(24, 0);
            }
            TOKEN_PRIVILEGES => out = token_privileges(&self.privileges),
            TOKEN_OWNER => pointer_then(&mut out, base, &self.owner.to_bytes()),
            TOKEN_PRIMARY_GROUP => pointer_then(&mut out, base, &self.primary_group.to_bytes()),
            TOKEN_DEFAULT_DACL => match &self.default_dacl {
                Some(a) => pointer_then(&mut out, base, a),
                None => out.extend_from_slice(&0u64.to_le_bytes()),
            },
            TOKEN_SOURCE => {
                out.extend_from_slice(b"Poly\0\0\0\0");
                out.extend_from_slice(&self.authentication_id.to_le_bytes());
            }
            TOKEN_TYPE => u32s(&mut out, &[self.kind]),
            TOKEN_IMPERSONATION_LEVEL => {
                if self.kind == TOKEN_PRIMARY {
                    return Err(STATUS_INVALID_INFO_CLASS);
                }
                u32s(&mut out, &[self.level]);
            }
            TOKEN_STATISTICS => {
                out.extend_from_slice(&self.id.to_le_bytes());
                out.extend_from_slice(&self.authentication_id.to_le_bytes());
                out.extend_from_slice(&i64::MAX.to_le_bytes());
                let dynamic = 0x1F4u32;
                u32s(
                    &mut out,
                    &[
                        self.kind,
                        self.level,
                        dynamic,
                        dynamic - 0x80,
                        self.groups.len() as u32,
                        self.privileges.len() as u32,
                    ],
                );
                out.extend_from_slice(&self.modified_id.to_le_bytes());
            }
            TOKEN_SESSION_ID => u32s(&mut out, &[self.session]),
            TOKEN_GROUPS_AND_PRIVILEGES => {
                // Header (56 bytes), the user and groups, the restricting SIDs, the privileges.
                let mut all: Vec<(&Sid, u32)> = alloc::vec![(&self.user, self.user_attrs)];
                all.extend(self.groups.iter().map(|(s, a)| (s, *a)));
                let rs: Vec<(&Sid, u32)> = self.restricted.iter().map(|(s, a)| (s, *a)).collect();
                let sid_len: usize = all.iter().map(|(s, _)| s.len()).sum();
                let res_len: usize = rs.iter().map(|(s, _)| s.len()).sum();
                let a_off = 56usize;
                let r_off = a_off + 16 * all.len() + sid_len;
                let p_off = r_off + 16 * rs.len() + res_len;
                u32s(&mut out, &[all.len() as u32, sid_len as u32]);
                out.extend_from_slice(&(base + a_off as u64).to_le_bytes());
                u32s(&mut out, &[rs.len() as u32, res_len as u32]);
                out.extend_from_slice(&(base + r_off as u64).to_le_bytes());
                u32s(&mut out, &[self.privileges.len() as u32, 12 * self.privileges.len() as u32]);
                out.extend_from_slice(&(base + p_off as u64).to_le_bytes());
                out.extend_from_slice(&self.authentication_id.to_le_bytes());
                sid_and_attributes(&mut out, base, &all);
                sid_and_attributes(&mut out, base, &rs);
                for &(l, a) in &self.privileges {
                    u32s(&mut out, &[l, 0, a]);
                }
            }
            TOKEN_SANDBOX_INERT => u32s(&mut out, &[u32::from(self.flags & F_SANDBOX_INERT != 0)]),
            TOKEN_AUDIT_POLICY => out.resize(31, 0),
            TOKEN_ORIGIN => out.extend_from_slice(&self.origin.to_le_bytes()),
            TOKEN_ELEVATION_TYPE => u32s(&mut out, &[self.elevation_type]),
            TOKEN_LINKED_TOKEN => return Err(STATUS_NO_SUCH_LOGON_SESSION),
            TOKEN_ELEVATION => u32s(&mut out, &[u32::from(self.elevated)]),
            TOKEN_HAS_RESTRICTIONS => out.push(u8::from(self.has_restrictions())),
            TOKEN_ACCESS_INFORMATION => {
                // The 88-byte header and the privileges it points at.
                out.extend_from_slice(&0u64.to_le_bytes());
                out.extend_from_slice(&0u64.to_le_bytes());
                out.extend_from_slice(&(base + 88).to_le_bytes());
                out.extend_from_slice(&self.authentication_id.to_le_bytes());
                u32s(&mut out, &[self.kind, self.level, self.mandatory_policy, 0x40_2000, 0, 0]);
                out.extend_from_slice(&[0; 24]);
                out.extend_from_slice(&token_privileges(&self.privileges));
            }
            TOKEN_VIRTUALIZATION_ALLOWED => u32s(&mut out, &[u32::from(self.flags & F_VIRT_ALLOWED != 0)]),
            TOKEN_VIRTUALIZATION_ENABLED => u32s(&mut out, &[u32::from(self.flags & F_VIRT_ENABLED != 0)]),
            TOKEN_INTEGRITY_LEVEL => {
                let label = sid::mandatory(self.integrity_rid());
                sid_and_attributes(&mut out, base, &[(&label, SE_GROUP_INTEGRITY | SE_GROUP_INTEGRITY_ENABLED)]);
            }
            TOKEN_UI_ACCESS => u32s(&mut out, &[u32::from(self.flags & F_UI_ACCESS != 0)]),
            TOKEN_MANDATORY_POLICY => u32s(&mut out, &[self.mandatory_policy]),
            TOKEN_IS_APP_CONTAINER
            | TOKEN_APP_CONTAINER_NUMBER
            | TOKEN_PRIVATE_NAME_SPACE
            | TOKEN_IS_SANDBOXED
            | TOKEN_IS_APP_SILO => u32s(&mut out, &[0]),
            TOKEN_APP_CONTAINER_SID => out.extend_from_slice(&0u64.to_le_bytes()),
            TOKEN_USER_CLAIM_ATTRIBUTES | TOKEN_DEVICE_CLAIM_ATTRIBUTES | TOKEN_SECURITY_ATTRIBUTES => {
                out.extend_from_slice(&[0; 16])
            }
            TOKEN_IS_RESTRICTED => u32s(&mut out, &[u32::from(self.is_restricted())]),
            TOKEN_PROCESS_TRUST_LEVEL => out.extend_from_slice(&0u64.to_le_bytes()),
            TOKEN_SINGLETON_ATTRIBUTES | TOKEN_BNO_ISOLATION => out.resize(16, 0),
            _ => return Err(STATUS_INVALID_INFO_CLASS),
        }
        Ok(out)
    }
}

/// What the caller handed to `SetTokenInformation`, resolved.
#[derive(Clone, Debug)]
pub enum SetData {
    /// A SID (`TOKEN_OWNER`, `TOKEN_PRIMARY_GROUP`, `TOKEN_MANDATORY_LABEL`);
    /// `None` for a NULL pointer.
    Sid(Option<Sid>),
    /// A default DACL; `None` for NULL.
    Acl(Option<Vec<u8>>),
    /// A `DWORD` value.
    Value(u32),
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
/// at the current end of `out`, then the SIDs; the pointers are relative to
/// `base`, the address of the whole structure's first byte.
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
    fn adjusting_privileges() {
        const E: u32 = SE_PRIVILEGE_ENABLED;
        const D: u32 = SE_PRIVILEGE_ENABLED_BY_DEFAULT;
        let held = [(18, 0), (23, E | D), (35, 0)];
        let a = adjust_privileges(&held, false, &[(18, E), (35, E)]);
        assert_eq!(a.privileges, [(18, E), (23, E | D), (35, E)]);
        assert_eq!(a.previous, [(18, 0), (35, 0)]);
        assert!(a.all_assigned);
        let b = adjust_privileges(&a.privileges, false, &[(18, E)]);
        assert_eq!(b.privileges, a.privileges);
        assert!(b.previous.is_empty() && b.all_assigned);
        let c = adjust_privileges(&held, false, &[(4, E), (18, E)]);
        assert_eq!(c.previous, [(18, 0)]);
        assert!(!c.all_assigned);
        let d = adjust_privileges(&held, false, &[(23, 0), (35, SE_PRIVILEGE_REMOVED)]);
        assert_eq!(d.privileges, [(18, 0), (23, D)]);
        assert_eq!(d.previous, [(23, E | D), (35, 0)]);
        let e = adjust_privileges(&a.privileges, true, &[(4, E)]);
        assert_eq!(e.privileges, [(18, 0), (23, D), (35, 0)]);
        assert_eq!(e.previous, [(18, E), (23, E | D), (35, E)]);
        assert!(e.all_assigned);
        let bytes = token_privileges(&held);
        assert_eq!(bytes.len(), 4 + 3 * 12);
        assert_eq!(parse_token_privileges(&bytes).unwrap(), held);
        assert_eq!(parse_token_privileges(&bytes[..20]), None);
    }

    #[test]
    fn the_default_token_is_the_oracles() {
        let t = Token::poly_user();
        assert_eq!(t.groups.len(), 12);
        assert_eq!(t.groups.iter().map(|(s, _)| s.len()).sum::<usize>(), 180);
        assert_eq!(t.privileges.len(), 24);
        assert_eq!(t.query(class::TOKEN_GROUPS, 0).unwrap().len(), 380);
        assert_eq!(t.query(class::TOKEN_PRIVILEGES, 0).unwrap().len(), 292);
        assert_eq!(t.query(class::TOKEN_DEFAULT_DACL, 0).unwrap().len(), 88);
        assert_eq!(t.query(class::TOKEN_USER, 0).unwrap().len(), 44);
        assert_eq!(t.query(class::TOKEN_OWNER, 0).unwrap().len(), 24);
        assert_eq!(t.query(class::TOKEN_PRIMARY_GROUP, 0).unwrap().len(), 36);
        assert_eq!(t.query(class::TOKEN_INTEGRITY_LEVEL, 0).unwrap().len(), 28);
        assert_eq!(t.query(class::TOKEN_LOGON_SID, 0).unwrap().len(), 44);
        assert_eq!(t.query(class::TOKEN_STATISTICS, 0).unwrap().len(), 56);
        assert_eq!(t.integrity_rid(), 0x3000);
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
    fn small_classes() {
        let t = Token::poly_user();
        assert_eq!(t.query(class::TOKEN_ELEVATION, 0).unwrap(), [1, 0, 0, 0]);
        assert_eq!(t.query(class::TOKEN_ELEVATION_TYPE, 0).unwrap(), [1, 0, 0, 0]);
        assert_eq!(t.query(class::TOKEN_TYPE, 0).unwrap(), [1, 0, 0, 0]);
        assert_eq!(t.query(class::TOKEN_SESSION_ID, 0).unwrap(), [0, 0, 0, 0]);
        assert_eq!(t.query(class::TOKEN_HAS_RESTRICTIONS, 0).unwrap(), [0]);
        assert_eq!(t.query(class::TOKEN_IMPERSONATION_LEVEL, 0), Err(STATUS_INVALID_INFO_CLASS));
        assert_eq!(t.query(999, 0), Err(STATUS_INVALID_INFO_CLASS));
        let il = t.query(class::TOKEN_INTEGRITY_LEVEL, 0x100).unwrap();
        assert_eq!(u64_at(&il, 0), 0x110);
        assert_eq!(u32_at(&il, 8), 0x60);
        assert_eq!(sddl::to_string(&Sid::from_bytes(&il[16..]).unwrap()), "S-1-16-12288");
    }

    #[test]
    fn filtering() {
        let t = Token::poly_user();
        let admins = sid::builtin(544);
        let same = t.filter(0, &[], &[], &[], 9).unwrap();
        assert!(same.same_context(&t) && !same.has_restrictions());
        let max = t.filter(DISABLE_MAX_PRIVILEGE, &[], &[], &[], 9).unwrap();
        assert_eq!(max.privileges, [(23, 3)]);
        assert!(max.has_restrictions() && !max.same_context(&t));
        let lua = t.filter(LUA_TOKEN, &[], &[], &[], 9).unwrap();
        assert_eq!(lua.privileges.len(), 5);
        assert_eq!(lua.groups[2], (admins.clone(), SE_GROUP_USE_FOR_DENY_ONLY));
        let dis = t.filter(0, &[admins.clone(), Sid::new(5, &[18])], &[], &[], 9).unwrap();
        assert_eq!(dis.groups[2].1, 0x10);
        assert!(dis.has_restrictions() && !dis.is_restricted());
        let none = t.filter(0, &[Sid::new(5, &[18])], &[], &[], 9).unwrap();
        assert!(!none.has_restrictions());
        let res = t.filter(0, &[], &[], &[(sid::world(), 0)], 9).unwrap();
        assert!(res.is_restricted());
        assert_eq!(t.filter(0, &[], &[], &[(sid::world(), 0x17)], 9), Err(SecErr::InvalidParameter));
        assert_eq!(res.filter(0, &[], &[], &[(sid::nt(12), 0)], 9), Err(SecErr::InvalidParameter));
        let del = t.filter(0, &[], &[17, 18, 7], &[], 9).unwrap();
        assert_eq!(del.privileges.len(), 22);
        let u = t.filter(0, &[t.user.clone()], &[], &[], 9).unwrap();
        assert_eq!(u.user_attrs, 0x10);
        assert!(u.has_restrictions());
    }

    #[test]
    fn groups_and_set_information() {
        let mut t = Token::poly_user();
        let admins = sid::builtin(544).to_bytes();
        assert_eq!(t.adjust_groups(false, &[(admins.clone(), 0)]), Err(STATUS_CANT_DISABLE_MANDATORY));
        let (prev, all) = t.adjust_groups(false, &[(admins.clone(), 4)]).unwrap();
        assert!(prev.is_empty() && all);
        let (_, all) = t.adjust_groups(false, &[(Sid::new(5, &[18]).to_bytes(), 4)]).unwrap();
        assert!(!all);
        let mut d = t.filter(0, &[sid::builtin(544)], &[], &[], 9).unwrap();
        assert_eq!(d.adjust_groups(false, &[(admins, 4)]), Err(STATUS_CANT_ENABLE_DENY_ONLY));
        let low = Some(sid::mandatory(0x1000));
        t.set_information(class::TOKEN_INTEGRITY_LEVEL, &SetData::Sid(low)).unwrap();
        assert_eq!(t.integrity_rid(), 0x1000);
        assert_eq!(
            t.set_information(class::TOKEN_INTEGRITY_LEVEL, &SetData::Sid(Some(sid::mandatory(0x2000)))),
            Err(SecErr::PrivilegeNotHeld.nt())
        );
        assert_eq!(
            t.set_information(class::TOKEN_INTEGRITY_LEVEL, &SetData::Sid(Some(sid::nt(18)))),
            Err(STATUS_INVALID_LABEL)
        );
        let mut u = Token::poly_user();
        assert_eq!(
            u.set_information(class::TOKEN_OWNER, &SetData::Sid(Some(sid::builtin(545)))),
            Err(SecErr::InvalidOwner.nt())
        );
        u.set_information(class::TOKEN_OWNER, &SetData::Sid(Some(u.user.clone()))).unwrap();
        assert_eq!(
            u.set_information(class::TOKEN_PRIMARY_GROUP, &SetData::Sid(Some(sid::nt(18)))),
            Err(SecErr::InvalidGroup.nt())
        );
    }

    #[test]
    fn privilege_checks() {
        let t = Token::poly_user();
        let mut set = [(23u32, 0u32), (7, 0)];
        assert!(!t.privilege_check(&mut set, true));
        assert_eq!(set, [(23, 0x8000_0000), (7, 0)]);
        assert!(t.privilege_check(&mut set, false));
        assert!(t.privilege_check(&mut [], true));
        assert!(!t.privilege_check(&mut [], false));
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
