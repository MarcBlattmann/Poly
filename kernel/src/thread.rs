//! Kernel threads: spawn, yield, sleep on wait queues or for a time, exit
//! and join; preempted round robin by each CPU's timer tick, on every CPU.
//!
//! Which thread runs where is decided by [`poly_sched::Scheduler`] (per-CPU
//! run queues, placement, stealing, affinity; host-tested); this module owns
//! what only the kernel can: thread stacks, saved stack pointers, the
//! context switch ([`arch::context`]) and the reschedule IPIs.
//!
//! # Threads
//!
//! - After [`init`], the code that was running on the boot stack is thread 0
//!   (`ThreadId::BOOT`) and keeps that stack.
//! - Each CPU has an idle thread that runs when the CPU has nothing else to
//!   run. It halts the CPU (`sti; hlt`) until an interrupt, and gives way
//!   once something made a thread runnable there. CPU 0's is created by
//!   [`init`]; an application processor's by [`prepare_cpu`], and the AP's
//!   start-up code runs on its stack and becomes it ([`run_idle`]).
//! - New threads start with interrupts enabled (once the interrupt
//!   controllers are set up; a thread that starts before that keeps them
//!   off, and is not preempted). They may run on any CPU unless given an
//!   affinity ([`spawn_on`], [`set_affinity`], [`pin_current`]).
//! - Every other thread gets a [`STACK_SIZE`] stack, one buddy block from
//!   [`FRAMES`], mapped in its table slot's part of [`STACKS`] (not reached
//!   through the direct map) above an unmapped guard (ADR 0050), and
//!   registered with the backtrace walker so a panic inside a thread walks
//!   its whole stack. An overflow faults on the guard; the page fault cannot
//!   be delivered on the full stack, so it becomes a double fault on its own
//!   IST stack, which reports "kernel stack overflow in thread <name>"
//!   ([`stack_guard_hit`]) instead of the thread silently overwriting
//!   whatever lies below.
//! - A thread function returns a code, which [`join`] collects. An exited
//!   thread's stack is freed by a thread that runs after it was switched
//!   away from (it cannot be freed while still in use); its table slot is
//!   freed once it is also joined or [`detach`]ed.
//! - A *user thread* ([`spawn_user`], used by [`crate::user`]) is a thread
//!   like any other that also carries a [`UserThread`]: its process, address
//!   space, FPU state and FS/GS bases. Every switch hands the outgoing and
//!   incoming threads' user state to `arch::user::switch`, which saves the
//!   outgoing one's FPU state and bases and loads the incoming thread's
//!   address space (the kernel PML4 for kernel threads), FPU state and bases
//!   and points the ring-3 entries (TSS.rsp0, the `syscall` stack) of this
//!   CPU at its kernel stack. The user state of an exited thread is dropped
//!   by the reaper, after [`THREADS`] is unlocked: the last reference to an
//!   address space frees its memory.
//!
//! # Locking, interrupts and the switch
//!
//! All scheduler state is in [`THREADS`], one spinlock for all CPUs, which
//! keeps interrupts disabled while held, so a timer interrupt can never find
//! it held by the thread it interrupted. Every switch runs inside an
//! [`InterruptsOff`] section on its CPU, so the thread cannot move while it
//! switches, and the lock is released *before* the context switch. Until
//! the outgoing thread's registers are saved, another CPU must not resume
//! it (or reap its stack): the scheduler marks it as being switched away
//! from, and the CPU clears that ([`poly_sched::Scheduler::saved`]) first
//! thing on the incoming thread ([`finish_switch`]), with the lock, which
//! also publishes the saved stack pointer to whichever CPU resumes it. A
//! thread never switches while it holds a spinlock (they mask interrupts,
//! so the tick cannot preempt it, and [`switch`] checks that nothing blocks
//! with one held) or from inside an interrupt handler.
//!
//! When a thread becomes runnable on an idle CPU's queue (a spawn, a wake,
//! a migration), the scheduler records that CPU and the kernel sends it a
//! reschedule IPI after unlocking ([`kick`]), which ends its `hlt`.
//!
//! # Preemption and sleeping
//!
//! Each CPU's timer interrupt calls [`on_timer_tick`] on every tick
//! (`TICK_HZ`), on the CPU's interrupt stack: it wakes the threads whose
//! [`sleep`] has ended and the wait queues of timed waits
//! ([`WaitQueue::wait_until_deadline`]) whose deadline has passed (one CPU
//! does that per due deadline). The interrupt's way out, back on the
//! thread's stack, then calls [`preempt_tick`], which switches threads once
//! that CPU's running one has used up its time slice and the CPU has
//! another to run, its own or one it takes from a busier CPU (ADR 0061).
//!
//! # Stack depth
//!
//! Stacks are painted when allocated; the reaper records how deep each was
//! used ([`deepest_stack`], `poly.debug.stack_report`), and
//! `poly.debug.thread_stack_kib=<n>` maps only the top `n` KiB of each, so
//! deep paths overflow (and are reported with a backtrace) sooner. An
//! interrupt takes at most [`INTERRUPT_STACK_BUDGET`] of the interrupted
//! thread's stack.

use crate::arch::{self, backtrace, context, cpu, cpu::InterruptsOff, percpu};
use crate::frames::FRAMES;
use crate::kernel_test;
use crate::mm;
use crate::sync::{self, SpinLock};
use crate::time;
use crate::user::UserThread;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::AtomicU64;
use core::time::Duration;
use poly_mem::{BuddyAllocator, StackArea};
pub use poly_sched::{CpuSet, Error, State, ThreadId};
use poly_sched::{Join, Scheduler, Switch};

/// Thread table size, boot and idle threads included.
pub const MAX_THREADS: usize = 64;

/// Most CPUs the scheduler runs.
pub const MAX_CPUS: usize = arch::MAX_CPUS;

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
    /// The affinity names no CPU the scheduler has.
    NoSuchCpu,
}

/// Bytes of virtual address space per thread stack: the stack at the top,
/// the rest an unmapped guard (as large as the stack, so a frame that
/// skips a page or two still lands in it).
const STACK_SLOT_SIZE: u64 = 2 * STACK_SIZE;

/// Where thread stacks are mapped: slot `i` holds the stack of the thread
/// in table slot `i`, above its guard (ADR 0050).
pub const STACKS: StackArea = StackArea::new(mm::THREAD_STACKS_BASE, STACK_SLOT_SIZE, STACK_SIZE, MAX_THREADS);

const _: () = assert!(STACKS.size() <= mm::THREAD_STACKS_SIZE, "the stacks do not fit the stack area");

/// A thread stack's frames: one buddy block, not mapped (yet, or any more).
#[derive(Debug)]
struct StackFrames {
    phys: u64,
}

impl StackFrames {
    fn allocate() -> Result<StackFrames, SpawnError> {
        let phys = {
            let mut frames = FRAMES.lock();
            let alloc = frames.as_mut().ok_or(SpawnError::NoMemory)?;
            alloc.allocate(STACK_ORDER).ok_or(SpawnError::NoMemory)?
        };
        // Painted through the direct map before anything runs on it, so
        // the reaper can tell how deep the thread ever went (`Stack::used`).
        let virt = arch::phys_to_virt(phys).expect("thread stacks come from RAM in the direct map");
        // SAFETY: the block was just allocated for this stack and nothing
        // else refers to it; the direct map maps it read-write.
        unsafe { mm::paint_stack(virt, STACK_SIZE) };
        Ok(StackFrames { phys })
    }

    fn free(self) {
        let mut frames = FRAMES.lock();
        let alloc = frames.as_mut().expect("stacks come from the frame allocator");
        alloc.free(self.phys, STACK_ORDER).expect("thread stack freed twice");
    }

