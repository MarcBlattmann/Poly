//! What `NtQuerySecurityObject` and `NtSetSecurityObject` do with a
//! descriptor and a `SECURITY_INFORMATION` mask (spec sheets
//! `NtQuerySecurityObject`, `NtSetSecurityObject`, `GetKernelObjectSecurity`,
//! `SetKernelObjectSecurity`, `SetSecurityInfo`): select the parts to hand
//! out, and merge new parts into the stored descriptor.

use crate::acl;
use crate::err::{SecErr, SecResult};
use crate::sd::*;
use crate::sid::{self, Sid};
use crate::token::{SE_GROUP_OWNER, SE_GROUP_USE_FOR_DENY_ONLY, SE_PRIVILEGE_ENABLED, Token};
use alloc::vec::Vec;

/// The `SECURITY_INFORMATION` bits the SACL side covers: the SACL itself
/// (needs `ACCESS_SYSTEM_SECURITY`) and the label.
pub const SACL_SIDE: u32 = SACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION;
/// Every bit `NtQuerySecurityObject` accepts without error:
/// owner, group, DACL, SACL, label, attribute, scope, process trust label,
/// access filter, backup and the protection flags.
pub const KNOWN_INFORMATION: u32 = 0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x20 | 0x40 | 0x80 | 0x100;

/// The ACEs of a SACL that are label ACEs, and the others.
fn split_sacl(sacl: &[u8]) -> (Vec<acl::Ace>, Vec<acl::Ace>) {
    acl::aces_in(sacl).into_iter().partition(|a| a.kind == acl::SYSTEM_MANDATORY_LABEL_ACE_TYPE)
}

/// An ACL of `revision` holding `aces`, sized exactly.
fn acl_of(revision: u8, aces: &[acl::Ace]) -> Vec<u8> {
    let body: Vec<u8> = aces.iter().flat_map(|a| a.bytes.iter().copied()).collect();
    let size = acl::ACL_HEADER + body.len();
    let mut v = Vec::with_capacity(size);
    v.extend_from_slice(&[revision, 0]);
    v.extend_from_slice(&(size as u16).to_le_bytes());
    v.extend_from_slice(&(aces.len() as u16).to_le_bytes());
    v.extend_from_slice(&[0, 0]);
    v.extend(body);
    v
}

/// `acl` cut to the ACEs it holds: the size field names the bytes in use
/// (an ACL built in a 1024-byte buffer is stored at its used size).
pub fn trimmed_acl(acl_bytes: &[u8]) -> Vec<u8> {
    let aces = acl::aces_in(acl_bytes);
    acl_of(acl_bytes.first().copied().unwrap_or(acl::ACL_REVISION), &aces)
}

/// The parts of `stored` that `info` selects, as a descriptor to hand out.
/// The SACL side: the audit ACEs only with `SACL_SECURITY_INFORMATION`,
/// the label ACEs only with `LABEL_SECURITY_INFORMATION`; asking for either
/// marks the control `SE_SACL_AUTO_INHERITED` (spec sheet: a label query
/// of an object without a label answers control 0x8800).
pub fn select(stored: &Parts, info: u32) -> Parts {
    let mut out = Parts { revision: SECURITY_DESCRIPTOR_REVISION, ..Parts::default() };
    if info & OWNER_SECURITY_INFORMATION != 0 {
        out.owner = stored.owner.clone();
    }
    if info & GROUP_SECURITY_INFORMATION != 0 {
        out.group = stored.group.clone();
    }
    if info & DACL_SECURITY_INFORMATION != 0 && stored.control & SE_DACL_PRESENT != 0 {
        out.dacl = stored.dacl.clone();
        out.control |= SE_DACL_PRESENT
            | (stored.control & (SE_DACL_PROTECTED | SE_DACL_AUTO_INHERITED | SE_DACL_AUTO_INHERIT_REQ));
    }
    if info & SACL_SIDE != 0 {
        // The flag is the stored one, and a descriptor without any SACL
        // answers a label query with it (control 0x8800); a label set by
        // `SetSecurityObject` clears it (the sheets: control 0x8010).
        if info & LABEL_SECURITY_INFORMATION != 0
            && (stored.control & SE_SACL_AUTO_INHERITED != 0 || stored.sacl.is_none())
        {
            out.control |= SE_SACL_AUTO_INHERITED;
        }
        if let Some(sacl) = &stored.sacl {
            let (labels, audits) = split_sacl(sacl);
            let mut keep: Vec<acl::Ace> = Vec::new();
            if info & SACL_SECURITY_INFORMATION != 0 {
                keep.extend(audits);
            }
            if info & LABEL_SECURITY_INFORMATION != 0 {
                keep.extend(labels);
            }
            {
                let rev = sacl.first().copied().unwrap_or(acl::ACL_REVISION);
                out.sacl = Some(acl_of(rev, &keep));
                out.control |= SE_SACL_PRESENT | (stored.control & (SE_SACL_PROTECTED | SE_SACL_AUTO_INHERIT_REQ));
            }
        }
    }
    out
}

