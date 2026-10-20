//! Kernel threads: spawn, yield, sleep on wait queues, exit and join.
//!
//! Which thread runs is decided by [`poly_sched::Scheduler`] (round robin,
//! host-tested); this module owns what only the kernel can: thread stacks,
//! saved stack pointers and the context switch ([`arch::context`]).
//!
//! # Threads
//!
//! - After [`init`], the code that was running on the boot stack is thread 0
//!   (`ThreadId::BOOT`) and keeps that stack.
//! - The idle thread runs when no other thread is runnable. Until the kernel
//!   takes interrupts nothing could ever make a thread runnable again at that
//!   point, so it panics with a deadlock message (which gives a backtrace of
//!   the idle thread, not a hang).
//! - Every other thread gets a [`STACK_SIZE`] stack, one buddy block from
//!   [`FRAMES`] reached through [`arch::phys_to_virt`], registered with the
//!   backtrace walker so a panic inside a thread walks its whole stack.
//!   TODO(paging): there is no guard page yet. Once the kernel builds its own
//!   page tables, map stacks in a dedicated area with an unmapped page below
//!   each, so an overflow faults (onto the double-fault IST stack) instead of
//!   silently corrupting the neighbouring frames.
//! - A thread function returns a code, which [`join`] collects. An exited
//!   thread's stack is freed by the next thread to run (it cannot be freed
//!   while still in use); its table slot is freed once it is also joined or
//!   [`detach`]ed.
//!
//! # Locking and interrupts
//!
//! All scheduler state is in [`THREADS`]. Everything that locks it disables
//! interrupts first ([`cpu::InterruptsOff`]), so the timer interrupt can
//! never find the lock held by the thread it interrupted. The lock is
//! released *before* the context switch, which is sound on one CPU because
//! interrupts stay off until the switch is done. SMP will need more: the
//! outgoing thread is already on the run queue while its registers are being
//! saved, so another CPU must not pick it (or reap its stack) until the
//! switch has finished, e.g. by holding the lock across the switch and
//! releasing it in the incoming thread.
//!
//! # Preemption
//!
//! There is no timer interrupt yet. Its handler only has to call
//! [`preempt_tick`]; see there for the rules.

use crate::arch::{self, backtrace, context, cpu::InterruptsOff};
use crate::frames::FRAMES;
use crate::kernel_test;
use crate::sync::SpinLock;
use core::cell::UnsafeCell;
use poly_mem::BuddyAllocator;
pub use poly_sched::{Error, State, ThreadId};
use poly_sched::{Join, Scheduler, Switch};

/// Thread table size, boot and idle threads included.
pub const MAX_THREADS: usize = 64;

/// Buddy order of a kernel stack: 2^2 frames = 16 KiB.
const STACK_ORDER: usize = 2;
pub const STACK_SIZE: u64 = BuddyAllocator::block_size(STACK_ORDER);

/// Timer ticks a thread runs before it is preempted, if another thread is
/// runnable (10 ms at a 1 kHz tick).
pub const TIME_SLICE_TICKS: u32 = 10;

const _: () = assert!(MAX_THREADS <= backtrace::THREAD_STACK_SLOTS);

/// A thread's code: called with the `arg` given to [`spawn`]; the return
/// value is the exit code [`join`] reports.
pub type ThreadFn = fn(usize) -> usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnError {
    /// Every thread slot is in use.
    TableFull,
    /// No free block for the stack.
    NoMemory,
    /// The stack landed in RAM the kernel cannot map yet.
    StackNotMapped,
}

#[derive(Clone, Copy, Debug)]
struct Stack {
    phys: u64,
    virt: u64,
}

