//! The local APIC (SDM Vol. 3A chapter 11).
//!
//! In xAPIC mode the registers are a 4 KiB memory-mapped page (default
//! physical address 0xfee0_0000, see `IA32_APIC_BASE`), one 32-bit register
//! every 16 bytes (§11.4.1, table 11-1 "Local APIC Register Address Map"). In
//! x2APIC mode the same registers are MSRs 0x800 + offset / 16 (§11.12.1.2,
//! table 11-6), the ID is 32 bits wide and the interrupt command register is
//! one 64-bit MSR. [`ApicRegisters`] hides that difference, with [`XApic`]
//! over [`Mmio`] and [`X2Apic`] over [`Msr`]; [`LocalApic`] holds everything
//! else and is the same code in both modes.
//!
//! The local APIC is per CPU: every CPU programs its own, and in xAPIC mode
//! each CPU reaches its own through the same physical address.

use core::hint::spin_loop;
use core::sync::atomic::{Ordering, fence};

use crate::time::{Calibration, CalibrationConfig, CalibrationError, Counter, calibrate, mul_div};
use crate::{CpuFeatures, DeliveryMode, Mmio, Msr, Polarity, Trigger};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApicError {
    /// That LVT entry does not exist on this APIC (see [`Version::max_lvt`]).
    NotPresent,
    /// The CPU lacks the feature (x2APIC, TSC-deadline mode).
    Unsupported,
    /// Vectors 0 to 15 are reserved; fixed and lowest-priority delivery of
    /// them is an illegal-vector error (§11.5.2).
    InvalidVector,
    /// The destination does not fit the mode (8 bits in xAPIC mode).
    InvalidDestination,
    /// A delivery mode / shorthand combination the ICR does not allow.
    InvalidIpi,
    /// TSC-deadline mode is set with [`LocalApic::enable_tsc_deadline`], not
    /// with a count.
    InvalidTimerMode,
    /// The xAPIC delivery status bit stayed set.
    Timeout,
    /// `IA32_APIC_BASE` cannot go from x2APIC to xAPIC mode directly.
    IllegalTransition,
}

/// The version register (§11.4.8, figure 11-7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Version(pub u32);

impl Version {
    pub const fn version(self) -> u8 {
        self.0 as u8
    }

    /// Integrated APIC (0x1x) rather than an external 82489DX (0x0x).
    pub const fn is_integrated(self) -> bool {
        self.version() >= 0x10
    }

    /// Number of LVT entries minus one.
    pub const fn max_lvt(self) -> u8 {
        (self.0 >> 16) as u8
    }

    /// EOI-broadcast suppression can be enabled in the SVR.
    pub const fn eoi_broadcast_suppression(self) -> bool {
        self.0 & (1 << 24) != 0
    }
}

/// The local vector table entries (§11.5.1, figure 11-8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LvtReg {
    Timer,
    Cmci,
    Thermal,
    Perf,
    Lint0,
    Lint1,
    Error,
}

impl LvtReg {
    pub const ALL: [LvtReg; 7] =
        [LvtReg::Timer, LvtReg::Cmci, LvtReg::Thermal, LvtReg::Perf, LvtReg::Lint0, LvtReg::Lint1, LvtReg::Error];

    pub const fn offset(self) -> u16 {
        match self {
            LvtReg::Timer => reg::LVT_TIMER,
            LvtReg::Cmci => reg::LVT_CMCI,
            LvtReg::Thermal => reg::LVT_THERMAL,
            LvtReg::Perf => reg::LVT_PERF,
            LvtReg::Lint0 => reg::LVT_LINT0,
            LvtReg::Lint1 => reg::LVT_LINT1,
            LvtReg::Error => reg::LVT_ERROR,
        }
    }

    /// Whether an APIC with this version has the entry. The timer, LINT0,
    /// LINT1 and error entries always exist; the performance counter entry
    /// arrived with max LVT 4 (P6), thermal with 5 (Pentium 4) and CMCI with
    /// 6 (§11.5.1). In x2APIC mode touching a missing entry raises #GP.
    pub const fn is_present(self, version: Version) -> bool {
        match self {
            LvtReg::Perf => version.max_lvt() >= 4,
            LvtReg::Thermal => version.max_lvt() >= 5,
            LvtReg::Cmci => version.max_lvt() >= 6,
            _ => true,
        }
    }
}

/// APIC timer mode, LVT timer bits 18:17 (§11.5.4, figure 11-8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimerMode {
    OneShot,
    Periodic,
    TscDeadline,
}

/// One LVT entry. Fields an entry does not have (polarity on the timer,
/// timer mode on LINT0) are reserved and must stay 0 in that entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Lvt(u32);

impl Lvt {
    pub const VECTOR: u32 = 0xff;
    pub const DELIVERY_SHIFT: u32 = 8;
    /// Read-only: an interrupt is pending delivery.
    pub const DELIVERY_STATUS: u32 = 1 << 12;
    /// LINT only.
    pub const ACTIVE_LOW: u32 = 1 << 13;
    /// Read-only, LINT only, level-triggered only.
    pub const REMOTE_IRR: u32 = 1 << 14;
    /// LINT only.
    pub const LEVEL: u32 = 1 << 15;
    pub const MASKED: u32 = 1 << 16;
    pub const TIMER_MODE_SHIFT: u32 = 17;
    pub const READ_ONLY: u32 = Self::DELIVERY_STATUS | Self::REMOTE_IRR;

    /// Fixed delivery of `vector`, unmasked, edge-triggered, active high.
    pub const fn new(vector: u8) -> Lvt {
        Lvt(vector as u32)
    }

    /// The reset value: masked, everything else 0.
    pub const fn masked() -> Lvt {
        Lvt(Self::MASKED)
    }

