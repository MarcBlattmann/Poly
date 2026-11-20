//! ulock_wait / ulock_wait2 / ulock_wake: compare-and-wait on a word in
//! user memory, the kernel side of libSystem's locks
//! (`specs/macos/bsd/sys_ulock_wait.yaml`, `sys_ulock_wait2.yaml`,
//! `sys_ulock_wake.yaml`).
//!
//! There is no public header: the operation numbers and flags are the ones
//! the spec sheets record. This module decodes the operation and keeps the
//! waiters; reading the word, blocking and the errno conventions are the
//! personality's.

use crate::waitq::{ThreadId, WaitQueue};

pub const UL_COMPARE_AND_WAIT: u32 = 1;
pub const UL_UNFAIR_LOCK: u32 = 2;
pub const UL_COMPARE_AND_WAIT_SHARED: u32 = 3;
pub const UL_COMPARE_AND_WAIT64: u32 = 5;
/// Wake every waiter (ulock_wake).
pub const ULF_WAKE_ALL: u32 = 0x100;
/// Errors come back as the negative errno, with the carry flag clear.
pub const ULF_NO_ERRNO: u32 = 0x0100_0000;

/// A decoded operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Op {
    pub kind: u32,
    pub wake_all: bool,
    pub no_errno: bool,
}

impl Op {
    /// Decode an operation word; `None` for an operation that does not
    /// exist (`EINVAL`).
    pub fn decode(op: u32) -> Option<Op> {
        let kind = op & 0xff;
        if !matches!(kind, UL_COMPARE_AND_WAIT | UL_UNFAIR_LOCK | UL_COMPARE_AND_WAIT_SHARED | UL_COMPARE_AND_WAIT64) {
            return None;
        }
        Some(Op { kind, wake_all: op & ULF_WAKE_ALL != 0, no_errno: op & ULF_NO_ERRNO != 0 })
    }

    /// The size of the word: 8 for the 64-bit operation, else 4. The
    /// address must be aligned to it.
    pub fn word_size(self) -> u64 {
        if self.kind == UL_COMPARE_AND_WAIT64 { 8 } else { 4 }
    }

    /// Whether waiters in other processes can meet on the word.
    pub fn shared(self) -> bool {
        self.kind == UL_COMPARE_AND_WAIT_SHARED
    }
}

/// Where waiters meet: an address in one task, or a place in a memory
/// object that several tasks map (the kernel names it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    Private { task: u64, addr: u64 },
    Shared { object: u64, offset: u64 },
}

/// The waiters of every word. A woken waiter's result is the number of
/// threads still waiting on the word when it was woken.
#[derive(Debug, Default)]
pub struct UlockTable {
    q: WaitQueue<Key, u32>,
}

impl UlockTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue(&mut self, key: Key, thread: ThreadId) {
        self.q.enqueue(key, thread);
    }

    /// Wake one waiter, or all with `all`; the threads woken (none:
    /// ulock_wake fails with `ENOENT`).
    pub fn wake(&mut self, key: Key, all: bool) -> alloc::vec::Vec<ThreadId> {
        let mut woken = alloc::vec::Vec::new();
        if all {
            self.q.wake_all(key, || 0, &mut |t| woken.push(t));
        } else {
            let left = self.q.waiters(key).saturating_sub(1) as u32;
            if let Some(t) = self.q.wake_one(key, left) {
                woken.push(t);
            }
        }
        woken
    }

    /// The result of a woken wait.
    pub fn take(&mut self, thread: ThreadId) -> Option<u32> {
        self.q.take(thread)
    }

    pub fn cancel(&mut self, thread: ThreadId) -> bool {
        self.q.cancel(thread)
    }

    pub fn waiters(&self, key: Key) -> usize {
        self.q.waiters(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_operations() {
        assert_eq!(Op::decode(0), None);
        assert_eq!(Op::decode(99), None);
        let o = Op::decode(UL_COMPARE_AND_WAIT | ULF_NO_ERRNO).unwrap();
        assert!(o.no_errno && !o.wake_all && !o.shared());
        assert_eq!(Op::decode(UL_COMPARE_AND_WAIT64).unwrap().word_size(), 8);
        assert!(Op::decode(UL_COMPARE_AND_WAIT_SHARED | ULF_WAKE_ALL).unwrap().wake_all);
    }

    #[test]
    fn wake_one_reports_who_is_left() {
        let mut t = UlockTable::new();
        let k = Key::Private { task: 1, addr: 0x1000 };
        t.enqueue(k, 7);
        t.enqueue(k, 8);
        assert_eq!(t.wake(k, false), [7]);
        assert_eq!(t.take(7), Some(1));
        assert_eq!(t.wake(k, true), [8]);
        assert_eq!(t.take(8), Some(0));
        assert!(t.wake(k, false).is_empty());
    }
}
