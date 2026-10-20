//! virtio devices over PCI.
//!
//! The pieces every virtio driver shares, without the hardware access:
//!
//! - [`PciTransport`] finds the virtio structures (common configuration,
//!   notification area, ISR status, device configuration) through the
//!   function's vendor-specific PCI capabilities.
//! - [`CommonCfg`] wraps the common configuration registers, and [`begin`]
//!   runs device initialisation as a small state machine: reset,
//!   ACKNOWLEDGE, DRIVER, feature negotiation and FEATURES_OK, virtqueue
//!   setup, DRIVER_OK.
//! - [`SplitQueue`] drives a split virtqueue in caller-provided memory.
//!
//! Register access goes through [`poly_pci::ConfigSpace`] and
//! [`poly_pci::Mmio`], which the kernel implements. Only the modern (virtio
//! 1.0 and later) interface is supported; legacy-only devices are refused.
//!
//! Source: OASIS Virtual I/O Device (VIRTIO) Version 1.2 specification,
//! chiefly §2 (basic facilities), §3.1 (device initialisation) and §4.1
//! (virtio over PCI). Section numbers below refer to it. As in `poly-pci`,
//! everything the device provides is untrusted.

#![no_std]

mod common;
mod pci;
mod queue;

pub use common::{COMMON_CFG_MIN_LEN, CommonCfg, Configuring, FeatureNegotiation, InitError, NO_VECTOR, begin};
pub use pci::{Isr, Notifier, PciCfgWindow, PciTransport, Region, TransportError, Width, cfg_type};
pub use queue::{AddError, Buffer, DescState, Layout, QueueAddresses, SetupError, SplitQueue, Used, UsedError};

use poly_pci::Header;

/// PCI vendor ID of every virtio device (§4.1.2).
pub const PCI_VENDOR_ID: u16 = 0x1af4;

/// Virtio device IDs (§5), as reported by [`device_id_of`].
pub mod device_id {
    pub const NETWORK: u16 = 1;
    pub const BLOCK: u16 = 2;
    pub const CONSOLE: u16 = 3;
    pub const ENTROPY: u16 = 4;
    pub const BALLOON: u16 = 5;
    pub const SCSI: u16 = 8;
    pub const NINEP: u16 = 9;
    pub const GPU: u16 = 16;
    pub const INPUT: u16 = 18;
    pub const SOCKET: u16 = 19;
    pub const SOUND: u16 = 25;
    pub const FILESYSTEM: u16 = 26;
}

/// The virtio device ID of a PCI function, or `None` if it is not a
/// virtio device (§4.1.2). Modern devices use PCI device ID 0x1040 plus the
/// type; transitional ones (0x1000-0x103f) carry the ID in the subsystem
/// ID instead.
pub fn device_id_of(header: &Header) -> Option<u16> {
    if header.vendor_id != PCI_VENDOR_ID {
        return None;
    }
    match header.device_id {
        0x1040..=0x107f => Some(header.device_id - 0x1040),
        0x1000..=0x103f if header.subsystem_id != 0 => Some(header.subsystem_id),
        _ => None,
    }
}

/// Feature bits defined by the specification itself rather than by a device
/// type (§6). Device-specific features use bits 0-23 and 50 and up.
pub mod feature {
    pub const INDIRECT_DESC: u64 = 1 << 28;
    pub const EVENT_IDX: u64 = 1 << 29;
    /// A modern (1.0 or later) device. Required.
    pub const VERSION_1: u64 = 1 << 32;
    /// The device goes through the platform's IOMMU: buffer addresses are
    /// I/O virtual addresses. A driver should accept it when offered, or
    /// the device may refuse to work.
    pub const ACCESS_PLATFORM: u64 = 1 << 33;
    pub const RING_PACKED: u64 = 1 << 34;
    pub const IN_ORDER: u64 = 1 << 35;
    pub const ORDER_PLATFORM: u64 = 1 << 36;
    pub const SR_IOV: u64 = 1 << 37;
    pub const NOTIFICATION_DATA: u64 = 1 << 38;
    pub const NOTIF_CONFIG_DATA: u64 = 1 << 39;
    pub const RING_RESET: u64 = 1 << 40;

    /// Features whose protocol this crate does not implement; [`crate::FeatureNegotiation::accept`]
    /// never accepts them. Packed rings, indirect descriptors and
    /// notification data change the queue or notification format; with
    /// IN_ORDER the device may report a whole batch with one used element,
    /// which [`crate::SplitQueue`] does not expect.
    pub const UNSUPPORTED: u64 = INDIRECT_DESC | RING_PACKED | IN_ORDER | NOTIFICATION_DATA | NOTIF_CONFIG_DATA;
}

/// Device status bits (§2.1).
pub mod status {
    /// The guest has noticed the device.
    pub const ACKNOWLEDGE: u8 = 1;
    /// The guest knows how to drive it.
    pub const DRIVER: u8 = 2;
    /// The driver is set up and ready.
    pub const DRIVER_OK: u8 = 4;
    /// Feature negotiation is complete.
    pub const FEATURES_OK: u8 = 8;
    /// The device hit an error it cannot recover from without a reset.
    pub const DEVICE_NEEDS_RESET: u8 = 64;
    /// The driver gave up on the device.
    pub const FAILED: u8 = 128;
}

#[cfg(test)]
mod sim;

#[cfg(test)]
mod tests {
    use super::*;
    use poly_pci::{Bdf, ClassCode, HeaderType};

