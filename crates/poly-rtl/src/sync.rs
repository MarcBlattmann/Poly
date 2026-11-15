//! Thread synchronisation of the run-time library: slim reader/writer locks, condition variables,
//! run-once (one-time initialisation) and `RtlWaitOnAddress`, one implementation for `ntdll`'s
//! `Rtl*` exports and `kernel32`'s names (Learn: "Slim Reader/Writer (SRW) Locks", "Condition
//! Variables", "One-Time Initialization", `WaitOnAddress`).
//!
//! The objects are pointer-sized words the caller owns (`winnt.h`: `RTL_SRWLOCK`,
//! `RTL_CONDITION_VARIABLE`, `RTL_RUN_ONCE`; zero is the initial state). Whoever has to wait
//! parks on a [`Lot`]: a table of buckets keyed by the word's address, each a spin lock and a
//! first-come-first-served list of waiters that live on the waiters' own stacks, so nothing is
//! allocated. A waiter blocks on an event of its own (`Sys`), so a wake is exactly one event
//! set. The words are only touched by atomic operations; while a word has waiters, its changes
//! are made under its bucket's lock.
//!
//! # Words (what a program may observe; the tests of `specs/windows/ntdll.dll` pin them)
//!
//! * SRW lock: 0 free; bit 0 is set while the lock is held; bits 4 and up count the shared
//!   holders, so exclusive is `1` and `n` readers are `(n << 4) | 1`; bit 1 says that waiters
//!   are queued (not observable by a program that does not wait). A waiting writer holds back
//!   readers that come later; a release hands the lock directly to the first waiter, or to all
//!   the readers at the head of the queue, so waiters are served in turn and nobody barges.
//! * Condition variable: 0 while nobody sleeps; 1 while somebody does. The sleepers are woken
//!   in the order they went to sleep.
//! * Run-once: 0 not started; `1` (bit 0) an initialisation is running (`| 4` while threads
//!   wait for it); `context | 2` once it has finished (`context`'s two low bits are zero).

use core::cell::UnsafeCell;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// `STATUS_UNSUCCESSFUL`, `STATUS_TIMEOUT`, `STATUS_PENDING`, `STATUS_RESOURCE_NOT_OWNED`,
/// `STATUS_INVALID_PARAMETER_2`, `STATUS_INVALID_PARAMETER_3` (`ntstatus.h`).
pub const STATUS_UNSUCCESSFUL: u32 = 0xC000_0001;
pub const STATUS_TIMEOUT: u32 = 0x0000_0102;
pub const STATUS_PENDING: u32 = 0x0000_0103;
pub const STATUS_RESOURCE_NOT_OWNED: u32 = 0xC000_0264;
pub const STATUS_INVALID_PARAMETER_2: u32 = 0xC000_00F0;
pub const STATUS_INVALID_PARAMETER_3: u32 = 0xC000_00F1;

/// `RTL_RUN_ONCE_CHECK_ONLY`, `RTL_RUN_ONCE_ASYNC`, `RTL_RUN_ONCE_INIT_FAILED` (`winnt.h`).
pub const RUN_ONCE_CHECK_ONLY: u32 = 1;
pub const RUN_ONCE_ASYNC: u32 = 2;
pub const RUN_ONCE_INIT_FAILED: u32 = 4;
/// `RTL_CONDITION_VARIABLE_LOCKMODE_SHARED` (`winnt.h`).
pub const CONDITION_VARIABLE_LOCKMODE_SHARED: u32 = 1;

/// What waiting needs from the system: an auto-reset event per blocked thread, waiting on it
/// with an optional timeout, setting it, and giving the processor away.
pub trait Sys {
    /// An auto-reset event that is not signalled (events handed back with [`Sys::put_event`]
    /// may be reused); 0 if none could be made (the waiter then spins).
    fn get_event(&self) -> u64;
    /// Hand an unsignalled event back.
    fn put_event(&self, event: u64);
    /// Wait for the event: `timeout` is `NtWaitForSingleObject`'s (`None` forever, negative
    /// relative in 100 ns, positive absolute). `true` when it was signalled, `false` on timeout.
    fn wait(&self, event: u64, timeout: Option<i64>) -> bool;
    /// Signal the event.
    fn set(&self, event: u64);
    /// Give the processor away (spin loops).
    fn yield_now(&self);
}

// ----- the parking lot -----------------------------------------------------------------------

const KIND_EXCLUSIVE: u8 = 0;
const KIND_SHARED: u8 = 1;
const KIND_CONDITION: u8 = 2;
const KIND_ONCE: u8 = 3;
const KIND_ADDRESS: u8 = 4;

/// One waiter: on its thread's stack while it is queued.
struct Node {
    next: *mut Node,
    prev: *mut Node,
    addr: usize,
    kind: u8,
    /// In the bucket's list (changed under the bucket's lock).
    linked: bool,
    event: u64,
    /// Set by the waker before it signals the event.
    done: AtomicBool,
}

impl Node {
    fn new(addr: usize, kind: u8) -> Node {
        Node { next: null_mut(), prev: null_mut(), addr, kind, linked: false, event: 0, done: AtomicBool::new(false) }
    }
}

struct Bucket {
    lock: AtomicBool,
    head: UnsafeCell<*mut Node>,
    tail: UnsafeCell<*mut Node>,
}

impl Bucket {
    const fn new() -> Bucket {
        Bucket { lock: AtomicBool::new(false), head: UnsafeCell::new(null_mut()), tail: UnsafeCell::new(null_mut()) }
    }

