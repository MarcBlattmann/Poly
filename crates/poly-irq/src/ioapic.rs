//! The I/O APIC (Intel 82093AA I/O APIC datasheet) and the mapping from ISA
//! IRQs to its inputs (ACPI 6.5 §5.2.12.5, "Interrupt Source Override
//! Structure").
//!
//! Each I/O APIC has two memory-mapped registers: IOREGSEL at offset 0x00
//! selects an internal register and IOWIN at 0x10 reads or writes it
//! (§3.1). Internal registers (§3.2): 0x00 ID, 0x01 version, 0x02
//! arbitration ID, then one 64-bit redirection entry per input at
//! 0x10 + 2n (low dword) and 0x11 + 2n (high dword). The MADT lists every
//! I/O APIC with the first global system interrupt (GSI) it handles; input n
//! is GSI `gsi_base + n`.
//!
//! The select-then-access pair is not atomic: callers serialise access to
//! one I/O APIC.

use crate::{DeliveryMode, Mmio, Polarity, Trigger};

pub mod reg {
    /// Memory-mapped offsets.
    pub const IOREGSEL: usize = 0x00;
    pub const IOWIN: usize = 0x10;

    /// Internal register indices.
    pub const ID: u8 = 0x00;
    pub const VERSION: u8 = 0x01;
    pub const ARBITRATION: u8 = 0x02;
    pub const REDIRECTION_TABLE: u8 = 0x10;

    pub const fn redirection_low(input: u8) -> u8 {
        REDIRECTION_TABLE + 2 * input
    }
}

/// One redirection table entry (82093AA §3.2.4, "IOREDTBL[23:0]").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RedirectionEntry(u64);

impl RedirectionEntry {
    pub const VECTOR: u64 = 0xff;
    pub const DELIVERY_SHIFT: u32 = 8;
    /// Destination mode: logical (otherwise physical).
    pub const LOGICAL: u64 = 1 << 11;
    /// Read-only: the interrupt is waiting to be delivered.
    pub const DELIVERY_STATUS: u64 = 1 << 12;
    pub const ACTIVE_LOW: u64 = 1 << 13;
    /// Read-only: a level-triggered interrupt was accepted and awaits EOI.
    pub const REMOTE_IRR: u64 = 1 << 14;
    pub const LEVEL: u64 = 1 << 15;
    pub const MASKED: u64 = 1 << 16;
    pub const DESTINATION_SHIFT: u32 = 56;
    pub const READ_ONLY: u64 = Self::DELIVERY_STATUS | Self::REMOTE_IRR;

    /// Fixed delivery of `vector` to the local APIC with ID `destination`
    /// (physical mode), edge-triggered, active high, unmasked.
    pub const fn new(vector: u8, destination: u8) -> RedirectionEntry {
        RedirectionEntry(vector as u64 | (destination as u64) << Self::DESTINATION_SHIFT)
    }

    /// The reset value: masked, everything else 0.
    pub const fn masked() -> RedirectionEntry {
        RedirectionEntry(Self::MASKED)
    }

