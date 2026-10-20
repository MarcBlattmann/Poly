//! The kernel side of readiness (ADR 0042): how a user thread blocks on
//! file wait queues, and the timers timer files use.
//!
//! - [`ThreadBlocker`] is `poly_vfs::Blocker` for a thread of a process:
//!   its waker unparks the thread ([`process::unpark`]); blocking is
//!   [`process::park`], which also ends on the thread's signal notification
//!   or the end of its process (then the wait is interrupted and the
//!   system call layer decides between `EINTR` and a restart).
//! - [`timers`] is `poly_vfs::timerfd::TimerService`: the monotonic clock,
//!   the wall clock, and one-shot callbacks run by the `timers` kernel
//!   thread (started at the first use), which sleeps until the earliest
//!   one is due. Callbacks run in that thread with no lock held, never in
//!   an interrupt handler, so they may notify wait queues.

use crate::process::{self, Process, ThreadFlags, Unparked};
use crate::sync::{SetOnce, SpinLock};
use crate::thread::{self, WaitQueue};
use crate::{kprintln, time};
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use poly_vfs::timerfd::{TimerService, TimerTarget};
use poly_vfs::{Blocker, PollEvents, Waker, Woke};

/// Wakes one thread from [`process::park`].
struct ThreadWaker(Arc<ThreadFlags>);

impl Waker for ThreadWaker {
    fn wake(&self, _: PollEvents) {
        process::unpark(&self.0);
    }
}

/// `poly_vfs::Blocker` for the running thread of `process`.
pub struct ThreadBlocker<'a> {
    process: &'a Process,
    flags: Arc<ThreadFlags>,
    waker: Arc<dyn Waker>,
}

impl<'a> ThreadBlocker<'a> {
    /// The blocker of the running thread, whose flags are `flags`.
    pub fn new(process: &'a Process, flags: Arc<ThreadFlags>) -> ThreadBlocker<'a> {
        let waker = Arc::new(ThreadWaker(Arc::clone(&flags)));
        ThreadBlocker { process, flags, waker }
    }
}

impl core::fmt::Debug for ThreadBlocker<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ThreadBlocker").field("pid", &self.process.pid).finish_non_exhaustive()
    }
}

impl Blocker for ThreadBlocker<'_> {
    fn waker(&self) -> Arc<dyn Waker> {
        Arc::clone(&self.waker)
    }

    fn block(&mut self, until: Option<u64>) -> Woke {
        match process::park(self.process, &self.flags, until) {
            Unparked::Woken => Woke::Woken,
            Unparked::TimedOut => Woke::TimedOut,
            Unparked::Interrupted => Woke::Interrupted,
        }
    }
}

// ---- timers ------------------------------------------------------------------

/// The callbacks waiting for their time, by (time, id), and each id's time.
#[derive(Default)]
struct Table {
    due: BTreeMap<(u64, u64), Weak<dyn TimerTarget>>,
    by_id: BTreeMap<u64, u64>,
}

static TABLE: SpinLock<Option<Table>> = SpinLock::new(None);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// The timer thread sleeps here; `KICKED` says the earliest time moved.
static WAKE: WaitQueue = WaitQueue::new();
static KICKED: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);

/// The kernel's timer service.
#[derive(Debug)]
pub struct KernelTimers;

static SERVICE: SetOnce<Arc<KernelTimers>> = SetOnce::new();

/// The timer service (one for the machine).
pub fn timers() -> Arc<KernelTimers> {
    if let Some(s) = SERVICE.get() {
        return Arc::clone(s);
    }
    match SERVICE.set(Arc::new(KernelTimers)) {
        Ok(s) => Arc::clone(s),
        // Someone else is setting it: it is the same zero-sized service.
        Err(s) => s,
    }
}

/// Callbacks waiting (for tests).
pub fn pending_timers() -> usize {
    TABLE.lock().as_ref().map_or(0, |t| t.by_id.len())
}

impl TimerService for KernelTimers {
    fn monotonic_ns(&self) -> u64 {
        time::now_ns()
    }

    fn realtime_ns(&self) -> u64 {
        time::realtime_ns()
    }

    fn arm(&self, at_ns: u64, target: Weak<dyn TimerTarget>) -> u64 {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let earliest = {
            let mut t = TABLE.lock();
            let table = t.get_or_insert_with(Table::default);
            table.due.insert((at_ns, id), target);
            table.by_id.insert(id, at_ns);
            table.due.keys().next() == Some(&(at_ns, id))
        };
        start_thread();
        if earliest {
            KICKED.store(true, Ordering::SeqCst);
            WAKE.wake_all();
        }
        id
    }

    fn cancel(&self, id: u64) {
        let removed = {
            let mut t = TABLE.lock();
            t.as_mut().and_then(|table| {
                let at = table.by_id.remove(&id)?;
                table.due.remove(&(at, id))
            })
        };
        // The target's last reference (a timer file) goes unlocked.
        drop(removed);
    }
}

fn start_thread() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    if thread::spawn_named("timers", timer_thread, 0).is_err() {
        STARTED.store(false, Ordering::SeqCst);
        kprintln!("fs: cannot start the timer thread; timer files only expire when read");
    }
}

/// Run every callback that is due, then sleep until the next one (or a
/// new earlier one).
fn timer_thread(_: usize) -> usize {
    loop {
        let now = time::now_ns();
        let (due, next): (Vec<Weak<dyn TimerTarget>>, Option<u64>) = {
            let mut t = TABLE.lock();
            let table = t.get_or_insert_with(Table::default);
            let mut due = Vec::new();
            while let Some(entry) = table.due.first_entry() {
                let (at, id) = *entry.key();
                if at > now {
                    break;
                }
                due.push(entry.remove());
                table.by_id.remove(&id);
            }
            (due, table.due.keys().next().map(|&(at, _)| at))
        };
        for target in due {
            if let Some(target) = target.upgrade() {
                target.expire();
            }
        }
        let kicked = || KICKED.swap(false, Ordering::SeqCst);
        match next {
            Some(at) if at <= time::now_ns() => {}
            Some(at) => {
                WAKE.wait_until_deadline(kicked, at);
            }
            None => WAKE.wait_until(kicked),
        }
    }
}
