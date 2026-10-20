use core::arch::asm;

pub fn halt_forever() -> ! {
    loop {
        // SAFETY: cli; hlt only stops this CPU.
        unsafe { asm!("cli", "hlt", options(nomem, nostack)) }
    }
}

pub fn breakpoint() {
    // SAFETY: int3 traps to our breakpoint handler, which returns.
    unsafe { asm!("int3", options(nomem, nostack)) }
}

/// The one-byte `ret` instruction (C3H), for tests that execute data.
pub const RETURN_OPCODE: u8 = 0xc3;

pub fn read_cr2() -> u64 {
    let v: u64;
    // SAFETY: reading CR2 has no side effects.
    unsafe { asm!("mov {}, cr2", out(reg) v, options(nomem, nostack, preserves_flags)) }
    v
}

pub fn read_cr3() -> u64 {
    let v: u64;
    // SAFETY: reading CR3 has no side effects.
    unsafe { asm!("mov {}, cr3", out(reg) v, options(nomem, nostack, preserves_flags)) }
    v
}

/// Load a new page-table root. Flushes every non-global TLB entry
/// (SDM Vol. 3A §4.10.4.1).
///
/// # Safety
/// `pml4` must be the physical address of a valid PML4 that maps the code
/// that is running, its stack and everything else the kernel still uses.
pub unsafe fn write_cr3(pml4: u64) {
    // SAFETY: forwarded from the caller. Not `nomem`: memory accesses must
    // not move across the switch.
    unsafe { asm!("mov cr3, {}", in(reg) pml4, options(nostack, preserves_flags)) }
}

/// CR4.PGE (bit 7): global pages (SDM Vol. 3A §2.5).
pub const CR4_PGE: u64 = 1 << 7;

pub fn read_cr4() -> u64 {
    let v: u64;
    // SAFETY: reading CR4 has no side effects.
    unsafe { asm!("mov {}, cr4", out(reg) v, options(nomem, nostack, preserves_flags)) }
    v
}

/// # Safety
/// The new value must be valid for this CPU and keep the kernel running
/// (no paging-mode change).
pub unsafe fn write_cr4(v: u64) {
    // SAFETY: forwarded from the caller.
    unsafe { asm!("mov cr4, {}", in(reg) v, options(nostack, preserves_flags)) }
}

/// IA32_EFER (SDM Vol. 4, Table 2-2).
pub const MSR_EFER: u32 = 0xc000_0080;
/// IA32_EFER.NXE (bit 11): enables the execute-disable bit in page tables.
pub const EFER_NXE: u64 = 1 << 11;
/// IA32_APIC_BASE (SDM Vol. 4, Table 2-2): bits 12 and up hold the local
/// APIC's physical base address.
pub const MSR_APIC_BASE: u32 = 0x1b;
/// IA32_PAT (SDM Vol. 4, Table 2-2; Vol. 3A §11.12.2): eight memory types,
/// one byte each, selected by a page's PAT, PCD and PWT bits.
pub const MSR_PAT: u32 = 0x277;

/// # Safety
/// `msr` must exist on this CPU (reading a missing MSR raises #GP).
pub unsafe fn read_msr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: forwarded from the caller.
    unsafe { asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) }
    (u64::from(hi) << 32) | u64::from(lo)
}

/// # Safety
/// `msr` must exist on this CPU, and the value must be valid for it and keep
/// the kernel running.
pub unsafe fn write_msr(msr: u32, v: u64) {
    // SAFETY: forwarded from the caller.
    unsafe {
        asm!("wrmsr", in("ecx") msr, in("eax") v as u32, in("edx") (v >> 32) as u32, options(nostack, preserves_flags))
    }
}

