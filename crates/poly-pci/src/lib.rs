//! PCI and PCI Express configuration space.
//!
//! Everything the kernel needs to find PCI functions and set them up, minus
//! the hardware access itself: register layout, header parsing, BAR decoding
//! and size probing, bus enumeration, the capability list, and MSI / MSI-X
//! programming. The kernel supplies two small access traits: [`ConfigSpace`]
//! (normally over ECAM, see [`Ecam`]) and [`Mmio`] (a mapped BAR region, used
//! here for the MSI-X table and by the virtio transport).
//!
//! Sources: PCI Local Bus Specification 3.0 (chapter 6, configuration space;
//! §6.8, MSI and MSI-X), PCI-to-PCI Bridge Architecture Specification 1.2
//! (chapter 3, the type 1 header) and PCI Express Base Specification (§7.2.2,
//! ECAM). Everything read from a device is untrusted: nothing here panics or
//! indexes out of bounds on it, and every walk over device-provided links is
//! bounded. Nothing allocates.

#![no_std]

mod bar;
mod capability;
mod enumerate;
mod header;
mod msi;

pub use bar::{Bar, BarError, BarKind, probe_bar, probe_bars};
pub use capability::{Capabilities, Capability, cap_id};
pub use enumerate::{scan, scan_all};
pub use header::{
    BridgeBuses, ClassCode, Header, HeaderType, command, disable, enable, read_command, status, write_command,
};
pub use msi::{Message, Msi, MsiError, MsiX};

use core::fmt;

/// Bus / device / function address of one PCI function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Bdf {
    bus: u8,
    device: u8,
    function: u8,
}

impl Bdf {
    pub const MAX_DEVICE: u8 = 31;
    pub const MAX_FUNCTION: u8 = 7;

    /// `device` is truncated to 5 bits and `function` to 3, the widths they
    /// have on the wire.
    pub const fn new(bus: u8, device: u8, function: u8) -> Bdf {
        Bdf { bus, device: device & Self::MAX_DEVICE, function: function & Self::MAX_FUNCTION }
    }

    pub const fn bus(self) -> u8 {
        self.bus
    }

    pub const fn device(self) -> u8 {
        self.device
    }

    pub const fn function(self) -> u8 {
        self.function
    }
}

impl fmt::Display for Bdf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02x}:{:02x}.{}", self.bus, self.device, self.function)
    }
}

/// Access to the configuration space of PCI functions.
///
/// `offset` is a byte offset into the function's configuration space,
/// naturally aligned for the access width and below 4096 (256 with the legacy
/// port mechanism). Reads of a function that does not exist must return all
/// ones, as the hardware does (PCI 3.0 §6.1).
///
/// Writes must happen at the width asked for. The status register next to
/// the command register has write-one-to-clear bits (PCI 3.0 §6.2.3), so
/// emulating a 16-bit write with a 32-bit read-modify-write would clear them.
///
/// Every method takes `&self`: configuration registers are shared device
/// state, so implementations use raw pointers or interior mutability.
/// Serialising access between CPUs is the caller's job.
pub trait ConfigSpace {
    fn read32(&self, bdf: Bdf, offset: u16) -> u32;
    fn write32(&self, bdf: Bdf, offset: u16, value: u32);
    fn write16(&self, bdf: Bdf, offset: u16, value: u16);
    fn write8(&self, bdf: Bdf, offset: u16, value: u8);

    /// Configuration reads have no side effects, so narrow reads may be
    /// carved out of the containing dword.
    fn read16(&self, bdf: Bdf, offset: u16) -> u16 {
        (self.read32(bdf, offset & !3) >> ((offset & 2) * 8)) as u16
    }

    fn read8(&self, bdf: Bdf, offset: u16) -> u8 {
        (self.read32(bdf, offset & !3) >> ((offset & 3) * 8)) as u8
    }
}

/// A memory-mapped (or port-mapped) register block, such as part of a BAR.
///
/// `offset` is relative to the start of the block. Values are register
/// contents in CPU byte order; PCI registers are little-endian, so on a
/// big-endian CPU the implementation swaps. Callers in this repo check
/// offsets against the block's length before using them, but an
/// implementation must still never touch memory outside its block: ignore
/// such writes and return all ones for such reads.
///
/// Reads take `&self` even though some registers clear on read (the virtio
/// ISR status, for one): device registers are shared state that changes
/// underneath the CPU anyway.
pub trait Mmio {
    fn read8(&self, offset: usize) -> u8;
    fn read16(&self, offset: usize) -> u16;
    fn read32(&self, offset: usize) -> u32;
    fn write8(&self, offset: usize, value: u8);
    fn write16(&self, offset: usize, value: u16);
    fn write32(&self, offset: usize, value: u32);
}

/// Configuration register offsets shared by all header types (PCI 3.0
/// §6.1, figure 6-1) and the type 1 bridge fields (PCI-to-PCI Bridge 1.2
/// §3.2).
pub mod reg {
    pub const VENDOR_ID: u16 = 0x00;
    pub const DEVICE_ID: u16 = 0x02;
    pub const COMMAND: u16 = 0x04;
    pub const STATUS: u16 = 0x06;
    pub const REVISION_ID: u16 = 0x08;
    pub const PROG_IF: u16 = 0x09;
    pub const SUBCLASS: u16 = 0x0a;
    pub const CLASS: u16 = 0x0b;
    pub const HEADER_TYPE: u16 = 0x0e;
    pub const BAR0: u16 = 0x10;
    /// Type 0 only.
    pub const SUBSYSTEM_VENDOR_ID: u16 = 0x2c;
    /// Type 0 only.
    pub const SUBSYSTEM_ID: u16 = 0x2e;
    /// Types 0 and 1.
    pub const CAPABILITIES_POINTER: u16 = 0x34;
    /// Type 2 (CardBus bridge) keeps its capability pointer here instead.
    pub const CARDBUS_CAPABILITIES_POINTER: u16 = 0x14;
    pub const INTERRUPT_LINE: u16 = 0x3c;
    pub const INTERRUPT_PIN: u16 = 0x3d;

