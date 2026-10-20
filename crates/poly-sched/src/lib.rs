//! Scheduler bookkeeping for kernel threads: which thread runs next, which
//! threads wait, and when the running one has used up its time slice.
//!
//! This crate only decides; it never touches a stack or a register. Every
//! operation that changes the running thread returns a [`Switch`] naming the
//! thread to save and the thread to resume, and the kernel performs the
//! context switch (kernel/src/thread.rs). That split keeps the policy
//! testable on the host, including against a reference model driven by
//! random operations.
//!
//! # Threads
//!
//! A [`Scheduler`] owns a fixed table of `N` thread slots and no other
//! memory, so it can live in a `static`. A [`ThreadId`] is a slot index plus
//! the slot's generation, which changes every time the slot is freed: an id
//! kept after its thread is gone never names the next thread in that slot.
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
//! running, so it becomes thread 0, `Running`. Slot 1 is the **idle thread**
//! ([`ThreadId::IDLE`]): it runs only when no other thread is runnable, is
//! never on the run queue, and can be switched away from at any time. It
//! cannot block, exit, be joined or be detached.
//!
//! # Queues
//!
//! The run queue and every [`WaitQueue`] are FIFO lists linked through a
//! single `next` field per slot. A thread is on at most one list at a time
//! (the run queue, one wait queue, or the free list), so one link suffices
//! and no operation allocates. Wait queues belong to the caller (a lock, a
//! device, a completion); they hold only slot indices, so a wait queue must
//! only ever be used with the scheduler its threads came from, and must not
//! be dropped while threads wait on it (they would stay blocked forever).
//!
//! # Time slices
//!
//! Every thread that is switched in gets [`Scheduler::time_slice`] ticks.
//! The timer interrupt calls [`Scheduler::tick`] once per tick; it returns
//! true when the running thread should be preempted, and the caller then
//! does [`Scheduler::yield_now`]. A thread whose slice runs out while nobody
//! else is runnable simply gets a fresh one. The idle thread has no slice:
//! `tick` asks for preemption as soon as any thread is runnable.
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

/// Names one thread: its slot plus the slot's generation at spawn time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThreadId {
    slot: u16,
    generation: u32,
}

impl ThreadId {
    /// The boot thread: whatever code created the scheduler.
    pub const BOOT: ThreadId = ThreadId { slot: BOOT_SLOT, generation: 0 };
    /// The idle thread, which runs when nothing else can.
    pub const IDLE: ThreadId = ThreadId { slot: IDLE_SLOT, generation: 0 };

    /// Index into the thread table, `0..N`. The kernel keys its per-thread
    /// state (saved stack pointer, stack memory) by this.
    pub const fn slot(self) -> usize {
        self.slot as usize
    }

    pub const fn generation(self) -> u32 {
        self.generation
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
    /// On the run queue (or the idle thread while another thread runs).
    Runnable,
    /// The current thread.
    Running,
    /// On a wait queue, or waiting for a thread to exit.
    Blocked,
    /// Finished; waiting to be reaped, joined or detached.
    Exited,
}

/// A change of running thread: save `from`, resume `to`. Never the same
/// thread twice.
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
    /// The idle thread cannot block, exit, be joined or be detached.
    Idle,
    /// A thread cannot join itself.
    Current,
    /// The thread is detached, so it cannot be joined.
    Detached,
    /// `reap`: the thread has not exited, or was reaped already.
    NotExited,
}

impl Error {
    pub const fn describe(self) -> &'static str {
        match self {
            Error::TableFull => "thread table full",
            Error::NoSuchThread => "no such thread",
            Error::Idle => "not allowed for the idle thread",
            Error::Current => "a thread cannot join itself",
            Error::Detached => "thread is detached",
            Error::NotExited => "thread has not exited or was already reaped",
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
    };
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

/// Round-robin scheduler state for up to `N` threads (boot and idle
/// included). See the crate documentation.
#[derive(Debug)]
pub struct Scheduler<const N: usize> {
    slots: [Slot; N],
    run: WaitQueue,
    /// Free slots, most recently freed first.
    free: Link,
    current: u16,
    time_slice: u32,
    /// Ticks left in the current thread's slice.
    left: u32,
}

impl<const N: usize> Scheduler<N> {
    /// A scheduler whose caller is the running boot thread, with an idle
    /// thread and `N - 2` free slots. Every thread runs for `time_slice`
    /// ticks before [`tick`](Self::tick) asks to preempt it.
    pub const fn new(time_slice: u32) -> Self {
        assert!(N >= 2 && N <= u16::MAX as usize, "N must leave room for the boot and idle threads");
        assert!(time_slice > 0, "a time slice is at least one tick");
        let mut slots = [Slot::FREE; N];
        slots[BOOT_SLOT as usize].state = SlotState::Running;
        slots[IDLE_SLOT as usize].state = SlotState::Runnable;
        // Free list in ascending order, so the first spawns get slots 2, 3, ...
        let mut free = None;
        let mut i = N;
        while i > 2 {
            i -= 1;
            slots[i].next = free;
            free = Some(i as u16);
        }
        Scheduler { slots, run: WaitQueue::new(), free, current: BOOT_SLOT, time_slice, left: time_slice }
    }

    /// The running thread.
    pub fn current(&self) -> ThreadId {
        self.id(self.current)
    }

    pub fn time_slice(&self) -> u32 {
        self.time_slice
    }

    /// Whether any thread is waiting on the run queue.
    pub fn has_runnable(&self) -> bool {
        !self.run.is_empty()
    }

