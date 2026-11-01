//! Finding the virtio structures of a PCI function (§4.1.4).
//!
//! Each structure is described by a vendor-specific PCI capability:
//!
//! ```text
//! +0   cap_vndr (0x09), cap_next, cap_len, cfg_type
//! +4   bar, id, padding[2]
//! +8   le32 offset    within the BAR
//! +12  le32 length
//! +16  le32 notify_off_multiplier   (notification capability only)
//! +16  u8 pci_cfg_data[4]            (PCI configuration access capability only)
//! ```
//!
//! A device may offer several capabilities of one type, best first; the
//! driver uses the first one it can (§4.1.4.1). Capabilities with an unknown
//! type, a reserved BAR number or a length too short for their type are
//! skipped as if absent.

use crate::common::COMMON_CFG_MIN_LEN;
use poly_pci::{Bar, Bdf, Capabilities, ConfigSpace, Header, Mmio, cap_id};

/// `cfg_type` values (§4.1.4).
pub mod cfg_type {
    pub const COMMON: u8 = 1;
    pub const NOTIFY: u8 = 2;
    pub const ISR: u8 = 3;
    pub const DEVICE: u8 = 4;
    pub const PCI: u8 = 5;
    pub const SHARED_MEMORY: u8 = 8;
    pub const VENDOR: u8 = 9;
}

const CAP_LEN: usize = 16;
const NOTIFY_CAP_LEN: usize = 20;
const PCI_CFG_CAP_LEN: usize = 20;

/// Where a structure lives: a range of one of the function's BARs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub bar: u8,
    pub offset: u32,
    pub length: u32,
}

impl Region {
    /// Bus address (or port) of the region, if it lies inside its BAR.
    /// `bars` comes from [`poly_pci::probe_bars`].
    pub fn address(&self, bars: &[Option<Bar>; 6]) -> Option<u64> {
        let bar = bars.get(self.bar as usize).copied().flatten()?;
        if !bar.contains(self.offset as u64, self.length as u64) {
            return None;
        }
        bar.address.checked_add(self.offset as u64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    /// No usable capability for a structure every device must have.
    MissingCommon,
    MissingNotify,
    MissingIsr,
    /// A structure does not fit in its BAR.
    OutsideBar,
    /// A PCI configuration access with a bad BAR or misaligned offset.
    BadWindowAccess,
}

/// The virtio structures of one PCI function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PciTransport {
    /// At least [`crate::COMMON_CFG_MIN_LEN`] bytes.
    pub common: Region,
    /// At least 2 bytes.
    pub notify: Region,
    /// Queue `q`'s notification register is `notify_off(q) * multiplier`
    /// bytes into the notification region; 0 means one register for all.
    pub notify_off_multiplier: u32,
    /// At least 1 byte.
    pub isr: Region,
    /// Device-specific configuration; not every device type has one.
    pub device: Option<Region>,
    /// Indirect access to the BARs through configuration space.
    pub pci_cfg: Option<PciCfgWindow>,
}

impl PciTransport {
    /// Walk the function's capabilities and pick out the virtio structures.
    pub fn discover<C: ConfigSpace + ?Sized>(cfg: &C, header: &Header) -> Result<PciTransport, TransportError> {
        let bdf = header.bdf;
        let (mut common, mut notify, mut isr, mut device, mut pci_cfg) = (None, None, None, None, None);
        let mut multiplier = 0;
        for cap in Capabilities::new(cfg, header).filter(|c| c.id == cap_id::VENDOR_SPECIFIC) {
            if !cap.fits(CAP_LEN) {
                continue;
            }
            let base = cap.offset as u16;
            let head = cfg.read32(bdf, base);
            let cap_len = (head >> 16) as u8 as usize;
            let ty = (head >> 24) as u8;
            let bar = cfg.read8(bdf, base + 4);
            let region = Region { bar, offset: cfg.read32(bdf, base + 8), length: cfg.read32(bdf, base + 12) };
            if cap_len < CAP_LEN {
                continue;
            }
            // The PCI access capability's bar / offset / length fields are
            // the window's registers, not a location.
            if ty == cfg_type::PCI {
                if pci_cfg.is_none() && cap_len >= PCI_CFG_CAP_LEN && cap.fits(PCI_CFG_CAP_LEN) {
                    pci_cfg = Some(PciCfgWindow { offset: cap.offset });
                }
                continue;
            }
            if bar > 5 {
                continue;
            }
            match ty {
                cfg_type::COMMON if common.is_none() && region.length >= COMMON_CFG_MIN_LEN => {
                    common = Some(region);
                }
                cfg_type::NOTIFY
                    if notify.is_none()
                        && region.length >= 2
                        && cap_len >= NOTIFY_CAP_LEN
                        && cap.fits(NOTIFY_CAP_LEN) =>
                {
                    notify = Some(region);
                    multiplier = cfg.read32(bdf, base + 16);
                }
                cfg_type::ISR if isr.is_none() && region.length >= 1 => isr = Some(region),
                cfg_type::DEVICE if device.is_none() => device = Some(region),
                _ => {}
            }
        }
        Ok(PciTransport {
            common: common.ok_or(TransportError::MissingCommon)?,
            notify: notify.ok_or(TransportError::MissingNotify)?,
            notify_off_multiplier: multiplier,
            isr: isr.ok_or(TransportError::MissingIsr)?,
            device,
            pci_cfg,
        })
    }