impl Stack {
    fn allocate() -> Result<Stack, SpawnError> {
        let mut frames = FRAMES.lock();
        let alloc = frames.as_mut().ok_or(SpawnError::NoMemory)?;
        // Only RAM inside the boot mapping is reachable for now, and plain
        // `allocate` prefers the small leftover blocks at the top of RAM.
        // TODO(paging): once a direct map covers all RAM, plain `allocate`.
        let phys = alloc.allocate_below(STACK_ORDER, arch::BOOT_MAPPED_PHYS).ok_or(SpawnError::NoMemory)?;
        match arch::phys_to_virt(phys + STACK_SIZE - 1) {
            Some(last) => Ok(Stack { phys, virt: last + 1 - STACK_SIZE }),
            None => {
                alloc.free(phys, STACK_ORDER).expect("freeing a block just allocated");
                Err(SpawnError::StackNotMapped)
            }
        }
    }

    fn free(self) {
        let mut frames = FRAMES.lock();
        let alloc = frames.as_mut().expect("stacks come from the frame allocator");
        alloc.free(self.phys, STACK_ORDER).expect("thread stack freed twice");
    }

    fn top(&self) -> u64 {
        self.virt + STACK_SIZE
    }
}

struct Threads {
    sched: Scheduler<MAX_THREADS>,
    /// Stack of each slot's thread; `None` for the boot thread (boot stack)
    /// and free slots.
    stacks: [Option<Stack>; MAX_THREADS],
    ready: bool,
}

static THREADS: SpinLock<Threads> =
    SpinLock::new(Threads { sched: Scheduler::new(TIME_SLICE_TICKS), stacks: [None; MAX_THREADS], ready: false });

/// Saved stack pointer of each slot's thread while it is not running.
struct SavedRsp([UnsafeCell<u64>; MAX_THREADS]);

// SAFETY: a slot is written by `spawn` before its thread is first scheduled
// and by the switch away from its thread, and read by the switch into it.
// The scheduler orders these (a thread is never switched into while it runs
// or before it is spawned), and all of them happen with interrupts disabled
// on the one CPU, so they never overlap.
unsafe impl Sync for SavedRsp {}

static SAVED_RSP: SavedRsp = SavedRsp([const { UnsafeCell::new(0) }; MAX_THREADS]);

/// Make the running code thread 0 and create the idle thread. Call once,
/// after the frame allocator is up.
pub fn init() {
    let _irq = InterruptsOff::new();
    let stack = Stack::allocate().expect("no memory for the idle thread's stack");
    let mut t = THREADS.lock();
    assert!(!t.ready, "threads initialised twice");
    install(&mut t, ThreadId::IDLE, stack, idle_main, 0);
    t.ready = true;
    drop(t);
    crate::kprintln!(
        "thread: scheduler up, {MAX_THREADS} slots, {} KiB stacks, time slice {TIME_SLICE_TICKS} ticks",
        STACK_SIZE >> 10
    );
}

/// Start `f(arg)` in a new thread. It runs once the threads ahead of it on
/// the run queue have had their turn; the caller keeps running.
pub fn spawn(f: ThreadFn, arg: usize) -> Result<ThreadId, SpawnError> {
    let stack = Stack::allocate()?;
    let _irq = InterruptsOff::new();
    let mut t = THREADS.lock();
    assert!(t.ready, "thread::spawn before thread::init");
    reap(&mut t);
    let Ok(id) = t.sched.spawn() else {
        drop(t);
        stack.free();
        return Err(SpawnError::TableFull);
    };
    install(&mut t, id, stack, f, arg);
    Ok(id)
}

/// Give `stack` to thread `id` and set it up to start in `f(arg)`.
fn install(t: &mut Threads, id: ThreadId, stack: Stack, f: ThreadFn, arg: usize) {
    // SAFETY: the stack was just allocated for this thread and is mapped;
    // `thread_start` has the signature `prepare` asks for.
    let rsp = unsafe { context::prepare(stack.top(), thread_start as *const () as usize, f as usize, arg) };
    // SAFETY: the slot's thread has not run yet (see `SavedRsp`).
    unsafe { *SAVED_RSP.0[id.slot()].get() = rsp };
    backtrace::register_thread_stack(id.slot(), stack.virt, STACK_SIZE);
    t.stacks[id.slot()] = Some(stack);
}

