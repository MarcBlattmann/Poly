//! Scheduler bookkeeping for kernel threads on one or more CPUs: which
//! thread runs next on each CPU, which threads wait, when the running one
//! has used up its time slice, and where a thread that becomes runnable
//! should go.
//!
//! This crate only decides; it never touches a stack or a register. Every
//! operation that changes the running thread of a CPU returns a [`Switch`]
//! naming the thread to save and the thread to resume, and the kernel
//! performs the context switch (kernel/src/thread.rs). That split keeps the
//! policy testable on the host, including against a reference model driven
//! by random operations.
//!
//! # Threads
//!
//! A [`Scheduler`] owns a fixed table of `N` thread slots and `C` CPU
//! records and no other memory, so it can live in a `static`. A
//! [`ThreadId`] is a slot index plus the slot's generation, which changes
//! every time the slot is freed: an id kept after its thread is gone never
//! names the next thread in that slot.
//!
//! ```text
//!          spawn               yield_now (or tick, then yield_now)
//! (free) ────────► Runnable ◄─────────────────────────────────────► Running
//!                     ▲                                                │
//!                     │ wake_one / wake_all,                           │ block_current, or join
//!                     │ or the joined thread exits                     │ while that thread runs
//!                     │                                                ▼
//!                     └───────────────────────────────────────────── Blocked
//!
//! Running ── exit_current ──► Exited ── reap, and join or detach ──► (free)
//! ```
//!
//! Two slots exist from the start. Slot 0 is the **boot thread**
//! ([`ThreadId::BOOT`]): whatever code creates the scheduler is already
//! running on CPU 0, so it becomes thread 0, `Running`. Slot 1 is CPU 0's
//! **idle thread** ([`ThreadId::IDLE`]). Every other CPU gets an idle
//! thread of its own when it is added ([`Scheduler::add_cpu`]): the code
//! that brings the CPU up becomes it. An idle thread runs only when its CPU
//! has nothing else to run, is never on a run queue, runs only on its own
//! CPU, and can be switched away from at any time. It cannot block, exit,
//! be joined or be detached.
//!
//! # CPUs and run queues
//!
//! CPU 0 is online from the start; the others are added and then put
//! online ([`Scheduler::set_online`]) by the kernel as they come up. Each
//! CPU has its own FIFO run queue and its own current thread and time
//! slice. All of it sits behind one lock in the kernel, so moving a thread
//! between queues is just list surgery.
//!
//! - **Placement.** A thread that becomes runnable (spawned or woken) goes
//!   to the queue of the CPU it last ran on if that CPU is idle, else to
//!   any idle CPU it may run on, else to the least loaded one (load = queued
//!   threads plus the running one), preferring the CPU it last ran on. Only
//!   online CPUs in its affinity are considered.
//! - **Picking.** A CPU runs the first thread of its own queue, but takes
//!   one from another CPU's queue instead ("stealing") when its own queue
//!   is empty, or when the other queue is longer than its own by two or
//!   more. Stealing respects affinity. With one CPU this is exactly the old
//!   round robin.
//! - **Kicks.** Placing a thread on an idle CPU's queue records that CPU in
//!   a set the kernel collects with [`Scheduler::take_kicks`] and sends a
//!   reschedule interrupt to, so an idle CPU halted in `hlt` wakes up.
//! - **Affinity** ([`CpuSet`]): the CPUs a thread may run on, all of them
//!   by default. [`Scheduler::set_affinity`] moves a queued thread; a
//!   running one moves at its CPU's next tick (or yield).
//! - **Draining** ([`Scheduler::drain`]): a CPU that takes no threads and
//!   gives its own away: its queue moves to other CPUs at once, its
//!   running thread at its next tick or yield, after which it runs only
//!   its idle thread (the kernel parks such CPUs while one CPU must run
//!   alone).
//!
//! # Switching
//!
//! A thread switched out on one CPU is on a run queue (or a wait queue, or
//! exited) before its registers are saved. Another CPU must not resume it
//! (or reap its stack) until that has happened. So every switch marks the
//! outgoing thread as *saving*, and the CPU that switched clears it with
//! [`Scheduler::saved`] once the switch is complete (and implicitly at its
//! next scheduler operation, which can only come after the switch). A
//! saving thread is never picked or reaped; when it is cleared and the
//! thread is runnable, an idle CPU is kicked to take it.
//!
//! # Wait queues
//!
//! Every [`WaitQueue`] and run queue is a FIFO list linked through a single
//! `next` field per slot. A thread is on at most one list at a time (a run
//! queue, one wait queue, or the free list), so one link suffices and no
//! operation allocates. Wait queues belong to the caller (a lock, a device,
//! a completion); they hold only slot indices, so a wait queue must only
//! ever be used with the scheduler its threads came from, and must not be
//! dropped while threads wait on it (they would stay blocked forever).
//!
//! # Time slices
//!
//! Every thread that is switched in gets [`Scheduler::time_slice`] ticks.
//! Each CPU's timer interrupt calls [`Scheduler::tick`] once per tick; it
//! returns true when that CPU's running thread should be preempted, and the
//! caller then does [`Scheduler::yield_now`]. A thread whose slice runs out
//! while nothing else could run there simply gets a fresh one. An idle
//! thread has no slice: `tick` asks for preemption as soon as its CPU could
//! run something, its own queue's or another's.
//!
//! # Exit, reap and join
//!
//! [`Scheduler::exit_current`] marks the thread `Exited` and wakes its
//! joiners, but the kernel is still running on that thread's stack at that
//! point. Once it has switched away it frees the stack and calls
//! [`Scheduler::reap`] ([`Scheduler::unreaped`] finds threads waiting for
//! that). The slot is freed when the thread is both reaped and either joined
//! ([`Scheduler::join`] collects its exit code) or detached
//! ([`Scheduler::detach`]: nobody will join it).

