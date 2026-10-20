//! Kernel locks.
//!
//! - [`SpinLock`] busy-waits and keeps interrupts disabled on this CPU while
//!   it is held, so an interrupt handler that takes the same lock can never
//!   find it held by the code it interrupted (which would spin forever), and
//!   the holder is never preempted. Critical sections must be short and must
//!   never block, yield or sleep: [`spinlocks_held`] counts the locks this
//!   CPU holds and the scheduler refuses to switch threads while it is not
//!   0. It is a real SMP lock: an atomic exchange with Acquire / Release
//!   ordering. While a CPU spins (interrupts off) it keeps answering TLB
//!   shootdowns (`arch::wait_hook`), so a holder that waits for this CPU's
//!   acknowledgement cannot deadlock against it.
//! - [`RawIrqSpinLock`] is the same lock without data, for the kernel heap
//!   (`poly_heap::LockedHeap::with_lock`).
//! - [`Mutex`] sleeps instead of spinning: a thread that finds it locked
//!   blocks on a wait queue until the holder unlocks it. It may be held
//!   across blocking calls (including another mutex, sleep or join), but
//!   cannot be taken in an interrupt handler (that panics) or by the idle
//!   thread. Locking a mutex the thread already holds is a deadlock and
//!   panics.
//! - [`RawMutex`] is a [`Mutex`] without data, the sleeping
//!   `poly_vfs::lock::RawLock` the file system crates hold across disk
//!   I/O (ext2's volume lock, the page cache's inode locks).
//! - [`SetOnce`] holds a value written once during boot and read without a
//!   lock afterwards (the local APIC, the clock).
//!
//! Lock order where locks nest: the heap, then `paging::KERNEL` or a user
//! address space's tables, then [`FRAMES`](crate::frames::FRAMES); the
//! scheduler's `THREADS` takes `FRAMES` only with `try_lock`, and drops the
//! user state of exited threads (address spaces) only after unlocking.
//! Nothing allocates while holding `FRAMES`. `THREADS` is a leaf apart
//! from that: code holding it takes no other lock (waking threads from
//! under another lock is fine: that lock comes first). A user address
//! space's table lock may be held while a TLB shootdown waits for other
//! CPUs (see `arch/x86_64/tlb.rs`); the shootdown's own lock comes after
//! every other lock. The full order is in docs/ARCHITECTURE.md
//! ("Synchronisation").

use crate::arch::cpu::{self, InterruptsOff};
use crate::thread::{self, ThreadId, WaitQueue};
use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use poly_heap::RawLock;

/// This CPU's count of spinlocks (and the heap's raw lock) held.
fn held() -> &'static AtomicUsize {
    crate::arch::percpu::this().spinlocks()
}

/// How many spinlocks this CPU holds. Their holders have interrupts off.
pub fn spinlocks_held() -> usize {
    held().load(Ordering::Relaxed)
}

/// A busy-waiting lock that disables interrupts on this CPU from before it
/// is taken until after it is released, and restores them to how they were.
#[derive(Debug)]
pub struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: access to `value` is serialised by `locked`.
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        SpinLock { locked: AtomicBool::new(false), value: UnsafeCell::new(value) }
    }

    /// Disable interrupts, then spin until the lock is free. Interrupts stay
    /// off until the guard is dropped.
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let irq = InterruptsOff::new();
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            while self.locked.load(Ordering::Relaxed) {
                crate::arch::wait_hook();
                core::hint::spin_loop();
            }
        }
        SpinLockGuard::new(self, irq)
    }

    /// Take the lock if it is free, without spinning. On failure the
    /// interrupt state is unchanged.
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let irq = InterruptsOff::new();
        let free = self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok();
        free.then(|| SpinLockGuard::new(self, irq))
    }
}

/// Exclusive access to a [`SpinLock`]'s value. Not `Send`: it carries this
/// CPU's interrupt state.
#[derive(Debug)]
pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
    // Dropped after `Drop::drop` released the lock: interrupts come back on
    // only once the lock is free.
    _irq: InterruptsOff,
    _not_send: PhantomData<*const ()>,
}

impl<'a, T> SpinLockGuard<'a, T> {
    fn new(lock: &'a SpinLock<T>, irq: InterruptsOff) -> Self {
        held().fetch_add(1, Ordering::Relaxed);
        SpinLockGuard { lock, _irq: irq, _not_send: PhantomData }
    }
}

