//! A simulated virtio block device for the unit tests, written from the
//! device requirements of VIRTIO 1.2 (§2.7 split virtqueues, §4.1.4.3 common
//! configuration, §5.2 block device), not from the driver code.
//!
//! One shared state sits behind three register blocks: the common
//! configuration, the device configuration and the notification region.
//! The device serves its queue when notified (or when a test says so). Bus
//! addresses are host addresses.

extern crate std;

use crate::{ID_LEN, feature, req_status, req_type};
use core::cell::{Cell, RefCell};
use core::sync::atomic::{Ordering, fence};
use poly_pci::Mmio;
use poly_virtio::status;
use std::rc::Rc;
use std::{alloc, vec, vec::Vec};

/// A zeroed, page-aligned allocation standing in for DMA memory.
pub struct DmaMemory {
    pub ptr: core::ptr::NonNull<u8>,
    pub len: usize,
    layout: alloc::Layout,
}

impl DmaMemory {
    pub fn new(len: usize) -> DmaMemory {
        let layout = alloc::Layout::from_size_align(len.max(1), 4096).unwrap();
        // SAFETY: the layout has a non-zero size.
        let ptr = core::ptr::NonNull::new(unsafe { alloc::alloc_zeroed(layout) }).unwrap();
        DmaMemory { ptr, len, layout }
    }

    pub fn region(&self) -> crate::DmaRegion {
        crate::DmaRegion { ptr: self.ptr, bus: self.ptr.as_ptr() as u64, len: self.len }
    }
}