    pub const fn from_bits(bits: u32) -> Lvt {
        Lvt(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn with_mask(self, masked: bool) -> Lvt {
        Lvt(if masked { self.0 | Self::MASKED } else { self.0 & !Self::MASKED })
    }

    pub const fn with_vector(self, vector: u8) -> Lvt {
        Lvt(self.0 & !Self::VECTOR | vector as u32)
    }

    pub const fn with_delivery(self, mode: DeliveryMode) -> Lvt {
        Lvt(self.0 & !(0b111 << Self::DELIVERY_SHIFT) | mode.bits() << Self::DELIVERY_SHIFT)
    }

    pub const fn with_polarity(self, polarity: Polarity) -> Lvt {
        match polarity {
            Polarity::ActiveHigh => Lvt(self.0 & !Self::ACTIVE_LOW),
            Polarity::ActiveLow => Lvt(self.0 | Self::ACTIVE_LOW),
        }
    }

    pub const fn with_trigger(self, trigger: Trigger) -> Lvt {
        match trigger {
            Trigger::Edge => Lvt(self.0 & !Self::LEVEL),
            Trigger::Level => Lvt(self.0 | Self::LEVEL),
        }
    }

    pub const fn with_timer_mode(self, mode: TimerMode) -> Lvt {
        let bits = match mode {
            TimerMode::OneShot => 0b00,
            TimerMode::Periodic => 0b01,
            TimerMode::TscDeadline => 0b10,
        };
        Lvt(self.0 & !(0b11 << Self::TIMER_MODE_SHIFT) | bits << Self::TIMER_MODE_SHIFT)
    }

    pub const fn vector(self) -> u8 {
        self.0 as u8
    }

    pub const fn is_masked(self) -> bool {
        self.0 & Self::MASKED != 0
    }

    pub const fn delivery(self) -> Option<DeliveryMode> {
        DeliveryMode::from_bits(self.0 >> Self::DELIVERY_SHIFT)
    }

    pub const fn polarity(self) -> Polarity {
        if self.0 & Self::ACTIVE_LOW != 0 { Polarity::ActiveLow } else { Polarity::ActiveHigh }
    }

    pub const fn trigger(self) -> Trigger {
        if self.0 & Self::LEVEL != 0 { Trigger::Level } else { Trigger::Edge }
    }

    /// `None` for the reserved encoding 0b11.
    pub const fn timer_mode(self) -> Option<TimerMode> {
        match (self.0 >> Self::TIMER_MODE_SHIFT) & 0b11 {
            0b00 => Some(TimerMode::OneShot),
            0b01 => Some(TimerMode::Periodic),
            0b10 => Some(TimerMode::TscDeadline),
            _ => None,
        }
    }

    pub const fn is_pending(self) -> bool {
        self.0 & Self::DELIVERY_STATUS != 0
    }

    pub const fn remote_irr(self) -> bool {
        self.0 & Self::REMOTE_IRR != 0
    }
}

/// APIC timer clock divider (§11.5.4, figure 11-10 "Divide Configuration
/// Register"): bits 0, 1 and 3, bit 2 is reserved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Divide {
    By1,
    By2,
    By4,
    By8,
    By16,
    By32,
    By64,
    By128,
}

impl Divide {
    pub const fn bits(self) -> u32 {
        match self {
            Divide::By2 => 0b0000,
            Divide::By4 => 0b0001,
            Divide::By8 => 0b0010,
            Divide::By16 => 0b0011,
            Divide::By32 => 0b1000,
            Divide::By64 => 0b1001,
            Divide::By128 => 0b1010,
            Divide::By1 => 0b1011,
        }
    }

    pub const fn from_bits(bits: u32) -> Divide {
        match bits & 0b1011 {
            0b0000 => Divide::By2,
            0b0001 => Divide::By4,
            0b0010 => Divide::By8,
            0b0011 => Divide::By16,
            0b1000 => Divide::By32,
            0b1001 => Divide::By64,
            0b1010 => Divide::By128,
            _ => Divide::By1,
        }
    }

    pub const fn divisor(self) -> u32 {
        match self {
            Divide::By1 => 1,
            Divide::By2 => 2,
            Divide::By4 => 4,
            Divide::By8 => 8,
            Divide::By16 => 16,
            Divide::By32 => 32,
            Divide::By64 => 64,
            Divide::By128 => 128,
        }
    }
}

/// The error status register (§11.5.3, figure 11-9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErrorStatus(pub u32);

impl ErrorStatus {
    pub const SEND_CHECKSUM: u32 = 1 << 0;
    pub const RECEIVE_CHECKSUM: u32 = 1 << 1;
    pub const SEND_ACCEPT: u32 = 1 << 2;
    pub const RECEIVE_ACCEPT: u32 = 1 << 3;
    pub const REDIRECTABLE_IPI: u32 = 1 << 4;
    pub const SEND_ILLEGAL_VECTOR: u32 = 1 << 5;
    pub const RECEIVE_ILLEGAL_VECTOR: u32 = 1 << 6;
    pub const ILLEGAL_REGISTER: u32 = 1 << 7;

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, bits: u32) -> bool {
        self.0 & bits == bits
    }
}

/// Where an IPI goes (§11.6.1, figure 11-12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Destination {
    /// One APIC by ID (0xff, or 0xffff_ffff in x2APIC mode, broadcasts).
    Physical(u32),
    /// The APICs selected by their logical destination registers.
    Logical(u32),
    /// Shorthand 01: this CPU.
    ToSelf,
    /// Shorthand 10.
    AllIncludingSelf,
    /// Shorthand 11.
    AllExcludingSelf,
}

/// An inter-processor interrupt: what goes into the ICR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ipi {
    pub vector: u8,
    pub delivery: DeliveryMode,
    pub destination: Destination,
    pub trigger: Trigger,
    /// Level: assert (true) or de-assert. Only the obsolete INIT level
    /// de-assert uses false.
    pub assert: bool,
}

impl Ipi {
    pub const DELIVERY_STATUS: u32 = 1 << 12;

    pub const fn fixed(destination: Destination, vector: u8) -> Ipi {
        Ipi { vector, delivery: DeliveryMode::Fixed, destination, trigger: Trigger::Edge, assert: true }
    }