impl<T> Deref for SpinLockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the guard proves we hold the lock.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard proves we hold the lock exclusively.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        held().fetch_sub(1, Ordering::Relaxed);
        self.lock.locked.store(false, Ordering::Release);
    }
}

/// [`SpinLock`] without data, for `poly_heap::LockedHeap`: an interrupt
/// handler (or a thread that preempted the holder) may allocate.
#[derive(Debug, Default)]
pub struct RawIrqSpinLock {
    locked: AtomicBool,
    /// Whether interrupts were on before the holder took the lock. Only the
    /// holder touches it.
    restore: AtomicBool,
}

impl RawIrqSpinLock {
    pub const fn new() -> RawIrqSpinLock {
        RawIrqSpinLock { locked: AtomicBool::new(false), restore: AtomicBool::new(false) }
    }
}

// SAFETY: `locked` moves from false to true (Acquire) for exactly one
// caller at a time, and only `unlock` (Release), called by the holder,
// clears it; the interrupt state handling does not affect that.
unsafe impl RawLock for RawIrqSpinLock {
    fn lock(&self) {
        let was_enabled = cpu::interrupts_enabled();
        cpu::disable_interrupts();
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            while self.locked.load(Ordering::Relaxed) {
                crate::arch::wait_hook();
                core::hint::spin_loop();
            }
        }
        held().fetch_add(1, Ordering::Relaxed);
        self.restore.store(was_enabled, Ordering::Relaxed);
    }

    unsafe fn unlock(&self) {
        let enable = self.restore.load(Ordering::Relaxed);
        held().fetch_sub(1, Ordering::Relaxed);
        self.locked.store(false, Ordering::Release);
        if enable {
            // SAFETY: interrupts were on when the caller took the lock, so
            // the code that owns this CPU's interrupt state wants them on.
            unsafe { cpu::enable_interrupts() };
        }
    }
}

/// A lock that sleeps: threads that find it held block on a wait queue and
/// are woken one at a time as it is released. Not FIFO-fair: a running thread
/// can take a just-released mutex before the woken waiter runs (which then
/// waits again).
pub struct Mutex<T> {
    locked: AtomicBool,
    /// [`owner_tag`] of the holder, 0 when free: catches a thread locking a
    /// mutex it already holds.
    owner: AtomicU64,
    waiters: WaitQueue,
    value: UnsafeCell<T>,
}

// SAFETY: access to `value` is serialised by `locked`.
unsafe impl<T: Send> Sync for Mutex<T> {}
// SAFETY: as for any lock owning a `T`.
unsafe impl<T: Send> Send for Mutex<T> {}

impl<T> core::fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mutex").field("locked", &self.locked.load(Ordering::Relaxed)).finish_non_exhaustive()
    }
}

/// A non-zero number identifying thread `id`.
fn owner_tag(id: ThreadId) -> u64 {
    (id.slot() as u64 + 1) << 32 | id.generation() as u64
}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Mutex {
            locked: AtomicBool::new(false),
            owner: AtomicU64::new(0),
            waiters: WaitQueue::new(),
            value: UnsafeCell::new(value),
        }
    }

    fn try_acquire(&self) -> bool {
        self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }

    /// Take the mutex, blocking until it is free. Panics if this thread
    /// already holds it (that would never return).
    pub fn lock(&self) -> MutexGuard<'_, T> {
        assert!(!crate::arch::irq::in_interrupt(), "Mutex::lock in an interrupt handler");
        let me = owner_tag(thread::current());
        if self.owner.load(Ordering::Relaxed) == me {
            panic!("deadlock: thread {} locks a mutex it already holds", thread::current());
        }
        if !self.try_acquire() {
            // The check runs with the scheduler locked, and `unlock` clears
            // `locked` before it wakes a waiter, so a release cannot slip
            // between a failed check and going to sleep.
            self.waiters.wait_until(|| self.try_acquire());
        }
        self.owner.store(me, Ordering::Relaxed);
        MutexGuard { mutex: self, _not_send: PhantomData }
    }

    /// Take the mutex if it is free, without blocking.
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        if !self.try_acquire() {
            return None;
        }
        self.owner.store(owner_tag(thread::current()), Ordering::Relaxed);
        Some(MutexGuard { mutex: self, _not_send: PhantomData })
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    /// The value, without locking: `&mut self` proves nobody holds it.
    /// Usable where locking is not (a drop during reaping).
    pub fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }
}

