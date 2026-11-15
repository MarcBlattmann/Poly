//! Thread pool rules (Learn: "Thread Pools", "Using the Thread Pool Functions", `TP_CALLBACK_ENVIRON`,
//! `CreateThreadpool`, `SetThreadpoolThreadMinimum`, `SetThreadpoolTimer`): the sizing of a pool,
//! the callback environment's layout and the due-time arithmetic. The threads, the locks and the
//! kernel objects are `kernelbase`'s (`threadpool.rs` there); what is decided here is pure.

use alloc::collections::VecDeque;

/// The default pool's thread limit (Learn: `SetThreadpoolThreadMaximum`: the default is 500).
pub const DEFAULT_MAXIMUM: u32 = 500;
/// What a wait object's callback is told (`WAIT_OBJECT_0`, `WAIT_TIMEOUT`).
pub const WAIT_OBJECT_0: u32 = 0;
pub const WAIT_TIMEOUT: u32 = 258;
/// How long an idle pool thread above the minimum waits for work before it ends, in 100 ns.
pub const IDLE_THREAD_TIMEOUT: i64 = 100_000_000;

/// Where the fields of `TP_CALLBACK_ENVIRON_V3` (`winnt.h`) are, in a process whose pointers are
/// `ptr` bytes: `Version` (a `ULONG`) at 0, then six pointers, `Flags`, `CallbackPriority`, `Size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvLayout {
    pub pool: usize,
    pub cleanup_group: usize,
    pub cancel_callback: usize,
    pub finalization: usize,
    pub flags: usize,
}

impl EnvLayout {
    pub const fn new(ptr: usize) -> EnvLayout {
        EnvLayout { pool: ptr, cleanup_group: 2 * ptr, cancel_callback: 3 * ptr, finalization: 6 * ptr, flags: 7 * ptr }
    }
}

/// The monotonic time (100 ns) at which a due time passes: a negative `FILETIME` is relative to
/// `now`; a positive one is an absolute system time (100 ns since 1601), which `system_now` is
/// the current value of; zero and times in the past are due now.
pub fn deadline(due: i64, now: i64, system_now: i64) -> i64 {
    if due < 0 { now.saturating_add(due.saturating_neg()) } else { now.saturating_add((due - system_now).max(0)) }
}

/// A `NtWaitFor*` timeout (100 ns, negative relative) for sleeping until `deadline`.
pub fn wait_timeout(deadline: i64, now: i64) -> i64 {
    -(deadline.saturating_sub(now).max(0))
}

/// What a submit asks the pool to do: wake a sleeping thread, start a new one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dispatch {
    pub wake: bool,
    pub spawn: bool,
}

/// What a pool thread does next.
#[derive(Debug, PartialEq, Eq)]
pub enum Next<J> {
    /// Run this job.
    Run(J),
    /// Sleep until woken (for at most [`IDLE_THREAD_TIMEOUT`] if the thread may end).
    Sleep { may_end: bool },
    /// The pool is closing and has nothing left: the thread ends.
    Exit,
}

/// A pool's queue and thread counts. Everything happens under the caller's lock.
#[derive(Debug)]
pub struct PoolState<J> {
    pub min: u32,
    pub max: u32,
    /// Threads that exist (running a job, sleeping, or about to start).
    pub threads: u32,
    /// Threads asleep waiting for a job.
    pub idle: u32,
    pub queue: VecDeque<J>,
    pub closing: bool,
}

impl<J> Default for PoolState<J> {
    fn default() -> Self {
        PoolState { min: 0, max: DEFAULT_MAXIMUM, threads: 0, idle: 0, queue: VecDeque::new(), closing: false }
    }
}

impl<J> PoolState<J> {
    /// Queue `job`. A sleeper is woken whenever there is one; a thread is started when more jobs
    /// wait than there are sleepers to take them, up to the maximum (so two callbacks that wait for
    /// each other both run, and a pool limited to one thread runs them in turn). A `spawn` has
    /// already been counted in [`threads`](Self::threads); undo it with [`spawn_failed`](Self::spawn_failed).
    pub fn push(&mut self, job: J) -> Dispatch {
        self.queue.push_back(job);
        let wake = self.idle > 0;
        let spawn = self.queue.len() > self.idle as usize && self.threads < self.max;
        if spawn {
            self.threads += 1;
        }
        Dispatch { wake, spawn }
    }