    pub const fn nmi(destination: Destination) -> Ipi {
        Ipi { vector: 0, delivery: DeliveryMode::Nmi, destination, trigger: Trigger::Edge, assert: true }
    }

    /// INIT, the first step of starting an application processor (§10.4.4.1
    /// in Vol. 3A, "Typical BSP Initialization Sequence").
    pub const fn init(destination: Destination) -> Ipi {
        Ipi { vector: 0, delivery: DeliveryMode::Init, destination, trigger: Trigger::Edge, assert: true }
    }

    /// Start-up IPI: the target starts in real mode at `page * 4096`.
    pub const fn startup(destination: Destination, page: u8) -> Ipi {
        Ipi { vector: page, delivery: DeliveryMode::StartUp, destination, trigger: Trigger::Edge, assert: true }
    }

    /// Start-up IPI for a trampoline at `address`, which must be 4 KiB
    /// aligned and below 1 MiB.
    pub const fn startup_at(destination: Destination, address: u64) -> Option<Ipi> {
        if !address.is_multiple_of(4096) || address >= 0x10_0000 {
            return None;
        }
        Some(Ipi::startup(destination, (address >> 12) as u8))
    }

    /// The destination field and the low ICR dword.
    pub const fn encode(&self) -> (u32, u32) {
        let (dest, logical, shorthand) = match self.destination {
            Destination::Physical(id) => (id, 0, 0b00),
            Destination::Logical(mask) => (mask, 1, 0b00),
            Destination::ToSelf => (0, 0, 0b01),
            Destination::AllIncludingSelf => (0, 0, 0b10),
            Destination::AllExcludingSelf => (0, 0, 0b11),
        };
        let level = matches!(self.trigger, Trigger::Level) as u32;
        let low = self.vector as u32
            | self.delivery.bits() << 8
            | logical << 11
            | (self.assert as u32) << 14
            | level << 15
            | shorthand << 18;
        (dest, low)
    }
}

/// The local APIC of the CPU running this code.
#[derive(Debug)]
pub struct LocalApic<R> {
    regs: R,
    tsc_deadline: bool,
    icr_spins: u32,
}

impl<R: ApicRegisters> LocalApic<R> {
    /// Polls of the xAPIC delivery status bit before giving up on an IPI.
    pub const DEFAULT_ICR_SPINS: u32 = 1 << 20;

    /// SVR bit 8: APIC software enable.
    pub const SVR_ENABLE: u32 = 1 << 8;

    /// Wraps the registers. Changes nothing. `features` says whether the
    /// timer has TSC-deadline mode.
    pub fn new(regs: R, features: &CpuFeatures) -> LocalApic<R> {
        LocalApic { regs, tsc_deadline: features.tsc_deadline, icr_spins: Self::DEFAULT_ICR_SPINS }
    }

    pub fn registers(&self) -> &R {
        &self.regs
    }

    pub fn set_icr_spin_limit(&mut self, spins: u32) {
        self.icr_spins = spins;
    }

    pub fn is_x2apic(&self) -> bool {
        self.regs.is_x2apic()
    }

    /// The APIC ID: bits 31:24 of the ID register in xAPIC mode, all 32 bits
    /// in x2APIC mode (§11.4.6, §11.12.5.1).
    pub fn id(&self) -> u32 {
        let id = self.regs.read(reg::ID);
        if self.is_x2apic() { id } else { id >> 24 }
    }

    pub fn version(&self) -> Version {
        Version(self.regs.read(reg::VERSION))
    }

    /// Software-enables the APIC and sets the spurious interrupt vector
    /// (§11.9, figure 11-23). Other SVR bits (focus processor checking,
    /// EOI-broadcast suppression) are kept. On P6 and Pentium processors bits
    /// 3:0 of the vector are hardwired to 1, so 0xff is the portable choice.
    pub fn enable(&self, spurious_vector: u8) {
        let svr = self.regs.read(reg::SVR) & !0xff;
        self.regs.write(reg::SVR, svr | Self::SVR_ENABLE | spurious_vector as u32);
    }

    /// Software-disables the APIC; it masks every LVT entry (§11.4.7.2).
    pub fn disable(&self) {
        let svr = self.regs.read(reg::SVR);
        self.regs.write(reg::SVR, svr & !Self::SVR_ENABLE);
    }

    pub fn is_enabled(&self) -> bool {
        self.regs.read(reg::SVR) & Self::SVR_ENABLE != 0
    }

    pub fn spurious_vector(&self) -> u8 {
        self.regs.read(reg::SVR) as u8
    }

    /// Interrupts with a priority class (vector / 16) at or below
    /// `priority >> 4` are held back (§11.8.3.1).
    pub fn set_task_priority(&self, priority: u8) {
        self.regs.write(reg::TPR, priority as u32);
    }

    pub fn task_priority(&self) -> u8 {
        self.regs.read(reg::TPR) as u8
    }

    pub fn processor_priority(&self) -> u8 {
        self.regs.read(reg::PPR) as u8
    }

    /// Signals the end of the interrupt being serviced (§11.8.5). Not for
    /// the spurious vector, which is never in service. x2APIC mode requires
    /// the value 0.
    pub fn eoi(&self) {
        self.regs.write(reg::EOI, 0);
    }

    fn vector_bit(&self, base: u16, vector: u8) -> bool {
        let r = self.regs.read(base + 0x10 * (vector as u16 / 32));
        r & (1 << (vector % 32)) != 0
    }

    pub fn is_in_service(&self, vector: u8) -> bool {
        self.vector_bit(reg::ISR, vector)
    }

    pub fn is_requested(&self, vector: u8) -> bool {
        self.vector_bit(reg::IRR, vector)
    }

    pub fn lvt(&self, which: LvtReg) -> Result<Lvt, ApicError> {
        if !which.is_present(self.version()) {
            return Err(ApicError::NotPresent);
        }
        Ok(Lvt(self.regs.read(which.offset())))
    }

