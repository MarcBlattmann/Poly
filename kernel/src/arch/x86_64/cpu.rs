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
}

pub fn features() -> Features {
    use core::arch::x86_64::__cpuid;
    // The boot stub already checked that leaf 0x80000001 exists (long mode).
    let basic = __cpuid(1);
    let ext = __cpuid(0x8000_0001);
    Features { nx: ext.edx & (1 << 20) != 0, page_1g: ext.edx & (1 << 26) != 0, pge: basic.edx & (1 << 13) != 0 }
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

/// What CPUID says about the processor, for the shell's `cpu` command
/// (Intel SDM Vol. 2A, `CPUID`; AMD APM Vol. 3, appendix E).
#[derive(Clone, Copy, Debug)]
pub struct Identity {
    /// Leaf 0: "GenuineIntel", "AuthenticAMD", "TCGTCGTCGTCG" (QEMU), ...
    pub vendor: [u8; 12],
    /// Leaves 0x80000002-4, NUL-padded; empty where the CPU has none.
    pub brand: [u8; 48],
    pub family: u32,
    pub model: u32,
    pub stepping: u32,
    /// Feature registers, indexed by [`FeatureReg`].
    regs: [u32; 6],
}

/// The CPUID registers [`FEATURES`] names bits in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureReg {
    Leaf1Edx,
    Leaf1Ecx,
    Leaf7Ebx,
    Leaf7Ecx,
    Ext1Edx,
    Ext1Ecx,
}

/// Feature flags worth listing, with their names as Linux's /proc/cpuinfo
/// spells them (the names in the SDM's CPUID tables, lowercased).
pub const FEATURES: &[(FeatureReg, u8, &str)] = {
    use FeatureReg::*;
    &[
        (Leaf1Edx, 0, "fpu"),
        (Leaf1Edx, 4, "tsc"),
        (Leaf1Edx, 5, "msr"),
        (Leaf1Edx, 6, "pae"),
        (Leaf1Edx, 9, "apic"),
        (Leaf1Edx, 11, "sep"),
        (Leaf1Edx, 12, "mtrr"),
        (Leaf1Edx, 13, "pge"),
        (Leaf1Edx, 15, "cmov"),
        (Leaf1Edx, 16, "pat"),
        (Leaf1Edx, 19, "clflush"),
        (Leaf1Edx, 23, "mmx"),
        (Leaf1Edx, 24, "fxsr"),
        (Leaf1Edx, 25, "sse"),
        (Leaf1Edx, 26, "sse2"),
        (Leaf1Edx, 28, "ht"),
        (Leaf1Ecx, 0, "sse3"),
        (Leaf1Ecx, 1, "pclmulqdq"),
        (Leaf1Ecx, 9, "ssse3"),
        (Leaf1Ecx, 12, "fma"),
        (Leaf1Ecx, 13, "cx16"),
        (Leaf1Ecx, 17, "pcid"),
        (Leaf1Ecx, 19, "sse4_1"),
        (Leaf1Ecx, 20, "sse4_2"),
        (Leaf1Ecx, 21, "x2apic"),
        (Leaf1Ecx, 22, "movbe"),
        (Leaf1Ecx, 23, "popcnt"),
        (Leaf1Ecx, 24, "tsc_deadline_timer"),
        (Leaf1Ecx, 25, "aes"),
        (Leaf1Ecx, 26, "xsave"),
        (Leaf1Ecx, 28, "avx"),
        (Leaf1Ecx, 29, "f16c"),
        (Leaf1Ecx, 30, "rdrand"),
        (Leaf1Ecx, 31, "hypervisor"),
        (Leaf7Ebx, 0, "fsgsbase"),
        (Leaf7Ebx, 3, "bmi1"),
        (Leaf7Ebx, 5, "avx2"),
        (Leaf7Ebx, 7, "smep"),
        (Leaf7Ebx, 8, "bmi2"),
        (Leaf7Ebx, 9, "erms"),
        (Leaf7Ebx, 10, "invpcid"),
        (Leaf7Ebx, 16, "avx512f"),
        (Leaf7Ebx, 18, "rdseed"),
        (Leaf7Ebx, 19, "adx"),
        (Leaf7Ebx, 20, "smap"),
        (Leaf7Ebx, 23, "clflushopt"),
        (Leaf7Ebx, 29, "sha_ni"),
        (Leaf7Ecx, 2, "umip"),
        (Leaf7Ecx, 3, "pku"),
        (Leaf7Ecx, 16, "la57"),
        (Leaf7Ecx, 22, "rdpid"),
        (Ext1Edx, 11, "syscall"),
        (Ext1Edx, 20, "nx"),
        (Ext1Edx, 26, "pdpe1gb"),
        (Ext1Edx, 27, "rdtscp"),
        (Ext1Edx, 29, "lm"),
        (Ext1Ecx, 0, "lahf_lm"),
        (Ext1Ecx, 2, "svm"),
        (Ext1Ecx, 5, "abm"),
    ]
};

