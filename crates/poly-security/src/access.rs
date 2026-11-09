//! The access check (`AccessCheck`, `NtAccessCheck`, and every object open
//! that checks a token against a security descriptor).
//!
//! The model is Windows' (Learn, "How AccessCheck Works", "Mandatory
//! Integrity Control", "Access Control Lists") and what the spec sheets
//! observed:
//!
//! 1. Privileges first: `WRITE_OWNER` with `SeTakeOwnershipPrivilege`
//!    enabled, `ACCESS_SYSTEM_SECURITY` with `SeSecurityPrivilege`; they are
//!    reported as used.
//! 2. The owner implicitly has `READ_CONTROL` and `WRITE_DAC` unless the DACL
//!    has an `OWNER RIGHTS` ACE.
//! 3. A descriptor without a DACL, or with a NULL DACL, grants everything;
//!    an empty DACL grants nothing.
//! 4. Otherwise the ACEs are walked in order: an allow ACE grants its mask
//!    for a SID the token has enabled (not deny-only), a deny ACE refuses its
//!    mask for a SID the token has enabled or deny-only; the first ACE
//!    decides a bit.
//! 5. A token with restricting SIDs has to pass a second walk with the
//!    restricting SIDs in place of its groups (only for write access if the
//!    token is write-restricted).
//! 6. Mandatory integrity: a token below the object's label (Medium with
//!    no-write-up if there is none) loses what the label's policy covers.
//!
//! A request is all or nothing, except `MAXIMUM_ALLOWED`.

use crate::acl;
use crate::err::SecErr;
use crate::sd::{self, Parts};
use crate::sid::{self, Sid};
use crate::token::{SE_GROUP_ENABLED, SE_GROUP_USE_FOR_DENY_ONLY, SE_PRIVILEGE_ENABLED};
use alloc::vec::Vec;

pub const DELETE: u32 = 0x0001_0000;
pub const READ_CONTROL: u32 = 0x0002_0000;
pub const WRITE_DAC: u32 = 0x0004_0000;
pub const WRITE_OWNER: u32 = 0x0008_0000;
pub const SYNCHRONIZE: u32 = 0x0010_0000;
pub const ACCESS_SYSTEM_SECURITY: u32 = 0x0100_0000;
pub const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
pub const GENERIC_ALL: u32 = 0x1000_0000;
pub const GENERIC_EXECUTE: u32 = 0x2000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_BITS: u32 = GENERIC_ALL | GENERIC_EXECUTE | GENERIC_WRITE | GENERIC_READ;
/// `SeSecurityPrivilege`, `SeTakeOwnershipPrivilege`.
pub const SE_SECURITY_PRIVILEGE: u32 = 8;
pub const SE_TAKE_OWNERSHIP_PRIVILEGE: u32 = 9;
/// `SE_PRIVILEGE_USED_FOR_ACCESS` (`winnt.h`).
pub const SE_PRIVILEGE_USED_FOR_ACCESS: u32 = 0x8000_0000;

/// `GENERIC_MAPPING`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct GenericMapping {
    pub read: u32,
    pub write: u32,
    pub execute: u32,
    pub all: u32,
}

impl GenericMapping {
    /// `RtlMapGenericMask`: each generic bit set is replaced by its field,
    /// checked one after the other (read, write, execute, all) with the
    /// bits cleared at the end, so a field that itself holds a later
    /// generic bit is expanded, and none is left.
    pub fn map(&self, mask: u32) -> u32 {
        let mut m = mask;
        for (bit, field) in [
            (GENERIC_READ, self.read),
            (GENERIC_WRITE, self.write),
            (GENERIC_EXECUTE, self.execute),
            (GENERIC_ALL, self.all),
        ] {
            if m & bit != 0 {
                m |= field;
            }
        }
        m & !GENERIC_BITS
    }
}

/// The part of a token the check looks at.
#[derive(Clone, Debug)]
pub struct CheckToken<'a> {
    pub user: &'a Sid,
    pub user_attrs: u32,
    pub groups: &'a [(Sid, u32)],
    pub restricted: &'a [(Sid, u32)],
    pub privileges: &'a [(u32, u32)],
    /// The integrity level (the label SID's last sub-authority).
    pub integrity: u32,
    pub write_restricted: bool,
}

/// The mandatory label's `RID` of an untrusted, medium token...: the
/// default label of an object without one.
const DEFAULT_LABEL: u32 = sid::rid::SECURITY_MANDATORY_MEDIUM_RID;

