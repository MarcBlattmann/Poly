//! The tables on their own: pools, weak references, associations, locks,
//! counts, selectors and run-time classes on a small simulated memory.

use crate::Mem;
use crate::arc::*;
use crate::runtime::Runtime;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

#[derive(Default)]
struct Sim {
    bytes: BTreeMap<u64, u8>,
    next: u64,
}

impl Mem for Sim {
    fn u8(&self, addr: u64) -> u8 {
        self.bytes.get(&addr).copied().unwrap_or(0)
    }
    fn write_u64(&mut self, addr: u64, value: u64) {
        for (i, b) in value.to_le_bytes().into_iter().enumerate() {
            self.bytes.insert(addr + i as u64, b);
        }
    }
    fn alloc(&mut self, bytes: &[u8]) -> u64 {
        if self.next == 0 {
            self.next = 0x9000_0000;
        }
        let at = self.next;
        for (i, &b) in bytes.iter().enumerate() {
            self.bytes.insert(at + i as u64, b);
        }
        self.next += (bytes.len() as u64).next_multiple_of(16).max(16);
        at
    }
}

#[test]
fn pools_release_newest_first_inner_pools_first() {
    let mut p = Pools::default();
    assert!(!p.add(10), "no pool: leaked");
    let outer = p.push();
    p.add(1);
    p.add(2);
    let inner = p.push();
    assert_ne!(inner, outer);
    p.add(3);
    p.add(4);
    let start = p.start_of(inner).unwrap();
    let mut got = Vec::new();
    while let Some(o) = p.pop_one(start) {
        got.push(o);
    }
    assert_eq!(got, [4, 3]);
    // A push after the pop gets the popped pool's token.
    assert_eq!(p.push(), inner);
    p.add(5);
    let start = p.start_of(outer).unwrap();
    got.clear();
    while let Some(o) = p.pop_one(start) {
        got.push(o);
    }
    assert_eq!(got, [5, 2, 1]);
    assert!(!p.is_open());
    assert_eq!(p.start_of(outer), None);
    // Nested empty pools have different tokens.
    let a = p.push();
    let b = p.push();
    assert_ne!(a, b);
}

#[test]
fn counts_and_dealloc() {
    let mut r = RefCounts::default();
    assert_eq!(r.count(7), 1);
    r.retain(7);
    r.retain(7);
    assert_eq!(r.count(7), 3);
    assert_eq!(r.release(7), Released::Alive);
    assert_eq!(r.release(7), Released::Alive);
    assert_eq!(r.release(7), Released::Dealloc);
    assert_eq!(r.count(7), 0);
    r.retain(7);
    assert_eq!(r.count(7), 0, "a retain inside -dealloc does not resurrect");
    assert_eq!(r.release(7), Released::Deallocating);
    r.freed(7);
    assert_eq!(r.count(7), 1);
}

#[test]
fn weak_locations_are_zeroed_and_moved() {
    let mut w = WeakTable::default();
    w.store(0x100, 7);
    w.store(0x108, 7);
    w.store(0x110, 8);
    w.store(0x110, 7); // re-pointed
    w.store(0x108, 0); // unregistered
    let mut z = w.clear(7);
    z.sort();
    assert_eq!(z, [0x100, 0x110]);
    assert_eq!(w.target(0x100), None);
    assert!(w.clear(8).is_empty());
}

#[test]
fn associations_by_pointer_key() {
    let mut a = Associations::default();
    assert_eq!(a.set(1, 0, Assoc { value: 9, policy: policy::RETAIN }), None);
    assert_eq!(a.set(1, 1, Assoc { value: 10, policy: policy::ASSIGN }), None);
    assert_eq!(a.get(1, 0).unwrap().value, 9);
    assert_eq!(a.get(2, 0), None);
    let old = a.set(1, 0, Assoc { value: 11, policy: policy::COPY }).unwrap();
    assert_eq!(old.value, 9);
    assert!(policy::owns(old.policy) && policy::copies(policy::COPY) && policy::atomic(policy::COPY));
    assert_eq!(a.set(1, 1, Assoc { value: 0, policy: policy::ASSIGN }).unwrap().value, 10);
    assert_eq!(a.remove_all(1).len(), 1);
    assert!(!a.has_any(1));
}

#[test]
fn sync_locks_are_recursive_per_thread() {
    let mut s = SyncLocks::default();
    assert!(s.try_enter(5, 1));
    assert!(s.try_enter(5, 1));
    assert!(!s.try_enter(5, 2));
    assert!(!s.exit(5, 2), "another thread does not own it");
    assert!(s.exit(5, 1));
    assert!(!s.try_enter(5, 2));
    assert!(s.exit(5, 1));
    assert!(!s.exit(5, 1), "one exit too many");
    assert!(s.try_enter(5, 2));
}

