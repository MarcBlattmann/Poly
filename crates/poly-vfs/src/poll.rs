//! Readiness: what an open file is ready for, who waits for that to
//! change, and blocking until it does (ADR 0042).
//!
//! - [`PollEvents`] are the event bits, with Linux's values (UAPI
//!   `asm-generic/poll.h`; `linux/eventpoll.h` uses the same ones), so
//!   `poll(2)`, `select(2)` and `epoll(7)` all speak them.
//! - Every file reports its current readiness
//!   ([`OpenFile::poll`](crate::OpenFile::poll)). A file whose readiness
//!   changes (a pipe, an event counter, a timer, later a terminal or a
//!   socket) has a [`WaitQueue`]; whoever changes the readiness calls
//!   [`WaitQueue::notify`] with the events that may have become true.
//! - Waiting is registering a [`Waker`] on the queues of interest
//!   ([`WaitQueue::register`], undone when the [`Registration`] is
//!   dropped), checking again, and blocking. Blocking itself is the
//!   kernel's: a [`Blocker`] parks the calling thread until its waker
//!   runs, a deadline passes or a signal interrupts it. Host tests use
//!   threads.
//! - An epoll set's interest list entries are wakers too (`epoll.rs`), so
//!   the same notification reaches a thread in `poll` and an epoll set.
//!
//! No lock of this module is held while a waker runs: [`WaitQueue::notify`]
//! copies the wakers out first. A waker may therefore take locks of its
//! own and notify other queues (an epoll set notifies the threads waiting
//! on it, and the sets that contain it).

use crate::file::OpenFile;
use crate::sync::SpinMutex;
use crate::{Errno, Result};
use alloc::sync::Arc;
use alloc::vec::Vec;

/// Poll event bits (`POLLIN` and friends, `EPOLLIN` and friends).
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PollEvents(u32);

impl PollEvents {
    pub const NONE: PollEvents = PollEvents(0);
    /// There is data to read (`POLLIN`).
    pub const IN: PollEvents = PollEvents(0x001);
    /// Urgent data or an exceptional condition (`POLLPRI`).
    pub const PRI: PollEvents = PollEvents(0x002);
    /// Writing is possible now (`POLLOUT`).
    pub const OUT: PollEvents = PollEvents(0x004);
    /// An error condition (`POLLERR`); always reported, never asked for.
    pub const ERR: PollEvents = PollEvents(0x008);
    /// Hang up (`POLLHUP`); always reported, never asked for.
    pub const HUP: PollEvents = PollEvents(0x010);
    /// The descriptor is not open (`POLLNVAL`, poll(2) only).
    pub const NVAL: PollEvents = PollEvents(0x020);
    pub const RDNORM: PollEvents = PollEvents(0x040);
    pub const RDBAND: PollEvents = PollEvents(0x080);
    pub const WRNORM: PollEvents = PollEvents(0x100);
    pub const WRBAND: PollEvents = PollEvents(0x200);
    pub const MSG: PollEvents = PollEvents(0x400);
    /// The peer of a stream socket shut down writing (`POLLRDHUP`).
    pub const RDHUP: PollEvents = PollEvents(0x2000);

    /// Readable, as files report it.
    pub const READABLE: PollEvents = PollEvents(0x001 | 0x040);
    /// Writable, as files report it.
    pub const WRITABLE: PollEvents = PollEvents(0x004 | 0x100);
    /// Reported whether asked for or not (poll(2): "This bit is always
    /// returned; it is not necessary to set it in events").
    pub const ALWAYS: PollEvents = PollEvents(0x008 | 0x010);
    /// Every event bit (the rest of a 32-bit epoll mask are flags).
    pub const ALL: PollEvents = PollEvents(0x27ff);

    pub const fn from_bits(bits: u32) -> PollEvents {
        PollEvents(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: PollEvents) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: PollEvents) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: PollEvents) -> PollEvents {
        PollEvents(self.0 | other.0)
    }

    pub const fn intersection(self, other: PollEvents) -> PollEvents {
        PollEvents(self.0 & other.0)
    }

    pub const fn without(self, other: PollEvents) -> PollEvents {
        PollEvents(self.0 & !other.0)
    }
}