    /// Map the frames as the stack of table slot `slot`, whose part of
    /// [`STACKS`] is unmapped (its last thread was reaped). Holding
    /// [`THREADS`] makes the caller the slot's only user.
    fn map(self, _threads: &mut Threads, slot: usize) -> Stack {
        let virt = STACKS.stack(slot).expect("thread slot outside the stack area").start;
        // Only the top `usable` bytes with `poly.debug.thread_stack_kib`;
        // the frames below stay unmapped, an extra guard.
        let usable = stack_limit();
        let skip = STACK_SIZE - usable;
        // SAFETY: a slot's stack pages are mapped when its thread is
        // installed and unmapped when that thread is reaped, both with
        // THREADS locked, which the caller holds; so they are unmapped now
        // and nobody else touches them. The frames are ours until `unmap`.
        unsafe { mm::map_thread_stack(virt + skip, self.phys + skip, usable) };
        backtrace::register_thread_stack(slot, virt + skip, usable);
        Stack { phys: self.phys, virt, usable }
    }
}

/// A thread's stack, mapped in [`STACKS`].
#[derive(Clone, Copy, Debug)]
struct Stack {
    phys: u64,
    virt: u64,
    /// Bytes mapped, at the top: [`STACK_SIZE`] unless shrunk for
    /// debugging ([`stack_limit`]).
    usable: u64,
}

impl Stack {
    fn top(&self) -> u64 {
        self.virt + STACK_SIZE
    }

    /// The bytes of this stack that were ever in use, read off the paint
    /// (it must be mapped).
    fn used(&self) -> u64 {
        // SAFETY: the stack's top `usable` bytes are mapped while its
        // thread exists, and only read here.
        unsafe { mm::stack_used(self.top() - self.usable, self.usable) }
    }

    /// Unmap the stack of a reaped thread; its frames can be freed then.
    fn unmap(self, _threads: &mut Threads) -> StackFrames {
        // SAFETY: the caller holds THREADS, so owns the slot's pages (see
        // `map`), and the thread is reaped: it was switched away from for
        // good, and nothing points into its stack any more.
        unsafe { mm::unmap_thread_stack(self.top() - self.usable, self.usable) };
        StackFrames { phys: self.phys }
    }
}

/// Bytes of each thread stack that are mapped: [`STACK_SIZE`], or less with
/// `poly.debug.thread_stack_kib=<n>`, which leaves the rest of the stack
/// unmapped so that paths running deep show up as overflows sooner.
static STACK_LIMIT: AtomicU64 = AtomicU64::new(STACK_SIZE);

fn stack_limit() -> u64 {
    STACK_LIMIT.load(SeqCst)
}

/// Apply `poly.debug.thread_stack_kib=<n>` (whole pages, 4 KiB up to
/// [`STACK_SIZE`]) to the threads made from now on. Call at boot, before
/// spawning anything.
pub fn configure(boot: &crate::boot_info::BootInfo) {
    REPORT.store(boot.has_flag("poly.debug.stack_report"), SeqCst);
    let Some(kib) = boot.arg("poly.debug.thread_stack_kib").and_then(|v| v.parse::<u64>().ok()) else { return };
    let bytes = (kib << 10).clamp(4096, STACK_SIZE) & !4095;
    STACK_LIMIT.store(bytes, SeqCst);
    crate::kprintln!("thread: debugging: new threads get only the top {} KiB of their stacks", bytes >> 10);
}

/// Set by `poly.debug.stack_report`: [`report_stacks`] prints.
static REPORT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// With `poly.debug.stack_report`, print how deep the thread stacks and
/// the interrupt stacks were used (a test run calls this as it ends, also
/// after a panic, so it does not wait for the thread table).
pub fn report_stacks() {
    if !REPORT.load(SeqCst) {
        return;
    }
    match deepest_stack() {
        Some(d) => crate::kprintln!(
            "stacks: deepest thread stack use {} of {} bytes (thread '{}', started in {:#x})",
            d.bytes,
            stack_limit(),
            d.name,
            d.entry
        ),
        None => crate::kprintln!("stacks: the thread table is locked"),
    }
    for cpu in 0..arch::percpu::count() {
        if let Some(bytes) = arch::irqstack::used(cpu) {
            crate::kprintln!(
                "stacks: CPU {cpu}'s interrupt stack: deepest use {bytes} of {} bytes",
                arch::irqstack::SIZE
            );
        }
    }
    crate::kprintln!("stacks: interrupts nested on an interrupt stack: {}", arch::irqstack::nested());
}

/// The deepest use of a thread stack, and whose it was.
#[derive(Clone, Copy, Debug)]
pub struct Deepest {
    pub bytes: u64,
    pub name: &'static str,
    /// The thread's function (`cargo xtask symbolize` names it; 0 for the
    /// idle threads).
    pub entry: usize,
}

impl Deepest {
    const NONE: Deepest = Deepest { bytes: 0, name: "", entry: 0 };

    /// Keep slot `slot`'s thread if its stack went `used` bytes deep, more
    /// than the record.
    fn note(&mut self, t: &Threads, slot: usize, used: u64) {
        // The test's spinner goes deep on purpose; it is not what the record
        // is to show.
        if used > self.bytes && t.entries[slot] != deep_spinner as usize {
            *self = Deepest { bytes: used, name: t.names[slot], entry: t.entries[slot] };
        }
    }
}

/// The deepest any thread stack was ever used so far (the threads
/// running now included); `None` if the thread table is locked.
pub fn deepest_stack() -> Option<Deepest> {
    let _irq = InterruptsOff::new();
    // Other CPUs take the lock all the time, but one stopped by a panic
    // may hold it for good: try for a while only.
    let t = (0..1_000_000).find_map(|_| {
        let t = THREADS.try_lock();
        if t.is_none() {
            core::hint::spin_loop();
        }
        t
    })?;
    let mut deepest = t.deepest;
    for (slot, stack) in t.stacks.iter().enumerate() {
        if let Some(stack) = stack {
            deepest.note(&t, slot, stack.used());
        }
    }
    Some(deepest)
}

