//! Inherited ACEs for files and registry keys (ADR 0171): a stored
//! descriptor per path holds only what was set explicitly; the descriptor an
//! object shows is that plus the inheritable ACEs of its parent's, computed
//! down the path. So setting a parent's DACL changes every child that did
//! not protect its own (the sheets of `SetNamedSecurityInfoW`).
//!
//! The rules are Learn's ("ACE inheritance", "Order of ACEs in a DACL",
//! `ACE_HEADER` flags): a child ACE has `INHERITED_ACE`; for a file only
//! `OBJECT_INHERIT` ACEs apply and the child keeps no inherit flags; for a
//! container `CONTAINER_INHERIT` ACEs keep `OBJECT_INHERIT` and
//! `CONTAINER_INHERIT` unless `NO_PROPAGATE`, `OBJECT_INHERIT`-only ACEs
//! become `INHERIT_ONLY` ones.

use crate::acl::{self, Ace};
use crate::sd::{self, Parts};
use crate::sdops::acl_of;
use alloc::vec::Vec;

pub const OBJECT_INHERIT: u8 = 0x1;
pub const CONTAINER_INHERIT: u8 = 0x2;
pub const NO_PROPAGATE: u8 = 0x4;
pub const INHERIT_ONLY: u8 = 0x8;
pub const INHERITED: u8 = 0x10;

/// `CREATOR OWNER` (`S-1-3-0`), whose `CONTAINER_INHERIT | INHERIT_ONLY` ACE stays
/// `INHERIT_ONLY` when inherited to a subcontainer (`GetSecurityInfo.RegistryKey.yaml`, `new_key`).
const CREATOR_OWNER: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0];

/// What was set on one path.
#[derive(Clone, Debug, Default)]
pub struct Stored {
    pub owner: Option<Vec<u8>>,
    pub group: Option<Vec<u8>>,
    /// `Some(None)`: a NULL DACL; `Some(Some(acl))`: the explicit ACEs;
    /// `None`: nothing set (all inherited).
    pub dacl: Option<Option<Vec<u8>>>,
    pub protected: bool,
    pub sacl: Option<Vec<u8>>,
    /// The DACL is used exactly as stored: no inherited ACEs are added and no
    /// `SE_DACL_AUTO_INHERITED` is shown (unless `auto_inherited` overrides it). What
    /// `NtSetSecurityObject` stores when the descriptor does not ask for automatic inheritance
    /// (`SetFileSecurityW`, `SetKernelObjectSecurity`, `RegSetKeySecurity`), and a new file's
    /// own descriptor when its directory passes nothing down.
    pub verbatim: bool,
    /// Explicit override for `SE_DACL_AUTO_INHERITED` on a verbatim entry (used to freeze an
    /// existing child's `0x8404` DACL when `RegSetKeySecurity` modifies its parent without
    /// propagating).
    pub auto_inherited: Option<bool>,
    /// Whether `SE_SACL_AUTO_INHERITED` (`0x0800`) is set on this path and inherited below it
    /// (e.g. `HKLM\SOFTWARE\Wow6432Node`, `GetSecurityInfo.RegistryKey.yaml` `parts`).
    pub sacl_auto_inherited: bool,
    /// True when this entry is only a non-propagating snapshot taken before a verbatim parent
    /// change (`RegSetKeySecurity`), so a later propagating `SetSecurityInfo` on an ancestor
    /// clears it and propagates normally.
    pub snapshot: bool,
}