impl core::ops::BitOr for PollEvents {
    type Output = PollEvents;

    fn bitor(self, rhs: PollEvents) -> PollEvents {
        self.union(rhs)
    }
}

impl core::ops::BitAnd for PollEvents {
    type Output = PollEvents;

    fn bitand(self, rhs: PollEvents) -> PollEvents {
        self.intersection(rhs)
    }
}

impl core::fmt::Debug for PollEvents {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PollEvents({:#x})", self.0)
    }
}

/// Something to wake when a file's readiness may have changed: a thread
/// blocked in a read, `poll` or `epoll_wait`, or an epoll set's entry.
/// `events` are the events that may have become true. Wakers run with no
/// lock of the queue held; they must not block.
pub trait Waker: Send + Sync {
    fn wake(&self, events: PollEvents);
}

/// The wakers waiting on one file, each with the events it cares about.
#[derive(Default)]
pub struct WaitQueue {
    waiters: SpinMutex<Waiters>,
}

#[derive(Default)]
struct Waiters {
    next_id: u64,
    list: Vec<(u64, PollEvents, Arc<dyn Waker>)>,
}

impl WaitQueue {
    pub fn new() -> WaitQueue {
        WaitQueue::default()
    }

    /// Wake `waker` on every [`notify`](Self::notify) whose events meet
    /// `interest` or [`PollEvents::ALWAYS`], until the returned
    /// registration is dropped.
    pub fn register(self: &Arc<Self>, interest: PollEvents, waker: Arc<dyn Waker>) -> Registration {
        let mut w = self.waiters.lock();
        let id = w.next_id;
        w.next_id += 1;
        w.list.push((id, interest | PollEvents::ALWAYS, waker));
        Registration { queue: Arc::clone(self), id }
    }

    /// Readiness may have changed by `events`: wake every waiter that
    /// cares, after letting go of the queue's lock.
    pub fn notify(&self, events: PollEvents) {
        let wake: Vec<Arc<dyn Waker>> = {
            let w = self.waiters.lock();
            if w.list.is_empty() {
                return;
            }
            w.list.iter().filter(|(_, interest, _)| interest.intersects(events)).map(|(_, _, k)| k.clone()).collect()
        };
        for waker in wake {
            waker.wake(events);
        }
    }

    /// Registered waiters.
    pub fn len(&self) -> usize {
        self.waiters.lock().list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl core::fmt::Debug for WaitQueue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WaitQueue").field("waiters", &self.len()).finish()
    }
}

/// A waker's place on a [`WaitQueue`]; dropping it takes the waker off.
pub struct Registration {
    queue: Arc<WaitQueue>,
    id: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let removed = {
            let mut w = self.queue.waiters.lock();
            w.list.iter().position(|(id, _, _)| *id == self.id).map(|i| w.list.swap_remove(i))
        };
        // The waker's last reference (an epoll entry, say) goes unlocked.
        drop(removed);
    }
}

impl core::fmt::Debug for Registration {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Registration").field("id", &self.id).finish_non_exhaustive()
    }
}

/// Why [`Blocker::block`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Woke {
    /// The blocker's waker ran (perhaps before the block, or for an
    /// earlier wait: callers check again).
    Woken,
    /// The deadline passed.
    TimedOut,
    /// A signal (or the end of the process) interrupts the wait; the
    /// caller gives up with `EINTR`, which the system call layer turns
    /// into a restart or an error.
    Interrupted,
}

/// How the calling thread blocks; the kernel implements it with its
/// scheduler.
pub trait Blocker {
    /// A waker that ends this thread's [`block`](Self::block), or makes the
    /// next one return at once if it runs first (wake-ups are not lost).
    fn waker(&self) -> Arc<dyn Waker>;

    /// Block until the waker runs, the monotonic clock reaches `until`
    /// (nanoseconds), or the thread is interrupted. Spurious returns are
    /// allowed.
    fn block(&mut self, until: Option<u64>) -> Woke;
}

/// How long a wait may last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timeout {
    /// Do not block: check once.
    Now,
    /// Until the monotonic clock reads this many nanoseconds.
    At(u64),
    /// Until something happens.
    Never,
}