/// Exclusive access to a [`Mutex`]'s value; unlocks on drop. Not `Send`:
/// the thread that locked it unlocks it.
pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
    _not_send: PhantomData<*const ()>,
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the guard proves we hold the mutex.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard proves we hold the mutex exclusively.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Mutex<T> {
    /// Release the mutex the calling thread holds and wake one waiter.
    fn release(&self) {
        self.owner.store(0, Ordering::Relaxed);
        self.locked.store(false, Ordering::Release);
        self.waiters.wake_one();
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.release();
    }
}

/// A [`Mutex`] without data, for code that locks and unlocks by hand: the
/// sleeping `poly_vfs::lock::RawLock` the file system crates take as their
/// lock type. Same rules as the mutex: not in interrupt handlers, not
/// twice from one thread (that panics), and the thread that locked it
/// unlocks it.
#[derive(Debug)]
pub struct RawMutex(Mutex<()>);

impl RawMutex {
    pub const fn new() -> RawMutex {
        RawMutex(Mutex::new(()))
    }

    pub fn is_locked(&self) -> bool {
        self.0.is_locked()
    }
}

impl Default for RawMutex {
    fn default() -> RawMutex {
        RawMutex::new()
    }
}

// SAFETY: `lock` returns only to the thread whose `try_acquire` moved
// `locked` from false to true (Acquire), and only `unlock` (`release`)
// clears it (Release): one holder at a time, each seeing the last one's
// writes. Forgetting the guard keeps the mutex held until `unlock`.
unsafe impl poly_vfs::lock::RawLock for RawMutex {
    fn lock(&self) {
        core::mem::forget(self.0.lock());
    }

    unsafe fn unlock(&self) {
        self.0.release();
    }
}

/// A value set once (normally during boot, before other threads or CPUs
/// run) and read without locking from then on.
pub struct SetOnce<T> {
    state: AtomicU8,
    value: UnsafeCell<MaybeUninit<T>>,
}

const EMPTY: u8 = 0;
const WRITING: u8 = 1;
const READY: u8 = 2;

// SAFETY: the value is written once, by the caller that moved `state` from
// EMPTY to WRITING, and only read (shared) after READY is published.
unsafe impl<T: Send + Sync> Sync for SetOnce<T> {}

impl<T> SetOnce<T> {
    pub const fn new() -> SetOnce<T> {
        SetOnce { state: AtomicU8::new(EMPTY), value: UnsafeCell::new(MaybeUninit::uninit()) }
    }

    /// Store `value` unless a value was already stored (or is being
    /// stored), in which case it is handed back.
    pub fn set(&self, value: T) -> Result<&T, T> {
        if self.state.compare_exchange(EMPTY, WRITING, Ordering::Acquire, Ordering::Relaxed).is_err() {
            return Err(value);
        }
        // SAFETY: winning the EMPTY -> WRITING exchange makes this the only
        // writer, and readers wait for READY.
        let stored = unsafe { (*self.value.get()).write(value) };
        self.state.store(READY, Ordering::Release);
        Ok(stored)
    }

    pub fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::Acquire) != READY {
            return None;
        }
        // SAFETY: READY is published after the value was written, and it is
        // never written again.
        Some(unsafe { (*self.value.get()).assume_init_ref() })
    }
}

impl<T> Default for SetOnce<T> {
    fn default() -> Self {
        SetOnce::new()
    }
}

impl<T> Drop for SetOnce<T> {
    fn drop(&mut self) {
        if *self.state.get_mut() == READY {
            // SAFETY: READY means the value was initialised, and `&mut self`
            // means nothing borrows it any more.
            unsafe { self.value.get_mut().assume_init_drop() }
        }
    }
}

impl<T: core::fmt::Debug> core::fmt::Debug for SetOnce<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("SetOnce").field(&self.get()).finish()
    }
}

// --- Tests ------------------------------------------------------------------

use crate::arch::irq;
use crate::kernel_test;
use crate::thread::{State, spawn, yield_now};
use crate::time;
use core::sync::atomic::Ordering::SeqCst;

kernel_test! {
    fn spinlock_excludes_until_guard_dropped() -> TestResult {
        let lock = SpinLock::new(0u32);
        let mut guard = lock.lock();
        *guard += 1;
        if lock.try_lock().is_some() {
            return Err("lock taken twice");
        }
        drop(guard);
        let guard = lock.try_lock().ok_or("dropping the guard did not release the lock")?;
        if *guard == 1 { Ok(()) } else { Err("write through the guard was lost") }
    }
}

