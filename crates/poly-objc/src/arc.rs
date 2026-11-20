//! Memory management tables, kept outside the objects: reference counts,
//! zeroing weak references, associated objects, autorelease pools and the
//! recursive locks of `@synchronized`. The caller sends the messages these
//! tables decide on (`-release`, `-dealloc`, `-copyWithZone:`).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

/// What `release` found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Released {
    /// References remain.
    Alive,
    /// That was the last one: send `-dealloc`.
    Dealloc,
    /// The object is already being deallocated (nothing to do).
    Deallocating,
}

/// Reference counts beyond the first, and objects being deallocated.
#[derive(Debug, Default)]
pub struct RefCounts {
    extra: BTreeMap<u64, u64>,
    dying: BTreeSet<u64>,
}

impl RefCounts {
    /// One more reference.
    pub fn retain(&mut self, obj: u64) {
        if !self.dying.contains(&obj) {
            *self.extra.entry(obj).or_insert(0) += 1;
        }
    }

    /// One fewer.
    pub fn release(&mut self, obj: u64) -> Released {
        if self.dying.contains(&obj) {
            return Released::Deallocating;
        }
        match self.extra.get_mut(&obj) {
            Some(n) if *n > 1 => {
                *n -= 1;
                Released::Alive
            }
            Some(_) => {
                self.extra.remove(&obj);
                Released::Alive
            }
            None => {
                self.dying.insert(obj);
                Released::Dealloc
            }
        }
    }

    /// What `-retainCount` answers: 0 while deallocating.
    pub fn count(&self, obj: u64) -> u64 {
        if self.dying.contains(&obj) { 0 } else { 1 + self.extra.get(&obj).copied().unwrap_or(0) }
    }

    pub fn is_deallocating(&self, obj: u64) -> bool {
        self.dying.contains(&obj)
    }

    /// The object's memory is freed: forget it.
    pub fn freed(&mut self, obj: u64) {
        self.dying.remove(&obj);
        self.extra.remove(&obj);
    }
}

/// Zeroing weak references: object -> the locations that refer to it.
#[derive(Debug, Default)]
pub struct WeakTable {
    by_object: BTreeMap<u64, BTreeSet<u64>>,
    by_location: BTreeMap<u64, u64>,
}

impl WeakTable {
    /// Register `location` as a weak reference to `obj` (0: none),
    /// leaving any earlier registration of the location.
    pub fn store(&mut self, location: u64, obj: u64) {
        self.unregister(location);
        if obj != 0 {
            self.by_object.entry(obj).or_default().insert(location);
            self.by_location.insert(location, obj);
        }
    }

    /// Forget `location`.
    pub fn unregister(&mut self, location: u64) {
        if let Some(old) = self.by_location.remove(&location)
            && let Some(set) = self.by_object.get_mut(&old)
        {
            set.remove(&location);
            if set.is_empty() {
                self.by_object.remove(&old);
            }
        }
    }

    /// The object `location` is registered for.
    pub fn target(&self, location: u64) -> Option<u64> {
        self.by_location.get(&location).copied()
    }

    /// `obj` dies: the locations to set to nil (they are forgotten).
    pub fn clear(&mut self, obj: u64) -> Vec<u64> {
        let locs: Vec<u64> = self.by_object.remove(&obj).map(|s| s.into_iter().collect()).unwrap_or_default();
        for l in &locs {
            self.by_location.remove(l);
        }
        locs
    }
}

/// `objc_AssociationPolicy` (`<objc/runtime.h>`).
pub mod policy {
    pub const ASSIGN: u64 = 0;
    pub const RETAIN_NONATOMIC: u64 = 1;
    pub const COPY_NONATOMIC: u64 = 3;
    pub const RETAIN: u64 = 0o1401;
    pub const COPY: u64 = 0o1403;

    /// Whether the association owns a reference to its value.
    pub const fn owns(p: u64) -> bool {
        matches!(p, RETAIN_NONATOMIC | COPY_NONATOMIC | RETAIN | COPY)
    }

    /// Whether the value is copied when set.
    pub const fn copies(p: u64) -> bool {
        matches!(p, COPY_NONATOMIC | COPY)
    }

    /// Whether a get retains and autoreleases the value.
    pub const fn atomic(p: u64) -> bool {
        matches!(p, RETAIN | COPY)
    }
}

/// One association.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Assoc {
    pub value: u64,
    pub policy: u64,
}

/// Associated objects: (object, key) -> value.
#[derive(Debug, Default)]
pub struct Associations {
    map: BTreeMap<u64, BTreeMap<u64, Assoc>>,
}

