//! CPU time per thread and per process (ADR 0044), for the CPU-time clocks,
//! times(2), getrusage(2) and the CPU-time timers.
//!
//! - **How long a thread ran** is measured at every context switch: the
//!   scheduler calls [`switched`], which adds the time since the outgoing
//!   thread was switched in to its total, with the monotonic clock. The
//!   running thread's total grows by the time since it was switched in
//!   when it is read.
//! - **User or system** comes from sampling: every timer tick counts one
//!   sample for the running thread, as user time if the tick interrupted
//!   ring 3 and as system time otherwise ([`tick`]). A thread's measured
//!   total is split in the ratio of its samples (all of it is system time
//!   until the first sample). Neither part ever goes down: a read clamps
//!   the split so that both are at least what the thread last reported,
//!   since a new sample shifts the ratio.
//! - Accounting is per thread-table slot; a slot remembers the process and
//!   thread ids of the user thread in it ([`started`], from the scheduler).
//!   When a thread is reaped its time moves to its process
//!   ([`ProcessCpu`], in [`Process`]), so the process's total is what its
//!   ended threads used plus what its live ones have used so far.
//!
//! Every CPU switches and ticks: the table is one spinlock, taken with
//! interrupts off by each CPU's scheduler and tick; a slot knows whether
//! its thread runs (on some CPU) now, and each tick samples the thread
//! running on the CPU that took it.

use crate::process::Process;
use crate::sync::SpinLock;
use crate::thread::MAX_THREADS;
use crate::time;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use poly_personality_linux::kernel::CpuTimes;

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    /// The user thread's process and thread id; 0 for kernel threads.
    pid: u32,
    tid: u32,
    /// Time run before it was last switched in.
    ran_ns: u64,
    /// When it was last switched in.
    since: u64,
    user_ticks: u64,
    system_ticks: u64,
    /// The split last reported, which later reads never go below.
    reported_user_ns: u64,
    reported_system_ns: u64,
    /// Running on some CPU now (since `since`).
    running: bool,
}

impl Slot {
    /// The slot's CPU time at `now`, if it is the running one.
    fn times(&mut self, running: bool, now: u64) -> CpuTimes {
        let total = self.ran_ns + if running { now.saturating_sub(self.since) } else { 0 };
        let samples = self.user_ticks + self.system_ticks;
        let ideal_user_ns = if samples == 0 {
            0
        } else {
            (u128::from(total) * u128::from(self.user_ticks) / u128::from(samples)) as u64
        };
        // The total only grows, so the range is never empty (and both
        // bounds are 0 for a fresh slot).
        let floor = self.reported_user_ns;
        let ceiling = total.saturating_sub(self.reported_system_ns).max(floor);
        let user_ns = ideal_user_ns.clamp(floor, ceiling);
        let system_ns = total.saturating_sub(user_ns).max(self.reported_system_ns);
        self.reported_user_ns = user_ns;
        self.reported_system_ns = system_ns;
        CpuTimes { user_ns, system_ns }
    }
}

struct Table {
    slots: [Slot; MAX_THREADS],
}

static TABLE: SpinLock<Table> = SpinLock::new(Table {
    slots: {
        let free = Slot {
            pid: 0,
            tid: 0,
            ran_ns: 0,
            since: 0,
            user_ticks: 0,
            system_ticks: 0,
            reported_user_ns: 0,
            reported_system_ns: 0,
            running: false,
        };
        let mut slots = [free; MAX_THREADS];
        // The boot thread runs from the start.
        slots[0].running = true;
        slots
    },
});

/// The CPU time a process's ended threads used, and its waited-for
/// children's.
#[derive(Debug, Default)]
pub struct ProcessCpu {
    user_ns: AtomicU64,
    system_ns: AtomicU64,
    children_user_ns: AtomicU64,
    children_system_ns: AtomicU64,
}

impl ProcessCpu {
    fn add(&self, t: CpuTimes) {
        self.user_ns.fetch_add(t.user_ns, Ordering::Relaxed);
        self.system_ns.fetch_add(t.system_ns, Ordering::Relaxed);
    }

    fn ended_threads(&self) -> CpuTimes {
        CpuTimes { user_ns: self.user_ns.load(Ordering::Relaxed), system_ns: self.system_ns.load(Ordering::Relaxed) }
    }

    /// A child that has been waited for used `t` (it and its own waited-for
    /// children): getrusage(2) `RUSAGE_CHILDREN`.
    #[allow(dead_code, reason = "wait4 (the processes milestone) adds what a reaped child used")]
    pub fn add_child(&self, t: CpuTimes) {
        self.children_user_ns.fetch_add(t.user_ns, Ordering::Relaxed);
        self.children_system_ns.fetch_add(t.system_ns, Ordering::Relaxed);
    }

    pub fn children(&self) -> CpuTimes {
        CpuTimes {
            user_ns: self.children_user_ns.load(Ordering::Relaxed),
            system_ns: self.children_system_ns.load(Ordering::Relaxed),
        }
    }
}

