//! CPU time per thread and per process (ADR 0044, ADR 0070), for the
//! CPU-time clocks, times(2), getrusage(2) and the CPU-time timers.
//!
//! - **How long a thread ran** is measured at every context switch: the
//!   scheduler calls [`switched`], which adds the time since the outgoing
//!   thread was switched in to its total, with the monotonic clock. The
//!   running thread's total grows by the time since it was switched in
//!   when it is read.
//! - **User or system:** each CPU keeps a [`Stint`] for the thread it runs,
//!   in time-stamp counter cycles: the architecture's entry code calls
//!   [`entered_kernel`] on every way into the kernel from ring 3 (a system
//!   call, an interrupt, an exception) and [`returning_to_user`] on every
//!   way back, and each span counts in the mode it was spent in. At a
//!   switch the stint's cycles move to the outgoing thread's slot. A
//!   thread's measured total is split in the ratio of its cycles
//!   ([`Split`]); neither part ever goes down.
//! - Accounting is per thread-table slot; a slot remembers the process and
//!   thread ids of the user thread in it ([`started`], from the scheduler).
//!   When a thread is reaped its time moves to its process
//!   ([`ProcessCpu`], in [`Process`]), so the process's total is what its
//!   ended threads used plus what its live ones have used so far.
//!
//! Every CPU switches: the table is one spinlock, taken with interrupts
//! off by each CPU's scheduler; a slot knows whether its thread runs now,
//! and on which CPU. The mode changes do not take it: a CPU's stint is
//! written only by that CPU, with interrupts off, and read by others
//! (for a thread running there) through a sequence count.

use crate::process::Process;
use crate::sync::SpinLock;
use crate::thread::{MAX_CPUS, MAX_THREADS};
use crate::time;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering, fence};
use poly_personality_linux::kernel::CpuTimes;
use poly_sched::cputime::{ModeCycles, Split, Stint};

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    /// The user thread's process and thread id; 0 for kernel threads.
    pid: u32,
    tid: u32,
    /// Time run before it was last switched in.
    ran_ns: u64,
    /// When it was last switched in.
    since: u64,
    /// Cycles in each mode before it was last switched in.
    cycles: ModeCycles,
    /// The split last reported, which later reads never go below.
    reported: Split,
    /// Running on CPU `cpu` now (since `since`).
    running: bool,
    cpu: usize,
}

impl Slot {
    /// Slot `slot`'s CPU time at `now` (a monotonic clock reading); its
    /// current stint counts if it runs.
    fn times(&mut self, slot: usize, now: u64) -> CpuTimes {
        let (total, cycles) = if self.running {
            let stint = CPUS[self.cpu].read(slot).map_or(ModeCycles::default(), |s| s.used_at(cycles_now()));
            (self.ran_ns + now.saturating_sub(self.since), self.cycles.plus(stint))
        } else {
            (self.ran_ns, self.cycles)
        };
        let split = self.reported.update(total, cycles);
        CpuTimes { user_ns: split.user_ns, system_ns: split.system_ns }
    }
}

const FREE: Slot = Slot {
    pid: 0,
    tid: 0,
    ran_ns: 0,
    since: 0,
    cycles: ModeCycles { user: 0, system: 0 },
    reported: Split { user_ns: 0, system_ns: 0 },
    running: false,
    cpu: 0,
};

struct Table {
    slots: [Slot; MAX_THREADS],
}

static TABLE: SpinLock<Table> = SpinLock::new(Table {
    slots: {
        let mut slots = [FREE; MAX_THREADS];
        // The boot thread runs from the start, on CPU 0.
        slots[0].running = true;
        slots
    },
});

fn cycles_now() -> u64 {
    crate::arch::cpu::rdtsc()
}

/// One CPU's [`Stint`]: written only by that CPU with interrupts off,
/// read by any CPU through the sequence count (odd while a write is under
/// way), for a consistent copy.
struct CpuStint {
    seq: AtomicU64,
    /// The thread-table slot of the thread the stint is for.
    slot: AtomicUsize,
    mark: AtomicU64,
    user: AtomicBool,
    used_user: AtomicU64,
    used_system: AtomicU64,
}

static CPUS: [CpuStint; MAX_CPUS] = [const {
    CpuStint {
        seq: AtomicU64::new(0),
        slot: AtomicUsize::new(0),
        mark: AtomicU64::new(0),
        user: AtomicBool::new(false),
        used_user: AtomicU64::new(0),
        used_system: AtomicU64::new(0),
    }
}; MAX_CPUS];

