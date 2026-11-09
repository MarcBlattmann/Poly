//! Turning the MMU on with the identity map of `poly_aarch64_stub::mmu`.
//!
//! The sequence follows the Arm ARM (D8, "enabling the stage 1 MMU"): program
//! `MAIR_EL1`, `TCR_EL1` and `TTBR0_EL1`, synchronise, invalidate the TLB,
//! then set `SCTLR_EL1.{M, C, I}` and synchronise again. The code that runs
//! across the switch is identity mapped, so the next instruction is fetched
//! from the same address before and after.

use crate::cpu;
use core::arch::asm;
use core::cell::UnsafeCell;
use poly_aarch64_stub::mmu::{self, L1_ENTRIES};
use poly_aarch64_stub::regs;

/// `SCTLR_EL1.M`: the stage 1 MMU is on.
const SCTLR_M: u64 = 1 << 0;
/// `SCTLR_EL1.C`: data accesses are cacheable.
const SCTLR_C: u64 = 1 << 2;
/// `SCTLR_EL1.I`: instruction fetches are cacheable.
const SCTLR_I: u64 = 1 << 12;

#[repr(C, align(4096))]
struct Table(UnsafeCell<[u64; L1_ENTRIES]>);

// SAFETY: written once by `enable` on the boot CPU before the MMU is on and
// never again; afterwards only the hardware table walker reads it.
unsafe impl Sync for Table {}

static L1: Table = Table(UnsafeCell::new([0; L1_ENTRIES]));

/// Identity map `[ram_base, ram_end)` and the device gigabytes below it, and
/// turn the MMU and the caches on.
///
/// # Safety
/// Called once, on the boot CPU, with the MMU off, and everything the stub
/// uses from here on (its image, stack, heap, the UART) lies in the mapped
/// ranges.
pub unsafe fn enable(ram_base: u64, ram_end: u64) -> Result<(), mmu::MapError> {
    // SAFETY: caller's contract: nothing else touches the table, and the
    // MMU is off, so the walker does not read it yet.
    let table = unsafe { &mut *L1.0.get() };
    mmu::identity_map(table, ram_base, ram_end)?;
    let ips = regs::ips_for(cpu::id_aa64mmfr0_el1());
    // SAFETY: the registers describe the table just built; the barriers
    // order the table writes and the register writes before the walker
    // can use them (`dsb ishst`) and before the TLB invalidation takes
    // effect; the TLB holds nothing yet but is invalidated as the ARM ARM's
    // sequence does. The code running here is inside the identity map.
    unsafe {
        cpu::set_mair_el1(mmu::MAIR_EL1);
        cpu::set_tcr_el1(mmu::tcr_el1(ips));
        cpu::set_ttbr0_el1(L1.0.get() as u64);
        asm!("dsb ishst", "tlbi vmalle1", "dsb nsh", "isb", options(nostack));
        cpu::set_sctlr_el1(cpu::sctlr_el1() | SCTLR_M | SCTLR_C | SCTLR_I);
    }
    cpu::isb();
    Ok(())
}

/// Whether `SCTLR_EL1.M` is set.
pub fn enabled() -> bool {
    cpu::sctlr_el1() & SCTLR_M != 0
}