impl Drop for DmaMemory {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with this layout.
        unsafe { alloc::dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

fn rd<T: Copy>(addr: u64) -> T {
    // SAFETY: the driver handed these addresses to the device; tests keep
    // the memory alive while the device works.
    unsafe { core::ptr::read_volatile(addr as *const T) }
}

fn wr<T: Copy>(addr: u64, v: T) {
    // SAFETY: as for `rd`.
    unsafe { core::ptr::write_volatile(addr as *mut T, v) }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct QueueRegs {
    pub max_size: u16,
    pub size: u16,
    pub msix: u16,
    pub enable: u16,
    pub desc: u64,
    pub driver: u64,
    pub device: u64,
}

/// One request as the device saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seen {
    pub ty: u32,
    pub sector: u64,
    /// Data bytes (excluding header and status).
    pub len: u32,
    /// Descriptors in the chain.
    pub descs: usize,
    pub status: u8,
}

pub struct Device {
    pub disk: Vec<u8>,
    pub id: [u8; ID_LEN],
    /// Sectors whose reads and writes fail with IOERR.
    pub bad_sectors: Vec<u64>,
    /// Answer every request with this status instead.
    pub status_override: Option<u8>,
    /// Serve the queue when notified.
    pub auto: bool,
    /// Complete what a round of serving took in reverse order.
    pub reverse: bool,
    /// Take requests but complete nothing until `release_held`.
    pub hold: bool,
    held: Vec<(u16, u32)>,
    /// Hand back this id in the next used element (a broken device).
    pub bogus_id: Option<u32>,
    pub log: Vec<Seen>,
    last_avail: u16,
    used_idx: u16,
}

pub struct Inner {
    pub offered: u64,
    pub status: Cell<u8>,
    pub driver_features: Cell<u64>,
    dfselect: Cell<u32>,
    gfselect: Cell<u32>,
    queue_select: Cell<u16>,
    pub queue: RefCell<QueueRegs>,
    pub generation: Cell<u8>,
    /// Bump the generation on every configuration read (a device whose
    /// configuration never settles).
    pub churn: Cell<bool>,
    /// The device never finishes a reset.
    pub stuck_reset: Cell<bool>,
    pub config: RefCell<Vec<u8>>,
    pub notifies: Cell<u32>,
    pub status_writes: RefCell<Vec<u8>>,
    pub dev: RefCell<Device>,
}

/// Handles on the shared state, one per register block.
#[derive(Clone)]
pub struct Sim(pub Rc<Inner>);
pub struct CommonRegs(pub Rc<Inner>);
pub struct ConfigRegs(pub Rc<Inner>);
pub struct NotifyRegs(pub Rc<Inner>);

impl Sim {
    /// A device of `sectors` sectors offering `offered` (VERSION_1 added
    /// unless `legacy`), with a queue of at most `max_queue` entries and the
    /// given configuration fields.
    pub fn new(sectors: u64, offered: u64, max_queue: u16) -> Sim {
        let mut config = vec![0u8; 60];
        config[0..8].copy_from_slice(&sectors.to_le_bytes());
        let dev = Device {
            disk: vec![0; (sectors * 512) as usize],
            id: *b"poly-sim-disk-0001\0\0",
            bad_sectors: Vec::new(),
            status_override: None,
            auto: true,
            reverse: false,
            hold: false,
            held: Vec::new(),
            bogus_id: None,
            log: Vec::new(),
            last_avail: 0,
            used_idx: 0,
        };
        Sim(Rc::new(Inner {
            offered,
            status: Cell::new(0),
            driver_features: Cell::new(0),
            dfselect: Cell::new(0),
            gfselect: Cell::new(0),
            queue_select: Cell::new(0),
            queue: RefCell::new(QueueRegs { max_size: max_queue, size: max_queue, msix: 0xffff, ..Default::default() }),
            generation: Cell::new(0),
            churn: Cell::new(false),
            stuck_reset: Cell::new(false),
            config: RefCell::new(config),
            notifies: Cell::new(0),
            status_writes: RefCell::new(Vec::new()),
            dev: RefCell::new(dev),
        }))
    }

    pub fn set_config32(&self, offset: usize, v: u32) {
        self.0.config.borrow_mut()[offset..offset + 4].copy_from_slice(&v.to_le_bytes());
    }

    pub fn common(&self) -> CommonRegs {
        CommonRegs(self.0.clone())
    }

    pub fn config(&self) -> ConfigRegs {
        ConfigRegs(self.0.clone())
    }

    pub fn notify(&self) -> NotifyRegs {
        NotifyRegs(self.0.clone())
    }

    pub fn dev(&self) -> std::cell::RefMut<'_, Device> {
        self.0.dev.borrow_mut()
    }

    pub fn negotiated(&self, f: u64) -> bool {
        self.0.driver_features.get() & f != 0
    }

    /// Serve every request in the available ring.
    pub fn serve(&self) {
        let q = *self.0.queue.borrow();
        if q.enable == 0 || self.0.status.get() & status::DRIVER_OK == 0 {
            return;
        }
        let event_idx = self.negotiated(poly_virtio::feature::EVENT_IDX);
        let mut dev = self.0.dev.borrow_mut();
        let mut done = Vec::new();
        while rd::<u16>(q.driver + 2) != dev.last_avail {
            fence(Ordering::Acquire);
            let head = rd::<u16>(q.driver + 4 + 2 * (dev.last_avail % q.size) as u64);
            dev.last_avail = dev.last_avail.wrapping_add(1);
            if event_idx {
                // Notify me about the next one too.
                wr(q.device + 4 + 8 * q.size as u64, dev.last_avail);
            }
            let len = self.execute(&mut dev, &q, head);
            done.push((head, len));
        }
        if dev.reverse {
            done.reverse();
        }
        if dev.hold {
            dev.held.extend(done);
        } else {
            for (head, len) in done {
                complete(&mut dev, &q, head, len);
            }
        }
    }

    /// Complete everything held back.
    pub fn release_held(&self) {
        let q = *self.0.queue.borrow();
        let mut dev = self.0.dev.borrow_mut();
        let held = core::mem::take(&mut dev.held);
        for (head, len) in held {
            complete(&mut dev, &q, head, len);
        }
    }

    /// Run one request; returns the bytes written into its buffer.
    fn execute(&self, dev: &mut Device, q: &QueueRegs, head: u16) -> u32 {
        // Walk the chain (§2.7.5): readable descriptors, then writable ones.
        let (mut readable, mut writable) = (Vec::new(), Vec::new());
        let mut i = head;
        loop {
            assert!(i < q.size, "descriptor index out of range");
            assert!(readable.len() + writable.len() < q.size as usize, "descriptor loop");
            let d = q.desc + 16 * i as u64;
            let (addr, len, flags, next) = (rd::<u64>(d), rd::<u32>(d + 8), rd::<u16>(d + 12), rd::<u16>(d + 14));
            if flags & 2 != 0 {
                writable.push((addr, len));
            } else {
                assert!(writable.is_empty(), "readable descriptor after a writable one");
                readable.push((addr, len));
            }
            if flags & 1 == 0 {
                break;
            }
            i = next;
        }
        let descs = readable.len() + writable.len();
        let gather = |parts: &[(u64, u32)]| -> Vec<u8> {
            let mut v = Vec::new();
            for &(a, l) in parts {
                // SAFETY: the driver lent us this buffer.
                v.extend_from_slice(unsafe { core::slice::from_raw_parts(a as *const u8, l as usize) });
            }
            v
        };
        let out = gather(&readable);
        assert!(out.len() >= 16, "header missing");
        let ty = u32::from_le_bytes(out[0..4].try_into().unwrap());
        let sector = u64::from_le_bytes(out[8..16].try_into().unwrap());
        let data_out = &out[16..];
        // The status byte is the last writable byte (§5.2.6).
        let &(last_addr, last_len) = writable.last().expect("no status byte");
        let status_addr = last_addr + last_len as u64 - 1;
        let mut data_in = writable.clone();
        data_in.last_mut().unwrap().1 -= 1;
        let in_len: u64 = data_in.iter().map(|&(_, l)| l as u64).sum();

        let in_range = |len: u64| {
            len.is_multiple_of(512)
                && sector.checked_mul(512).and_then(|s| s.checked_add(len)).is_some_and(|e| e <= dev.disk.len() as u64)
        };
        let touches_bad = |len: u64| dev.bad_sectors.iter().any(|&b| b >= sector && b < sector + len / 512);
        let mut written = 0u32;
        let mut st = match ty {
            req_type::IN if !in_range(in_len) || touches_bad(in_len) => req_status::IOERR,
            req_type::IN => {
                let mut at = sector as usize * 512;
                for &(a, l) in &data_in {
                    // SAFETY: the driver made this piece device-writable.
                    unsafe { core::ptr::copy_nonoverlapping(dev.disk[at..].as_ptr(), a as *mut u8, l as usize) };
                    at += l as usize;
                }
                written = in_len as u32;
                req_status::OK
            }
            req_type::OUT if self.negotiated(feature::RO) => req_status::IOERR,
            req_type::OUT if !in_range(data_out.len() as u64) || touches_bad(data_out.len() as u64) => {
                req_status::IOERR
            }
            req_type::OUT => {
                let at = sector as usize * 512;
                dev.disk[at..at + data_out.len()].copy_from_slice(data_out);
                req_status::OK
            }
            req_type::FLUSH if self.negotiated(feature::FLUSH) => req_status::OK,
            req_type::GET_ID => {
                let n = (in_len as usize).min(ID_LEN);
                // SAFETY: as for IN.
                unsafe { core::ptr::copy_nonoverlapping(dev.id.as_ptr(), data_in[0].0 as *mut u8, n) };
                written = n as u32;
                req_status::OK
            }
            _ => req_status::UNSUPP,
        };
        if let Some(s) = dev.status_override {
            st = s;
        }
        wr(status_addr, st);
        dev.log.push(Seen { ty, sector, len: (data_out.len() as u64 + in_len) as u32, descs, status: st });
        written + 1
    }

    pub fn reset_device(&self) {
        *self.0.queue.borrow_mut() = {
            let q = *self.0.queue.borrow();
            QueueRegs { max_size: q.max_size, size: q.max_size, msix: 0xffff, ..Default::default() }
        };
        self.0.driver_features.set(0);
        let mut dev = self.0.dev.borrow_mut();
        dev.held.clear();
        dev.last_avail = 0;
        dev.used_idx = 0;
    }
}

fn complete(dev: &mut Device, q: &QueueRegs, head: u16, len: u32) {
    let id = dev.bogus_id.take().unwrap_or(head as u32);
    let e = q.device + 4 + 8 * (dev.used_idx % q.size) as u64;
    wr(e, id);
    wr(e + 4, len);
    fence(Ordering::Release);
    dev.used_idx = dev.used_idx.wrapping_add(1);
    wr(q.device + 2, dev.used_idx);
}

impl Mmio for CommonRegs {
    fn read8(&self, offset: usize) -> u8 {
        match offset {
            0x14 => self.0.status.get(),
            0x15 => self.0.generation.get(),
            _ => panic!("8-bit read at {offset:#x}"),
        }
    }