    pub const fn from_bits(bits: u64) -> RedirectionEntry {
        RedirectionEntry(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn with_vector(self, vector: u8) -> RedirectionEntry {
        RedirectionEntry(self.0 & !Self::VECTOR | vector as u64)
    }

    pub const fn with_delivery(self, mode: DeliveryMode) -> RedirectionEntry {
        RedirectionEntry(self.0 & !(0b111 << Self::DELIVERY_SHIFT) | (mode.bits() as u64) << Self::DELIVERY_SHIFT)
    }

    pub const fn with_logical(self, logical: bool) -> RedirectionEntry {
        RedirectionEntry(if logical { self.0 | Self::LOGICAL } else { self.0 & !Self::LOGICAL })
    }

    pub const fn with_polarity(self, polarity: Polarity) -> RedirectionEntry {
        match polarity {
            Polarity::ActiveHigh => RedirectionEntry(self.0 & !Self::ACTIVE_LOW),
            Polarity::ActiveLow => RedirectionEntry(self.0 | Self::ACTIVE_LOW),
        }
    }

    pub const fn with_trigger(self, trigger: Trigger) -> RedirectionEntry {
        match trigger {
            Trigger::Edge => RedirectionEntry(self.0 & !Self::LEVEL),
            Trigger::Level => RedirectionEntry(self.0 | Self::LEVEL),
        }
    }

    pub const fn with_mask(self, masked: bool) -> RedirectionEntry {
        RedirectionEntry(if masked { self.0 | Self::MASKED } else { self.0 & !Self::MASKED })
    }

    /// An APIC ID in physical mode, a set of APICs in logical mode.
    pub const fn with_destination(self, destination: u8) -> RedirectionEntry {
        RedirectionEntry(self.0 & !(0xff << Self::DESTINATION_SHIFT) | (destination as u64) << Self::DESTINATION_SHIFT)
    }

    pub const fn vector(self) -> u8 {
        self.0 as u8
    }

    /// `None` for the reserved encoding 0b011.
    pub const fn delivery(self) -> Option<DeliveryMode> {
        DeliveryMode::from_bits((self.0 >> Self::DELIVERY_SHIFT) as u32)
    }

    pub const fn is_logical(self) -> bool {
        self.0 & Self::LOGICAL != 0
    }

    pub const fn polarity(self) -> Polarity {
        if self.0 & Self::ACTIVE_LOW != 0 { Polarity::ActiveLow } else { Polarity::ActiveHigh }
    }

    pub const fn trigger(self) -> Trigger {
        if self.0 & Self::LEVEL != 0 { Trigger::Level } else { Trigger::Edge }
    }

    pub const fn is_masked(self) -> bool {
        self.0 & Self::MASKED != 0
    }

    pub const fn is_pending(self) -> bool {
        self.0 & Self::DELIVERY_STATUS != 0
    }

    pub const fn remote_irr(self) -> bool {
        self.0 & Self::REMOTE_IRR != 0
    }

    pub const fn destination(self) -> u8 {
        (self.0 >> Self::DESTINATION_SHIFT) as u8
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoApicError {
    /// The version register reads all ones: nothing is mapped there.
    NotPresent,
    /// The input or GSI is not handled by this I/O APIC.
    NoSuchInput,
    /// An unmasked fixed or lowest-priority entry with a vector below 16.
    InvalidVector,
    /// Start-up and the reserved encoding are not valid here.
    InvalidDelivery,
}

/// One I/O APIC.
#[derive(Debug)]
pub struct IoApic<M> {
    mmio: M,
    gsi_base: u32,
    inputs: u8,
}

impl<M: Mmio> IoApic<M> {
    /// IOREGSEL is 8 bits wide, so at most this many redirection entries
    /// can be addressed, whatever the version register claims.
    pub const MAX_INPUTS: u8 = (256 - reg::REDIRECTION_TABLE as u16) as u8 / 2;

    /// Reads the version register; changes nothing. `gsi_base` comes from
    /// the MADT (`poly_acpi::IoApic::gsi_base`).
    pub fn new(mmio: M, gsi_base: u32) -> Result<IoApic<M>, IoApicError> {
        let mut ioapic = IoApic { mmio, gsi_base, inputs: 0 };
        let version = ioapic.read(reg::VERSION);
        if version == u32::MAX {
            return Err(IoApicError::NotPresent);
        }
        let max_entry = (version >> 16) as u8;
        ioapic.inputs = max_entry.saturating_add(1).min(Self::MAX_INPUTS);
        Ok(ioapic)
    }

    pub fn mmio(&self) -> &M {
        &self.mmio
    }

    fn read(&self, index: u8) -> u32 {
        self.mmio.write32(reg::IOREGSEL, index as u32);
        self.mmio.read32(reg::IOWIN)
    }

    fn write(&self, index: u8, value: u32) {
        self.mmio.write32(reg::IOREGSEL, index as u32);
        self.mmio.write32(reg::IOWIN, value);
    }

    /// The I/O APIC ID (§3.2.1, bits 27:24).
    pub fn id(&self) -> u8 {
        ((self.read(reg::ID) >> 24) & 0xf) as u8
    }

    /// Bits 7:0 of the version register (0x11 for the 82093AA).
    pub fn version(&self) -> u8 {
        self.read(reg::VERSION) as u8
    }

    /// Number of inputs (maximum redirection entry + 1, §3.2.2).
    pub fn inputs(&self) -> u8 {
        self.inputs
    }

    pub fn gsi_base(&self) -> u32 {
        self.gsi_base
    }

    /// The input `gsi` arrives on, if this I/O APIC handles it.
    pub fn input_for(&self, gsi: u32) -> Option<u8> {
        let n = gsi.checked_sub(self.gsi_base)?;
        if n < self.inputs as u32 { Some(n as u8) } else { None }
    }

    pub fn entry(&self, input: u8) -> Result<RedirectionEntry, IoApicError> {
        if input >= self.inputs {
            return Err(IoApicError::NoSuchInput);
        }
        let low = self.read(reg::redirection_low(input)) as u64;
        let high = self.read(reg::redirection_low(input) + 1) as u64;
        Ok(RedirectionEntry(high << 32 | low))
    }

    /// Writes a redirection entry. The input is masked while the two halves
    /// are written, so a half-updated entry is never live: low dword with
    /// the mask set, high dword, then the final low dword.
    pub fn set_entry(&self, input: u8, entry: RedirectionEntry) -> Result<(), IoApicError> {
        if input >= self.inputs {
            return Err(IoApicError::NoSuchInput);
        }
        match entry.delivery() {
            None | Some(DeliveryMode::StartUp) => return Err(IoApicError::InvalidDelivery),
            Some(DeliveryMode::Fixed | DeliveryMode::LowestPriority) if !entry.is_masked() && entry.vector() < 16 => {
                return Err(IoApicError::InvalidVector);
            }
            _ => {}
        }
        let bits = entry.bits() & !RedirectionEntry::READ_ONLY;
        let low = reg::redirection_low(input);
        self.write(low, (bits | RedirectionEntry::MASKED) as u32);
        self.write(low + 1, (bits >> 32) as u32);
        if !entry.is_masked() {
            self.write(low, bits as u32);
        }
        Ok(())
    }

    pub fn set_masked(&self, input: u8, masked: bool) -> Result<(), IoApicError> {
        let entry = self.entry(input)?;
        // Only the low dword changes.
        let bits = entry.with_mask(masked).bits() & !RedirectionEntry::READ_ONLY;
        self.write(reg::redirection_low(input), bits as u32);
        Ok(())
    }

    /// Masks every input, keeping the rest of each entry.
    pub fn mask_all(&self) {
        for input in 0..self.inputs {
            let _ = self.set_masked(input, true);
        }
    }

    /// Sets the entry for global system interrupt `gsi`.
    pub fn route(&self, gsi: u32, entry: RedirectionEntry) -> Result<(), IoApicError> {
        self.set_entry(self.input_for(gsi).ok_or(IoApicError::NoSuchInput)?, entry)
    }
}

/// The I/O APIC among `ioapics` that handles `gsi`.
pub fn for_gsi<M: Mmio>(ioapics: &[IoApic<M>], gsi: u32) -> Option<&IoApic<M>> {
    ioapics.iter().find(|io| io.input_for(gsi).is_some())
}

/// One MADT Interrupt Source Override, as plain values so this crate does
/// not depend on `poly-acpi`. The kernel copies the fields of
/// `poly_acpi::InterruptOverride` (`flags` is its `MpsIntiFlags.0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsaOverride {
    /// 0 = ISA, the only bus defined.
    pub bus: u8,
    /// The ISA IRQ.
    pub source: u8,
    pub gsi: u32,
    /// MPS INTI flags (ACPI 6.5 table 5.26): polarity in bits 1:0, trigger
    /// mode in bits 3:2.
    pub flags: u16,
}

impl IsaOverride {
    /// `None` for "conforms to the bus" (00) and the reserved value (10).
    pub const fn polarity(&self) -> Option<Polarity> {
        match self.flags & 0b11 {
            0b01 => Some(Polarity::ActiveHigh),
            0b11 => Some(Polarity::ActiveLow),
            _ => None,
        }
    }

    /// `None` for "conforms to the bus" (00) and the reserved value (10).
    pub const fn trigger(&self) -> Option<Trigger> {
        match (self.flags >> 2) & 0b11 {
            0b01 => Some(Trigger::Edge),
            0b11 => Some(Trigger::Level),
            _ => None,
        }
    }
}

/// Where an ISA IRQ arrives and how it is signalled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsaRoute {
    pub gsi: u32,
    pub polarity: Polarity,
    pub trigger: Trigger,
}

impl IsaRoute {
    /// A fixed, physical-mode, unmasked entry for this route.
    pub const fn entry(self, vector: u8, destination: u8) -> RedirectionEntry {
        RedirectionEntry::new(vector, destination).with_polarity(self.polarity).with_trigger(self.trigger)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverrideError {
    /// The override is for a bus other than ISA.
    NotIsa,
    /// The source is not an ISA IRQ (0 to 15).
    NoSuchIrq,
}

/// The 16 ISA IRQs and the GSIs they arrive on.
///
/// Without overrides, ISA IRQ n is GSI n, edge-triggered, active high (ACPI
/// 6.5 §5.2.12.5: ISA interrupts are assumed identity-mapped onto the first
/// I/O APIC's inputs). An override moves an IRQ and/or changes its polarity
/// and trigger mode; flags that conform to the bus (or are reserved) keep
/// the ISA defaults. When an override moves IRQ a onto GSI b, IRQ b no
/// longer has an identity mapping unless it has an override of its own:
/// with the usual "IRQ 0 -> GSI 2", GSI 2 belongs to the timer, not to the
/// cascade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsaRoutes {
    routes: [IsaRoute; 16],
    overridden: u16,
}

impl IsaRoutes {
    pub const IRQS: u8 = 16;

    pub const fn identity() -> IsaRoutes {
        let mut routes = [IsaRoute { gsi: 0, polarity: Polarity::ActiveHigh, trigger: Trigger::Edge }; 16];
        let mut irq = 0;
        while irq < 16 {
            routes[irq].gsi = irq as u32;
            irq += 1;
        }
        IsaRoutes { routes, overridden: 0 }
    }

    /// Applies every override that is valid, skipping the rest.
    pub fn from_overrides(overrides: impl IntoIterator<Item = IsaOverride>) -> IsaRoutes {
        let mut routes = IsaRoutes::identity();
        for o in overrides {
            let _ = routes.apply(o);
        }
        routes
    }

    pub fn apply(&mut self, o: IsaOverride) -> Result<(), OverrideError> {
        if o.bus != 0 {
            return Err(OverrideError::NotIsa);
        }
        if o.source >= Self::IRQS {
            return Err(OverrideError::NoSuchIrq);
        }
        self.routes[o.source as usize] = IsaRoute {
            gsi: o.gsi,
            polarity: o.polarity().unwrap_or(Polarity::ActiveHigh),
            trigger: o.trigger().unwrap_or(Trigger::Edge),
        };
        self.overridden |= 1 << o.source;
        Ok(())
    }

    fn is_overridden(&self, irq: u8) -> bool {
        self.overridden & (1 << irq) != 0
    }

    /// Where ISA IRQ `irq` arrives. `None` for IRQs above 15 and for an
    /// identity-mapped IRQ whose GSI an override gave to another IRQ.
    pub fn route(&self, irq: u8) -> Option<IsaRoute> {
        if irq >= Self::IRQS {
            return None;
        }
        let route = self.routes[irq as usize];
        if !self.is_overridden(irq)
            && (0..Self::IRQS).any(|other| self.is_overridden(other) && self.routes[other as usize].gsi == route.gsi)
        {
            return None;
        }
        Some(route)
    }

    /// The ISA IRQ that arrives on `gsi`, if any.
    pub fn irq_for_gsi(&self, gsi: u32) -> Option<u8> {
        (0..Self::IRQS).find(|&irq| self.route(irq).is_some_and(|r| r.gsi == gsi))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::SimIoApic;

    #[test]
    fn entry_encoding_round_trips() {
        let e = RedirectionEntry::new(0x31, 0x0f)
            .with_delivery(DeliveryMode::LowestPriority)
            .with_logical(true)
            .with_polarity(Polarity::ActiveLow)
            .with_trigger(Trigger::Level)
            .with_mask(true);
        assert_eq!(e.bits(), 0x0f00_0000_0001_a931);
        assert_eq!(e.vector(), 0x31);
        assert_eq!(e.delivery(), Some(DeliveryMode::LowestPriority));
        assert!(e.is_logical() && e.is_masked());
        assert_eq!(e.polarity(), Polarity::ActiveLow);
        assert_eq!(e.trigger(), Trigger::Level);
        assert_eq!(e.destination(), 0x0f);
        assert_eq!(RedirectionEntry::from_bits(e.bits()), e);
        let back = e
            .with_logical(false)
            .with_polarity(Polarity::ActiveHigh)
            .with_trigger(Trigger::Edge)
            .with_mask(false)
            .with_delivery(DeliveryMode::Fixed)
            .with_destination(0xff)
            .with_vector(0x40);
        assert_eq!(back.bits(), 0xff00_0000_0000_0040);
        for bits in 0..8 {
            let e = RedirectionEntry::from_bits((bits as u64) << 8);
            assert_eq!(e.delivery().map(DeliveryMode::bits), DeliveryMode::from_bits(bits).map(DeliveryMode::bits));
        }
        assert!(RedirectionEntry::from_bits(1 << 12).is_pending());
        assert!(RedirectionEntry::from_bits(1 << 14).remote_irr());
        assert!(RedirectionEntry::masked().is_masked());
    }

    #[test]
    fn identifies_and_sizes() {
        let sim = SimIoApic::new(0x0017_0011, 24);
        sim.poke(reg::ID, 2 << 24);
        let io = IoApic::new(&sim, 0).unwrap();
        assert_eq!(io.id(), 2);
        assert_eq!(io.version(), 0x11);
        assert_eq!(io.inputs(), 24);
        // Every index the driver selected was written as a full dword.
        assert_eq!(IoApic::new(&SimIoApic::new(u32::MAX, 0), 0).err(), Some(IoApicError::NotPresent));
        // A version register claiming 256 inputs is clamped to what IOREGSEL
        // can address.
        let big = SimIoApic::new(0x00ff_0020, 120);
        assert_eq!(IoApic::new(&big, 0).unwrap().inputs(), 120);
        assert_eq!(IoApic::<&SimIoApic>::MAX_INPUTS, 120);
    }

    #[test]
    fn entries_are_written_masked_first() {
        let sim = SimIoApic::new(0x0017_0011, 24);
        let io = IoApic::new(&sim, 0).unwrap();
        let e = RedirectionEntry::new(0x41, 3).with_trigger(Trigger::Level);
        sim.clear_log();
        io.set_entry(5, e).unwrap();
        let low = reg::redirection_low(5);
        assert_eq!(sim.log(), [(low, 0x0001_8041), (low + 1, 0x0300_0000), (low, 0x0000_8041)]);
        assert_eq!(io.entry(5), Ok(e));
        assert!(!sim.was_live_half_written(5));
        // A masked entry needs no third write.
        sim.clear_log();
        io.set_entry(6, e.with_mask(true)).unwrap();
        assert_eq!(sim.log().len(), 2);
        assert_eq!(io.set_entry(24, e), Err(IoApicError::NoSuchInput));
        assert_eq!(io.entry(24), Err(IoApicError::NoSuchInput));
        assert_eq!(io.set_entry(1, RedirectionEntry::new(15, 0)), Err(IoApicError::InvalidVector));
        assert_eq!(io.set_entry(1, RedirectionEntry::new(15, 0).with_mask(true)), Ok(()));
        let nmi = RedirectionEntry::new(0, 0).with_delivery(DeliveryMode::Nmi);
        assert_eq!(io.set_entry(1, nmi), Ok(()));
        let sipi = RedirectionEntry::new(0x40, 0).with_delivery(DeliveryMode::StartUp);
        assert_eq!(io.set_entry(1, sipi), Err(IoApicError::InvalidDelivery));
        assert_eq!(io.set_entry(1, RedirectionEntry::from_bits(0b011 << 8 | 0x40)), Err(IoApicError::InvalidDelivery));
    }

    #[test]
    fn read_only_bits_are_preserved() {
        let sim = SimIoApic::new(0x0017_0011, 24);
        let io = IoApic::new(&sim, 0).unwrap();
        io.set_entry(9, RedirectionEntry::new(0x49, 0).with_trigger(Trigger::Level)).unwrap();
        sim.set_remote_irr(9, true);
        let e = io.entry(9).unwrap();
        assert!(e.remote_irr());
        // Writing back what was read must not try to set the read-only bit.
        io.set_masked(9, true).unwrap();
        assert!(sim.log().iter().all(|&(_, v)| v & (1 << 14) == 0));
        assert!(io.entry(9).unwrap().remote_irr());
        assert!(io.entry(9).unwrap().is_masked());
        // Version and arbitration registers are read-only.
        sim.clear_log();
        io.mmio().write32(reg::IOREGSEL, reg::VERSION as u32);
        io.mmio().write32(reg::IOWIN, 0);
        assert_eq!(io.version(), 0x11);
    }

    #[test]
    fn mask_all_and_unmask() {
        let sim = SimIoApic::new(0x0017_0011, 24);
        let io = IoApic::new(&sim, 0).unwrap();
        for i in 0..24 {
            io.set_entry(i, RedirectionEntry::new(0x30 + i, 0)).unwrap();
        }
        io.mask_all();
        for i in 0..24 {
            let e = io.entry(i).unwrap();
            assert!(e.is_masked());
            assert_eq!(e.vector(), 0x30 + i);
        }
        io.set_masked(4, false).unwrap();
        assert!(!io.entry(4).unwrap().is_masked());
        assert_eq!(io.set_masked(30, false), Err(IoApicError::NoSuchInput));
    }

    #[test]
    fn routes_by_gsi_across_ioapics() {
        let a = SimIoApic::new(0x0017_0011, 24);
        let b = SimIoApic::new(0x0017_0020, 24);
        let ioapics = [IoApic::new(&a, 0).unwrap(), IoApic::new(&b, 24).unwrap()];
        assert_eq!(ioapics[1].input_for(23), None);
        assert_eq!(ioapics[1].input_for(24), Some(0));
        assert_eq!(ioapics[1].input_for(47), Some(23));
        assert_eq!(ioapics[1].input_for(48), None);
        let io = for_gsi(&ioapics, 30).unwrap();
        assert_eq!(io.gsi_base(), 24);
        io.route(30, RedirectionEntry::new(0x60, 1)).unwrap();
        assert_eq!(ioapics[1].entry(6).unwrap().vector(), 0x60);
        assert_eq!(ioapics[0].route(30, RedirectionEntry::new(0x60, 1)), Err(IoApicError::NoSuchInput));
        assert!(for_gsi(&ioapics, 48).is_none());
    }

    /// The overrides QEMU's MADT carries, plus a typical active-low SCI.
    fn qemu_like() -> [IsaOverride; 6] {
        let o = |source, gsi, flags| IsaOverride { bus: 0, source, gsi, flags };
        [o(0, 2, 0), o(5, 5, 0xd), o(9, 9, 0xf), o(10, 10, 0xd), o(11, 11, 0xd), o(3, 3, 0b1010)]
    }

    #[test]
    fn applies_interrupt_source_overrides() {
        let routes = IsaRoutes::from_overrides(qemu_like());
        let edge_high = |gsi| Some(IsaRoute { gsi, polarity: Polarity::ActiveHigh, trigger: Trigger::Edge });
        assert_eq!(routes.route(0), edge_high(2));
        assert_eq!(routes.route(1), edge_high(1));
        // GSI 2 went to IRQ 0.
        assert_eq!(routes.route(2), None);
        // Reserved flags keep the ISA defaults.
        assert_eq!(routes.route(3), edge_high(3));
        assert_eq!(routes.route(5), Some(IsaRoute { gsi: 5, polarity: Polarity::ActiveHigh, trigger: Trigger::Level }));
        assert_eq!(routes.route(9), Some(IsaRoute { gsi: 9, polarity: Polarity::ActiveLow, trigger: Trigger::Level }));
        assert_eq!(routes.route(15), edge_high(15));
        assert_eq!(routes.route(16), None);
        assert_eq!(routes.irq_for_gsi(2), Some(0));
        assert_eq!(routes.irq_for_gsi(9), Some(9));
        assert_eq!(routes.irq_for_gsi(0), None);
        assert_eq!(routes.irq_for_gsi(16), None);
        // Order does not matter.
        let mut reversed = qemu_like();
        reversed.reverse();
        assert_eq!(IsaRoutes::from_overrides(reversed), routes);
    }

    #[test]
    fn override_edge_cases() {
        let mut routes = IsaRoutes::identity();
        assert_eq!(routes.apply(IsaOverride { bus: 1, source: 4, gsi: 20, flags: 0 }), Err(OverrideError::NotIsa));
        assert_eq!(routes.apply(IsaOverride { bus: 0, source: 16, gsi: 20, flags: 0 }), Err(OverrideError::NoSuchIrq));
        assert_eq!(routes, IsaRoutes::identity());
        // An IRQ moved off the first I/O APIC, and a swap of two IRQs.
        routes.apply(IsaOverride { bus: 0, source: 4, gsi: 40, flags: 0 }).unwrap();
        routes.apply(IsaOverride { bus: 0, source: 6, gsi: 7, flags: 0 }).unwrap();
        routes.apply(IsaOverride { bus: 0, source: 7, gsi: 6, flags: 0 }).unwrap();
        assert_eq!(routes.route(4).unwrap().gsi, 40);
        assert_eq!(routes.route(6).unwrap().gsi, 7);
        assert_eq!(routes.route(7).unwrap().gsi, 6);
        assert_eq!(routes.irq_for_gsi(4), None);
        assert_eq!(routes.irq_for_gsi(40), Some(4));
        // A later override of the same IRQ wins.
        routes.apply(IsaOverride { bus: 0, source: 4, gsi: 41, flags: 0b1111 }).unwrap();
        assert_eq!(routes.route(4), Some(IsaRoute { gsi: 41, polarity: Polarity::ActiveLow, trigger: Trigger::Level }));
    }

    #[test]
    fn routes_an_isa_irq_end_to_end() {
        let sim = SimIoApic::new(0x0017_0011, 24);
        let io = IoApic::new(&sim, 0).unwrap();
        let routes = IsaRoutes::from_overrides(qemu_like());
        let timer = routes.route(0).unwrap();
        io.route(timer.gsi, timer.entry(0x20, 0)).unwrap();
        let sci = routes.route(9).unwrap();
        io.route(sci.gsi, sci.entry(0x29, 0)).unwrap();
        assert_eq!(io.entry(2).unwrap(), RedirectionEntry::new(0x20, 0));
        let e = io.entry(9).unwrap();
        assert_eq!(
            (e.vector(), e.polarity(), e.trigger(), e.is_masked()),
            (0x29, Polarity::ActiveLow, Trigger::Level, false)
        );
    }
}
