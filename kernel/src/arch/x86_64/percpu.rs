//! Per-CPU data, reached through the GS base.
//!
//! Every CPU has one [`PerCpu`]: the boot CPU's is a static, each
//! application processor's is allocated when it is brought up (`smp.rs`)
//! and lives for the kernel's lifetime. In ring 0, IA32_GS_BASE points at
//! the running CPU's, so [`this`] is one `mov` from `gs:[16]` (the area's
//! own address) and needs no lock: the area belongs to the CPU, and code
//! that must not move to another CPU while it uses a field runs with
//! interrupts off (the scheduler only switches threads from interrupts,
//! yields and blocks).
//!
//! The first four words are for the assembly entry paths (`syscall.rs`,
//! `idt.rs`):
//!
//! ```text
//! gs:[0]   kernel stack top of the running thread (syscall switches to it)
//! gs:[8]   the user RSP while the syscall entry builds its frame
//! gs:[16]  this area's own address
//! gs:[24]  top of this CPU's interrupt stack, 0 until it has one (irqstack.rs)
//! ```
//!
//! GS holds the user's base in ring 3; every entry from ring 3 swaps it
//! (`swapgs`). The NMI, machine check and double fault can arrive at any
//! instruction, including between `syscall` and `swapgs`, so their entry
//! (`idt.rs`, `isr_paranoid`) does not trust GS at all: it loads this CPU's
//! area from the word just above its IST stack top, where [`ist_top`]
//! stored it, and restores whatever GS base it found on the way out.
//!
//! [`cpu`] gives other CPUs' areas, for the fields they may read or set
//! (atomics only).

use super::cpu;
use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU16, AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// Most CPUs the kernel runs on (the scheduler's table has room for this
/// many idle threads).
pub const MAX_CPUS: usize = 16;

/// PCIDs per CPU (12 bits would allow 4096; 256 is plenty).
pub const PCIDS: usize = 256;

/// One CPU's data. `#[repr(C)]`: the assembly uses the first four fields.
#[repr(C)]
#[derive(Debug)]
pub struct PerCpu {
    /// Offset 0: top of the running thread's kernel stack, where `syscall`
    /// switches to. Set with TSS.rsp0 on every switch to a user thread.
    pub(super) kernel_rsp: AtomicU64,
    /// Offset 8: the user RSP while the syscall entry builds its frame.
    pub(super) user_rsp: AtomicU64,
    /// Offset 16: the address of this area.
    this: AtomicU64,
    /// Offset 24: the top of this CPU's interrupt stack, which the entry of
    /// every external interrupt and IPI switches to; 0 until it is set up.
    pub(super) interrupt_stack: AtomicU64,
    /// Work an interrupt handler left for the way out, back on the
    /// interrupted thread's stack (`irq::PENDING_*` bits).
    pub(super) irq_pending: AtomicU8,
    /// The CPU's number: 0 for the boot CPU, then in start-up order.
    index: AtomicUsize,
    /// Its local APIC ID.
    pub(super) apic_id: AtomicU32,
    /// The running thread (`poly_sched::ThreadId::to_bits`).
    current: AtomicU64,
    /// Spinlocks held on this CPU (`sync`).
    spinlocks: AtomicUsize,
    /// Interrupt handlers running (0 or 1: they do not nest).
    irq_depth: AtomicUsize,
    /// The interrupt being handled came from ring 3.
    interrupted_user: AtomicBool,
    /// The scheduler is dropping reaped threads' state here (`thread`).
    reaping: AtomicBool,
    /// Timer ticks this CPU took.
    ticks: AtomicU64,
    /// The FPU area whose state the registers hold (`fpu.rs`).
    pub(super) fpu_owner: AtomicPtr<u8>,
    /// The address space loaded (null: the kernel's), for its CPU mask
    /// (`addrspace.rs`).
    pub(super) active_space: AtomicPtr<u8>,
    /// The address-space id each PCID was last given to on this CPU, and
    /// the next PCID to hand out (`addrspace.rs`).
    pub(super) pcid_owner: [AtomicU64; PCIDS],
    pub(super) next_pcid: AtomicU16,
    /// This CPU's TSS (TSS.rsp0 is set on every switch to a user thread).
    pub(super) tss: AtomicPtr<super::gdt::TaskStateSegment>,
    /// Bring-up state, see `smp.rs`.
    pub(super) state: AtomicU32,
}

impl PerCpu {
    pub const fn new() -> PerCpu {
        PerCpu {
            kernel_rsp: AtomicU64::new(0),
            user_rsp: AtomicU64::new(0),
            this: AtomicU64::new(0),
            interrupt_stack: AtomicU64::new(0),
            irq_pending: AtomicU8::new(0),
            index: AtomicUsize::new(0),
            apic_id: AtomicU32::new(0),
            current: AtomicU64::new(0),
            spinlocks: AtomicUsize::new(0),
            irq_depth: AtomicUsize::new(0),
            interrupted_user: AtomicBool::new(false),
            reaping: AtomicBool::new(false),
            ticks: AtomicU64::new(0),
            fpu_owner: AtomicPtr::new(core::ptr::null_mut()),
            active_space: AtomicPtr::new(core::ptr::null_mut()),
            pcid_owner: [const { AtomicU64::new(0) }; PCIDS],
            next_pcid: AtomicU16::new(1),
            tss: AtomicPtr::new(core::ptr::null_mut()),
            state: AtomicU32::new(0),
        }
    }

    pub fn index(&self) -> usize {
        self.index.load(Ordering::Relaxed)
    }

    pub fn apic_id(&self) -> u32 {
        self.apic_id.load(Ordering::Relaxed)
    }