/// The inheritable ACEs of `parent` for a child that is a container or not.
/// Duplicate resulting ACEs (same type, flags, mask and SID) are emitted once, in first-seen
/// order (`SetSecurityInfo.RegistryKey.yaml` `unprotected`).
pub fn inherited_aces(parent: Option<&[u8]>, container: bool) -> Vec<Ace> {
    let mut out: Vec<Ace> = Vec::new();
    for a in parent.map(acl::aces_in).unwrap_or_default() {
        if !matches!(a.kind, acl::ACCESS_ALLOWED_ACE_TYPE | acl::ACCESS_DENIED_ACE_TYPE | acl::SYSTEM_AUDIT_ACE_TYPE) {
            continue;
        }
        let f = a.flags;
        let flags = if container {
            if f & CONTAINER_INHERIT != 0 {
                if f & NO_PROPAGATE != 0 {
                    INHERITED
                } else if f & INHERIT_ONLY != 0 && a.sid() == Some(&CREATOR_OWNER) {
                    INHERITED | (f & (OBJECT_INHERIT | CONTAINER_INHERIT)) | INHERIT_ONLY
                } else {
                    INHERITED | (f & (OBJECT_INHERIT | CONTAINER_INHERIT))
                }
            } else if f & OBJECT_INHERIT != 0 && f & NO_PROPAGATE == 0 {
                INHERITED | OBJECT_INHERIT | INHERIT_ONLY
            } else {
                continue;
            }
        } else if f & OBJECT_INHERIT != 0 {
            INHERITED
        } else {
            continue;
        };
        // Audit ACEs keep their success / failure bits.
        let flags = flags | (f & 0xC0);
        let mut bytes = a.bytes.clone();
        bytes[1] = flags;
        if !out.iter().any(|existing| existing.bytes == bytes) {
            out.push(Ace { kind: a.kind, flags, bytes });
        }
    }
    out
}

/// `acl`'s ACEs that were set explicitly (not `INHERITED_ACE`), in canonical order
/// (explicit deny ACEs before explicit allow ACEs, preserving order within each kind).
pub fn explicit_acl(acl_bytes: &[u8]) -> Vec<u8> {
    let mut deny = Vec::new();
    let mut rest = Vec::new();
    for a in acl::aces_in(acl_bytes).into_iter().filter(|a| a.flags & INHERITED == 0) {
        if a.kind == acl::ACCESS_DENIED_ACE_TYPE {
            deny.push(a);
        } else {
            rest.push(a);
        }
    }
    deny.extend(rest);
    acl_of(acl_bytes.first().copied().unwrap_or(acl::ACL_REVISION), &deny)
}

/// Every ACE of `acl` kept as an explicit ACE (`INHERITED_ACE` cleared in place): what
/// `SetSecurityInfo` / `SetNamedSecurityInfoW` with `PROTECTED_DACL_SECURITY_INFORMATION`
/// stores (`SetSecurityInfo.RegistryKey.yaml` `protected`).
pub fn unmark_inherited_acl(acl_bytes: &[u8]) -> Vec<u8> {
    let aces: Vec<Ace> = acl::aces_in(acl_bytes)
        .into_iter()
        .map(|mut a| {
            a.flags &= !INHERITED;
            a.bytes[1] = a.flags;
            a
        })
        .collect();
    acl_of(acl_bytes.first().copied().unwrap_or(acl::ACL_REVISION), &aces)
}