/// How a check ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// What was granted: the requested bits, or with `MAXIMUM_ALLOWED` all
    /// that could be; 0 when denied.
    pub granted: u32,
    /// `Ok` or the reason it was refused ([`SecErr::AccessDenied`], or
    /// [`SecErr::PrivilegeNotHeld`] for `ACCESS_SYSTEM_SECURITY`).
    pub status: Result<(), SecErr>,
    /// The privileges used (LUIDs), in the order they were used.
    pub privileges: Vec<u32>,
}

impl Outcome {
    fn denied(status: SecErr, privileges: Vec<u32>) -> Outcome {
        Outcome { granted: 0, status: Err(status), privileges }
    }
}

fn has_privilege(t: &CheckToken<'_>, luid: u32) -> bool {
    t.privileges.iter().any(|&(l, a)| l == luid && a & SE_PRIVILEGE_ENABLED != 0)
}

/// Whether `sid` is one the token can use for an allow ACE (`allow`) or a
/// deny ACE.
fn token_has(t: &CheckToken<'_>, sid: &Sid, allow: bool) -> bool {
    let usable = |attrs: u32| {
        if allow {
            attrs & SE_GROUP_ENABLED != 0 && attrs & SE_GROUP_USE_FOR_DENY_ONLY == 0
        } else {
            attrs & (SE_GROUP_ENABLED | SE_GROUP_USE_FOR_DENY_ONLY) != 0
        }
    };
    if t.user == sid && (!allow || t.user_attrs & SE_GROUP_USE_FOR_DENY_ONLY == 0) {
        return true;
    }
    t.groups.iter().any(|(g, a)| g == sid && usable(*a))
}

/// The same for the restricting SIDs (all are enabled).
fn restricted_has(t: &CheckToken<'_>, sid: &Sid) -> bool {
    t.restricted.iter().any(|(g, a)| g == sid && a & SE_GROUP_ENABLED != 0)
}

/// `OWNER RIGHTS`, `S-1-3-4`.
fn owner_rights() -> Sid {
    Sid::new(sid::SECURITY_CREATOR_SID_AUTHORITY, &[sid::rid::SECURITY_CREATOR_OWNER_RIGHTS_RID])
}

/// The label of a descriptor's SACL: (integrity RID, policy).
fn object_label(sacl: Option<&[u8]>) -> (u32, u32) {
    let Some(b) = sacl else { return (DEFAULT_LABEL, acl::SYSTEM_MANDATORY_LABEL_NO_WRITE_UP) };
    for a in acl::aces_in(b) {
        if a.kind == acl::SYSTEM_MANDATORY_LABEL_ACE_TYPE {
            if let Some(s) = a.sid().and_then(Sid::from_bytes) {
                return (s.rid().unwrap_or(0), a.mask() & 7);
            }
        }
    }
    (DEFAULT_LABEL, acl::SYSTEM_MANDATORY_LABEL_NO_WRITE_UP)
}

/// One pass over the DACL. `matches(sid, allow)` says whether the pass's
/// principals include the SID; `owner_matches` whether they include the
/// descriptor's owner. Returns the bits granted when `max`, else whether
/// every bit of `want` was.
fn walk(
    aces: &[acl::Ace],
    want: u32,
    max: bool,
    owner_matches: bool,
    owner_rights_ace: bool,
    matches: &dyn Fn(&Sid, bool) -> bool,
) -> u32 {
    let mut granted = if owner_matches && !owner_rights_ace { READ_CONTROL | WRITE_DAC } else { 0 };
    let mut denied = 0u32;
    for a in aces {
        if a.flags & acl::INHERIT_ONLY_ACE != 0 {
            continue;
        }
        let allow = match a.kind {
            acl::ACCESS_ALLOWED_ACE_TYPE => true,
            acl::ACCESS_DENIED_ACE_TYPE => false,
            _ => continue,
        };
        let Some(s) = a.sid().and_then(Sid::from_bytes) else { continue };
        let hit = if s == owner_rights() { owner_matches } else { matches(&s, allow) };
        if !hit {
            continue;
        }
        if allow {
            granted |= a.mask() & !denied;
        } else {
            denied |= a.mask() & !granted;
        }
        if !max && want & !granted == 0 {
            break;
        }
    }
    granted
}