    /// Writes an LVT entry; its read-only bits are cleared first. An
    /// unmasked fixed entry needs a vector of 16 or more. While the APIC is
    /// software-disabled the hardware keeps every entry masked.
    pub fn set_lvt(&self, which: LvtReg, lvt: Lvt) -> Result<(), ApicError> {
        if !which.is_present(self.version()) {
            return Err(ApicError::NotPresent);
        }
        if !lvt.is_masked() && lvt.delivery() == Some(DeliveryMode::Fixed) && lvt.vector() < 16 {
            return Err(ApicError::InvalidVector);
        }
        self.regs.write(which.offset(), lvt.bits() & !Lvt::READ_ONLY);
        Ok(())
    }

    pub fn set_masked(&self, which: LvtReg, masked: bool) -> Result<(), ApicError> {
        let lvt = self.lvt(which)?;
        self.set_lvt(which, lvt.with_mask(masked))
    }

    /// Masks every LVT entry this APIC has.
    pub fn mask_all_lvts(&self) {
        let version = self.version();
        for which in LvtReg::ALL {
            if which.is_present(version) {
                let lvt = Lvt(self.regs.read(which.offset()));
                self.regs.write(which.offset(), lvt.with_mask(true).bits() & !Lvt::READ_ONLY);
            }
        }
    }

    /// Errors seen since the last call. The ESR is updated by a write and
    /// then read (§11.5.3); x2APIC mode requires the written value to be 0.
    pub fn error_status(&self) -> ErrorStatus {
        self.regs.write(reg::ESR, 0);
        ErrorStatus(self.regs.read(reg::ESR))
    }

    pub fn timer_divide(&self) -> Divide {
        Divide::from_bits(self.regs.read(reg::TIMER_DIVIDE))
    }

    pub fn set_timer_divide(&self, divide: Divide) {
        let keep = self.regs.read(reg::TIMER_DIVIDE) & !0b1011;
        self.regs.write(reg::TIMER_DIVIDE, keep | divide.bits());
    }

    /// Starts the timer counting down from `initial_count` at the bus /
    /// crystal clock divided by `divide`, interrupting at `vector` when it
    /// reaches 0, once or (periodic) every time (§11.5.4). The order follows
    /// the SDM: divider, LVT, then the initial count, which starts it.
    pub fn start_timer(
        &self,
        vector: u8,
        mode: TimerMode,
        divide: Divide,
        initial_count: u32,
    ) -> Result<(), ApicError> {
        if mode == TimerMode::TscDeadline {
            return Err(ApicError::InvalidTimerMode);
        }
        self.set_timer_divide(divide);
        self.set_lvt(LvtReg::Timer, Lvt::new(vector).with_timer_mode(mode))?;
        self.regs.write(reg::TIMER_INITIAL, initial_count);
        Ok(())
    }

    /// Stops the timer: masks it and writes an initial count of 0. In
    /// TSC-deadline mode, also write a deadline of 0.
    pub fn stop_timer(&self) {
        let lvt = Lvt(self.regs.read(reg::LVT_TIMER));
        self.regs.write(reg::LVT_TIMER, lvt.with_mask(true).bits() & !Lvt::READ_ONLY);
        self.regs.write(reg::TIMER_INITIAL, 0);
    }

    pub fn timer_initial_count(&self) -> u32 {
        self.regs.read(reg::TIMER_INITIAL)
    }

    pub fn timer_current_count(&self) -> u32 {
        self.regs.read(reg::TIMER_CURRENT)
    }

    /// Puts the timer in TSC-deadline mode at `vector` (§11.5.4.1), where it
    /// interrupts when the TSC reaches the value in `IA32_TSC_DEADLINE`.
    pub fn enable_tsc_deadline(&self, vector: u8) -> Result<(), ApicError> {
        if !self.tsc_deadline {
            return Err(ApicError::Unsupported);
        }
        self.set_lvt(LvtReg::Timer, Lvt::new(vector).with_timer_mode(TimerMode::TscDeadline))
    }

    /// Arms (or with 0 disarms) the TSC deadline. The MSR exists in both
    /// modes. The fence orders an earlier memory-mapped LVT write before the
    /// `wrmsr`, which §11.5.4.1 requires in xAPIC mode.
    pub fn set_tsc_deadline<S: Msr>(&self, msr: &S, deadline: u64) -> Result<(), ApicError> {
        if !self.tsc_deadline {
            return Err(ApicError::Unsupported);
        }
        fence(Ordering::SeqCst);
        msr.write(IA32_TSC_DEADLINE, deadline);
        Ok(())
    }

    fn wait_icr_idle(&self) -> Result<(), ApicError> {
        for _ in 0..self.icr_spins {
            if self.regs.read_icr_low() & Ipi::DELIVERY_STATUS == 0 {
                return Ok(());
            }
            spin_loop();
        }
        Err(ApicError::Timeout)
    }

    /// Sends an IPI (§11.6.1). In xAPIC mode, waits (bounded) for the
    /// previous IPI to leave and for this one to be accepted, using the
    /// delivery status bit; x2APIC mode has no such bit.
    ///
    /// INIT-SIPI-SIPI timing (10 ms after INIT, 200 us between start-ups)
    /// is the caller's job.
    pub fn send_ipi(&self, ipi: &Ipi) -> Result<(), ApicError> {
        match ipi.delivery {
            DeliveryMode::ExtInt => return Err(ApicError::InvalidIpi),
            DeliveryMode::Fixed | DeliveryMode::LowestPriority if ipi.vector < 16 => {
                return Err(ApicError::InvalidVector);
            }
            _ => {}
        }
        if ipi.destination == Destination::ToSelf && ipi.delivery != DeliveryMode::Fixed {
            return Err(ApicError::InvalidIpi);
        }
        let (dest, low) = ipi.encode();
        if self.is_x2apic() {
            self.regs.write_icr(dest, low);
            return Ok(());
        }
        if dest > 0xff {
            return Err(ApicError::InvalidDestination);
        }
        self.wait_icr_idle()?;
        self.regs.write_icr(dest, low);
        self.wait_icr_idle()
    }