impl CpuStint {
    fn get(&self) -> Stint {
        Stint {
            mark: self.mark.load(Ordering::Relaxed),
            user: self.user.load(Ordering::Relaxed),
            used: ModeCycles {
                user: self.used_user.load(Ordering::Relaxed),
                system: self.used_system.load(Ordering::Relaxed),
            },
        }
    }

    /// Change the stint, on its own CPU with interrupts off (so nothing
    /// else writes it meanwhile); `slot` names the thread it is for from
    /// now on.
    fn write(&self, slot: usize, f: impl FnOnce(&mut Stint)) {
        let seq = self.seq.load(Ordering::Relaxed);
        self.seq.store(seq + 1, Ordering::Relaxed);
        fence(Ordering::Release);
        let mut s = self.get();
        f(&mut s);
        self.slot.store(slot, Ordering::Relaxed);
        self.mark.store(s.mark, Ordering::Relaxed);
        self.user.store(s.user, Ordering::Relaxed);
        self.used_user.store(s.used.user, Ordering::Relaxed);
        self.used_system.store(s.used.system, Ordering::Relaxed);
        self.seq.store(seq + 2, Ordering::Release);
    }

    /// A consistent copy, if the stint is `slot`'s.
    fn read(&self, slot: usize) -> Option<Stint> {
        loop {
            let seq = self.seq.load(Ordering::Acquire);
            if seq & 1 == 0 {
                let (s, mine) = (self.get(), self.slot.load(Ordering::Relaxed) == slot);
                fence(Ordering::Acquire);
                if self.seq.load(Ordering::Relaxed) == seq {
                    return mine.then_some(s);
                }
            }
            core::hint::spin_loop();
        }
    }
}

/// This CPU's running thread is now in user mode if `user`, else in the
/// kernel.
fn mode_change(user: bool) {
    let _irq = crate::arch::cpu::InterruptsOff::new();
    let now = cycles_now();
    let c = &CPUS[crate::thread::this_cpu()];
    let slot = c.slot.load(Ordering::Relaxed);
    c.write(slot, |s| s.enter(now, user));
}

/// The running thread entered the kernel from ring 3 (a system call, an
/// interrupt or an exception there). Called by the entry code.
pub fn entered_kernel() {
    mode_change(false);
}

/// The running thread is about to return to ring 3. Called by the exit
/// code, with interrupts off from here to the return.
pub fn returning_to_user() {
    mode_change(true);
}

/// The CPU time a process's ended threads used, and its waited-for
/// children's.
#[derive(Debug, Default)]
pub struct ProcessCpu {
    user_ns: AtomicU64,
    system_ns: AtomicU64,
    children_user_ns: AtomicU64,
    children_system_ns: AtomicU64,
    /// Page faults resolved without I/O (getrusage(2) `ru_minflt`; Poly
    /// counts every resolved fault so: nothing is ever paged out).
    minor_faults: AtomicU64,
    /// The most pages the process's memory held at once, as seen after
    /// its faults (`ru_maxrss`, in pages here).
    max_resident: AtomicU64,
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
    pub fn add_child(&self, t: CpuTimes) {
        self.children_user_ns.fetch_add(t.user_ns, Ordering::Relaxed);
        self.children_system_ns.fetch_add(t.system_ns, Ordering::Relaxed);
    }

    /// A page fault of the process was resolved; its memory now holds
    /// `resident` pages.
    pub fn fault(&self, resident: u64) {
        self.minor_faults.fetch_add(1, Ordering::Relaxed);
        self.max_resident.fetch_max(resident, Ordering::Relaxed);
    }

    /// The memory now holds `resident` pages (a peak that came without a
    /// fault: a mapping populated at once, the program's start).
    pub fn resident(&self, resident: u64) {
        self.max_resident.fetch_max(resident, Ordering::Relaxed);
    }