/// CPU features the memory manager depends on.
#[derive(Clone, Copy, Debug)]
pub struct Features {
    /// CPUID.80000001H:EDX.NX [bit 20]: IA32_EFER.NXE and the XD page bit.
    pub nx: bool,
    /// CPUID.80000001H:EDX.Page1GB [bit 26]: 1 GiB pages.
    pub page_1g: bool,
    /// CPUID.01H:EDX.PGE [bit 13]: global pages (CR4.PGE).
    pub pge: bool,
    /// CPUID.01H:EDX.PAT [bit 16]: the page attribute table (IA32_PAT).
    pub pat: bool,
}

pub fn features() -> Features {
    use core::arch::x86_64::__cpuid;
    // The boot stub already checked that leaf 0x80000001 exists (long mode).
    let basic = __cpuid(1);
    let ext = __cpuid(0x8000_0001);
    Features {
        nx: ext.edx & (1 << 20) != 0,
        page_1g: ext.edx & (1 << 26) != 0,
        pge: basic.edx & (1 << 13) != 0,
        pat: basic.edx & (1 << 16) != 0,
    }
}

/// RFLAGS.IF (bit 9): maskable interrupts enabled.
const RFLAGS_IF: u64 = 1 << 9;

/// Whether maskable interrupts are enabled on this CPU.
pub fn interrupts_enabled() -> bool {
    let flags: u64;
    // SAFETY: pushfq; pop only reads rflags (through the stack).
    unsafe { asm!("pushfq", "pop {}", out(reg) flags, options(nomem, preserves_flags)) }
    flags & RFLAGS_IF != 0
}

/// Disable maskable interrupts on this CPU. Prefer [`InterruptsOff`], which
/// restores the previous state.
pub fn disable_interrupts() {
    // SAFETY: cli only masks interrupts on this CPU. Not `nomem`: memory
    // accesses must not move out of the critical section that follows.
    unsafe { asm!("cli", options(nostack)) }
}

/// Enable maskable interrupts on this CPU.
///
/// # Safety
/// The caller owns this CPU's interrupt state: no [`InterruptsOff`] guard
/// or spinlock on this CPU expects interrupts to stay off (the scheduler's
/// data, for one, relies on that).
pub unsafe fn enable_interrupts() {
    // SAFETY: forwarded from the caller. Not `nomem`: memory accesses must
    // not move into the region where interrupts are on.
    unsafe { asm!("sti", options(nostack)) }
}

/// Enable interrupts and halt until the next one, atomically: `sti` takes
/// effect only after the following instruction (SDM Vol. 2B, `STI`), so an
/// interrupt that arrives in between still ends the `hlt` rather than being
/// handled before it and leaving the CPU asleep. Returns, with interrupts
/// enabled, after an interrupt has been handled.
///
/// # Safety
/// As for [`enable_interrupts`].
pub unsafe fn enable_interrupts_and_halt() {
    // SAFETY: forwarded from the caller; hlt only waits.
    unsafe { asm!("sti", "hlt", options(nostack)) }
}

/// The time-stamp counter (SDM Vol. 3B §18.17). Not serialising: it may be
/// read slightly before earlier instructions finish.
pub fn rdtsc() -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: rdtsc only reads the counter; CR4.TSD is clear, so it is
    // allowed at any privilege level (and we run in ring 0).
    unsafe { asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) }
    (u64::from(hi) << 32) | u64::from(lo)
}

/// Interrupts disabled on this CPU until dropped, then restored to how they
/// were. Nests. Held across a context switch, it belongs to the thread that
/// created it: that thread restores its own state when it resumes.
#[must_use]
#[derive(Debug)]
pub struct InterruptsOff {
    was_enabled: bool,
}

impl InterruptsOff {
    pub fn new() -> InterruptsOff {
        let was_enabled = interrupts_enabled();
        disable_interrupts();
        InterruptsOff { was_enabled }
    }
}

impl Drop for InterruptsOff {
    fn drop(&mut self) {
        if self.was_enabled {
            // SAFETY: interrupts were enabled when this guard was made, so
            // the code that owns this CPU's interrupt state wants them on.
            unsafe { enable_interrupts() }
        }
    }
}