struct Threads {
    sched: Scheduler<MAX_THREADS, MAX_CPUS>,
    /// Stack of each slot's thread; `None` for the boot thread (boot stack)
    /// and free slots.
    stacks: [Option<Stack>; MAX_THREADS],
    /// Each slot's thread in [`WaitQueue::wait_until_deadline`]: its
    /// deadline and the queue it waits on.
    timeouts: [Option<Timeout>; MAX_THREADS],
    /// The user state of each slot's thread; `None` for kernel threads.
    user: [Option<Box<UserThread>>; MAX_THREADS],
    /// Each slot's thread's name, for `ps` (empty: none).
    names: [&'static str; MAX_THREADS],
    /// Each slot's thread function, for [`Deepest`].
    entries: [usize; MAX_THREADS],
    /// The deepest a reaped thread's stack was used.
    deepest: Deepest,
    ready: bool,
}

/// A thread's timed wait, for the tick.
#[derive(Clone, Copy, Debug)]
struct Timeout {
    deadline: u64,
    /// The address of the [`WaitQueue`]. It stays valid while the entry
    /// exists: the waiting thread is inside `wait_until_deadline` (which
    /// borrows the queue) and removes the entry before it returns, with
    /// [`THREADS`] locked, as is every read of it.
    queue: usize,
}

static THREADS: SpinLock<Threads> = SpinLock::new(Threads {
    sched: Scheduler::new(TIME_SLICE_TICKS),
    stacks: [None; MAX_THREADS],
    timeouts: [None; MAX_THREADS],
    user: [const { None }; MAX_THREADS],
    names: {
        let mut names = [""; MAX_THREADS];
        names[ThreadId::BOOT.slot()] = "boot";
        names[ThreadId::IDLE.slot()] = "idle";
        names
    },
    entries: [0; MAX_THREADS],
    deepest: Deepest::NONE,
    ready: false,
});

/// Names of the application processors' idle threads.
const IDLE_NAMES: [&str; MAX_CPUS] = [
    "idle", "idle1", "idle2", "idle3", "idle4", "idle5", "idle6", "idle7", "idle8", "idle9", "idle10", "idle11",
    "idle12", "idle13", "idle14", "idle15",
];

/// Saved stack pointer of each slot's thread while it is not running.
struct SavedRsp([UnsafeCell<u64>; MAX_THREADS]);

// SAFETY: a slot is written by `spawn` (with THREADS locked) before its
// thread is first scheduled, and by the switch away from its thread on the
// CPU that ran it; it is read by the switch into it, on whichever CPU picks
// it. The scheduler never picks a thread before the switch away from it is
// complete (`Scheduler::saved`, called with THREADS locked after the write
// on the switching CPU), and the picking CPU takes THREADS after that, so
// the write happens before the read and they never overlap.
unsafe impl Sync for SavedRsp {}

static SAVED_RSP: SavedRsp = SavedRsp([const { UnsafeCell::new(0) }; MAX_THREADS]);

/// The CPU this code runs on. Only stable while interrupts are off.
pub fn this_cpu() -> usize {
    percpu::cpu_index()
}

/// Send a reschedule IPI to each CPU in `cpus` but this one. Call after
/// unlocking [`THREADS`].
fn kick(cpus: CpuSet) {
    if cpus.is_empty() {
        return;
    }
    let me = this_cpu();
    for c in cpus.iter().filter(|&c| c != me) {
        arch::smp::send_reschedule(c);
    }
}

/// Make the running code thread 0 and create CPU 0's idle thread. Call
/// once, after the frame allocator is up.
pub fn init() {
    let _irq = InterruptsOff::new();
    let stack = StackFrames::allocate().expect("no memory for the idle thread's stack");
    let mut t = THREADS.lock();
    assert!(!t.ready, "threads initialised twice");
    install(&mut t, ThreadId::IDLE, stack, idle_main, 0, None);
    t.ready = true;
    drop(t);
    percpu::this().set_current(ThreadId::BOOT.to_bits());
    crate::kprintln!(
        "thread: scheduler up, {MAX_THREADS} slots, {} KiB stacks above {} KiB guards from {:#x}, \
         time slice {TIME_SLICE_TICKS} ticks",
        STACK_SIZE >> 10,
        STACKS.guard_size() >> 10,
        mm::THREAD_STACKS_BASE
    );
}

/// Make room for CPU `cpu` (an application processor about to start): its
/// idle thread and that thread's stack, which the CPU's start-up code runs
/// on. Returns the idle thread and the stack's top.
pub fn prepare_cpu(cpu: usize) -> Result<(ThreadId, u64), SpawnError> {
    let stack = StackFrames::allocate()?;
    let _irq = InterruptsOff::new();
    let mut t = THREADS.lock();
    let id = match t.sched.add_cpu(cpu) {
        Ok(id) => id,
        Err(_) => {
            drop(t);
            stack.free();
            return Err(SpawnError::TableFull);
        }
    };
    let stack = stack.map(&mut t, id.slot());
    t.stacks[id.slot()] = Some(stack);
    t.names[id.slot()] = IDLE_NAMES.get(cpu).copied().unwrap_or("idle");
    t.entries[id.slot()] = 0;
    crate::cputime::started(id.slot(), None);
    crate::cputime::running(id.slot());
    Ok((id, stack.top()))
}

/// The running application processor `cpu` is ready to run threads: it
/// runs its idle thread (the code calling this) and takes threads from now
/// on. Interrupts are off.
pub fn cpu_online(cpu: usize) {
    let mut t = THREADS.lock();
    let idle = t.sched.idle_thread(cpu).expect("prepare_cpu ran for this CPU");
    percpu::this().set_current(idle.to_bits());
    t.sched.set_online(cpu, true).expect("the CPU was added");
}

/// The rest of an application processor's life: its idle loop.
pub fn run_idle() -> ! {
    // SAFETY: the AP holds no lock and no `InterruptsOff`; its interrupt
    // setup is complete.
    unsafe { cpu::enable_interrupts() };
    idle_main(0);
    unreachable!("the idle loop returned")
}

/// Start `f(arg)` in a new thread. It runs once the threads ahead of it on
/// the run queue of the CPU it lands on have had their turn (an idle CPU
/// starts it at once); the caller keeps running.
pub fn spawn(f: ThreadFn, arg: usize) -> Result<ThreadId, SpawnError> {
    spawn_named("", f, arg)
}

/// [`spawn`] with a name, which [`list`] reports.
pub fn spawn_named(name: &'static str, f: ThreadFn, arg: usize) -> Result<ThreadId, SpawnError> {
    spawn_with(name, f, arg, None, CpuSet::ALL)
}

/// [`spawn_named`] for a thread that may only run on `cpus`.
pub fn spawn_on(name: &'static str, f: ThreadFn, arg: usize, cpus: CpuSet) -> Result<ThreadId, SpawnError> {
    spawn_with(name, f, arg, None, cpus)
}

/// Start `f(arg)` in a new user thread named `name`, which carries `user`
/// from the start: its address space is loaded whenever it runs, on the
/// CPUs in `cpus`. `f` is expected to enter ring 3 (see
/// [`crate::user::spawn`]).
pub fn spawn_user(
    name: &'static str,
    user: Box<UserThread>,
    f: ThreadFn,
    arg: usize,
    cpus: CpuSet,
) -> Result<ThreadId, SpawnError> {
    spawn_with(name, f, arg, Some(user), cpus)
}

fn spawn_with(
    name: &'static str,
    f: ThreadFn,
    arg: usize,
    user: Option<Box<UserThread>>,
    cpus: CpuSet,
) -> Result<ThreadId, SpawnError> {
    let stack = StackFrames::allocate()?;
    let _irq = InterruptsOff::new();
    // On the interrupt stack, as in `finish_switch`: threads are spawned
    // deep inside system calls (`clone`).
    arch::irqstack::run_on(|| {
        // The table's guard must be gone before `bury` (a temporary in the
        // same expression would live on through it).
        let dead = reap(&mut THREADS.lock());
        bury(dead);
    });
    let mut t = THREADS.lock();
    assert!(t.ready, "thread::spawn before thread::init");
    let id = match t.sched.spawn(this_cpu(), cpus) {
        Ok(id) => id,
        Err(e) => {
            drop(t);
            stack.free();
            return Err(if e == Error::TableFull { SpawnError::TableFull } else { SpawnError::NoSuchCpu });
        }
    };
    // Set up before the lock goes: another CPU may pick it at once.
    install(&mut t, id, stack, f, arg, user);
    t.names[id.slot()] = name;
    let kicks = t.sched.take_kicks();
    drop(t);
    kick(kicks);
    Ok(id)
}

/// Give `stack` (and `user`, for a user thread) to thread `id`, mapped in
/// its slot of [`STACKS`], and set it up to start in `f(arg)`.
fn install(t: &mut Threads, id: ThreadId, stack: StackFrames, f: ThreadFn, arg: usize, user: Option<Box<UserThread>>) {
    let stack = stack.map(t, id.slot());
    t.entries[id.slot()] = f as usize;
    // SAFETY: the stack was just allocated for this thread and is mapped;
    // `thread_start` has the signature `prepare` asks for.
    let rsp = unsafe { context::prepare(stack.top(), thread_start as *const () as usize, f as usize, arg) };
    // SAFETY: the slot's thread has not run yet (see `SavedRsp`).
    unsafe { *SAVED_RSP.0[id.slot()].get() = rsp };
    t.stacks[id.slot()] = Some(stack);
    crate::cputime::started(id.slot(), user.as_deref().map(UserThread::cpu_owner));
    t.user[id.slot()] = user;
}

/// Where every new thread starts, called by the trampoline in
/// [`arch::context`]. Interrupts are disabled.
#[allow(improper_ctypes_definitions, reason = "only our trampoline calls this, passing on what `install` stored")]
extern "sysv64" fn thread_start(f: ThreadFn, arg: usize) -> ! {
    finish_switch();
    // A new thread starts with interrupts off, because every switch happens
    // with them off, but it has no `InterruptsOff` of its own to restore:
    // turn them on so the tick can preempt it.
    if arch::interrupts_ready() {
        // SAFETY: this thread holds no lock and no `InterruptsOff`; the
        // switch that started it is complete, and the controllers are set up.
        unsafe { cpu::enable_interrupts() };
    }
    exit(f(arg))
}

/// An idle thread: runs only when its CPU has nothing else to run, and
/// then halts until an interrupt.
fn idle_main(_: usize) -> usize {
    loop {
        cpu::disable_interrupts();
        let switched = yield_now();
        if !switched && !arch::interrupts_ready() {
            // Nothing could ever wake a thread: report it rather than hang.
            panic!("deadlock: every thread is blocked before interrupts are set up");
        }
        if !switched {
            // Another CPU runs alone for a while (`smp::Exclusive`).
            arch::smp::park_if_stopped();
        }
        // With interrupts off, nothing can make a thread runnable here
        // between that check and the halt: whatever does is an interrupt
        // (a device, the tick, or another CPU's reschedule IPI), which then
        // ends the `hlt` (`sti` holds interrupts back for one instruction).
        // SAFETY: the idle thread holds no lock and no `InterruptsOff`, so
        // it owns this CPU's interrupt state.
        unsafe {
            if switched {
                cpu::enable_interrupts();
            } else {
                cpu::enable_interrupts_and_halt();
            }
        }
    }
}

/// Perform the switch the scheduler decided on for this CPU. Interrupts
/// must be off and [`THREADS`] unlocked. Returns when `sw.from` is resumed
/// (on whichever CPU), which for an exited thread is never.
fn switch(sw: Switch) {
    // A spinlock held across the switch would stay held (with interrupts
    // off) while other threads run and spin on it; an interrupt handler that
    // blocks would leave its interrupt unacknowledged.
    assert_eq!(sync::spinlocks_held(), 0, "thread {} blocks or yields while holding a spinlock", sw.from);
    assert!(!arch::irq::in_interrupt(), "thread {} blocks inside an interrupt handler", sw.from);
    {
        let mut guard = THREADS.lock();
        let t = &mut *guard;
        let top = t.stacks[sw.to.slot()].map(|s| s.top());
        let [from, to] = t.user.get_disjoint_mut([sw.from.slot(), sw.to.slot()]).expect("switch to the running thread");
        arch::user::switch(from.as_deref_mut().map(|u| &mut u.arch), to.as_deref().map(|u| &u.arch), top);
    }
    crate::cputime::switched(sw.from.slot(), sw.to.slot());
    percpu::this().set_current(sw.to.to_bits());
    let save = SAVED_RSP.0[sw.from.slot()].get();
    // SAFETY: `sw.to` is not running anywhere and its switch-out is
    // complete, so its saved stack pointer is stable (see `SavedRsp`).
    let load = unsafe { *SAVED_RSP.0[sw.to.slot()].get() };
    // SAFETY: interrupts are off (every caller holds `InterruptsOff`); `load`
    // was stored by `install` or by the switch away from `sw.to`, whose stack
    // stays allocated until it exits and is reaped; `save` is a static slot.
    unsafe { context::switch(save, load) };
    finish_switch();
}

/// Runs on the thread just switched in (on the CPU that switched to it),
/// first thing: the switch is complete, so the thread switched away from
/// may run elsewhere now; reap what can be reaped.
///
/// On this CPU's interrupt stack (ADR 0061): the thread switched to may be
/// one resumed deep inside a system call, or on its way out of an
/// interrupt, and dropping an address space takes a fair amount of stack.
fn finish_switch() {
    arch::irqstack::run_on(|| {
        let (dead, kicks) = {
            let mut t = THREADS.lock();
            t.sched.saved(this_cpu());
            let dead = reap(&mut t);
            (dead, t.sched.take_kicks())
        };
        kick(kicks);
        // THREADS is unlocked again: freeing address spaces may take a
        // while and takes the frame allocator and the heap.
        bury(dead);
    });
}

/// Drop the user state [`reap`] took out. This runs in whichever thread
/// happened to switch or spawn: maybe inside an interrupt handler, maybe
/// holding sleeping locks. Drop code that may sleep, or take a lock that
/// does not mask interrupts, checks [`reaping`] and defers that part.
fn bury(dead: Graveyard) {
    let flag = percpu::this().reaping();
    let was = flag.swap(true, core::sync::atomic::Ordering::SeqCst);
    let n = dead.iter().flatten().count();
    drop(dead);
    BURYING.fetch_sub(n, core::sync::atomic::Ordering::SeqCst);
    flag.store(was, core::sync::atomic::Ordering::SeqCst);
}

/// User states [`reap`] took out of the table that [`bury`] has not
/// finished dropping yet (on any CPU): their memory is still on its way
/// out ([`dying_user_threads`]).
static BURYING: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Whether the running code is dropping the user state of a reaped thread
/// (see [`bury`]): it must not sleep or take non-masking locks.
pub fn reaping() -> bool {
    percpu::this().reaping().load(core::sync::atomic::Ordering::SeqCst)
}

/// User state of reaped threads, dropped by the caller of [`reap`] once
/// THREADS is unlocked.
type Graveyard = [Option<Box<UserThread>>; 4];

/// Free the stacks of exited threads and take out their user state. Never
/// sees a stack still in use: a thread is only reaped once the switch away
/// from it is complete. Stops early (the rest is reaped on a later switch
/// or spawn) when the frame allocator is busy or the graveyard is full.
fn reap(t: &mut Threads) -> Graveyard {
    let mut dead: Graveyard = Default::default();
    let mut buried = 0;
    while let Some(id) = t.sched.unreaped() {
        if t.user[id.slot()].is_some() && buried == dead.len() {
            break;
        }
        if let Some(stack) = t.stacks[id.slot()] {
            // Lock order is THREADS before FRAMES only with try_lock: if the
            // frame allocator is busy (its holder was switched away, or is
            // another CPU), the stack is freed on a later switch or spawn.
            let Some(mut frames) = FRAMES.try_lock() else { break };
            let mut deepest = t.deepest;
            deepest.note(t, id.slot(), stack.used());
            t.deepest = deepest;
            backtrace::unregister_thread_stack(id.slot());
            let stack = stack.unmap(t);
            let alloc = frames.as_mut().expect("stacks come from the frame allocator");
            alloc.free(stack.phys, STACK_ORDER).expect("thread stack freed twice");
            t.stacks[id.slot()] = None;
        }
        if let Some(user) = t.user[id.slot()].take() {
            crate::cputime::reaped(id.slot(), user.process());
            dead[buried] = Some(user);
            buried += 1;
            BURYING.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
        }
        t.sched.reap(id).expect("unreaped() names an exited thread");
    }
    dead
}

/// Run `f` on the running thread's user state, if it is a user thread.
/// `f` runs with [`THREADS`] locked and interrupts off, so it must be short
/// and must not call into this module.
pub fn with_current_user<R>(f: impl FnOnce(&mut UserThread) -> R) -> Option<R> {
    let _irq = InterruptsOff::new();
    let mut t = THREADS.lock();
    let me = current().slot();
    t.user[me].as_deref_mut().map(f)
}

/// The running thread.
pub fn current() -> ThreadId {
    ThreadId::from_bits(percpu::this().current())
}

/// What thread `id` is doing, or `None` if it no longer exists.
pub fn state(id: ThreadId) -> Option<State> {
    let _irq = InterruptsOff::new();
    THREADS.lock().sched.state(id)
}

/// The CPU thread `id` runs on, waits for, or ran on last.
pub fn cpu_of(id: ThreadId) -> Option<usize> {
    let _irq = InterruptsOff::new();
    THREADS.lock().sched.cpu_of(id)
}

/// Let thread `id` run only on `cpus`. A thread that must leave the CPU it
/// runs on does so at that CPU's next tick (at once if it is the caller).
pub fn set_affinity(id: ThreadId, cpus: CpuSet) -> Result<(), Error> {
    let _irq = InterruptsOff::new();
    let kicks = {
        let mut t = THREADS.lock();
        t.sched.set_affinity(id, cpus)?;
        t.sched.take_kicks()
    };
    kick(kicks);
    if id == current() && !cpus.contains(this_cpu()) {
        yield_now();
    }
    Ok(())
}

/// The CPUs thread `id` may run on.
pub fn affinity(id: ThreadId) -> Option<CpuSet> {
    let _irq = InterruptsOff::new();
    THREADS.lock().sched.affinity(id)
}

/// Keeps the running thread on its CPU until dropped (then it may run
/// anywhere it could before).
#[derive(Debug)]
#[must_use]
pub struct Pinned {
    id: ThreadId,
    before: CpuSet,
    cpu: usize,
}

impl Pinned {
    /// The CPU the thread is pinned to.
    pub fn cpu(&self) -> usize {
        self.cpu
    }
}

impl Drop for Pinned {
    fn drop(&mut self) {
        let _ = set_affinity(self.id, self.before);
    }
}

/// Pin the running thread to the CPU it runs on now.
pub fn pin_current() -> Pinned {
    let _irq = InterruptsOff::new();
    let id = current();
    let cpu = this_cpu();
    let before = {
        let mut t = THREADS.lock();
        let before = t.sched.affinity(id).unwrap_or(CpuSet::ALL);
        t.sched.set_affinity(id, CpuSet::one(cpu)).expect("the running thread may run here");
        before
    };
    Pinned { id, before, cpu }
}

/// Make CPU `cpu` give its threads to the other CPUs and take none
/// (`true`, for `smp::Exclusive`), or take threads again (`false`).
pub fn drain_cpu(cpu: usize, on: bool) {
    let _irq = InterruptsOff::new();
    let kicks = {
        let mut t = THREADS.lock();
        let _ = t.sched.drain(cpu, on);
        let mut kicks = t.sched.take_kicks();
        if on {
            // Its idle thread must look again (and park).
            kicks.insert(cpu);
        }
        kicks
    };
    kick(kicks);
}

/// The CPUs the scheduler runs threads on.
pub fn online_cpus() -> CpuSet {
    let _irq = InterruptsOff::new();
    THREADS.lock().sched.online()
}

/// Make thread `id` notice something at once if it runs on another CPU
/// (a signal, an exit request): a reschedule IPI interrupts it, and on its
/// way back to ring 3 it looks at its flags. A thread that is not running
/// sees them when it next runs.
pub fn kick_thread(id: ThreadId) {
    let _irq = InterruptsOff::new();
    let on = {
        let t = THREADS.lock();
        (t.sched.state(id) == Some(State::Running)).then(|| t.sched.cpu_of(id)).flatten()
    };
    if let Some(cpu) = on.filter(|&c| c != this_cpu()) {
        arch::smp::send_reschedule(cpu);
    }
}

/// One thread, as [`list`] reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadInfo {
    pub id: ThreadId,
    /// Empty for threads started with [`spawn`].
    pub name: &'static str,
    pub state: State,
    /// The CPU it runs on, waits for, or ran on last.
    pub cpu: usize,
}

