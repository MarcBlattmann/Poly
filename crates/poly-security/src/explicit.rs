//! `SetEntriesInAclW` and `GetExplicitEntriesFromAclW` (ADR 0171): ACLs
//! from `EXPLICIT_ACCESS_W` entries and back, with the rules the spec
//! sheets observed (`accctrl.h` layouts).
//!
//! - New allow ACEs go to the end, new deny ACEs and audit ACEs in front,
//!   the old ACEs keep their order (the old ACL is not put in canonical
//!   order); `SET_ACCESS` and `REVOKE_ACCESS` drop the trustee's old allow
//!   ACEs; an ACE is added even for a trustee that has one.
//! - The new ACL has the old one's size (less what a removal freed), else
//!   exactly what it needs.

use crate::account;
use crate::acl::{self, Ace};
use crate::sdops::acl_of;
use crate::sid::Sid;
use alloc::string::String;
use alloc::vec::Vec;

pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_NONE_MAPPED: u32 = 1332;

/// `ACCESS_MODE` (`accctrl.h`).
pub const NOT_USED_ACCESS: u32 = 0;
pub const GRANT_ACCESS: u32 = 1;
pub const SET_ACCESS: u32 = 2;
pub const DENY_ACCESS: u32 = 3;
pub const REVOKE_ACCESS: u32 = 4;
pub const SET_AUDIT_SUCCESS: u32 = 5;
pub const SET_AUDIT_FAILURE: u32 = 6;

/// The trustee of an entry.
#[derive(Clone, Debug)]
pub enum Trustee {
    /// `TRUSTEE_IS_SID`: the bytes at the pointer (to the SID's length).
    Sid(Vec<u8>),
    /// `TRUSTEE_IS_NAME`.
    Name(String),
    /// Any other form (`TRUSTEE_BAD_FORM`, objects forms, garbage).
    Bad,
}

/// One `EXPLICIT_ACCESS_W`.
#[derive(Clone, Debug)]
pub struct Entry {
    pub access: u32,
    pub mode: u32,
    pub inherit: u32,
    pub trustee: Trustee,
}

/// Resolve an entry's trustee to a SID.
fn sid_of(t: &Trustee) -> Result<Sid, u32> {
    match t {
        Trustee::Sid(b) => Sid::from_bytes(b).ok_or(ERROR_INVALID_PARAMETER),
        Trustee::Name(n) => account::lookup_name(n).map(|(s, _, _)| s).ok_or(ERROR_NONE_MAPPED),
        Trustee::Bad => Err(ERROR_INVALID_PARAMETER),
    }
}

/// `SetEntriesInAclW`: the new ACL, or `None` for no ACL (no entries and no
/// old ACL), or a Win32 error.
pub fn set_entries(entries: &[Entry], old: Option<&[u8]>) -> Result<Option<Vec<u8>>, u32> {
    // Everything is checked before anything is built.
    let mut resolved = Vec::with_capacity(entries.len());
    for e in entries {
        if !(GRANT_ACCESS..=SET_AUDIT_FAILURE).contains(&e.mode) || e.inherit & !0x1F != 0 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        resolved.push(sid_of(&e.trustee)?);
    }
    if entries.is_empty() && old.is_none() {
        return Ok(None);
    }
    let old_aces = old.map(acl::aces_in).unwrap_or_default();
    let old_size = old.map_or(0, |o| usize::from(u16::from_le_bytes([o[2], o[3]])));
    let rev = old.and_then(|o| o.first().copied()).unwrap_or(acl::ACL_REVISION).max(acl::ACL_REVISION);
    let mut kept: Vec<Option<Ace>> = old_aces.into_iter().map(Some).collect();
    let (mut audits, mut denies, mut allows): (Vec<Ace>, Vec<Ace>, Vec<Ace>) = (Vec::new(), Vec::new(), Vec::new());
    let mut removed = 0usize;
    let mut added = 0usize;
    let is_allow_of = |a: &Ace, s: &Sid| {
        a.kind == acl::ACCESS_ALLOWED_ACE_TYPE && a.sid().and_then(Sid::from_bytes).as_ref() == Some(s)
    };
    for (e, s) in entries.iter().zip(&resolved) {
        // An inherited-flag entry makes no ACE (and changes nothing).
        if e.inherit & 0x10 != 0 {
            continue;
        }
        let flags = (e.inherit & 0xF) as u8;
        if matches!(e.mode, SET_ACCESS | REVOKE_ACCESS) {
            for slot in kept.iter_mut() {
                if slot.as_ref().is_some_and(|a| is_allow_of(a, s)) {
                    removed += slot.take().map_or(0, |a| a.bytes.len());
                }
            }
            allows.retain(|a| !is_allow_of(a, s));
        }
        let sb = s.to_bytes();
        let make = |kind: u8, flags: u8| -> Result<Ace, u32> {
            let bytes = acl::ace(kind, flags, e.access, &sb).map_err(|_| ERROR_INVALID_PARAMETER)?;
            Ok(Ace { kind, flags, bytes })
        };
        match e.mode {
            GRANT_ACCESS | SET_ACCESS => {
                let a = make(acl::ACCESS_ALLOWED_ACE_TYPE, flags)?;
                added += a.bytes.len();
                allows.push(a);
            }
            DENY_ACCESS => {
                let a = make(acl::ACCESS_DENIED_ACE_TYPE, flags)?;
                added += a.bytes.len();
                denies.push(a);
            }
            SET_AUDIT_SUCCESS | SET_AUDIT_FAILURE => {
                let bit = if e.mode == SET_AUDIT_SUCCESS { 0x40 } else { 0x80 };
                let a = make(acl::SYSTEM_AUDIT_ACE_TYPE, flags | bit)?;
                added += a.bytes.len();
                audits.push(a);
            }
            _ => {}
        }
    }
    let mut all: Vec<Ace> = Vec::new();
    all.extend(audits);
    all.extend(denies);
    all.extend(kept.into_iter().flatten());
    all.extend(allows);
    let mut out = acl_of(rev, &all);
    if old.is_some() {
        let want = old_size.saturating_sub(removed.saturating_sub(added));
        if want > out.len() {
            out[2..4].copy_from_slice(&(want.min(0xFFFF) as u16).to_le_bytes());
            out.resize(want.min(0xFFFF), 0);
        }
    }
    Ok(Some(out))
}

