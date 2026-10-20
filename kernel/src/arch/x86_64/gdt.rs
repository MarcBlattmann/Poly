//! Global descriptor table and task state segment.
//!
//! Selector layout is chosen for `syscall`/`sysret`, which require kernel
//! code/data and user data/code to sit next to each other in this order.

use core::arch::asm;
use core::mem::size_of;

pub const KERNEL_CODE: u16 = 0x08;
pub const KERNEL_DATA: u16 = 0x10;
#[allow(dead_code, reason = "used by the syscall/sysret path")]
pub const USER_DATA: u16 = 0x18 | 3;
#[allow(dead_code, reason = "used by the syscall/sysret path")]
pub const USER_CODE: u16 = 0x20 | 3;
pub const TSS: u16 = 0x28;

/// IST slot used for double faults, so they get a known-good stack even
/// when the fault was caused by a stack overflow.
pub const DOUBLE_FAULT_IST: u8 = 1;

#[repr(C, packed(4))]
struct TaskStateSegment {
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

#[repr(C, align(16))]
struct Stack([u8; 16 * 1024]);

static mut DOUBLE_FAULT_STACK: Stack = Stack([0; 16 * 1024]);

/// Address range of the double-fault IST stack.
pub fn double_fault_stack() -> core::ops::Range<u64> {
    let bottom = (&raw const DOUBLE_FAULT_STACK) as u64;
    bottom..bottom + size_of::<Stack>() as u64
}

static mut TSS_SEGMENT: TaskStateSegment = TaskStateSegment {
    _reserved0: 0,
    rsp: [0; 3],
    _reserved1: 0,
    ist: [0; 7],
    _reserved2: 0,
    _reserved3: 0,
    iomap_base: size_of::<TaskStateSegment>() as u16,
};

static mut GDT: [u64; 7] = [
    0,
    0x00af_9a00_0000_ffff, // kernel code, 64-bit
    0x00cf_9200_0000_ffff, // kernel data
    0x00cf_f200_0000_ffff, // user data
    0x00af_fa00_0000_ffff, // user code, 64-bit
    0,                     // TSS low (filled in init)
    0,                     // TSS high
];

/// # Safety
/// Call once on the boot CPU, before interrupts are enabled.
pub unsafe fn init() {
    // SAFETY: single-threaded early boot; nothing else touches these statics.
    unsafe {
        let stack_top = (&raw const DOUBLE_FAULT_STACK) as u64 + size_of::<Stack>() as u64;
        TSS_SEGMENT.ist[DOUBLE_FAULT_IST as usize - 1] = stack_top;

        let base = (&raw const TSS_SEGMENT) as u64;
        let limit = size_of::<TaskStateSegment>() as u64 - 1;
        GDT[5] = (limit & 0xffff)
            | ((base & 0xff_ffff) << 16)
            | (0x89 << 40) // present, 64-bit TSS (available)
            | (((limit >> 16) & 0xf) << 48)
            | (((base >> 24) & 0xff) << 56);
        GDT[6] = base >> 32;

        let ptr = DescriptorTablePointer { limit: (size_of::<[u64; 7]>() - 1) as u16, base: (&raw const GDT) as u64 };
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