/// User threads whose process has ended: still on their way out (the
/// last one wakes the process's waiters before it exits) or exited and
/// not reaped yet, plus reaped ones whose user state another CPU (or this
/// one) is still dropping. Their memory, and the frames in it, goes when
/// that drop is done, so a count of user frames taken meanwhile still
/// includes those.
pub fn dying_user_threads() -> usize {
    let _irq = InterruptsOff::new();
    let t = THREADS.lock();
    t.user.iter().flatten().filter(|u| u.process().has_ended()).count()
        + BURYING.load(core::sync::atomic::Ordering::SeqCst)
}

/// Every thread that exists and has not been joined, in slot order.
pub fn list() -> Vec<ThreadInfo> {
    // Allocated before locking: nothing allocates under THREADS.
    let mut out = Vec::with_capacity(MAX_THREADS);
    let _irq = InterruptsOff::new();
    let t = THREADS.lock();
    for (id, state) in t.sched.live_threads() {
        out.push(ThreadInfo { id, name: t.names[id.slot()], state, cpu: t.sched.cpu_of(id).unwrap_or(0) });
    }
    out
}

/// A thread whose stack overflowed into its guard, as a fault report names
/// it ([`stack_guard_hit`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overflowed {
    pub slot: usize,
    /// `None` if the thread has no name, or the fault came at a moment the
    /// thread table was locked.
    pub name: Option<&'static str>,
}