#![no_std]

use core::fmt;

/// Slot index type; `N` is at most `u16::MAX`.
type Link = Option<u16>;

const BOOT_SLOT: u16 = 0;
const IDLE_SLOT: u16 = 1;
/// No thread (a CPU that has not been added).
const NONE: u16 = u16::MAX;

/// Most CPUs a scheduler can have: a [`CpuSet`] is one 64-bit word.
pub const MAX_CPUS: usize = 64;

/// A set of CPU numbers (`0..MAX_CPUS`), one bit each.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CpuSet(u64);

impl CpuSet {
    pub const EMPTY: CpuSet = CpuSet(0);
    pub const ALL: CpuSet = CpuSet(u64::MAX);

    /// The set with only `cpu` (empty if `cpu` is out of range).
    pub const fn one(cpu: usize) -> CpuSet {
        if cpu < MAX_CPUS { CpuSet(1 << cpu) } else { CpuSet::EMPTY }
    }

    /// CPUs `0..n`.
    pub const fn first_n(n: usize) -> CpuSet {
        if n >= MAX_CPUS { CpuSet::ALL } else { CpuSet((1 << n) - 1) }
    }

    pub const fn from_bits(bits: u64) -> CpuSet {
        CpuSet(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn contains(self, cpu: usize) -> bool {
        cpu < MAX_CPUS && self.0 & (1 << cpu) != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    pub fn insert(&mut self, cpu: usize) {
        *self = self.union(CpuSet::one(cpu));
    }

    pub fn remove(&mut self, cpu: usize) {
        self.0 &= !CpuSet::one(cpu).0;
    }

    pub const fn union(self, other: CpuSet) -> CpuSet {
        CpuSet(self.0 | other.0)
    }

    pub const fn intersection(self, other: CpuSet) -> CpuSet {
        CpuSet(self.0 & other.0)
    }

    pub const fn without(self, other: CpuSet) -> CpuSet {
        CpuSet(self.0 & !other.0)
    }

    /// The lowest CPU in the set.
    pub const fn first(self) -> Option<usize> {
        if self.0 == 0 { None } else { Some(self.0.trailing_zeros() as usize) }
    }

    /// The CPUs in the set, lowest first.
    pub fn iter(self) -> impl Iterator<Item = usize> {
        let mut bits = self.0;
        core::iter::from_fn(move || {
            let cpu = CpuSet(bits).first()?;
            bits &= bits - 1;
            Some(cpu)
        })
    }
}

impl fmt::Debug for CpuSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

/// The list format of Linux's `cpu/online` and of `taskset -c`: ranges
/// joined by commas ("0", "0-3", "0,2-5"); empty for an empty set.
impl fmt::Display for CpuSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        let mut cpus = self.iter().peekable();
        while let Some(start) = cpus.next() {
            let mut end = start;
            while cpus.peek() == Some(&(end + 1)) {
                end += 1;
                cpus.next();
            }
            if !first {
                f.write_str(",")?;
            }
            first = false;
            if end == start { write!(f, "{start}")? } else { write!(f, "{start}-{end}")? }
        }
        Ok(())
    }
}

/// Names one thread: its slot plus the slot's generation at spawn time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThreadId {
    slot: u16,
    generation: u32,
}

impl ThreadId {
    /// The boot thread: whatever code created the scheduler.
    pub const BOOT: ThreadId = ThreadId { slot: BOOT_SLOT, generation: 0 };
    /// CPU 0's idle thread, which runs when nothing else can.
    pub const IDLE: ThreadId = ThreadId { slot: IDLE_SLOT, generation: 0 };

    /// Index into the thread table, `0..N`. The kernel keys its per-thread
    /// state (saved stack pointer, stack memory) by this.
    pub const fn slot(self) -> usize {
        self.slot as usize
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }

    /// A single number naming the thread (slot and generation), for an
    /// atomic; [`ThreadId::from_bits`] turns it back.
    pub const fn to_bits(self) -> u64 {
        (self.slot as u64) << 32 | self.generation as u64
    }

    pub const fn from_bits(bits: u64) -> ThreadId {
        ThreadId { slot: (bits >> 32) as u16, generation: bits as u32 }
    }
}

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.slot, self.generation)
    }
}

/// What a live thread is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// On a run queue (or an idle thread while another thread runs).
    Runnable,
    /// The current thread of some CPU.
    Running,
    /// On a wait queue, or waiting for a thread to exit.
    Blocked,
    /// Finished; waiting to be reaped, joined or detached.
    Exited,
}