    /// Slots in use, boot and idle threads included.
    pub fn threads(&self) -> usize {
        self.slots.iter().filter(|s| s.state != SlotState::Free).count()
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

    /// Take a free slot for a new thread and put it at the back of the run
    /// queue. The caller sets up the thread's stack before it can be
    /// switched to, i.e. before it next yields, blocks or exits.
    pub fn spawn(&mut self) -> Result<ThreadId, Error> {
        let slot = self.free.ok_or(Error::TableFull)?;
        let s = &mut self.slots[slot as usize];
        self.free = s.next;
        debug_assert_eq!(s.state, SlotState::Free);
        s.state = SlotState::Runnable;
        push(&mut self.slots, &mut self.run, slot);
        Ok(self.id(slot))
    }

    /// Give up the CPU to the next runnable thread, if there is one; the
    /// current thread goes to the back of the run queue. `None` means keep
    /// running.
    pub fn yield_now(&mut self) -> Option<Switch> {
        if self.run.is_empty() {
            return None;
        }
        let cur = self.current;
        self.slots[cur as usize].state = SlotState::Runnable;
        if cur != IDLE_SLOT {
            push(&mut self.slots, &mut self.run, cur);
        }
        Some(self.switch_to_next())
    }

    /// Account one timer tick to the current thread. True when it should be
    /// preempted now: its slice is used up and another thread is runnable,
    /// or it is the idle thread and any thread is runnable. Keeps returning
    /// true until the caller switches (with [`yield_now`](Self::yield_now)).
    pub fn tick(&mut self) -> bool {
        if self.current == IDLE_SLOT {
            return !self.run.is_empty();
        }
        self.left = self.left.saturating_sub(1);
        if self.left > 0 {
            return false;
        }
        if self.run.is_empty() {
            self.left = self.time_slice;
            return false;
        }
        true
    }

    /// Block the current thread on `q` and switch to the next runnable
    /// thread (the idle thread if there is none).
    pub fn block_current(&mut self, q: &mut WaitQueue) -> Result<Switch, Error> {
        let cur = self.current;
        if cur == IDLE_SLOT {
            return Err(Error::Idle);
        }
        self.slots[cur as usize].state = SlotState::Blocked;
        push(&mut self.slots, q, cur);
        Ok(self.switch_to_next())
    }

    /// Move the longest-waiting thread on `q` to the back of the run queue.
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

    /// End the current thread with `code`, wake its joiners and switch away.
    /// The kernel must not reap it before that switch has happened.
    pub fn exit_current(&mut self, code: usize) -> Result<Switch, Error> {
        let cur = self.current;
        if cur == IDLE_SLOT {
            return Err(Error::Idle);
        }
        let s = &mut self.slots[cur as usize];
        s.state = SlotState::Exited;
        s.exit_code = code;
        let mut joiners = core::mem::take(&mut s.joiners);
        while let Some(j) = pop(&mut self.slots, &mut joiners) {
            self.make_runnable(j);
        }
        Ok(self.switch_to_next())
    }

    /// Collect the exit code of `t`, or block the current thread until `t`
    /// exits. After [`Join::Blocked`] the caller switches and, once resumed,
    /// calls `join` again: by then `t` has exited, or another joiner got
    /// there first ([`Error::NoSuchThread`]).
    pub fn join(&mut self, t: ThreadId) -> Result<Join, Error> {
        let slot = self.live(t)?;
        if slot == IDLE_SLOT {
            return Err(Error::Idle);
        }
        if slot == self.current {
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
        let cur = self.current;
        if cur == IDLE_SLOT {
            return Err(Error::Idle);
        }
        self.slots[cur as usize].state = SlotState::Blocked;
        let mut joiners = core::mem::take(&mut self.slots[slot as usize].joiners);
        push(&mut self.slots, &mut joiners, cur);
        self.slots[slot as usize].joiners = joiners;
        Ok(Join::Blocked(self.switch_to_next()))
    }

    /// Nobody will join `t`: free its slot as soon as it has exited and been
    /// reaped. Threads already blocked in `join` on it get
    /// [`Error::Detached`] once it exits.
    pub fn detach(&mut self, t: ThreadId) -> Result<(), Error> {
        let slot = self.live(t)?;
        if slot == IDLE_SLOT {
            return Err(Error::Idle);
        }
        self.slots[slot as usize].detached = true;
        self.free_if_done(slot);
        Ok(())
    }

    /// An exited thread whose resources the kernel has not released yet.
    /// Never the current thread.
    pub fn unreaped(&self) -> Option<ThreadId> {
        let slot = self.slots.iter().position(|s| s.state == SlotState::Exited && !s.reaped)?;
        Some(self.id(slot as u16))
    }

    /// The kernel has released the resources of exited thread `t` (it must
    /// have switched away from `t` first). Frees the slot if `t` was joined
    /// or detached.
    pub fn reap(&mut self, t: ThreadId) -> Result<(), Error> {
        let slot = self.existing(t)?;
        let s = &mut self.slots[slot as usize];
        if s.state != SlotState::Exited || s.reaped {
            return Err(Error::NotExited);
        }
        s.reaped = true;
        self.free_if_done(slot);
        Ok(())
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

    fn make_runnable(&mut self, slot: u16) {
        debug_assert_eq!(self.slots[slot as usize].state, SlotState::Blocked);
        self.slots[slot as usize].state = SlotState::Runnable;
        push(&mut self.slots, &mut self.run, slot);
    }

    /// Switch to the head of the run queue, or to the idle thread. The
    /// caller has already put the current thread where it belongs.
    fn switch_to_next(&mut self) -> Switch {
        let from = self.current();
        let next = pop(&mut self.slots, &mut self.run).unwrap_or(IDLE_SLOT);
        self.slots[next as usize].state = SlotState::Running;
        self.current = next;
        self.left = self.time_slice;
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
