//! Register offsets, the MSR numbers, IA32_APIC_BASE and the xAPIC / x2APIC register back ends.

use super::*;

/// Register offsets in the xAPIC page (table 11-1). The x2APIC MSR for each
/// is [`x2apic_msr`]`(offset)`.
pub mod reg {
    pub const ID: u16 = 0x020;
    pub const VERSION: u16 = 0x030;
    /// Task priority.
    pub const TPR: u16 = 0x080;
    /// Arbitration priority (xAPIC only).
    pub const APR: u16 = 0x090;
    /// Processor priority (read-only).
    pub const PPR: u16 = 0x0a0;
    /// End of interrupt (write-only).
    pub const EOI: u16 = 0x0b0;
    /// Logical destination (read-only in x2APIC mode).
    pub const LDR: u16 = 0x0d0;
    /// Destination format (xAPIC only).
    pub const DFR: u16 = 0x0e0;
    /// Spurious interrupt vector.
    pub const SVR: u16 = 0x0f0;
    /// In-service, trigger mode and interrupt request registers: eight
    /// 32-bit registers each, 16 bytes apart, covering vectors 0 to 255.
    pub const ISR: u16 = 0x100;
    pub const TMR: u16 = 0x180;
    pub const IRR: u16 = 0x200;
    /// Error status.
    pub const ESR: u16 = 0x280;
    pub const LVT_CMCI: u16 = 0x2f0;
    /// Interrupt command, low and high halves (one MSR in x2APIC mode).
    pub const ICR_LOW: u16 = 0x300;
    pub const ICR_HIGH: u16 = 0x310;
    pub const LVT_TIMER: u16 = 0x320;
    pub const LVT_THERMAL: u16 = 0x330;
    pub const LVT_PERF: u16 = 0x340;
    pub const LVT_LINT0: u16 = 0x350;
    pub const LVT_LINT1: u16 = 0x360;
    pub const LVT_ERROR: u16 = 0x370;
    pub const TIMER_INITIAL: u16 = 0x380;
    /// Read-only.
    pub const TIMER_CURRENT: u16 = 0x390;
    pub const TIMER_DIVIDE: u16 = 0x3e0;
    /// Self IPI (x2APIC only, write-only).
    pub const SELF_IPI: u16 = 0x3f0;
}

/// `IA32_APIC_BASE` (§11.4.4, figure 11-5; §11.12.1 for the x2APIC bit).
pub const IA32_APIC_BASE: u32 = 0x1b;
/// `IA32_TSC_DEADLINE` (§11.5.4.1).
pub const IA32_TSC_DEADLINE: u32 = 0x6e0;
pub const X2APIC_MSR_BASE: u32 = 0x800;

/// The x2APIC MSR holding the register at xAPIC offset `offset`.
pub const fn x2apic_msr(offset: u16) -> u32 {
    X2APIC_MSR_BASE + (offset as u32 >> 4)
}

/// The value of `IA32_APIC_BASE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApicBase(pub u64);

impl ApicBase {
    pub const BSP: u64 = 1 << 8;
    /// EXTD: x2APIC mode.
    pub const X2APIC: u64 = 1 << 10;
    /// EN: the APIC is globally enabled.
    pub const ENABLE: u64 = 1 << 11;
    /// Base address bits 12 up to MAXPHYADDR (at most 52).
    pub const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

    pub const fn is_bsp(self) -> bool {
        self.0 & Self::BSP != 0
    }

    pub const fn is_enabled(self) -> bool {
        self.0 & Self::ENABLE != 0
    }

    pub const fn is_x2apic(self) -> bool {
        self.0 & Self::X2APIC != 0
    }

    /// Physical address of the xAPIC register page.
    pub const fn address(self) -> u64 {
        self.0 & Self::ADDRESS_MASK
    }
}

pub fn read_apic_base<S: Msr>(msr: &S) -> ApicBase {
    ApicBase(msr.read(IA32_APIC_BASE))
}

/// Globally enables the local APIC in xAPIC mode. Leaving x2APIC mode for
/// xAPIC mode directly is illegal (§11.12.5, figure 11-27), so that is an
/// error.
pub fn enable_xapic<S: Msr>(msr: &S) -> Result<ApicBase, ApicError> {
    let base = read_apic_base(msr);
    if base.is_x2apic() {
        return Err(ApicError::IllegalTransition);
    }
    if !base.is_enabled() {
        msr.write(IA32_APIC_BASE, base.0 | ApicBase::ENABLE);
    }
    Ok(read_apic_base(msr))
}

