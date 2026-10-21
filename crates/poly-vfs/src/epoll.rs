//! Epoll sets, as epoll(7), epoll_ctl(2) and epoll_wait(2) describe them.
//!
//! An [`Epoll`] has an interest list (entries keyed by descriptor number
//! and open file description, as Linux keys them) and a ready list. Each
//! entry is a [`Waker`] registered on its file's [`WaitQueue`]: a
//! notification puts it on the ready list and wakes the threads waiting on
//! the set (and the sets that contain this one). [`Epoll::wait`] takes the
//! ready list and asks each file what it is ready for now:
//!
//! - level-triggered (the default): a file still ready goes back on the
//!   ready list, so the next wait reports it again;
//! - edge-triggered (`EPOLLET`): it is reported once per notification;
//! - `EPOLLONESHOT`: after one report the entry is disabled until
//!   `EPOLL_CTL_MOD` re-arms it.
//!
//! `EPOLLEXCLUSIVE` is accepted with the documented restrictions and
//! otherwise has no effect: every waiter is woken (the man page allows
//! "one or more"). `EPOLLWAKEUP` is accepted and ignored (there is no
//! suspend). An entry whose open file description has been closed
//! everywhere is dropped at the next call (epoll(7): closing every
//! descriptor of a description removes it from all interest lists).
//! Regular files and directories cannot be added (`EPERM`).

use crate::file::OpenFile;
use crate::object::FileObject;
use crate::poll::{Blocker, PollEvents, Registration, Timeout, WaitQueue, Waker, Woke};
use crate::sync::SpinMutex;
use crate::{Errno, Result};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

/// `EPOLLEXCLUSIVE` (UAPI `linux/eventpoll.h`).
pub const EPOLLEXCLUSIVE: u32 = 1 << 28;
/// `EPOLLWAKEUP`.
pub const EPOLLWAKEUP: u32 = 1 << 29;
/// `EPOLLONESHOT`.
pub const EPOLLONESHOT: u32 = 1 << 30;
/// `EPOLLET`.
pub const EPOLLET: u32 = 1 << 31;

/// The flags `EPOLLEXCLUSIVE` may be combined with (epoll_ctl(2):
/// "EPOLLIN, EPOLLOUT, EPOLLWAKEUP, and EPOLLET").
const EXCLUSIVE_OK: u32 = 0x001 | 0x004 | EPOLLWAKEUP | EPOLLET | EPOLLEXCLUSIVE;

/// How deep epoll sets may nest (epoll_ctl(2): "a nesting depth of epoll
/// instances greater than 5" is `ELOOP`).
pub const MAX_NESTING: usize = 5;

/// `epoll_ctl`'s operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpollOp {
    Add,
    Mod,
    Del,
}

/// `struct epoll_event`: the events (and flags) and the caller's data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EpollEvent {
    pub events: u32,
    pub data: u64,
}

/// An interest list entry's key: the descriptor and the open file
/// description's address.
type Key = (i32, usize);

fn key_of(fd: i32, file: &Arc<OpenFile>) -> Key {
    (fd, Arc::as_ptr(file) as usize)
}

/// One interest list entry.
struct Item {
    key: Key,
    file: Weak<OpenFile>,
    set: Weak<Epoll>,
    /// Events, flags and data, as the last `ADD` / `MOD` gave them.
    event: SpinMutex<EpollEvent>,
    /// On the ready list.
    queued: AtomicBool,
    /// `EPOLLONESHOT` reported: no more events until `MOD`.
    disabled: AtomicBool,
}

impl Item {
    fn interest(&self) -> PollEvents {
        PollEvents::from_bits(self.event.lock().events) & PollEvents::ALL
    }
}

impl Waker for Item {
    fn wake(&self, events: PollEvents) {
        if self.disabled.load(Ordering::Acquire) || !events.intersects(self.interest() | PollEvents::ALWAYS) {
            return;
        }
        if let Some(set) = self.set.upgrade() {
            set.mark_ready(self.key);
        }
    }
}

struct Entry {
    item: Arc<Item>,
    /// On the file's wait queue (none for a file whose readiness never
    /// changes).
    registration: Option<Registration>,
}

#[derive(Default)]
struct State {
    items: BTreeMap<Key, Entry>,
    ready: VecDeque<Key>,
}

/// An epoll set.
pub struct Epoll {
    state: SpinMutex<State>,
    queue: Arc<WaitQueue>,
}

impl Epoll {
    pub fn new() -> Arc<Epoll> {
        Arc::new(Epoll { state: SpinMutex::new(State::default()), queue: Arc::new(WaitQueue::new()) })
    }