    /// Check that every structure lies inside its BAR, before mapping them.
    pub fn check_bars(&self, bars: &[Option<Bar>; 6]) -> Result<(), TransportError> {
        let regions = [Some(self.common), Some(self.notify), Some(self.isr), self.device];
        if regions.iter().flatten().all(|r| r.address(bars).is_some()) {
            Ok(())
        } else {
            Err(TransportError::OutsideBar)
        }
    }

    /// The notification register of queue `queue`, whose `queue_notify_off`
    /// the device reported during setup (§4.1.4.4). `None` if the device's
    /// numbers put it outside the notification region.
    pub fn notifier(&self, queue: u16, queue_notify_off: u16) -> Option<Notifier> {
        let offset = queue_notify_off as u64 * self.notify_off_multiplier as u64;
        if offset + 2 > self.notify.length as u64 {
            return None;
        }
        Some(Notifier { offset: offset as usize, queue })
    }
}

/// Length of `struct virtio_pci_cap64` (§4.1.4.7): the common capability
/// fields plus `offset_hi` and `length_hi`.
const CAP64_LEN: usize = 24;

/// A shared memory region (§2.10, §4.1.4.7): memory of the device the
/// driver maps, in a BAR, named by its `shmid` (the capability's `id`
/// byte). Offset and length are 64-bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SharedMemory {
    pub id: u8,
    pub bar: u8,
    pub offset: u64,
    pub length: u64,
}

impl SharedMemory {
    /// Bus address of the region, if it lies inside its BAR.
    pub fn address(&self, bars: &[Option<Bar>; 6]) -> Option<u64> {
        let bar = bars.get(self.bar as usize).copied().flatten()?;
        if !bar.contains(self.offset, self.length) {
            return None;
        }
        bar.address.checked_add(self.offset)
    }
}