/// Where every new thread starts, called by the trampoline in
/// [`arch::context`]. Interrupts are disabled.
#[allow(improper_ctypes_definitions, reason = "only our trampoline calls this, passing on what `install` stored")]
extern "sysv64" fn thread_start(f: ThreadFn, arg: usize) -> ! {
    finish_switch();
    // TODO(timer): enable interrupts here. A new thread starts with them off
    // because every switch happens with interrupts off, and it has no
    // `InterruptsOff` of its own to restore.
    exit(f(arg))
}

/// The idle thread: runs only when no other thread is runnable.
fn idle_main(_: usize) -> usize {
    loop {
        if !yield_now() {
            // TODO(timer): enable interrupts and halt (`sti; hlt`, which takes
            // no interrupt in between) until one makes a thread runnable.
            panic!("deadlock: every thread is blocked and no interrupt can wake one");
        }
    }
}

/// Perform the switch the scheduler decided on. Interrupts must be off and
/// [`THREADS`] unlocked. Returns when `sw.from` is resumed, which for an
/// exited thread is never.
fn switch(sw: Switch) {
    let save = SAVED_RSP.0[sw.from.slot()].get();
    // SAFETY: `sw.to` is not running, so its saved stack pointer is stable
    // (see `SavedRsp`).
    let load = unsafe { *SAVED_RSP.0[sw.to.slot()].get() };
    // SAFETY: interrupts are off (every caller holds `InterruptsOff`); `load`
    // was stored by `install` or by the switch away from `sw.to`, whose stack
    // stays allocated until it exits and is reaped; `save` is a static slot.
    unsafe { context::switch(save, load) };
    finish_switch();
}

/// Runs on the thread just switched in, first thing.
fn finish_switch() {
    reap(&mut THREADS.lock());
}

/// Free the stacks of exited threads. Never sees the current thread's own
/// stack: a thread is only reaped after the switch away from it.
fn reap(t: &mut Threads) {
    while let Some(id) = t.sched.unreaped() {
        if let Some(stack) = t.stacks[id.slot()] {
            // Lock order is THREADS before FRAMES only with try_lock: if the
            // frame allocator is busy (its holder was switched away), the
            // stack is freed on a later switch or spawn instead.
            let Some(mut frames) = FRAMES.try_lock() else { return };
            backtrace::unregister_thread_stack(id.slot());
            let alloc = frames.as_mut().expect("stacks come from the frame allocator");
            alloc.free(stack.phys, STACK_ORDER).expect("thread stack freed twice");
            t.stacks[id.slot()] = None;
        }
        t.sched.reap(id).expect("unreaped() names an exited thread");
    }
}

/// The running thread.
pub fn current() -> ThreadId {
    let _irq = InterruptsOff::new();
    THREADS.lock().sched.current()
}

/// What thread `id` is doing, or `None` if it no longer exists.
pub fn state(id: ThreadId) -> Option<State> {
    let _irq = InterruptsOff::new();
    THREADS.lock().sched.state(id)
}

/// Let the next runnable thread run; this thread goes to the back of the
/// run queue. False if there was nobody to switch to.
pub fn yield_now() -> bool {
    let _irq = InterruptsOff::new();
    let sw = THREADS.lock().sched.yield_now();
    sw.map(switch).is_some()
}

/// End the running thread with exit code `code`.
pub fn exit(code: usize) -> ! {
    let _irq = InterruptsOff::new();
    let sw = THREADS.lock().sched.exit_current(code).expect("the idle thread cannot exit");
    switch(sw);
    unreachable!("exited thread {} was resumed", sw.from)
}

/// Wait for thread `id` to exit and return its exit code. Each thread can be
/// joined once.
pub fn join(id: ThreadId) -> Result<usize, Error> {
    let _irq = InterruptsOff::new();
    loop {
        let r = THREADS.lock().sched.join(id);
        match r? {
            Join::Exited(code) => return Ok(code),
            Join::Blocked(sw) => switch(sw),
        }
    }
}

/// Nobody will join `id`: release it as soon as it exits.
pub fn detach(id: ThreadId) -> Result<(), Error> {
    let _irq = InterruptsOff::new();
    THREADS.lock().sched.detach(id)
}

