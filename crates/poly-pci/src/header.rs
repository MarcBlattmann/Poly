//! The common configuration header (PCI 3.0 §6.2) and the bus numbers of a
//! PCI-to-PCI bridge (PCI-to-PCI Bridge 1.2 §3.2.5).

use crate::{Bdf, ConfigSpace, reg};

/// Command register bits (PCI 3.0 §6.2.2).
pub mod command {
    pub const IO_SPACE: u16 = 1 << 0;
    pub const MEMORY_SPACE: u16 = 1 << 1;
    pub const BUS_MASTER: u16 = 1 << 2;
    pub const PARITY_ERROR_RESPONSE: u16 = 1 << 6;
    pub const SERR_ENABLE: u16 = 1 << 8;
    pub const INTX_DISABLE: u16 = 1 << 10;
}

/// Status register bits (PCI 3.0 §6.2.3). The error bits are
/// write-one-to-clear.
pub mod status {
    pub const INTERRUPT_STATUS: u16 = 1 << 3;
    pub const CAPABILITIES_LIST: u16 = 1 << 4;
    pub const MASTER_DATA_PARITY_ERROR: u16 = 1 << 8;
    pub const SIGNALED_TARGET_ABORT: u16 = 1 << 11;
    pub const RECEIVED_TARGET_ABORT: u16 = 1 << 12;
    pub const RECEIVED_MASTER_ABORT: u16 = 1 << 13;
    pub const SIGNALED_SYSTEM_ERROR: u16 = 1 << 14;
    pub const DETECTED_PARITY_ERROR: u16 = 1 << 15;
}

/// Layout of the rest of the header, from bits 6:0 of the header type
/// register (PCI 3.0 §6.2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeaderType {
    /// Type 0: an ordinary function.
    Endpoint,
    /// Type 1: PCI-to-PCI bridge.
    Bridge,
    /// Type 2: CardBus bridge.
    CardBus,
    Unknown(u8),
}

impl HeaderType {
    pub const fn from_raw(raw: u8) -> HeaderType {
        match raw & 0x7f {
            0 => HeaderType::Endpoint,
            1 => HeaderType::Bridge,
            2 => HeaderType::CardBus,
            other => HeaderType::Unknown(other),
        }
    }

    /// Number of base address registers this layout has.
    pub const fn bar_count(self) -> usize {
        match self {
            HeaderType::Endpoint => 6,
            HeaderType::Bridge => 2,
            HeaderType::CardBus | HeaderType::Unknown(_) => 0,
        }
    }
}

/// Class code triple (PCI 3.0 §6.2.1 and appendix D).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClassCode {
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
}

impl ClassCode {
    pub const fn is_pci_bridge(self) -> bool {
        self.class == 0x06 && self.subclass == 0x04
    }
}

/// Bus numbers of a type 1 header.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BridgeBuses {
    pub primary: u8,
    /// Bus directly behind the bridge.
    pub secondary: u8,
    /// Highest bus behind the bridge.
    pub subordinate: u8,
}

/// A snapshot of a function's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub bdf: Bdf,
    pub vendor_id: u16,
    pub device_id: u16,
    pub revision: u8,
    pub class: ClassCode,
    pub header_type: HeaderType,
    /// Bit 7 of the header type: functions 1-7 may exist. Only meaningful
    /// on function 0.
    pub multifunction: bool,
    pub command: u16,
    pub status: u16,
    pub interrupt_line: u8,
    /// 0 for none, 1-4 for INTA#-INTD#. Anything else is a broken device.
    pub interrupt_pin: u8,
    /// Zero unless the header is type 0.
    pub subsystem_vendor_id: u16,
    pub subsystem_id: u16,
    /// Present for type 1 headers.
    pub bridge: Option<BridgeBuses>,
}

impl Header {
    /// Read a function's header. `None` if no function answers at `bdf`.
    pub fn read<C: ConfigSpace + ?Sized>(cfg: &C, bdf: Bdf) -> Option<Header> {
        let id = cfg.read32(bdf, reg::VENDOR_ID);
        let vendor_id = id as u16;
        // Absent functions read all ones (PCI 3.0 §6.2.1). 0 is no valid
        // vendor either, and is what some broken bridges return.
        if vendor_id == 0xffff || vendor_id == 0 {
            return None;
        }
        let cmd_status = cfg.read32(bdf, reg::COMMAND);
        let class = cfg.read32(bdf, reg::REVISION_ID);
        let ht = cfg.read8(bdf, reg::HEADER_TYPE);
        let header_type = HeaderType::from_raw(ht);
        let intr = cfg.read32(bdf, reg::INTERRUPT_LINE);
        let (subsystem_vendor_id, subsystem_id) = match header_type {
            HeaderType::Endpoint => {
                let s = cfg.read32(bdf, reg::SUBSYSTEM_VENDOR_ID);
                (s as u16, (s >> 16) as u16)
            }
            _ => (0, 0),
        };
        let bridge = match header_type {
            HeaderType::Bridge => {
                let b = cfg.read32(bdf, reg::PRIMARY_BUS);
                Some(BridgeBuses { primary: b as u8, secondary: (b >> 8) as u8, subordinate: (b >> 16) as u8 })
            }
            _ => None,
        };
        Some(Header {
            bdf,
            vendor_id,
            device_id: (id >> 16) as u16,
            revision: class as u8,
            class: ClassCode { class: (class >> 24) as u8, subclass: (class >> 16) as u8, prog_if: (class >> 8) as u8 },
            header_type,
            multifunction: ht & 0x80 != 0,
            command: cmd_status as u16,
            status: (cmd_status >> 16) as u16,
            interrupt_line: intr as u8,
            interrupt_pin: (intr >> 8) as u8,
            subsystem_vendor_id,
            subsystem_id,
            bridge,
        })
    }
}

