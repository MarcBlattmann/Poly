//! Futex wait queues: which threads wait on which futex word, in which
//! order, and for which bits.
//!
//! This part knows nothing about Linux's encodings or user memory: it is
//! the queue bookkeeping behind `FUTEX_WAIT` / `FUTEX_WAKE` /
//! `FUTEX_REQUEUE` and could serve other "wait on an address" interfaces
//! too. The kernel owns one table for the whole system and locks it
//! around every use ([`crate::kernel::LinuxKernel::with_futexes`]).
//!
//! Waiters on one word are woken first come, first served (Poly has no
//! thread priorities yet). Each waiter carries a bitset; a wake only takes
//! waiters whose bitset shares a bit with the waker's (futex(2),
//! `FUTEX_WAIT_BITSET`). Requeued waiters keep their place order and their
//! bitset and go to the back of the target queue.

use alloc::collections::{BTreeMap, VecDeque};

/// What identifies a futex word. Two waits find each other only with equal
/// keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FutexKey {
    /// A private futex (`FUTEX_PRIVATE_FLAG`): the address space id and
    /// the user address.
    Private { space: u64, addr: u64 },
    /// A shared futex (no `FUTEX_PRIVATE_FLAG`) in memory only one address
    /// space maps: keyed like [`FutexKey::Private`] but never the same
    /// futex, since a private waiter is not found by a shared wake (and the
    /// other way round; futex's spec sheet).
    Unshared { space: u64, addr: u64 },
    /// A word in memory several address spaces can map: the backing
    /// object's id (a file, a shared memory segment) and the offset in it.
    Shared { object: u64, offset: u64 },
}

/// A waiting thread, by thread id.
pub type Waiter = i32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    waiter: Waiter,
    bitset: u32,
}

/// Every waiting thread, by key, in arrival order.
#[derive(Clone, Debug, Default)]
pub struct FutexTable {
    queues: BTreeMap<FutexKey, VecDeque<Entry>>,
    keys: BTreeMap<Waiter, FutexKey>,
}

impl FutexTable {
    pub const fn new() -> Self {
        FutexTable { queues: BTreeMap::new(), keys: BTreeMap::new() }
    }

    /// Queue `waiter` on `key` with `bitset` (not 0). Returns false, and
    /// changes nothing, if the waiter is already queued somewhere: a thread
    /// waits on one futex at a time.
    pub fn enqueue(&mut self, key: FutexKey, waiter: Waiter, bitset: u32) -> bool {
        if self.keys.contains_key(&waiter) {
            return false;
        }
        self.keys.insert(waiter, key);
        self.queues.entry(key).or_default().push_back(Entry { waiter, bitset });
        true
    }

    /// Dequeue up to `max` waiters on `key` whose bitset shares a bit with
    /// `bitset`, oldest first, calling `woken` for each. Returns how many.
    pub fn wake(&mut self, key: FutexKey, max: usize, bitset: u32, woken: &mut dyn FnMut(Waiter)) -> usize {
        let Some(queue) = self.queues.get_mut(&key) else { return 0 };
        let mut n = 0;
        let mut i = 0;
        while n < max && i < queue.len() {
            if queue[i].bitset & bitset != 0 {
                let e = queue.remove(i).expect("index in range");
                self.keys.remove(&e.waiter);
                woken(e.waiter);
                n += 1;
            } else {
                i += 1;
            }
        }
        if queue.is_empty() {
            self.queues.remove(&key);
        }
        n
    }

    /// Wake up to `max_wake` waiters on `from` (any bitset), then move up
    /// to `max_requeue` of the rest, oldest first, to the back of `to`.
    /// Returns `(woken, requeued)`.
    pub fn requeue(
        &mut self,
        from: FutexKey,
        to: FutexKey,
        max_wake: usize,
        max_requeue: usize,
        woken: &mut dyn FnMut(Waiter),
    ) -> (usize, usize) {
        let n_woken = self.wake(from, max_wake, u32::MAX, woken);
        let Some(queue) = self.queues.get_mut(&from) else { return (n_woken, 0) };
        let n = max_requeue.min(queue.len());
        let moved: VecDeque<Entry> = queue.drain(..n).collect();
        if queue.is_empty() {
            self.queues.remove(&from);
        }
        for e in &moved {
            self.keys.insert(e.waiter, to);
        }
        self.queues.entry(to).or_default().extend(moved);
        (n_woken, n)
    }