kernel_test! {
    fn spinlock_masks_interrupts_while_held() -> TestResult {
        if !cpu::interrupts_enabled() {
            return Err("kernel tests should run with interrupts enabled");
        }
        let (a, b) = (SpinLock::new(()), SpinLock::new(()));
        let held = spinlocks_held();
        let outer = a.lock();
        if cpu::interrupts_enabled() || spinlocks_held() != held + 1 {
            return Err("taking a spinlock left interrupts enabled");
        }
        let inner = b.lock();
        if a.try_lock().is_some() || cpu::interrupts_enabled() {
            return Err("a failed try_lock changed the interrupt state");
        }
        drop(inner);
        if cpu::interrupts_enabled() {
            return Err("releasing the inner lock enabled interrupts while the outer one is held");
        }
        // No timer tick gets in while the lock is held, however long that is.
        let ticks = time::ticks();
        let start = time::now_ns();
        while time::now_ns() - start < 5 * time::NANOS_PER_TICK {
            core::hint::spin_loop();
        }
        if time::ticks() != ticks {
            return Err("a timer interrupt arrived while a spinlock was held");
        }
        drop(outer);
        if !cpu::interrupts_enabled() || spinlocks_held() != held {
            return Err("releasing the last lock did not restore interrupts");
        }
        let start = time::now_ns();
        while time::ticks() == ticks {
            if time::now_ns() - start > 1_000_000_000 {
                return Err("the timer interrupt held back by the lock never arrived");
            }
            core::hint::spin_loop();
        }
        // The heap's lock masks interrupts too.
        let heap = crate::heap::HEAP.lock();
        let masked = !cpu::interrupts_enabled();
        drop(heap);
        if masked && cpu::interrupts_enabled() { Ok(()) } else { Err("the heap lock does not mask interrupts") }
    }
}

static SHARED: SpinLock<u32> = SpinLock::new(0);
static HANDLER_RUNS: AtomicUsize = AtomicUsize::new(0);

/// An interrupt handler that takes the lock the interrupted code may hold.
fn bump_shared(_: usize) {
    *SHARED.lock() += 1;
    HANDLER_RUNS.fetch_add(1, SeqCst);
}

kernel_test! {
    fn interrupt_handler_waits_for_a_lock_held_by_the_code_it_interrupts() -> TestResult {
        let vector = irq::allocate_vector(bump_shared, 0).ok_or("no free interrupt vector")?;
        let result = (|| {
            HANDLER_RUNS.store(0, SeqCst);
            let mut guard = SHARED.lock();
            // The interrupt is raised while the lock is held. Were it
            // delivered now, its handler would spin on SHARED forever.
            irq::send_self_ipi(vector);
            for _ in 0..100_000 {
                core::hint::spin_loop();
            }
            if HANDLER_RUNS.load(SeqCst) != 0 {
                return Err("an interrupt was delivered while a spinlock was held");
            }
            *guard = 41;
            drop(guard);
            // Pending since the IPI; delivered as soon as interrupts are on.
            for _ in 0..100_000 {
                if HANDLER_RUNS.load(SeqCst) != 0 {
                    break;
                }
                core::hint::spin_loop();
            }
            if HANDLER_RUNS.load(SeqCst) != 1 || *SHARED.lock() != 42 {
                return Err("the held-back interrupt did not run once the lock was released");
            }
            Ok(())
        })();
        irq::free_vector(vector);
        result
    }
}

static COUNTER: Mutex<u64> = Mutex::new(0);
static RAW: RawMutex = RawMutex::new();
static RAW_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Adds to RAW_COUNTER non-atomically (load, yield, store) under RAW.
fn add_slowly_raw(_: usize) -> usize {
    use poly_vfs::lock::RawLock;
    for _ in 0..ADDS_PER_THREAD {
        RAW.lock();
        let v = RAW_COUNTER.load(Ordering::Relaxed);
        yield_now();
        RAW_COUNTER.store(v + 1, Ordering::Relaxed);
        // SAFETY: this thread took RAW just above.
        unsafe { RAW.unlock() };
    }
    0
}