    /// Measures the timer's rate at `divide` against `reference` (counting at
    /// `reference_hz`) and leaves the timer stopped. Run with interrupts
    /// disabled. The countdown starts at `u32::MAX`; if it reaches 0 before
    /// the measurements end, the result is [`CalibrationError::TargetStuck`]
    /// and a larger divider is needed.
    pub fn calibrate_timer<C: Counter>(
        &self,
        reference: &C,
        reference_hz: u64,
        divide: Divide,
        config: &CalibrationConfig,
    ) -> Result<Calibration, CalibrationError> {
        self.set_timer_divide(divide);
        let lvt = Lvt::masked().with_vector(0xff).with_timer_mode(TimerMode::OneShot);
        self.regs.write(reg::LVT_TIMER, lvt.bits());
        self.regs.write(reg::TIMER_INITIAL, u32::MAX);
        let result = calibrate(&ApicTimerCounter(self), reference, reference_hz, config);
        let expired = self.timer_current_count() == 0;
        self.stop_timer();
        if expired { Err(CalibrationError::TargetStuck) } else { result }
    }
}

/// The APIC timer counting down from `u32::MAX`, seen as a counter that
/// counts up.
#[derive(Debug)]
pub struct ApicTimerCounter<'a, R>(pub &'a LocalApic<R>);

impl<R: ApicRegisters> Counter for ApicTimerCounter<'_, R> {
    fn read(&self) -> u64 {
        !self.0.timer_current_count() as u64
    }

    fn bits(&self) -> u32 {
        32
    }
}

