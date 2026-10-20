//! Global descriptor table and task state segment.
//!
//! Selector layout is chosen for `syscall`/`sysret`, which require kernel
//! code/data and user data/code to sit next to each other in this order
//! (SDM Vol. 2B, `SYSCALL` and `SYSRET`; IA32_STAR is programmed in
//! `syscall.rs`).
//!
//! The TSS holds the stacks the CPU switches to: `rsp0`, the kernel stack
//! of the running thread, for interrupts and exceptions from ring 3 (the
//! scheduler updates it on every switch to a user thread), and two
//! interrupt-stack-table entries (see [`DOUBLE_FAULT_IST`], [`NMI_IST`]).
//! Every CPU has its own GDT and TSS ([`CpuTables`]): the boot CPU's are
//! static, an application processor's come with its per-CPU area, and its
//! IST stacks are its own. The top word of each IST stack holds the CPU's
//! per-CPU area for the paranoid entry (`percpu::ist_top`).

use core::arch::{asm, global_asm};
use core::mem::size_of;
use core::sync::atomic::Ordering;

pub const KERNEL_CODE: u16 = 0x08;
pub const KERNEL_DATA: u16 = 0x10;
pub const USER_DATA: u16 = 0x18 | 3;
pub const USER_CODE: u16 = 0x20 | 3;
pub const TSS: u16 = 0x28;

/// IST slot used for double faults, so they get a known-good stack even
/// when the fault was caused by a stack overflow.
pub const DOUBLE_FAULT_IST: u8 = 1;

/// IST slot for the non-maskable interrupt and the machine check. Neither
/// can be masked, so either may arrive in the first instructions of the
/// `syscall` entry, while `rsp` is still the user's stack: without a stack
/// of their own they would run on memory user mode chose.
pub const NMI_IST: u8 = 2;

#[repr(C, packed(4))]
#[derive(Debug)]
pub struct TaskStateSegment {
    _reserved0: u32,
    rsp: [u64; 3],
    _reserved1: u64,
    ist: [u64; 7],
    _reserved2: u64,
    _reserved3: u16,
    iomap_base: u16,
}

#[repr(C, packed)]
pub(super) struct DescriptorTablePointer {
    pub limit: u16,
    pub base: u64,
}

// The double-fault stack. The linker script puts it above an unmapped guard
// page and defines the symbols around it.
global_asm!(
    r#"
    .section .stack.double_fault, "aw", @nobits
    .balign 4096
    .skip 16 * 1024
"#,
    options(att_syntax)
);

// The NMI / machine-check stack, likewise above a guard page.
global_asm!(
    r#"
    .section .stack.nmi, "aw", @nobits
    .balign 4096
    .skip 16 * 1024
"#,
    options(att_syntax)
);

unsafe extern "C" {
    static __double_fault_stack_bottom: u8;
    static __double_fault_stack_top: u8;
    static __nmi_stack_bottom: u8;
    static __nmi_stack_top: u8;
}

/// Address range of the double-fault IST stack.
pub fn double_fault_stack() -> core::ops::Range<u64> {
    (&raw const __double_fault_stack_bottom) as u64..(&raw const __double_fault_stack_top) as u64
}

/// Address range of the NMI / machine-check IST stack.
pub fn nmi_stack() -> core::ops::Range<u64> {
    (&raw const __nmi_stack_bottom) as u64..(&raw const __nmi_stack_top) as u64
}

/// Point this CPU's TSS.rsp0 at `top`: the stack the CPU switches to when
/// an interrupt or exception arrives in ring 3. The scheduler sets it to
/// the kernel stack of every user thread it switches to.
pub fn set_kernel_stack(top: u64) {
    let tss = super::percpu::this().tss.load(Ordering::Relaxed);
    assert!(!tss.is_null(), "no TSS loaded on this CPU");
    // SAFETY: this CPU's TSS, which only this CPU writes, and only with
    // interrupts off (the scheduler's switch): the CPU reads rsp0 only
    // while delivering an interrupt from ring 3, which cannot happen while
    // the kernel runs this. The field is not 8-byte aligned in the packed
    // TSS, hence the unaligned write.
    unsafe { (&raw mut (*tss).rsp).cast::<u64>().write_unaligned(top) }
}

/// One CPU's GDT and TSS. The GDT's layout is the same on every CPU; only
/// the TSS descriptor differs (it points at the CPU's own TSS).
#[repr(C, align(16))]
#[derive(Debug)]
pub struct CpuTables {
    gdt: [u64; 7],
    tss: TaskStateSegment,
}