/// A change of running thread on one CPU: save `from`, resume `to`. Never
/// the same thread twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Switch {
    pub from: ThreadId,
    pub to: ThreadId,
}

/// Result of [`Scheduler::join`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    /// The thread had exited with this code; it can no longer be joined.
    Exited(usize),
    /// The thread is still alive; the current thread is now blocked until
    /// it exits. Perform the switch, then call `join` again when resumed.
    Blocked(Switch),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Every slot is in use.
    TableFull,
    /// The id names no live thread: its slot was freed (and maybe reused),
    /// or the thread was already joined.
    NoSuchThread,
    /// An idle thread cannot block, exit, be joined or be detached, and
    /// only runs on its own CPU.
    Idle,
    /// A thread cannot join itself.
    Current,
    /// The thread is detached, so it cannot be joined.
    Detached,
    /// `reap`: the thread has not exited, was reaped already, or is still
    /// being switched away from.
    NotExited,
    /// The CPU number is out of range, the CPU was added already, or an
    /// affinity names no CPU.
    NoSuchCpu,
}

impl Error {
    pub const fn describe(self) -> &'static str {
        match self {
            Error::TableFull => "thread table full",
            Error::NoSuchThread => "no such thread",
            Error::Idle => "not allowed for an idle thread",
            Error::Current => "a thread cannot join itself",
            Error::Detached => "thread is detached",
            Error::NotExited => "thread has not exited or was already reaped",
            Error::NoSuchCpu => "no such CPU",
        }
    }
}

/// FIFO of blocked threads, linked through the scheduler's slots. Not
/// `Clone`: two copies would share links and corrupt each other.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WaitQueue {
    head: Link,
    tail: Link,
}

impl WaitQueue {
    pub const fn new() -> WaitQueue {
        WaitQueue { head: None, tail: None }
    }

