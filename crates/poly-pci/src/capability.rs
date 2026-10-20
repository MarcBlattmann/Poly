//! The capability list (PCI 3.0 §6.7).
//!
//! If status bit 4 is set, the capabilities pointer register holds the
//! offset of the first capability. Each capability starts with its ID byte
//! and the offset of the next one; 0 ends the list. Capabilities live in the
//! device-specific part of the header, at 0x40 or above, and the bottom two
//! bits of every pointer are reserved and masked off.

use crate::{Bdf, ConfigSpace, Header, HeaderType, reg, status};

/// Capability IDs (PCI Code and ID Assignment Specification, chapter 2).
pub mod cap_id {
    pub const POWER_MANAGEMENT: u8 = 0x01;
    pub const MSI: u8 = 0x05;
    pub const VENDOR_SPECIFIC: u8 = 0x09;
    pub const PCI_EXPRESS: u8 = 0x10;
    pub const MSI_X: u8 = 0x11;
}

/// One entry of the list: its ID and where it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Capability {
    pub id: u8,
    pub offset: u8,
}

impl Capability {
    /// Is a structure of `len` bytes at this capability's offset inside the
    /// 256-byte header? Parsers check this before reading their fields.
    pub const fn fits(self, len: usize) -> bool {
        self.offset as usize + len <= 256
    }
}

/// Iterator over a function's capabilities.
///
/// There are 48 dword-aligned offsets from 0x40 to 0xfc; the iterator
/// remembers which it has visited and stops at the first repeat, so a
/// looping chain from a broken or hostile device yields each capability once
/// and then ends.
#[derive(Debug)]
pub struct Capabilities<'a, C: ?Sized> {
    cfg: &'a C,
    bdf: Bdf,
    next: u8,
    seen: u64,
}

impl<'a, C: ConfigSpace + ?Sized> Capabilities<'a, C> {
    /// The capabilities of the function `header` was read from. Empty if its
    /// status register says it has none or its layout has no pointer.
    pub fn new(cfg: &'a C, header: &Header) -> Capabilities<'a, C> {
        let pointer = match header.header_type {
            HeaderType::Endpoint | HeaderType::Bridge => Some(reg::CAPABILITIES_POINTER),
            HeaderType::CardBus => Some(reg::CARDBUS_CAPABILITIES_POINTER),
            HeaderType::Unknown(_) => None,
        };
        let next = match pointer {
            Some(p) if header.status & status::CAPABILITIES_LIST != 0 => cfg.read8(header.bdf, p),
            _ => 0,
        };
        Capabilities { cfg, bdf: header.bdf, next, seen: 0 }
    }

    /// The first capability with ID `id`.
    pub fn find_id(mut self, id: u8) -> Option<Capability> {
        self.find(|c| c.id == id)
    }
}

impl<C: ConfigSpace + ?Sized> Iterator for Capabilities<'_, C> {
    type Item = Capability;

    fn next(&mut self) -> Option<Capability> {
        let offset = self.next & !3;
        if offset < 0x40 {
            return None;
        }
        let bit = 1u64 << ((offset - 0x40) / 4);
        if self.seen & bit != 0 {
            self.next = 0;
            return None;
        }
        self.seen |= bit;
        let head = self.cfg.read32(self.bdf, offset as u16);
        self.next = (head >> 8) as u8;
        Some(Capability { id: head as u8, offset })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Function, Sim};
    extern crate std;
    use std::vec::Vec;

    fn caps(f: Function) -> Vec<(u8, u8)> {
        let s = Sim::new();
        let b = Bdf::new(0, 5, 0);
        s.add(b, f);
        let h = Header::read(&s, b).unwrap();
        Capabilities::new(&s, &h).map(|c| (c.id, c.offset)).collect()
    }

    fn base() -> Function {
        Function::endpoint(1, 2, [2, 0, 0])
    }

    #[test]
    fn walks_the_chain() {
        let f = base()
            .with8(reg::CAPABILITIES_POINTER, 0x40)
            .cap(0x40, cap_id::POWER_MANAGEMENT, 0x50, &[])
            .cap(0x50, cap_id::MSI, 0x70, &[])
            .cap(0x70, cap_id::MSI_X, 0x00, &[]);
        assert_eq!(caps(f), [(0x01, 0x40), (0x05, 0x50), (0x11, 0x70)]);
    }

    #[test]
    fn needs_the_status_bit() {
        let f = base().with8(reg::CAPABILITIES_POINTER, 0x40).with8(0x40, cap_id::MSI);
        assert!(caps(f).is_empty());
    }

    #[test]
    fn looping_chain_ends() {
        let f = base().with8(reg::CAPABILITIES_POINTER, 0x40).cap(0x40, cap_id::POWER_MANAGEMENT, 0x50, &[]).cap(
            0x50,
            cap_id::MSI,
            0x40,
            &[],
        );
        assert_eq!(caps(f), [(0x01, 0x40), (0x05, 0x50)]);
        // A capability pointing at itself.
        let f = base().with8(reg::CAPABILITIES_POINTER, 0x80).cap(0x80, cap_id::MSI_X, 0x80, &[]);
        assert_eq!(caps(f), [(0x11, 0x80)]);
    }

    #[test]
    fn longest_possible_chain() {
        // All 48 slots in use, the last pointing back at the first.
        let mut f = base().with8(reg::CAPABILITIES_POINTER, 0x40);
        for off in (0x40..=0xfc).step_by(4) {
            let next = if off == 0xfc { 0x40 } else { off + 4 };
            f = f.cap(off as u8, 0x09, next as u8, &[]);
        }
        assert_eq!(caps(f).len(), 48);
    }

    #[test]
    fn bad_pointers() {
        // Into the standard header: stop.
        let f = base().with8(reg::CAPABILITIES_POINTER, 0x40).cap(0x40, cap_id::MSI, 0x3c, &[]);
        assert_eq!(caps(f), [(0x05, 0x40)]);
        // Unaligned: the reserved low bits are masked.
        let f = base().with8(reg::CAPABILITIES_POINTER, 0x43).cap(0x40, cap_id::MSI, 0x52, &[]).cap(
            0x50,
            cap_id::MSI_X,
            0,
            &[],
        );
        assert_eq!(caps(f), [(0x05, 0x40), (0x11, 0x50)]);
    }

    #[test]
    fn find_and_fits() {
        let s = Sim::new();
        let b = Bdf::new(0, 5, 0);
        s.add(
            b,
            base().with8(reg::CAPABILITIES_POINTER, 0x40).cap(0x40, cap_id::VENDOR_SPECIFIC, 0xf0, &[]).cap(
                0xf0,
                cap_id::MSI,
                0,
                &[],
            ),
        );
        let h = Header::read(&s, b).unwrap();
        let msi = Capabilities::new(&s, &h).find_id(cap_id::MSI).unwrap();
        assert_eq!(msi.offset, 0xf0);
        assert!(msi.fits(16));
        assert!(!msi.fits(17));
        assert_eq!(Capabilities::new(&s, &h).find_id(cap_id::MSI_X), None);
    }
}
