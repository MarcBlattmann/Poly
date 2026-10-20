//! The x86 encoding of message signalled interrupts (SDM Vol. 3A §11.11,
//! "Message Signalled Interrupts").
//!
//! A PCI function with MSI or MSI-X raises an interrupt by writing `data` to
//! `address`. On x86 the address selects the local APIC(s) and the data the
//! vector, delivery mode and trigger mode, much like an I/O APIC redirection
//! entry. `poly-pci` puts the message into the device; this module builds it.
//!
//! ```text
//! address  31:20 0xFEE   19:12 destination ID   3 redirection hint   2 destination mode
//! data     15 trigger mode   14 level   10:8 delivery mode   7:0 vector
//! ```

use crate::{DeliveryMode, Trigger};

/// Bits 31:20 of every MSI address.
pub const ADDRESS_BASE: u64 = 0xfee0_0000;
/// Destination ID field, bits 19:12 (an 8-bit APIC ID in physical mode).
pub const DESTINATION_SHIFT: u32 = 12;
/// RH: redirection hint (lowest-priority arbitration among the destinations).
pub const REDIRECTION_HINT: u64 = 1 << 3;
/// DM: logical destination mode (with RH set); physical otherwise.
pub const LOGICAL: u64 = 1 << 2;

pub const DATA_DELIVERY_SHIFT: u32 = 8;
/// Level for a level-triggered message: 1 = assert. Ignored for edge.
pub const DATA_ASSERT: u32 = 1 << 14;
pub const DATA_LEVEL: u32 = 1 << 15;

/// An address / data pair for an MSI capability or an MSI-X table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MsiMessage {
    pub address: u64,
    pub data: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsiError {
    /// The address holds an 8-bit APIC ID; larger x2APIC IDs need interrupt
    /// remapping.
    DestinationTooWide,
    /// Vectors 0 to 15 are reserved for exceptions; fixed and
    /// lowest-priority delivery of them is illegal (§11.5.2).
    InvalidVector,
    /// Only fixed, lowest priority, SMI, NMI, INIT and ExtINT are defined
    /// for messages.
    InvalidDelivery,
}

impl MsiMessage {
    /// Fixed delivery of `vector` to the local APIC with ID `apic_id`,
    /// physical destination mode, edge-triggered: what a device driver
    /// normally wants.
    pub const fn fixed(apic_id: u32, vector: u8) -> Result<MsiMessage, MsiError> {
        MsiMessage::new(apic_id, vector, DeliveryMode::Fixed, Trigger::Edge)
    }

    /// Any combination §11.11 allows, physical destination mode.
    pub const fn new(
        apic_id: u32,
        vector: u8,
        delivery: DeliveryMode,
        trigger: Trigger,
    ) -> Result<MsiMessage, MsiError> {
        if apic_id > 0xff {
            return Err(MsiError::DestinationTooWide);
        }
        match delivery {
            DeliveryMode::StartUp => return Err(MsiError::InvalidDelivery),
            DeliveryMode::Fixed | DeliveryMode::LowestPriority if vector < 16 => {
                return Err(MsiError::InvalidVector);
            }
            _ => {}
        }
        let address = ADDRESS_BASE | (apic_id as u64) << DESTINATION_SHIFT;
        let mut data = vector as u32 | delivery.bits() << DATA_DELIVERY_SHIFT;
        if let Trigger::Level = trigger {
            data |= DATA_LEVEL | DATA_ASSERT;
        }
        Ok(MsiMessage { address, data })
    }

    pub const fn vector(self) -> u8 {
        self.data as u8
    }

    /// The physical-mode destination APIC ID.
    pub const fn destination(self) -> u8 {
        (self.address >> DESTINATION_SHIFT) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_edge_message() {
        let m = MsiMessage::fixed(0, 0x41).unwrap();
        assert_eq!(m, MsiMessage { address: 0xfee0_0000, data: 0x41 });
        let m = MsiMessage::fixed(3, 0xef).unwrap();
        assert_eq!(m.address, 0xfee0_3000);
        assert_eq!((m.vector(), m.destination()), (0xef, 3));
        assert_eq!(MsiMessage::fixed(0xff, 0x30).unwrap().address, 0xfeef_f000);
    }

    #[test]
    fn delivery_and_trigger_bits() {
        let m = MsiMessage::new(1, 0x50, DeliveryMode::LowestPriority, Trigger::Level).unwrap();
        assert_eq!(m.data, 0x50 | 0b001 << 8 | 1 << 14 | 1 << 15);
        // NMI ignores the vector, so a low one is fine.
        let m = MsiMessage::new(0, 0, DeliveryMode::Nmi, Trigger::Edge).unwrap();
        assert_eq!(m.data, 0b100 << 8);
    }

    #[test]
    fn rejects_what_cannot_be_encoded() {
        assert_eq!(MsiMessage::fixed(256, 0x30), Err(MsiError::DestinationTooWide));
        assert_eq!(MsiMessage::fixed(0, 15), Err(MsiError::InvalidVector));
        assert_eq!(MsiMessage::new(0, 0x30, DeliveryMode::StartUp, Trigger::Edge), Err(MsiError::InvalidDelivery));
    }
}