/// The initial count for a timer period of `period_ns` at `timer_hz`
/// (as measured by [`LocalApic::calibrate_timer`]). `None` if it rounds to
/// 0 or does not fit 32 bits.
pub const fn timer_count_for(timer_hz: u64, period_ns: u64) -> Option<u32> {
    match mul_div(period_ns, timer_hz, crate::time::NANOS_PER_SEC) {
        0 => None,
        n if n > u32::MAX as u64 => None,
        n => Some(n as u32),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::sim::{SimApic, SimClock, SimHpet};
    use std::vec::Vec;

    const ALL: CpuFeatures =
        CpuFeatures { tsc: true, apic: true, x2apic: true, tsc_deadline: true, arat: true, invariant_tsc: true };

    #[test]
    fn x2apic_msr_numbers() {
        assert_eq!(x2apic_msr(reg::ID), 0x802);
        assert_eq!(x2apic_msr(reg::EOI), 0x80b);
        assert_eq!(x2apic_msr(reg::SVR), 0x80f);
        assert_eq!(x2apic_msr(reg::ESR), 0x828);
        assert_eq!(x2apic_msr(reg::ICR_LOW), 0x830);
        assert_eq!(x2apic_msr(reg::LVT_TIMER), 0x832);
        assert_eq!(x2apic_msr(reg::TIMER_DIVIDE), 0x83e);
        assert_eq!(x2apic_msr(reg::SELF_IPI), 0x83f);
    }

    #[test]
    fn lvt_encoding_round_trips() {
        let l = Lvt::new(0x40)
            .with_delivery(DeliveryMode::Nmi)
            .with_polarity(Polarity::ActiveLow)
            .with_trigger(Trigger::Level)
            .with_mask(true);
        assert_eq!(l.bits(), 0x40 | 0b100 << 8 | 1 << 13 | 1 << 15 | 1 << 16);
        assert_eq!(l.vector(), 0x40);
        assert_eq!(l.delivery(), Some(DeliveryMode::Nmi));
        assert_eq!(l.polarity(), Polarity::ActiveLow);
        assert_eq!(l.trigger(), Trigger::Level);
        assert!(l.is_masked());
        let l = l.with_mask(false).with_polarity(Polarity::ActiveHigh).with_trigger(Trigger::Edge);
        assert_eq!(l.bits(), 0x40 | 0b100 << 8);
        for mode in [TimerMode::OneShot, TimerMode::Periodic, TimerMode::TscDeadline] {
            assert_eq!(Lvt::new(0x20).with_timer_mode(mode).timer_mode(), Some(mode));
        }
        assert_eq!(Lvt::from_bits(0b11 << 17).timer_mode(), None);
        assert_eq!(Lvt::from_bits(0b011 << 8).delivery(), None);
        assert!(Lvt::from_bits(1 << 12).is_pending());
        assert!(Lvt::from_bits(1 << 14).remote_irr());
        assert_eq!(Lvt::new(1).with_vector(0xfe).vector(), 0xfe);
    }

    #[test]
    fn divide_encoding_round_trips() {
        let all = [
            Divide::By1,
            Divide::By2,
            Divide::By4,
            Divide::By8,
            Divide::By16,
            Divide::By32,
            Divide::By64,
            Divide::By128,
        ];
        for (i, d) in all.into_iter().enumerate() {
            assert_eq!(Divide::from_bits(d.bits()), d);
            assert_eq!(d.divisor(), 1 << i);
            assert_eq!(d.bits() & 0b0100, 0);
        }
        // Bit 2 is reserved and ignored.
        assert_eq!(Divide::from_bits(0b0111), Divide::By16);
    }

    #[test]
    fn ipi_encoding() {
        // The SDM's broadcast INIT and SIPI values (Vol. 3A §10.4.4.1).
        assert_eq!(Ipi::init(Destination::AllExcludingSelf).encode(), (0, 0x000c_4500));
        assert_eq!(Ipi::startup(Destination::AllExcludingSelf, 0x08).encode(), (0, 0x000c_4608));
        assert_eq!(Ipi::fixed(Destination::Physical(3), 0x30).encode(), (3, 0x4030));
        assert_eq!(Ipi::fixed(Destination::Logical(0b101), 0x31).encode(), (0b101, 0x4831));
        assert_eq!(Ipi::fixed(Destination::ToSelf, 0x32).encode(), (0, 0x4_4032));
        assert_eq!(Ipi::fixed(Destination::AllIncludingSelf, 0x33).encode(), (0, 0x8_4033));
        assert_eq!(Ipi::nmi(Destination::Physical(1)).encode(), (1, 0x4400));
        let deassert = Ipi { assert: false, trigger: Trigger::Level, ..Ipi::init(Destination::Physical(2)) };
        assert_eq!(deassert.encode(), (2, 0x8500));
        assert_eq!(Ipi::startup_at(Destination::Physical(1), 0x8000), Some(Ipi::startup(Destination::Physical(1), 8)));
        assert_eq!(Ipi::startup_at(Destination::Physical(1), 0x8001), None);
        assert_eq!(Ipi::startup_at(Destination::Physical(1), 0x10_0000), None);
    }

    #[test]
    fn apic_base_transitions() {
        let clock = SimClock::new(1);
        let sim = SimApic::new(&clock, 0, 0x0005_0014);
        let base = read_apic_base(&&sim);
        assert!(base.is_bsp() && base.is_enabled() && !base.is_x2apic());
        assert_eq!(base.address(), 0xfee0_0000);
        assert_eq!(enable_x2apic(&&sim, &CpuFeatures::default()), Err(ApicError::Unsupported));
        let base = enable_x2apic(&&sim, &ALL).unwrap();
        assert!(base.is_x2apic() && base.is_enabled());
        assert_eq!(enable_xapic(&&sim), Err(ApicError::IllegalTransition));
        // Idempotent once in x2APIC mode.
        assert_eq!(enable_x2apic(&&sim, &ALL), Ok(base));

        // From globally disabled: EN first, then EXTD (the sim rejects
        // EXTD without EN).
        let sim = SimApic::new(&clock, 0, 0x0005_0014);
        sim.set_apic_base(0xfee0_0100);
        assert!(!read_apic_base(&&sim).is_enabled());
        assert!(enable_xapic(&&sim).unwrap().is_enabled());
        let sim = SimApic::new(&clock, 0, 0x0005_0014);
        sim.set_apic_base(0xfee0_0100);
        assert!(enable_x2apic(&&sim, &ALL).unwrap().is_x2apic());
    }

    /// Everything a driver can observe, collected while exercising most of
    /// the interface.
    fn scenario<R: ApicRegisters>(apic: &LocalApic<R>) -> Vec<u64> {
        let mut seen = Vec::new();
        apic.mask_all_lvts();
        apic.enable(0xff);
        apic.set_task_priority(0);
        seen.push(apic.id() as u64);
        seen.push(apic.version().0 as u64);
        seen.push(apic.is_enabled() as u64);
        seen.push(apic.spurious_vector() as u64);
        apic.set_lvt(LvtReg::Error, Lvt::new(0xfe)).unwrap();
        apic.set_lvt(LvtReg::Lint0, Lvt::masked().with_delivery(DeliveryMode::ExtInt)).unwrap();
        apic.set_lvt(LvtReg::Lint1, Lvt::new(0).with_delivery(DeliveryMode::Nmi)).unwrap();
        seen.push(apic.error_status().0 as u64);
        apic.start_timer(0x40, TimerMode::Periodic, Divide::By16, 100_000).unwrap();
        seen.push(apic.timer_divide().divisor() as u64);
        seen.push(apic.timer_initial_count() as u64);
        apic.send_ipi(&Ipi::fixed(Destination::Physical(1), 0x50)).unwrap();
        apic.send_ipi(&Ipi::init(Destination::AllExcludingSelf)).unwrap();
        apic.send_ipi(&Ipi::startup(Destination::AllExcludingSelf, 8)).unwrap();
        // An illegal vector caught by the hardware rather than by us.
        apic.registers().write_icr(1, 0x4005);
        seen.push(apic.error_status().0 as u64);
        seen.push(apic.error_status().0 as u64);
        apic.eoi();
        apic.set_task_priority(0x20);
        seen.push(apic.task_priority() as u64);
        seen.push(apic.processor_priority() as u64);
        for which in LvtReg::ALL {
            seen.push(apic.lvt(which).map_or(u64::MAX, |l| l.bits() as u64));
        }
        apic.stop_timer();
        seen.push(apic.timer_initial_count() as u64);
        seen
    }

    #[test]
    fn xapic_and_x2apic_behave_the_same() {
        let clock = SimClock::new(1);
        for version in [0x0004_0011, 0x0005_0014, 0x0106_0015] {
            let (x, x2) = (SimApic::new(&clock, 3, version), SimApic::new(&clock, 3, version));
            enable_x2apic(&&x2, &ALL).unwrap();
            let a = LocalApic::new(XApic(&x), &ALL);
            let b = LocalApic::new(X2Apic(&x2), &ALL);
            assert!(!a.is_x2apic() && b.is_x2apic());
            let sa = scenario(&a);
            let sb = scenario(&b);
            assert_eq!(sa, sb, "version {version:#x}");
            assert_eq!(sa[0], 3);
            assert_eq!(x.register_writes(), x2.register_writes());
            assert_eq!(x.ipis(), x2.ipis());
            assert_eq!(x.ipis().len(), 4);
            assert_eq!(x.eois(), 1);
            assert_eq!(x2.eois(), 1);
            // The illegal-vector IPI was reported once, then cleared.
            assert_eq!(sa[4], 0);
            assert_eq!(sa[7], ErrorStatus::SEND_ILLEGAL_VECTOR as u64);
            assert_eq!(sa[8], 0);
        }
    }

    #[test]
    fn mask_all_skips_missing_entries() {
        let clock = SimClock::new(1);
        // Max LVT 3: no perf, thermal or CMCI entry. The sim panics on any
        // access to one, like x2APIC hardware raising #GP.
        let sim = SimApic::new(&clock, 0, 0x0003_0014);
        enable_x2apic(&&sim, &ALL).unwrap();
        let apic = LocalApic::new(X2Apic(&sim), &ALL);
        apic.mask_all_lvts();
        assert_eq!(apic.lvt(LvtReg::Thermal), Err(ApicError::NotPresent));
        assert_eq!(apic.set_lvt(LvtReg::Cmci, Lvt::masked()), Err(ApicError::NotPresent));
        assert_eq!(apic.set_masked(LvtReg::Perf, true), Err(ApicError::NotPresent));
        assert!(apic.lvt(LvtReg::Lint0).unwrap().is_masked());
        let v = Version(0x0105_0014);
        assert!(v.is_integrated() && v.eoi_broadcast_suppression() && v.max_lvt() == 5 && v.version() == 0x14);
        assert!(LvtReg::Thermal.is_present(v) && !LvtReg::Cmci.is_present(v));
    }

    #[test]
    fn lvt_masks_stick_while_software_disabled() {
        let clock = SimClock::new(1);
        let sim = SimApic::new(&clock, 0, 0x0005_0014);
        let apic = LocalApic::new(XApic(&sim), &ALL);
        assert!(!apic.is_enabled());
        apic.set_lvt(LvtReg::Lint1, Lvt::new(0x22)).unwrap();
        assert!(apic.lvt(LvtReg::Lint1).unwrap().is_masked());
        apic.enable(0xff);
        apic.set_masked(LvtReg::Lint1, false).unwrap();
        assert!(!apic.lvt(LvtReg::Lint1).unwrap().is_masked());
        apic.disable();
        assert!(!apic.is_enabled());
        assert!(apic.lvt(LvtReg::Lint1).unwrap().is_masked());
    }

    #[test]
    fn read_only_bits_are_not_written() {
        let clock = SimClock::new(1);
        let sim = SimApic::new(&clock, 0, 0x0005_0014);
        let apic = LocalApic::new(XApic(&sim), &ALL);
        apic.enable(0xff);
        // A level-triggered LINT0 with remote IRR and delivery status set by
        // the hardware.
        sim.poke(reg::LVT_LINT0, 0x0000_d030 | 1 << 16);
        let lvt = apic.lvt(LvtReg::Lint0).unwrap();
        assert!(lvt.remote_irr() && lvt.is_pending());
        apic.set_masked(LvtReg::Lint0, false).unwrap();
        assert_eq!(sim.last_write(reg::LVT_LINT0), Some(0x0000_8030));
        // The hardware's bits are still there; ours changed.
        let lvt = apic.lvt(LvtReg::Lint0).unwrap();
        assert!(lvt.remote_irr() && !lvt.is_masked());
        // Writes to read-only registers are ignored in xAPIC mode.
        apic.registers().write(reg::VERSION, 0);
        assert_eq!(apic.version().0, 0x0005_0014);
    }

    #[test]
    fn rejects_bad_vectors_and_destinations() {
        let clock = SimClock::new(1);
        let sim = SimApic::new(&clock, 0, 0x0005_0014);
        let apic = LocalApic::new(XApic(&sim), &ALL);
        apic.enable(0xff);
        assert_eq!(apic.set_lvt(LvtReg::Timer, Lvt::new(15)), Err(ApicError::InvalidVector));
        assert_eq!(apic.set_lvt(LvtReg::Timer, Lvt::new(15).with_mask(true)), Ok(()));
        assert_eq!(apic.set_lvt(LvtReg::Lint1, Lvt::new(0).with_delivery(DeliveryMode::Nmi)), Ok(()));
        assert_eq!(apic.send_ipi(&Ipi::fixed(Destination::Physical(1), 2)), Err(ApicError::InvalidVector));
        assert_eq!(apic.send_ipi(&Ipi::fixed(Destination::Physical(256), 0x40)), Err(ApicError::InvalidDestination));
        assert_eq!(apic.send_ipi(&Ipi::nmi(Destination::ToSelf)), Err(ApicError::InvalidIpi));
        let extint = Ipi { delivery: DeliveryMode::ExtInt, ..Ipi::fixed(Destination::Physical(1), 0x40) };
        assert_eq!(apic.send_ipi(&extint), Err(ApicError::InvalidIpi));
        assert!(sim.ipis().is_empty());
        assert_eq!(apic.start_timer(0x40, TimerMode::TscDeadline, Divide::By1, 1), Err(ApicError::InvalidTimerMode));

        // x2APIC IDs are 32 bits.
        let sim = SimApic::new(&clock, 0, 0x0005_0014);
        enable_x2apic(&&sim, &ALL).unwrap();
        let apic = LocalApic::new(X2Apic(&sim), &ALL);
        apic.send_ipi(&Ipi::fixed(Destination::Physical(0x1_0000), 0x40)).unwrap();
        assert_eq!(sim.ipis(), [(0x1_0000, 0x4040)]);
    }

    #[test]
    fn xapic_ipis_poll_delivery_status() {
        let clock = SimClock::new(1);
        let sim = SimApic::new(&clock, 0, 0x0005_0014);
        let mut apic = LocalApic::new(XApic(&sim), &ALL);
        sim.set_busy_reads(5);
        apic.send_ipi(&Ipi::fixed(Destination::Physical(1), 0x40)).unwrap();
        // The second send waited for the first to drain.
        apic.send_ipi(&Ipi::fixed(Destination::Physical(2), 0x41)).unwrap();
        assert!(sim.icr_reads() >= 10);
        assert_eq!(sim.ipis(), [(1, 0x4040), (2, 0x4041)]);
        // A delivery that never completes times out instead of hanging.
        sim.set_busy_reads(u32::MAX);
        apic.set_icr_spin_limit(100);
        assert_eq!(apic.send_ipi(&Ipi::fixed(Destination::Physical(3), 0x42)), Err(ApicError::Timeout));
        assert_eq!(apic.send_ipi(&Ipi::fixed(Destination::Physical(4), 0x43)), Err(ApicError::Timeout));
        assert_eq!(sim.ipis().len(), 3, "nothing is sent while the previous IPI is still pending");
    }

    #[test]
    fn isr_and_irr_bits() {
        let clock = SimClock::new(1);
        let sim = SimApic::new(&clock, 0, 0x0005_0014);
        let apic = LocalApic::new(XApic(&sim), &ALL);
        sim.poke(reg::ISR + 0x10 * 2, 1 << 1); // vector 65
        sim.poke(reg::IRR + 0x10 * 7, 1 << 31); // vector 255
        assert!(apic.is_in_service(65) && !apic.is_in_service(64));
        assert!(apic.is_requested(255) && !apic.is_requested(65));
    }

    #[test]
    fn timer_one_shot_and_periodic() {
        let clock = SimClock::new(1);
        let sim = SimApic::new(&clock, 0, 0x0005_0014);
        let apic = LocalApic::new(XApic(&sim), &ALL);
        apic.enable(0xff);
        // Sim bus clock: 1 GHz. Divide by 4: 250 MHz, 4 ns per tick.
        apic.start_timer(0x40, TimerMode::OneShot, Divide::By4, 1000).unwrap();
        assert_eq!(apic.timer_current_count(), 1000);
        clock.advance_ps(400_000); // 400 ns: 100 ticks
        assert_eq!(apic.timer_current_count(), 900);
        clock.advance_ps(10_000_000);
        assert_eq!(apic.timer_current_count(), 0);
        let lvt = apic.lvt(LvtReg::Timer).unwrap();
        assert_eq!((lvt.vector(), lvt.timer_mode(), lvt.is_masked()), (0x40, Some(TimerMode::OneShot), false));
        apic.start_timer(0x41, TimerMode::Periodic, Divide::By1, 1000).unwrap();
        clock.advance_ps(2_500_000); // 2500 ticks: 2.5 periods
        assert_eq!(apic.timer_current_count(), 500);
        // The current count register is read-only.
        apic.registers().write(reg::TIMER_CURRENT, 7);
        assert_eq!(apic.timer_current_count(), 500);
        apic.stop_timer();
        assert_eq!(apic.timer_current_count(), 0);
        assert!(apic.lvt(LvtReg::Timer).unwrap().is_masked());
    }

    #[test]
    fn tsc_deadline_mode() {
        let clock = SimClock::new(1);
        let sim = SimApic::new(&clock, 0, 0x0005_0014);
        let none = LocalApic::new(XApic(&sim), &CpuFeatures::default());
        assert_eq!(none.enable_tsc_deadline(0x40), Err(ApicError::Unsupported));
        assert_eq!(none.set_tsc_deadline(&&sim, 5), Err(ApicError::Unsupported));
        let apic = LocalApic::new(XApic(&sim), &ALL);
        apic.enable(0xff);
        apic.enable_tsc_deadline(0x40).unwrap();
        assert_eq!(apic.lvt(LvtReg::Timer).unwrap().timer_mode(), Some(TimerMode::TscDeadline));
        apic.set_tsc_deadline(&&sim, 123_456).unwrap();
        assert_eq!(sim.tsc_deadline(), 123_456);
        apic.set_tsc_deadline(&&sim, 0).unwrap();
        assert_eq!(sim.tsc_deadline(), 0);
    }

    #[test]
    fn calibrates_the_timer_against_the_hpet() {
        let clock = SimClock::new(5).read_cost(300_000, 400_000);
        let sim = SimApic::new(&clock, 0, 0x0005_0014);
        let hpet_sim = SimHpet::new(&clock, true);
        let hpet = crate::Hpet::new(&hpet_sim).unwrap();
        hpet.enable();
        let hz = hpet.capabilities().frequency_hz();
        for (backend_x2, divide) in [(false, Divide::By16), (true, Divide::By1)] {
            if backend_x2 {
                enable_x2apic(&&sim, &ALL).unwrap();
            }
            let backend: Backend<&SimApic, &SimApic> =
                if backend_x2 { Backend::X2Apic(X2Apic(&sim)) } else { Backend::XApic(XApic(&sim)) };
            let apic = LocalApic::new(backend, &ALL);
            assert_eq!(apic.is_x2apic(), backend_x2);
            let cal = apic.calibrate_timer(&hpet, hz, divide, &CalibrationConfig::for_reference(hz)).unwrap();
            let expected = 1_000_000_000 / divide.divisor() as u64;
            let err_ppm = mul_div(cal.hz.abs_diff(expected), 1_000_000, expected);
            assert!(err_ppm < 100, "{divide:?}: {cal:?}");
            assert_eq!(apic.timer_initial_count(), 0);
            assert!(apic.lvt(LvtReg::Timer).unwrap().is_masked());
            assert_eq!(timer_count_for(cal.hz, 1_000_000), Some((cal.hz / 1000) as u32));
        }
    }

    #[test]
    fn timer_calibration_detects_expiry() {
        // A 1 GHz timer at divide 1 runs out after 4.3 s; ask for 6 x 1 s.
        let clock = SimClock::new(5).read_cost(1_000_000, 0);
        let sim = SimApic::new(&clock, 0, 0x0005_0014);
        let hpet_sim = SimHpet::new(&clock, true);
        let hpet = crate::Hpet::new(&hpet_sim).unwrap();
        hpet.enable();
        let hz = hpet.capabilities().frequency_hz();
        let apic = LocalApic::new(XApic(&sim), &ALL);
        let config = CalibrationConfig { window: hz, max_spins: u64::MAX, ..CalibrationConfig::for_reference(hz) };
        assert_eq!(apic.calibrate_timer(&hpet, hz, Divide::By1, &config), Err(CalibrationError::TargetStuck));
    }

    #[test]
    fn timer_counts() {
        assert_eq!(timer_count_for(1_000_000_000, 1_000_000), Some(1_000_000));
        assert_eq!(timer_count_for(62_500_000, 1), None);
        assert_eq!(timer_count_for(1_000_000_000, 5_000_000_000), None);
        assert_eq!(timer_count_for(u64::MAX, u64::MAX), None);
    }
}