    /// `(minor faults, most resident pages)` so far.
    pub fn memory_counts(&self) -> (u64, u64) {
        (self.minor_faults.load(Ordering::Relaxed), self.max_resident.load(Ordering::Relaxed))
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
    TABLE.lock().slots[slot] = Slot { pid, tid, since: time::now_ns(), ..FREE };
}

/// The thread in `slot` runs on this CPU from now on, without a switch: an
/// application processor's idle thread, which is its start-up code.
pub fn running(slot: usize) {
    let cpu = crate::thread::this_cpu();
    let mut t = TABLE.lock();
    t.slots[slot].since = time::now_ns();
    t.slots[slot].running = true;
    t.slots[slot].cpu = cpu;
    let now = cycles_now();
    CPUS[cpu].write(slot, |s| *s = Stint::start(now));
}

/// This CPU switches from the thread in slot `from` to the one in `to`,
/// with interrupts off.
pub fn switched(from: usize, to: usize) {
    let now = time::now_ns();
    let cpu = crate::thread::this_cpu();
    let mut t = TABLE.lock();
    let cycles = cycles_now();
    let stint = CPUS[cpu].read(from).map_or(ModeCycles::default(), |s| s.used_at(cycles));
    CPUS[cpu].write(to, |s| *s = Stint::start(cycles));
    let s = &mut t.slots[from];
    if s.running {
        s.ran_ns += now.saturating_sub(s.since);
    }
    s.cycles = s.cycles.plus(stint);
    s.running = false;
    let s = &mut t.slots[to];
    s.since = now;
    s.running = true;
    s.cpu = cpu;
}

/// The thread in `slot` has been reaped: what it used moves to `process`.
pub fn reaped(slot: usize, process: &Process) {
    let mut t = TABLE.lock();
    let mut s = t.slots[slot];
    if s.pid == process.pid {
        s.running = false;
        process.cpu.add(s.times(slot, 0));
    }
    t.slots[slot] = FREE;
}

/// Thread `tid` of process `pid`, if it is live.
pub fn thread(pid: u32, tid: u32) -> Option<CpuTimes> {
    let now = time::now_ns();
    let mut t = TABLE.lock();
    let (i, s) = t.slots.iter_mut().enumerate().find(|(_, s)| s.pid == pid && s.tid == tid && pid != 0)?;
    Some(s.times(i, now))
}

/// Every live thread of process `pid`: `(tid, times)`.
pub fn threads_of(pid: u32) -> Vec<(u32, CpuTimes)> {
    let mut out = Vec::with_capacity(MAX_THREADS);
    let now = time::now_ns();
    let mut t = TABLE.lock();
    for (i, s) in t.slots.iter_mut().enumerate() {
        if s.pid == pid && pid != 0 {
            out.push((s.tid, s.times(i, now)));
        }
    }
    out
}

/// What `process` used: its ended threads and its live ones so far.
pub fn process(process: &Process) -> CpuTimes {
    let now = time::now_ns();
    let mut t = TABLE.lock();
    let mut total = process.cpu.ended_threads();
    for (i, s) in t.slots.iter_mut().enumerate() {
        if s.pid == process.pid && process.pid != 0 {
            let c = s.times(i, now);
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
    TABLE.lock().slots[me].times(me, now)
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
        // Sleeping 30 ms: the clock time that passed minus the CPU time
        // charged is the time the thread was off the CPU, most of the 30
        // ms. Not "charged under 20 ms": a host that stalls the emulator
        // while the thread runs (before it blocks, or after it wakes)
        // charges it for the stall, which passes on the clock too. A thread
        // charged for its sleep fails every attempt; a stall right before
        // it blocks spoils one at most.
        for _ in 0..3 {
            let (start, charged) = (time::now_ns(), current().total_ns());
            crate::thread::sleep(core::time::Duration::from_millis(30));
            let off = (time::now_ns() - start).saturating_sub(current().total_ns() - charged);
            if off >= 15 * time::NANOS_PER_MILLI {
                return Ok(());
            }
        }
        Err("a sleeping thread was charged for the time it slept")
    }
}

kernel_test! {
    /// A kernel thread never runs in ring 3: all of its time is system
    /// time, also what its current stint has used so far.
    fn a_kernel_thread_has_only_system_time() -> TestResult {
        let start = time::now_ns();
        while time::now_ns() - start < 5 * time::NANOS_PER_MILLI {
            core::hint::spin_loop();
        }
        let t = current();
        if t.user_ns != 0 || t.system_ns == 0 {
            crate::kprintln!("cputime: kernel thread: user {} ns, system {} ns", t.user_ns, t.system_ns);
            return Err("a kernel thread's time was not all system time");
        }
        Ok(())
    }
}