    /// A thread that was to start did not.
    pub fn spawn_failed(&mut self) {
        self.threads = self.threads.saturating_sub(1);
    }

    /// What the calling thread does next: take a job, or note it sleeps (`idle`).
    pub fn take_next(&mut self) -> Next<J> {
        if let Some(j) = self.queue.pop_front() {
            return Next::Run(j);
        }
        if self.closing {
            self.threads = self.threads.saturating_sub(1);
            return Next::Exit;
        }
        self.idle += 1;
        Next::Sleep { may_end: self.threads > self.min }
    }

    /// The thread woke up (signalled or timed out): it is no longer asleep.
    pub fn woke(&mut self) {
        self.idle = self.idle.saturating_sub(1);
    }

    /// A woken thread that found nothing for [`IDLE_THREAD_TIMEOUT`]: whether it ends (yes while
    /// the pool has more threads than its minimum).
    pub fn idle_exit(&mut self) -> bool {
        if self.queue.is_empty() && self.threads > self.min {
            self.threads -= 1;
            true
        } else {
            false
        }
    }

    /// `SetThreadpoolThreadMinimum`: raising the minimum above the maximum raises the maximum.
    /// Returns how many threads to start now (a pool keeps at least its minimum alive).
    pub fn set_min(&mut self, min: u32) -> u32 {
        self.min = min;
        if self.max < min {
            self.max = min;
        }
        let start = min.saturating_sub(self.threads);
        self.threads += start;
        start
    }