/// Timer interrupt entry point: account one tick to the running thread and,
/// if its time slice is used up and another thread is runnable, switch to
/// that thread. Returns whether it switched (once this thread runs again).
///
/// The timer handler must call it:
/// - with interrupts disabled, as they are inside an interrupt gate;
/// - after acknowledging the interrupt (EOI): the thread switched to may run
///   for a long time before this returns, and a new thread never returns
///   through this handler at all;
/// - on the interrupted thread's own kernel stack, not an IST stack. The
///   handler's frames and the trap frame stay on that stack while the thread
///   is switched out; when it is resumed this returns and the handler
///   returns with `iretq` as usual. An IST stack would be overwritten by the
///   next interrupt in the meantime.
///
/// The interrupted thread may have been anywhere except inside a
/// `THREADS` critical section (those run with interrupts off). Code that
/// must not be preempted (holding a spinlock) must keep interrupts off,
/// which the interrupt-safe spinlocks of the next milestone will do.
pub fn preempt_tick() -> bool {
    let _irq = InterruptsOff::new();
    let sw = {
        let mut t = THREADS.lock();
        if t.ready && t.sched.tick() { t.sched.yield_now() } else { None }
    };
    sw.map(switch).is_some()
}

/// Threads blocked until some condition holds, woken by whoever changes it.
pub struct WaitQueue(UnsafeCell<poly_sched::WaitQueue>);

// SAFETY: the inner queue is only accessed with THREADS locked.
unsafe impl Sync for WaitQueue {}

impl core::fmt::Debug for WaitQueue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("WaitQueue")
    }
}

impl WaitQueue {
    pub const fn new() -> WaitQueue {
        WaitQueue(UnsafeCell::new(poly_sched::WaitQueue::new()))
    }

    /// Block until `ready()` returns true. `ready` runs with the scheduler
    /// locked and interrupts off, so a waker that makes it true and then
    /// calls [`wake_one`](Self::wake_one) or [`wake_all`](Self::wake_all)
    /// cannot slip in between the check and going to sleep: no lost wakeups.
    /// It must be short and must not call into this module.
    pub fn wait_until(&self, mut ready: impl FnMut() -> bool) {
        let _irq = InterruptsOff::new();
        loop {
            let sw = {
                let mut t = THREADS.lock();
                if ready() {
                    return;
                }
                // SAFETY: THREADS is locked.
                let q = unsafe { &mut *self.0.get() };
                t.sched.block_current(q).expect("the idle thread cannot sleep")
            };
            switch(sw);
        }
    }

    /// Wake the longest-waiting thread. False if none was waiting.
    pub fn wake_one(&self) -> bool {
        let _irq = InterruptsOff::new();
        let mut t = THREADS.lock();
        // SAFETY: THREADS is locked.
        t.sched.wake_one(unsafe { &mut *self.0.get() }).is_some()
    }

    /// Wake every waiting thread; returns how many there were.
    pub fn wake_all(&self) -> usize {
        let _irq = InterruptsOff::new();
        let mut t = THREADS.lock();
        // SAFETY: THREADS is locked.
        t.sched.wake_all(unsafe { &mut *self.0.get() })
    }
}

// --- Tests ------------------------------------------------------------------

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};

/// Characters recorded by threads, in the order they ran.
struct Trace {
    buf: [u8; 32],
    len: usize,
}

static TRACE: SpinLock<Trace> = SpinLock::new(Trace { buf: [0; 32], len: 0 });

fn record(c: u8) {
    let mut t = TRACE.lock();
    let len = t.len;
    if len < t.buf.len() {
        t.buf[len] = c;
        t.len += 1;
    }
}

fn ping(tag: usize) -> usize {
    for _ in 0..3 {
        record(tag as u8);
        yield_now();
    }
    tag + 1
}