    pub const PRIMARY_BUS: u16 = 0x18;
    pub const SECONDARY_BUS: u16 = 0x19;
    pub const SUBORDINATE_BUS: u16 = 0x1a;
}

/// Size of one function's configuration space with ECAM.
pub const CONFIG_SPACE_SIZE: u16 = 4096;

/// One ECAM window: the memory-mapped configuration space of a PCI segment
/// group (PCI Express Base Specification §7.2.2).
///
/// Each function gets 4 KiB at `base + (bus << 20 | device << 15 | function
/// << 12)`. `base` is the address that formula gives for bus 0, which is what
/// the ACPI MCFG table's base address field holds (PCI Firmware
/// Specification, MCFG); the window only decodes `start_bus..=end_bus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ecam {
    pub base: u64,
    pub start_bus: u8,
    pub end_bus: u8,
}

impl Ecam {
    /// Offset of a configuration register from the bus 0 base.
    pub const fn offset(bdf: Bdf, offset: u16) -> u64 {
        ((bdf.bus as u64) << 20) | ((bdf.device as u64) << 15) | ((bdf.function as u64) << 12) | (offset as u64 & 0xfff)
    }

    /// Physical address of a configuration register, if the window decodes
    /// it.
    pub fn address(&self, bdf: Bdf, offset: u16) -> Option<u64> {
        if bdf.bus < self.start_bus || bdf.bus > self.end_bus || offset >= CONFIG_SPACE_SIZE {
            return None;
        }
        self.base.checked_add(Self::offset(bdf, offset))
    }

    /// The physical range the window occupies, as `(start, length)`: what
    /// the kernel has to map. `None` if the bus range is inverted or the
    /// window would wrap the address space.
    pub fn window(&self) -> Option<(u64, u64)> {
        if self.end_bus < self.start_bus {
            return None;
        }
        let start = self.base.checked_add((self.start_bus as u64) << 20)?;
        let len = (self.end_bus as u64 - self.start_bus as u64 + 1) << 20;
        start.checked_add(len - 1)?;
        Some((start, len))
    }
}

#[cfg(test)]
pub(crate) mod sim;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bdf_truncates_and_displays() {
        let b = Bdf::new(3, 0x3f, 9);
        assert_eq!((b.bus(), b.device(), b.function()), (3, 0x1f, 1));
        extern crate std;
        assert_eq!(std::format!("{}", Bdf::new(0x1a, 2, 7)), "1a:02.7");
    }

    #[test]
    fn ecam_addresses() {
        let e = Ecam { base: 0xb000_0000, start_bus: 0, end_bus: 0xff };
        assert_eq!(e.address(Bdf::new(0, 0, 0), 0), Some(0xb000_0000));
        assert_eq!(e.address(Bdf::new(1, 2, 3), 0x10), Some(0xb000_0000 + (1 << 20) + (2 << 15) + (3 << 12) + 0x10));
        assert_eq!(e.address(Bdf::new(0xff, 31, 7), 0xffc), Some(0xb000_0000 + 0x0fff_fffc));
        assert_eq!(e.address(Bdf::new(0, 0, 0), 0x1000), None);
        assert_eq!(e.window(), Some((0xb000_0000, 256 << 20)));
    }

    #[test]
    fn ecam_partial_window() {
        let e = Ecam { base: 0xe000_0000, start_bus: 0x10, end_bus: 0x1f };
        assert_eq!(e.address(Bdf::new(0x0f, 0, 0), 0), None);
        assert_eq!(e.address(Bdf::new(0x20, 0, 0), 0), None);
        assert_eq!(e.address(Bdf::new(0x10, 0, 0), 4), Some(0xe100_0004));
        assert_eq!(e.window(), Some((0xe100_0000, 16 << 20)));
        assert_eq!(Ecam { base: 0, start_bus: 2, end_bus: 1 }.window(), None);
        assert_eq!(Ecam { base: u64::MAX - 0x1000, start_bus: 0, end_bus: 0 }.window(), None);
        assert_eq!(Ecam { base: u64::MAX - 0x1000, start_bus: 0, end_bus: 0 }.address(Bdf::new(0, 1, 0), 0), None);
    }

    #[test]
    fn narrow_reads_come_from_the_dword() {
        let s = sim::Sim::new();
        let b = Bdf::new(0, 1, 0);
        s.add(b, sim::Function::endpoint(0x1234, 0x5678, [0x02, 0x00, 0x00]));
        assert_eq!(s.read16(b, reg::VENDOR_ID), 0x1234);
        assert_eq!(s.read16(b, reg::DEVICE_ID), 0x5678);
        assert_eq!(s.read8(b, reg::DEVICE_ID + 1), 0x56);
        assert_eq!(s.read32(Bdf::new(0, 2, 0), 0), u32::MAX);
    }
}