/// A thread starts in `slot`: user thread `ids` (process id, thread id),
/// or a kernel thread (`None`). Its time starts at 0.
pub fn started(slot: usize, ids: Option<(u32, u32)>) {
    let (pid, tid) = ids.unwrap_or((0, 0));
    TABLE.lock().slots[slot] = Slot { pid, tid, since: time::now_ns(), ..Slot::default() };
}

/// The thread in `slot` runs from now on, without a switch: an
/// application processor's idle thread, which is its start-up code.
pub fn running(slot: usize) {
    let mut t = TABLE.lock();
    t.slots[slot].since = time::now_ns();
    t.slots[slot].running = true;
}

/// A CPU switches from the thread in slot `from` to the one in `to`.
pub fn switched(from: usize, to: usize) {
    let now = time::now_ns();
    let mut t = TABLE.lock();
    let s = &mut t.slots[from];
    if s.running {
        s.ran_ns += now.saturating_sub(s.since);
    }
    s.running = false;
    let s = &mut t.slots[to];
    s.since = now;
    s.running = true;
}

/// A timer tick interrupted the thread running on this CPU, in ring 3 if
/// `user`.
pub fn tick(user: bool) {
    let running = crate::thread::current().slot();
    let mut t = TABLE.lock();
    let s = &mut t.slots[running];
    if user {
        s.user_ticks += 1;
    } else {
        s.system_ticks += 1;
    }
}

/// The thread in `slot` has been reaped: what it used moves to `process`.
pub fn reaped(slot: usize, process: &Process) {
    let mut t = TABLE.lock();
    let mut s = t.slots[slot];
    if s.pid == process.pid {
        process.cpu.add(s.times(false, 0));
    }
    t.slots[slot] = Slot::default();
}

/// Thread `tid` of process `pid`, if it is live.
pub fn thread(pid: u32, tid: u32) -> Option<CpuTimes> {
    let now = time::now_ns();
    let mut t = TABLE.lock();
    let s = t.slots.iter_mut().find(|s| s.pid == pid && s.tid == tid && pid != 0)?;
    let running = s.running;
    Some(s.times(running, now))
}

/// Every live thread of process `pid`: `(tid, times)`.
pub fn threads_of(pid: u32) -> Vec<(u32, CpuTimes)> {
    let mut out = Vec::with_capacity(MAX_THREADS);
    let now = time::now_ns();
    let mut t = TABLE.lock();
    for s in t.slots.iter_mut() {
        if s.pid == pid && pid != 0 {
            let running = s.running;
            out.push((s.tid, s.times(running, now)));
        }
    }
    out
}

/// What `process` used: its ended threads and its live ones so far.
pub fn process(process: &Process) -> CpuTimes {
    let now = time::now_ns();
    let mut t = TABLE.lock();
    let mut total = process.cpu.ended_threads();
    for s in t.slots.iter_mut() {
        if s.pid == process.pid && process.pid != 0 {
            let running = s.running;
            let c = s.times(running, now);
            total.user_ns += c.user_ns;
            total.system_ns += c.system_ns;
        }
    }
    total
}

/// The running thread's CPU time (kernel threads too).
pub fn current() -> CpuTimes {
    let now = time::now_ns();
    let me = crate::thread::current().slot();
    TABLE.lock().slots[me].times(true, now)
}

// --- Tests ------------------------------------------------------------------

use crate::kernel_test;

kernel_test! {
    fn the_running_thread_accumulates_cpu_time() -> TestResult {
        let start = time::now_ns();
        let before = current();
        while time::now_ns() - start < 20 * time::NANOS_PER_MILLI {
            core::hint::spin_loop();
        }
        let after = current();
        let ran = after.total_ns() - before.total_ns();
        // The thread may be preempted meanwhile, but it ran for some of it.
        if ran == 0 || ran > time::now_ns() - start {
            crate::kprintln!("cputime: {ran} ns of CPU time over 20 ms");
            return Err("a busy kernel thread's CPU time did not grow plausibly");
        }
        let start = current().total_ns();
        crate::thread::sleep(core::time::Duration::from_millis(30));
        if current().total_ns() - start > 20 * time::NANOS_PER_MILLI {
            return Err("a sleeping thread was charged for the time it slept");
        }
        Ok(())
    }
}

kernel_test! {
    /// A new sample shifts the user / system ratio; neither part may go
    /// down for it (getrusage(2) and times(2) callers subtract readings).
    fn the_split_never_goes_down() -> TestResult {
        let mut s = Slot { ran_ns: 1000, user_ticks: 1, system_ticks: 1, ..Slot::default() };
        let mut last = s.times(false, 0);
        let steps: [(u64, bool); 6] = [(10, true), (10, true), (10, true), (500, false), (0, false), (0, false)];
        for (ran, user) in steps {
            s.ran_ns += ran;
            if user {
                s.user_ticks += 1;
            } else {
                s.system_ticks += 1;
            }
            let now = s.times(false, 0);
            if now.user_ns < last.user_ns || now.system_ns < last.system_ns {
                return Err("a part of the CPU time went down");
            }
            if now.user_ns + now.system_ns != s.ran_ns {
                return Err("the parts do not add up to the total");
            }
            last = now;
        }
        Ok(())
    }
}
