//! [`WaitQueue`]: threads blocked until some condition holds.

use super::*;

/// Threads blocked until some condition holds, woken by whoever changes it.
pub struct WaitQueue(pub(super) UnsafeCell<poly_sched::WaitQueue>);

// SAFETY: the inner queue is only accessed with THREADS locked.
unsafe impl Sync for WaitQueue {}

impl core::fmt::Debug for WaitQueue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("WaitQueue")
    }
}

impl WaitQueue {
    pub const fn new() -> WaitQueue {
        WaitQueue(UnsafeCell::new(poly_sched::WaitQueue::new()))
    }

    /// Block until `ready()` returns true. `ready` runs with the scheduler
    /// locked and interrupts off, so a waker that makes it true and then
    /// calls [`wake_one`](Self::wake_one) or [`wake_all`](Self::wake_all)
    /// (on any CPU) cannot slip in between the check and going to sleep: no
    /// lost wakeups. It must be short and must not call into this module or
    /// take a lock (atomics and reading the clock are fine).
    pub fn wait_until(&self, mut ready: impl FnMut() -> bool) {
        let _irq = InterruptsOff::new();
        loop {
            let sw = {
                let mut t = THREADS.lock();
                if ready() {
                    return;
                }
                // SAFETY: THREADS is locked.
                let q = unsafe { &mut *self.0.get() };
                t.sched.block_current(this_cpu(), q).expect("an idle thread cannot sleep")
            };
            switch(sw);
        }
    }

    /// Like [`wait_until`](Self::wait_until), but give up once the clock
    /// reads `deadline_ns` or later: true if `ready()` returned true, false
    /// on the timeout. The thread is woken by a wake on this queue or by the
    /// first tick after the deadline, whichever comes first (a tick wakes
    /// the whole queue; the other waiters check their conditions and go
    /// back to sleep).
    pub fn wait_until_deadline(&self, mut ready: impl FnMut() -> bool, deadline_ns: u64) -> bool {
        assert!(time::is_running(), "timed wait before the timer is running");
        let _irq = InterruptsOff::new();
        loop {
            let sw = {
                let mut t = THREADS.lock();
                let me = current().slot();
                if ready() {
                    t.timeouts[me] = None;
                    return true;
                }
                if time::now_ns() >= deadline_ns {
                    t.timeouts[me] = None;
                    return false;
                }
                t.timeouts[me] = Some(Timeout { deadline: deadline_ns, queue: self as *const WaitQueue as usize });
                NEXT_WAKE.fetch_min(deadline_ns, SeqCst);
                // SAFETY: THREADS is locked.
                let q = unsafe { &mut *self.0.get() };
                t.sched.block_current(this_cpu(), q).expect("an idle thread cannot sleep")
            };
            switch(sw);
        }
    }

    /// Wake the longest-waiting thread. False if none was waiting.
    pub fn wake_one(&self) -> bool {
        let _irq = InterruptsOff::new();
        let (woke, kicks) = {
            let mut t = THREADS.lock();
            // SAFETY: THREADS is locked.
            let woke = t.sched.wake_one(unsafe { &mut *self.0.get() }).is_some();
            (woke, t.sched.take_kicks())
        };
        kick(kicks);
        woke
    }

    /// Wake every waiting thread; returns how many there were.
    pub fn wake_all(&self) -> usize {
        let _irq = InterruptsOff::new();
        let (n, kicks) = {
            let mut t = THREADS.lock();
            // SAFETY: THREADS is locked.
            let n = t.sched.wake_all(unsafe { &mut *self.0.get() });
            (n, t.sched.take_kicks())
        };
        kick(kicks);
        n
    }
}