    fn read16(&self, offset: usize) -> u16 {
        let q = *self.0.queue.borrow();
        let selected = self.0.queue_select.get() == 0;
        match offset {
            0x10 => 0xffff,
            0x12 => 1,
            0x16 => self.0.queue_select.get(),
            // Only queue 0 exists.
            0x18 => {
                if selected {
                    q.size
                } else {
                    0
                }
            }
            0x1a => {
                if selected {
                    q.msix
                } else {
                    0xffff
                }
            }
            0x1c => {
                if selected {
                    q.enable
                } else {
                    0
                }
            }
            0x1e => 0,
            _ => panic!("16-bit read at {offset:#x}"),
        }
    }

    fn read32(&self, offset: usize) -> u32 {
        match offset {
            0x00 => self.0.dfselect.get(),
            0x04 => match self.0.dfselect.get() {
                0 => self.0.offered as u32,
                1 => (self.0.offered >> 32) as u32,
                _ => 0,
            },
            0x08 => self.0.gfselect.get(),
            _ => panic!("32-bit read at {offset:#x}"),
        }
    }

    fn write8(&self, offset: usize, value: u8) {
        assert_eq!(offset, 0x14, "8-bit write at {offset:#x}");
        self.0.status_writes.borrow_mut().push(value);
        if value == 0 {
            if self.0.stuck_reset.get() {
                self.0.status.set(status::DEVICE_NEEDS_RESET);
                return;
            }
            self.0.status.set(0);
            Sim(self.0.clone()).reset_device();
            return;
        }
        let mut value = value;
        if value & status::FEATURES_OK != 0 && self.0.driver_features.get() & !self.0.offered != 0 {
            value &= !status::FEATURES_OK;
        }
        self.0.status.set(value);
    }