impl Timeout {
    fn until(self) -> Option<u64> {
        match self {
            Timeout::At(t) => Some(t),
            _ => None,
        }
    }
}

/// What one `poll` entry watches.
#[derive(Clone, Debug)]
pub enum PollTarget {
    /// A negative descriptor: ignored, `revents` 0.
    Ignore,
    /// A descriptor that is not open: `POLLNVAL`.
    Closed,
    File(Arc<OpenFile>),
}

/// One entry of [`poll`]: the file, the events reported if true, and (on
/// return) the events that are true.
#[derive(Clone, Debug)]
pub struct PollEntry {
    pub target: PollTarget,
    pub events: PollEvents,
    pub revents: PollEvents,
}

impl PollEntry {
    /// An entry as `poll(2)` makes it: `events` plus `POLLERR` and
    /// `POLLHUP`, which are reported unasked.
    pub fn new(target: PollTarget, events: PollEvents) -> PollEntry {
        PollEntry { target, events: events | PollEvents::ALWAYS, revents: PollEvents::NONE }
    }

    /// An entry that reports exactly `events` (`select(2)` asks for what
    /// it maps). `POLLNVAL` is reported for a closed descriptor either way.
    pub fn exact(target: PollTarget, events: PollEvents) -> PollEntry {
        PollEntry { target, events, revents: PollEvents::NONE }
    }
}

/// Fill in every entry's `revents`; returns how many are not 0.
fn scan(entries: &mut [PollEntry]) -> usize {
    let mut ready = 0;
    for e in entries.iter_mut() {
        e.revents = match &e.target {
            PollTarget::Ignore => PollEvents::NONE,
            PollTarget::Closed => PollEvents::NVAL,
            PollTarget::File(f) => f.poll() & e.events,
        };
        if !e.revents.is_empty() {
            ready += 1;
        }
    }
    ready
}

/// `poll(2)` over open files: fills in `revents` and returns the number of
/// entries with any; waits (registered on every file's queue) until one
/// has some, `timeout` passes (then 0) or the thread is interrupted
/// (`EINTR`). A file without a wait queue never changes on its own; it is
/// checked again whenever the thread wakes.
pub fn poll(entries: &mut [PollEntry], blocker: &mut dyn Blocker, timeout: Timeout) -> Result<usize> {
    let ready = scan(entries);
    if ready > 0 || timeout == Timeout::Now {
        return Ok(ready);
    }
    let waker = blocker.waker();
    let _registered: Vec<Registration> = entries
        .iter()
        .filter_map(|e| match &e.target {
            PollTarget::File(f) => f.wait_queue().map(|q| q.register(e.events, waker.clone())),
            _ => None,
        })
        .collect();
    loop {
        // Checked after registering: a change between the first scan and
        // the registration still wakes us.
        let ready = scan(entries);
        if ready > 0 {
            return Ok(ready);
        }
        match blocker.block(timeout.until()) {
            Woke::Interrupted => return Err(Errno::EINTR),
            Woke::TimedOut => return Ok(scan(entries)),
            Woke::Woken => {}
        }
    }
}

/// Run `attempt` until it stops failing with `EAGAIN`, blocking on
/// `queue` for `interest` in between: the waiting half of a blocking read
/// or write on a file whose operations never block themselves.
/// `EINTR` when the thread is interrupted, `EAGAIN` when `timeout` passes.
pub fn wait_until_done<T>(
    queue: &Arc<WaitQueue>,
    interest: PollEvents,
    blocker: &mut dyn Blocker,
    timeout: Timeout,
    mut attempt: impl FnMut() -> Result<T>,
) -> Result<T> {
    match attempt() {
        Err(Errno::EAGAIN) if timeout != Timeout::Now => {}
        other => return other,
    }
    let _registered = queue.register(interest, blocker.waker());
    loop {
        match attempt() {
            Err(Errno::EAGAIN) => {}
            other => return other,
        }
        match blocker.block(timeout.until()) {
            Woke::Interrupted => return Err(Errno::EINTR),
            Woke::TimedOut => return attempt(),
            Woke::Woken => {}
        }
    }
}