/// Current command register.
pub fn read_command<C: ConfigSpace + ?Sized>(cfg: &C, bdf: Bdf) -> u16 {
    cfg.read16(bdf, reg::COMMAND)
}

/// Write the command register. A 16-bit write, so the status register's
/// write-one-to-clear bits are left alone.
pub fn write_command<C: ConfigSpace + ?Sized>(cfg: &C, bdf: Bdf, value: u16) {
    cfg.write16(bdf, reg::COMMAND, value);
}

/// Set `bits` in the command register (e.g. memory decode and bus
/// mastering before using a device), leaving the others as they are.
pub fn enable<C: ConfigSpace + ?Sized>(cfg: &C, bdf: Bdf, bits: u16) {
    write_command(cfg, bdf, read_command(cfg, bdf) | bits);
}

/// Clear `bits` in the command register.
pub fn disable<C: ConfigSpace + ?Sized>(cfg: &C, bdf: Bdf, bits: u16) {
    write_command(cfg, bdf, read_command(cfg, bdf) & !bits);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Function, Sim};

    #[test]
    fn parses_endpoint() {
        let s = Sim::new();
        let b = Bdf::new(0, 3, 0);
        s.add(
            b,
            Function::endpoint(0x1af4, 0x1041, [0x02, 0x00, 0x00])
                .with8(reg::REVISION_ID, 1)
                .with16(reg::SUBSYSTEM_VENDOR_ID, 0x1af4)
                .with16(reg::SUBSYSTEM_ID, 0x0001)
                .with8(reg::INTERRUPT_LINE, 11)
                .with8(reg::INTERRUPT_PIN, 1)
                .status_bits(status::CAPABILITIES_LIST),
        );
        let h = Header::read(&s, b).unwrap();
        assert_eq!((h.vendor_id, h.device_id, h.revision), (0x1af4, 0x1041, 1));
        assert_eq!(h.class, ClassCode { class: 2, subclass: 0, prog_if: 0 });
        assert_eq!(h.header_type, HeaderType::Endpoint);
        assert!(!h.multifunction);
        assert_eq!((h.interrupt_line, h.interrupt_pin), (11, 1));
        assert_eq!((h.subsystem_vendor_id, h.subsystem_id), (0x1af4, 1));
        assert_ne!(h.status & status::CAPABILITIES_LIST, 0);
        assert_eq!(h.bridge, None);
    }

    #[test]
    fn parses_bridge_and_multifunction() {
        let s = Sim::new();
        let b = Bdf::new(0, 1, 0);
        s.add(b, Function::bridge(0, 4, 9).multifunction());
        let h = Header::read(&s, b).unwrap();
        assert_eq!(h.header_type, HeaderType::Bridge);
        assert!(h.multifunction);
        assert!(h.class.is_pci_bridge());
        assert_eq!(h.bridge, Some(BridgeBuses { primary: 0, secondary: 4, subordinate: 9 }));
        assert_eq!(h.header_type.bar_count(), 2);
    }

    #[test]
    fn absent_and_zero_vendor() {
        let s = Sim::new();
        assert_eq!(Header::read(&s, Bdf::new(0, 0, 0)), None);
        s.add(Bdf::new(0, 0, 0), Function::endpoint(0, 0, [0; 3]));
        assert_eq!(Header::read(&s, Bdf::new(0, 0, 0)), None);
    }

    #[test]
    fn header_type_raw() {
        assert_eq!(HeaderType::from_raw(0x81), HeaderType::Bridge);
        assert_eq!(HeaderType::from_raw(0x02), HeaderType::CardBus);
        assert_eq!(HeaderType::from_raw(0x7f), HeaderType::Unknown(0x7f));
        assert_eq!(HeaderType::Unknown(5).bar_count(), 0);
    }

    #[test]
    fn command_writes_leave_status_alone() {
        let s = Sim::new();
        let b = Bdf::new(0, 2, 0);
        s.add(b, Function::endpoint(1, 2, [0; 3]).status_bits(status::DETECTED_PARITY_ERROR));
        enable(&s, b, command::MEMORY_SPACE | command::BUS_MASTER);
        assert_eq!(read_command(&s, b), command::MEMORY_SPACE | command::BUS_MASTER);
        disable(&s, b, command::BUS_MASTER);
        assert_eq!(read_command(&s, b), command::MEMORY_SPACE);
        // The write-one-to-clear error bit survived both writes.
        assert_ne!(s.get16(b, reg::STATUS) & status::DETECTED_PARITY_ERROR, 0);
    }
}