impl Identity {
    pub fn vendor(&self) -> &str {
        core::str::from_utf8(&self.vendor).unwrap_or("?")
    }

    /// The brand string without its padding; empty if the CPU has none.
    pub fn brand(&self) -> &str {
        let end = self.brand.iter().position(|&b| b == 0).unwrap_or(self.brand.len());
        core::str::from_utf8(&self.brand[..end]).unwrap_or("?").trim()
    }

    pub fn has(&self, reg: FeatureReg, bit: u8) -> bool {
        self.regs[reg as usize] & (1 << bit) != 0
    }

    /// The names of the [`FEATURES`] this CPU has.
    pub fn features(&self) -> impl Iterator<Item = &'static str> + '_ {
        FEATURES.iter().filter(|&&(reg, bit, _)| self.has(reg, bit)).map(|&(_, _, name)| name)
    }
}

/// Read the processor's identity with CPUID.
pub fn identify() -> Identity {
    use core::arch::x86_64::__cpuid;
    let leaf0 = __cpuid(0);
    let mut vendor = [0u8; 12];
    for (i, r) in [leaf0.ebx, leaf0.edx, leaf0.ecx].into_iter().enumerate() {
        vendor[i * 4..i * 4 + 4].copy_from_slice(&r.to_le_bytes());
    }
    let max_extended = __cpuid(0x8000_0000).eax;
    let mut brand = [0u8; 48];
    if max_extended >= 0x8000_0004 {
        for (i, leaf) in (0x8000_0002..=0x8000_0004).enumerate() {
            let r = __cpuid(leaf);
            for (j, v) in [r.eax, r.ebx, r.ecx, r.edx].into_iter().enumerate() {
                brand[i * 16 + j * 4..i * 16 + j * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
    }
    let leaf1 = __cpuid(1);
    // Display family and model (SDM Vol. 2A, CPUID, figure "Version
    // Information Returned by CPUID in EAX").
    let base_family = (leaf1.eax >> 8) & 0xf;
    let family = if base_family == 0xf { base_family + ((leaf1.eax >> 20) & 0xff) } else { base_family };
    let mut model = (leaf1.eax >> 4) & 0xf;
    if base_family == 0x6 || base_family == 0xf {
        model |= ((leaf1.eax >> 16) & 0xf) << 4;
    }
    let (leaf7_ebx, leaf7_ecx) = if leaf0.eax >= 7 {
        let r = core::arch::x86_64::__cpuid_count(7, 0);
        (r.ebx, r.ecx)
    } else {
        (0, 0)
    };
    let (ext1_edx, ext1_ecx) = if max_extended >= 0x8000_0001 {
        let r = __cpuid(0x8000_0001);
        (r.edx, r.ecx)
    } else {
        (0, 0)
    };
    Identity {
        vendor,
        brand,
        family,
        model,
        stepping: leaf1.eax & 0xf,
        regs: [leaf1.edx, leaf1.ecx, leaf7_ebx, leaf7_ecx, ext1_edx, ext1_ecx],
    }
}
