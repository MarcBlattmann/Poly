//! One io_uring instance: its two shared regions and the state only the
//! kernel sees (pending requests, kept-back CQEs, registrations).
//!
//! The lock ([`crate::ns::lock::Mutex`], a spinlock) only guards memory
//! updates: requests run with it released, so a request that calls back
//! into the kernel never holds it.

use super::abi::{Cqe, Layout, Sqe, sq_flags};
use super::{FileHandle, RingMemory, Watch};
use crate::kernel::Deadline;
use crate::ns::lock::Mutex;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

/// What the kernel does when an instance has news for whoever waits on its
/// descriptor (a CQE was posted, or a waiting request may go on): wake the
/// descriptor's waiters.
pub type Notifier = Arc<dyn Fn() + Send + Sync>;

/// What a request that could not finish at once waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    /// Descriptor `fd` (a temporary one for a registered file, opened again
    /// at every attempt) to be ready for `events`; then the request runs.
    Ready { events: u16 },
    /// A poll request: the events of `fd` it asks for. A multishot one
    /// stays after each CQE; `armed` is false after it fired. With a
    /// [`Watch`] on the file it fires again at every wake-up of the file
    /// (the watch's count moved past `seen`, its count when it fired), as
    /// Linux posts one CQE per wake-up; without one, once the descriptor
    /// was seen not ready.
    Poll { events: u16, multishot: bool, armed: bool, seen: u64 },
    /// A timeout: due at `deadline` (on the monotonic clock), or done
    /// early once the instance has posted `target` CQEs in all.
    Timeout { deadline: Deadline, target: Option<u64> },
}

/// See [`Instance::wait_targets`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WaitTargets {
    pub fds: Vec<(i32, u16)>,
    pub timeout: Option<crate::abi::Timespec>,
    pub recheck: bool,
}

/// A request waiting in an instance, with the rest of its link chain.
#[derive(Clone, Debug)]
pub struct Pending {
    pub sqe: Sqe,
    pub wait: Wait,
    /// The SQEs linked after this one, which start when it completes.
    pub chain: Vec<Sqe>,
    /// The process that submitted it: its descriptor numbers are that
    /// process's, so only its threads run it again.
    pub owner: i32,
    /// The kernel's watch on the request's file (waking the instance when
    /// the file's readiness changes), if it has one.
    pub watch: Option<Arc<dyn Watch>>,
}

impl Pending {
    /// A request of process `owner` waiting for `wait`, without a watch.
    pub fn new(sqe: Sqe, wait: Wait, chain: Vec<Sqe>, owner: i32) -> Pending {
        Pending { sqe, wait, chain, owner, watch: None }
    }
}

#[derive(Default)]
struct State {
    /// The kernel's SQ head and CQ tail (also stored in the region, where
    /// user space reads them).
    sq_head: u32,
    cq_tail: u32,
    sq_dropped: u32,
    sq_flags: u32,
    /// CQEs that did not fit, oldest first (`IORING_FEAT_NODROP`).
    overflow: VecDeque<Cqe>,
    /// CQEs posted in all, for timeouts that count completions.
    posted: u64,
    pending: Vec<Pending>,
    /// `IORING_REGISTER_BUFFERS`: base and length of each buffer.
    buffers: Option<Vec<(u64, u64)>>,
    /// `IORING_REGISTER_FILES`: the table (empty slots `None`).
    files: Option<Vec<Option<FileHandle>>>,
    eventfd: Option<FileHandle>,
    /// A CQE was posted since the eventfd was last signalled.
    signal: bool,
    /// `IORING_SETUP_R_DISABLED` until `IORING_REGISTER_ENABLE_RINGS`.
    disabled: bool,
}

/// An io_uring instance.
pub struct Instance {
    pub layout: Layout,
    /// The `IORING_SETUP_*` flags it was made with.
    pub setup_flags: u32,
    rings: Arc<dyn RingMemory>,
    sqes: Arc<dyn RingMemory>,
    state: Mutex<State>,
    /// A watched file woke since requests were last tried: a waiting
    /// request may go on ([`kick`](Self::kick)).
    kicked: AtomicBool,
    notifier: Mutex<Option<Notifier>>,
}

impl core::fmt::Debug for Instance {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Instance").field("layout", &self.layout).field("setup_flags", &self.setup_flags).finish()
    }
}