    fn write16(&self, offset: usize, value: u16) {
        let selected = self.0.queue_select.get() == 0;
        let mut q = self.0.queue.borrow_mut();
        match offset {
            0x10 => {}
            0x16 => self.0.queue_select.set(value),
            0x18 if selected => q.size = value,
            0x1a if selected => q.msix = value,
            0x1c if selected => q.enable = value,
            _ => panic!("16-bit write at {offset:#x}"),
        }
    }

    fn write32(&self, offset: usize, value: u32) {
        let lo = |old: u64| (old & !0xffff_ffff) | value as u64;
        let hi = |old: u64| (old & 0xffff_ffff) | (value as u64) << 32;
        let mut q = self.0.queue.borrow_mut();
        match offset {
            0x00 => self.0.dfselect.set(value),
            0x08 => self.0.gfselect.set(value),
            0x0c => {
                let old = self.0.driver_features.get();
                self.0.driver_features.set(if self.0.gfselect.get() == 0 { lo(old) } else { hi(old) });
            }
            0x20 => q.desc = lo(q.desc),
            0x24 => q.desc = hi(q.desc),
            0x28 => q.driver = lo(q.driver),
            0x2c => q.driver = hi(q.driver),
            0x30 => q.device = lo(q.device),
            0x34 => q.device = hi(q.device),
            _ => panic!("32-bit write at {offset:#x}"),
        }
    }
}

impl Mmio for ConfigRegs {
    fn read8(&self, offset: usize) -> u8 {
        self.0.config.borrow()[offset]
    }

    fn read16(&self, offset: usize) -> u16 {
        u16::from_le_bytes(self.0.config.borrow()[offset..offset + 2].try_into().unwrap())
    }

    fn read32(&self, offset: usize) -> u32 {
        if self.0.churn.get() {
            self.0.generation.set(self.0.generation.get().wrapping_add(1));
        }
        u32::from_le_bytes(self.0.config.borrow()[offset..offset + 4].try_into().unwrap())
    }

    fn write8(&self, offset: usize, _: u8) {
        panic!("configuration write at {offset:#x}");
    }

    fn write16(&self, offset: usize, _: u16) {
        panic!("configuration write at {offset:#x}");
    }

    fn write32(&self, offset: usize, _: u32) {
        panic!("configuration write at {offset:#x}");
    }
}

impl Mmio for NotifyRegs {
    fn read8(&self, _: usize) -> u8 {
        unimplemented!()
    }

    fn read16(&self, _: usize) -> u16 {
        unimplemented!()
    }

    fn read32(&self, _: usize) -> u32 {
        unimplemented!()
    }

    fn write8(&self, _: usize, _: u8) {
        unimplemented!()
    }

    fn write16(&self, offset: usize, value: u16) {
        assert_eq!((offset, value), (0, 0), "notification for another queue");
        self.0.notifies.set(self.0.notifies.get() + 1);
        if self.0.dev.borrow().auto {
            Sim(self.0.clone()).serve();
        }
    }

    fn write32(&self, _: usize, _: u32) {
        unimplemented!()
    }
}