impl Associations {
    /// Set (or with `value` 0 remove) an association; returns the old one,
    /// which the caller releases if its policy owned it.
    pub fn set(&mut self, obj: u64, key: u64, a: Assoc) -> Option<Assoc> {
        if a.value == 0 {
            let inner = self.map.get_mut(&obj)?;
            let old = inner.remove(&key);
            if inner.is_empty() {
                self.map.remove(&obj);
            }
            return old;
        }
        self.map.entry(obj).or_default().insert(key, a)
    }

    pub fn get(&self, obj: u64, key: u64) -> Option<Assoc> {
        self.map.get(&obj)?.get(&key).copied()
    }

    /// `obj` dies (or `objc_removeAssociatedObjects`): its associations,
    /// for the caller to release.
    pub fn remove_all(&mut self, obj: u64) -> Vec<Assoc> {
        self.map.remove(&obj).map(|m| m.into_values().collect()).unwrap_or_default()
    }

    pub fn has_any(&self, obj: u64) -> bool {
        self.map.contains_key(&obj)
    }
}

/// One thread's autorelease pools: one stack of objects, with a 0 entry
/// where each pool starts (objects are never 0). A token names the
/// position of its pool's 0, so nested pools have different tokens and a
/// pool pushed after a pop gets the popped pool's token.
#[derive(Debug, Default)]
pub struct Pools {
    stack: Vec<u64>,
    open: usize,
}

/// Tokens are positions in the stack, from this base (never 0).
const TOKEN_BASE: u64 = 0x1000;

impl Pools {
    /// `objc_autoreleasePoolPush`: a token naming the new pool.
    pub fn push(&mut self) -> u64 {
        self.stack.push(0);
        self.open += 1;
        TOKEN_BASE + 8 * (self.stack.len() as u64 - 1)
    }

    /// Whether a pool is open.
    pub fn is_open(&self) -> bool {
        self.open > 0
    }

    /// `objc_autorelease`: false if no pool is open (the object leaks).
    pub fn add(&mut self, obj: u64) -> bool {
        if self.open == 0 || obj == 0 {
            return false;
        }
        self.stack.push(obj);
        true
    }

    /// The position a token names; `None` for a token of no open pool.
    pub fn start_of(&self, token: u64) -> Option<usize> {
        let pos = usize::try_from(token.checked_sub(TOKEN_BASE)? / 8).ok()?;
        (self.stack.get(pos) == Some(&0)).then_some(pos)
    }

    /// One step of `objc_autoreleasePoolPop(token)`: the next object to
    /// release (newest first, inner pools before outer ones), or `None`
    /// when the pool and the ones above it are empty, at which point they
    /// are closed. The caller releases each object with no lock held (a
    /// `-dealloc` may autorelease into the pool being popped, or push and
    /// pop a pool of its own).
    pub fn pop_one(&mut self, start: usize) -> Option<u64> {
        while self.stack.len() > start + 1 {
            let top = self.stack.pop().unwrap_or(0);
            if top != 0 {
                return Some(top);
            }
            self.open -= 1;
        }
        if self.stack.len() == start + 1 {
            self.stack.pop();
            self.open -= 1;
        }
        None
    }

    /// The number of objects waiting in open pools.
    pub fn pending(&self) -> usize {
        self.stack.iter().filter(|&&o| o != 0).count()
    }
}

/// The recursive locks of `@synchronized`: object -> (owner, depth).
#[derive(Debug, Default)]
pub struct SyncLocks {
    held: BTreeMap<u64, (u64, u64)>,
}

impl SyncLocks {
    /// Try to enter `obj`'s lock as thread `tid`: false while another
    /// thread holds it.
    pub fn try_enter(&mut self, obj: u64, tid: u64) -> bool {
        match self.held.get_mut(&obj) {
            Some((owner, depth)) if *owner == tid => {
                *depth += 1;
                true
            }
            Some(_) => false,
            None => {
                self.held.insert(obj, (tid, 1));
                true
            }
        }
    }

    /// Leave one level: false if `tid` does not hold the lock
    /// (`OBJC_SYNC_NOT_OWNING_THREAD_ERROR`).
    pub fn exit(&mut self, obj: u64, tid: u64) -> bool {
        match self.held.get_mut(&obj) {
            Some((owner, depth)) if *owner == tid => {
                *depth -= 1;
                if *depth == 0 {
                    self.held.remove(&obj);
                }
                true
            }
            _ => false,
        }
    }
}
