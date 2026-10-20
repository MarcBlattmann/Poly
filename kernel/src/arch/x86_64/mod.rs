//! x86-64 support. Everything CPU- or board-specific lives under `arch/` so
//! that an aarch64 port only has to add a sibling module.

pub mod backtrace;
mod boot;
pub mod context;
pub mod cpu;
pub mod gdt;
pub mod idt;
pub mod paging;
pub mod port;
pub mod serial;

pub use paging::{phys_range_to_virt, phys_to_virt, virt_range_to_phys};

/// Virtual address where physical address 0 is mapped for the kernel image
/// (and, until [`paging::init`], for all of the first GiB).
pub const KERNEL_VMA: u64 = 0xffff_ffff_8000_0000;

/// Physical memory reachable through the boot page tables, before
/// [`paging::init`] switches to the direct map.
pub const BOOT_MAPPED_PHYS: u64 = 1 << 30;

/// # Safety
/// Call once, first thing on the boot CPU.
pub unsafe fn init() {
    // SAFETY: forwarded from the caller.
    unsafe {
        gdt::init();
        idt::init();
    }
}