/// Switches the local APIC to x2APIC mode: first globally enabled (xAPIC),
/// then EXTD, the two legal steps of figure 11-27.
pub fn enable_x2apic<S: Msr>(msr: &S, features: &CpuFeatures) -> Result<ApicBase, ApicError> {
    if !features.x2apic {
        return Err(ApicError::Unsupported);
    }
    let mut base = read_apic_base(msr);
    if !base.is_enabled() {
        msr.write(IA32_APIC_BASE, base.0 | ApicBase::ENABLE);
        base = read_apic_base(msr);
    }
    if !base.is_x2apic() {
        msr.write(IA32_APIC_BASE, base.0 | ApicBase::ENABLE | ApicBase::X2APIC);
    }
    Ok(read_apic_base(msr))
}

/// Raw register access in either mode. `reg` is always the xAPIC offset
/// (see [`reg`]); the interrupt command register goes through its own
/// methods because its layout differs between the modes.
pub trait ApicRegisters {
    fn read(&self, reg: u16) -> u32;
    fn write(&self, reg: u16, value: u32);
    /// Writes the destination and then the low dword; writing the low dword
    /// is what sends the IPI. xAPIC destinations are 8 bits (the caller has
    /// checked).
    fn write_icr(&self, destination: u32, low: u32);
    fn read_icr_low(&self) -> u32;
    fn is_x2apic(&self) -> bool;
}

/// xAPIC mode: the memory-mapped page. Every register is accessed as an
/// aligned 32-bit word (§11.4.1).
#[derive(Debug)]
pub struct XApic<M>(pub M);

impl<M: Mmio> ApicRegisters for XApic<M> {
    fn read(&self, reg: u16) -> u32 {
        self.0.read32(reg as usize)
    }

    fn write(&self, reg: u16, value: u32) {
        self.0.write32(reg as usize, value)
    }

    fn write_icr(&self, destination: u32, low: u32) {
        self.0.write32(reg::ICR_HIGH as usize, destination << 24);
        self.0.write32(reg::ICR_LOW as usize, low);
    }

    fn read_icr_low(&self) -> u32 {
        self.0.read32(reg::ICR_LOW as usize)
    }

    fn is_x2apic(&self) -> bool {
        false
    }
}

/// x2APIC mode: MSRs 0x800 to 0x8ff (§11.12.1.2).
#[derive(Debug)]
pub struct X2Apic<S>(pub S);

impl<S: Msr> ApicRegisters for X2Apic<S> {
    fn read(&self, reg: u16) -> u32 {
        self.0.read(x2apic_msr(reg)) as u32
    }

    fn write(&self, reg: u16, value: u32) {
        self.0.write(x2apic_msr(reg), value as u64)
    }

    fn write_icr(&self, destination: u32, low: u32) {
        self.0.write(x2apic_msr(reg::ICR_LOW), (destination as u64) << 32 | low as u64)
    }

    fn read_icr_low(&self) -> u32 {
        self.0.read(x2apic_msr(reg::ICR_LOW)) as u32
    }

    fn is_x2apic(&self) -> bool {
        true
    }
}

/// Either mode, chosen at run time.
#[derive(Debug)]
pub enum Backend<M, S> {
    XApic(XApic<M>),
    X2Apic(X2Apic<S>),
}

impl<M: Mmio, S: Msr> ApicRegisters for Backend<M, S> {
    fn read(&self, reg: u16) -> u32 {
        match self {
            Backend::XApic(a) => a.read(reg),
            Backend::X2Apic(a) => a.read(reg),
        }
    }

    fn write(&self, reg: u16, value: u32) {
        match self {
            Backend::XApic(a) => a.write(reg, value),
            Backend::X2Apic(a) => a.write(reg, value),
        }
    }

    fn write_icr(&self, destination: u32, low: u32) {
        match self {
            Backend::XApic(a) => a.write_icr(destination, low),
            Backend::X2Apic(a) => a.write_icr(destination, low),
        }
    }

    fn read_icr_low(&self) -> u32 {
        match self {
            Backend::XApic(a) => a.read_icr_low(),
            Backend::X2Apic(a) => a.read_icr_low(),
        }
    }

    fn is_x2apic(&self) -> bool {
        matches!(self, Backend::X2Apic(_))
    }
}