impl core::fmt::Display for Overflowed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.name {
            Some(name) => write!(f, "{name} (slot {})", self.slot),
            None => write!(f, "in slot {}", self.slot),
        }
    }
}

/// The thread whose stack guard holds `addr`, if any: a fault there is an
/// overflow of that thread's stack. Called from the fault handler, so it
/// only looks the name up if the thread table is free.
pub fn stack_guard_hit(addr: u64) -> Option<Overflowed> {
    // Below the guard proper, the part of a stack that
    // `poly.debug.thread_stack_kib` left unmapped.
    let shrunk = |a: u64| {
        let slot = STACKS.stack_holding(a, 0)?;
        (a < STACKS.stack(slot)?.end - stack_limit()).then_some(slot)
    };
    let slot = STACKS.guard_hit(addr).or_else(|| shrunk(addr))?;
    let name = THREADS.try_lock().map(|t| t.names[slot]).filter(|n| !n.is_empty());
    Some(Overflowed { slot, name })
}

/// Let the next runnable thread run; this thread goes to the back of its
/// CPU's run queue (another CPU may take it from there). False if there
/// was nobody to switch to.
pub fn yield_now() -> bool {
    let _irq = InterruptsOff::new();
    let (sw, kicks) = {
        let mut t = THREADS.lock();
        let sw = t.sched.yield_now(this_cpu());
        (sw, t.sched.take_kicks())
    };
    kick(kicks);
    sw.map(switch).is_some()
}

/// End the running thread with exit code `code`.
pub fn exit(code: usize) -> ! {
    let _irq = InterruptsOff::new();
    let (sw, kicks) = {
        let mut t = THREADS.lock();
        let sw = t.sched.exit_current(this_cpu(), code).expect("an idle thread cannot exit");
        (sw, t.sched.take_kicks())
    };
    kick(kicks);
    switch(sw);
    unreachable!("exited thread {} was resumed", sw.from)
}