    fn acquire(&self, sys: &dyn Sys) {
        let mut spins = 0u32;
        while self.lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            spins += 1;
            if spins < 100 {
                core::hint::spin_loop();
            } else {
                sys.yield_now();
            }
        }
    }

    fn release(&self) {
        self.lock.store(false, Ordering::Release);
    }

    /// Queue `node` last. The bucket's lock is held.
    unsafe fn push(&self, node: *mut Node) {
        // SAFETY: the lock is held, `node` is valid and not queued.
        unsafe {
            (*node).next = null_mut();
            (*node).prev = *self.tail.get();
            if (*self.tail.get()).is_null() {
                *self.head.get() = node;
            } else {
                (**self.tail.get()).next = node;
            }
            *self.tail.get() = node;
            (*node).linked = true;
        }
    }

    /// Unqueue `node`. The bucket's lock is held and `node` is queued here.
    unsafe fn remove(&self, node: *mut Node) {
        // SAFETY: the lock is held; the neighbours are queued nodes.
        unsafe {
            let (p, n) = ((*node).prev, (*node).next);
            if p.is_null() {
                *self.head.get() = n;
            } else {
                (*p).next = n;
            }
            if n.is_null() {
                *self.tail.get() = p;
            } else {
                (*n).prev = p;
            }
            (*node).next = null_mut();
            (*node).prev = null_mut();
            (*node).linked = false;
        }
    }

    /// The first node queued for `addr`, or null. The lock is held.
    unsafe fn first(&self, addr: usize) -> *mut Node {
        // SAFETY: the lock is held, every queued node is valid.
        unsafe {
            let mut n = *self.head.get();
            while !n.is_null() && (*n).addr != addr {
                n = (*n).next;
            }
            n
        }
    }
}

const BUCKETS: usize = 64;

/// The table the waiters of one process park in (one `static` per DLL that blocks threads).
pub struct Lot {
    buckets: [Bucket; BUCKETS],
}

// SAFETY: every field of a bucket is only touched with its spin lock held.
unsafe impl Sync for Lot {}
// SAFETY: as above; the nodes it points to belong to blocked threads.
unsafe impl Send for Lot {}

impl core::fmt::Debug for Lot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Lot").finish_non_exhaustive()
    }
}

impl Default for Lot {
    fn default() -> Lot {
        Lot::new()
    }
}

/// SRW lock word bits.
const LOCKED: usize = 1;
const WAITERS: usize = 2;
const READER: usize = 16;
/// Run-once word bits.
const RUNNING: usize = 1;
const DONE: usize = 2;
const ONCE_WAITERS: usize = 4;

impl Lot {
    pub const fn new() -> Lot {
        Lot { buckets: [const { Bucket::new() }; BUCKETS] }
    }

    fn bucket(&self, addr: usize) -> &Bucket {
        &self.buckets[((addr >> 4) ^ (addr >> 10)) % BUCKETS]
    }

    /// Block until the waker has handed `node` over, or `timeout` ends: the bucket's lock must
    /// be released and `node` queued. Returns `true` when woken. A node that times out is
    /// unqueued here (and `false` returned); one that a waker took in the meantime counts as
    /// woken.
    unsafe fn park(&self, sys: &dyn Sys, node: *mut Node, timeout: Option<i64>) -> bool {
        // SAFETY: `node` is the caller's, queued, and valid until this returns.
        unsafe {
            let ev = (*node).event;
            if ev == 0 {
                // No event could be made: spin until the waker is done (timeouts are ignored).
                while !(*node).done.load(Ordering::Acquire) {
                    sys.yield_now();
                }
                return true;
            }
            if sys.wait(ev, timeout) {
                // The wake sets `done` before the event.
                return true;
            }
            let b = self.bucket((*node).addr);
            b.acquire(sys);
            let linked = (*node).linked;
            if linked {
                b.remove(node);
            }
            b.release();
            if linked {
                return false;
            }
            // A waker took the node while we timed out: its signal is on the way.
            sys.wait(ev, None);
            true
        }
    }

    /// Wake `node` (already unqueued; the waker's own bucket lock may be held): the node's
    /// owner may return as soon as the event is set, so nothing of it is touched afterwards.
    unsafe fn wake(&self, sys: &dyn Sys, node: *mut Node) {
        // SAFETY: the owner is blocked in `park` and does not return before the event is set.
        unsafe {
            let ev = (*node).event;
            (*node).done.store(true, Ordering::Release);
            if ev != 0 {
                sys.set(ev);
            }
        }
    }

    /// Queue `node` with a new event, the bucket's lock held.
    unsafe fn enqueue(&self, sys: &dyn Sys, b: &Bucket, node: *mut Node) {
        // SAFETY: valid node; the lock is held.
        unsafe {
            (*node).event = sys.get_event();
            b.push(node);
        }
    }

    fn finish(&self, sys: &dyn Sys, node: &Node) {
        if node.event != 0 {
            sys.put_event(node.event);
        }
    }

    // ----- SRW locks ---------------------------------------------------------------------