    fn header(vendor_id: u16, device_id: u16, subsystem_id: u16) -> Header {
        Header {
            bdf: Bdf::new(0, 0, 0),
            vendor_id,
            device_id,
            revision: 1,
            class: ClassCode { class: 0, subclass: 0, prog_if: 0 },
            header_type: HeaderType::Endpoint,
            multifunction: false,
            command: 0,
            status: 0,
            interrupt_line: 0,
            interrupt_pin: 0,
            subsystem_vendor_id: PCI_VENDOR_ID,
            subsystem_id,
            bridge: None,
        }
    }

    /// Everything together, the way a driver will use it: discover the
    /// structures, initialise the device, set up a queue, then push
    /// requests through it, notifying when the device asks for it.
    #[test]
    fn end_to_end() {
        use crate::sim::{Bytes, CommonDevice, Config, DeviceQueue, DmaMemory};
        extern crate std;
        use std::{vec, vec::Vec};

        let pci = Config::new();
        pci.put(0, &0x1042_1af4u32.to_le_bytes());
        pci.put(0x06, &poly_pci::status::CAPABILITIES_LIST.to_le_bytes());
        pci.put(0x34, &[0x40]);
        pci.put(0x40, &[0x09, 0x50, 16, cfg_type::COMMON, 4, 0, 0, 0, 0, 0, 0, 0, 0x38, 0, 0, 0]);
        pci.put(0x50, &[0x09, 0x60, 16, cfg_type::ISR, 4, 0, 0, 0, 0, 0x10, 0, 0, 1, 0, 0, 0]);
        pci.put(0x60, &[0x09, 0x00, 20, cfg_type::NOTIFY, 4, 0, 0, 0, 0, 0x20, 0, 0, 0x10, 0, 0, 0, 2, 0, 0, 0]);
        let h = Header::read(&pci, Bdf::new(0, 2, 0)).unwrap();
        assert_eq!(device_id_of(&h), Some(device_id::BLOCK));
        let t = PciTransport::discover(&pci, &h).unwrap();

        let common = CommonCfg::new(CommonDevice::new(feature::VERSION_1 | feature::EVENT_IDX, &[64, 64]));
        let notify = Bytes { mem: vec![0; 16].into(), ..Bytes::default() };
        let cfg = begin(&common, 100).unwrap().accept(feature::EVENT_IDX).unwrap();
        let event_idx = cfg.has(feature::EVENT_IDX);
        assert!(event_idx);

        let size = cfg.queue_max_size(1).unwrap().min(16);
        let layout = Layout::new(size).unwrap();
        let mem = DmaMemory::new(layout.total());
        let mut state = vec![DescState::default(); size as usize];
        // SAFETY: the memory outlives the queue and only the queue and the
        // simulated device use it.
        let mut q = unsafe { SplitQueue::new(mem.ptr(), mem.phys(), layout, &mut state, event_idx) }.unwrap();
        let notify_off = cfg.enable_queue(1, size, q.addresses(), None).unwrap();
        let notifier = t.notifier(1, notify_off).unwrap();
        cfg.driver_ok();
        assert_eq!(common.status(), status::ACKNOWLEDGE | status::DRIVER | status::FEATURES_OK | status::DRIVER_OK);

        // The device side sees the queue through the registers the driver
        // programmed, not through the driver's objects.
        let regs = common.regs().queues.borrow()[1];
        assert_eq!(regs.enable, 1);
        let addrs = QueueAddresses { desc: regs.desc, driver: regs.driver, device: regs.device };
        let mut dev = DeviceQueue::new(addrs, regs.size, event_idx);

        let mut kicks = 0;
        for round in 0..200u32 {
            let mut reqs: Vec<[u8; 4]> = (0..3).map(|i| (round * 3 + i).to_be_bytes()).collect();
            let mut resps = vec![[0u8; 8]; 3];
            for (req, resp) in reqs.iter_mut().zip(resps.iter_mut()) {
                let r = Buffer { addr: req.as_mut_ptr() as u64, len: 4 };
                let w = Buffer { addr: resp.as_mut_ptr() as u64, len: 8 };
                q.add(&[r], &[w]).unwrap();
            }
            if q.should_notify() {
                notifier.notify(&notify);
                kicks += 1;
                // The simulated device only runs when kicked, and asks for
                // the next kick once it has drained the ring.
                dev.process(|d| [d, d].concat());
            }
            let mut got = 0;
            while let Some(u) = q.pop_used().unwrap() {
                assert_eq!(u.len, 8);
                got += 1;
            }
            assert_eq!(got, 3);
            for (req, resp) in reqs.iter().zip(&resps) {
                assert_eq!(&resp[..4], req);
                assert_eq!(&resp[4..], req);
            }
        }
        assert_eq!(kicks, 200);
        assert!(notify.writes.borrow().iter().all(|&w| w == (2, 1)));
    }

    #[test]
    fn device_types() {
        assert_eq!(device_id_of(&header(0x1af4, 0x1042, 0)), Some(device_id::BLOCK));
        assert_eq!(device_id_of(&header(0x1af4, 0x1041, 0)), Some(device_id::NETWORK));
        assert_eq!(device_id_of(&header(0x1af4, 0x1000, 1)), Some(device_id::NETWORK));
        assert_eq!(device_id_of(&header(0x1af4, 0x1000, 0)), None);
        assert_eq!(device_id_of(&header(0x1af4, 0x1100, 0)), None);
        assert_eq!(device_id_of(&header(0x8086, 0x1042, 0)), None);
    }
}
