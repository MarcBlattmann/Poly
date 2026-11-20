//! psynch: the kernel side of libpthread's mutexes and condition variables
//! (`specs/macos/bsd/psynch_*.yaml`).
//!
//! The arguments are the library's sequence words. What the sheets record,
//! and this module keeps:
//!
//! - cvwait waits with a relative timeout; a timeout is `ETIMEDOUT` in the
//!   low byte of the error, sequence state above it (`0x100 | 60` for the
//!   observed arguments). The words are not read.
//! - cvsignal wakes one waiter. When none waits in the kernel but the
//!   sequence word says one has registered in user space (`cvlsgen`: the
//!   lock sequence in the low 32 bits, the signal sequence in the high 32,
//!   counted in steps of `0x100`), the signal is kept (a pre-post) and the
//!   next cvwait on the word returns at once.
//! - cvbroad wakes every waiter of its word only.
//! - mutexwait blocks until a mutexdrop of the same word; mutexdrop with
//!   nobody waiting succeeds.

use crate::ulock::Key;
use crate::waitq::{ThreadId, WaitQueue};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// One sequence step (libpthread counts in units of `0x100`; the low byte
/// holds flags).
pub const SEQ_INC: u32 = 0x100;

/// How many waiters the sequence word `cvlsgen` says have registered in
/// user space and not been signalled yet.
pub fn user_waiters(cvlsgen: u64) -> u32 {
    let lseq = (cvlsgen as u32) & !0xff;
    let sseq = ((cvlsgen >> 32) as u32) & !0xff;
    lseq.wrapping_sub(sseq) / SEQ_INC
}

/// What a cvwait returns when it does not time out: the sequence it was
/// given, with one step consumed (`0x101` for `0x100`).
pub fn cv_result(cvlsgen: u64) -> u32 {
    ((cvlsgen as u32) & !0xff) | 1
}

/// What a timed-out cvwait fails with: the errno in the low byte, the
/// sequence state above it.
pub fn cv_error(cvlsgen: u64, errno: u32) -> u32 {
    ((cvlsgen as u32) & 0xff00) | (errno & 0xff)
}

#[derive(Debug, Default)]
pub struct PsynchTable {
    cv: WaitQueue<Key, u32>,
    prepost: BTreeMap<Key, u32>,
    mutex: WaitQueue<Key, u32>,
}

impl PsynchTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a cvwait: `Some(result)` if a pre-posted signal was there to
    /// consume, otherwise `thread` is queued and must block.
    pub fn cvwait(&mut self, key: Key, cvlsgen: u64, thread: ThreadId) -> Option<u32> {
        if let Some(n) = self.prepost.get_mut(&key) {
            *n -= 1;
            if *n == 0 {
                self.prepost.remove(&key);
            }
            return Some(cv_result(cvlsgen));
        }
        self.cv.enqueue(key, thread);
        None
    }

    /// cvsignal: wake one waiter (`thread`, if given and waiting), or keep
    /// the signal for a waiter registered in user space.
    pub fn cvsignal(&mut self, key: Key, cvlsgen: u64, thread: Option<ThreadId>) -> Vec<ThreadId> {
        let result = cv_result(cvlsgen);
        let woken = match thread {
            Some(t) if self.cv.wake_thread(key, t, result) => Some(t),
            _ => self.cv.wake_one(key, result),
        };
        match woken {
            Some(t) => alloc::vec![t],
            None => {
                self.post(key, cvlsgen);
                Vec::new()
            }
        }
    }

    /// cvbroad: wake every waiter of the word, or keep the signal for the
    /// waiters registered in user space.
    pub fn cvbroad(&mut self, key: Key, cvlsgen: u64) -> Vec<ThreadId> {
        let result = cv_result(cvlsgen);
        let mut woken = Vec::new();
        self.cv.wake_all(key, || result, &mut |t| woken.push(t));
        if woken.is_empty() {
            self.post(key, cvlsgen);
        }
        woken
    }

    fn post(&mut self, key: Key, cvlsgen: u64) {
        let want = user_waiters(cvlsgen);
        let have = self.prepost.get(&key).copied().unwrap_or(0);
        if have < want {
            self.prepost.insert(key, have + 1);
        }
    }

    /// Start a mutexwait: `thread` is queued and must block.
    pub fn mutexwait(&mut self, key: Key, thread: ThreadId) {
        self.mutex.enqueue(key, thread);
    }

    /// mutexdrop: hand the mutex to the first waiter, if any; `mgen` is what
    /// the woken mutexwait returns.
    pub fn mutexdrop(&mut self, key: Key, mgen: u32) -> Vec<ThreadId> {
        self.mutex.wake_one(key, mgen).into_iter().collect()
    }

    /// The result of a woken wait (cvwait or mutexwait).
    pub fn take(&mut self, thread: ThreadId) -> Option<u32> {
        self.cv.take(thread).or_else(|| self.mutex.take(thread))
    }

    pub fn cancel(&mut self, thread: ThreadId) -> bool {
        self.cv.cancel(thread) | self.mutex.cancel(thread)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: Key = Key::Private { task: 1, addr: 0x2000 };
    const OTHER: Key = Key::Private { task: 1, addr: 0x2040 };

    #[test]
    fn sequence_words() {
        assert_eq!(user_waiters(0x100), 1);
        assert_eq!(user_waiters(0x100 | (0x100 << 32)), 0);
        assert_eq!(cv_result(0x100), 257);
        assert_eq!(cv_error(0x100, 60), 316);
    }

    #[test]
    fn signal_is_preposted_once() {
        let mut t = PsynchTable::new();
        assert!(t.cvsignal(K, 0x100, None).is_empty());
        assert!(t.cvsignal(K, 0x100, None).is_empty());
        assert_eq!(t.cvwait(K, 0x100, 5), Some(257));
        assert_eq!(t.cvwait(K, 0x100, 5), None);
        assert!(t.cancel(5));
    }

    #[test]
    fn broadcast_wakes_only_its_word() {
        let mut t = PsynchTable::new();
        assert_eq!(t.cvwait(OTHER, 0, 9), None);
        assert!(t.cvbroad(K, 0).is_empty());
        assert_eq!(t.take(9), None);
        assert_eq!(t.cvbroad(OTHER, 0), [9]);
        assert_eq!(t.take(9), Some(1));
    }

    #[test]
    fn mutex_drop_wakes_a_waiter() {
        let mut t = PsynchTable::new();
        assert!(t.mutexdrop(K, 0).is_empty());
        t.mutexwait(K, 3);
        assert_eq!(t.mutexdrop(K, 0x200), [3]);
        assert_eq!(t.take(3), Some(0x200));
    }
}