    /// Take `waiter` off its queue (after a timeout or a signal). Returns
    /// whether it was still queued; false means someone woke it.
    pub fn remove(&mut self, waiter: Waiter) -> bool {
        let Some(key) = self.keys.remove(&waiter) else { return false };
        let queue = self.queues.get_mut(&key).expect("a queued waiter's key has a queue");
        queue.retain(|e| e.waiter != waiter);
        if queue.is_empty() {
            self.queues.remove(&key);
        }
        true
    }

    /// The key `waiter` is queued on, if it is.
    pub fn key_of(&self, waiter: Waiter) -> Option<FutexKey> {
        self.keys.get(&waiter).copied()
    }

    /// The waiters on `key`, oldest first.
    pub fn waiters(&self, key: FutexKey) -> impl Iterator<Item = Waiter> + '_ {
        self.queues.get(&key).into_iter().flat_map(|q| q.iter().map(|e| e.waiter))
    }

    /// The number of queued waiters.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec::Vec;

    const A: FutexKey = FutexKey::Private { space: 1, addr: 0x1000 };
    const B: FutexKey = FutexKey::Private { space: 1, addr: 0x1004 };
    const C: FutexKey = FutexKey::Shared { object: 1, offset: 0x1000 };

    fn wake(t: &mut FutexTable, key: FutexKey, max: usize, bitset: u32) -> Vec<Waiter> {
        let mut v = Vec::new();
        t.wake(key, max, bitset, &mut |w| v.push(w));
        v
    }

    #[test]
    fn first_come_first_served() {
        let mut t = FutexTable::new();
        for w in 1..=5 {
            assert!(t.enqueue(A, w, u32::MAX));
        }
        assert!(!t.enqueue(B, 3, u32::MAX), "one futex at a time");
        assert_eq!(wake(&mut t, A, 2, u32::MAX), [1, 2]);
        assert_eq!(wake(&mut t, B, 2, u32::MAX), [] as [Waiter; 0]);
        assert_eq!(wake(&mut t, A, 0, u32::MAX), [] as [Waiter; 0]);
        assert_eq!(wake(&mut t, A, usize::MAX, u32::MAX), [3, 4, 5]);
        assert!(t.is_empty());
    }

    #[test]
    fn keys_are_distinct() {
        let mut t = FutexTable::new();
        t.enqueue(A, 1, u32::MAX);
        t.enqueue(C, 2, u32::MAX);
        assert_eq!(wake(&mut t, C, 9, u32::MAX), [2], "a shared key never matches a private one");
        assert_eq!(t.key_of(1), Some(A));
    }

    #[test]
    fn bitsets_select_waiters() {
        let mut t = FutexTable::new();
        t.enqueue(A, 1, 0b01);
        t.enqueue(A, 2, 0b10);
        t.enqueue(A, 3, 0b11);
        assert_eq!(wake(&mut t, A, 9, 0b10), [2, 3]);
        assert_eq!(wake(&mut t, A, 9, 0b10), [] as [Waiter; 0]);
        assert_eq!(wake(&mut t, A, 9, u32::MAX), [1]);
    }

    #[test]
    fn requeue_moves_to_the_back() {
        let mut t = FutexTable::new();
        t.enqueue(B, 9, 0b100);
        for w in 1..=5 {
            t.enqueue(A, w, 0b100);
        }
        let mut woken = Vec::new();
        assert_eq!(t.requeue(A, B, 1, 3, &mut |w| woken.push(w)), (1, 3));
        assert_eq!(woken, [1]);
        assert_eq!(t.waiters(A).collect::<Vec<_>>(), [5]);
        assert_eq!(t.waiters(B).collect::<Vec<_>>(), [9, 2, 3, 4]);
        assert_eq!(t.key_of(3), Some(B));
        assert_eq!(wake(&mut t, B, 9, 0b100), [9, 2, 3, 4], "bitsets kept");
    }

    #[test]
    fn remove_after_timeout() {
        let mut t = FutexTable::new();
        t.enqueue(A, 1, 1);
        t.enqueue(A, 2, 1);
        assert!(t.remove(1));
        assert!(!t.remove(1), "already gone");
        assert_eq!(wake(&mut t, A, 9, 1), [2]);
        assert!(!t.remove(2), "woken, not queued");
        assert!(t.queues.is_empty() && t.keys.is_empty());
    }

    /// A small deterministic generator (xorshift64*), so the model test is
    /// reproducible without a dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// The obvious model: one list of (waiter, key, bitset) in arrival
    /// order. Requeued waiters move to the end, which puts them behind
    /// every waiter already on the target key.
    #[derive(Default)]
    struct Model(Vec<(Waiter, FutexKey, u32)>);

    impl Model {
        fn wake(&mut self, key: FutexKey, max: usize, bitset: u32) -> Vec<Waiter> {
            let mut woken = Vec::new();
            self.0.retain(|&(w, k, b)| {
                if woken.len() < max && k == key && b & bitset != 0 {
                    woken.push(w);
                    false
                } else {
                    true
                }
            });
            woken
        }

        fn requeue(&mut self, from: FutexKey, to: FutexKey, wake: usize, requeue: usize) -> (Vec<Waiter>, usize) {
            let woken = self.wake(from, wake, u32::MAX);
            let mut moved = Vec::new();
            self.0.retain(|&(w, k, b)| {
                if moved.len() < requeue && k == from {
                    moved.push((w, to, b));
                    false
                } else {
                    true
                }
            });
            let n = moved.len();
            self.0.extend(moved);
            (woken, n)
        }
    }

    #[test]
    fn randomized_against_a_model() {
        let keys = [A, B, C, FutexKey::Private { space: 2, addr: 0x1000 }];
        for seed in 1..=20u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
            let mut t = FutexTable::new();
            let mut m = Model::default();
            for _ in 0..400 {
                let key = keys[rng.below(4) as usize];
                match rng.below(5) {
                    0 | 1 => {
                        let w = rng.below(24) as Waiter;
                        let bitset = [u32::MAX, 1, 2, 3][rng.below(4) as usize];
                        let queued = m.0.iter().any(|e| e.0 == w);
                        assert_eq!(t.enqueue(key, w, bitset), !queued);
                        if !queued {
                            m.0.push((w, key, bitset));
                        }
                    }
                    2 => {
                        let max = rng.below(4) as usize;
                        let bitset = [u32::MAX, 1, 2][rng.below(3) as usize];
                        assert_eq!(wake(&mut t, key, max, bitset), m.wake(key, max, bitset));
                    }
                    3 => {
                        let to = keys[rng.below(4) as usize];
                        let (w, r) = (rng.below(3) as usize, rng.below(4) as usize);
                        let mut woken = Vec::new();
                        let (nw, nr) = t.requeue(key, to, w, r, &mut |x| woken.push(x));
                        let (mw, mr) = m.requeue(key, to, w, r);
                        assert_eq!((woken.clone(), nr), (mw, mr));
                        assert_eq!(nw, woken.len());
                    }
                    _ => {
                        let w = rng.below(24) as Waiter;
                        let before = m.0.len();
                        m.0.retain(|e| e.0 != w);
                        assert_eq!(t.remove(w), m.0.len() != before);
                    }
                }
                assert_eq!(t.len(), m.0.len());
                for &k in &keys {
                    let model: Vec<Waiter> = m.0.iter().filter(|e| e.1 == k).map(|e| e.0).collect();
                    assert_eq!(t.waiters(k).collect::<Vec<_>>(), model, "seed {seed}");
                }
            }
        }
    }
}
