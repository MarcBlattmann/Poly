//! x86-64 support. Everything CPU- or board-specific lives under `arch/` so
//! that an aarch64 port only has to add a sibling module.

pub mod backtrace;
mod boot;
pub mod cpu;
pub mod gdt;
pub mod idt;
pub mod port;
pub mod serial;

/// Virtual address where physical address 0 is mapped for the kernel.
pub const KERNEL_VMA: u64 = 0xffff_ffff_8000_0000;

/// Physical memory reachable through the boot page tables.
pub const BOOT_MAPPED_PHYS: u64 = 1 << 30;

/// Translate a physical address covered by the boot mapping.
pub fn phys_to_virt(phys: u64) -> Option<u64> {
    (phys < BOOT_MAPPED_PHYS).then_some(phys + KERNEL_VMA)
}

/// # Safety
/// Call once, first thing on the boot CPU.
pub unsafe fn init() {
    // SAFETY: forwarded from the caller.
    unsafe {
        gdt::init();
        idt::init();
    }
}