    /// `SetThreadpoolThreadMaximum`: at least one thread; a minimum above it comes down to it.
    pub fn set_max(&mut self, max: u32) {
        self.max = max.max(1);
        if self.min > self.max {
            self.min = self.max;
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::sync::{Arc, Condvar, Mutex};
    use std::vec::Vec;

    #[test]
    fn env_layout_of_both_pointer_sizes() {
        let l = EnvLayout::new(8);
        assert_eq!((l.pool, l.cleanup_group, l.cancel_callback, l.finalization, l.flags), (8, 16, 24, 48, 56));
        let l = EnvLayout::new(4);
        assert_eq!((l.pool, l.cleanup_group, l.cancel_callback, l.finalization, l.flags), (4, 8, 12, 24, 28));
    }

    #[test]
    fn deadlines() {
        assert_eq!(deadline(-1_500_000, 1000, 5_000_000), 1_501_000);
        assert_eq!(deadline(0, 1000, 5_000_000), 1000);
        assert_eq!(deadline(1, 1000, 5_000_000), 1000, "an absolute time in the past is due now");
        assert_eq!(deadline(5_100_000, 1000, 5_000_000), 101_000);
        assert_eq!(deadline(i64::MIN, 10, 0), i64::MAX, "no overflow");
        assert_eq!(wait_timeout(200, 100), -100);
        assert_eq!(wait_timeout(100, 200), 0);
    }

    #[test]
    fn spawn_wake_and_limits() {
        let mut p = PoolState::<u32>::default();
        // Nobody asleep: each job gets a thread until the maximum.
        p.set_max(2);
        assert_eq!(p.push(1), Dispatch { wake: false, spawn: true });
        assert_eq!(p.push(2), Dispatch { wake: false, spawn: true });
        assert_eq!(p.push(3), Dispatch { wake: false, spawn: false }, "at the maximum the job waits");
        assert_eq!(p.threads, 2);
        // The threads run the jobs in order, then sleep.
        assert_eq!(p.take_next(), Next::Run(1));
        assert_eq!(p.take_next(), Next::Run(2));
        assert_eq!(p.take_next(), Next::Run(3));
        assert_eq!(p.take_next(), Next::Sleep { may_end: true });
        // One sleeper: a job wakes it and starts nobody; a second job in a row needs a second thread.
        p.set_max(3);
        let d = p.push(4);
        assert_eq!(d, Dispatch { wake: true, spawn: false });
        assert_eq!(p.push(5), Dispatch { wake: true, spawn: true });
        p.woke();
        assert_eq!(p.take_next(), Next::Run(4));
        assert_eq!(p.threads, 3);
        p.spawn_failed();
        assert_eq!(p.threads, 2);
    }

    #[test]
    fn minimum_keeps_threads_and_raises_the_maximum() {
        let mut p = PoolState::<u32>::default();
        p.set_max(3);
        assert_eq!(p.set_min(5), 5);
        assert_eq!((p.min, p.max, p.threads), (5, 5, 5));
        assert_eq!(p.set_min(2), 0, "lowering starts nothing");
        assert_eq!(p.take_next(), Next::Sleep { may_end: true });
        // Above the minimum a thread that found nothing ends; at it, it stays.
        p.woke();
        assert!(p.idle_exit());
        assert_eq!(p.threads, 4);
        p.set_min(4);
        assert!(!p.idle_exit());
        p.set_max(1);
        assert_eq!((p.min, p.max), (1, 1));
    }

    #[test]
    fn closing_drains_the_queue_then_ends_the_threads() {
        let mut p = PoolState::<u32>::default();
        assert!(p.push(1).spawn);
        p.closing = true;
        assert_eq!(p.take_next(), Next::Run(1));
        assert_eq!(p.take_next(), Next::Exit);
        assert_eq!(p.threads, 0);
    }

    /// Many producers and workers over one lock: every job runs once, no thread count goes wrong,
    /// and with a maximum of `max` never more than `max` jobs run at once.
    #[test]
    fn workers_and_producers_under_load() {
        for max in [1u32, 2, 5] {
            let state = Arc::new((Mutex::new(PoolState::<u32>::default()), Condvar::new()));
            state.0.lock().unwrap().set_max(max);
            let done = Arc::new(Mutex::new(Vec::new()));
            let (running, peak) = (Arc::new(Mutex::new(0u32)), Arc::new(Mutex::new(0u32)));
            let handles = Arc::new(Mutex::new(Vec::new()));
            fn spawn_worker(
                state: Arc<(Mutex<PoolState<u32>>, Condvar)>,
                done: Arc<Mutex<Vec<u32>>>,
                running: Arc<Mutex<u32>>,
                peak: Arc<Mutex<u32>>,
                handles: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
            ) {
                let h2 = handles.clone();
                let h = std::thread::spawn(move || {
                    let mut g = state.0.lock().unwrap();
                    loop {
                        match g.take_next() {
                            Next::Run(j) => {
                                drop(g);
                                {
                                    let mut r = running.lock().unwrap();
                                    *r += 1;
                                    let mut p = peak.lock().unwrap();
                                    *p = (*p).max(*r);
                                }
                                std::thread::sleep(std::time::Duration::from_micros(200));
                                *running.lock().unwrap() -= 1;
                                done.lock().unwrap().push(j);
                                g = state.0.lock().unwrap();
                            }
                            Next::Exit => return,
                            Next::Sleep { .. } => {
                                g = state.1.wait(g).unwrap();
                                g.woke();
                            }
                        }
                    }
                });
                h2.lock().unwrap().push(h);
            }
            let producers: Vec<_> = (0..4u32)
                .map(|p| {
                    let (state, done, running, peak, handles) =
                        (state.clone(), done.clone(), running.clone(), peak.clone(), handles.clone());
                    std::thread::spawn(move || {
                        for i in 0..200u32 {
                            let d = state.0.lock().unwrap().push(p * 1000 + i);
                            if d.wake {
                                state.1.notify_one();
                            }
                            if d.spawn {
                                spawn_worker(
                                    state.clone(),
                                    done.clone(),
                                    running.clone(),
                                    peak.clone(),
                                    handles.clone(),
                                );
                            }
                        }
                    })
                })
                .collect();
            for p in producers {
                p.join().unwrap();
            }
            while done.lock().unwrap().len() < 800 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            {
                let mut g = state.0.lock().unwrap();
                g.closing = true;
            }
            state.1.notify_all();
            for h in std::mem::take(&mut *handles.lock().unwrap()) {
                h.join().unwrap();
            }
            let mut d = done.lock().unwrap().clone();
            d.sort_unstable();
            d.dedup();
            assert_eq!(d.len(), 800, "every job ran exactly once");
            assert!(*peak.lock().unwrap() <= max, "peak {} over the maximum {max}", peak.lock().unwrap());
            assert_eq!(state.0.lock().unwrap().threads, 0);
        }
    }
}