#[test]
fn selectors_are_unique_by_name() {
    let mut m = Sim::default();
    let mut rt = Runtime::new();
    let a = rt.sel(&mut m, b"foo:");
    assert_eq!(rt.sel(&mut m, b"foo:"), a);
    assert_ne!(rt.sel(&mut m, b"foo"), a);
    assert_eq!(m.cstr(a), b"foo:");
    let e = rt.sel(&mut m, b"");
    assert_eq!(rt.sel(&mut m, b""), e);
    // A name already in memory becomes canonical when it is new.
    let at = m.alloc(b"bar\0");
    assert_eq!(rt.sel_at(&m, at), at);
    assert_eq!(rt.sel(&mut m, b"bar"), at);
}

#[test]
fn run_time_classes() {
    let mut m = Sim::default();
    let mut rt = Runtime::new();
    // A root class, then a subclass of it.
    let (root, root_meta) = (0x5000, 0x5100);
    let name = m.alloc(b"Root\0");
    assert!(rt.allocate_class_pair(&mut m, 0, name, root, root_meta));
    assert_eq!(rt.class_named(b"Root"), None, "not before registration");
    rt.register_class_pair(&m, root);
    assert_eq!(rt.class_named(b"Root"), Some(root));
    assert_eq!(m.u64(root), root_meta);
    assert_eq!(m.u64(root_meta + 8), root, "the root metaclass's superclass is the root class");
    assert_eq!(rt.class(root).unwrap().instance_size, 8);
    let again = m.alloc(b"Root\0");
    assert!(!rt.allocate_class_pair(&mut m, 0, again, 0x6000, 0x6100), "name taken");

    let (sub, sub_meta) = (0x5200, 0x5300);
    let sname = m.alloc(b"Sub\0");
    assert!(rt.allocate_class_pair(&mut m, root, sname, sub, sub_meta));
    assert_eq!(m.u64(sub_meta + 8), root_meta);
    assert_eq!(m.u64(sub_meta), root_meta, "a metaclass's isa is the root metaclass");
    let iname = m.alloc(b"x\0");
    assert!(rt.add_ivar(sub, iname, 0, 4, 2));
    assert!(!rt.add_ivar(sub, iname, 0, 4, 2), "the name is taken");
    assert_eq!(rt.class(sub).unwrap().instance_size, 12);
    let (s2, s2m) = (0x5400, 0x5500);
    let n2 = m.alloc(b"Sub2\0");
    assert!(!rt.allocate_class_pair(&mut m, sub, n2, s2, s2m), "an unregistered superclass");
    rt.register_class_pair(&m, sub);
    assert!(!rt.add_ivar(sub, m.alloc(b"y\0"), 0, 4, 2), "no ivars after registration");

    // Methods: add, replace, lookup through the superclass, exchange.
    let hash = rt.sel(&mut m, b"hash");
    let other = rt.sel(&mut m, b"other");
    assert!(rt.add_method(&mut m, root, hash, 0xaaa, Some(b"Q16@0:8")));
    assert!(!rt.add_method(&mut m, root, hash, 0xbbb, None), "the class has it");
    assert_eq!(rt.lookup(sub, hash), Some(0xaaa));
    assert!(rt.add_method(&mut m, sub, hash, 0xccc, None));
    assert_eq!(rt.lookup(sub, hash), Some(0xccc), "never stale");
    assert_eq!(rt.lookup(root, hash), Some(0xaaa));
    assert_eq!(rt.replace_method(&mut m, sub, other, 0xddd, None), 0);
    assert_eq!(rt.replace_method(&mut m, sub, other, 0xeee, Some(b"v@:")), 0xddd);
    let h = rt.own_method(sub, other).unwrap();
    assert_eq!(m.cstr(rt.method(h).unwrap().types), b"", "a NULL type string stays empty");
    let hh = rt.own_method(sub, hash).unwrap();
    rt.exchange(h, hh);
    assert_eq!(rt.lookup(sub, hash), Some(0xeee));
    assert_eq!(rt.lookup(sub, other), Some(0xccc));
    assert!(rt.is_subclass(sub, root) && !rt.is_subclass(root, sub));
    assert!(rt.should_resolve(sub, other));
    assert!(!rt.should_resolve(sub, other), "the resolver runs once");
}
