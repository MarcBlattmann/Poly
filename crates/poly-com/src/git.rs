//! The registrations of the process's global interface table (Learn: "IGlobalInterfaceTable",
//! "RegisterInterfaceInGlobal"; sheet `CLSID_StdGlobalInterfaceTable`): a cookie for every
//! registration, never reused, and the apartment that made it.
//!
//! The table holds interface pointers as plain numbers; the DLL adds and releases the object's
//! reference around the calls into this table (never while a lock is held).

use crate::guid::Guid;
use alloc::vec::Vec;

/// `CLSID_StdGlobalInterfaceTable` (`objidl.h`): {00000323-0000-0000-C000-000000000046}.
pub const CLSID_STD_GLOBAL_INTERFACE_TABLE: Guid =
    Guid { data1: 0x323, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
/// `IID_IGlobalInterfaceTable` (`objidl.h`): {00000146-0000-0000-C000-000000000046}.
pub const IID_IGLOBAL_INTERFACE_TABLE: Guid =
    Guid { data1: 0x146, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
/// `CLSID_BackgroundCopyManager` (`bits.h`): {4991D34B-80A1-4291-83B6-3328366B9097}.
pub const CLSID_BACKGROUND_COPY_MANAGER: Guid =
    Guid { data1: 0x4991_D34B, data2: 0x80A1, data3: 0x4291, data4: [0x83, 0xB6, 0x33, 0x28, 0x36, 0x6B, 0x90, 0x97] };

/// The apartment that made a registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// A single-threaded apartment: its thread's id.
    Sta(u32),
    /// The multithreaded apartment.
    Mta,
}

/// One registered interface pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Registration {
    pub cookie: u32,
    /// The interface pointer, as a number.
    pub object: usize,
    /// The interface id given at registration (not checked against the object).
    pub iid: Guid,
    pub owner: Owner,
}

/// The first cookie; each next one is 4 more. Never 0, never reused.
const FIRST_COOKIE: u32 = 0x0001_0004;

/// The table.
#[derive(Debug)]
pub struct Table {
    entries: Vec<Registration>,
    next: u32,
}

impl Table {
    pub const fn new() -> Table {
        Table { entries: Vec::new(), next: FIRST_COOKIE }
    }

    /// Register `object`; the cookie is different for every registration, also for an object
    /// registered before.
    pub fn register(&mut self, object: usize, iid: Guid, owner: Owner) -> u32 {
        let cookie = self.next;
        self.next = self.next.wrapping_add(4);
        if self.next == 0 {
            self.next = FIRST_COOKIE;
        }
        self.entries.push(Registration { cookie, object, iid, owner });
        cookie
    }

    /// The registration of `cookie`, if there is one.
    pub fn get(&self, cookie: u32) -> Option<Registration> {
        self.entries.iter().find(|r| r.cookie == cookie).copied()
    }

    /// Remove the registration of `cookie` and return it.
    pub fn revoke(&mut self, cookie: u32) -> Option<Registration> {
        let i = self.entries.iter().position(|r| r.cookie == cookie)?;
        Some(self.entries.remove(i))
    }

    /// Remove every registration of an apartment that ended and return them.
    pub fn remove_owner(&mut self, owner: Owner) -> Vec<Registration> {
        let mut gone = Vec::new();
        self.entries.retain(|r| {
            if r.owner == owner {
                gone.push(*r);
                false
            } else {
                true
            }
        });
        gone
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a registration made by `owner` is fetched in the same apartment by `caller` (the very
/// object comes back); otherwise the caller gets a proxy.
pub fn same_apartment(owner: Owner, caller: Owner) -> bool {
    owner == caller
}

#[cfg(test)]
mod tests {
    use super::*;

    const IID: Guid = Guid { data1: 1, data2: 0, data3: 0, data4: [0; 8] };

    #[test]
    fn cookies_are_non_zero_distinct_and_never_reused() {
        let mut t = Table::new();
        let a = t.register(0x1000, IID, Owner::Mta);
        let b = t.register(0x1000, IID, Owner::Mta);
        assert!(a != 0 && b != 0 && a != b);
        assert!(t.revoke(a).is_some());
        let c = t.register(0x1000, IID, Owner::Mta);
        assert!(c != a && c != b);
        // The cookies the sheet probes are nobody's.
        for bad in [0, 1, 0x1234, 0xFFFF_FFFF] {
            assert!(t.get(bad).is_none(), "{bad:#x}");
            assert!(t.revoke(bad).is_none(), "{bad:#x}");
        }
    }

    #[test]
    fn revoke_removes_and_a_second_revoke_fails() {
        let mut t = Table::new();
        let a = t.register(8, IID, Owner::Sta(3));
        assert_eq!(t.get(a).map(|r| r.object), Some(8));
        assert_eq!(t.revoke(a).map(|r| r.owner), Some(Owner::Sta(3)));
        assert!(t.get(a).is_none());
        assert!(t.revoke(a).is_none());
        assert!(t.is_empty());
    }

    #[test]
    fn an_apartment_that_ends_takes_its_registrations() {
        let mut t = Table::new();
        let a = t.register(8, IID, Owner::Sta(3));
        let b = t.register(16, IID, Owner::Sta(4));
        let c = t.register(24, IID, Owner::Mta);
        let gone = t.remove_owner(Owner::Sta(3));
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].cookie, a);
        assert!(t.get(a).is_none());
        assert!(t.get(b).is_some() && t.get(c).is_some());
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn the_same_apartment_is_the_same_thread_or_the_mta() {
        assert!(same_apartment(Owner::Mta, Owner::Mta));
        assert!(same_apartment(Owner::Sta(1), Owner::Sta(1)));
        assert!(!same_apartment(Owner::Sta(1), Owner::Sta(2)));
        assert!(!same_apartment(Owner::Sta(1), Owner::Mta));
        assert!(!same_apartment(Owner::Mta, Owner::Sta(1)));
    }

    #[test]
    fn class_ids_are_the_documented_ones() {
        assert_eq!(
            &CLSID_STD_GLOBAL_INTERFACE_TABLE.to_text()[..],
            &"{00000323-0000-0000-C000-000000000046}".encode_utf16().collect::<Vec<_>>()[..]
        );
        assert_eq!(
            &IID_IGLOBAL_INTERFACE_TABLE.to_text()[..],
            &"{00000146-0000-0000-C000-000000000046}".encode_utf16().collect::<Vec<_>>()[..]
        );
        assert_eq!(
            &CLSID_BACKGROUND_COPY_MANAGER.to_text()[..],
            &"{4991D34B-80A1-4291-83B6-3328366B9097}".encode_utf16().collect::<Vec<_>>()[..]
        );
    }
}
