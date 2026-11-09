//! System registers and barriers: thin wrappers over `mrs`/`msr`/`isb`.

use core::arch::asm;

macro_rules! read_sysreg {
    ($(#[$doc:meta])* $name:ident, $reg:literal) => {
        $(#[$doc])*
        pub fn $name() -> u64 {
            let value: u64;
            // SAFETY: reading a system register has no side effect and
            // touches no memory; the register exists at EL1 on every
            // Armv8-A core.
            unsafe { asm!(concat!("mrs {}, ", $reg), out(reg) value, options(nomem, nostack, preserves_flags)) };
            value
        }
    };
}

macro_rules! write_sysreg {
    ($(#[$doc:meta])* $name:ident, $reg:literal) => {
        $(#[$doc])*
        ///
        /// # Safety
        /// The new value must keep the CPU in a state the rest of the stub
        /// expects (the caller says why in its own safety comment).
        pub unsafe fn $name(value: u64) {
            // SAFETY: the caller upholds this function's contract; `msr` to
            // an EL1 register is legal at EL1.
            unsafe { asm!(concat!("msr ", $reg, ", {}"), in(reg) value, options(nostack, preserves_flags)) };
        }
    };
}

read_sysreg!(
    /// `CurrentEL`: the exception level in bits 3:2.
    current_el, "CurrentEL"
);
read_sysreg!(
    /// `MIDR_EL1`: which CPU this is.
    midr_el1, "midr_el1"
);
read_sysreg!(
    /// `CNTFRQ_EL0`: the generic timer's frequency in Hz.
    cntfrq_el0, "cntfrq_el0"
);
read_sysreg!(
    /// `ID_AA64MMFR0_EL1`: memory model features (physical address range).
    id_aa64mmfr0_el1, "id_aa64mmfr0_el1"
);
read_sysreg!(
    /// `VBAR_EL1`: where the exception vector table is.
    vbar_el1, "vbar_el1"
);
read_sysreg!(
    /// `SCTLR_EL1`: system control (MMU, caches).
    sctlr_el1, "sctlr_el1"
);
read_sysreg!(
    /// `MPIDR_EL1`: which CPU of the machine this is.
    mpidr_el1, "mpidr_el1"
);

write_sysreg!(
    /// `SCTLR_EL1`.
    set_sctlr_el1, "sctlr_el1"
);
write_sysreg!(
    /// `TTBR0_EL1`: the base of the translation table for low addresses.
    set_ttbr0_el1, "ttbr0_el1"
);
write_sysreg!(
    /// `TCR_EL1`: translation control.
    set_tcr_el1, "tcr_el1"
);
write_sysreg!(
    /// `MAIR_EL1`: memory attribute encodings.
    set_mair_el1, "mair_el1"
);

/// `CNTPCT_EL0`, the physical counter, after an `isb` so the read is not
/// done early (Arm ARM, "Generic Timer": the counter read may be reordered
/// without it).
pub fn counter() -> u64 {
    let value: u64;
    // SAFETY: `isb` and reading the physical counter have no side effect;
    // EL1 may read it (CNTHCTL_EL2.EL1PCTEN is set before entering EL1).
    unsafe { asm!("isb", "mrs {}, cntpct_el0", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

/// Instruction synchronisation barrier.
pub fn isb() {
    // SAFETY: a barrier has no effect on memory or registers.
    unsafe { asm!("isb", options(nomem, nostack, preserves_flags)) };
}

/// Wait for an event; the CPU's idle loop.
pub fn wfe() {
    // SAFETY: `wfe` only pauses the CPU until an event or interrupt.
    unsafe { asm!("wfe", options(nomem, nostack, preserves_flags)) };
}

/// Stop this CPU for good.
pub fn park() -> ! {
    loop {
        wfe();
    }
}
