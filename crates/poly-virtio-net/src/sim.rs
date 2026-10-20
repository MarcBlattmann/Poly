//! A simulated virtio network device for the unit tests, written from the
//! device requirements of VIRTIO 1.2 (§2.7 split virtqueues, §4.1.4.3
//! common configuration, §5.1 network device), not from the driver code.
//!
//! One shared state sits behind three register blocks: the common
//! configuration, the device configuration and the notification region.
//! Frames the driver transmits are collected in `sent`; frames put in the
//! `inbox` go into receive buffers as the driver provides them. Bus
//! addresses are host addresses.

extern crate std;

use crate::{HEADER_LEN, NetHeader, feature};
use core::cell::{Cell, RefCell};
use core::sync::atomic::{Ordering, fence};
use poly_pci::Mmio;
use poly_virtio::status;
use std::collections::VecDeque;
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

/// The device side of one queue.
#[derive(Debug, Default)]
pub struct QueueState {
    last_avail: u16,
    used_idx: u16,
    /// Used buffer notifications the device would have sent.
    pub interrupts: u32,
    pub notifies: u32,
}

/// A descriptor chain: (address, length, device-writable) per descriptor.
type Chain = (u16, Vec<(u64, u32, bool)>);

#[derive(Debug)]
pub struct Device {
    pub rx: QueueState,
    pub tx: QueueState,
    /// Frames transmitted, without their headers.
    pub sent: Vec<Vec<u8>>,
    pub sent_headers: Vec<NetHeader>,
    /// Frames waiting for receive buffers.
    pub inbox: VecDeque<Vec<u8>>,
    /// Frames dropped because a buffer was too small (no MRG_RXBUF).
    pub rx_too_big: u32,
    /// Take transmitted frames when notified.
    pub tx_auto: bool,
    /// With MRG_RXBUF: publish `used.idx` after the first buffer of each
    /// packet only, the rest at the next `deliver`.
    pub split_publish: bool,
    /// Hand back this id in the next used element (a broken device).
    pub bogus_id: Option<u32>,
    /// Write this `num_buffers` into the next received packet.
    pub bogus_count: Option<u16>,
    /// Report this length for the next received buffer.
    pub bogus_len: Option<u32>,
    /// Used elements written but not yet published.
    unpublished: u16,
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
    pub config: RefCell<Vec<u8>>,
    pub status_writes: RefCell<Vec<u8>>,
    pub dev: RefCell<Device>,
}

/// Handles on the shared state, one per register block.
#[derive(Clone, Debug)]
pub struct Sim(pub Rc<Inner>);
#[derive(Debug)]
pub struct CommonRegs(pub Rc<Inner>);
#[derive(Debug)]
pub struct ConfigRegs(pub Rc<Inner>);
#[derive(Debug)]
pub struct NotifyRegs(pub Rc<Inner>);

pub const SIM_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
/// Length of the configuration structure the simulated device offers.
pub const CONFIG_LEN: u32 = 24;