impl Instance {
    /// A new instance over `rings` and `sqes` (zero-filled, of the sizes
    /// `layout` asks for), with the ring sizes and masks written.
    pub fn new(layout: Layout, setup_flags: u32, rings: Arc<dyn RingMemory>, sqes: Arc<dyn RingMemory>) -> Instance {
        let put = |off: u32, v: u32| rings.store_release(off as usize, v);
        put(Layout::SQ_MASK, layout.sq_entries - 1);
        put(Layout::SQ_ENTRIES, layout.sq_entries);
        put(Layout::CQ_MASK, layout.cq_entries - 1);
        put(Layout::CQ_ENTRIES, layout.cq_entries);
        let mut state = State { disabled: setup_flags & super::abi::setup::R_DISABLED != 0, ..State::default() };
        // Without a polling thread every submission needs a call: say so,
        // so SQPOLL users call io_uring_enter(IORING_ENTER_SQ_WAKEUP).
        if setup_flags & super::abi::setup::SQPOLL != 0 {
            state.sq_flags = sq_flags::NEED_WAKEUP;
            rings.store_release(Layout::SQ_FLAGS as usize, state.sq_flags);
        }
        Instance {
            layout,
            setup_flags,
            rings,
            sqes,
            state: Mutex::new(state),
            kicked: AtomicBool::new(false),
            notifier: Mutex::new(None),
        }
    }

    /// What to call when the instance has news for its descriptor's
    /// waiters (set by the kernel when it makes the descriptor).
    pub fn set_notifier(&self, n: Notifier) {
        *self.notifier.lock() = Some(n);
    }

    fn notify(&self) {
        let n = self.notifier.lock().clone();
        if let Some(n) = n {
            n();
        }
    }

    /// A watched file of a waiting request woke: note it and wake the
    /// descriptor's waiters (called from the file's wake-up, holding no
    /// lock of the instance).
    pub fn kick(&self) {
        self.kicked.store(true, Ordering::SeqCst);
        self.notify();
    }

    /// Whether a watched file woke since the last [`take_kick`](Self::take_kick).
    pub fn kicked(&self) -> bool {
        self.kicked.load(Ordering::SeqCst)
    }

    /// Clear the kick (before requests are tried again, so a wake-up
    /// during the attempt is not lost).
    pub fn take_kick(&self) -> bool {
        self.kicked.swap(false, Ordering::SeqCst)
    }

    /// The region mmap(2) maps at file offset `offset` (masked with
    /// `IORING_OFF_MASK`): the rings for `IORING_OFF_SQ_RING` and
    /// `IORING_OFF_CQ_RING`, the SQEs for `IORING_OFF_SQES`.
    pub fn region(&self, offset: u64) -> Option<&Arc<dyn RingMemory>> {
        match offset & super::abi::OFF_MASK {
            super::abi::OFF_SQ_RING | super::abi::OFF_CQ_RING => Some(&self.rings),
            super::abi::OFF_SQES => Some(&self.sqes),
            _ => None,
        }
    }

    fn load(&self, off: u32) -> u32 {
        self.rings.load_acquire(off as usize)
    }

    fn store(&self, off: u32, v: u32) {
        self.rings.store_release(off as usize, v);
    }

    /// Whether the ring is still disabled (`IORING_SETUP_R_DISABLED`).
    pub fn disabled(&self) -> bool {
        self.state.lock().disabled
    }

    /// `IORING_REGISTER_ENABLE_RINGS`: `false` if it was not disabled.
    pub fn enable(&self) -> bool {
        core::mem::replace(&mut self.state.lock().disabled, false)
    }