/// One entry of `GetExplicitEntriesFromAclW`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Explicit {
    pub access: u32,
    pub mode: u32,
    pub inherit: u32,
    pub sid: Vec<u8>,
}

/// `GetExplicitEntriesFromAclW`: one entry per allow and deny ACE and two
/// per audit ACE, none for inherited ACEs; a mandatory label (or any other
/// kind of ACE) is `ERROR_INVALID_PARAMETER`.
pub fn explicit_entries(acl_bytes: &[u8]) -> Result<Vec<Explicit>, u32> {
    let mut out = Vec::new();
    for a in acl::aces_in(acl_bytes) {
        if a.flags & 0x10 != 0 {
            continue;
        }
        let sid = a.sid().map(<[u8]>::to_vec).ok_or(ERROR_INVALID_PARAMETER)?;
        let inherit = u32::from(a.flags & 0xF);
        match a.kind {
            acl::ACCESS_ALLOWED_ACE_TYPE => out.push(Explicit { access: a.mask(), mode: GRANT_ACCESS, inherit, sid }),
            acl::ACCESS_DENIED_ACE_TYPE => out.push(Explicit { access: a.mask(), mode: DENY_ACCESS, inherit, sid }),
            acl::SYSTEM_AUDIT_ACE_TYPE => {
                for mode in [SET_AUDIT_SUCCESS, SET_AUDIT_FAILURE] {
                    out.push(Explicit { access: a.mask(), mode, inherit, sid: sid.clone() });
                }
            }
            _ => return Err(ERROR_INVALID_PARAMETER),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sid;

    fn entry(mode: u32, access: u32, name: &str) -> Entry {
        Entry { access, mode, inherit: 0, trustee: Trustee::Name(name.into()) }
    }

    #[test]
    fn deny_first_and_old_order_kept() {
        let a = set_entries(&[entry(1, 0x120089, "Everyone"), entry(3, 0x1F01FF, "SYSTEM")], None).unwrap().unwrap();
        let aces = acl::aces_in(&a);
        assert_eq!((aces[0].kind, aces[1].kind, a.len()), (1, 0, 8 + 20 + 20));
        // Grant with an old ACL: appended, size kept; revoke shrinks it.
        let mut old = acl_of(2, &aces);
        old[2..4].copy_from_slice(&1024u16.to_le_bytes());
        old.resize(1024, 0);
        let g = set_entries(&[entry(1, 1, "Everyone")], Some(&old)).unwrap().unwrap();
        assert_eq!((u16::from_le_bytes([g[2], g[3]]), acl::aces_in(&g).len()), (1024, 3));
        let r = set_entries(&[entry(4, 0, "Everyone")], Some(&old)).unwrap().unwrap();
        assert_eq!((u16::from_le_bytes([r[2], r[3]]), acl::aces_in(&r).len()), (1004, 1));
        assert_eq!(set_entries(&[entry(0, 1, "Everyone")], None), Err(87));
        assert_eq!(set_entries(&[entry(1, 1, "S-1-1-0")], None), Err(1332));
        assert_eq!(set_entries(&[], None), Ok(None));
    }

    #[test]
    fn explicit_round_trip() {
        let a = set_entries(&[entry(5, 1, "Everyone"), entry(1, 7, "Users")], None).unwrap().unwrap();
        let e = explicit_entries(&a).unwrap();
        assert_eq!(e.iter().map(|x| x.mode).collect::<Vec<_>>(), [5, 6, 1]);
        assert_eq!(Sid::from_bytes(&e[2].sid).unwrap(), sid::builtin(545));
    }
}