/// The shared memory region `id` of a function (`VIRTIO_PCI_CAP_SHARED_MEMORY_CFG`
/// capabilities, §4.1.4.7), the first usable one if the device lists it
/// more than once. Kept apart from [`PciTransport::discover`]: only some
/// device types have shared memory (the GPU's host-visible region, §5.7.4).
pub fn find_shared_memory<C: ConfigSpace + ?Sized>(cfg: &C, header: &Header, id: u8) -> Option<SharedMemory> {
    let bdf = header.bdf;
    Capabilities::new(cfg, header).filter(|c| c.id == cap_id::VENDOR_SPECIFIC).find_map(|cap| {
        if !cap.fits(CAP64_LEN) {
            return None;
        }
        let base = cap.offset as u16;
        let head = cfg.read32(bdf, base);
        let cap_len = (head >> 16) as u8 as usize;
        if (head >> 24) as u8 != cfg_type::SHARED_MEMORY || cap_len < CAP64_LEN {
            return None;
        }
        let bar = cfg.read8(bdf, base + 4);
        if bar > 5 || cfg.read8(bdf, base + 5) != id {
            return None;
        }
        let lo_hi = |at: u16| cfg.read32(bdf, base + at) as u64 | (cfg.read32(bdf, base + at + 8) as u64) << 32;
        let region = SharedMemory { id, bar, offset: lo_hi(8), length: lo_hi(12) };
        (region.length != 0).then_some(region)
    })
}

/// Tells the device a queue has new buffers (§4.1.5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Notifier {
    offset: usize,
    queue: u16,
}

impl Notifier {
    /// Offset of the register within the notification region.
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Write the queue index to the register. `notify` maps the
    /// notification region. (With VIRTIO_F_NOTIFICATION_DATA the format
    /// would differ; this crate never negotiates it.)
    pub fn notify<M: Mmio + ?Sized>(&self, notify: &M) {
        notify.write16(self.offset, self.queue);
    }
}

/// The ISR status byte (§4.1.4.5), used with INTx interrupts. Reading it
/// acknowledges the interrupt and clears it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Isr {
    /// A virtqueue has used buffers.
    pub queue: bool,
    /// The device configuration changed.
    pub config: bool,
}

impl Isr {
    /// Read (and so clear) the status. `isr` maps the ISR region.
    pub fn read<M: Mmio + ?Sized>(isr: &M) -> Isr {
        let v = isr.read8(0);
        Isr { queue: v & 1 != 0, config: v & 2 != 0 }
    }
}

/// Access width for [`PciCfgWindow`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    U8 = 1,
    U16 = 2,
    U32 = 4,
}

/// The PCI configuration access capability (§4.1.4, "PCI configuration access capability"): a window through
/// which the driver reaches any BAR via configuration space, for when the
/// BARs cannot be mapped (early boot, for one). Set the BAR, offset and
/// width, then read or write `pci_cfg_data`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PciCfgWindow {
    pub offset: u8,
}

impl PciCfgWindow {
    pub fn read<C: ConfigSpace + ?Sized>(
        &self,
        cfg: &C,
        bdf: Bdf,
        bar: u8,
        offset: u32,
        width: Width,
    ) -> Result<u32, TransportError> {
        let data = self.select(cfg, bdf, bar, offset, width)?;
        Ok(match width {
            Width::U8 => cfg.read8(bdf, data) as u32,
            Width::U16 => cfg.read16(bdf, data) as u32,
            Width::U32 => cfg.read32(bdf, data),
        })
    }

    pub fn write<C: ConfigSpace + ?Sized>(
        &self,
        cfg: &C,
        bdf: Bdf,
        bar: u8,
        offset: u32,
        width: Width,
        value: u32,
    ) -> Result<(), TransportError> {
        let data = self.select(cfg, bdf, bar, offset, width)?;
        match width {
            Width::U8 => cfg.write8(bdf, data, value as u8),
            Width::U16 => cfg.write16(bdf, data, value as u16),
            Width::U32 => cfg.write32(bdf, data, value),
        }
        Ok(())
    }