/// What may change an owner: the token's user, a group it may own with, or
/// `SeRestorePrivilege` (spec sheets: with that privilege disabled only
/// the user and an owner group are accepted).
pub fn owner_allowed(t: &Token, owner: &[u8]) -> bool {
    let Some(s) = Sid::from_bytes(owner) else { return false };
    s == t.user
        || t.groups.iter().any(|(g, a)| *g == s && a & SE_GROUP_OWNER != 0 && a & SE_GROUP_USE_FOR_DENY_ONLY == 0)
        || t.privileges.iter().any(|&(l, a)| l == 18 && a & SE_PRIVILEGE_ENABLED != 0)
}

/// Merge the parts `info` selects from `new` into `stored`. OWNER or GROUP
/// information with a descriptor that lacks the part is
/// [`SecErr::InvalidSd`]; a missing DACL with DACL information is a NULL
/// DACL; the protection flags (`PROTECTED_DACL`, `UNPROTECTED_DACL`) and the
/// descriptor's own `SE_DACL_PROTECTED` decide the stored protection; the
/// label side replaces only the label ACEs of the SACL.
pub fn merge(stored: &Parts, new: &Parts, info: u32) -> SecResult<Parts> {
    let mut out = stored.clone();
    if info & OWNER_SECURITY_INFORMATION != 0 {
        out.owner = Some(new.owner.clone().ok_or(SecErr::InvalidSd)?);
        out.control &= !SE_OWNER_DEFAULTED;
    }
    if info & GROUP_SECURITY_INFORMATION != 0 {
        out.group = Some(new.group.clone().ok_or(SecErr::InvalidSd)?);
        out.control &= !SE_GROUP_DEFAULTED;
    }
    if info & DACL_SECURITY_INFORMATION != 0 {
        out.dacl = if new.control & SE_DACL_PRESENT != 0 { new.dacl.clone() } else { None };
        out.control |= SE_DACL_PRESENT;
        out.control &= !(SE_DACL_PROTECTED | SE_DACL_DEFAULTED | SE_DACL_AUTO_INHERITED | SE_DACL_AUTO_INHERIT_REQ);
        let protect = new.control & SE_DACL_PROTECTED != 0 || info & PROTECTED_DACL_SECURITY_INFORMATION != 0;
        if protect && info & UNPROTECTED_DACL_SECURITY_INFORMATION == 0
            || (protect && info & PROTECTED_DACL_SECURITY_INFORMATION != 0)
        {
            out.control |= SE_DACL_PROTECTED;
        }
    }
    if info & SACL_SECURITY_INFORMATION != 0 {
        out.sacl = if new.control & SE_SACL_PRESENT != 0 { new.sacl.clone() } else { None };
        out.control |= SE_SACL_PRESENT;
        out.control &= !SE_SACL_AUTO_INHERITED;
    } else if info & LABEL_SECURITY_INFORMATION != 0 {
        // Replace the label ACEs; keep the audit ACEs.
        let old_audits = stored.sacl.as_deref().map(split_sacl).map(|(_, a)| a).unwrap_or_default();
        let new_labels = new.sacl.as_deref().map(split_sacl).map(|(l, _)| l).unwrap_or_default();
        let mut all = old_audits;
        all.extend(new_labels);
        out.sacl = Some(acl_of(acl::ACL_REVISION, &all));
        out.control |= SE_SACL_PRESENT;
        out.control &= !SE_SACL_AUTO_INHERITED;
        out.control = (out.control & !SE_SACL_PROTECTED) | (new.control & SE_SACL_PROTECTED);
    }
    Ok(out)
}

/// The integrity RID of a descriptor's label, if it has one.
pub fn label_rid(sacl: Option<&[u8]>) -> Option<u32> {
    let (labels, _) = split_sacl(sacl?);
    labels.first().and_then(|a| a.sid()).and_then(Sid::from_bytes).and_then(|s| s.rid())
}

/// A label above the caller's integrity level is refused
/// (`ERROR_INVALID_LABEL`).
pub fn label_allowed(t: &Token, new: &Parts) -> bool {
    label_rid(new.sacl.as_deref()).is_none_or(|rid| rid <= t.integrity_rid())
}

/// The default descriptor of a new object made under `t`: owner the token
/// owner, group its primary group, DACL its default DACL with each mask's
/// generic rights mapped by `map`.
pub fn default_descriptor(t: &Token, map: &dyn Fn(u32) -> u32) -> Parts {
    let dacl = t.default_dacl.as_ref().map(|d| {
        let aces: Vec<acl::Ace> = acl::aces_in(d)
            .into_iter()
            .map(|mut a| {
                if a.bytes.len() >= 8 {
                    let m = map(a.mask()).to_le_bytes();
                    a.bytes[4..8].copy_from_slice(&m);
                }
                a
            })
            .collect();
        acl_of(d[0], &aces)
    });
    Parts {
        revision: SECURITY_DESCRIPTOR_REVISION,
        sbz1: 0,
        control: SE_DACL_PRESENT,
        owner: Some(t.owner.to_bytes()),
        group: Some(t.primary_group.to_bytes()),
        sacl: None,
        dacl,
    }
}

