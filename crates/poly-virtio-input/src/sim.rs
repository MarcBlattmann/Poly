//! A simulated virtio input device for the unit tests, written from the
//! device requirements of VIRTIO 1.2 (§2.7 split virtqueues, §4.1.4.3
//! common configuration, §5.8 input device), not from the driver code.
//!
//! One shared state sits behind three register blocks: the common
//! configuration, the device configuration (`select`/`subsel` answered
//! from a table) and the notification region. Events put in the `inbox`
//! go into event buffers as the driver provides them; events the driver
//! sends on the status queue are collected in `status`. Bus addresses are
//! host addresses.

use crate::{EVENT_LEN, RawEvent, cfg, event_bytes, parse_event};
use core::cell::{Cell, RefCell};
use core::sync::atomic::{Ordering, fence};
use poly_pci::Mmio;
use poly_virtio::status;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;
use std::{alloc, vec, vec::Vec};

/// A zeroed, page-aligned allocation standing in for DMA memory.
#[derive(Debug)]
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

/// §2.7.7 / §2.7.10: has the index crossed `event` going from `old` to `new`?
fn need_event(event: u16, new: u16, old: u16) -> bool {
    new.wrapping_sub(event).wrapping_sub(1) < new.wrapping_sub(old)
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

#[derive(Debug, Default)]
pub struct QueueState {
    last_avail: u16,
    used_idx: u16,
    /// Used buffer notifications the device would have sent.
    pub interrupts: u32,
    pub notifies: u32,
}

/// A descriptor chain: its head and (address, length, device-writable)
/// per descriptor.
type Chain = (u16, Vec<(u64, u32, bool)>);

#[derive(Debug, Default)]
pub struct Device {
    pub events: QueueState,
    pub status_q: QueueState,
    /// Events waiting for buffers.
    pub inbox: VecDeque<RawEvent>,
    /// Events the driver sent.
    pub status: Vec<RawEvent>,
    /// Hand back this id in the next used element (a broken device).
    pub bogus_id: Option<u32>,
    /// Report this length for the next event buffer.
    pub short_len: Option<u32>,
}

#[derive(Debug)]
pub struct Inner {
    pub offered: u64,
    pub status: Cell<u8>,
    pub driver_features: Cell<u64>,
    dfselect: Cell<u32>,
    gfselect: Cell<u32>,
    queue_select: Cell<u16>,
    pub queues: RefCell<Vec<QueueRegs>>,
    pub config_msix: Cell<u16>,
    pub generation: Cell<u8>,
    /// What each (select, subsel) answers.
    pub table: RefCell<BTreeMap<(u8, u8), Vec<u8>>>,
    select: Cell<u8>,
    subsel: Cell<u8>,
    /// Bump the generation on the next this-many configuration reads (a
    /// configuration that keeps changing).
    pub unstable: Cell<u32>,
    pub dev: RefCell<Device>,
}

#[derive(Clone, Debug)]
pub struct Sim(pub Rc<Inner>);
#[derive(Debug)]
pub struct CommonRegs(pub Rc<Inner>);
#[derive(Debug)]
pub struct ConfigRegs(pub Rc<Inner>);
#[derive(Debug)]
pub struct NotifyRegs(pub Rc<Inner>);

/// Length of the configuration structure: header and union.
pub const CONFIG_LEN: u32 = (cfg::U + cfg::U_LEN) as u32;

impl Sim {
    /// A device offering `offered`, with queues of at most `max_queue`
    /// entries, answering the configuration from `table`.
    pub fn new(offered: u64, max_queue: u16, table: &[((u8, u8), Vec<u8>)]) -> Sim {
        let q = QueueRegs { max_size: max_queue, size: max_queue, msix: 0xffff, ..Default::default() };
        Sim(Rc::new(Inner {
            offered,
            status: Cell::new(0),
            driver_features: Cell::new(0),
            dfselect: Cell::new(0),
            gfselect: Cell::new(0),
            queue_select: Cell::new(0),
            queues: RefCell::new(vec![q; 2]),
            config_msix: Cell::new(0xffff),
            generation: Cell::new(0),
            table: RefCell::new(table.iter().cloned().collect()),
            select: Cell::new(0),
            subsel: Cell::new(0),
            unstable: Cell::new(0),
            dev: RefCell::new(Device::default()),
        }))
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

    pub fn queue(&self, i: usize) -> QueueRegs {
        self.0.queues.borrow()[i]
    }

    fn negotiated(&self, f: u64) -> bool {
        self.0.driver_features.get() & f != 0
    }

    fn live(&self, q: &QueueRegs) -> bool {
        q.enable != 0 && self.0.status.get() & status::DRIVER_OK != 0
    }

    /// The next available chain: (head, [(address, length, writable)]).
    fn take_chain(q: &QueueRegs, st: &mut QueueState, event_idx: bool) -> Option<Chain> {
        if rd::<u16>(q.driver + 2) == st.last_avail {
            return None;
        }
        fence(Ordering::Acquire);
        let head = rd::<u16>(q.driver + 4 + 2 * (st.last_avail % q.size) as u64);
        st.last_avail = st.last_avail.wrapping_add(1);
        if event_idx {
            wr(q.device + 4 + 8 * q.size as u64, st.last_avail);
        }
        let mut descs = Vec::new();
        let mut i = head;
        loop {
            assert!(i < q.size && descs.len() < q.size as usize, "bad descriptor chain");
            let d = q.desc + 16 * i as u64;
            let (addr, len, flags, next) = (rd::<u64>(d), rd::<u32>(d + 8), rd::<u16>(d + 12), rd::<u16>(d + 14));
            descs.push((addr, len, flags & 2 != 0));
            if flags & 1 == 0 {
                break;
            }
            i = next;
        }
        Some((head, descs))
    }

    fn put_used(q: &QueueRegs, st: &mut QueueState, id: u32, len: u32, event_idx: bool) {
        let e = q.device + 4 + 8 * (st.used_idx % q.size) as u64;
        wr(e, id);
        wr(e + 4, len);
        st.used_idx = st.used_idx.wrapping_add(1);
        fence(Ordering::Release);
        let old = rd::<u16>(q.device + 2);
        wr(q.device + 2, st.used_idx);
        fence(Ordering::SeqCst);
        let suppressed = if event_idx {
            !need_event(rd::<u16>(q.driver + 4 + 2 * q.size as u64), st.used_idx, old)
        } else {
            rd::<u16>(q.driver) & 1 != 0
        };
        if !suppressed {
            st.interrupts += 1;
        }
    }

    /// The device has these events; deliver what fits.
    pub fn inject(&self, events: &[RawEvent]) {
        self.dev().inbox.extend(events.iter().copied());
        self.deliver();
    }

    /// Move inbox events into event buffers while there are buffers.
    pub fn deliver(&self) {
        let q = self.queue(0);
        if !self.live(&q) {
            return;
        }
        let event_idx = self.negotiated(poly_virtio::feature::EVENT_IDX);
        let mut dev = self.0.dev.borrow_mut();
        let dev = &mut *dev;
        while let Some(&e) = dev.inbox.front() {
            let Some((head, descs)) = Self::take_chain(&q, &mut dev.events, event_idx) else { break };
            dev.inbox.pop_front();
            let (addr, len, writable) = descs[0];
            assert!(writable && len as usize >= EVENT_LEN, "an event buffer must be 8 writable bytes");
            wr(addr, event_bytes(e));
            let id = dev.bogus_id.take().unwrap_or(head as u32);
            let len = dev.short_len.take().unwrap_or(EVENT_LEN as u32);
            Self::put_used(&q, &mut dev.events, id, len, event_idx);
        }
    }

    /// Take what the driver put on the status queue.
    pub fn take_status(&self) {
        let q = self.queue(1);
        if !self.live(&q) {
            return;
        }
        let event_idx = self.negotiated(poly_virtio::feature::EVENT_IDX);
        let mut dev = self.0.dev.borrow_mut();
        let dev = &mut *dev;
        while let Some((head, descs)) = Self::take_chain(&q, &mut dev.status_q, event_idx) {
            for (addr, len, writable) in descs {
                assert!(!writable && len as usize == EVENT_LEN, "a status buffer is one device-readable event");
                dev.status.push(parse_event(&rd::<[u8; EVENT_LEN]>(addr)));
            }
            Self::put_used(&q, &mut dev.status_q, head as u32, 0, event_idx);
        }
    }

    fn reset_device(&self) {
        for q in self.0.queues.borrow_mut().iter_mut() {
            *q = QueueRegs { max_size: q.max_size, size: q.max_size, msix: 0xffff, ..Default::default() };
        }
        self.0.driver_features.set(0);
        let mut dev = self.0.dev.borrow_mut();
        dev.events = QueueState::default();
        dev.status_q = QueueState::default();
    }
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
        let sel = self.0.queue_select.get() as usize;
        let qs = self.0.queues.borrow();
        let q = qs.get(sel).copied();
        match offset {
            0x10 => self.0.config_msix.get(),
            0x12 => qs.len() as u16,
            0x16 => sel as u16,
            0x18 => q.map_or(0, |q| q.size),
            0x1a => q.map_or(0xffff, |q| q.msix),
            0x1c => q.map_or(0, |q| q.enable),
            0x1e => sel as u16,
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
        if value == 0 {
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
        let sel = self.0.queue_select.get() as usize;
        let mut qs = self.0.queues.borrow_mut();
        match offset {
            0x10 => self.0.config_msix.set(if value < 2 { value } else { 0xffff }),
            0x16 => self.0.queue_select.set(value),
            0x18 => qs[sel].size = value,
            0x1a => qs[sel].msix = if value < 2 { value } else { 0xffff },
            0x1c => qs[sel].enable = value,
            _ => panic!("16-bit write at {offset:#x}"),
        }
    }

    fn write32(&self, offset: usize, value: u32) {
        let lo = |old: u64| (old & !0xffff_ffff) | value as u64;
        let hi = |old: u64| (old & 0xffff_ffff) | (value as u64) << 32;
        let sel = self.0.queue_select.get() as usize;
        let mut qs = self.0.queues.borrow_mut();
        match offset {
            0x00 => self.0.dfselect.set(value),
            0x08 => self.0.gfselect.set(value),
            0x0c => {
                let old = self.0.driver_features.get();
                self.0.driver_features.set(if self.0.gfselect.get() == 0 { lo(old) } else { hi(old) });
            }
            0x20 => qs[sel].desc = lo(qs[sel].desc),
            0x24 => qs[sel].desc = hi(qs[sel].desc),
            0x28 => qs[sel].driver = lo(qs[sel].driver),
            0x2c => qs[sel].driver = hi(qs[sel].driver),
            0x30 => qs[sel].device = lo(qs[sel].device),
            0x34 => qs[sel].device = hi(qs[sel].device),
            _ => panic!("32-bit write at {offset:#x}"),
        }
    }
}

impl ConfigRegs {
    fn answer(&self) -> Vec<u8> {
        let key = (self.0.select.get(), self.0.subsel.get());
        self.0.table.borrow().get(&key).cloned().unwrap_or_default()
    }
}

impl Mmio for ConfigRegs {
    fn read8(&self, offset: usize) -> u8 {
        if self.0.unstable.get() > 0 {
            self.0.unstable.set(self.0.unstable.get() - 1);
            self.0.generation.set(self.0.generation.get().wrapping_add(1));
        }
        match offset {
            cfg::SELECT => self.0.select.get(),
            cfg::SUBSEL => self.0.subsel.get(),
            cfg::SIZE => self.answer().len().min(cfg::U_LEN) as u8,
            o if (cfg::U..cfg::U + cfg::U_LEN).contains(&o) => self.answer().get(o - cfg::U).copied().unwrap_or(0),
            _ => 0,
        }
    }

    fn read16(&self, offset: usize) -> u16 {
        panic!("16-bit configuration read at {offset:#x}");
    }

    fn read32(&self, offset: usize) -> u32 {
        panic!("32-bit configuration read at {offset:#x}");
    }

    fn write8(&self, offset: usize, value: u8) {
        match offset {
            cfg::SELECT => self.0.select.set(value),
            cfg::SUBSEL => self.0.subsel.set(value),
            _ => panic!("configuration write at {offset:#x}"),
        }
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

    /// Queue `q`'s register is at `4 * q` (notify_off = q, multiplier 4).
    fn write16(&self, offset: usize, value: u16) {
        assert_eq!(offset, 4 * value as usize, "notification register and queue index disagree");
        let sim = Sim(self.0.clone());
        match value {
            0 => {
                sim.dev().events.notifies += 1;
                sim.deliver();
            }
            1 => {
                sim.dev().status_q.notifies += 1;
                sim.take_status();
            }
            q => panic!("notification for queue {q}"),
        }
    }

    fn write32(&self, _: usize, _: u32) {
        unimplemented!()
    }
}
