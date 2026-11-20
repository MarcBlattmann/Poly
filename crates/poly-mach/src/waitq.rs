//! Threads waiting on keys, and the results their wakers left them.
//!
//! The crate never blocks. A call that has to wait puts the caller's thread
//! in a [`WaitQueue`] under a key and tells the kernel to block it; a waker
//! takes the thread off the queue, leaves a result for it and asks the kernel
//! to unblock it. When the blocked thread runs again it takes its result
//! ([`WaitQueue::take`]); if there is none it timed out or was interrupted,
//! and [`WaitQueue::cancel`] takes it off the queue. Everything runs under
//! the one lock the kernel holds around the Mach state, so a wake that races
//! with a timeout is decided by whoever takes the lock first.

use alloc::collections::{BTreeMap, VecDeque};

/// A kernel thread id.
pub type ThreadId = u64;

#[derive(Debug)]
pub struct WaitQueue<K: Ord + Copy, R> {
    queues: BTreeMap<K, VecDeque<ThreadId>>,
    waiting_on: BTreeMap<ThreadId, K>,
    results: BTreeMap<ThreadId, R>,
}

impl<K: Ord + Copy, R> Default for WaitQueue<K, R> {
    fn default() -> Self {
        WaitQueue { queues: BTreeMap::new(), waiting_on: BTreeMap::new(), results: BTreeMap::new() }
    }
}

impl<K: Ord + Copy, R> WaitQueue<K, R> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue `thread` (last) under `key`. A thread waits on one key at a
    /// time; queueing it again moves it.
    pub fn enqueue(&mut self, key: K, thread: ThreadId) {
        self.cancel(thread);
        self.results.remove(&thread);
        self.queues.entry(key).or_default().push_back(thread);
        self.waiting_on.insert(thread, key);
    }

    /// Wake the first thread waiting on `key` with `result`.
    pub fn wake_one(&mut self, key: K, result: R) -> Option<ThreadId> {
        let q = self.queues.get_mut(&key)?;
        let t = q.pop_front()?;
        if q.is_empty() {
            self.queues.remove(&key);
        }
        self.waiting_on.remove(&t);
        self.results.insert(t, result);
        Some(t)
    }

    /// Wake `thread` if it waits on `key`.
    pub fn wake_thread(&mut self, key: K, thread: ThreadId, result: R) -> bool {
        if self.waiting_on.get(&thread) != Some(&key) {
            return false;
        }
        self.cancel(thread);
        self.results.insert(thread, result);
        true
    }

    /// Wake every thread waiting on `key`; `result` makes each one's result.
    pub fn wake_all(&mut self, key: K, mut result: impl FnMut() -> R, woken: &mut dyn FnMut(ThreadId)) -> usize {
        let Some(q) = self.queues.remove(&key) else { return 0 };
        let n = q.len();
        for t in q {
            self.waiting_on.remove(&t);
            self.results.insert(t, result());
            woken(t);
        }
        n
    }

    /// The result a waker left for `thread`, if any.
    pub fn take(&mut self, thread: ThreadId) -> Option<R> {
        self.results.remove(&thread)
    }

    /// Take `thread` off whatever queue it is on; whether it was on one.
    pub fn cancel(&mut self, thread: ThreadId) -> bool {
        let Some(key) = self.waiting_on.remove(&thread) else { return false };
        if let Some(q) = self.queues.get_mut(&key) {
            q.retain(|&t| t != thread);
            if q.is_empty() {
                self.queues.remove(&key);
            }
        }
        true
    }

    /// The key `thread` waits on.
    pub fn key_of(&self, thread: ThreadId) -> Option<K> {
        self.waiting_on.get(&thread).copied()
    }

    /// How many threads wait on `key`.
    pub fn waiters(&self, key: K) -> usize {
        self.queues.get(&key).map_or(0, |q| q.len())
    }

    /// The keys that have waiters.
    pub fn keys(&self) -> impl Iterator<Item = K> + '_ {
        self.queues.keys().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_in_order_and_cancel() {
        let mut q: WaitQueue<u32, i32> = WaitQueue::new();
        q.enqueue(1, 10);
        q.enqueue(1, 11);
        q.enqueue(2, 12);
        assert_eq!(q.waiters(1), 2);
        assert_eq!(q.wake_one(1, 5), Some(10));
        assert_eq!(q.take(10), Some(5));
        assert!(q.cancel(11));
        assert_eq!(q.wake_one(1, 5), None);
        assert!(!q.wake_thread(1, 12, 0));
        assert!(q.wake_thread(2, 12, 7));
        assert_eq!(q.take(12), Some(7));
        assert_eq!(q.take(12), None);
    }

    #[test]
    fn wake_all_leaves_results() {
        let mut q: WaitQueue<u32, i32> = WaitQueue::new();
        q.enqueue(3, 1);
        q.enqueue(3, 2);
        let mut seen = alloc::vec::Vec::new();
        assert_eq!(q.wake_all(3, || 0, &mut |t| seen.push(t)), 2);
        assert_eq!(seen, [1, 2]);
        assert_eq!(q.take(1), Some(0));
        assert_eq!(q.key_of(2), None);
    }
}