/// A label ACE SACL for `rid` with policy `policy` (a job's default).
pub fn label_sacl(rid: u32, policy: u32) -> Vec<u8> {
    let ace =
        acl::ace(acl::SYSTEM_MANDATORY_LABEL_ACE_TYPE, 0, policy, &sid::mandatory(rid).to_bytes()).expect("label SID");
    acl_of(acl::ACL_REVISION, &[acl::Ace { kind: acl::SYSTEM_MANDATORY_LABEL_ACE_TYPE, flags: 0, bytes: ace }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sddl;
    use crate::sdstr;

    fn parts(s: &str) -> Parts {
        let w: Vec<u16> = s.encode_utf16().collect();
        Parts::from_relative(&sdstr::parse(&w).unwrap()).unwrap()
    }

    fn text(p: &Parts, info: u32) -> alloc::string::String {
        sdstr::format(p, info).unwrap().0
    }

    #[test]
    fn select_and_merge() {
        let stored = parts("O:BAG:SYD:(A;;GA;;;WD)S:(AU;SAFA;GA;;;WD)(ML;;NW;;;LW)");
        assert_eq!(text(&select(&stored, OWNER_SECURITY_INFORMATION), 0xff), "O:BA");
        assert_eq!(text(&select(&stored, LABEL_SECURITY_INFORMATION), 0xff), "S:(ML;;NW;;;LW)");
        assert_eq!(text(&select(&stored, SACL_SECURITY_INFORMATION), 0xff), "S:(AU;SAFA;GA;;;WD)");
        let none = select(&stored, 0);
        assert_eq!(none.control, 0);
        let lab = select(&parts("O:BA"), LABEL_SECURITY_INFORMATION);
        assert_eq!(lab.control, SE_SACL_AUTO_INHERITED);
        // Merge the DACL, protected.
        let m = merge(&stored, &parts("D:P(A;;GR;;;WD)"), DACL_SECURITY_INFORMATION).unwrap();
        assert_eq!(text(&m, 0xff), "O:BAG:SYD:P(A;;GR;;;WD)S:(AU;SAFA;GA;;;WD)(ML;;NW;;;LW)");
        let m = merge(&m, &parts("D:(A;;GR;;;WD)"), DACL_SECURITY_INFORMATION).unwrap();
        assert_eq!(m.control & SE_DACL_PROTECTED, 0);
        let m = merge(&stored, &parts("O:BA"), DACL_SECURITY_INFORMATION).unwrap();
        assert!(m.dacl.is_none() && m.control & SE_DACL_PRESENT != 0);
        // The label side keeps the audit ACEs.
        let m = merge(&stored, &parts("S:(ML;;NWNR;;;ME)"), LABEL_SECURITY_INFORMATION).unwrap();
        assert_eq!(text(&m, 0xff), "O:BAG:SYD:(A;;GA;;;WD)S:(AU;SAFA;GA;;;WD)(ML;;NWNR;;;ME)");
        assert_eq!(merge(&stored, &parts("G:SY"), OWNER_SECURITY_INFORMATION), Err(SecErr::InvalidSd));
        assert_eq!(label_rid(m.sacl.as_deref()), Some(0x2000));
    }

    #[test]
    fn owners_and_defaults() {
        let t = Token::poly_user();
        assert!(owner_allowed(&t, &t.user.to_bytes()));
        assert!(owner_allowed(&t, &sid::builtin(544).to_bytes()));
        assert!(owner_allowed(&t, &sid::builtin(545).to_bytes()), "SeRestore is enabled");
        let mut low = t.clone();
        low.privileges.iter_mut().for_each(|p| p.1 = 0);
        assert!(!owner_allowed(&low, &sid::builtin(545).to_bytes()));
        assert!(owner_allowed(&low, &sid::builtin(544).to_bytes()));
        let ev = default_descriptor(&t, &|m| {
            if m & 0x1000_0000 != 0 {
                0x1f0003
            } else if m & 0x8000_0000 != 0 {
                0x120001
            } else {
                m
            }
        });
        assert_eq!(
            text(&ev, 0xff),
            alloc::format!(
                "O:BAG:S-1-5-21-520193847-884726105-1936024571-513D:(A;;0x1f0003;;;BA)(A;;0x1f0003;;;SY)(A;;0x120001;;;{})",
                sddl::to_string(&crate::token::logon_sid(crate::token::POLY_LOGON_LUID))
            )
        );
        assert_eq!(ev.to_relative().len(), 144);
    }
}
