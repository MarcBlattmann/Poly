//! Thread stacks: where they are mapped ([`STACKS`]), their frames, how
//! deep they were used (`poly.debug.stack_report`) and the debug limit.

use super::*;

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
pub(super) struct StackFrames {
    pub(super) phys: u64,
}

impl StackFrames {
    pub(super) fn allocate() -> Result<StackFrames, SpawnError> {
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

    pub(super) fn free(self) {
        let mut frames = FRAMES.lock();
        let alloc = frames.as_mut().expect("stacks come from the frame allocator");
        alloc.free(self.phys, STACK_ORDER).expect("thread stack freed twice");
    }

    /// Map the frames as the stack of table slot `slot`, whose part of
    /// [`STACKS`] is unmapped (its last thread was reaped). Holding
    /// [`THREADS`] makes the caller the slot's only user.
    pub(super) fn map(self, _threads: &mut Threads, slot: usize) -> Stack {
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
pub(super) struct Stack {
    pub(super) phys: u64,
    pub(super) virt: u64,
    /// Bytes mapped, at the top: [`STACK_SIZE`] unless shrunk for
    /// debugging ([`stack_limit`]).
    pub(super) usable: u64,
}

impl Stack {
    pub(super) fn top(&self) -> u64 {
        self.virt + STACK_SIZE
    }

    /// The bytes of this stack that were ever in use, read off the paint
    /// (it must be mapped).
    pub(super) fn used(&self) -> u64 {
        // SAFETY: the stack's top `usable` bytes are mapped while its
        // thread exists, and only read here.
        unsafe { mm::stack_used(self.top() - self.usable, self.usable) }
    }

    /// Unmap the stack of a reaped thread; its frames can be freed then.
    pub(super) fn unmap(self, _threads: &mut Threads) -> StackFrames {
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

pub(super) fn stack_limit() -> u64 {
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
    pub(super) const NONE: Deepest = Deepest { bytes: 0, name: "", entry: 0 };

    /// Keep slot `slot`'s thread if its stack went `used` bytes deep, more
    /// than the record.
    pub(super) fn note(&mut self, t: &Threads, slot: usize, used: u64) {
        // The test's spinner goes deep on purpose; it is not what the record
        // is to show.
        if used > self.bytes && t.entries[slot] != deep_spinner as *const () as usize {
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