kernel_test! {
    /// The raw mutex the file system crates use excludes and sleeps like
    /// the mutex itself.
    fn raw_mutex_excludes_and_blocks_until_released() -> TestResult {
        use poly_vfs::lock::RawLock;
        RAW_COUNTER.store(0, Ordering::Relaxed);
        RAW.lock();
        let t = spawn(add_slowly_raw, 0).map_err(|_| "spawn failed")?;
        yield_now();
        let blocked = thread::state(t) == Some(State::Blocked);
        // SAFETY: taken above by this thread.
        unsafe { RAW.unlock() };
        thread::join(t).map_err(|_| "join failed")?;
        if !blocked {
            return Err("a thread waiting for a held raw mutex did not block");
        }
        let mut ids = [None; 3];
        for id in ids.iter_mut() {
            *id = Some(spawn(add_slowly_raw, 0).map_err(|_| "spawn failed")?);
        }
        for id in ids.into_iter().flatten() {
            thread::join(id).map_err(|_| "join failed")?;
        }
        if RAW_COUNTER.load(Ordering::Relaxed) != 4 * ADDS_PER_THREAD {
            return Err("updates under the raw mutex were lost");
        }
        if RAW.is_locked() { Err("raw mutex still locked after every unlock") } else { Ok(()) }
    }
}
const ADDS_PER_THREAD: u64 = 200;

/// Adds to COUNTER non-atomically, yielding in the middle of every update:
/// without mutual exclusion, updates would be lost.
fn add_slowly(_: usize) -> usize {
    for _ in 0..ADDS_PER_THREAD {
        let mut c = COUNTER.lock();
        let v = *c;
        yield_now();
        *c = v + 1;
    }
    0
}

kernel_test! {
    fn mutex_excludes_and_blocks_until_released() -> TestResult {
        *COUNTER.lock() = 0;
        // A thread that finds the mutex held blocks (rather than spins) and
        // runs once it is released.
        let guard = COUNTER.lock();
        if COUNTER.try_lock().is_some() {
            return Err("try_lock took a held mutex");
        }
        let t = spawn(add_slowly, 0).map_err(|_| "spawn failed")?;
        yield_now();
        if thread::state(t) != Some(State::Blocked) {
            drop(guard);
            let _ = thread::join(t);
            return Err("a thread waiting for a held mutex did not block");
        }
        drop(guard);
        thread::join(t).map_err(|_| "join failed")?;
        if *COUNTER.lock() != ADDS_PER_THREAD {
            return Err("the blocked thread did not finish its updates");
        }

        // Four threads that yield (and get preempted) while holding it.
        *COUNTER.lock() = 0;
        let mut ids = [None; 4];
        for id in ids.iter_mut() {
            *id = Some(spawn(add_slowly, 0).map_err(|_| "spawn failed")?);
        }
        for id in ids.into_iter().flatten() {
            thread::join(id).map_err(|_| "join failed")?;
        }
        let total = *COUNTER.lock();
        if total != 4 * ADDS_PER_THREAD {
            crate::kprintln!("sync: {total} of {} mutex-protected updates survived", 4 * ADDS_PER_THREAD);
            return Err("updates under the mutex were lost");
        }
        if COUNTER.is_locked() { Err("mutex still locked after every guard was dropped") } else { Ok(()) }
    }
}

kernel_test! {
    fn set_once_keeps_the_first_value() -> TestResult {
        let cell = SetOnce::new();
        if cell.get().is_some() {
            return Err("an empty SetOnce has a value");
        }
        if cell.set(1u32).is_err() || cell.set(2).is_ok() || cell.get() != Some(&1) {
            return Err("SetOnce did not keep exactly the first value");
        }
        Ok(())
    }
}

static RELOCKED: Mutex<()> = Mutex::new(());

kernel_test! {
    /// Negative test (`poly.selftest.deadlock`): a thread locks a mutex it
    /// already holds, which must panic (with a backtrace of that thread)
    /// rather than hang.
    fn deadlock_on_request(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag("poly.selftest.deadlock") {
            return Ok(());
        }
        let t = spawn(lock_twice, 0).map_err(|_| "spawn failed")?;
        let _ = thread::join(t);
        Err("locking a mutex twice did not panic")
    }
}

/// Kept out of line so the backtrace check has a frame to look for.
#[inline(never)]
fn lock_twice(_: usize) -> usize {
    let _first = RELOCKED.lock();
    let _second = RELOCKED.lock();
    0
}