    /// `RtlTryAcquireSRWLockExclusive`.
    pub fn try_acquire_exclusive(&self, word: &AtomicUsize) -> bool {
        word.compare_exchange(0, LOCKED, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }

    /// `RtlTryAcquireSRWLockShared`.
    pub fn try_acquire_shared(&self, word: &AtomicUsize) -> bool {
        let mut s = word.load(Ordering::Relaxed);
        loop {
            // Held exclusively (no readers counted) or somebody waits: no.
            let new = if s == 0 {
                READER | LOCKED
            } else if s & WAITERS == 0 && s >= READER {
                s + READER
            } else {
                return false;
            };
            match word.compare_exchange_weak(s, new, Ordering::Acquire, Ordering::Relaxed) {
                Ok(_) => return true,
                Err(v) => s = v,
            }
        }
    }

    /// `RtlAcquireSRWLockExclusive`.
    pub fn acquire_exclusive(&self, sys: &dyn Sys, word: &AtomicUsize) {
        if self.try_acquire_exclusive(word) {
            return;
        }
        let addr = word as *const AtomicUsize as usize;
        let mut node = Node::new(addr, KIND_EXCLUSIVE);
        let b = self.bucket(addr);
        b.acquire(sys);
        let mut s = word.load(Ordering::Relaxed);
        loop {
            if s == 0 {
                match word.compare_exchange_weak(0, LOCKED, Ordering::Acquire, Ordering::Relaxed) {
                    Ok(_) => {
                        b.release();
                        return;
                    }
                    Err(v) => s = v,
                }
            } else if s & WAITERS != 0 {
                break;
            } else {
                match word.compare_exchange_weak(s, s | WAITERS, Ordering::Relaxed, Ordering::Relaxed) {
                    Ok(_) => break,
                    Err(v) => s = v,
                }
            }
        }
        let n: *mut Node = &mut node;
        // SAFETY: `node` lives on this stack until `park` returns it unqueued.
        unsafe {
            self.enqueue(sys, b, n);
            b.release();
            // The releaser hands the lock over before it wakes us.
            self.park(sys, n, None);
        }
        self.finish(sys, &node);
    }

    /// `RtlAcquireSRWLockShared`.
    pub fn acquire_shared(&self, sys: &dyn Sys, word: &AtomicUsize) {
        if self.try_acquire_shared(word) {
            return;
        }
        let addr = word as *const AtomicUsize as usize;
        let mut node = Node::new(addr, KIND_SHARED);
        let b = self.bucket(addr);
        b.acquire(sys);
        let mut s = word.load(Ordering::Relaxed);
        loop {
            if s & WAITERS != 0 {
                break;
            }
            let (new, got) = if s == 0 {
                (READER | LOCKED, true)
            } else if s >= READER {
                (s + READER, true)
            } else {
                (s | WAITERS, false)
            };
            match word.compare_exchange_weak(s, new, Ordering::Acquire, Ordering::Relaxed) {
                Ok(_) if got => {
                    b.release();
                    return;
                }
                Ok(_) => break,
                Err(v) => s = v,
            }
        }
        let n: *mut Node = &mut node;
        // SAFETY: as in `acquire_exclusive`.
        unsafe {
            self.enqueue(sys, b, n);
            b.release();
            self.park(sys, n, None);
        }
        self.finish(sys, &node);
    }

    /// Give a lock that has just become free to the first waiter, or to all the readers at the
    /// head of the queue, and store the word that says so. The bucket's lock is held.
    unsafe fn hand_over(&self, sys: &dyn Sys, b: &Bucket, word: &AtomicUsize, addr: usize) {
        // SAFETY: the lock is held; queued nodes are valid.
        unsafe {
            let head = b.first(addr);
            if head.is_null() {
                word.store(0, Ordering::Release);
                return;
            }
            let exclusive = (*head).kind == KIND_EXCLUSIVE;
            // Take the nodes to wake off the queue first, chained through `next`.
            let (mut chain, mut last): (*mut Node, *mut Node) = (null_mut(), null_mut());
            let mut count = 0usize;
            loop {
                let n = b.first(addr);
                if n.is_null() || (count > 0 && (exclusive || (*n).kind != KIND_SHARED)) {
                    break;
                }
                b.remove(n);
                if last.is_null() {
                    chain = n;
                } else {
                    (*last).next = n;
                }
                last = n;
                count += 1;
            }
            let held = if exclusive { LOCKED } else { (count * READER) | LOCKED };
            let more = if b.first(addr).is_null() { 0 } else { WAITERS };
            word.store(held | more, Ordering::Release);
            let mut n = chain;
            while !n.is_null() {
                let next = (*n).next;
                self.wake(sys, n);
                n = next;
            }
        }
    }

    /// `RtlReleaseSRWLockExclusive`: releasing a lock that is not held exclusively does nothing.
    pub fn release_exclusive(&self, sys: &dyn Sys, word: &AtomicUsize) {
        let addr = word as *const AtomicUsize as usize;
        loop {
            let s = word.load(Ordering::Relaxed);
            if s == 0 || (s & WAITERS == 0 && s != LOCKED) {
                // Free, or held shared: not an exclusive hold.
                return;
            }
            if s == LOCKED {
                if word.compare_exchange_weak(LOCKED, 0, Ordering::Release, Ordering::Relaxed).is_ok() {
                    return;
                }
                continue;
            }
            // Waiters are queued: the hand-over is made under the bucket's lock.
            let b = self.bucket(addr);
            b.acquire(sys);
            let s = word.load(Ordering::Relaxed);
            if s & WAITERS == 0 {
                b.release();
                continue;
            }
            if s < READER {
                // SAFETY: the bucket's lock is held.
                unsafe { self.hand_over(sys, b, word, addr) };
            }
            b.release();
            return;
        }
    }

    /// `RtlReleaseSRWLockShared`: `Err(STATUS_RESOURCE_NOT_OWNED)` for a free lock (the caller
    /// raises it); a lock held exclusively counts as held by one reader and is freed.
    pub fn release_shared(&self, sys: &dyn Sys, word: &AtomicUsize) -> Result<(), u32> {
        let addr = word as *const AtomicUsize as usize;
        loop {
            let s = word.load(Ordering::Relaxed);
            if s == 0 {
                return Err(STATUS_RESOURCE_NOT_OWNED);
            }
            if s & WAITERS == 0 {
                let new = if s < 2 * READER { 0 } else { s - READER };
                if word.compare_exchange_weak(s, new, Ordering::Release, Ordering::Relaxed).is_ok() {
                    return Ok(());
                }
                continue;
            }
            let b = self.bucket(addr);
            b.acquire(sys);
            let s = word.load(Ordering::Relaxed);
            if s & WAITERS == 0 {
                b.release();
                continue;
            }
            if s >= 2 * READER {
                // Other readers still hold it.
                word.store(s - READER, Ordering::Release);
            } else {
                // The last holder (or an exclusive one, counted as a reader) is gone.
                // SAFETY: the bucket's lock is held.
                unsafe { self.hand_over(sys, b, word, addr) };
            }
            b.release();
            return Ok(());
        }
    }

    // ----- condition variables -----------------------------------------------------------

    /// `RtlSleepConditionVariable*`: queue as a sleeper, run `unlock` (which gives up the lock
    /// the caller holds), wait until woken or `timeout` ends, then run `relock`. `true` when
    /// woken, `false` on timeout.
    pub fn sleep(
        &self,
        sys: &dyn Sys,
        cv: &AtomicUsize,
        timeout: Option<i64>,
        unlock: &mut dyn FnMut(),
        relock: &mut dyn FnMut(),
    ) -> bool {
        let addr = cv as *const AtomicUsize as usize;
        let mut node = Node::new(addr, KIND_CONDITION);
        let n: *mut Node = &mut node;
        let b = self.bucket(addr);
        b.acquire(sys);
        // SAFETY: `node` lives on this stack until `park` returns it unqueued.
        unsafe { self.enqueue(sys, b, n) };
        cv.store(1, Ordering::Release);
        b.release();
        unlock();
        // SAFETY: as above.
        let woken = unsafe { self.park(sys, n, timeout) };
        if !woken {
            // Unqueued by `park`: the last sleeper leaving clears the word.
            b.acquire(sys);
            // SAFETY: the lock is held.
            if unsafe { b.first(addr) }.is_null() {
                cv.store(0, Ordering::Release);
            }
            b.release();
        }
        self.finish(sys, &node);
        relock();
        woken
    }

    /// `RtlWakeConditionVariable`: the sleeper that has waited longest.
    pub fn wake_one(&self, sys: &dyn Sys, cv: &AtomicUsize) {
        self.wake_sleepers(sys, cv, false);
    }

    /// `RtlWakeAllConditionVariable`.
    pub fn wake_all(&self, sys: &dyn Sys, cv: &AtomicUsize) {
        self.wake_sleepers(sys, cv, true);
    }

    fn wake_sleepers(&self, sys: &dyn Sys, cv: &AtomicUsize, all: bool) {
        if cv.load(Ordering::Acquire) == 0 {
            return;
        }
        let addr = cv as *const AtomicUsize as usize;
        let b = self.bucket(addr);
        b.acquire(sys);
        // SAFETY: the lock is held; woken owners return only after their event is set.
        unsafe {
            loop {
                let n = b.first(addr);
                if n.is_null() {
                    break;
                }
                b.remove(n);
                self.wake(sys, n);
                if !all {
                    break;
                }
            }
            if b.first(addr).is_null() {
                cv.store(0, Ordering::Release);
            }
        }
        b.release();
    }

    // ----- run-once ------------------------------------------------------------------------

    /// `RtlRunOnceBeginInitialize`. `Ok(Some(context))`: done (`STATUS_SUCCESS`); `Ok(None)`: the
    /// caller must initialise (`STATUS_PENDING`); `Err(status)`.
    pub fn run_once_begin(&self, sys: &dyn Sys, once: &AtomicUsize, flags: u32) -> Result<Option<usize>, u32> {
        if flags & !(RUN_ONCE_CHECK_ONLY | RUN_ONCE_ASYNC) != 0 {
            return Err(STATUS_INVALID_PARAMETER_2);
        }
        let addr = once as *const AtomicUsize as usize;
        loop {
            let s = once.load(Ordering::Acquire);
            if s & DONE != 0 {
                return Ok(Some(s & !3));
            }
            if flags & RUN_ONCE_CHECK_ONLY != 0 {
                return Err(STATUS_UNSUCCESSFUL);
            }
            if s & RUNNING == 0 {
                if once.compare_exchange(s, s | RUNNING, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                    return Ok(None);
                }
                continue;
            }
            if flags & RUN_ONCE_ASYNC != 0 {
                return Ok(None);
            }
            // Somebody initialises: wait for the end of its run.
            let mut node = Node::new(addr, KIND_ONCE);
            let n: *mut Node = &mut node;
            let b = self.bucket(addr);
            b.acquire(sys);
            let s = once.load(Ordering::Acquire);
            let queued = s & (RUNNING | DONE) == RUNNING
                && (s & ONCE_WAITERS != 0
                    || once.compare_exchange(s, s | ONCE_WAITERS, Ordering::Relaxed, Ordering::Relaxed).is_ok());
            if !queued {
                b.release();
                continue;
            }
            // SAFETY: `node` lives on this stack until `park` returns.
            unsafe {
                self.enqueue(sys, b, n);
                b.release();
                self.park(sys, n, None);
            }
            self.finish(sys, &node);
        }
    }

    /// `RtlRunOnceComplete`: `flags` is `RUN_ONCE_ASYNC` and/or `RUN_ONCE_INIT_FAILED`.
    pub fn run_once_complete(&self, sys: &dyn Sys, once: &AtomicUsize, flags: u32, context: usize) -> Result<(), u32> {
        if flags & !(RUN_ONCE_ASYNC | RUN_ONCE_INIT_FAILED) != 0 {
            return Err(STATUS_INVALID_PARAMETER_2);
        }
        let failed = flags & RUN_ONCE_INIT_FAILED != 0;
        if context & 3 != 0 || (failed && context != 0) {
            return Err(STATUS_INVALID_PARAMETER_3);
        }
        let new = if failed { 0 } else { context | DONE };
        let mut s = once.load(Ordering::Acquire);
        loop {
            if s & RUNNING == 0 || s & DONE != 0 {
                return Err(STATUS_UNSUCCESSFUL);
            }
            match once.compare_exchange_weak(s, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => break,
                Err(v) => s = v,
            }
        }
        if s & ONCE_WAITERS != 0 {
            let addr = once as *const AtomicUsize as usize;
            let b = self.bucket(addr);
            b.acquire(sys);
            // SAFETY: the lock is held; the owners return only after their event is set.
            unsafe {
                loop {
                    let n = b.first(addr);
                    if n.is_null() {
                        break;
                    }
                    b.remove(n);
                    self.wake(sys, n);
                }
            }
            b.release();
        }
        Ok(())
    }

    // ----- WaitOnAddress -------------------------------------------------------------------

    /// `RtlWaitOnAddress` / `WaitOnAddress`: sleep while `size` bytes (1, 2, 4 or 8) at `address`
    /// equal those at `compare`. `Ok(true)` woken or the value differs, `Ok(false)` timeout.
    ///
    /// # Safety
    /// Both pointers are valid for `size` bytes.
    pub unsafe fn wait_on_address(
        &self,
        sys: &dyn Sys,
        address: *const u8,
        compare: *const u8,
        size: usize,
        timeout: Option<i64>,
    ) -> bool {
        let addr = address as usize;
        let mut node = Node::new(addr, KIND_ADDRESS);
        let n: *mut Node = &mut node;
        let b = self.bucket(addr);
        b.acquire(sys);
        // SAFETY: the caller's pointers; atomic loads of the sizes a program can wait on.
        let same = unsafe {
            match size {
                1 => (*(address as *const core::sync::atomic::AtomicU8)).load(Ordering::SeqCst) == *compare,
                2 => {
                    (*(address as *const core::sync::atomic::AtomicU16)).load(Ordering::SeqCst)
                        == (compare as *const u16).read_unaligned()
                }
                4 => {
                    (*(address as *const core::sync::atomic::AtomicU32)).load(Ordering::SeqCst)
                        == (compare as *const u32).read_unaligned()
                }
                _ => {
                    (*(address as *const core::sync::atomic::AtomicU64)).load(Ordering::SeqCst)
                        == (compare as *const u64).read_unaligned()
                }
            }
        };
        if !same {
            b.release();
            return true;
        }
        // SAFETY: `node` lives on this stack until `park` returns it unqueued.
        let woken = unsafe {
            self.enqueue(sys, b, n);
            b.release();
            self.park(sys, n, timeout)
        };
        self.finish(sys, &node);
        woken
    }

    /// `RtlWakeAddressSingle` and `RtlWakeAddressAll`.
    pub fn wake_address(&self, sys: &dyn Sys, address: *const u8, all: bool) {
        let addr = address as usize;
        let b = self.bucket(addr);
        b.acquire(sys);
        // SAFETY: the lock is held.
        unsafe {
            loop {
                let n = b.first(addr);
                if n.is_null() {
                    break;
                }
                b.remove(n);
                self.wake(sys, n);
                if !all {
                    break;
                }
            }
        }
        b.release();
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU64;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};
    use std::vec::Vec;

    /// Events as (signalled, condvar) pairs, and a pool of the ones handed back.
    #[derive(Default)]
    struct Host {
        events: Mutex<HashMap<u64, Arc<(Mutex<bool>, Condvar)>>>,
        free: Mutex<Vec<u64>>,
        next: AtomicU64,
        made: AtomicU64,
    }

    impl Host {
        fn ev(&self, e: u64) -> Arc<(Mutex<bool>, Condvar)> {
            self.events.lock().unwrap().get(&e).unwrap().clone()
        }
    }

    impl Sys for Host {
        fn get_event(&self) -> u64 {
            if let Some(e) = self.free.lock().unwrap().pop() {
                return e;
            }
            let h = self.next.fetch_add(4, Ordering::Relaxed) + 4;
            self.events.lock().unwrap().insert(h, Arc::new((Mutex::new(false), Condvar::new())));
            self.made.fetch_add(1, Ordering::Relaxed);
            h
        }
        fn put_event(&self, event: u64) {
            assert!(!*self.ev(event).0.lock().unwrap(), "an event is handed back signalled");
            self.free.lock().unwrap().push(event);
        }
        fn wait(&self, event: u64, timeout: Option<i64>) -> bool {
            let e = self.ev(event);
            let (m, cv) = &*e;
            let mut g = m.lock().unwrap();
            let deadline = timeout.map(|t| {
                // Negative: relative in 100 ns; a positive (absolute) time is "in the past".
                Instant::now() + Duration::from_nanos(if t < 0 { (-t as u64) * 100 } else { 0 })
            });
            while !*g {
                match deadline {
                    None => g = cv.wait(g).unwrap(),
                    Some(d) => {
                        let now = Instant::now();
                        if now >= d {
                            return false;
                        }
                        g = cv.wait_timeout(g, d - now).unwrap().0;
                    }
                }
            }
            *g = false;
            true
        }
        fn set(&self, event: u64) {
            let e = self.ev(event);
            *e.0.lock().unwrap() = true;
            e.1.notify_one();
        }
        fn yield_now(&self) {
            std::thread::yield_now();
        }
    }

    fn rel_ms(ms: u64) -> Option<i64> {
        Some(-((ms * 10_000) as i64))
    }

    #[test]
    fn srw_single_thread_words() {
        let (lot, host, w) = (Lot::new(), Host::default(), AtomicUsize::new(0));
        assert!(lot.try_acquire_exclusive(&w));
        assert_eq!(w.load(Ordering::Relaxed), 1);
        assert!(!lot.try_acquire_exclusive(&w));
        assert!(!lot.try_acquire_shared(&w));
        lot.release_exclusive(&host, &w);
        assert_eq!(w.load(Ordering::Relaxed), 0);
        for n in 1..=8usize {
            assert!(lot.try_acquire_shared(&w));
            assert_eq!(w.load(Ordering::Relaxed), (n << 4) | 1);
        }
        assert!(!lot.try_acquire_exclusive(&w));
        for _ in 0..8 {
            lot.release_shared(&host, &w).unwrap();
        }
        assert_eq!(w.load(Ordering::Relaxed), 0);
        // Releasing a free lock.
        assert_eq!(lot.release_shared(&host, &w), Err(STATUS_RESOURCE_NOT_OWNED));
        lot.release_exclusive(&host, &w);
        assert_eq!(w.load(Ordering::Relaxed), 0);
        // A shared release of an exclusive hold frees it; an exclusive release of readers does nothing.
        assert!(lot.try_acquire_exclusive(&w));
        assert_eq!(lot.release_shared(&host, &w), Ok(()));
        assert_eq!(w.load(Ordering::Relaxed), 0);
        assert!(lot.try_acquire_shared(&w));
        lot.release_exclusive(&host, &w);
        assert_eq!(w.load(Ordering::Relaxed), 0x11);
    }

    #[test]
    fn srw_blocks_and_hands_over() {
        let lot = Arc::new(Lot::new());
        let host = Arc::new(Host::default());
        let w = Arc::new(AtomicUsize::new(0));
        let got = Arc::new(AtomicU64::new(0));
        lot.acquire_exclusive(&*host, &w);
        let t = {
            let (lot, host, w, got) = (lot.clone(), host.clone(), w.clone(), got.clone());
            std::thread::spawn(move || {
                lot.acquire_exclusive(&*host, &w);
                got.store(1, Ordering::SeqCst);
                lot.release_exclusive(&*host, &w);
            })
        };
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(got.load(Ordering::SeqCst), 0);
        // The waiter is queued: the word says so, and a try (even shared) fails.
        assert_eq!(w.load(Ordering::Relaxed), 3);
        assert!(!lot.try_acquire_shared(&w));
        lot.release_exclusive(&*host, &w);
        t.join().unwrap();
        assert_eq!(got.load(Ordering::SeqCst), 1);
        assert_eq!(w.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn waiting_writer_holds_back_later_readers() {
        let lot = Arc::new(Lot::new());
        let host = Arc::new(Host::default());
        let w = Arc::new(AtomicUsize::new(0));
        let order = Arc::new(Mutex::new(Vec::new()));
        lot.acquire_shared(&*host, &w);
        let spawn = |name: &'static str, shared: bool| {
            let (lot, host, w, order) = (lot.clone(), host.clone(), w.clone(), order.clone());
            std::thread::spawn(move || {
                if shared {
                    lot.acquire_shared(&*host, &w);
                    order.lock().unwrap().push(name);
                    lot.release_shared(&*host, &w).unwrap();
                } else {
                    lot.acquire_exclusive(&*host, &w);
                    order.lock().unwrap().push(name);
                    lot.release_exclusive(&*host, &w);
                }
            })
        };
        let writer = spawn("writer", false);
        std::thread::sleep(Duration::from_millis(100));
        assert!(!lot.try_acquire_shared(&w), "a waiting writer holds readers back");
        let reader = spawn("reader", true);
        std::thread::sleep(Duration::from_millis(100));
        assert!(order.lock().unwrap().is_empty());
        lot.release_shared(&*host, &w).unwrap();
        writer.join().unwrap();
        reader.join().unwrap();
        assert_eq!(*order.lock().unwrap(), ["writer", "reader"]);
        assert_eq!(w.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn readers_waiting_behind_a_writer_enter_together() {
        let lot = Arc::new(Lot::new());
        let host = Arc::new(Host::default());
        let w = Arc::new(AtomicUsize::new(0));
        let inside = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        lot.acquire_exclusive(&*host, &w);
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let (lot, host, w, inside, max) = (lot.clone(), host.clone(), w.clone(), inside.clone(), max.clone());
                std::thread::spawn(move || {
                    lot.acquire_shared(&*host, &w);
                    let n = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    max.fetch_max(n, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(100));
                    inside.fetch_sub(1, Ordering::SeqCst);
                    lot.release_shared(&*host, &w).unwrap();
                })
            })
            .collect();
        std::thread::sleep(Duration::from_millis(100));
        lot.release_exclusive(&*host, &w);
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(max.load(Ordering::SeqCst), 4, "the queued readers took the lock together");
        assert_eq!(w.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn srw_mutual_exclusion_under_load() {
        let lot = Arc::new(Lot::new());
        let host = Arc::new(Host::default());
        let w = Arc::new(AtomicUsize::new(0));
        let (a, b) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
        let bad = Arc::new(AtomicU64::new(0));
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let (lot, host, w, a, b, bad) =
                    (lot.clone(), host.clone(), w.clone(), a.clone(), b.clone(), bad.clone());
                std::thread::spawn(move || {
                    for k in 0..3000 {
                        if i % 3 == 0 || (i % 3 == 1 && k % 7 == 0) {
                            if !(k % 5 == 0 && lot.try_acquire_exclusive(&w)) {
                                lot.acquire_exclusive(&*host, &w);
                            }
                            let v = a.load(Ordering::Relaxed);
                            a.store(v + 1, Ordering::Relaxed);
                            std::hint::spin_loop();
                            b.store(v + 1, Ordering::Relaxed);
                            lot.release_exclusive(&*host, &w);
                        } else {
                            lot.acquire_shared(&*host, &w);
                            if a.load(Ordering::Relaxed) != b.load(Ordering::Relaxed) {
                                bad.fetch_add(1, Ordering::Relaxed);
                            }
                            lot.release_shared(&*host, &w).unwrap();
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(bad.load(Ordering::Relaxed), 0, "a reader saw a writer half done");
        assert_eq!(a.load(Ordering::Relaxed), b.load(Ordering::Relaxed));
        assert_eq!(w.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn srw_lock_has_no_owner() {
        let (lot, host, w) = (Lot::new(), Host::default(), AtomicUsize::new(0));
        let lot = Arc::new(lot);
        let host = Arc::new(host);
        let w = Arc::new(w);
        {
            let (lot, host, w) = (lot.clone(), host.clone(), w.clone());
            std::thread::spawn(move || lot.acquire_exclusive(&*host, &w)).join().unwrap();
        }
        assert!(!lot.try_acquire_exclusive(&w));
        lot.release_exclusive(&*host, &w);
        assert!(lot.try_acquire_exclusive(&w));
    }

    #[test]
    fn condition_variable_wakes_in_order_and_times_out() {
        let lot = Arc::new(Lot::new());
        let host = Arc::new(Host::default());
        let (cv, lock) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        // Nobody wakes it: a timeout, and the lock comes back.
        lot.acquire_exclusive(&*host, &lock);
        let t0 = Instant::now();
        let woken = lot.sleep(
            &*host,
            &cv,
            rel_ms(100),
            &mut || lot.release_exclusive(&*host, &lock),
            &mut || lot.acquire_exclusive(&*host, &lock),
        );
        assert!(!woken);
        assert!(t0.elapsed() >= Duration::from_millis(90));
        assert_eq!(lock.load(Ordering::Relaxed), 1, "the lock is held again");
        assert_eq!(cv.load(Ordering::Relaxed), 0);
        lot.release_exclusive(&*host, &lock);
        // A wake with nobody asleep is lost.
        lot.wake_one(&*host, &cv);
        lot.wake_all(&*host, &cv);
        lot.acquire_exclusive(&*host, &lock);
        assert!(!lot.sleep(
            &*host,
            &cv,
            rel_ms(50),
            &mut || lot.release_exclusive(&*host, &lock),
            &mut || lot.acquire_exclusive(&*host, &lock)
        ));
        lot.release_exclusive(&*host, &lock);
        // Three sleepers, woken one at a time in the order they went to sleep.
        let order = Arc::new(Mutex::new(Vec::new()));
        let threads: Vec<_> = (0..3)
            .map(|i| {
                let (lot, host, cv, lock, order) = (lot.clone(), host.clone(), cv.clone(), lock.clone(), order.clone());
                let t = std::thread::spawn(move || {
                    lot.acquire_exclusive(&*host, &lock);
                    let woken = lot.sleep(
                        &*host,
                        &cv,
                        None,
                        &mut || lot.release_exclusive(&*host, &lock),
                        &mut || lot.acquire_exclusive(&*host, &lock),
                    );
                    assert!(woken);
                    assert_eq!(lock.load(Ordering::Relaxed) & 1, 1, "returns with the lock");
                    order.lock().unwrap().push(i);
                    lot.release_exclusive(&*host, &lock);
                });
                std::thread::sleep(Duration::from_millis(100));
                t
            })
            .collect();
        assert_eq!(cv.load(Ordering::Relaxed), 1);
        for k in 1..=3 {
            lot.wake_one(&*host, &cv);
            std::thread::sleep(Duration::from_millis(100));
            assert_eq!(order.lock().unwrap().len(), k);
        }
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(*order.lock().unwrap(), [0, 1, 2]);
        assert_eq!(cv.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn woken_sleeper_returns_only_when_the_waker_lets_go() {
        let lot = Arc::new(Lot::new());
        let host = Arc::new(Host::default());
        let (cv, lock) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let back = Arc::new(AtomicU64::new(0));
        let t = {
            let (lot, host, cv, lock, back) = (lot.clone(), host.clone(), cv.clone(), lock.clone(), back.clone());
            std::thread::spawn(move || {
                lot.acquire_shared(&*host, &lock);
                assert!(lot.sleep(
                    &*host,
                    &cv,
                    None,
                    &mut || {
                        lot.release_shared(&*host, &lock).unwrap();
                    },
                    &mut || lot.acquire_shared(&*host, &lock)
                ));
                back.store(1, Ordering::SeqCst);
                lot.release_shared(&*host, &lock).unwrap();
            })
        };
        std::thread::sleep(Duration::from_millis(100));
        lot.acquire_exclusive(&*host, &lock);
        lot.wake_all(&*host, &cv);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(back.load(Ordering::SeqCst), 0, "still waiting for the lock");
        lot.release_exclusive(&*host, &lock);
        t.join().unwrap();
        assert_eq!(back.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn condition_variable_ping_pong_loses_no_wake() {
        let lot = Arc::new(Lot::new());
        let host = Arc::new(Host::default());
        let (cv, lock) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let turn = Arc::new(AtomicU64::new(0));
        let log = Arc::new(Mutex::new(Vec::new()));
        let threads: Vec<_> = (0..2u64)
            .map(|me| {
                let (lot, host, cv, lock, turn, log) =
                    (lot.clone(), host.clone(), cv.clone(), lock.clone(), turn.clone(), log.clone());
                std::thread::spawn(move || {
                    for _ in 0..300 {
                        lot.acquire_exclusive(&*host, &lock);
                        while turn.load(Ordering::Relaxed) != me {
                            let woken = lot.sleep(
                                &*host,
                                &cv,
                                rel_ms(5000),
                                &mut || lot.release_exclusive(&*host, &lock),
                                &mut || lot.acquire_exclusive(&*host, &lock),
                            );
                            assert!(woken, "a wake was lost");
                        }
                        log.lock().unwrap().push(me);
                        turn.store(1 - me, Ordering::Relaxed);
                        lot.wake_all(&*host, &cv);
                        lot.release_exclusive(&*host, &lock);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 600);
        assert!(log.windows(2).all(|p| p[0] != p[1]));
    }

    #[test]
    fn many_sleepers_all_wake_and_exclude_each_other() {
        let lot = Arc::new(Lot::new());
        let host = Arc::new(Host::default());
        let (cv, lock) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (go, inside, max) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (lot, host, cv, lock, go, inside, max) =
                    (lot.clone(), host.clone(), cv.clone(), lock.clone(), go.clone(), inside.clone(), max.clone());
                std::thread::spawn(move || {
                    lot.acquire_exclusive(&*host, &lock);
                    while go.load(Ordering::Relaxed) == 0 {
                        lot.sleep(
                            &*host,
                            &cv,
                            None,
                            &mut || lot.release_exclusive(&*host, &lock),
                            &mut || lot.acquire_exclusive(&*host, &lock),
                        );
                    }
                    let n = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    max.fetch_max(n, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(5));
                    inside.fetch_sub(1, Ordering::SeqCst);
                    lot.release_exclusive(&*host, &lock);
                })
            })
            .collect();
        std::thread::sleep(Duration::from_millis(150));
        lot.acquire_exclusive(&*host, &lock);
        go.store(1, Ordering::Relaxed);
        lot.wake_all(&*host, &cv);
        lot.release_exclusive(&*host, &lock);
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(max.load(Ordering::SeqCst), 1);
        assert_eq!(lock.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn run_once_words_and_statuses() {
        let (lot, host, w) = (Lot::new(), Host::default(), AtomicUsize::new(0));
        assert_eq!(lot.run_once_begin(&host, &w, RUN_ONCE_CHECK_ONLY), Err(STATUS_UNSUCCESSFUL));
        assert_eq!(lot.run_once_complete(&host, &w, 0, 0x800), Err(STATUS_UNSUCCESSFUL), "nobody began");
        assert_eq!(lot.run_once_begin(&host, &w, 0x10), Err(STATUS_INVALID_PARAMETER_2));
        assert_eq!(lot.run_once_begin(&host, &w, 8), Err(STATUS_INVALID_PARAMETER_2));
        assert_eq!(lot.run_once_begin(&host, &w, 0), Ok(None));
        assert_eq!(w.load(Ordering::Relaxed), 1);
        assert_eq!(lot.run_once_begin(&host, &w, RUN_ONCE_CHECK_ONLY), Err(STATUS_UNSUCCESSFUL));
        assert_eq!(lot.run_once_begin(&host, &w, RUN_ONCE_ASYNC), Ok(None), "async does not wait");
        assert_eq!(lot.run_once_complete(&host, &w, 0, 3), Err(STATUS_INVALID_PARAMETER_3));
        assert_eq!(lot.run_once_complete(&host, &w, 8, 0), Err(STATUS_INVALID_PARAMETER_2));
        assert_eq!(lot.run_once_complete(&host, &w, RUN_ONCE_INIT_FAILED, 0x400), Err(STATUS_INVALID_PARAMETER_3));
        assert_eq!(lot.run_once_complete(&host, &w, RUN_ONCE_INIT_FAILED, 0), Ok(()));
        assert_eq!(w.load(Ordering::Relaxed), 0);
        assert_eq!(lot.run_once_begin(&host, &w, 0), Ok(None));
        assert_eq!(lot.run_once_complete(&host, &w, 0, 0x404), Ok(()));
        assert_eq!(w.load(Ordering::Relaxed) & 3, 2);
        assert_eq!(lot.run_once_begin(&host, &w, RUN_ONCE_CHECK_ONLY), Ok(Some(0x404)));
        assert_eq!(lot.run_once_complete(&host, &w, 0, 0), Err(STATUS_UNSUCCESSFUL), "already complete");
        w.store(0, Ordering::Relaxed);
        assert_eq!(lot.run_once_begin(&host, &w, 0), Ok(None));
        assert_eq!(lot.run_once_complete(&host, &w, 0, 0x800), Ok(()));
        assert_eq!(w.load(Ordering::Relaxed), 0x802);
        assert_eq!(lot.run_once_begin(&host, &w, RUN_ONCE_ASYNC), Ok(Some(0x800)));
    }

    #[test]
    fn run_once_second_thread_waits_and_takes_over_after_a_failure() {
        let lot = Arc::new(Lot::new());
        let host = Arc::new(Host::default());
        let w = Arc::new(AtomicUsize::new(0));
        assert_eq!(lot.run_once_begin(&*host, &w, 0), Ok(None));
        let waiter = |expect_pending: bool| {
            let (lot, host, w) = (lot.clone(), host.clone(), w.clone());
            std::thread::spawn(move || {
                let r = lot.run_once_begin(&*host, &w, 0);
                if expect_pending {
                    assert_eq!(r, Ok(None));
                    lot.run_once_complete(&*host, &w, 0, 0xC00).unwrap();
                } else {
                    assert_eq!(r, Ok(Some(0xC00)));
                }
            })
        };
        let t = waiter(false);
        let t2 = waiter(false);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(w.load(Ordering::Relaxed), 5, "running, with waiters");
        lot.run_once_complete(&*host, &w, 0, 0xC00).unwrap();
        t.join().unwrap();
        t2.join().unwrap();
        // The first initialisation fails: a waiter takes over and completes.
        w.store(0, Ordering::Relaxed);
        assert_eq!(lot.run_once_begin(&*host, &w, 0), Ok(None));
        let t = waiter(true);
        std::thread::sleep(Duration::from_millis(100));
        lot.run_once_complete(&*host, &w, RUN_ONCE_INIT_FAILED, 0).unwrap();
        t.join().unwrap();
        assert_eq!(w.load(Ordering::Relaxed), 0xC02);
    }

    #[test]
    fn run_once_runs_once_for_many_threads() {
        for _ in 0..20 {
            let lot = Arc::new(Lot::new());
            let host = Arc::new(Host::default());
            let w = Arc::new(AtomicUsize::new(0));
            let runs = Arc::new(AtomicU64::new(0));
            let threads: Vec<_> = (0..6)
                .map(|_| {
                    let (lot, host, w, runs) = (lot.clone(), host.clone(), w.clone(), runs.clone());
                    std::thread::spawn(move || match lot.run_once_begin(&*host, &w, 0).unwrap() {
                        None => {
                            runs.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(3));
                            lot.run_once_complete(&*host, &w, 0, 0x40).unwrap();
                            0x40
                        }
                        Some(c) => c,
                    })
                })
                .collect();
            for t in threads {
                assert_eq!(t.join().unwrap(), 0x40);
            }
            assert_eq!(runs.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn wait_on_address_wakes_and_compares() {
        let lot = Arc::new(Lot::new());
        let host = Arc::new(Host::default());
        let v = Arc::new(AtomicU64::new(7));
        let compare = 7u32;
        let cell = Arc::new(core::sync::atomic::AtomicU32::new(7));
        // A different value: returns at once.
        let other = 8u32;
        // SAFETY: valid pointers of the stated size.
        unsafe {
            assert!(lot.wait_on_address(&*host, cell.as_ptr().cast(), (&other as *const u32).cast(), 4, rel_ms(50)));
            // Same value, nobody wakes: timeout.
            assert!(!lot.wait_on_address(&*host, cell.as_ptr().cast(), (&compare as *const u32).cast(), 4, rel_ms(50)));
        }
        let t = {
            let (lot, host, cell, v) = (lot.clone(), host.clone(), cell.clone(), v.clone());
            std::thread::spawn(move || {
                let c = 7u32;
                // SAFETY: valid pointers of the stated size.
                let r = unsafe { lot.wait_on_address(&*host, cell.as_ptr().cast(), (&c as *const u32).cast(), 4, None) };
                v.store(r as u64 + 100, Ordering::SeqCst);
            })
        };
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(v.load(Ordering::SeqCst), 7);
        cell.store(9, Ordering::SeqCst);
        lot.wake_address(&*host, cell.as_ptr().cast(), false);
        t.join().unwrap();
        assert_eq!(v.load(Ordering::SeqCst), 101);
    }

    #[test]
    fn events_are_reused() {
        let (lot, host, w) = (Arc::new(Lot::new()), Arc::new(Host::default()), Arc::new(AtomicUsize::new(0)));
        for _ in 0..50 {
            lot.acquire_exclusive(&*host, &w);
            let t = {
                let (lot, host, w) = (lot.clone(), host.clone(), w.clone());
                std::thread::spawn(move || {
                    lot.acquire_exclusive(&*host, &w);
                    lot.release_exclusive(&*host, &w);
                })
            };
            std::thread::sleep(Duration::from_millis(2));
            lot.release_exclusive(&*host, &w);
            t.join().unwrap();
        }
        assert!(host.made.load(Ordering::Relaxed) <= 4, "events came back to the pool");
    }
}