    pub const fn is_empty(&self) -> bool {
        self.head.is_none()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotState {
    Free,
    Runnable,
    Running,
    Blocked,
    Exited,
}

#[derive(Debug)]
struct Slot {
    generation: u32,
    state: SlotState,
    /// Next thread on whichever list this slot is on.
    next: Link,
    detached: bool,
    /// Exit code collected by `join`; the id no longer resolves.
    joined: bool,
    /// The kernel has released the thread's resources (its stack).
    reaped: bool,
    exit_code: usize,
    /// Threads blocked in `join` on this one.
    joiners: WaitQueue,
    /// The CPU whose run queue the thread is on, that runs it, or that ran
    /// it last.
    cpu: u8,
    /// The CPUs it may run on.
    affinity: CpuSet,
    /// An idle thread (of CPU `cpu`).
    idle: bool,
    /// Switched away from, but its registers may not be saved yet: no other
    /// CPU may resume or reap it until [`Scheduler::saved`].
    saving: bool,
}

impl Slot {
    const FREE: Slot = Slot {
        generation: 0,
        state: SlotState::Free,
        next: None,
        detached: false,
        joined: false,
        reaped: false,
        exit_code: 0,
        joiners: WaitQueue::new(),
        cpu: 0,
        affinity: CpuSet::ALL,
        idle: false,
        saving: false,
    };
}

/// One CPU's scheduling state.
#[derive(Debug)]
struct Cpu {
    run: WaitQueue,
    /// Threads on `run`.
    queued: u16,
    /// The running thread; [`NONE`] before the CPU is added.
    current: u16,
    /// Its idle thread; [`NONE`] before the CPU is added.
    idle: u16,
    /// Ticks left in the current thread's slice.
    left: u32,
    /// Placement may put threads here.
    online: bool,
    /// The thread this CPU switched away from last, until the switch is
    /// complete ([`Scheduler::saved`]).
    prev: Link,
    /// Gives its threads away and takes none ([`Scheduler::drain`]).
    draining: bool,
}

impl Cpu {
    const ABSENT: Cpu =
        Cpu {
            run: WaitQueue::new(),
            queued: 0,
            current: NONE,
            idle: NONE,
            left: 0,
            online: false,
            prev: None,
            draining: false,
        };

    fn exists(&self) -> bool {
        self.idle != NONE
    }

    fn is_idle(&self) -> bool {
        self.current == self.idle
    }
}

fn push(slots: &mut [Slot], q: &mut WaitQueue, slot: u16) {
    slots[slot as usize].next = None;
    match q.tail {
        Some(tail) => slots[tail as usize].next = Some(slot),
        None => q.head = Some(slot),
    }
    q.tail = Some(slot);
}

fn pop(slots: &mut [Slot], q: &mut WaitQueue) -> Link {
    let head = q.head?;
    q.head = slots[head as usize].next.take();
    if q.head.is_none() {
        q.tail = None;
    }
    Some(head)
}

/// Take `slot` out of `q`, wherever it is; false if it is not on it.
fn unlink(slots: &mut [Slot], q: &mut WaitQueue, slot: u16) -> bool {
    let mut prev: Link = None;
    let mut at = q.head;
    while let Some(s) = at {
        if s == slot {
            let next = slots[s as usize].next.take();
            match prev {
                Some(p) => slots[p as usize].next = next,
                None => q.head = next,
            }
            if q.tail == Some(s) {
                q.tail = prev;
            }
            return true;
        }
        prev = at;
        at = slots[s as usize].next;
    }
    false
}

/// Round-robin scheduler state for up to `N` threads (boot and idle threads
/// included) on up to `C` CPUs. See the crate documentation.
#[derive(Debug)]
pub struct Scheduler<const N: usize, const C: usize = 1> {
    slots: [Slot; N],
    cpus: [Cpu; C],
    /// Free slots, most recently freed first.
    free: Link,
    time_slice: u32,
    /// CPUs to send a reschedule interrupt to (see [`Scheduler::take_kicks`]).
    kicks: CpuSet,
}

impl<const N: usize, const C: usize> Scheduler<N, C> {
    /// A scheduler whose caller is the running boot thread on CPU 0, with
    /// CPU 0's idle thread and `N - 2` free slots. Every thread runs for
    /// `time_slice` ticks before [`tick`](Self::tick) asks to preempt it.
    pub const fn new(time_slice: u32) -> Self {
        assert!(N >= 2 && N < NONE as usize, "N must leave room for the boot and idle threads");
        assert!(C >= 1 && C <= MAX_CPUS, "between 1 and MAX_CPUS CPUs");
        assert!(time_slice > 0, "a time slice is at least one tick");
        let mut slots = [Slot::FREE; N];
        slots[BOOT_SLOT as usize].state = SlotState::Running;
        slots[IDLE_SLOT as usize].state = SlotState::Runnable;
        slots[IDLE_SLOT as usize].idle = true;
        slots[IDLE_SLOT as usize].affinity = CpuSet::one(0);
        // Free list in ascending order, so the first spawns get slots 2, 3, ...
        let mut free = None;
        let mut i = N;
        while i > 2 {
            i -= 1;
            slots[i].next = free;
            free = Some(i as u16);
        }
        let mut cpus = [Cpu::ABSENT; C];
        cpus[0].current = BOOT_SLOT;
        cpus[0].idle = IDLE_SLOT;
        cpus[0].left = time_slice;
        cpus[0].online = true;
        Scheduler { slots, cpus, free, time_slice, kicks: CpuSet::EMPTY }
    }

    /// Add CPU `cpu`: its idle thread is created and is its running thread
    /// (the code bringing the CPU up becomes it). The CPU takes no threads
    /// until it is put [online](Self::set_online).
    pub fn add_cpu(&mut self, cpu: usize) -> Result<ThreadId, Error> {
        if cpu >= C || self.cpus[cpu].exists() {
            return Err(Error::NoSuchCpu);
        }
        let slot = self.free.ok_or(Error::TableFull)?;
        let s = &mut self.slots[slot as usize];
        self.free = s.next.take();
        s.state = SlotState::Running;
        s.idle = true;
        s.cpu = cpu as u8;
        s.affinity = CpuSet::one(cpu);
        let c = &mut self.cpus[cpu];
        c.current = slot;
        c.idle = slot;
        c.left = self.time_slice;
        Ok(self.id(slot))
    }

    /// Let placement put threads on `cpu` (it is up and runs its scheduler),
    /// or stop it (threads already queued there stay, and other CPUs may
    /// still take them).
    pub fn set_online(&mut self, cpu: usize, online: bool) -> Result<(), Error> {
        let c = self.cpus.get_mut(cpu).filter(|c| c.exists()).ok_or(Error::NoSuchCpu)?;
        c.online = online;
        Ok(())
    }

    /// The CPUs placement uses.
    pub fn online(&self) -> CpuSet {
        let mut set = CpuSet::EMPTY;
        for (i, c) in self.cpus.iter().enumerate() {
            if c.online {
                set.insert(i);
            }
        }
        set
    }

    /// The CPUs that have been added (online or not).
    pub fn present(&self) -> CpuSet {
        let mut set = CpuSet::EMPTY;
        for (i, c) in self.cpus.iter().enumerate() {
            if c.exists() {
                set.insert(i);
            }
        }
        set
    }

    /// The running thread of `cpu`.
    pub fn current(&self, cpu: usize) -> ThreadId {
        self.id(self.cpus[cpu].current)
    }

    /// The idle thread of `cpu`, once it is added.
    pub fn idle_thread(&self, cpu: usize) -> Option<ThreadId> {
        let c = self.cpus.get(cpu).filter(|c| c.exists())?;
        Some(self.id(c.idle))
    }

    /// Whether `cpu` is running its idle thread.
    pub fn is_idle(&self, cpu: usize) -> bool {
        self.cpus[cpu].is_idle()
    }

    pub fn time_slice(&self) -> u32 {
        self.time_slice
    }

    /// Threads waiting on `cpu`'s run queue.
    pub fn queued(&self, cpu: usize) -> usize {
        usize::from(self.cpus[cpu].queued)
    }

    /// Whether `cpu` could switch to another thread now: one on its own
    /// queue, or one it would take from another CPU's.
    pub fn has_runnable(&self, cpu: usize) -> bool {
        self.first_own(cpu).is_some() || self.find_steal(cpu, self.steal_threshold(cpu)).is_some()
    }

    /// Slots in use, boot and idle threads included.
    pub fn threads(&self) -> usize {
        self.slots.iter().filter(|s| s.state != SlotState::Free).count()
    }

    /// Every thread that exists and has not been joined, with its state, in
    /// slot order (boot and CPU 0's idle thread first).
    pub fn live_threads(&self) -> impl Iterator<Item = (ThreadId, State)> + '_ {
        (0..self.slots.len()).filter_map(|slot| {
            let id = self.id(slot as u16);
            Some((id, self.state(id)?))
        })
    }

    /// State of thread `t`, or `None` if it no longer exists (freed or joined).
    pub fn state(&self, t: ThreadId) -> Option<State> {
        let slot = self.live(t).ok()?;
        Some(match self.slots[slot as usize].state {
            SlotState::Runnable => State::Runnable,
            SlotState::Running => State::Running,
            SlotState::Blocked => State::Blocked,
            SlotState::Exited => State::Exited,
            SlotState::Free => unreachable!("live() never returns a free slot"),
        })
    }

    /// The CPU that runs `t`, whose queue it is on, or that ran it last.
    pub fn cpu_of(&self, t: ThreadId) -> Option<usize> {
        let slot = self.live(t).ok()?;
        Some(usize::from(self.slots[slot as usize].cpu))
    }

    /// The CPUs `t` may run on.
    pub fn affinity(&self, t: ThreadId) -> Option<CpuSet> {
        let slot = self.live(t).ok()?;
        Some(self.slots[slot as usize].affinity)
    }

    /// Whether `t` is an idle thread.
    pub fn is_idle_thread(&self, t: ThreadId) -> bool {
        self.live(t).is_ok_and(|slot| self.slots[slot as usize].idle)
    }

    /// Take a free slot for a new thread that may run on `affinity` (which
    /// must name a CPU that has been added) and put it on a run queue (see the crate docs for which), as if it had last
    /// run on `cpu` (the CPU spawning it). The caller sets up the thread's
    /// stack before it can be switched to, i.e. before it releases the
    /// lock this scheduler is behind.
    pub fn spawn(&mut self, cpu: usize, affinity: CpuSet) -> Result<ThreadId, Error> {
        let affinity = affinity.intersection(CpuSet::first_n(C));
        if affinity.intersection(self.present()).is_empty() {
            return Err(Error::NoSuchCpu);
        }
        let slot = self.free.ok_or(Error::TableFull)?;
        let s = &mut self.slots[slot as usize];
        self.free = s.next;
        debug_assert_eq!(s.state, SlotState::Free);
        s.state = SlotState::Runnable;
        s.cpu = cpu as u8;
        s.affinity = affinity;
        self.place(slot);
        Ok(self.id(slot))
    }

    /// Give up `cpu` to the next thread it could run, if there is one; the
    /// current thread goes to the back of `cpu`'s run queue (or elsewhere,
    /// if its affinity no longer allows `cpu`). `None` means keep running.
    pub fn yield_now(&mut self, cpu: usize) -> Option<Switch> {
        self.finish(cpu);
        let cur = self.cpus[cpu].current;
        let must_move = self.must_move(cpu);
        let next = match self.pick(cpu) {
            Some(next) => next,
            None if must_move => self.cpus[cpu].idle,
            None => return None,
        };
        self.slots[cur as usize].state = SlotState::Runnable;
        if !self.slots[cur as usize].idle {
            if must_move {
                self.place(cur);
            } else {
                self.enqueue(cur, cpu);
            }
        }
        Some(self.switch_to(cpu, next))
    }

    /// Account one timer tick to `cpu`'s running thread. True when it
    /// should be preempted now: its slice is used up and the CPU has
    /// another thread it could run, or it is the idle thread and there is
    /// any, or its affinity no longer allows this CPU. Keeps returning true
    /// until the caller switches (with [`yield_now`](Self::yield_now)).
    pub fn tick(&mut self, cpu: usize) -> bool {
        self.finish(cpu);
        let cur = self.cpus[cpu].current as usize;
        if self.slots[cur].idle {
            return self.has_runnable(cpu);
        }
        if self.must_move(cpu) {
            return true;
        }
        let c = &mut self.cpus[cpu];
        c.left = c.left.saturating_sub(1);
        if c.left > 0 {
            return false;
        }
        if self.has_runnable(cpu) {
            return true;
        }
        self.cpus[cpu].left = self.time_slice;
        false
    }

    /// Whether `cpu` should switch away from its running thread right away
    /// (a reschedule interrupt): it is idle and has something to run, or
    /// the running thread may no longer run there.
    pub fn needs_switch(&self, cpu: usize) -> bool {
        let cur = self.cpus[cpu].current as usize;
        if self.slots[cur].idle { self.has_runnable(cpu) } else { self.must_move(cpu) }
    }

    /// Make `cpu` give its threads away and take none (`true`), or take
    /// threads again and go back online (`false`). Draining takes the CPU
    /// offline, moves every thread queued there to an online CPU it may run
    /// on (one pinned to `cpu` stays, to run once it is back), and kicks
    /// `cpu` so its running thread leaves at once.
    pub fn drain(&mut self, cpu: usize, on: bool) -> Result<(), Error> {
        let c = self.cpus.get_mut(cpu).filter(|c| c.exists()).ok_or(Error::NoSuchCpu)?;
        c.draining = on;
        c.online = !on;
        if !on {
            return Ok(());
        }
        let mut q = core::mem::take(&mut self.cpus[cpu].run);
        self.cpus[cpu].queued = 0;
        let mut stay = WaitQueue::new();
        while let Some(slot) = pop(&mut self.slots, &mut q) {
            let to = self.select_cpu(slot);
            if to == cpu {
                push(&mut self.slots, &mut stay, slot);
                self.cpus[cpu].queued += 1;
            } else {
                self.enqueue(slot, to);
            }
        }
        self.cpus[cpu].run = stay;
        if !self.cpus[cpu].is_idle() {
            self.kicks.insert(cpu);
        }
        Ok(())
    }

    /// Whether `cpu` is [draining](Self::drain).
    pub fn is_draining(&self, cpu: usize) -> bool {
        self.cpus[cpu].draining
    }

    /// Whether `cpu`'s running thread must leave it: not an idle thread,
    /// and its affinity excludes the CPU or the CPU is draining.
    fn must_move(&self, cpu: usize) -> bool {
        let cur = &self.slots[self.cpus[cpu].current as usize];
        !cur.idle && (self.cpus[cpu].draining || !cur.affinity.contains(cpu))
    }

    /// Block `cpu`'s current thread on `q` and switch to the next thread
    /// the CPU could run (its idle thread if there is none).
    pub fn block_current(&mut self, cpu: usize, q: &mut WaitQueue) -> Result<Switch, Error> {
        self.finish(cpu);
        let cur = self.cpus[cpu].current;
        if self.slots[cur as usize].idle {
            return Err(Error::Idle);
        }
        self.slots[cur as usize].state = SlotState::Blocked;
        push(&mut self.slots, q, cur);
        let next = self.pick(cpu).unwrap_or(self.cpus[cpu].idle);
        Ok(self.switch_to(cpu, next))
    }

    /// Make the longest-waiting thread on `q` runnable and place it on a
    /// run queue.
    pub fn wake_one(&mut self, q: &mut WaitQueue) -> Option<ThreadId> {
        let slot = pop(&mut self.slots, q)?;
        self.make_runnable(slot);
        Some(self.id(slot))
    }

    /// Wake every thread on `q`, in the order they blocked. Returns how many.
    pub fn wake_all(&mut self, q: &mut WaitQueue) -> usize {
        let mut n = 0;
        while self.wake_one(q).is_some() {
            n += 1;
        }
        n
    }

    /// End `cpu`'s current thread with `code`, wake its joiners and switch
    /// away. The kernel must not reap it before that switch has happened.
    pub fn exit_current(&mut self, cpu: usize, code: usize) -> Result<Switch, Error> {
        self.finish(cpu);
        let cur = self.cpus[cpu].current;
        if self.slots[cur as usize].idle {
            return Err(Error::Idle);
        }
        let s = &mut self.slots[cur as usize];
        s.state = SlotState::Exited;
        s.exit_code = code;
        let mut joiners = core::mem::take(&mut s.joiners);
        while let Some(j) = pop(&mut self.slots, &mut joiners) {
            self.make_runnable(j);
        }
        let next = self.pick(cpu).unwrap_or(self.cpus[cpu].idle);
        Ok(self.switch_to(cpu, next))
    }

    /// Collect the exit code of `t`, or block `cpu`'s current thread until
    /// `t` exits. After [`Join::Blocked`] the caller switches and, once
    /// resumed, calls `join` again: by then `t` has exited, or another
    /// joiner got there first ([`Error::NoSuchThread`]).
    pub fn join(&mut self, cpu: usize, t: ThreadId) -> Result<Join, Error> {
        self.finish(cpu);
        let slot = self.live(t)?;
        if self.slots[slot as usize].idle {
            return Err(Error::Idle);
        }
        let cur = self.cpus[cpu].current;
        if slot == cur {
            return Err(Error::Current);
        }
        let s = &mut self.slots[slot as usize];
        if s.detached {
            return Err(Error::Detached);
        }
        if s.state == SlotState::Exited {
            s.joined = true;
            let code = s.exit_code;
            self.free_if_done(slot);
            return Ok(Join::Exited(code));
        }
        if self.slots[cur as usize].idle {
            return Err(Error::Idle);
        }
        self.slots[cur as usize].state = SlotState::Blocked;
        let mut joiners = core::mem::take(&mut self.slots[slot as usize].joiners);
        push(&mut self.slots, &mut joiners, cur);
        self.slots[slot as usize].joiners = joiners;
        let next = self.pick(cpu).unwrap_or(self.cpus[cpu].idle);
        Ok(Join::Blocked(self.switch_to(cpu, next)))
    }

    /// Nobody will join `t`: free its slot as soon as it has exited and been
    /// reaped. Threads already blocked in `join` on it get
    /// [`Error::Detached`] once it exits.
    pub fn detach(&mut self, t: ThreadId) -> Result<(), Error> {
        let slot = self.live(t)?;
        if self.slots[slot as usize].idle {
            return Err(Error::Idle);
        }
        self.slots[slot as usize].detached = true;
        self.free_if_done(slot);
        Ok(())
    }

    /// An exited thread whose resources the kernel may release now: not
    /// reaped yet, and switched away from completely. Never a running
    /// thread.
    pub fn unreaped(&self) -> Option<ThreadId> {
        let slot = self.slots.iter().position(|s| s.state == SlotState::Exited && !s.reaped && !s.saving)?;
        Some(self.id(slot as u16))
    }

    /// The kernel has released the resources of exited thread `t` (it must
    /// have switched away from `t` first). Frees the slot if `t` was joined
    /// or detached.
    pub fn reap(&mut self, t: ThreadId) -> Result<(), Error> {
        let slot = self.existing(t)?;
        let s = &mut self.slots[slot as usize];
        if s.state != SlotState::Exited || s.reaped || s.saving {
            return Err(Error::NotExited);
        }
        s.reaped = true;
        self.free_if_done(slot);
        Ok(())
    }

    /// The switch `cpu` made last is complete: the thread it switched away
    /// from has its registers saved, so another CPU may now resume it (and
    /// is kicked to, if it is runnable and a CPU is idle) or reap it.
    pub fn saved(&mut self, cpu: usize) {
        let Some(prev) = self.cpus[cpu].prev else { return };
        self.finish(cpu);
        let s = &self.slots[prev as usize];
        if s.state != SlotState::Runnable || s.idle {
            return;
        }
        let queue = usize::from(s.cpu);
        if self.cpus[queue].is_idle() {
            self.kicks.insert(queue);
            return;
        }
        let allowed = s.affinity.intersection(self.online());
        if let Some(idle) = allowed.iter().find(|&c| c != cpu && self.cpus[c].is_idle() && self.cpus[c].queued == 0) {
            self.kicks.insert(idle);
        }
    }

    /// Let `t` run only on `cpus` (which must name at least one CPU that
    /// has been added). A queued thread moves to an allowed CPU's
    /// queue at once; a running one at its CPU's next tick or yield (that
    /// CPU is kicked). Not for idle threads.
    pub fn set_affinity(&mut self, t: ThreadId, cpus: CpuSet) -> Result<(), Error> {
        let slot = self.live(t)?;
        let cpus = cpus.intersection(CpuSet::first_n(C));
        if cpus.intersection(self.present()).is_empty() {
            return Err(Error::NoSuchCpu);
        }
        if self.slots[slot as usize].idle {
            return Err(Error::Idle);
        }
        self.slots[slot as usize].affinity = cpus;
        let s = &self.slots[slot as usize];
        let on = usize::from(s.cpu);
        if cpus.contains(on) {
            return Ok(());
        }
        match s.state {
            SlotState::Runnable => {
                let mut q = core::mem::take(&mut self.cpus[on].run);
                let found = unlink(&mut self.slots, &mut q, slot);
                self.cpus[on].run = q;
                if found {
                    self.cpus[on].queued -= 1;
                    self.place(slot);
                }
            }
            SlotState::Running => self.kicks.insert(on),
            _ => {}
        }
        Ok(())
    }

    /// The CPUs that had a thread placed on their queue while idle (or must
    /// move their running thread) since the last call: send each a
    /// reschedule interrupt.
    pub fn take_kicks(&mut self) -> CpuSet {
        core::mem::take(&mut self.kicks)
    }

    fn id(&self, slot: u16) -> ThreadId {
        ThreadId { slot, generation: self.slots[slot as usize].generation }
    }

    /// Slot of `t` if it is in use, even if already joined.
    fn existing(&self, t: ThreadId) -> Result<u16, Error> {
        match self.slots.get(t.slot()) {
            Some(s) if s.generation == t.generation && s.state != SlotState::Free => Ok(t.slot),
            _ => Err(Error::NoSuchThread),
        }
    }

    /// Slot of `t` if it is in use and not yet joined.
    fn live(&self, t: ThreadId) -> Result<u16, Error> {
        let slot = self.existing(t)?;
        if self.slots[slot as usize].joined { Err(Error::NoSuchThread) } else { Ok(slot) }
    }

    /// Whatever `cpu` switched away from last has been saved: `cpu` is
    /// running scheduler code again, which it only does after the switch.
    fn finish(&mut self, cpu: usize) {
        if let Some(prev) = self.cpus[cpu].prev.take() {
            self.slots[prev as usize].saving = false;
        }
    }

    fn make_runnable(&mut self, slot: u16) {
        debug_assert_eq!(self.slots[slot as usize].state, SlotState::Blocked);
        self.slots[slot as usize].state = SlotState::Runnable;
        self.place(slot);
    }

    /// Put runnable `slot` on the run queue of the CPU [`select_cpu`] picks.
    fn place(&mut self, slot: u16) {
        let cpu = self.select_cpu(slot);
        self.enqueue(slot, cpu);
    }

    fn enqueue(&mut self, slot: u16, cpu: usize) {
        let mut q = core::mem::take(&mut self.cpus[cpu].run);
        push(&mut self.slots, &mut q, slot);
        let c = &mut self.cpus[cpu];
        c.run = q;
        c.queued += 1;
        self.slots[slot as usize].cpu = cpu as u8;
        if self.cpus[cpu].is_idle() {
            self.kicks.insert(cpu);
        }
    }

    /// An idle, online CPU with nothing queued.
    fn idle_cpu(&self, cpu: usize) -> bool {
        let c = &self.cpus[cpu];
        c.online && c.is_idle() && c.queued == 0
    }

    fn load(&self, cpu: usize) -> usize {
        let c = &self.cpus[cpu];
        usize::from(c.queued) + usize::from(!c.is_idle())
    }

    /// Where runnable `slot` should wait (see the crate docs).
    fn select_cpu(&self, slot: u16) -> usize {
        let s = &self.slots[slot as usize];
        let prev = usize::from(s.cpu);
        let allowed = s.affinity.intersection(self.online());
        if allowed.is_empty() {
            // Nowhere to run now: wait where it may run once that CPU is
            // back (any CPU may still take it from there if allowed).
            let present = s.affinity.intersection(self.present());
            return if present.contains(prev) { prev } else { present.first().unwrap_or(0) };
        }
        if allowed.contains(prev) && self.idle_cpu(prev) {
            return prev;
        }
        if let Some(idle) = allowed.iter().find(|&c| self.idle_cpu(c)) {
            return idle;
        }
        let mut best = if allowed.contains(prev) { prev } else { allowed.first().unwrap_or(0) };
        for c in allowed.iter() {
            if self.load(c) < self.load(best) {
                best = c;
            }
        }
        best
    }

    /// Whether `cpu` may resume `slot` now.
    fn pickable(&self, slot: u16, cpu: usize) -> bool {
        let s = &self.slots[slot as usize];
        !s.saving && s.affinity.contains(cpu) && !self.cpus[cpu].draining
    }

    /// The first thread of `cpu`'s queue it may resume.
    fn first_own(&self, cpu: usize) -> Option<u16> {
        let mut at = self.cpus[cpu].run.head;
        while let Some(slot) = at {
            if self.pickable(slot, cpu) {
                return Some(slot);
            }
            at = self.slots[slot as usize].next;
        }
        None
    }

    /// How long another CPU's queue must be for `cpu` to take from it: any
    /// length if its own queue is empty, else two more than its own.
    fn steal_threshold(&self, cpu: usize) -> usize {
        match usize::from(self.cpus[cpu].queued) {
            0 => 1,
            own => own + 2,
        }
    }

    /// The longest other queue with at least `threshold` threads and one
    /// `cpu` may take, and that thread.
    fn find_steal(&self, cpu: usize, threshold: usize) -> Option<(usize, u16)> {
        let mut best: Option<(usize, u16)> = None;
        for (other, c) in self.cpus.iter().enumerate() {
            let len = usize::from(c.queued);
            if other == cpu || len < threshold || best.is_some_and(|(b, _)| usize::from(self.cpus[b].queued) >= len) {
                continue;
            }
            let mut at = c.run.head;
            while let Some(slot) = at {
                if self.pickable(slot, cpu) {
                    best = Some((other, slot));
                    break;
                }
                at = self.slots[slot as usize].next;
            }
        }
        best
    }

    /// Take `slot` off `cpu`'s run queue.
    fn dequeue(&mut self, cpu: usize, slot: u16) {
        let mut q = core::mem::take(&mut self.cpus[cpu].run);
        let found = unlink(&mut self.slots, &mut q, slot);
        debug_assert!(found, "dequeue of a thread that is not queued there");
        self.cpus[cpu].run = q;
        self.cpus[cpu].queued -= 1;
    }

    /// The thread `cpu` should run next, taken off its queue: another
    /// CPU's if that one is much longer (or its own is empty), else its
    /// own first one.
    fn pick(&mut self, cpu: usize) -> Option<u16> {
        let (from, slot) = self
            .find_steal(cpu, self.steal_threshold(cpu))
            .or_else(|| self.first_own(cpu).map(|slot| (cpu, slot)))
            .or_else(|| self.find_steal(cpu, 1))?;
        self.dequeue(from, slot);
        Some(slot)
    }

    /// Make `next` (taken off its queue, or `cpu`'s idle thread) `cpu`'s
    /// running thread. The caller has already put the current one where it
    /// belongs.
    fn switch_to(&mut self, cpu: usize, next: u16) -> Switch {
        let from = self.current(cpu);
        self.slots[from.slot()].saving = true;
        let c = &mut self.cpus[cpu];
        c.prev = Some(from.slot);
        c.current = next;
        c.left = self.time_slice;
        let n = &mut self.slots[next as usize];
        n.state = SlotState::Running;
        n.cpu = cpu as u8;
        Switch { from, to: self.id(next) }
    }

    fn free_if_done(&mut self, slot: u16) {
        let s = &mut self.slots[slot as usize];
        if s.state != SlotState::Exited || !s.reaped || !(s.joined || s.detached) {
            return;
        }
        debug_assert!(s.joiners.is_empty());
        *s = Slot { generation: s.generation.wrapping_add(1), next: self.free, ..Slot::FREE };
        self.free = Some(slot);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod smp_tests;