/// Wait for thread `id` to exit and return its exit code. Each thread can be
/// joined once.
pub fn join(id: ThreadId) -> Result<usize, Error> {
    let _irq = InterruptsOff::new();
    loop {
        let (r, kicks) = {
            let mut t = THREADS.lock();
            let r = t.sched.join(this_cpu(), id);
            (r, t.sched.take_kicks())
        };
        kick(kicks);
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

/// Timer interrupt exit point: account one tick to this CPU's running
/// thread and, if its time slice is used up and the CPU has another thread
/// to run, switch to that thread. Returns whether it switched (once this
/// thread runs again).
///
/// The way out of the timer interrupt calls it (`irq::run_deferred`):
/// - with interrupts disabled, as they are inside an interrupt gate;
/// - after acknowledging the interrupt (EOI): the thread switched to may run
///   for a long time before this returns, and a new thread never returns
///   through this handler at all;
/// - on the interrupted thread's own kernel stack, not the CPU's interrupt
///   stack or an IST stack, which the next interrupt would reuse while the
///   thread is switched out. The trap frame stays on the thread's stack;
///   when it is resumed this returns and the interrupt returns with `iretq`
///   as usual (on whichever CPU resumed it). The handler's own work ran on
///   the interrupt stack before, so this is all an interrupt adds to the
///   thread's stack (ADR 0061).
///
/// The interrupted thread may have been anywhere except inside a
/// spinlock's critical section (`THREADS` included): spinlocks keep
/// interrupts off while held, so the tick cannot get in there.
pub fn preempt_tick() -> bool {
    let _irq = InterruptsOff::new();
    let (sw, kicks) = {
        let mut t = THREADS.lock();
        let cpu = this_cpu();
        let sw = if t.ready && t.sched.tick(cpu) { t.sched.yield_now(cpu) } else { None };
        (sw, t.sched.take_kicks())
    };
    kick(kicks);
    sw.map(switch).is_some()
}

/// Called by the timer interrupt handler on every tick, on the interrupt
/// stack: wake the sleepers whose time is up. The handler's way out then
/// accounts the tick ([`preempt_tick`]).
pub fn on_timer_tick() {
    wake_sleepers();
}

/// The reschedule IPI's way out (same rules as [`preempt_tick`]): switch if
/// this CPU is idle and got a thread to run, or its running thread may no
/// longer run here.
pub fn on_reschedule() {
    let _irq = InterruptsOff::new();
    let (sw, kicks) = {
        let mut t = THREADS.lock();
        let cpu = this_cpu();
        let sw = if t.ready && t.sched.needs_switch(cpu) { t.sched.yield_now(cpu) } else { None };
        (sw, t.sched.take_kicks())
    };
    kick(kicks);
    if let Some(sw) = sw {
        switch(sw);
    }
}

/// Earliest deadline among the sleepers and the timed waits (`u64::MAX`:
/// none), so the tick only looks at them when one is due.
static NEXT_WAKE: AtomicU64 = AtomicU64::new(u64::MAX);

/// Block the running thread for at least `duration` (measured with the
/// monotonic clock). It is woken by the first tick after the deadline, so it
/// usually sleeps up to one tick longer.
pub fn sleep(duration: Duration) {
    let ns = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
    sleep_until(time::now_ns().saturating_add(ns));
}

/// Block the running thread until the clock reads `deadline_ns` or later:
/// a timed wait on a queue of its own that nothing else wakes.
pub fn sleep_until(deadline_ns: u64) {
    assert!(time::is_running(), "thread::sleep before the timer is running");
    let alone = WaitQueue::new();
    while time::now_ns() < deadline_ns {
        alone.wait_until_deadline(|| false, deadline_ns);
    }
}

/// Wake the timed waits (sleepers among them) whose deadline has passed,
/// earliest deadline first, and register the earliest of the others. The
/// CPU whose tick sees the deadline first claims it (resets it with a
/// compare-exchange), so one CPU wakes per deadline; a wait that registers
/// in between is in the table by the time this scan locks it. Waking in
/// deadline order keeps the order when a host stall makes several due at
/// once: woken onto one CPU's queue, they run in that order.
fn wake_sleepers() {
    let next = NEXT_WAKE.load(SeqCst);
    let now = time::now_ns();
    if next == u64::MAX || now < next || NEXT_WAKE.compare_exchange(next, u64::MAX, SeqCst, SeqCst).is_err() {
        return;
    }
    wake_due(now);
}

/// The part of [`wake_sleepers`] for when a deadline is due: out of line,
/// so the tick's frame stays small when none is (most ticks).
#[cold]
#[inline(never)]
fn wake_due(now: u64) {
    let _irq = InterruptsOff::new();
    let kicks = {
        let mut t = THREADS.lock();
        let mut due = [(u64::MAX, 0usize); MAX_THREADS];
        let mut n = 0;
        for slot in 0..MAX_THREADS {
            let Some(to) = t.timeouts[slot] else { continue };
            if to.deadline > now {
                NEXT_WAKE.fetch_min(to.deadline, SeqCst);
            } else {
                due[n] = (to.deadline, slot);
                n += 1;
            }
        }
        due[..n].sort_unstable();
        for &(_, slot) in &due[..n] {
            let Some(to) = t.timeouts[slot].take() else { continue };
            // SAFETY: the entry existed, so the queue is alive (see
            // `Timeout`); THREADS is locked, as its inner queue requires.
            let q = unsafe { &mut *(*(to.queue as *const WaitQueue)).0.get() };
            t.sched.wake_all(q);
        }
        t.sched.take_kicks()
    };
    kick(kicks);
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
    /// (on any CPU) cannot slip in between the check and going to sleep: no
    /// lost wakeups. It must be short and must not call into this module or
    /// take a lock (atomics and reading the clock are fine).
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
                t.sched.block_current(this_cpu(), q).expect("an idle thread cannot sleep")
            };
            switch(sw);
        }
    }

    /// Like [`wait_until`](Self::wait_until), but give up once the clock
    /// reads `deadline_ns` or later: true if `ready()` returned true, false
    /// on the timeout. The thread is woken by a wake on this queue or by the
    /// first tick after the deadline, whichever comes first (a tick wakes
    /// the whole queue; the other waiters check their conditions and go
    /// back to sleep).
    pub fn wait_until_deadline(&self, mut ready: impl FnMut() -> bool, deadline_ns: u64) -> bool {
        assert!(time::is_running(), "timed wait before the timer is running");
        let _irq = InterruptsOff::new();
        loop {
            let sw = {
                let mut t = THREADS.lock();
                let me = current().slot();
                if ready() {
                    t.timeouts[me] = None;
                    return true;
                }
                if time::now_ns() >= deadline_ns {
                    t.timeouts[me] = None;
                    return false;
                }
                t.timeouts[me] = Some(Timeout { deadline: deadline_ns, queue: self as *const WaitQueue as usize });
                NEXT_WAKE.fetch_min(deadline_ns, SeqCst);
                // SAFETY: THREADS is locked.
                let q = unsafe { &mut *self.0.get() };
                t.sched.block_current(this_cpu(), q).expect("an idle thread cannot sleep")
            };
            switch(sw);
        }
    }

    /// Wake the longest-waiting thread. False if none was waiting.
    pub fn wake_one(&self) -> bool {
        let _irq = InterruptsOff::new();
        let (woke, kicks) = {
            let mut t = THREADS.lock();
            // SAFETY: THREADS is locked.
            let woke = t.sched.wake_one(unsafe { &mut *self.0.get() }).is_some();
            (woke, t.sched.take_kicks())
        };
        kick(kicks);
        woke
    }

    /// Wake every waiting thread; returns how many there were.
    pub fn wake_all(&self) -> usize {
        let _irq = InterruptsOff::new();
        let (n, kicks) = {
            let mut t = THREADS.lock();
            // SAFETY: THREADS is locked.
            let n = t.sched.wake_all(unsafe { &mut *self.0.get() });
            (n, t.sched.take_kicks())
        };
        kick(kicks);
        n
    }
}

// --- Tests ------------------------------------------------------------------

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};

/// For tests whose threads must take turns in a fixed order: the calling
/// thread stays on its CPU and cannot be preempted while this is held (the
/// others still can, but each starts with a fresh time slice and gives up
/// the CPU long before it ends). Start the threads that take turns with it
/// with [`spawn_here`], so they share its CPU.
fn no_preemption() -> (Pinned, InterruptsOff) {
    let pin = pin_current();
    (pin, InterruptsOff::new())
}

/// [`spawn`] on the calling thread's CPU only.
pub fn spawn_here(f: ThreadFn, arg: usize) -> Result<ThreadId, SpawnError> {
    spawn_on("", f, arg, CpuSet::one(this_cpu()))
}