    /// Entries in the interest list.
    pub fn len(&self) -> usize {
        self.state.lock().items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Put entry `key` on the ready list (once) and wake the waiters.
    fn mark_ready(&self, key: Key) {
        {
            let mut s = self.state.lock();
            let Some(entry) = s.items.get(&key) else { return };
            if entry.item.queued.swap(true, Ordering::AcqRel) {
                return;
            }
            s.ready.push_back(key);
        }
        self.queue.notify(PollEvents::READABLE);
    }

    /// Drop entries whose open file description is gone.
    fn prune(s: &mut State) -> Vec<Entry> {
        let dead: Vec<Key> = s.items.iter().filter(|(_, e)| e.item.file.strong_count() == 0).map(|(k, _)| *k).collect();
        dead.iter().filter_map(|k| s.items.remove(k)).collect()
    }

    /// The epoll sets in this one's interest list.
    fn children(&self) -> Vec<Arc<Epoll>> {
        let files: Vec<Arc<OpenFile>> =
            self.state.lock().items.values().filter_map(|e| e.item.file.upgrade()).collect();
        files.iter().filter_map(|f| f.object_as::<Epoll>()).collect()
    }

    /// Whether `target` reaches `self` through nested sets, or nests more
    /// than `budget` deep.
    fn too_deep_or_loops(&self, target: &Arc<Epoll>, budget: usize) -> bool {
        if core::ptr::eq(Arc::as_ptr(target), self) {
            return true;
        }
        if budget == 0 {
            return true;
        }
        target.children().iter().any(|c| self.too_deep_or_loops(c, budget - 1))
    }

    /// `epoll_ctl(2)` for descriptor `fd`, open as `file`. Errors:
    /// `EINVAL` for `file` being this set, bad `EPOLLEXCLUSIVE` use;
    /// `EPERM` for a regular file or directory; `EEXIST` (`Add`) and
    /// `ENOENT` (`Mod`, `Del`); `ELOOP` for a loop of sets or nesting
    /// deeper than [`MAX_NESTING`].
    pub fn ctl(self: &Arc<Self>, op: EpollOp, fd: i32, file: &Arc<OpenFile>, event: EpollEvent) -> Result<()> {
        if file.object_as::<Epoll>().is_some_and(|e| Arc::ptr_eq(&e, self)) {
            return Err(Errno::EINVAL);
        }
        if file.object().is_none() && file.device().is_none() {
            return Err(if file.is_path_only() { Errno::EBADF } else { Errno::EPERM });
        }
        let exclusive = event.events & EPOLLEXCLUSIVE != 0;
        if op == EpollOp::Add && exclusive && event.events & !EXCLUSIVE_OK != 0 {
            return Err(Errno::EINVAL);
        }
        if op == EpollOp::Add
            && let Some(inner) = file.object_as::<Epoll>()
        {
            if exclusive {
                return Err(Errno::EINVAL);
            }
            if self.too_deep_or_loops(&inner, MAX_NESTING - 1) {
                return Err(Errno::ELOOP);
            }
        }
        let key = key_of(fd, file);
        let mut dropped = Vec::new();
        let mut unregistered = None;
        {
            let mut s = self.state.lock();
            dropped.extend(Self::prune(&mut s));
            match op {
                EpollOp::Add => {
                    if s.items.contains_key(&key) {
                        return Err(Errno::EEXIST);
                    }
                    let item = Arc::new(Item {
                        key,
                        file: Arc::downgrade(file),
                        set: Arc::downgrade(self),
                        event: SpinMutex::new(event),
                        queued: AtomicBool::new(false),
                        disabled: AtomicBool::new(false),
                    });
                    let interest = item.interest();
                    let registration = file.wait_queue().map(|q| q.register(interest, item.clone()));
                    s.items.insert(key, Entry { item, registration });
                }
                EpollOp::Mod => {
                    let entry = s.items.get_mut(&key).ok_or(Errno::ENOENT)?;
                    let old = *entry.item.event.lock();
                    if exclusive || old.events & EPOLLEXCLUSIVE != 0 {
                        return Err(Errno::EINVAL);
                    }
                    *entry.item.event.lock() = event;
                    entry.item.disabled.store(false, Ordering::Release);
                    let interest = entry.item.interest();
                    let item = entry.item.clone();
                    unregistered = entry.registration.take();
                    entry.registration = file.wait_queue().map(|q| q.register(interest, item));
                }
                EpollOp::Del => {
                    let entry = s.items.remove(&key).ok_or(Errno::ENOENT)?;
                    s.ready.retain(|k| *k != key);
                    dropped.push(entry);
                }
            }
        }
        // Old registrations go with no lock of the set held.
        drop(dropped);
        drop(unregistered);
        if op != EpollOp::Del {
            // A file that is ready already is reported by the next wait.
            let interest = PollEvents::from_bits(event.events) & PollEvents::ALL;
            if file.poll().intersects(interest | PollEvents::ALWAYS) {
                self.mark_ready(key);
            }
        }
        Ok(())
    }

    /// Take up to `max` events off the ready list (see the module
    /// documentation for what goes back on it).
    fn collect(&self, max: usize) -> Vec<EpollEvent> {
        let batch: Vec<Arc<Item>> = {
            let mut s = self.state.lock();
            let keys: Vec<Key> = s.ready.drain(..).collect();
            keys.iter()
                .filter_map(|k| s.items.get(k))
                .map(|e| {
                    e.item.queued.store(false, Ordering::Release);
                    e.item.clone()
                })
                .collect()
        };
        let mut out = Vec::new();
        // The entries not looked at go back first, those reported (and
        // still level-triggered) behind them: with a small `maxevents`
        // every ready entry gets its turn (epoll_wait's spec sheet: round
        // robin).
        let mut again = Vec::new();
        let mut reported = Vec::new();
        let mut dead = false;
        for item in batch {
            if out.len() >= max {
                again.push(item);
                continue;
            }
            if item.disabled.load(Ordering::Acquire) {
                continue;
            }
            let Some(file) = item.file.upgrade() else {
                dead = true;
                continue;
            };
            let event = *item.event.lock();
            let interest = PollEvents::from_bits(event.events) & PollEvents::ALL;
            let events = file.poll() & (interest | PollEvents::ALWAYS);
            if events.is_empty() {
                continue;
            }
            out.push(EpollEvent { events: events.bits(), data: event.data });
            if event.events & EPOLLONESHOT != 0 {
                item.disabled.store(true, Ordering::Release);
            } else if event.events & EPOLLET == 0 {
                reported.push(item);
            }
        }
        again.append(&mut reported);
        let mut dropped = Vec::new();
        {
            let mut s = self.state.lock();
            for item in again {
                let current = s.items.get(&item.key).is_some_and(|e| Arc::ptr_eq(&e.item, &item));
                if current && !item.queued.swap(true, Ordering::AcqRel) {
                    s.ready.push_back(item.key);
                }
            }
            if dead {
                dropped = Self::prune(&mut s);
            }
        }
        drop(dropped);
        out
    }

    /// `epoll_wait(2)`: up to `max` (at least 1) ready events; waits until
    /// there are some, `timeout` passes (then none) or the thread is
    /// interrupted (`EINTR`).
    pub fn wait(&self, max: usize, blocker: &mut dyn Blocker, timeout: Timeout) -> Result<Vec<EpollEvent>> {
        let max = max.max(1);
        let out = self.collect(max);
        if !out.is_empty() || timeout == Timeout::Now {
            return Ok(out);
        }
        let _registered = self.queue.register(PollEvents::READABLE, blocker.waker());
        let until = match timeout {
            Timeout::At(t) => Some(t),
            _ => None,
        };
        loop {
            let out = self.collect(max);
            if !out.is_empty() {
                return Ok(out);
            }
            match blocker.block(until) {
                Woke::Interrupted => return Err(Errno::EINTR),
                Woke::TimedOut => return Ok(self.collect(max)),
                Woke::Woken => {}
            }
        }
    }
}

impl core::fmt::Debug for Epoll {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = self.state.lock();
        f.debug_struct("Epoll").field("items", &s.items.len()).field("ready", &s.ready.len()).finish()
    }
}

impl FileObject for Epoll {
    fn seek_stays_at_zero(&self) -> bool {
        true
    }

    fn read(&self, _: &mut [u8]) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn write(&self, _: &[u8]) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    /// Readable when a wait would report something (epoll(7): an epoll
    /// file descriptor is "readable" when it has events), without taking
    /// anything off the ready list.
    fn poll(&self) -> PollEvents {
        let ready: Vec<Arc<Item>> = {
            let s = self.state.lock();
            s.ready.iter().filter_map(|k| s.items.get(k)).map(|e| e.item.clone()).collect()
        };
        let any = ready.iter().any(|item| {
            !item.disabled.load(Ordering::Acquire)
                && item.file.upgrade().is_some_and(|f| f.poll().intersects(item.interest() | PollEvents::ALWAYS))
        });
        if any { PollEvents::READABLE } else { PollEvents::NONE }
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.queue)
    }
}