/// The descriptor of the object at the end of `chain` (one entry per path
/// component from the volume or hive root down to it, `None` where nothing
/// was set), given the `default` descriptor of an unset root and whether
/// the object is a container.
pub fn effective(default: &Parts, chain: &[Option<&Stored>], container: bool) -> Parts {
    let mut cur = default.clone();
    // Whether the level above kept its DACL as given (`Stored::verbatim`).
    let mut above_verbatim = false;
    for (i, st) in chain.iter().enumerate() {
        let is_container = i + 1 < chain.len() || container;
        let mut next = default.clone();
        if i > 0 || st.is_some() {
            let inherited = if i == 0 { Vec::new() } else { inherited_aces(cur.dacl.as_deref(), is_container) };
            let protected = st.is_some_and(|s| s.protected);
            let verbatim = st.is_some_and(|s| s.verbatim && s.dacl.is_some());
            let has_dacl = st.is_some_and(|s| s.dacl.is_some());
            let auto_inherited = if let Some(ov) = st.and_then(|s| s.auto_inherited) {
                ov
            } else if verbatim {
                false
            } else if has_dacl {
                true
            } else {
                i > 0 && cur.control & sd::SE_DACL_AUTO_INHERITED != 0
            };
            next.control = sd::SE_DACL_PRESENT
                | if auto_inherited { sd::SE_DACL_AUTO_INHERITED } else { 0 }
                | if protected { sd::SE_DACL_PROTECTED } else { 0 };
            let rev = acl::ACL_REVISION;
            next.dacl = match st.and_then(|s| s.dacl.as_ref()) {
                Some(Some(explicit)) if verbatim => Some(explicit.clone()),
                Some(None) => None,
                Some(Some(explicit)) => {
                    let mut all = acl::aces_in(explicit);
                    if !protected {
                        all.extend(inherited);
                    }
                    Some(acl_of(explicit.first().copied().unwrap_or(rev), &all))
                }
                None if i == 0 => default.dacl.clone(),
                // A file that was put in a directory whose DACL is kept as given and passes
                // nothing down (the test directory, `seed_verbatim_path`) shows that DACL: it got
                // the same default as the directory when it was put there.
                None if inherited.is_empty() && above_verbatim => {
                    next.control &= !sd::SE_DACL_AUTO_INHERITED;
                    next.owner = cur.owner.clone();
                    next.group = cur.group.clone();
                    cur.dacl.clone()
                }
                None => Some(acl_of(rev, &inherited)),
            };
            if i > 0 && cur.control & sd::SE_SACL_AUTO_INHERITED != 0 {
                next.control |= sd::SE_SACL_AUTO_INHERITED;
            }
            if let Some(s) = st {
                if let Some(o) = &s.owner {
                    next.owner = Some(o.clone());
                }
                if let Some(g) = &s.group {
                    next.group = Some(g.clone());
                }
                if s.sacl_auto_inherited {
                    next.control |= sd::SE_SACL_AUTO_INHERITED;
                }
                if let Some(sacl) = &s.sacl {
                    next.sacl = Some(sacl.clone());
                    next.control |= sd::SE_SACL_PRESENT;
                    if !s.sacl_auto_inherited {
                        next.control &= !sd::SE_SACL_AUTO_INHERITED;
                    }
                }
            }
        }
        above_verbatim = st.is_some_and(|s| s.verbatim);
        cur = next;
    }
    cur
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdstr;

    fn parts(s: &str) -> Parts {
        let w: Vec<u16> = s.encode_utf16().collect();
        Parts::from_relative(&sdstr::parse(&w).unwrap()).unwrap()
    }

    #[test]
    fn children_inherit_and_protect() {
        let default = parts("O:BAG:SYD:P(A;OICI;GA;;;SY)");
        let dir = Stored {
            dacl: Some(Some(parts("D:(A;OICI;GA;;;BA)(A;;GR;;;WD)").dacl.unwrap())),
            protected: true,
            ..Stored::default()
        };
        let file = effective(&default, &[None, Some(&dir), None], false);
        let a = acl::aces_in(file.dacl.as_ref().unwrap());
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].flags, INHERITED);
        let sub = effective(&default, &[None, Some(&dir), None], true);
        assert_eq!(acl::aces_in(sub.dacl.as_ref().unwrap())[0].flags, INHERITED | OBJECT_INHERIT | CONTAINER_INHERIT);
        assert_eq!(file.control, sd::SE_DACL_PRESENT | sd::SE_DACL_AUTO_INHERITED);
        // The directory itself: explicit ACEs only, protected.
        let d = effective(&default, &[None, Some(&dir)], true);
        assert_eq!(acl::aces_in(d.dacl.as_ref().unwrap()).len(), 2);
        assert_eq!(d.control, 0x1404);
    }

    /// specs/windows/advapi32.dll/SetFileSecurityW.yaml, `dacl_replaced`: a DACL set without
    /// automatic inheritance is shown as given, without AI and without the parent's ACEs.
    #[test]
    fn a_verbatim_dacl_is_shown_as_given() {
        let default = parts("O:BAG:SYD:P(A;OICI;GA;;;SY)");
        let dir = Stored { dacl: Some(Some(parts("D:(A;OICI;GA;;;BA)").dacl.unwrap())), ..Stored::default() };
        let given = parts("D:(A;ID;FA;;;WD)").dacl.unwrap();
        let file = Stored { dacl: Some(Some(given.clone())), verbatim: true, ..Stored::default() };
        let f = effective(&default, &[None, Some(&dir), Some(&file)], false);
        assert_eq!(f.dacl.as_deref(), Some(given.as_slice()));
        assert_eq!(f.control, sd::SE_DACL_PRESENT);
        let protected = Stored { protected: true, ..file };
        assert_eq!(effective(&default, &[None, Some(&dir), Some(&protected)], false).control, 0x1004);
    }

    /// A file laid out in a directory kept verbatim that passes nothing down shows the
    /// directory's DACL, owner and group (the test directory, `GetFileSecurityW.yaml`,
    /// `default_security`); a directory with inheritable ACEs still passes them on.
    #[test]
    fn a_laid_out_file_takes_its_verbatim_directorys_dacl() {
        let default = parts("O:BAG:SYD:P(A;OICI;GA;;;SY)");
        let d = parts("O:WDG:WDD:(A;;FA;;;SY)(A;;FA;;;BA)");
        let dir = Stored {
            owner: d.owner.clone(),
            group: d.group.clone(),
            dacl: Some(d.dacl.clone()),
            verbatim: true,
            ..Stored::default()
        };
        let f = effective(&default, &[None, Some(&dir), None], false);
        assert_eq!(f.dacl, d.dacl);
        assert_eq!((f.owner, f.group), (d.owner, d.group));
        assert_eq!(f.control & sd::SE_DACL_AUTO_INHERITED, 0);
        let passing =
            Stored { dacl: Some(Some(parts("D:(A;OICI;FA;;;BA)").dacl.unwrap())), verbatim: true, ..Stored::default() };
        let g = effective(&default, &[None, Some(&passing), None], false);
        assert_eq!(acl::aces_in(g.dacl.as_ref().unwrap())[0].flags, INHERITED);
    }

    #[test]
    fn registry_key_inheritance_creator_owner_and_dedup() {
        let default = parts("O:BAG:SYD:P(A;CI;KA;;;SY)");
        let sw_dacl = parts("D:PAI(A;CI;KR;;;BU)(A;CI;KA;;;BA)(A;CI;KA;;;SY)(A;CIIO;KA;;;CO)").dacl.unwrap();
        let sw = Stored { dacl: Some(Some(sw_dacl)), protected: true, ..Stored::default() };
        // Child key below HKLM\SOFTWARE inherits 4 ACEs; CREATOR OWNER keeps INHERIT_ONLY (0x1a).
        let base = effective(&default, &[None, Some(&sw), None], true);
        assert_eq!(base.control, sd::SE_DACL_PRESENT | sd::SE_DACL_AUTO_INHERITED);
        let aces = acl::aces_in(base.dacl.as_ref().unwrap());
        assert_eq!(aces.len(), 4);
        assert_eq!(aces[0].flags, INHERITED | CONTAINER_INHERIT);
        assert_eq!(aces[3].flags, INHERITED | CONTAINER_INHERIT | INHERIT_ONLY);

        // Protecting the key turns inherited ACEs into explicit ones (0x02, 0x0a for CO).
        let mut with_inter = aces.clone();
        let inter_ace = acl::aces_in(&parts("D:(A;OICI;0x1;;;IU)").dacl.unwrap()).pop().unwrap();
        with_inter.push(inter_ace.clone());
        let prot_acl = unmark_inherited_acl(&acl_of(acl::ACL_REVISION, &with_inter));
        let prot_aces = acl::aces_in(&prot_acl);
        assert_eq!(prot_aces.len(), 5);
        assert_eq!(prot_aces[0].flags, CONTAINER_INHERIT);
        assert_eq!(prot_aces[3].flags, CONTAINER_INHERIT | INHERIT_ONLY);
        assert_eq!(prot_aces[4].flags, OBJECT_INHERIT | CONTAINER_INHERIT);

        // Unprotecting after protecting keeps all explicit ACEs plus re-inherited parent ACEs,
        // while a child deduplicates identical inherited ACEs.
        let mut with_dup = prot_aces.clone();
        with_dup.push(inter_ace);
        let unprot = Stored {
            dacl: Some(Some(explicit_acl(&acl_of(acl::ACL_REVISION, &with_dup)))),
            protected: false,
            ..Stored::default()
        };
        let parent_eff = effective(&default, &[None, Some(&sw), Some(&unprot)], true);
        assert_eq!(parent_eff.control, sd::SE_DACL_PRESENT | sd::SE_DACL_AUTO_INHERITED);
        assert_eq!(acl::aces_in(parent_eff.dacl.as_ref().unwrap()).len(), 10);
        let child_eff = effective(&default, &[None, Some(&sw), Some(&unprot), None], true);
        assert_eq!(acl::aces_in(child_eff.dacl.as_ref().unwrap()).len(), 5);
    }

    #[test]
    fn null_dacl_hkcu_and_sacl_auto_inherited() {
        let default = parts("O:BAG:SYD:P(A;CI;KA;;;SY)");
        // SetSecurityInfo NULL DACL keeps SE_DACL_AUTO_INHERITED (0x9404); child gets 0 ACEs (0x8404).
        let null_set = Stored { dacl: Some(None), protected: true, ..Stored::default() };
        let p = effective(&default, &[None, Some(&null_set)], true);
        assert!(p.dacl.is_none());
        assert_eq!(p.control, sd::SE_DACL_PRESENT | sd::SE_DACL_AUTO_INHERITED | sd::SE_DACL_PROTECTED);
        let c = effective(&default, &[None, Some(&null_set), None], true);
        assert_eq!(acl::aces_in(c.dacl.as_ref().unwrap()).len(), 0);
        assert_eq!(c.control, sd::SE_DACL_PRESENT | sd::SE_DACL_AUTO_INHERITED);

        // Verbatim NULL DACL (RegSetKeySecurity) has no AI (0x8004).
        let null_reg = Stored { dacl: Some(None), verbatim: true, ..Stored::default() };
        let r = effective(&default, &[None, Some(&null_reg)], true);
        assert!(r.dacl.is_none());
        assert_eq!(r.control, sd::SE_DACL_PRESENT);

        // HKCU (verbatim, protected, no AI): child inherits 4 OICI ACEs with control 0x8004 (no AI).
        let hkcu_dacl = parts("D:P(A;OICI;KA;;;SY)(A;OICI;KA;;;BA)(A;;KR;;;AC)").dacl.unwrap();
        let hkcu = Stored { dacl: Some(Some(hkcu_dacl)), protected: true, verbatim: true, ..Stored::default() };
        let child = effective(&default, &[None, Some(&hkcu), None], true);
        assert_eq!(child.control, sd::SE_DACL_PRESENT);
        assert_eq!(acl::aces_in(child.dacl.as_ref().unwrap()).len(), 2);

        // Wow6432Node propagates SE_SACL_AUTO_INHERITED to its children.
        let wow = Stored { sacl_auto_inherited: true, ..Stored::default() };
        let wchild = effective(&default, &[None, Some(&wow), None], true);
        assert_ne!(wchild.control & sd::SE_SACL_AUTO_INHERITED, 0);
    }
}