impl CpuTables {
    pub const fn new() -> CpuTables {
        CpuTables {
            gdt: [
                0,
                0x00af_9a00_0000_ffff, // kernel code, 64-bit
                0x00cf_9200_0000_ffff, // kernel data
                0x00cf_f200_0000_ffff, // user data
                0x00af_fa00_0000_ffff, // user code, 64-bit
                0,                     // TSS low (filled in by `load`)
                0,                     // TSS high
            ],
            tss: TaskStateSegment {
                _reserved0: 0,
                rsp: [0; 3],
                _reserved1: 0,
                ist: [0; 7],
                _reserved2: 0,
                _reserved3: 0,
                iomap_base: size_of::<TaskStateSegment>() as u16,
            },
        }
    }
}

impl Default for CpuTables {
    fn default() -> Self {
        CpuTables::new()
    }
}

/// The boot CPU's tables.
static mut BSP_TABLES: CpuTables = CpuTables::new();

/// Load the boot CPU's GDT and TSS, with the double-fault and NMI stacks
/// from the linker script.
///
/// # Safety
/// Call once on the boot CPU, before interrupts are enabled, after GS
/// points at its per-CPU area.
pub unsafe fn init() {
    let area = super::percpu::boot_cpu();
    // SAFETY: the static stacks are this CPU's and 16-byte aligned; the
    // tables are touched only here, once, in early boot.
    unsafe {
        let df = super::percpu::ist_top(double_fault_stack().end, area);
        let nmi = super::percpu::ist_top(nmi_stack().end, area);
        load(&raw mut BSP_TABLES, df, nmi, area);
    }
}

/// Load an application processor's GDT and TSS from `tables`, with IST
/// stacks ending at `double_fault_end` and `nmi_end`.
///
/// # Safety
/// Once per CPU, first thing in long mode, on that CPU; `tables` and the
/// stacks are that CPU's alone and live forever; `area` is its per-CPU area.
pub unsafe fn init_ap(
    tables: *mut CpuTables,
    double_fault_end: u64,
    nmi_end: u64,
    area: &'static super::percpu::PerCpu,
) {
    // SAFETY: forwarded from the caller.
    unsafe {
        let df = super::percpu::ist_top(double_fault_end, area);
        let nmi = super::percpu::ist_top(nmi_end, area);
        load(tables, df, nmi, area);
    }
}

/// Fill in the TSS's IST entries and descriptor, then load the GDT, the
/// segments and the task register.
///
/// # Safety
/// As for [`init_ap`]; `tables` points at tables only this CPU uses.
unsafe fn load(tables: *mut CpuTables, df_top: u64, nmi_top: u64, area: &'static super::percpu::PerCpu) {
    // SAFETY: the caller hands us this CPU's tables.
    let t = unsafe { &mut *tables };
    t.tss.ist[DOUBLE_FAULT_IST as usize - 1] = df_top;
    t.tss.ist[NMI_IST as usize - 1] = nmi_top;
    let base = (&raw const t.tss) as u64;
    let limit = size_of::<TaskStateSegment>() as u64 - 1;
    t.gdt[5] = (limit & 0xffff)
        | ((base & 0xff_ffff) << 16)
        | (0x89 << 40) // present, 64-bit TSS (available)
        | (((limit >> 16) & 0xf) << 48)
        | (((base >> 24) & 0xff) << 56);
    t.gdt[6] = base >> 32;
    area.tss.store(&raw mut t.tss, Ordering::Release);
    let ptr = DescriptorTablePointer { limit: (size_of::<[u64; 7]>() - 1) as u16, base: (&raw const t.gdt) as u64 };
    // SAFETY: a valid GDT with the selectors below, kept alive forever;
    // reloading CS, the data segments and TR to it changes nothing else
    // (GS is not reloaded, so its base stays).
    unsafe {
        asm!("lgdt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));
        // Reload CS with a far return, then the data segments and TR.
        asm!(
            "push {sel}",
            "lea {tmp}, [rip + 55f]",
            "push {tmp}",
            "retfq",
            "55:",
            sel = in(reg) KERNEL_CODE as u64,
            tmp = lateout(reg) _,
            options(preserves_flags),
        );
        asm!(
            "mov ds, {0:x}",
            "mov es, {0:x}",
            "mov ss, {0:x}",
            in(reg) KERNEL_DATA,
            options(nostack, preserves_flags),
        );
        asm!("ltr {0:x}", in(reg) TSS, options(nostack, preserves_flags));
    }
}
