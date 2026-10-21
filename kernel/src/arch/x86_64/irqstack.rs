//! Each CPU's interrupt stack (ADR 0061).
//!
//! External interrupts and IPIs (vectors 32-255) do their work on a stack
//! of the CPU's own, not on the interrupted thread's: `irq_common`
//! (`idt.rs`) pushes the trap frame where the CPU put it (the thread's
//! stack, so the thread can be switched out with it), switches to this
//! CPU's interrupt stack for the handler, and switches back for the way
//! out, which may switch threads (the tick's preemption, a reschedule IPI)
//! or deliver a signal and so must run on the thread's own stack
//! (`irq::run_deferred`). The scheduler's clean-up after a switch, which
//! frees stacks and drops the address spaces of exited threads, runs here
//! too ([`run_on`]), whatever the stack of the thread switched to looked
//! like. A thread's stack then only has to hold the trap frame and the
//! switch path on top of the deepest system call, never a handler's work,
//! and interrupts cannot pile up on it.
//!
//! Interrupt gates keep interrupts off while a handler runs, and [`run_on`]
//! requires them off, so nothing nests on an interrupt stack; an interrupt
//! that arrives on it all the same (a bug: something enabled interrupts
//! there) stays on it, is counted ([`nested`]) and does not switch
//! threads.
//!
//! The stacks are [`SIZE`] bytes, one buddy block each, mapped in their own
//! area ([`paging::INTERRUPT_STACKS_BASE`], after the thread stacks) above
//! an unmapped guard as large as the stack: an overflow faults and is
//! reported as "kernel stack overflow on CPU <n>'s interrupt stack". They
//! are painted ([`poly_mem::STACK_PAINT`]) before first use, so [`used`]
//! tells how deep each was ever used.

use super::{backtrace, cpu, paging, percpu, percpu::PerCpu};
use core::arch::naked_asm;
use core::sync::atomic::{AtomicU64, Ordering};
use poly_mem::StackArea;

/// Bytes of each interrupt stack.
pub const SIZE: u64 = 16 << 10;

/// Buddy order of an interrupt stack's frames.
const ORDER: usize = 2;
const _: () = assert!(poly_mem::PAGE_SIZE << ORDER == SIZE);

/// Where the interrupt stacks are: CPU `i`'s at the top of slot `i`, above
/// a guard as large as the stack.
pub const AREA: StackArea = StackArea::new(paging::INTERRUPT_STACKS_BASE, 2 * SIZE, SIZE, percpu::MAX_CPUS);
const _: () = assert!(AREA.size() <= paging::INTERRUPT_STACKS_SIZE, "the interrupt stacks do not fit their area");

/// Interrupts that arrived while their CPU was on its interrupt stack.
static NESTED: AtomicU64 = AtomicU64::new(0);