/// Yield until `done()` holds, for up to `secs` seconds of clock time;
/// whether it came to hold. For tests that wait for another CPU to do
/// something (such as reaping a thread that ran there), which takes a
/// while, and longer when the host stalls that CPU: a count of yields
/// would be gone in no time.
pub fn wait_until(secs: u64, mut done: impl FnMut() -> bool) -> bool {
    let start = crate::time::now_ns();
    while !done() {
        if crate::time::now_ns() - start > secs * crate::time::NANOS_PER_SEC {
            return false;
        }
        yield_now();
    }
    true
}

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
    #[boot_only]
    fn yield_interleaves_round_robin() -> TestResult {
        let _irq = no_preemption();
        TRACE.lock().len = 0;
        if current() != ThreadId::BOOT {
            return Err("tests do not run on the boot thread");
        }
        let a = spawn_here(ping, b'a' as usize).map_err(|_| "spawn failed")?;
        let b = spawn_here(ping, b'b' as usize).map_err(|_| "spawn failed")?;
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
        let _irq = no_preemption();
        TOKENS.store(0, SeqCst);
        WOKEN.store(0, SeqCst);
        if WAITERS.wake_one() || WAITERS.wake_all() != 0 {
            return Err("woke a thread on an empty queue");
        }
        let w1 = spawn_here(waiter, 0).map_err(|_| "spawn failed")?;
        let w2 = spawn_here(waiter, 0).map_err(|_| "spawn failed")?;
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

static TIMED: WaitQueue = WaitQueue::new();
static TIMED_TOKENS: AtomicUsize = AtomicUsize::new(0);

/// Waits on TIMED for a token for `ms` milliseconds; exits with 1 if it got
/// one, 0 on the timeout.
fn timed_waiter(ms: usize) -> usize {
    let take = || TIMED_TOKENS.fetch_update(SeqCst, SeqCst, |n| n.checked_sub(1)).is_ok();
    TIMED.wait_until_deadline(take, time::now_ns() + ms as u64 * time::NANOS_PER_MILLI) as usize
}

kernel_test! {
    fn timed_waits_end_on_a_wake_or_the_deadline() -> TestResult {
        TIMED_TOKENS.store(0, SeqCst);
        // Nobody wakes it: the deadline does, not before it, and not long
        // after (the first tick after it).
        let start = time::now_ns();
        if TIMED.wait_until_deadline(|| false, start + 20 * time::NANOS_PER_MILLI) {
            return Err("a timed wait on a false condition reported success");
        }
        let waited = time::now_ns() - start;
        if !(20 * time::NANOS_PER_MILLI..520 * time::NANOS_PER_MILLI).contains(&waited) {
            return Err("a timed wait did not end at its deadline");
        }
        if !TIMED.wait_until_deadline(|| true, 0) {
            return Err("a timed wait on a true condition timed out");
        }
        // Two waiters on one queue: the short one times out (the tick wakes
        // the queue), the long one goes back to sleep until it is woken.
        let short = spawn(timed_waiter, 20).map_err(|_| "spawn failed")?;
        let long = spawn(timed_waiter, 10_000).map_err(|_| "spawn failed")?;
        let start = time::now_ns();
        if join(short) != Ok(0) {
            return Err("the short timed wait did not time out");
        }
        sleep(Duration::from_millis(5));
        if state(long) != Some(State::Blocked) {
            return Err("waking the queue for one deadline ended another thread's wait");
        }
        TIMED_TOKENS.store(1, SeqCst);
        TIMED.wake_one();
        if join(long) != Ok(1) {
            return Err("a timed wait that was woken did not report success");
        }
        if time::now_ns() - start > 5 * time::NANOS_PER_SEC {
            return Err("a woken timed wait waited for its deadline");
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
        // The other CPUs parked: nothing else takes frames or slots.
        let _only = crate::smp::exclusive();
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

/// Stands in for the timer: calls `preempt_tick` until told to stop, with
/// the real tick held off.
fn spinner(_: usize) -> usize {
    let _irq = InterruptsOff::new();
    while !STOP.load(SeqCst) {
        SPINS.fetch_add(1, SeqCst);
        preempt_tick();
    }
    0
}

kernel_test! {
    fn preempt_tick_switches_after_a_time_slice() -> TestResult {
        let _irq = no_preemption();
        for _ in 0..2 * TIME_SLICE_TICKS {
            if preempt_tick() {
                return Err("preempted with nobody else to run");
            }
        }
        SPINS.store(0, SeqCst);
        STOP.store(false, SeqCst);
        let t = spawn_here(spinner, 0).map_err(|_| "spawn failed")?;
        yield_now();
        // The spinner got a full slice, then the tick switched back here. A
        // real tick or two may land in the few instructions between the
        // spinner's start (interrupts on) and its `InterruptsOff`.
        let spins = SPINS.load(SeqCst);
        // Stopped either way: a spinner left running would hold this CPU
        // with interrupts off for the rest of the boot.
        STOP.store(true, SeqCst);
        let joined = join(t);
        if !(TIME_SLICE_TICKS as usize - 2..=TIME_SLICE_TICKS as usize).contains(&spins) {
            crate::kprintln!("thread: the spinner ran {spins} ticks of a {TIME_SLICE_TICKS}-tick slice");
            return Err("not preempted after one time slice");
        }
        joined.map(|_| ()).map_err(|_| "spinner did not finish")
    }
}

/// Progress of each `burn` thread.
static BURNED: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];

/// CPU-bound: never yields, blocks or sleeps. Only preemption lets anyone
/// else run while it does.
fn burn(i: usize) -> usize {
    while !STOP.load(SeqCst) {
        BURNED[i].fetch_add(1, SeqCst);
    }
    0
}

kernel_test! {
    fn cpu_bound_threads_are_preempted() -> TestResult {
        STOP.store(false, SeqCst);
        for b in &BURNED {
            b.store(0, SeqCst);
        }
        let a = spawn(burn, 0).map_err(|_| "spawn failed")?;
        let b = spawn(burn, 1).map_err(|_| "spawn failed")?;
        // This thread is CPU-bound too: it shares the CPU with both and sees
        // each make progress between its own turns. Two turns each take
        // about six time slices; allow far more for a slow or busy host.
        let start = time::now_ns();
        let mut turns = [0usize; 2];
        let mut last = [0usize; 2];
        while turns.iter().any(|&t| t < 2) && time::now_ns() - start < 2 * time::NANOS_PER_SEC {
            for i in 0..2 {
                let now = BURNED[i].load(SeqCst);
                if now != last[i] {
                    turns[i] += 1;
                    last[i] = now;
                }
            }
        }
        STOP.store(true, SeqCst);
        join(a).map_err(|_| "join failed")?;
        join(b).map_err(|_| "join failed")?;
        if turns[0] < 2 || turns[1] < 2 {
            return Err("CPU-bound threads did not take turns: no preemption");
        }
        Ok(())
    }
}

kernel_test! {
    fn sleep_lasts_at_least_the_duration() -> TestResult {
        for ms in [0u64, 1, 10] {
            let start = time::now_ns();
            sleep(Duration::from_millis(ms));
            let slept = time::now_ns() - start;
            if slept < ms * time::NANOS_PER_MILLI {
                return Err("sleep returned early");
            }
            if slept > ms * time::NANOS_PER_MILLI + 500 * time::NANOS_PER_MILLI {
                return Err("sleep overslept by more than half a second");
            }
        }
        Ok(())
    }
}

/// Where the napper of `list_names_threads_and_their_states` waits, and
/// what lets it go.
static NAPPERS: WaitQueue = WaitQueue::new();
static NAP_OVER: AtomicBool = AtomicBool::new(false);

/// Blocks until the test lets it go: a timed sleep could end before the
/// test looks when the host holds the emulator off for longer.
fn nap(_: usize) -> usize {
    NAPPERS.wait_until(|| NAP_OVER.load(SeqCst));
    0
}

kernel_test! {
    fn list_names_threads_and_their_states() -> TestResult {
        NAP_OVER.store(false, SeqCst);
        let t = spawn_named("napper", nap, 0).map_err(|_| "spawn failed")?;
        let wake = || {
            NAP_OVER.store(true, SeqCst);
            NAPPERS.wake_all();
        };
        let start = time::now_ns();
        while state(t) != Some(State::Blocked) {
            if time::now_ns() - start > 5 * time::NANOS_PER_SEC {
                wake();
                return Err("the thread never went to sleep");
            }
            yield_now();
        }
        let all = list();
        wake();
        let me = current();
        let find = |id: ThreadId| all.iter().find(|i| i.id == id);
        let listed = find(t).is_some_and(|i| i.name == "napper" && i.state == State::Blocked)
            && find(me).is_some_and(|i| i.state == State::Running && (me != ThreadId::BOOT || i.name == "boot"))
            && find(ThreadId::IDLE).is_some_and(|i| i.name == "idle" && i.cpu == 0)
            && all.iter().filter(|i| i.name.starts_with("idle")).count() == online_cpus().len();
        join(t).map_err(|_| "join failed")?;
        if !listed {
            return Err("list() did not report the threads' names and states");
        }
        if list().iter().any(|i| i.id == t) {
            return Err("a joined thread is still listed");
        }
        Ok(())
    }
}

/// The deadline each sleeper of `sleepers_wake_in_deadline_order` set
/// itself and when it woke, by its sleep in tens of milliseconds minus one.
static DEADLINE_AT: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static WOKE_AT: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

fn sleeper(ms: usize) -> usize {
    let deadline = time::now_ns() + ms as u64 * time::NANOS_PER_MILLI;
    DEADLINE_AT[ms / 10 - 1].store(deadline, SeqCst);
    sleep_until(deadline);
    WOKE_AT[ms / 10 - 1].store(time::now_ns(), SeqCst);
    0
}

/// Times `sleepers_wake_in_deadline_order` tries for a clean order.
const SLEEP_ORDER_ATTEMPTS: usize = 5;

kernel_test! {
    fn sleepers_wake_in_deadline_order() -> TestResult {
        // Started longest first; with every thread asleep the CPU idles in
        // `hlt` and only the timer can wake them. Each sleeper records the
        // deadline it set: the host may deschedule the emulator between the
        // threads' starts, so their deadlines are only ordered as recorded.
        //
        // Waking early is always wrong. The order is only promised for
        // sleepers whose deadlines are a tick or more apart: when the host
        // stalls the emulator past two deadlines, the first tick after it
        // finds both due and wakes them together, and they run in queue
        // order. So an attempt out of order is tried again; a wake-up that
        // ignores the earliest deadline (the bug this guards against) is out
        // of order every time.
        let _pin = pin_current();
        for _ in 0..SLEEP_ORDER_ATTEMPTS {
            for w in WOKE_AT.iter().chain(&DEADLINE_AT) {
                w.store(0, SeqCst);
            }
            // On one CPU: woken threads on several CPUs record their wake-up
            // as each of those CPUs gets to run them, which the host may hold
            // off for any of them; on one CPU they run in the order they were
            // woken.
            let ids = [spawn_here(sleeper, 30), spawn_here(sleeper, 20), spawn_here(sleeper, 10)];
            for id in ids {
                join(id.map_err(|_| "spawn failed")?).map_err(|_| "join failed")?;
            }
            let deadline = DEADLINE_AT.each_ref().map(|d| d.load(SeqCst));
            let woke = WOKE_AT.each_ref().map(|w| w.load(SeqCst));
            if woke.iter().zip(&deadline).any(|(w, d)| w < d) {
                return Err("a sleeper woke before its deadline");
            }
            let mut by_deadline = [0, 1, 2];
            by_deadline.sort_by_key(|&i| deadline[i]);
            if by_deadline.windows(2).all(|p| woke[p[0]] <= woke[p[1]]) {
                return Ok(());
            }
            crate::kprintln!("thread: sleepers woke out of order (deadlines {deadline:?}, woke {woke:?}); again");
        }
        Err("sleepers did not wake in deadline order")
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

/// The most of a thread's stack one interrupt may take, below wherever the
/// thread was: the trap frame and the way out, which may switch threads
/// (ADR 0061). The handler itself, and the clean-up after a switch, run on
/// the CPU's interrupt stack.
pub const INTERRUPT_STACK_BUDGET: u64 = 1024;

/// Bytes of thread `id`'s stack ever used (see [`deepest_stack`]).
pub fn stack_used_by(id: ThreadId) -> Option<u64> {
    let _irq = InterruptsOff::new();
    let t = THREADS.lock();
    t.sched.state(id)?;
    t.stacks[id.slot()].map(|s| s.used())
}

static DEEP_RSP: AtomicU64 = AtomicU64::new(0);
static DEEP_SPINS: AtomicUsize = AtomicUsize::new(0);
static DEEP_TURNS: AtomicUsize = AtomicUsize::new(0);
static DEEP_STOP: AtomicBool = AtomicBool::new(false);

/// Go down the stack a few hundred bytes at a time until at most a little
/// more than `room` bytes are left above `bottom`, then spin there with
/// interrupts on, calling nothing, until told to stop: whatever lands
/// below that point meanwhile is the interrupts' doing.
#[inline(never)]
fn descend(room: u64, bottom: u64) {
    let pad = [0u8; 256];
    core::hint::black_box(&pad);
    let rsp: u64;
    // SAFETY: reads the stack pointer, nothing else.
    unsafe { core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack, preserves_flags)) };
    if rsp - bottom > room + 320 {
        descend(room, bottom);
        // Used after the call, so the call cannot become a jump.
        core::hint::black_box(&pad);
        return;
    }
    DEEP_RSP.store(rsp, SeqCst);
    while !DEEP_STOP.load(SeqCst) {
        DEEP_SPINS.fetch_add(1, SeqCst);
    }
}

/// Spins (see [`descend`]) with only [`INTERRUPT_STACK_BUDGET`] bytes and a
/// little more of its stack left.
fn deep_spinner(_: usize) -> usize {
    let top = STACKS.stack(current().slot()).expect("a thread's slot has a stack").end;
    descend(INTERRUPT_STACK_BUDGET, top - stack_limit());
    0
}

/// Does nothing: something to reap.
fn short_lived(_: usize) -> usize {
    0
}

/// Takes turns with the deep spinner, counting the turns it saw it take
/// (it gives up the CPU only when the tick preempts it), and leaves
/// exited threads to reap, some of them on the switch back into it.
fn churner(_: usize) -> usize {
    let mut last = DEEP_SPINS.load(SeqCst);
    while !DEEP_STOP.load(SeqCst) {
        let now = DEEP_SPINS.load(SeqCst);
        if now != last {
            DEEP_TURNS.fetch_add(1, SeqCst);
            last = now;
        }
        if let Ok(t) = spawn_here(short_lived, 0) {
            let _ = detach(t);
        }
        yield_now();
    }
    0
}

kernel_test! {
    /// The overflow ADR 0061 is about: a thread deep in its stack (as deep
    /// as the clone path went) is interrupted by the tick, preempted,
    /// resumed and switched to while exited threads are reaped, over and
    /// over. With the handlers on the thread's stack this ran into the guard.
    fn interrupts_take_little_of_the_interrupted_threads_stack() -> TestResult {
        let _pin = pin_current();
        DEEP_STOP.store(false, SeqCst);
        DEEP_SPINS.store(0, SeqCst);
        DEEP_TURNS.store(0, SeqCst);
        DEEP_RSP.store(0, SeqCst);
        let here = CpuSet::one(this_cpu());
        let deep = spawn_on("deep-spinner", deep_spinner, 0, here).map_err(|_| "spawn failed")?;
        let churn = match spawn_here(churner, 0) {
            Ok(t) => t,
            Err(_) => {
                DEEP_STOP.store(true, SeqCst);
                let _ = join(deep);
                return Err("spawn failed");
            }
        };
        // Each of the spinner's turns after the first ended in a preemption.
        let preempted = wait_until(60, || DEEP_TURNS.load(SeqCst) >= 8);
        let used = stack_used_by(deep);
        DEEP_STOP.store(true, SeqCst);
        let joined = join(deep).is_ok() && join(churn).is_ok();
        if !preempted || !joined {
            return Err("the deep spinner was not preempted again and again, or did not end");
        }
        let used = used.ok_or("no stack for the deep spinner")?;
        let top = STACKS.stack(deep.slot()).ok_or("no stack slot")?.end;
        let rsp = DEEP_RSP.load(SeqCst);
        let room = rsp - (top - stack_limit());
        let cost = rsp.saturating_sub(top - used);
        crate::kprintln!(
            "thread: interrupts took {cost} bytes of a preempted thread's stack (budget {INTERRUPT_STACK_BUDGET}, \
             {room} left), over {} preemptions",
            DEEP_TURNS.load(SeqCst)
        );
        if cost > INTERRUPT_STACK_BUDGET {
            return Err("an interrupt takes more of the interrupted thread's stack than its budget");
        }
        Ok(())
    }
}