kernel_test! {
    fn yield_interleaves_round_robin() -> TestResult {
        TRACE.lock().len = 0;
        if current() != ThreadId::BOOT {
            return Err("tests do not run on the boot thread");
        }
        let a = spawn(ping, b'a' as usize).map_err(|_| "spawn failed")?;
        let b = spawn(ping, b'b' as usize).map_err(|_| "spawn failed")?;
        if state(a) != Some(State::Runnable) {
            return Err("a new thread is not runnable");
        }
        for _ in 0..3 {
            record(b'M');
            if !yield_now() {
                return Err("yield found nobody to run");
            }
        }
        if join(a) != Ok(b'a' as usize + 1) || join(b) != Ok(b'b' as usize + 1) {
            return Err("wrong exit code");
        }
        let t = TRACE.lock();
        if &t.buf[..t.len] != b"MabMabMab" {
            return Err("threads did not take turns in round-robin order");
        }
        Ok(())
    }
}

static WAITERS: WaitQueue = WaitQueue::new();
static TOKENS: AtomicUsize = AtomicUsize::new(0);
static WOKEN: AtomicUsize = AtomicUsize::new(0);

fn take_token() -> bool {
    TOKENS.fetch_update(SeqCst, SeqCst, |n| n.checked_sub(1)).is_ok()
}

fn waiter(_: usize) -> usize {
    WAITERS.wait_until(take_token);
    WOKEN.fetch_add(1, SeqCst);
    0
}

kernel_test! {
    fn wait_queue_blocks_and_wakes() -> TestResult {
        TOKENS.store(0, SeqCst);
        WOKEN.store(0, SeqCst);
        if WAITERS.wake_one() || WAITERS.wake_all() != 0 {
            return Err("woke a thread on an empty queue");
        }
        let w1 = spawn(waiter, 0).map_err(|_| "spawn failed")?;
        let w2 = spawn(waiter, 0).map_err(|_| "spawn failed")?;
        let both_blocked = || state(w1) == Some(State::Blocked) && state(w2) == Some(State::Blocked);
        yield_now();
        if !both_blocked() || WOKEN.load(SeqCst) != 0 {
            return Err("waiters did not block");
        }
        // Woken while the condition is still false: they go back to sleep.
        if WAITERS.wake_all() != 2 {
            return Err("wake_all did not find both waiters");
        }
        yield_now();
        if !both_blocked() || WOKEN.load(SeqCst) != 0 {
            return Err("waiters did not re-check their condition");
        }
        TOKENS.store(1, SeqCst);
        if !WAITERS.wake_one() {
            return Err("wake_one found nobody");
        }
        yield_now();
        if WOKEN.load(SeqCst) != 1 || state(w1) != Some(State::Exited) || state(w2) != Some(State::Blocked) {
            return Err("wake_one did not wake exactly the first waiter");
        }
        TOKENS.store(1, SeqCst);
        if WAITERS.wake_all() != 1 {
            return Err("wake_all did not find the remaining waiter");
        }
        if join(w1) != Ok(0) || join(w2) != Ok(0) || WOKEN.load(SeqCst) != 2 {
            return Err("waiters did not finish");
        }
        Ok(())
    }
}

fn triple(arg: usize) -> usize {
    arg * 3
}

fn free_frames() -> u64 {
    FRAMES.lock().as_ref().map_or(0, |f| f.free_frames())
}

fn live_threads() -> usize {
    let _irq = InterruptsOff::new();
    THREADS.lock().sched.threads()
}