/// Give CPU `cpu` (whose per-CPU area is `area`) its interrupt stack: frames
/// from the frame allocator, painted, mapped in its slot of [`AREA`] and
/// registered with the backtrace walker. Call once per CPU, on the boot
/// CPU, before that CPU takes interrupts (until then they stay on the
/// interrupted stack).
pub fn set_up(cpu: usize, area: &'static PerCpu) -> Result<(), &'static str> {
    let stack = AREA.stack(cpu).ok_or("CPU number beyond the interrupt stack area")?;
    if area.interrupt_stack.load(Ordering::Acquire) != 0 {
        return Err("the CPU has an interrupt stack already");
    }
    let phys = crate::frames::FRAMES
        .lock()
        .as_mut()
        .and_then(|f| f.allocate(ORDER))
        .ok_or("no memory for an interrupt stack")?;
    let virt = paging::phys_to_virt(phys).ok_or("an interrupt stack outside the direct map")?;
    // SAFETY: the block was just allocated for this stack; the direct map
    // maps it read-write and nothing else refers to it.
    unsafe { crate::mm::paint_stack(virt, SIZE) };
    // SAFETY: the slot belongs to this CPU, whose stack is mapped only
    // here, once (checked above), and never unmapped; the frames are its
    // for good.
    unsafe { paging::map_thread_stack(stack.start, phys, SIZE) };
    backtrace::register_interrupt_stack(cpu, stack.start, SIZE);
    area.interrupt_stack.store(stack.end, Ordering::Release);
    Ok(())
}

/// The CPU whose interrupt stack's guard holds `addr`: a fault there is an
/// overflow of that stack.
pub fn guard_hit(addr: u64) -> Option<usize> {
    AREA.guard_hit(addr)
}

/// The bytes of CPU `cpu`'s interrupt stack ever used, if it has one.
pub fn used(cpu: usize) -> Option<u64> {
    let top = percpu::cpu(cpu)?.interrupt_stack.load(Ordering::Acquire);
    // SAFETY: a CPU's interrupt stack is mapped for good once its top is
    // published; reading it while the CPU uses it is fine (volatile words).
    (top != 0).then(|| unsafe { crate::mm::stack_used(top - SIZE, SIZE) })
}

/// Interrupts so far that arrived while their CPU was already on its
/// interrupt stack (something enabled interrupts there): 0, or a bug.
pub fn nested() -> u64 {
    NESTED.load(Ordering::Relaxed)
}

/// Whether `addr` lies on the running CPU's interrupt stack.
pub fn holds(addr: u64) -> bool {
    let top = percpu::this().interrupt_stack.load(Ordering::Relaxed);
    top != 0 && (top - SIZE..top).contains(&addr)
}

/// Whether the running code is on its CPU's interrupt stack.
pub fn on_it() -> bool {
    let rsp: u64;
    // SAFETY: reads the stack pointer, nothing else.
    unsafe { core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack, preserves_flags)) };
    holds(rsp)
}

/// Called by the interrupt entry with the address of the trap frame, which
/// is on the stack the interrupt arrived on: count the nesting this
/// module rules out.
pub(super) fn note_entry(frame: u64) {
    if holds(frame) {
        NESTED.fetch_add(1, Ordering::Relaxed);
    }
}

/// Run `f` on this CPU's interrupt stack, with interrupts off (required:
/// an interrupt arriving meanwhile would find its stack in use). Runs it
/// where it is if this CPU has no interrupt stack yet or is on it already.
/// `f` must not switch threads, block or enable interrupts.
pub fn run_on<F: FnOnce()>(f: F) {
    let top = percpu::this().interrupt_stack.load(Ordering::Relaxed);
    if top == 0 || on_it() {
        return f();
    }
    assert!(!cpu::interrupts_enabled(), "run_on: interrupts are enabled");
    let mut f = Some(f);
    // SAFETY: interrupts are off and this CPU is not on its interrupt stack,
    // so nothing else uses it until `call_on_stack` returns; `top` is its
    // 16-byte aligned top, and `trampoline::<F>` takes the `Option<F>` that
    // `arg` points to, which outlives the call.
    unsafe { call_on_stack((&raw mut f).cast(), trampoline::<F> as *const () as usize, top) };

    extern "sysv64" fn trampoline<F: FnOnce()>(arg: *mut u8) {
        // SAFETY: `run_on` passes its `Option<F>`, alive and not otherwise
        // referred to during the call.
        let f = unsafe { &mut *arg.cast::<Option<F>>() };
        (f.take().expect("run_on calls its closure once"))();
    }
}

/// Call `f(arg)` with the stack pointer at `top`, then return on the old
/// stack. The frame-pointer chain goes on from the new stack into the old
/// one, so backtraces show the caller.
///
/// # Safety
/// `top` is the 16-byte aligned top of a mapped stack nothing else uses
/// meanwhile; `f` is an `extern "sysv64" fn(*mut u8)`.
#[unsafe(naked)]
unsafe extern "sysv64" fn call_on_stack(arg: *mut u8, f: usize, top: u64) {
    // rbp keeps the old stack (callee-saved, so `f` returns it intact) and
    // heads the frame record `f`'s own frame links to.
    naked_asm!("push rbp", "mov rbp, rsp", "mov rsp, rdx", "call rsi", "mov rsp, rbp", "pop rbp", "ret")
}