    /// Point the window at `bar` + `offset`; returns where `pci_cfg_data`
    /// is. The offset must be aligned to the width, as that section requires.
    fn select<C: ConfigSpace + ?Sized>(
        &self,
        cfg: &C,
        bdf: Bdf,
        bar: u8,
        offset: u32,
        width: Width,
    ) -> Result<u16, TransportError> {
        if bar > 5 || !offset.is_multiple_of(width as u32) {
            return Err(TransportError::BadWindowAccess);
        }
        let base = self.offset as u16;
        cfg.write8(bdf, base + 4, bar);
        cfg.write32(bdf, base + 8, offset);
        cfg.write32(bdf, base + 12, width as u32);
        Ok(base + 16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Bytes, Config};
    use poly_pci::{BarKind, status};
    extern crate std;
    use std::vec;

    const B: Bdf = Bdf::new(0, 4, 0);

    fn header(cap_ptr: u8) -> (Config, Header) {
        let c = Config::new();
        c.put(0, &0x1042_1af4u32.to_le_bytes());
        c.put(0x06, &status::CAPABILITIES_LIST.to_le_bytes());
        c.put(0x34, &[cap_ptr]);
        let h = Header::read(&c, B).unwrap();
        (c, h)
    }

    fn reg(bar: u8, offset: u32, length: u32) -> Region {
        Region { bar, offset, length }
    }

    /// A vendor capability; `extra` is the dword at +16, if any.
    fn vcap(c: &Config, at: u8, next: u8, ty: u8, r: Region, extra: Option<u32>) {
        let Region { bar, offset, length } = r;
        let len = if extra.is_some() { 20 } else { 16 };
        c.put(at, &[0x09, next, len, ty, bar, 0, 0, 0]);
        c.put(at + 8, &offset.to_le_bytes());
        c.put(at + 12, &length.to_le_bytes());
        if let Some(x) = extra {
            c.put(at + 16, &x.to_le_bytes());
        }
    }

    /// The layout QEMU gives a modern virtio-pci device: MSI-X first, then
    /// the virtio structures in BAR 4.
    fn qemu_like() -> (Config, Header) {
        let (c, h) = header(0x98);
        c.put(0x98, &[0x11, 0x88, 0, 0]);
        vcap(&c, 0x88, 0x78, cfg_type::COMMON, reg(4, 0x0000, 0x1000), None);
        vcap(&c, 0x78, 0x68, cfg_type::ISR, reg(4, 0x1000, 0x1000), None);
        vcap(&c, 0x68, 0x54, cfg_type::DEVICE, reg(4, 0x2000, 0x1000), None);
        vcap(&c, 0x54, 0x40, cfg_type::NOTIFY, reg(4, 0x3000, 0x1000), Some(4));
        vcap(&c, 0x40, 0x00, cfg_type::PCI, reg(0, 0, 0), Some(0));
        (c, h)
    }

    #[test]
    fn discovers_qemu_layout() {
        let (c, h) = qemu_like();
        let t = PciTransport::discover(&c, &h).unwrap();
        assert_eq!(t.common, Region { bar: 4, offset: 0, length: 0x1000 });
        assert_eq!(t.isr, Region { bar: 4, offset: 0x1000, length: 0x1000 });
        assert_eq!(t.device, Some(Region { bar: 4, offset: 0x2000, length: 0x1000 }));
        assert_eq!(t.notify, Region { bar: 4, offset: 0x3000, length: 0x1000 });
        assert_eq!(t.notify_off_multiplier, 4);
        assert_eq!(t.pci_cfg, Some(PciCfgWindow { offset: 0x40 }));

        let mut bars = [None; 6];
        bars[4] = Some(Bar { kind: BarKind::Memory64, address: 0xfe00_0000, size: 0x4000, prefetchable: true });
        assert_eq!(t.check_bars(&bars), Ok(()));
        assert_eq!(t.notify.address(&bars), Some(0xfe00_3000));
        bars[4] = Some(Bar { kind: BarKind::Memory64, address: 0xfe00_0000, size: 0x3800, prefetchable: true });
        assert_eq!(t.check_bars(&bars), Err(TransportError::OutsideBar));
        assert_eq!(t.check_bars(&[None; 6]), Err(TransportError::OutsideBar));
    }

    #[test]
    fn skips_unusable_capabilities() {
        let (c, h) = header(0x40);
        // Common: reserved BAR, too short a cap_len, too short a region,
        // then a good one; a second good one is ignored.
        vcap(&c, 0x40, 0x50, cfg_type::COMMON, reg(7, 0, 0x1000), None);
        vcap(&c, 0x50, 0x60, cfg_type::COMMON, reg(0, 0, 0x1000), None);
        c.put(0x52, &[8]);
        vcap(&c, 0x60, 0x70, cfg_type::COMMON, reg(0, 0, 0x37), None);
        vcap(&c, 0x70, 0x80, cfg_type::COMMON, reg(1, 0x100, 0x38), None);
        vcap(&c, 0x80, 0x90, cfg_type::COMMON, reg(2, 0, 0x1000), None);
        // Notify without its multiplier field, then a good one.
        vcap(&c, 0x90, 0xa0, cfg_type::NOTIFY, reg(0, 0, 0x100), None);
        vcap(&c, 0xa0, 0xb8, cfg_type::NOTIFY, reg(0, 0x200, 0x100), Some(0));
        // An unknown type, then the ISR.
        vcap(&c, 0xb8, 0xc8, 0x42, reg(0, 0, 0x100), None);
        vcap(&c, 0xc8, 0xf4, cfg_type::ISR, reg(0, 0x300, 1), None);
        // A capability running off the end of the header.
        c.put(0xf4, &[0x09, 0, 16, cfg_type::DEVICE]);
        let t = PciTransport::discover(&c, &h).unwrap();
        assert_eq!(t.common, Region { bar: 1, offset: 0x100, length: 0x38 });
        assert_eq!(t.notify, Region { bar: 0, offset: 0x200, length: 0x100 });
        assert_eq!(t.notify_off_multiplier, 0);
        assert_eq!(t.isr, Region { bar: 0, offset: 0x300, length: 1 });
        assert_eq!((t.device, t.pci_cfg), (None, None));
    }

    /// A `virtio_pci_cap64` shared memory capability.
    #[allow(clippy::too_many_arguments)]
    fn shm_cap(c: &Config, at: u8, next: u8, len: u8, bar: u8, id: u8, offset: u64, length: u64) {
        c.put(at, &[0x09, next, len, cfg_type::SHARED_MEMORY, bar, id, 0, 0]);
        c.put(at + 8, &(offset as u32).to_le_bytes());
        c.put(at + 12, &(length as u32).to_le_bytes());
        c.put(at + 16, &((offset >> 32) as u32).to_le_bytes());
        c.put(at + 20, &((length >> 32) as u32).to_le_bytes());
    }

    #[test]
    fn finds_shared_memory_regions() {
        let (c, h) = header(0x40);
        // Region 2 first; a capability too short for the 64-bit fields; a
        // reserved BAR; then region 1 with 64-bit offset and length.
        shm_cap(&c, 0x40, 0x58, 24, 2, 2, 0x1000, 0x2000);
        shm_cap(&c, 0x58, 0x70, 16, 2, 1, 0, 0x1000);
        shm_cap(&c, 0x70, 0x88, 24, 6, 1, 0, 0x1000);
        shm_cap(&c, 0x88, 0x00, 24, 2, 1, 0x1_0000_0000, 0x2_0000_0000);
        assert_eq!(
            find_shared_memory(&c, &h, 1),
            Some(SharedMemory { id: 1, bar: 2, offset: 0x1_0000_0000, length: 0x2_0000_0000 })
        );
        assert_eq!(find_shared_memory(&c, &h, 2).map(|s| (s.offset, s.length)), Some((0x1000, 0x2000)));
        assert_eq!(find_shared_memory(&c, &h, 3), None);
        let shm = find_shared_memory(&c, &h, 2).unwrap();
        let mut bars = [None; 6];
        bars[2] = Some(Bar { kind: BarKind::Memory64, address: 0x8_0000_0000, size: 0x4000, prefetchable: true });
        assert_eq!(shm.address(&bars), Some(0x8_0000_1000));
        bars[2] = Some(Bar { kind: BarKind::Memory64, address: 0x8_0000_0000, size: 0x2000, prefetchable: true });
        assert_eq!(shm.address(&bars), None);
        // The ordinary structures do not see it.
        let (c, h) = qemu_like();
        assert_eq!(find_shared_memory(&c, &h, 1), None);
    }

    #[test]
    fn missing_structures() {
        let (c, h) = header(0x40);
        vcap(&c, 0x40, 0x50, cfg_type::COMMON, reg(0, 0, 0x1000), None);
        vcap(&c, 0x50, 0x00, cfg_type::ISR, reg(0, 0x1000, 4), None);
        assert_eq!(PciTransport::discover(&c, &h), Err(TransportError::MissingNotify));
        vcap(&c, 0x50, 0x00, cfg_type::NOTIFY, reg(0, 0x1000, 4), Some(2));
        assert_eq!(PciTransport::discover(&c, &h), Err(TransportError::MissingIsr));
        let (c, h) = header(0);
        assert_eq!(PciTransport::discover(&c, &h), Err(TransportError::MissingCommon));
    }

    #[test]
    fn looping_capabilities_terminate() {
        let (c, h) = header(0x40);
        vcap(&c, 0x40, 0x50, cfg_type::COMMON, reg(0, 0, 0x1000), None);
        vcap(&c, 0x50, 0x40, cfg_type::ISR, reg(0, 0x1000, 4), None);
        assert_eq!(PciTransport::discover(&c, &h), Err(TransportError::MissingNotify));
    }

    #[test]
    fn notifier_offsets() {
        let (c, h) = qemu_like();
        let t = PciTransport::discover(&c, &h).unwrap();
        let n = t.notifier(3, 3).unwrap();
        assert_eq!(n.offset(), 12);
        assert_eq!(t.notifier(0, 0x3ff).map(|n| n.offset()), Some(0xffc));
        assert_eq!(t.notifier(0, 0x400), None);
        assert_eq!(PciTransport { notify_off_multiplier: u32::MAX, ..t }.notifier(0, u16::MAX), None);
        assert_eq!(PciTransport { notify_off_multiplier: 0, ..t }.notifier(7, 9).map(|n| n.offset()), Some(0));
        let regs = Bytes { mem: vec![0; 16].into(), ..Bytes::default() };
        n.notify(&regs);
        assert_eq!(*regs.writes.borrow(), [(12, 3)]);
    }

    #[test]
    fn isr_reads_and_clears() {
        let isr = Bytes { mem: vec![3].into(), read_clears: true, ..Bytes::default() };
        assert_eq!(Isr::read(&isr), Isr { queue: true, config: true });
        assert_eq!(Isr::read(&isr), Isr::default());
    }

    #[test]
    fn pci_cfg_window() {
        let (c, h) = qemu_like();
        let w = PciTransport::discover(&c, &h).unwrap().pci_cfg.unwrap();
        c.put(0x50, &[0xaa, 0xbb, 0xcc, 0xdd]);
        assert_eq!(w.read(&c, B, 4, 0x3000, Width::U16), Ok(0xbbaa));
        let bytes = c.bytes.borrow();
        assert_eq!(bytes[0x44], 4);
        assert_eq!(&bytes[0x48..0x50], &[0x00, 0x30, 0, 0, 2, 0, 0, 0]);
        drop(bytes);
        w.write(&c, B, 2, 0x10, Width::U32, 0x1234_5678).unwrap();
        assert_eq!(&c.bytes.borrow()[0x48..0x54], &[0x10, 0, 0, 0, 4, 0, 0, 0, 0x78, 0x56, 0x34, 0x12]);
        assert_eq!(w.read(&c, B, 6, 0, Width::U8), Err(TransportError::BadWindowAccess));
        assert_eq!(w.read(&c, B, 0, 2, Width::U32), Err(TransportError::BadWindowAccess));
    }
}