/// The check. `sd` has an owner and a group (the caller checked), `desired`
/// has no generic bits.
pub fn check(sd: &Parts, token: &CheckToken<'_>, desired: u32, mapping: &GenericMapping) -> Outcome {
    let mut used: Vec<u32> = Vec::new();
    if desired == 0 {
        return Outcome::denied(SecErr::AccessDenied, used);
    }
    let max = desired & MAXIMUM_ALLOWED != 0;
    let mut want = desired & !MAXIMUM_ALLOWED;
    // Privileges.
    let mut priv_granted = 0u32;
    if want & WRITE_OWNER != 0 && has_privilege(token, SE_TAKE_OWNERSHIP_PRIVILEGE) {
        priv_granted |= WRITE_OWNER;
        used.push(SE_TAKE_OWNERSHIP_PRIVILEGE);
    }
    if want & ACCESS_SYSTEM_SECURITY != 0 {
        if has_privilege(token, SE_SECURITY_PRIVILEGE) {
            priv_granted |= ACCESS_SYSTEM_SECURITY;
            used.push(SE_SECURITY_PRIVILEGE);
        } else {
            return Outcome::denied(SecErr::PrivilegeNotHeld, used);
        }
    }
    want &= !priv_granted;

    let dacl_present = sd.control & sd::SE_DACL_PRESENT != 0;
    let dacl_bytes = if dacl_present { sd.dacl.as_deref() } else { None };
    let everything = !dacl_present || dacl_bytes.is_none();
    let aces: Vec<acl::Ace> = match dacl_bytes {
        Some(b) => acl::aces_in(b),
        None => Vec::new(),
    };
    let owner = sd.owner.as_deref().and_then(Sid::from_bytes);
    let owner_rights_ace = aces.iter().any(|a| a.sid().and_then(Sid::from_bytes).is_some_and(|s| s == owner_rights()));

    let mut granted;
    if everything {
        granted = if max { mapping.all } else { want };
    } else {
        let owner_ok = owner.as_ref().is_some_and(|o| token_has(token, o, true));
        granted = walk(&aces, want, max, owner_ok, owner_rights_ace, &|s, allow| token_has(token, s, allow));
    }
    // The second pass for restricting SIDs.
    if !token.restricted.is_empty() {
        let write_bits = mapping.write | DELETE | WRITE_DAC | WRITE_OWNER;
        let second = if everything {
            u32::MAX
        } else {
            let owner_ok = owner.as_ref().is_some_and(|o| restricted_has(token, o));
            walk(&aces, want, max, owner_ok, owner_rights_ace, &|s, _| restricted_has(token, s))
        };
        if token.write_restricted {
            granted = (granted & !write_bits) | (granted & write_bits & second);
        } else {
            granted &= second;
        }
    }
    // Mandatory integrity control.
    let (label, policy) = object_label(sd.sacl.as_deref().filter(|_| sd.control & sd::SE_SACL_PRESENT != 0));
    if token.integrity < label {
        let mut allowed = mapping.read | mapping.execute | READ_CONTROL | SYNCHRONIZE;
        if policy & acl::SYSTEM_MANDATORY_LABEL_NO_READ_UP != 0 {
            allowed &= !(mapping.read | READ_CONTROL);
        }
        if policy & acl::SYSTEM_MANDATORY_LABEL_NO_EXECUTE_UP != 0 {
            allowed &= !(mapping.execute | SYNCHRONIZE);
        }
        granted &= allowed;
    }
    granted &= !MAXIMUM_ALLOWED;
    let all = if max { granted | priv_granted } else { granted | priv_granted };
    if max {
        // The other requested bits all have to be among them.
        if want & !granted != 0 || granted == 0 && priv_granted == 0 {
            return Outcome::denied(SecErr::AccessDenied, used);
        }
        // MAXIMUM_ALLOWED does not add privileges.
        return Outcome { granted, status: Ok(()), privileges: used };
    }
    if want & !granted != 0 {
        return Outcome::denied(SecErr::AccessDenied, used);
    }
    Outcome { granted: all & (desired | priv_granted) & !MAXIMUM_ALLOWED, status: Ok(()), privileges: used }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sid::{builtin, mandatory, world};
    use alloc::vec;

    const MAP: GenericMapping = GenericMapping { read: 1, write: 2, execute: 4, all: 7 };

    fn token<'a>(
        user: &'a Sid,
        groups: &'a [(Sid, u32)],
        restricted: &'a [(Sid, u32)],
        integrity: u32,
    ) -> CheckToken<'a> {
        CheckToken { user, user_attrs: 0, groups, restricted, privileges: &[], integrity, write_restricted: false }
    }

    fn sd_with(owner: &Sid, dacl: Option<Vec<u8>>, sacl: Option<Vec<u8>>) -> Parts {
        let mut control = sd::SE_DACL_PRESENT;
        if sacl.is_some() {
            control |= sd::SE_SACL_PRESENT;
        }
        Parts {
            revision: 1,
            sbz1: 0,
            control,
            owner: Some(owner.to_bytes()),
            group: Some(owner.to_bytes()),
            sacl,
            dacl,
        }
    }

    #[test]
    fn dacl_order_and_maximum() {
        let user = Sid::new(5, &[21, 1, 2, 3, 1000]);
        let on = SE_GROUP_ENABLED;
        let groups = vec![(world(), on), (builtin(544), on)];
        let t = token(&user, &groups, &[], 0x3000);
        let owner = Sid::new(5, &[99, 1]);
        let a = acl::build(2, &[(0, 0, 7, &world()), (1, 0, 2, &builtin(544))]);
        let o = check(&sd_with(&owner, Some(a), None), &t, 2, &MAP);
        assert_eq!(o.granted, 2);
        let a = acl::build(2, &[(1, 0, 2, &builtin(544)), (0, 0, 7, &world())]);
        let sdp = sd_with(&owner, Some(a), None);
        assert_eq!(check(&sdp, &t, 2, &MAP).status, Err(SecErr::AccessDenied));
        assert_eq!(check(&sdp, &t, 1, &MAP).granted, 1);
        assert_eq!(check(&sdp, &t, 3, &MAP).granted, 0);
        assert_eq!(check(&sdp, &t, MAXIMUM_ALLOWED, &MAP).granted, 5);
        // An empty DACL grants nothing; the owner has the implicit rights.
        let empty = sd_with(&owner, Some(acl::build(2, &[])), None);
        assert_eq!(check(&empty, &t, MAXIMUM_ALLOWED, &MAP).status, Err(SecErr::AccessDenied));
        let mine = sd_with(&user, Some(acl::build(2, &[])), None);
        assert_eq!(check(&mine, &t, MAXIMUM_ALLOWED, &MAP).granted, READ_CONTROL | WRITE_DAC);
        // OWNER RIGHTS replaces them.
        let ow = acl::build(2, &[(0, 0, 1, &owner_rights())]);
        let mine = sd_with(&user, Some(ow), None);
        assert_eq!(check(&mine, &t, MAXIMUM_ALLOWED, &MAP).granted, 1);
        // A NULL DACL grants everything.
        let null = sd_with(&owner, None, None);
        assert_eq!(check(&null, &t, 0x1f0007, &MAP).granted, 0x1f0007);
    }

    #[test]
    fn deny_only_and_restricted() {
        let user = Sid::new(5, &[21, 1, 2, 3, 1000]);
        let groups = vec![
            (world(), SE_GROUP_ENABLED),
            (builtin(544), SE_GROUP_USE_FOR_DENY_ONLY),
            (builtin(545), SE_GROUP_ENABLED),
        ];
        let t = token(&user, &groups, &[], 0x3000);
        let owner = Sid::new(5, &[99, 1]);
        let a = acl::build(2, &[(1, 0, 2, &builtin(544)), (0, 0, 7, &world())]);
        assert_eq!(check(&sd_with(&owner, Some(a), None), &t, MAXIMUM_ALLOWED, &MAP).granted, 5);
        let a = acl::build(2, &[(0, 0, 1, &builtin(545)), (0, 0, 2, &world()), (0, 0, 4, &builtin(544))]);
        let sdp = sd_with(&owner, Some(a), None);
        assert_eq!(check(&sdp, &t, MAXIMUM_ALLOWED, &MAP).granted, 3);
        // Restricting SIDs: both lists have to grant.
        let a = acl::build(2, &[(0, 0, 1, &builtin(545)), (0, 0, 2, &world()), (0, 0, 4, &builtin(544))]);
        let plain =
            vec![(world(), SE_GROUP_ENABLED), (builtin(545), SE_GROUP_ENABLED), (builtin(544), SE_GROUP_ENABLED)];
        let r = vec![(world(), 7u32)];
        let tr = token(&user, &plain, &r, 0x3000);
        let sdp = sd_with(&owner, Some(a.clone()), None);
        assert_eq!(check(&sdp, &tr, MAXIMUM_ALLOWED, &MAP).granted, 2);
        let r2 = vec![(Sid::new(5, &[12]), 7u32)];
        let tr2 = token(&user, &plain, &r2, 0x3000);
        assert_eq!(check(&sdp, &tr2, MAXIMUM_ALLOWED, &MAP).status, Err(SecErr::AccessDenied));
    }

    #[test]
    fn integrity() {
        let user = Sid::new(5, &[21, 1, 2, 3, 1000]);
        let groups = vec![(world(), SE_GROUP_ENABLED)];
        let owner = Sid::new(5, &[99, 1]);
        let all = acl::build(2, &[(0, 0, 0x1f0007, &world())]);
        let low = token(&user, &groups, &[], 0x1000);
        let med = token(&user, &groups, &[], 0x2000);
        let plain = sd_with(&owner, Some(all.clone()), None);
        assert_eq!(check(&plain, &med, MAXIMUM_ALLOWED, &MAP).granted, 0x1f0007);
        assert_eq!(check(&plain, &low, MAXIMUM_ALLOWED, &MAP).granted, 0x120005);
        let label = |rid: u32, policy: u32| acl::build(3, &[(0x11, 0, policy, &mandatory(rid))]);
        let nw_high = sd_with(&owner, Some(all.clone()), Some(label(0x3000, 1)));
        assert_eq!(check(&nw_high, &med, MAXIMUM_ALLOWED, &MAP).granted, 0x120005);
        let nr = sd_with(&owner, Some(all.clone()), Some(label(0x3000, 3)));
        assert_eq!(check(&nr, &med, MAXIMUM_ALLOWED, &MAP).granted, 0x100004);
        let nx = sd_with(&owner, Some(all.clone()), Some(label(0x3000, 5)));
        assert_eq!(check(&nx, &med, MAXIMUM_ALLOWED, &MAP).granted, 0x20001);
        let nall = sd_with(&owner, Some(all), Some(label(0x3000, 7)));
        assert_eq!(check(&nall, &med, MAXIMUM_ALLOWED, &MAP).status, Err(SecErr::AccessDenied));
    }

    #[test]
    fn privileges_and_generic_mapping() {
        let user = Sid::new(5, &[21, 1, 2, 3, 1000]);
        let groups = vec![(world(), SE_GROUP_ENABLED)];
        let owner = Sid::new(5, &[99, 1]);
        let privs = [(9u32, SE_PRIVILEGE_ENABLED), (8, SE_PRIVILEGE_ENABLED)];
        let t = CheckToken { privileges: &privs, ..token(&user, &groups, &[], 0x3000) };
        let sdp = sd_with(&owner, Some(acl::build(2, &[])), None);
        let o = check(&sdp, &t, WRITE_OWNER | ACCESS_SYSTEM_SECURITY, &MAP);
        assert_eq!(o.granted, WRITE_OWNER | ACCESS_SYSTEM_SECURITY);
        assert_eq!(o.privileges, [9, 8]);
        let t2 = token(&user, &groups, &[], 0x3000);
        assert_eq!(check(&sdp, &t2, ACCESS_SYSTEM_SECURITY, &MAP).status, Err(SecErr::PrivilegeNotHeld));
        assert_eq!(check(&sdp, &t2, WRITE_OWNER, &MAP).status, Err(SecErr::AccessDenied));
        // RtlMapGenericMask: nested expansion and the odd mappings.
        let m = GenericMapping { read: GENERIC_WRITE | 1, write: 2, execute: 4, all: 7 };
        assert_eq!(m.map(GENERIC_READ), 3);
        assert_eq!(m.map(GENERIC_ALL), 7);
        let all_g = GenericMapping { read: GENERIC_ALL, write: GENERIC_ALL, execute: GENERIC_ALL, all: GENERIC_ALL };
        assert_eq!(all_g.map(0x8000_0010), 0x10);
        let f = GenericMapping { read: 0x120089, write: 0x120116, execute: 0x1200a0, all: 0x1f01ff };
        assert_eq!(f.map(0xc000_0000), 0x12019f);
        assert_eq!(f.map(0xFFFF_FFFF), 0x0FFF_FFFF | 0x1f01ff);
        let ff = GenericMapping { read: u32::MAX, write: u32::MAX, execute: u32::MAX, all: u32::MAX };
        assert_eq!(ff.map(GENERIC_READ), 0x0FFF_FFFF);
    }
}