impl Sim {
    /// A device offering `offered` with queues of at most `max_queue`
    /// entries, the MAC address [`SIM_MAC`], link up, MTU `mtu`.
    pub fn new(offered: u64, max_queue: u16, mtu: u16) -> Sim {
        let mut config = vec![0u8; CONFIG_LEN as usize];
        config[0..6].copy_from_slice(&SIM_MAC);
        config[6..8].copy_from_slice(&crate::link_status::LINK_UP.to_le_bytes());
        config[8..10].copy_from_slice(&1u16.to_le_bytes());
        config[10..12].copy_from_slice(&mtu.to_le_bytes());
        let q = QueueRegs { max_size: max_queue, size: max_queue, msix: 0xffff, ..Default::default() };
        Sim(Rc::new(Inner {
            offered,
            status: Cell::new(0),
            driver_features: Cell::new(0),
            dfselect: Cell::new(0),
            gfselect: Cell::new(0),
            queue_select: Cell::new(0),
            queues: RefCell::new(vec![q; 3]),
            config_msix: Cell::new(0xffff),
            generation: Cell::new(0),
            config: RefCell::new(config),
            status_writes: RefCell::new(Vec::new()),
            dev: RefCell::new(Device {
                rx: QueueState::default(),
                tx: QueueState::default(),
                sent: Vec::new(),
                sent_headers: Vec::new(),
                inbox: VecDeque::new(),
                rx_too_big: 0,
                tx_auto: true,
                split_publish: false,
                bogus_id: None,
                bogus_count: None,
                bogus_len: None,
                unpublished: 0,
            }),
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

    pub fn negotiated(&self, f: u64) -> bool {
        self.0.driver_features.get() & f != 0
    }

    pub fn queue(&self, i: usize) -> QueueRegs {
        self.0.queues.borrow()[i]
    }

    /// Change the link status (and the configuration generation, §2.5.1).
    pub fn set_link(&self, up: bool) {
        let v: u16 = if up { crate::link_status::LINK_UP } else { 0 };
        self.0.config.borrow_mut()[6..8].copy_from_slice(&v.to_le_bytes());
        self.0.generation.set(self.0.generation.get().wrapping_add(1));
    }

    fn live(&self, q: &QueueRegs) -> bool {
        q.enable != 0 && self.0.status.get() & status::DRIVER_OK != 0
    }

    /// The next available chain of queue `q`, if any.
    fn take_chain(q: &QueueRegs, st: &mut QueueState, event_idx: bool) -> Option<Chain> {
        if rd::<u16>(q.driver + 2) == st.last_avail {
            return None;
        }
        fence(Ordering::Acquire);
        let head = rd::<u16>(q.driver + 4 + 2 * (st.last_avail % q.size) as u64);
        st.last_avail = st.last_avail.wrapping_add(1);
        if event_idx {
            // Notify me about the next one too (§2.7.10).
            wr(q.device + 4 + 8 * q.size as u64, st.last_avail);
        }
        let mut descs = Vec::new();
        let mut i = head;
        loop {
            assert!(i < q.size, "descriptor index out of range");
            assert!(descs.len() < q.size as usize, "descriptor loop");
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

    /// Write a used element; published (`used.idx`) only if `publish`.
    #[allow(clippy::too_many_arguments)]
    fn put_used(
        q: &QueueRegs,
        st: &mut QueueState,
        bogus: &mut Option<u32>,
        head: u16,
        len: u32,
        publish: bool,
        event_idx: bool,
    ) {
        let id = bogus.take().unwrap_or(head as u32);
        let e = q.device + 4 + 8 * (st.used_idx % q.size) as u64;
        wr(e, id);
        wr(e + 4, len);
        st.used_idx = st.used_idx.wrapping_add(1);
        if publish {
            Self::publish(q, st, event_idx);
        }
    }

    /// Make the used elements written so far visible, and count the
    /// interrupt the device would send unless the driver suppressed it
    /// (§2.7.7: `used_event` with EVENT_IDX, else the NO_INTERRUPT flag).
    fn publish(q: &QueueRegs, st: &mut QueueState, event_idx: bool) {
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

    /// Take every frame the driver queued for transmission.
    pub fn take_tx(&self) {
        let q = self.queue(1);
        if !self.live(&q) {
            return;
        }
        let event_idx = self.negotiated(poly_virtio::feature::EVENT_IDX);
        let mut dev = self.0.dev.borrow_mut();
        let dev = &mut *dev;
        while let Some((head, descs)) = Self::take_chain(&q, &mut dev.tx, event_idx) {
            let mut bytes = Vec::new();
            for &(addr, len, writable) in &descs {
                assert!(!writable, "a device-writable descriptor in a transmit chain");
                // SAFETY: the driver lent us this buffer.
                bytes.extend_from_slice(unsafe { core::slice::from_raw_parts(addr as *const u8, len as usize) });
            }
            assert!(bytes.len() >= HEADER_LEN, "transmit buffer without a header");
            let h: [u8; HEADER_LEN] = bytes[..HEADER_LEN].try_into().unwrap();
            dev.sent_headers.push(NetHeader::from_bytes(&h));
            dev.sent.push(bytes[HEADER_LEN..].to_vec());
            Self::put_used(&q, &mut dev.tx, &mut dev.bogus_id, head, 0, true, event_idx);
        }
    }

    /// Put a frame in the inbox and deliver what fits.
    pub fn inject(&self, frame: &[u8]) {
        self.0.dev.borrow_mut().inbox.push_back(frame.to_vec());
        self.deliver();
    }

    /// Move inbox frames into receive buffers while there are buffers
    /// (§5.1.6.4), and publish anything held back.
    pub fn deliver(&self) {
        let q = self.queue(0);
        if !self.live(&q) {
            return;
        }
        let event_idx = self.negotiated(poly_virtio::feature::EVENT_IDX);
        let mrg = self.negotiated(feature::MRG_RXBUF);
        let mut dev = self.0.dev.borrow_mut();
        let dev = &mut *dev;
        if dev.unpublished > 0 {
            dev.unpublished = 0;
            Self::publish(&q, &mut dev.rx, event_idx);
        }
        while let Some(frame) = dev.inbox.front().cloned() {
            let mut packet = NetHeader::default().to_bytes().to_vec();
            packet.extend_from_slice(&frame);
            // Gather buffers until the packet fits (one without MRG_RXBUF).
            let save = dev.rx.last_avail;
            let mut chains = Vec::new();
            let mut room = 0usize;
            while room < packet.len() && (mrg || chains.is_empty()) {
                let Some(c) = Self::take_chain(&q, &mut dev.rx, event_idx) else { break };
                room += c.1.iter().filter(|d| d.2).map(|d| d.1 as usize).sum::<usize>();
                chains.push(c);
            }
            if room < packet.len() {
                if mrg || chains.is_empty() {
                    // Wait for more buffers.
                    dev.rx.last_avail = save;
                    break;
                }
                // Too big for one buffer: dropped, the buffer stays ours.
                dev.rx.last_avail = save;
                dev.inbox.pop_front();
                dev.rx_too_big += 1;
                break;
            }
            dev.inbox.pop_front();
            let count = dev.bogus_count.take().unwrap_or(if mrg { chains.len() as u16 } else { 1 });
            packet[10..12].copy_from_slice(&count.to_le_bytes());
            let mut at = 0;
            let n = chains.len();
            for (i, (head, descs)) in chains.into_iter().enumerate() {
                let start = at;
                for (addr, len, writable) in descs {
                    assert!(writable, "a device-readable descriptor in a receive chain");
                    let k = (len as usize).min(packet.len() - at);
                    // SAFETY: the driver lent us this buffer, device-writable.
                    unsafe { core::ptr::copy_nonoverlapping(packet[at..].as_ptr(), addr as *mut u8, k) };
                    at += k;
                }
                let len = dev.bogus_len.take().unwrap_or((at - start) as u32);
                let publish = !dev.split_publish || !mrg || i == 0 || n == 1;
                if !publish {
                    dev.unpublished += 1;
                }
                Self::put_used(&q, &mut dev.rx, &mut dev.bogus_id, head, len, publish, event_idx);
            }
            if dev.unpublished > 0 {
                // Hold the rest back until the next delivery.
                break;
            }
        }
    }

    pub fn reset_device(&self) {
        for q in self.0.queues.borrow_mut().iter_mut() {
            *q = QueueRegs { max_size: q.max_size, size: q.max_size, msix: 0xffff, ..Default::default() };
        }
        self.0.driver_features.set(0);
        let mut dev = self.0.dev.borrow_mut();
        dev.rx = QueueState::default();
        dev.tx = QueueState::default();
        dev.unpublished = 0;
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
        self.0.status_writes.borrow_mut().push(value);
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
            // Any vector below 3 is accepted (the function has 3 entries).
            0x10 => self.0.config_msix.set(if value < 3 { value } else { 0xffff }),
            0x16 => self.0.queue_select.set(value),
            0x18 => qs[sel].size = value,
            0x1a => qs[sel].msix = if value < 3 { value } else { 0xffff },
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

impl Mmio for ConfigRegs {
    fn read8(&self, offset: usize) -> u8 {
        self.0.config.borrow()[offset]
    }

    fn read16(&self, offset: usize) -> u16 {
        u16::from_le_bytes(self.0.config.borrow()[offset..offset + 2].try_into().unwrap())
    }

    fn read32(&self, offset: usize) -> u32 {
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

    /// Queue `q`'s register is at `4 * q` (notify_off = q, multiplier 4).
    fn write16(&self, offset: usize, value: u16) {
        assert_eq!(offset, 4 * value as usize, "notification register and queue index disagree");
        let sim = Sim(self.0.clone());
        match value {
            0 => {
                sim.dev().rx.notifies += 1;
                sim.deliver();
            }
            1 => {
                sim.dev().tx.notifies += 1;
                if sim.dev().tx_auto {
                    sim.take_tx();
                }
            }
            q => panic!("notification for queue {q}"),
        }
    }

    fn write32(&self, _: usize, _: u32) {
        unimplemented!()
    }
}
