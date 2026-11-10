//! `BuildSecurityDescriptorW` (ADR 0171): a new self-relative security
//! descriptor from trustees, access and audit entries and an old
//! descriptor, with the rules of the spec sheet
//! (specs/windows/advapi32.dll/BuildSecurityDescriptorW.yaml; `aclapi.h`,
//! `accctrl.h`).
//!
//! - The control is always `SE_DACL_PRESENT | SE_SACL_PRESENT` (and
//!   self-relative), whatever the old one had; a list with nothing to hold
//!   is present and NULL.
//! - A trustee given as owner or group replaces the old one; none keeps it.
//! - Each list is the old list merged with the entries for it. New denied
//!   entries go first. New allowed entries go behind the old list's leading
//!   denied entries, or last when the old list has a denied entry behind an
//!   allowed one. `SET_ACCESS` and `REVOKE_ACCESS` drop the trustee's old
//!   allowed entries; `GRANT_ACCESS` drops an old denied entry of the
//!   trustee for exactly those rights. Audit entries go first.
//! - An entry whose inheritance has `INHERITED_ACE` makes no entry.
//!
//! The merge differs from `SetEntriesInAclW`'s ([`crate::explicit`]), which
//! puts new allowed entries last.

use crate::acl::{self, Ace};
use crate::explicit::{
    DENY_ACCESS, ERROR_INVALID_PARAMETER, Entry, GRANT_ACCESS, REVOKE_ACCESS, SET_ACCESS, SET_AUDIT_FAILURE,
    SET_AUDIT_SUCCESS, Trustee,
};
use crate::sd::{Parts, SE_DACL_PRESENT, SE_SACL_PRESENT, SECURITY_DESCRIPTOR_REVISION};
use crate::sdops::acl_of;
use crate::sid::Sid;
use alloc::vec::Vec;

/// `INHERITED_ACE` (`winnt.h`).
const INHERITED_ACE: u32 = 0x10;

/// What the call was given, the old descriptor already read.
#[derive(Debug)]
pub struct Request<'a> {
    pub owner: Option<&'a Trustee>,
    pub group: Option<&'a Trustee>,
    pub access: &'a [Entry],
    pub audit: &'a [Entry],
    pub old: Option<&'a Parts>,
}

/// The new descriptor's parts ([`Parts::to_relative`] gives its bytes), or
/// a Win32 error. `resolve` turns a trustee into its SID (a name that is no
/// account is `ERROR_NONE_MAPPED`). Everything is checked before anything
/// is built.
pub fn build(r: &Request<'_>, resolve: &dyn Fn(&Trustee) -> Result<Sid, u32>) -> Result<Parts, u32> {
    for e in r.access.iter().chain(r.audit) {
        if !(GRANT_ACCESS..=SET_AUDIT_FAILURE).contains(&e.mode) {
            return Err(ERROR_INVALID_PARAMETER);
        }
    }
    let sid = |t: Option<&Trustee>, old: Option<&Vec<u8>>| -> Result<Option<Vec<u8>>, u32> {
        match t {
            Some(t) => Ok(Some(resolve(t)?.to_bytes())),
            None => Ok(old.cloned()),
        }
    };
    let owner = sid(r.owner, r.old.and_then(|o| o.owner.as_ref()))?;
    let group = sid(r.group, r.old.and_then(|o| o.group.as_ref()))?;
    let dacl = merge(r.access, r.old.and_then(|o| o.dacl.as_deref()), resolve)?;
    let sacl = merge(r.audit, r.old.and_then(|o| o.sacl.as_deref()), resolve)?;
    Ok(Parts {
        revision: SECURITY_DESCRIPTOR_REVISION,
        sbz1: 0,
        control: SE_DACL_PRESENT | SE_SACL_PRESENT,
        owner,
        group,
        sacl,
        dacl,
    })
}

