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
}

/// The inheritable ACEs of `parent` for a child that is a container or not.
pub fn inherited_aces(parent: Option<&[u8]>, container: bool) -> Vec<Ace> {
    let mut out = Vec::new();
    for a in parent.map(acl::aces_in).unwrap_or_default() {
        if !matches!(a.kind, acl::ACCESS_ALLOWED_ACE_TYPE | acl::ACCESS_DENIED_ACE_TYPE | acl::SYSTEM_AUDIT_ACE_TYPE) {
            continue;
        }
        let f = a.flags;
        let flags = if container {
            if f & CONTAINER_INHERIT != 0 {
                INHERITED | if f & NO_PROPAGATE != 0 { 0 } else { f & (OBJECT_INHERIT | CONTAINER_INHERIT) }
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
        out.push(Ace { kind: a.kind, flags, bytes });
    }
    out
}

/// `acl`'s ACEs that were set explicitly (not `INHERITED_ACE`).
pub fn explicit_acl(acl_bytes: &[u8]) -> Vec<u8> {
    let aces: Vec<Ace> = acl::aces_in(acl_bytes).into_iter().filter(|a| a.flags & INHERITED == 0).collect();
    acl_of(acl_bytes.first().copied().unwrap_or(acl::ACL_REVISION), &aces)
}

/// The descriptor of the object at the end of `chain` (one entry per path
/// component from the volume or hive root down to it, `None` where nothing
/// was set), given the `default` descriptor of an unset root and whether
/// the object is a container.
pub fn effective(default: &Parts, chain: &[Option<&Stored>], container: bool) -> Parts {
    let mut cur = default.clone();
    for (i, st) in chain.iter().enumerate() {
        let is_container = i + 1 < chain.len() || container;
        let mut next = default.clone();
        if i > 0 || st.is_some() {
            let inherited = if i == 0 { Vec::new() } else { inherited_aces(cur.dacl.as_deref(), is_container) };
            let protected = st.is_some_and(|s| s.protected);
            next.control =
                sd::SE_DACL_PRESENT | sd::SE_DACL_AUTO_INHERITED | if protected { sd::SE_DACL_PROTECTED } else { 0 };
            let rev = acl::ACL_REVISION;
            next.dacl = match st.and_then(|s| s.dacl.as_ref()) {
                Some(None) => {
                    next.control &= !sd::SE_DACL_AUTO_INHERITED;
                    None
                }
                Some(Some(explicit)) => {
                    let mut all = acl::aces_in(explicit);
                    if !protected {
                        all.extend(inherited);
                    }
                    Some(acl_of(explicit.first().copied().unwrap_or(rev), &all))
                }
                None if i == 0 => default.dacl.clone(),
                None => Some(acl_of(rev, &inherited)),
            };
            if let Some(s) = st {
                if let Some(o) = &s.owner {
                    next.owner = Some(o.clone());
                }
                if let Some(g) = &s.group {
                    next.group = Some(g.clone());
                }
                if let Some(sacl) = &s.sacl {
                    next.sacl = Some(sacl.clone());
                    next.control |= sd::SE_SACL_PRESENT;
                }
            }
        }
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
}
