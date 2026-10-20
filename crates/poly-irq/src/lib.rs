//! x86 interrupt controllers and timers.
//!
//! Everything the kernel needs to take interrupts and keep time on a PC,
//! minus the hardware access itself:
//!
//! - [`lapic`]: the local APIC, in xAPIC (memory-mapped) and x2APIC (MSR)
//!   mode behind one interface: enable, task priority, EOI, the local vector
//!   table, IPIs, the error status register and the APIC timer.
//! - [`ioapic`]: the I/O APIC redirection table and the mapping from ISA IRQs
//!   to global system interrupts (ACPI interrupt source overrides).
//! - [`pic`]: the legacy 8259 pair, remapped away from the exception vectors
//!   and masked.
//! - [`hpet`]: the High Precision Event Timer.
//! - [`time`]: tick / nanosecond conversion, calibrating one counter against
//!   another (TSC against the HPET, the APIC timer against either), and a
//!   monotonic clock.
//! - [`cpuid`]: the CPUID bits all of the above depend on, decoded from raw
//!   register values.
//! - [`msi`]: the x86 address / data encoding of message signalled
//!   interrupts, for PCI MSI and MSI-X.
//!
//! The kernel supplies three small access traits: [`Mmio`] (shared with
//! `poly-pci`; the xAPIC page, each I/O APIC and the HPET block), [`Msr`]
//! (`rdmsr` / `wrmsr`, for x2APIC and `IA32_APIC_BASE`) and [`PortIo`] (`in` /
//! `out`, for the 8259). Values read from hardware are untrusted: nothing here
//! panics on them, and every wait for the hardware is bounded. Nothing
//! allocates.
//!
//! Sources: Intel 64 and IA-32 Architectures Software Developer's Manual
//! (SDM) Vol. 3A chapter 11 "Advanced Programmable Interrupt Controller
//! (APIC)" (chapter 10 in editions before 2023; section numbers below are the
//! chapter 11 ones), Vol. 3B §18.17 "Time-Stamp Counter", Vol. 2A `CPUID`;
//! Intel 82093AA I/O Advanced Programmable Interrupt Controller datasheet;
//! Intel 8259A Programmable Interrupt Controller datasheet; IA-PC HPET
//! Specification 1.0a; ACPI Specification 6.5 §5.2.12 (MADT).

#![no_std]

pub mod cpuid;
pub mod hpet;
pub mod ioapic;
pub mod lapic;
pub mod msi;
pub mod pic;
pub mod time;

pub use cpuid::CpuFeatures;
pub use hpet::{Hpet, HpetError};
pub use ioapic::{IoApic, IoApicError, IsaOverride, IsaRoute, IsaRoutes, RedirectionEntry};
pub use lapic::{ApicError, ApicRegisters, Backend, Destination, Divide, Ipi, LocalApic, Lvt, LvtReg, X2Apic, XApic};
pub use msi::{MsiError, MsiMessage};
pub use pic::Pic8259;
pub use poly_pci::Mmio;
pub use time::{Calibration, CalibrationConfig, CalibrationError, Counter, MonotonicClock, Scale};

/// Model-specific register access (`rdmsr` / `wrmsr`).
///
/// Methods are safe to call: the implementation is the place that knows
/// whether the CPU has the register, and an access to a register it lacks
/// raises #GP. Everything in this crate only touches MSRs the caller has
/// told it exist (through [`CpuFeatures`]).
pub trait Msr {
    fn read(&self, msr: u32) -> u64;
    fn write(&self, msr: u32, value: u64);
}

/// x86 port I/O (`in` / `out`), byte-wide: all the 8259 needs.
pub trait PortIo {
    fn read8(&self, port: u16) -> u8;
    fn write8(&self, port: u16, value: u8);
}

impl<T: Msr + ?Sized> Msr for &T {
    fn read(&self, msr: u32) -> u64 {
        (**self).read(msr)
    }

    fn write(&self, msr: u32, value: u64) {
        (**self).write(msr, value)
    }
}

impl<T: PortIo + ?Sized> PortIo for &T {
    fn read8(&self, port: u16) -> u8 {
        (**self).read8(port)
    }

    fn write8(&self, port: u16, value: u8) {
        (**self).write8(port, value)
    }
}

/// How an interrupt message is delivered. The 3-bit encoding is the same in
/// the local vector table (SDM §11.5.1), the interrupt command register
/// (§11.6.1) and the I/O APIC redirection table (82093AA §3.2.4), but not
/// every mode is valid everywhere: LVT entries take fixed, SMI, NMI, INIT and
/// ExtINT; IPIs take everything but ExtINT; the I/O APIC everything but
/// start-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeliveryMode {
    Fixed,
    LowestPriority,
    Smi,
    Nmi,
    Init,
    StartUp,
    ExtInt,
}

impl DeliveryMode {
    pub const fn bits(self) -> u32 {
        match self {
            DeliveryMode::Fixed => 0b000,
            DeliveryMode::LowestPriority => 0b001,
            DeliveryMode::Smi => 0b010,
            DeliveryMode::Nmi => 0b100,
            DeliveryMode::Init => 0b101,
            DeliveryMode::StartUp => 0b110,
            DeliveryMode::ExtInt => 0b111,
        }
    }

    /// `None` for the reserved encoding 0b011.
    pub const fn from_bits(bits: u32) -> Option<DeliveryMode> {
        Some(match bits & 0b111 {
            0b000 => DeliveryMode::Fixed,
            0b001 => DeliveryMode::LowestPriority,
            0b010 => DeliveryMode::Smi,
            0b100 => DeliveryMode::Nmi,
            0b101 => DeliveryMode::Init,
            0b110 => DeliveryMode::StartUp,
            0b111 => DeliveryMode::ExtInt,
            _ => return None,
        })
    }
}

/// Polarity of an interrupt input pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Polarity {
    ActiveHigh,
    ActiveLow,
}

/// Trigger mode of an interrupt input pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trigger {
    Edge,
    Level,
}

#[cfg(test)]
pub(crate) mod sim;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_mode_round_trips() {
        for bits in 0..8 {
            match DeliveryMode::from_bits(bits) {
                Some(m) => assert_eq!(m.bits(), bits),
                None => assert_eq!(bits, 0b011),
            }
        }
        assert_eq!(DeliveryMode::from_bits(0b1101), Some(DeliveryMode::Init));
    }
}
