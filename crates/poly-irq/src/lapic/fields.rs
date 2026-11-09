//! The register types: version, LVT entries, divide configuration, error status and the interrupt command register.

use super::*;

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
pub struct Lvt(pub(super) u32);

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