    /// The running thread, as `ThreadId::to_bits`.
    pub fn current(&self) -> u64 {
        self.current.load(Ordering::Relaxed)
    }

    pub fn set_current(&self, bits: u64) {
        self.current.store(bits, Ordering::Relaxed);
    }

    pub fn spinlocks(&self) -> &AtomicUsize {
        &self.spinlocks
    }

    pub fn irq_depth(&self) -> &AtomicUsize {
        &self.irq_depth
    }

    pub fn interrupted_user(&self) -> &AtomicBool {
        &self.interrupted_user
    }

    pub fn reaping(&self) -> &AtomicBool {
        &self.reaping
    }

    pub fn ticks(&self) -> &AtomicU64 {
        &self.ticks
    }
}

impl Default for PerCpu {
    fn default() -> Self {
        PerCpu::new()
    }
}

/// Offset of [`PerCpu::interrupt_stack`], for the interrupt entry.
pub(super) const INTERRUPT_STACK_OFFSET: usize = core::mem::offset_of!(PerCpu, interrupt_stack);
const _: () = assert!(INTERRUPT_STACK_OFFSET == 24);

/// The boot CPU's area.
static BSP: PerCpu = PerCpu::new();

/// Every CPU's area by CPU number (null where none was set up).
static CPUS: [AtomicPtr<PerCpu>; MAX_CPUS] = [const { AtomicPtr::new(core::ptr::null_mut()) }; MAX_CPUS];
/// CPUs with an area: 1 + the highest number set up.
static COUNT: AtomicUsize = AtomicUsize::new(1);

/// Point GS at the boot CPU's area. The very first thing the kernel does:
/// every lock counts itself in it.
///
/// # Safety
/// Call once, on the boot CPU, before anything uses [`this`].
pub unsafe fn init_boot_cpu() {
    BSP.this.store((&raw const BSP) as u64, Ordering::Relaxed);
    CPUS[0].store((&raw const BSP).cast_mut(), Ordering::Release);
    // SAFETY: IA32_GS_BASE exists on every x86-64 CPU; nothing uses GS yet.
    unsafe {
        cpu::write_msr(cpu::MSR_GS_BASE, (&raw const BSP) as u64);
        cpu::write_msr(cpu::MSR_KERNEL_GS_BASE, 0);
    }
}

/// Register `area` as CPU `index`'s (an application processor's, before
/// it starts) and return it.
pub fn register(index: usize, area: &'static PerCpu, apic_id: u32) -> &'static PerCpu {
    assert!(index > 0 && index < MAX_CPUS, "CPU number out of range");
    area.this.store(area as *const PerCpu as u64, Ordering::Relaxed);
    area.index.store(index, Ordering::Relaxed);
    area.apic_id.store(apic_id, Ordering::Relaxed);
    CPUS[index].store((area as *const PerCpu).cast_mut(), Ordering::Release);
    COUNT.fetch_max(index + 1, Ordering::AcqRel);
    area
}

/// Point this CPU's GS base at `area` (an application processor, first
/// thing in long mode).
///
/// # Safety
/// `area` is this CPU's, registered with [`register`].
pub(super) unsafe fn load(area: &'static PerCpu) {
    // SAFETY: as in `init_boot_cpu`; the caller passes this CPU's area.
    unsafe {
        cpu::write_msr(cpu::MSR_GS_BASE, area as *const PerCpu as u64);
        cpu::write_msr(cpu::MSR_KERNEL_GS_BASE, 0);
    }
}

/// The boot CPU's area, also before GS is set (its APIC ID is filled in by
/// the interrupt setup).
pub(super) fn boot_cpu() -> &'static PerCpu {
    &BSP
}

/// The running CPU's area.
#[inline]
pub fn this() -> &'static PerCpu {
    let p: u64;
    // SAFETY: in ring 0 GS base always points at this CPU's area (set
    // first thing on each CPU, swapped on every ring transition, fixed up
    // by the paranoid entry), whose offset 16 holds its own address.
    unsafe { asm!("mov {}, gs:[16]", out(reg) p, options(nostack, preserves_flags, readonly)) };
    // SAFETY: the areas live for the kernel's lifetime.
    unsafe { &*(p as *const PerCpu) }
}

/// The running CPU's number. A thread may move to another CPU right after
/// reading it unless interrupts are off.
#[inline]
pub fn cpu_index() -> usize {
    this().index()
}

/// CPU `index`'s area, if it has one.
pub fn cpu(index: usize) -> Option<&'static PerCpu> {
    let p = CPUS.get(index)?.load(Ordering::Acquire);
    // SAFETY: only `init_boot_cpu` and `register` store, and only areas
    // that live for the kernel's lifetime.
    (!p.is_null()).then(|| unsafe { &*p })
}

/// How many CPU numbers are in use (the highest one plus one).
pub fn count() -> usize {
    COUNT.load(Ordering::Acquire)
}

/// Every CPU's area, by number.
pub fn all() -> impl Iterator<Item = &'static PerCpu> {
    (0..count()).filter_map(cpu)
}

/// The IST stack top to give the TSS for a stack ending at `end`, with
/// `area`'s address stored in the word at the top: the paranoid entry
/// finds this CPU's area there without trusting GS. The CPU pushes its
/// frame below the top, so the word stays.
///
/// # Safety
/// `end` is the 16-byte aligned end of a mapped, writable stack that only
/// this CPU uses, for interrupts.
pub unsafe fn ist_top(end: u64, area: &'static PerCpu) -> u64 {
    let top = end - 16;
    // SAFETY: the top 16 bytes of the stack, which the caller vouches for.
    unsafe { (top as *mut u64).write(area as *const PerCpu as u64) };
    top
}