/// One list: `None` (a NULL list) when there is neither an entry nor an old
/// list, the old list as it is when there are no entries, else the merge.
fn merge(
    entries: &[Entry],
    old: Option<&[u8]>,
    resolve: &dyn Fn(&Trustee) -> Result<Sid, u32>,
) -> Result<Option<Vec<u8>>, u32> {
    if entries.is_empty() {
        return Ok(old.map(<[u8]>::to_vec));
    }
    let mut sids = Vec::with_capacity(entries.len());
    for e in entries {
        sids.push(resolve(&e.trustee)?);
    }
    let old_aces = old.map(acl::aces_in).unwrap_or_default();
    // A denied entry behind an allowed one: new allowed entries go last.
    let first_allow = old_aces.iter().position(|a| a.kind == acl::ACCESS_ALLOWED_ACE_TYPE);
    let mixed = first_allow.is_some_and(|at| old_aces[at..].iter().any(|a| a.kind == acl::ACCESS_DENIED_ACE_TYPE));
    let revision = old.and_then(|o| o.first().copied()).unwrap_or(acl::ACL_REVISION).max(acl::ACL_REVISION);
    let mut kept: Vec<Ace> = old_aces;
    let (mut audits, mut denies, mut allows): (Vec<Ace>, Vec<Ace>, Vec<Ace>) = (Vec::new(), Vec::new(), Vec::new());
    let of = |a: &Ace, kind: u8, s: &Sid| a.kind == kind && a.sid().and_then(Sid::from_bytes).as_ref() == Some(s);
    for (e, s) in entries.iter().zip(&sids) {
        if e.inherit & INHERITED_ACE != 0 {
            continue;
        }
        let flags = (e.inherit & 0xFF) as u8;
        let sb = s.to_bytes();
        let make = |kind: u8, flags: u8| -> Result<Ace, u32> {
            let bytes = acl::ace(kind, flags, e.access, &sb).map_err(|_| ERROR_INVALID_PARAMETER)?;
            Ok(Ace { kind, flags, bytes })
        };
        match e.mode {
            GRANT_ACCESS => {
                kept.retain(|a| !(of(a, acl::ACCESS_DENIED_ACE_TYPE, s) && a.mask() == e.access));
                allows.push(make(acl::ACCESS_ALLOWED_ACE_TYPE, flags)?);
            }
            SET_ACCESS | REVOKE_ACCESS => {
                kept.retain(|a| !of(a, acl::ACCESS_ALLOWED_ACE_TYPE, s));
                allows.retain(|a| !of(a, acl::ACCESS_ALLOWED_ACE_TYPE, s));
                if e.mode == SET_ACCESS {
                    allows.push(make(acl::ACCESS_ALLOWED_ACE_TYPE, flags)?);
                }
            }
            DENY_ACCESS => denies.push(make(acl::ACCESS_DENIED_ACE_TYPE, flags)?),
            SET_AUDIT_SUCCESS => audits.push(make(acl::SYSTEM_AUDIT_ACE_TYPE, flags | 0x40)?),
            SET_AUDIT_FAILURE => audits.push(make(acl::SYSTEM_AUDIT_ACE_TYPE, flags | 0x80)?),
            _ => {}
        }
    }
    let at = if mixed { kept.len() } else { kept.iter().take_while(|a| a.kind == acl::ACCESS_DENIED_ACE_TYPE).count() };
    let tail = kept.split_off(at);
    let mut all = audits;
    all.extend(denies);
    all.extend(kept);
    all.extend(allows);
    all.extend(tail);
    Ok(Some(acl_of(revision, &all)))
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::explicit::{ERROR_NONE_MAPPED, NOT_USED_ACCESS};
    use crate::sddl;
    use std::string::String;

    const EVERYONE: &str = "S-1-1-0";
    const USERS: &str = "S-1-5-32-545";
    const SYSTEM: &str = "S-1-5-18";
    const LOCAL_SERVICE: &str = "S-1-5-19";
    const ME: &str = "S-1-5-21-1-2-3-1001";

    fn sid(s: &str) -> Sid {
        sddl::parse(&s.encode_utf16().collect::<Vec<u16>>()).unwrap()
    }

    fn resolve(t: &Trustee) -> Result<Sid, u32> {
        match t {
            Trustee::Sid(b) => Sid::from_bytes(b).ok_or(ERROR_INVALID_PARAMETER),
            Trustee::Name(n) if n == "CURRENT_USER" => Ok(sid(ME)),
            Trustee::Name(n) if n == "Everyone" => Ok(sid(EVERYONE)),
            _ => Err(ERROR_NONE_MAPPED),
        }
    }

    fn e(mode: u32, who: &str, access: u32, inherit: u32) -> Entry {
        Entry { access, mode, inherit, trustee: Trustee::Sid(sid(who).to_bytes()) }
    }

    fn ace(kind: u8, who: &str, mask: u32) -> Ace {
        Ace { kind, flags: 0, bytes: acl::ace(kind, 0, mask, &sid(who).to_bytes()).unwrap() }
    }

    /// An old descriptor with this DACL.
    fn old(aces: &[Ace]) -> Parts {
        Parts { revision: 1, control: SE_DACL_PRESENT, dacl: Some(acl_of(2, aces)), ..Parts::default() }
    }

    /// `(type, flags, mask, sid)` of a list's entries.
    fn list(acl_bytes: Option<&[u8]>) -> Vec<(u8, u8, u32, String)> {
        acl::aces_in(acl_bytes.unwrap())
            .iter()
            .map(|a| (a.kind, a.flags, a.mask(), sddl::to_string(&Sid::from_bytes(a.sid().unwrap()).unwrap())))
            .collect()
    }

    fn dacl_of(access: &[Entry], old: Option<&Parts>) -> Vec<(u8, u32, String)> {
        let r = Request { owner: None, group: None, access, audit: &[], old };
        list(build(&r, &resolve).unwrap().dacl.as_deref()).into_iter().map(|(k, _, m, s)| (k, m, s)).collect()
    }

    fn row(kind: u8, mask: u32, who: &str) -> (u8, u32, String) {
        (kind, mask, who.into())
    }

    /// The call Chrome's crash reporting makes (sheet: `chrome_call`).
    #[test]
    fn the_chrome_call() {
        let label = Ace { kind: 17, flags: 0, bytes: acl::ace(17, 0, 0, &sid("S-1-16-0").to_bytes()).unwrap() };
        let old = Parts {
            revision: 1,
            control: SE_DACL_PRESENT | SE_SACL_PRESENT,
            dacl: Some(acl_of(2, &[ace(0, SYSTEM, 0x1000_0000), ace(0, "S-1-15-2-1", 0xC000_0000)])),
            sacl: Some(acl_of(2, &[label])),
            ..Parts::default()
        };
        assert_eq!(old.to_relative().len(), 100);
        let entry =
            Entry { access: 0x1000_0000, mode: SET_ACCESS, inherit: 0, trustee: Trustee::Name("CURRENT_USER".into()) };
        let r = Request { owner: None, group: None, access: &[entry], audit: &[], old: Some(&old) };
        let new = build(&r, &resolve).unwrap();
        let bytes = new.to_relative();
        assert_eq!(bytes.len(), 136);
        assert_eq!(u16::from_le_bytes([bytes[2], bytes[3]]), 0x8014);
        assert_eq!((new.revision, &new.owner, &new.group), (1, &None, &None));
        let dacl = new.dacl.as_deref().unwrap();
        assert_eq!((dacl[0], dacl.len()), (2, 88));
        assert_eq!(
            list(Some(dacl)),
            [
                (0, 0, 0x1000_0000, ME.into()),
                (0, 0, 0x1000_0000, SYSTEM.into()),
                (0, 0, 0xC000_0000, "S-1-15-2-1".into())
            ]
        );
        assert_eq!(new.sacl, old.sacl);
        assert_eq!(new.sacl.as_ref().unwrap().len(), 28);
    }

    #[test]
    fn nothing_gives_the_header_with_two_null_lists() {
        let r = Request { owner: None, group: None, access: &[], audit: &[], old: None };
        let new = build(&r, &resolve).unwrap();
        assert_eq!((new.control, &new.dacl, &new.sacl), (0x14, &None, &None));
        assert_eq!(new.to_relative().len(), 20);
        // The old control does not survive; the old lists and SIDs do.
        let mut o = old(&[ace(0, EVERYONE, 1)]);
        o.control = 0x3C14;
        o.owner = Some(sid("S-1-5-32-544").to_bytes());
        o.group = Some(sid(SYSTEM).to_bytes());
        let r = Request { owner: None, group: None, access: &[], audit: &[], old: Some(&o) };
        let new = build(&r, &resolve).unwrap();
        assert_eq!((new.control, &new.dacl, &new.owner, &new.group), (0x14, &o.dacl, &o.owner, &o.group));
        assert_eq!(new.sacl, None);
    }

    #[test]
    fn owner_and_group_from_trustees() {
        let ba = Trustee::Sid(sid("S-1-5-32-544").to_bytes());
        let sy = Trustee::Sid(sid(SYSTEM).to_bytes());
        let size = |owner: Option<&Trustee>, group: Option<&Trustee>| {
            build(&Request { owner, group, access: &[], audit: &[], old: None }, &resolve).unwrap().to_relative().len()
        };
        assert_eq!((size(Some(&ba), None), size(None, Some(&sy)), size(Some(&ba), Some(&sy))), (36, 32, 48));
        let me = Trustee::Name("CURRENT_USER".into());
        let r = Request { owner: Some(&me), group: None, access: &[], audit: &[], old: None };
        assert_eq!(build(&r, &resolve).unwrap().owner, Some(sid(ME).to_bytes()));
        // A new owner replaces the old one and leaves the group.
        let mut o = Parts { revision: 1, ..Parts::default() };
        o.owner = Some(sid("S-1-5-32-544").to_bytes());
        o.group = Some(sid(SYSTEM).to_bytes());
        let ls = Trustee::Sid(sid(LOCAL_SERVICE).to_bytes());
        let r = Request { owner: Some(&ls), group: None, access: &[], audit: &[], old: Some(&o) };
        let new = build(&r, &resolve).unwrap();
        assert_eq!((&new.owner, &new.group), (&Some(sid(LOCAL_SERVICE).to_bytes()), &o.group));
        assert_eq!(new.to_relative().len() + 4, o.to_relative().len());
    }

    #[test]
    fn entries_without_an_old_list() {
        // Denied first, then the allowed ones in the order given; masks and flags as given.
        let r = Request {
            owner: None,
            group: None,
            access: &[e(GRANT_ACCESS, EVERYONE, 0x12_0089, 3), e(DENY_ACCESS, USERS, 2, 0)],
            audit: &[],
            old: None,
        };
        let new = build(&r, &resolve).unwrap();
        assert_eq!(list(new.dacl.as_deref()), [(1, 0, 2, USERS.into()), (0, 3, 0x12_0089, EVERYONE.into())]);
        let dacl = new.dacl.unwrap();
        assert_eq!((dacl[0], dacl.len()), (2, 8 + 24 + 20));
        // Entries for one trustee are not combined.
        let four = [
            e(GRANT_ACCESS, EVERYONE, 1, 0),
            e(GRANT_ACCESS, EVERYONE, 2, 0),
            e(SET_ACCESS, USERS, 4, 0),
            e(GRANT_ACCESS, USERS, 8, 0),
        ];
        assert_eq!(
            dacl_of(&four, None),
            [row(0, 1, EVERYONE), row(0, 2, EVERYONE), row(0, 4, USERS), row(0, 8, USERS)]
        );
        // Generic bits and a mask of 0 are stored as they are.
        assert_eq!(dacl_of(&[e(GRANT_ACCESS, EVERYONE, 0xC000_0000, 0)], None), [row(0, 0xC000_0000, EVERYONE)]);
        assert_eq!(dacl_of(&[e(SET_ACCESS, EVERYONE, 0, 0)], None), [row(0, 0, EVERYONE)]);
    }

    #[test]
    fn dropped_entries_leave_an_empty_list_not_a_null_one() {
        for entry in
            [e(GRANT_ACCESS, EVERYONE, 1, 0x10), e(GRANT_ACCESS, EVERYONE, 1, 0x1F), e(REVOKE_ACCESS, EVERYONE, 0, 0)]
        {
            let r = Request { owner: None, group: None, access: &[entry], audit: &[], old: None };
            let dacl = build(&r, &resolve).unwrap().dacl.unwrap();
            assert_eq!((dacl.len(), acl::aces_in(&dacl).len()), (8, 0));
        }
    }

    #[test]
    fn audit_entries_and_modes_in_the_other_list() {
        let r = Request {
            owner: None,
            group: None,
            access: &[e(SET_AUDIT_SUCCESS, EVERYONE, 1, 0)],
            audit: &[e(GRANT_ACCESS, EVERYONE, 2, 0), e(SET_AUDIT_FAILURE, USERS, 4, 0)],
            old: None,
        };
        let new = build(&r, &resolve).unwrap();
        assert_eq!(list(new.dacl.as_deref()), [(2, 0x40, 1, EVERYONE.into())]);
        assert_eq!(list(new.sacl.as_deref()), [(2, 0x80, 4, USERS.into()), (0, 0, 2, EVERYONE.into())]);
        // With an old SACL the new entry comes first and the old ones follow.
        let label = Ace { kind: 17, flags: 0, bytes: acl::ace(17, 0, 1, &sid("S-1-16-4096").to_bytes()).unwrap() };
        let audit = Ace { kind: 2, flags: 0x40, bytes: acl::ace(2, 0x40, 1, &sid(EVERYONE).to_bytes()).unwrap() };
        let o =
            Parts { revision: 1, control: SE_SACL_PRESENT, sacl: Some(acl_of(2, &[audit, label])), ..Parts::default() };
        let r = Request {
            owner: None,
            group: None,
            access: &[],
            audit: &[e(SET_AUDIT_FAILURE, USERS, 4, 0)],
            old: Some(&o),
        };
        let kinds: Vec<(u8, u8)> =
            list(build(&r, &resolve).unwrap().sacl.as_deref()).iter().map(|x| (x.0, x.1)).collect();
        assert_eq!(kinds, [(2, 0x80), (2, 0x40), (17, 0)]);
    }

    /// Sheet: `merge_allow` and `merge_deny`.
    #[test]
    fn where_a_new_entry_goes_in_an_old_list() {
        let (a1, a2, d) = (ace(0, SYSTEM, 1), ace(0, USERS, 2), ace(1, LOCAL_SERVICE, 4));
        let grant = [e(GRANT_ACCESS, EVERYONE, 8, 0)];
        let new = row(0, 8, EVERYONE);
        let (r1, r2, rd) = (row(0, 1, SYSTEM), row(0, 2, USERS), row(1, 4, LOCAL_SERVICE));
        let o = old(&[a1.clone(), a2.clone()]);
        assert_eq!(dacl_of(&grant, Some(&o)), [new.clone(), r1.clone(), r2.clone()]);
        let o = old(&[d.clone(), a1.clone(), a2.clone()]);
        assert_eq!(dacl_of(&grant, Some(&o)), [rd.clone(), new.clone(), r1.clone(), r2.clone()]);
        let o = old(&[a1.clone(), d.clone(), a2.clone()]);
        assert_eq!(dacl_of(&grant, Some(&o)), [r1.clone(), rd.clone(), r2.clone(), new.clone()]);
        let o = old(&[a1.clone(), a2.clone(), d.clone()]);
        assert_eq!(dacl_of(&grant, Some(&o)), [r1.clone(), r2.clone(), rd.clone(), new.clone()]);
        // A denied entry is first in every case.
        let deny = [e(DENY_ACCESS, EVERYONE, 8, 0)];
        let newd = row(1, 8, EVERYONE);
        for order in [[&a1, &d, &a2], [&d, &a1, &a2], [&a1, &a2, &d]] {
            let o = old(&order.map(Clone::clone));
            assert_eq!(dacl_of(&deny, Some(&o))[0], newd);
            assert_eq!(dacl_of(&deny, Some(&o)).len(), 4);
        }
    }

    /// Sheet: `merge_same_trustee`, `revoke`, `several_entries`.
    #[test]
    fn entries_for_a_trustee_the_old_list_names() {
        const GR: u32 = 0x8000_0000;
        const GW: u32 = 0x4000_0000;
        const GA: u32 = 0x1000_0000;
        // GRANT adds a second entry.
        let o = old(&[ace(0, EVERYONE, GR)]);
        assert_eq!(dacl_of(&[e(GRANT_ACCESS, EVERYONE, 2, 0)], Some(&o)), [row(0, 2, EVERYONE), row(0, GR, EVERYONE)]);
        // SET replaces the old allowed entry, at the place a new one goes.
        let o = old(&[ace(0, EVERYONE, GR), ace(1, USERS, GW), ace(0, SYSTEM, GA)]);
        assert_eq!(
            dacl_of(&[e(SET_ACCESS, EVERYONE, 2, 0)], Some(&o)),
            [row(1, GW, USERS), row(0, GA, SYSTEM), row(0, 2, EVERYONE)]
        );
        let o = old(&[ace(1, USERS, GW), ace(0, EVERYONE, GR), ace(0, SYSTEM, GA)]);
        assert_eq!(
            dacl_of(&[e(SET_ACCESS, EVERYONE, 2, 0)], Some(&o)),
            [row(1, GW, USERS), row(0, 2, EVERYONE), row(0, GA, SYSTEM)]
        );
        // GRANT of exactly the denied rights removes the denied entry.
        let o = old(&[ace(1, USERS, GW), ace(0, SYSTEM, GA)]);
        assert_eq!(dacl_of(&[e(GRANT_ACCESS, USERS, GW, 0)], Some(&o)), [row(0, GW, USERS), row(0, GA, SYSTEM)]);
        // A denied entry beside an allowed one of the same trustee: both stay.
        let o = old(&[ace(0, EVERYONE, GR)]);
        assert_eq!(dacl_of(&[e(DENY_ACCESS, EVERYONE, GR, 0)], Some(&o)), [row(1, GR, EVERYONE), row(0, GR, EVERYONE)]);
        // REVOKE removes allowed entries and leaves denied ones.
        let o = old(&[ace(0, EVERYONE, GR), ace(1, USERS, GW), ace(0, SYSTEM, GA)]);
        assert_eq!(dacl_of(&[e(REVOKE_ACCESS, EVERYONE, 0, 0)], Some(&o)), [row(1, GW, USERS), row(0, GA, SYSTEM)]);
        assert_eq!(dacl_of(&[e(REVOKE_ACCESS, USERS, 0, 0)], Some(&o)).len(), 3);
        // Several at once.
        let three = [e(REVOKE_ACCESS, USERS, 0, 0), e(SET_ACCESS, SYSTEM, 1, 0), e(DENY_ACCESS, LOCAL_SERVICE, 4, 0)];
        assert_eq!(
            dacl_of(&three, Some(&o)),
            [row(1, 4, LOCAL_SERVICE), row(0, GR, EVERYONE), row(1, GW, USERS), row(0, 1, SYSTEM)]
        );
    }

    #[test]
    fn errors_before_anything_is_built() {
        let bad = |access: &[Entry], owner: Option<&Trustee>| {
            build(&Request { owner, group: None, access, audit: &[], old: None }, &resolve).unwrap_err()
        };
        assert_eq!(bad(&[e(NOT_USED_ACCESS, EVERYONE, 1, 0)], None), ERROR_INVALID_PARAMETER);
        assert_eq!(bad(&[e(7, EVERYONE, 1, 0)], None), ERROR_INVALID_PARAMETER);
        let nobody = Trustee::Name("PolyNoSuchAccount".into());
        assert_eq!(bad(&[], Some(&nobody)), ERROR_NONE_MAPPED);
        let entry = Entry { access: 1, mode: GRANT_ACCESS, inherit: 0, trustee: nobody };
        assert_eq!(bad(&[entry], None), ERROR_NONE_MAPPED);
    }
}
