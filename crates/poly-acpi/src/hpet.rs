//! HPET Description Table, defined in the IA-PC HPET Specification 1.0a,
//! §3.2.4 "The ACPI 2.0 HPET Description Table (HPET)".

use crate::{Error, GenericAddress, Sdt, Table, u8_at, u16_at, u32_at};

/// Offsets in the table.
const EVENT_TIMER_BLOCK_ID: usize = 36;
const BASE_ADDRESS: usize = 40;
const HPET_NUMBER: usize = 52;
const MIN_CLOCK_TICK: usize = 53;
const PAGE_PROTECTION: usize = 55;
const LEN: usize = 56;

#[derive(Clone, Copy, Debug)]
pub struct Hpet<'a> {
    sdt: Sdt<'a>,
    /// Event Timer Block ID: a copy of the low 32 bits of the block's
    /// General Capabilities and ID register.
    pub block_id: u32,
    /// Where the timer block's registers live (system memory in practice).
    pub base: GenericAddress,
    /// Sequence number of this timer block, 0 for the first.
    pub number: u8,
    /// Main counter minimum clock tick in periodic mode, in counter ticks.
    pub minimum_tick: u16,
    /// Page protection and OEM attributes.
    pub page_protection: u8,
}

impl<'a> Table<'a> for Hpet<'a> {
    const SIGNATURE: [u8; 4] = *b"HPET";

    fn parse(sdt: Sdt<'a>) -> Result<Self, Error> {
        let sdt = sdt.expect(Self::SIGNATURE, LEN)?;
        let b = sdt.bytes();
        let field = || Error::Truncated;
        Ok(Hpet {
            sdt,
            block_id: u32_at(b, EVENT_TIMER_BLOCK_ID).ok_or_else(field)?,
            base: b.get(BASE_ADDRESS..).and_then(GenericAddress::parse).ok_or_else(field)?,
            number: u8_at(b, HPET_NUMBER).ok_or_else(field)?,
            minimum_tick: u16_at(b, MIN_CLOCK_TICK).ok_or_else(field)?,
            page_protection: u8_at(b, PAGE_PROTECTION).ok_or_else(field)?,
        })
    }
}

impl<'a> Hpet<'a> {
    pub fn sdt(&self) -> Sdt<'a> {
        self.sdt
    }

    /// Hardware revision ID (bits 7:0).
    pub fn hardware_revision(&self) -> u8 {
        self.block_id as u8
    }

    /// Number of comparators (timers) in the block. Bits 12:8 hold the index
    /// of the last one, so the count is one more.
    pub fn comparator_count(&self) -> u8 {
        ((self.block_id >> 8) & 0x1f) as u8 + 1
    }

    /// The main counter is 64 bits wide (bit 13, COUNT_SIZE_CAP).
    pub fn counter_is_64bit(&self) -> bool {
        self.block_id & (1 << 13) != 0
    }

    /// The block can take over the legacy PIT and RTC interrupts (bit 15).
    pub fn legacy_replacement_capable(&self) -> bool {
        self.block_id & (1 << 15) != 0
    }

    /// PCI vendor ID of the block's maker (bits 31:16).
    pub fn pci_vendor_id(&self) -> u16 {
        (self.block_id >> 16) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AddressSpace;
    use crate::testutil::*;

    #[test]
    fn fields() {
        let t = table(b"HPET", 1, &hpet_body());
        let h = Hpet::parse(Sdt::parse(&t).unwrap()).unwrap();
        assert_eq!(h.base.space, AddressSpace::SystemMemory);
        assert_eq!(h.base.address, 0xfed0_0000);
        assert_eq!(h.comparator_count(), 3);
        assert_eq!(h.pci_vendor_id(), 0x8086);
        assert_eq!(h.hardware_revision(), 1);
        assert!(h.counter_is_64bit());
        assert!(h.legacy_replacement_capable());
        assert_eq!(h.number, 0);
        assert_eq!(h.minimum_tick, 0x80);
        assert_eq!(h.page_protection, 0);
    }

    #[test]
    fn too_short_or_wrong() {
        let body = hpet_body();
        let t = table(b"HPET", 1, &body[..body.len() - 1]);
        assert_eq!(Hpet::parse(Sdt::parse(&t).unwrap()).map(|_| ()), Err(Error::Truncated));
        let t = table(b"HPEX", 1, &body);
        assert_eq!(Hpet::parse(Sdt::parse(&t).unwrap()).map(|_| ()), Err(Error::BadSignature));
    }

    #[test]
    fn prefixes_never_panic() {
        for_each_prefix(&table(b"HPET", 1, &hpet_body()), |sdt| {
            if let Ok(h) = Hpet::parse(sdt) {
                walk_hpet(&h);
            }
        });
    }
}