kernel_test! {
    fn exit_frees_slots_and_stacks() -> TestResult {
        let (frames_before, threads_before) = (free_frames(), live_threads());
        let room = MAX_THREADS - threads_before;
        // Fill the table three times over, joining every thread each time.
        for _ in 0..3 {
            let mut ids = [None; MAX_THREADS];
            let mut n = 0;
            loop {
                match spawn(triple, n) {
                    Ok(id) => ids[n] = Some(id),
                    Err(SpawnError::TableFull) => break,
                    Err(_) => return Err("out of memory for thread stacks"),
                }
                n += 1;
            }
            if n != room {
                return Err("thread table did not fill up as expected");
            }
            if free_frames() != frames_before - ((n as u64) << STACK_ORDER) {
                return Err("each thread should hold exactly one stack");
            }
            for (i, id) in ids.iter().take(n).enumerate() {
                if join(id.unwrap()) != Ok(i * 3) {
                    return Err("wrong exit code from join");
                }
            }
            if join(ids[0].unwrap()) != Err(Error::NoSuchThread) {
                return Err("a thread was joined twice");
            }
        }
        // Detached threads release everything without a join.
        for i in 0..room {
            let id = spawn(triple, i).map_err(|_| "spawn failed after the table was emptied")?;
            detach(id).map_err(|_| "detach failed")?;
        }
        while yield_now() {}
        if join(current()) != Err(Error::Current) {
            return Err("a thread joined itself");
        }
        if live_threads() != threads_before {
            return Err("exited threads still hold table slots");
        }
        if free_frames() != frames_before {
            return Err("exited threads still hold their stacks");
        }
        Ok(())
    }
}

static SPINS: AtomicUsize = AtomicUsize::new(0);
static STOP: AtomicBool = AtomicBool::new(false);

/// Stands in for the timer: calls `preempt_tick` until told to stop.
fn spinner(_: usize) -> usize {
    while !STOP.load(SeqCst) {
        SPINS.fetch_add(1, SeqCst);
        preempt_tick();
    }
    0
}

kernel_test! {
    fn preempt_tick_switches_after_a_time_slice() -> TestResult {
        for _ in 0..2 * TIME_SLICE_TICKS {
            if preempt_tick() {
                return Err("preempted with nobody else to run");
            }
        }
        SPINS.store(0, SeqCst);
        STOP.store(false, SeqCst);
        let t = spawn(spinner, 0).map_err(|_| "spawn failed")?;
        yield_now();
        // The spinner got a full slice, then the tick switched back here.
        if SPINS.load(SeqCst) != TIME_SLICE_TICKS as usize {
            return Err("not preempted after exactly one time slice");
        }
        STOP.store(true, SeqCst);
        join(t).map(|_| ()).map_err(|_| "spinner did not finish")
    }
}

/// Exit codes of `walk_own_stack`.
const WALK_OK: usize = 0;
const WALK_NOT_BOTTOM: usize = 1;
const WALK_WRONG_END: usize = 2;

fn walk_own_stack(_: usize) -> usize {
    let mut last = 0;
    if backtrace::walk_from_here(|_, addr| last = addr) != poly_backtrace::End::Bottom {
        return WALK_NOT_BOTTOM;
    }
    if last != context::trampoline_return() { WALK_WRONG_END } else { WALK_OK }
}

kernel_test! {
    fn backtrace_walks_a_thread_stack_to_its_start() -> TestResult {
        let t = spawn(walk_own_stack, 0).map_err(|_| "spawn failed")?;
        match join(t) {
            Ok(WALK_OK) => Ok(()),
            Ok(WALK_NOT_BOTTOM) => Err("walk stopped before the bottom of the thread stack"),
            Ok(_) => Err("walk did not end at the thread trampoline"),
            Err(_) => Err("join failed"),
        }
    }
}

kernel_test! {
    /// Negative test (`poly.selftest.thread_panic`): a spawned thread panics,
    /// so the runner can check the backtrace walks the thread's own stack.
    fn panics_in_thread_on_request(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag("poly.selftest.thread_panic") {
            return Ok(());
        }
        let t = spawn(panic_in_thread, 0).map_err(|_| "spawn failed")?;
        let _ = join(t);
        Err("the thread did not panic")
    }
}

/// Kept out of line so the backtrace check has a frame to look for.
#[inline(never)]
fn panic_in_thread(_: usize) -> usize {
    panic!("panic requested by poly.selftest.thread_panic");
}

static NEVER: WaitQueue = WaitQueue::new();

kernel_test! {
    /// Negative test (`poly.selftest.deadlock`): the only thread blocks for
    /// good, so the idle thread must report a deadlock instead of hanging.
    fn deadlock_on_request(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag("poly.selftest.deadlock") {
            return Ok(());
        }
        NEVER.wait_until(|| false);
        Err("woke up from a wait nobody ends")
    }
}