    /// Take up to `max` SQEs from the SQ ring: advance the SQ head past
    /// each entry looked at; an SQ array value that names no SQE is
    /// dropped (the `dropped` counter grows) and not returned or counted.
    pub fn take_sqes(&self, max: u32) -> Vec<Sqe> {
        let mut s = self.state.lock();
        let tail = self.load(Layout::SQ_TAIL);
        let n = tail.wrapping_sub(s.sq_head).min(self.layout.sq_entries).min(max);
        let mask = self.layout.sq_entries - 1;
        let mut out = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let slot = s.sq_head & mask;
            s.sq_head = s.sq_head.wrapping_add(1);
            let index = match self.layout.array {
                Some(a) => self.rings.load_acquire(a as usize + 4 * slot as usize),
                None => slot,
            };
            if index >= self.layout.sq_entries {
                s.sq_dropped = s.sq_dropped.wrapping_add(1);
                self.store(Layout::SQ_DROPPED, s.sq_dropped);
                continue;
            }
            let mut raw = [0; Sqe::SIZE];
            self.sqes.read(index as usize * self.layout.sqe_size as usize, &mut raw);
            out.push(Sqe::from_bytes(&raw));
        }
        self.store(Layout::SQ_HEAD, s.sq_head);
        out
    }

    /// CQEs in the CQ ring that user space has not consumed.
    pub fn cq_ready(&self) -> u32 {
        let tail = self.state.lock().cq_tail;
        tail.wrapping_sub(self.load(Layout::CQ_HEAD))
    }

    /// Whether some CQEs are kept back (`IORING_SQ_CQ_OVERFLOW`).
    pub fn overflowing(&self) -> bool {
        !self.state.lock().overflow.is_empty()
    }

    /// Post `cqe`: into the CQ ring if it has room and nothing is kept
    /// back, otherwise at the end of the kept-back list.
    pub fn post(&self, cqe: Cqe) {
        {
            let mut s = self.state.lock();
            s.posted += 1;
            s.signal = true;
            self.flush_locked(&mut s);
            if s.overflow.is_empty() && self.room(&s) {
                self.write_cqe(&mut s, cqe);
            } else {
                s.overflow.push_back(cqe);
                self.set_overflow_flag(&mut s);
            }
        }
        self.notify();
    }

    /// Move kept-back CQEs into the ring while it has room.
    pub fn flush_overflow(&self) {
        let mut s = self.state.lock();
        self.flush_locked(&mut s);
    }

    /// CQEs posted in all.
    pub fn posted(&self) -> u64 {
        self.state.lock().posted
    }

    /// Whether a CQE was posted since the last call (for the registered
    /// eventfd), and the eventfd to signal.
    pub fn take_signal(&self) -> Option<FileHandle> {
        let mut s = self.state.lock();
        if !core::mem::replace(&mut s.signal, false) {
            return None;
        }
        s.eventfd.clone()
    }

    fn room(&self, s: &State) -> bool {
        s.cq_tail.wrapping_sub(self.load(Layout::CQ_HEAD)) < self.layout.cq_entries
    }

    fn write_cqe(&self, s: &mut State, cqe: Cqe) {
        let at =
            Layout::CQES as usize + (s.cq_tail & (self.layout.cq_entries - 1)) as usize * self.layout.cqe_size as usize;
        self.rings.write(at, &cqe.to_bytes());
        if self.layout.cqe_size > 16 {
            self.rings.write(at + 16, &[0; 16]);
        }
        s.cq_tail = s.cq_tail.wrapping_add(1);
        self.store(Layout::CQ_TAIL, s.cq_tail);
    }

    fn flush_locked(&self, s: &mut State) {
        if s.overflow.is_empty() {
            return;
        }
        while self.room(s) {
            let Some(cqe) = s.overflow.pop_front() else { break };
            self.write_cqe(s, cqe);
        }
        if s.overflow.is_empty() {
            s.sq_flags &= !sq_flags::CQ_OVERFLOW;
            self.store(Layout::SQ_FLAGS, s.sq_flags);
        }
    }

    fn set_overflow_flag(&self, s: &mut State) {
        s.sq_flags |= sq_flags::CQ_OVERFLOW;
        self.store(Layout::SQ_FLAGS, s.sq_flags);
    }

    // ---- pending requests ---------------------------------------------

    /// Add a request that waits.
    pub fn add_pending(&self, p: Pending) {
        self.state.lock().pending.push(p);
    }

    /// Take every pending request out (to try them without the lock).
    pub fn take_pending(&self) -> Vec<Pending> {
        core::mem::take(&mut self.state.lock().pending)
    }

    /// Put requests that still wait back, before any added meanwhile.
    pub fn restore_pending(&self, mut kept: Vec<Pending>) {
        let mut s = self.state.lock();
        kept.append(&mut s.pending);
        s.pending = kept;
    }

    /// What a `GETEVENTS` wait sleeps on: the descriptors (and events) of
    /// the waiting requests, the earliest timeout, and whether some
    /// request can only be looked at again later (a fired multishot poll,
    /// a request on a registered file).
    pub fn wait_targets(&self, owner: i32) -> WaitTargets {
        let s = self.state.lock();
        let mut t = WaitTargets::default();
        for p in &s.pending {
            let fixed = p.sqe.flags & super::abi::sqe_flags::FIXED_FILE != 0;
            match p.wait {
                Wait::Ready { events } | Wait::Poll { events, armed: true, .. } if !fixed && p.owner == owner => {
                    t.fds.push((p.sqe.fd, events))
                }
                // A fired multishot poll with a watch: the watch wakes the
                // descriptor's waiters at the file's next wake-up.
                Wait::Poll { armed: false, .. } if p.watch.is_some() => {}
                Wait::Timeout { deadline, .. } => {
                    if t.timeout.is_none_or(|d| (deadline.at.sec, deadline.at.nsec) < (d.sec, d.nsec)) {
                        t.timeout = Some(deadline.at);
                    }
                }
                _ => t.recheck = true,
            }
        }
        t
    }

    /// Whether any request waits.
    pub fn has_pending(&self) -> bool {
        !self.state.lock().pending.is_empty()
    }

    /// Whether a request of process `owner` waits.
    pub fn has_pending_of(&self, owner: i32) -> bool {
        self.state.lock().pending.iter().any(|p| p.owner == owner)
    }

    /// Remove the first pending request with `user_data` that `pick`
    /// accepts.
    pub fn remove_pending(&self, user_data: u64, pick: impl Fn(&Pending) -> bool) -> Option<Pending> {
        let mut s = self.state.lock();
        let i = s.pending.iter().position(|p| p.sqe.user_data == user_data && pick(p))?;
        Some(s.pending.remove(i))
    }

    /// Change the pending request with `user_data` that `pick` accepts with
    /// `f`; whether there was one.
    pub fn update_pending(
        &self,
        user_data: u64,
        pick: impl Fn(&Pending) -> bool,
        f: impl FnOnce(&mut Pending),
    ) -> bool {
        let mut s = self.state.lock();
        match s.pending.iter_mut().find(|p| p.sqe.user_data == user_data && pick(p)) {
            Some(p) => {
                f(p);
                true
            }
            None => false,
        }
    }

    // ---- registrations ------------------------------------------------

    /// Register buffers; `false` if some are registered already.
    pub fn set_buffers(&self, b: Vec<(u64, u64)>) -> bool {
        let mut s = self.state.lock();
        if s.buffers.is_some() {
            return false;
        }
        s.buffers = Some(b);
        true
    }

    /// Whether buffers are registered.
    pub fn has_buffers(&self) -> bool {
        self.state.lock().buffers.is_some()
    }

    /// Unregister the buffers; `false` if there were none.
    pub fn clear_buffers(&self) -> bool {
        self.state.lock().buffers.take().is_some()
    }

    /// Whether `[addr, addr + len)` lies inside registered buffer `index`.
    pub fn fixed_buffer_ok(&self, index: u16, addr: u64, len: u32) -> bool {
        let s = self.state.lock();
        let Some((base, blen)) = s.buffers.as_ref().and_then(|b| b.get(usize::from(index)).copied()) else {
            return false;
        };
        let Some(end) = addr.checked_add(u64::from(len)) else { return false };
        addr >= base && end <= base + blen
    }

    /// Register a file table; `false` if one is registered already.
    pub fn set_files(&self, f: Vec<Option<FileHandle>>) -> bool {
        let mut s = self.state.lock();
        if s.files.is_some() {
            return false;
        }
        s.files = Some(f);
        true
    }

    /// Unregister the file table; `false` if there was none.
    pub fn clear_files(&self) -> bool {
        self.state.lock().files.take().is_some()
    }

    /// The size of the file table (`None`: no table).
    pub fn files_len(&self) -> Option<usize> {
        self.state.lock().files.as_ref().map(Vec::len)
    }

    /// Replace slot `i` of the file table.
    pub fn set_file(&self, i: usize, f: Option<FileHandle>) {
        if let Some(slot) = self.state.lock().files.as_mut().and_then(|t| t.get_mut(i)) {
            *slot = f;
        }
    }

    /// The file in slot `i` of the table, if there is a table and one.
    pub fn file(&self, i: usize) -> Option<FileHandle> {
        self.state.lock().files.as_ref().and_then(|t| t.get(i).cloned().flatten())
    }

    /// Register the eventfd; `false` if one is registered already.
    pub fn set_eventfd(&self, h: FileHandle) -> bool {
        let mut s = self.state.lock();
        if s.eventfd.is_some() {
            return false;
        }
        s.eventfd = Some(h);
        true
    }

    /// Unregister the eventfd; `false` if there was none.
    pub fn clear_eventfd(&self) -> bool {
        self.state.lock().eventfd.take().is_some()
    }
}
