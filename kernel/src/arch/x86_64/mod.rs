//! x86-64 support. Everything CPU- or board-specific lives under `arch/` so
//! that an aarch64 port only has to add a sibling module.

pub mod apic;
pub mod backtrace;
mod boot;
pub mod context;
pub mod cpu;
pub mod dispi;
pub mod gdt;
pub mod idt;
pub mod irq;
pub mod mmio;
pub mod paging;
pub mod port;
pub mod serial;
pub mod time;

use crate::acpi::Platform;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_irq::CpuFeatures;

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

static INTERRUPTS_READY: AtomicBool = AtomicBool::new(false);

/// Whether [`init_interrupts`] has run: the 8259s are remapped, the APICs and
/// the tick are set up, so enabling interrupts is safe. (Before that, the
/// 8259s would deliver on the CPU exception vectors.)
pub fn interrupts_ready() -> bool {
    INTERRUPTS_READY.load(Ordering::Acquire)
}

/// The CPUID bits interrupt and timer setup depend on.
pub fn irq_features() -> CpuFeatures {
    use core::arch::x86_64::__cpuid;
    let max_basic = __cpuid(0).eax;
    let max_extended = __cpuid(0x8000_0000).eax;
    let leaf1 = __cpuid(1);
    // A leaf above the maximum reads as 0: every feature in it is absent.
    let leaf6_eax = if max_basic >= 6 { __cpuid(6).eax } else { 0 };
    let leaf_8000_0007_edx = if max_extended >= 0x8000_0007 { __cpuid(0x8000_0007).edx } else { 0 };
    CpuFeatures::from_cpuid(leaf1.ecx, leaf1.edx, leaf6_eax, leaf_8000_0007_edx)
}

/// Bring up interrupts and time on the boot CPU (ADR 0013): remap and mask
/// the 8259s, enable the local APIC, map and mask every I/O APIC, start the
/// reference timer (HPET), calibrate the TSC and the APIC timer, start the
/// monotonic clock and the periodic tick. Interrupts stay disabled; the
/// caller enables them once it is ready to be preempted.
///
/// # Safety
/// Call once, on the boot CPU, with interrupts disabled, after the heap and
/// `thread::init` (the tick preempts threads) and with the ACPI tables read.
pub unsafe fn init_interrupts(platform: Option<&Platform>) -> Result<(), &'static str> {
    assert!(!cpu::interrupts_enabled(), "interrupt setup with interrupts enabled");
    let features = irq_features();
    if platform.is_none_or(|p| p.has_8259) {
        apic::init_pic();
    }
    let lapic = apic::init_local_apic(&features, platform)?;
    if let Some(p) = platform {
        apic::init_io_apics(p);
    }
    time::init(platform, &features, lapic)?;
    INTERRUPTS_READY.store(true, Ordering::Release);
    Ok(())
}
