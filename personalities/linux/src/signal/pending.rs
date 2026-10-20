//! Pending signals: one queue per thread and one shared by the process.
//!
//! signal(7), "Real-time signals": "Multiple instances of real-time
//! signals can be queued. By contrast, if multiple instances of a standard
//! signal are delivered while that signal is currently blocked, then only
//! one instance is queued." So a standard signal is pending at most once
//! (the first sender's siginfo is kept), while every real-time signal is
//! queued with its own siginfo, "in the order they were sent".

use super::action::SYNCHRONOUS;
use super::action::is_realtime;
use crate::abi::SigInfo;
use crate::kernel::SigSet;
use alloc::vec::Vec;

/// Which of the `candidates` is delivered first.
///
/// signal(7): real-time signals are "delivered starting with the
/// lowest-numbered signal"; "if both standard and real-time signals are
/// pending for a process ... Linux, like many other implementations, gives
/// priority to standard signals". The order among standard signals is
/// unspecified; Poly takes the synchronous ones ([`SYNCHRONOUS`]) first,
/// then the lowest-numbered (to be confirmed by the diff runner).
pub fn next_signal(candidates: SigSet) -> Option<u32> {
    candidates
        .intersection(SYNCHRONOUS)
        .lowest()
        .or_else(|| candidates.intersection(SigSet::STANDARD).lowest())
        .or_else(|| candidates.lowest())
}

/// One queue of pending signals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SigQueue {
    set: SigSet,
    /// In arrival order; at most one entry per standard signal.
    entries: Vec<SigInfo>,
}

impl SigQueue {
    pub const fn new() -> Self {
        SigQueue { set: SigSet::EMPTY, entries: Vec::new() }
    }

    /// The signals with at least one instance queued.
    pub const fn set(&self) -> SigSet {
        self.set
    }

    pub const fn contains(&self, signal: u32) -> bool {
        self.set.contains(signal)
    }

    /// Queue `info` (signal 1-64). Returns false if it was a standard
    /// signal that is already pending: the instances merge and the queued
    /// siginfo stays the first one's.
    pub fn push(&mut self, info: SigInfo) -> bool {
        let signal = info.signal();
        if !is_realtime(signal) && self.set.contains(signal) {
            return false;
        }
        self.set.insert(signal);
        self.entries.push(info);
        true
    }

    /// Take the oldest instance of `signal`.
    pub fn take(&mut self, signal: u32) -> Option<SigInfo> {
        let i = self.entries.iter().position(|e| e.signal() == signal)?;
        let info = self.entries.remove(i);
        if !self.entries.iter().any(|e| e.signal() == signal) {
            self.set.remove(signal);
        }
        Some(info)
    }

    /// Discard every instance of the signals in `signals`.
    pub fn discard(&mut self, signals: SigSet) {
        self.entries.retain(|e| !signals.contains(e.signal()));
        self.set = self.set.without(signals);
    }

    /// The queued siginfos, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &SigInfo> + '_ {
        self.entries.iter()
    }

    /// The oldest queued siginfo `f` picks, to change in place (a POSIX
    /// timer's overrun count while its signal is pending).
    pub fn find_mut(&mut self, f: impl Fn(&SigInfo) -> bool) -> Option<&mut SigInfo> {
        self.entries.iter_mut().find(|e| f(e))
    }

    /// Queued real-time instances (they count against the queue limit).
    pub fn realtime_len(&self) -> usize {
        self.entries.iter().filter(|e| is_realtime(e.signal())).count()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::signal::*;

    fn info(sig: u32, pid: i32) -> SigInfo {
        SigInfo::from_process(sig, SI_QUEUE, pid, 0)
    }

    #[test]
    fn standard_signals_coalesce_and_keep_the_first_siginfo() {
        let mut q = SigQueue::new();
        assert!(q.push(info(SIGUSR1, 1)));
        assert!(!q.push(info(SIGUSR1, 2)));
        assert_eq!(q.len(), 1);
        assert_eq!(q.take(SIGUSR1).map(|i| i.pid()), Some(1));
        assert_eq!(q.take(SIGUSR1), None);
        assert!(q.set().is_empty());
    }

    #[test]
    fn realtime_signals_queue_in_order() {
        let mut q = SigQueue::new();
        for pid in 1..=3 {
            assert!(q.push(info(SIGRTMIN + 1, pid)));
        }
        q.push(info(SIGRTMIN, 9));
        assert_eq!(q.realtime_len(), 4);
        assert_eq!(q.take(SIGRTMIN + 1).map(|i| i.pid()), Some(1));
        assert!(q.contains(SIGRTMIN + 1), "two more instances");
        assert_eq!(q.take(SIGRTMIN + 1).map(|i| i.pid()), Some(2));
        assert_eq!(q.take(SIGRTMIN + 1).map(|i| i.pid()), Some(3));
        assert!(!q.contains(SIGRTMIN + 1) && q.contains(SIGRTMIN));
        q.discard(SigSet::only(SIGRTMIN));
        assert!(q.is_empty() && q.set().is_empty());
    }

    #[test]
    fn delivery_order() {
        let set = |sigs: &[u32]| sigs.iter().fold(SigSet::EMPTY, |s, &x| s.union(SigSet::only(x)));
        assert_eq!(next_signal(SigSet::EMPTY), None);
        assert_eq!(next_signal(set(&[SIGRTMIN + 3, SIGRTMIN + 1])), Some(SIGRTMIN + 1), "lowest RT first");
        assert_eq!(next_signal(set(&[NSIG, SIGTERM])), Some(SIGTERM), "standard before RT");
        assert_eq!(next_signal(set(&[SIGINT, SIGHUP, SIGTERM])), Some(SIGHUP), "lowest standard");
        assert_eq!(next_signal(set(&[SIGHUP, SIGSEGV, SIGRTMIN])), Some(SIGSEGV), "synchronous first");
        assert_eq!(next_signal(set(&[SIGSYS, SIGBUS])), Some(SIGBUS));
    }
}
