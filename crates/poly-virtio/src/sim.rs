//! Simulated hardware for the unit tests: "DMA" memory, the device side of
//! a split virtqueue, a device's common configuration registers, and one
//! function's configuration space.
//!
//! Bus addresses are host virtual addresses, so the simulated device reads
//! and writes the driver's buffers directly.

extern crate std;

use crate::{QueueAddresses, status};
use core::cell::{Cell, RefCell};
use core::ptr::NonNull;
use core::sync::atomic::{Ordering, fence};
use poly_pci::{Bdf, ConfigSpace, Mmio};
use std::{alloc, vec, vec::Vec};

/// A zeroed, page-aligned allocation standing in for DMA frames.
pub struct DmaMemory {
    ptr: NonNull<u8>,
    layout: alloc::Layout,
}

impl DmaMemory {
    pub fn new(len: usize) -> DmaMemory {
        // Slack at the end, so tests can offset into it.
        let layout = alloc::Layout::from_size_align(len + 64, 4096).unwrap();
        // SAFETY: the layout has a non-zero size.
        let ptr = NonNull::new(unsafe { alloc::alloc_zeroed(layout) }).unwrap();
        DmaMemory { ptr, layout }
    }

    pub fn ptr(&self) -> NonNull<u8> {
        self.ptr
    }

    pub fn phys(&self) -> u64 {
        self.ptr.as_ptr() as u64
    }
}

impl Drop for DmaMemory {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with this layout.
        unsafe { alloc::dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

fn rd<T: Copy>(addr: u64) -> T {
    // SAFETY: tests only pass addresses inside live queue memory or buffers.
    unsafe { core::ptr::read_volatile(addr as *const T) }
}

fn wr<T: Copy>(addr: u64, v: T) {
    // SAFETY: as for `rd`.
    unsafe { core::ptr::write_volatile(addr as *mut T, v) }
}

#[derive(Clone, Copy, Debug)]
pub struct Desc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
}

#[derive(Debug)]
pub struct Chain {
    pub head: u16,
    pub descs: Vec<Desc>,
}

/// The device's side of a split virtqueue (§2.7), written from the
/// specification's device requirements, not from the driver code.
pub struct DeviceQueue {
    a: QueueAddresses,
    size: u16,
    event_idx: bool,
    last_avail: u16,
    used_idx: u16,
}

impl DeviceQueue {
    pub fn new(a: QueueAddresses, size: u16, event_idx: bool) -> DeviceQueue {
        DeviceQueue { a, size, event_idx, last_avail: 0, used_idx: 0 }
    }

    pub fn avail_idx(&self) -> u16 {
        rd(self.a.driver + 2)
    }

    fn read_chain(&self, head: u16) -> Chain {
        let mut descs = Vec::new();
        let mut i = head;
        let mut seen_write = false;
        loop {
            assert!(i < self.size, "descriptor index out of range");
            assert!(descs.len() < self.size as usize, "descriptor loop");
            let d = self.a.desc + 16 * i as u64;
            let desc = Desc { addr: rd(d), len: rd(d + 8), flags: rd(d + 12) };
            let write = desc.flags & 2 != 0;
            assert!(write || !seen_write, "readable descriptor after a writable one");
            seen_write |= write;
            descs.push(desc);
            if desc.flags & 1 == 0 {
                return Chain { head, descs };
            }
            i = rd(d + 14);
        }
    }

    pub fn peek_chain(&self) -> Option<Chain> {
        if self.avail_idx() == self.last_avail {
            return None;
        }
        fence(Ordering::Acquire);
        let head = rd(self.a.driver + 4 + 2 * (self.last_avail % self.size) as u64);
        Some(self.read_chain(head))
    }

    pub fn take_chain(&mut self) -> Option<Chain> {
        let chain = self.peek_chain()?;
        self.last_avail = self.last_avail.wrapping_add(1);
        if self.event_idx {
            self.set_avail_event(self.last_avail);
        }
        Some(chain)
    }

    pub fn complete(&mut self, head: u16, len: u32) {
        self.write_elem(self.used_idx, head, len);
        fence(Ordering::Release);
        self.used_idx = self.used_idx.wrapping_add(1);
        wr(self.a.device + 2, self.used_idx);
    }

    /// Overwrite the most recent used element.
    pub fn rewrite_last(&mut self, id: u16, len: u32) {
        self.write_elem(self.used_idx.wrapping_sub(1), id, len);
    }

    fn write_elem(&self, idx: u16, id: u16, len: u32) {
        let e = self.a.device + 4 + 8 * (idx % self.size) as u64;
        wr(e, id as u32);
        wr(e + 4, len);
    }

    pub fn set_used_idx(&mut self, idx: u16) {
        self.used_idx = idx;
        wr(self.a.device + 2, idx);
    }

    /// Serve one buffer: hand `f` the readable bytes, scatter its answer
    /// over the writable pieces, and complete it with the bytes written.
    pub fn process_one(&mut self, mut f: impl FnMut(&[u8]) -> Vec<u8>) -> bool {
        let Some(chain) = self.take_chain() else { return false };
        let mut input = Vec::new();
        for d in chain.descs.iter().filter(|d| d.flags & 2 == 0) {
            // SAFETY: the driver handed us this buffer.
            input.extend_from_slice(unsafe { core::slice::from_raw_parts(d.addr as *const u8, d.len as usize) });
        }
        let output = f(&input);
        let mut written = 0;
        for d in chain.descs.iter().filter(|d| d.flags & 2 != 0) {
            let n = (d.len as usize).min(output.len() - written);
            // SAFETY: as above; the driver made this piece writable.
            unsafe { core::ptr::copy_nonoverlapping(output[written..].as_ptr(), d.addr as *mut u8, n) };
            written += n;
        }
        self.complete(chain.head, written as u32);
        true
    }

    /// Serve every available buffer; returns how many.
    pub fn process(&mut self, mut f: impl FnMut(&[u8]) -> Vec<u8>) -> usize {
        let mut n = 0;
        while self.process_one(&mut f) {
            n += 1;
        }
        n
    }

    pub fn set_no_notify(&self, on: bool) {
        wr(self.a.device, on as u16);
    }

    pub fn set_avail_event(&self, idx: u16) {
        wr(self.a.device + 4 + 8 * self.size as u64, idx);
    }

    pub fn no_interrupt(&self) -> bool {
        rd::<u16>(self.a.driver) & 1 != 0
    }

    pub fn used_event(&self) -> u16 {
        rd(self.a.driver + 4 + 2 * self.size as u64)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueueRegs {
    pub max_size: u16,
    pub size: u16,
    pub msix: u16,
    pub enable: u16,
    pub notify_off: u16,
    pub desc: u64,
    pub driver: u64,
    pub device: u64,
}

/// A device's common configuration structure (§4.1.4.3), following the
/// device requirements: status writes of 0 reset, FEATURES_OK is refused
/// when `accepts` says so, MSI-X vectors beyond `vectors` read back as
/// NO_VECTOR. Accesses of the wrong width fail the test.
pub struct CommonDevice {
    pub offered: u64,
    pub accepts: fn(u64) -> bool,
    pub vectors: u16,
    /// Status reads that still return the old value after a reset.
    pub reset_delay: Cell<u32>,
    pub status: Cell<u8>,
    pub status_writes: RefCell<Vec<u8>>,
    pub generation: Cell<u8>,
    /// Bump the generation on this many reads of it (a config change
    /// racing the driver).
    pub generation_churn: Cell<u32>,
    pub driver_features: Cell<u64>,
    pub queues: RefCell<Vec<QueueRegs>>,
    dfselect: Cell<u32>,
    gfselect: Cell<u32>,
    config_msix: Cell<u16>,
    queue_select: Cell<u16>,
}

impl CommonDevice {
    pub fn new(offered: u64, queues: &[u16]) -> CommonDevice {
        let queues = queues
            .iter()
            .enumerate()
            .map(|(i, &max_size)| QueueRegs {
                max_size,
                size: max_size,
                msix: 0xffff,
                notify_off: i as u16,
                ..QueueRegs::default()
            })
            .collect();
        CommonDevice {
            offered,
            accepts: |_| true,
            vectors: 4,
            reset_delay: Cell::new(0),
            status: Cell::new(0),
            status_writes: RefCell::new(vec![]),
            generation: Cell::new(0),
            generation_churn: Cell::new(0),
            driver_features: Cell::new(0),
            queues: RefCell::new(queues),
            dfselect: Cell::new(0),
            gfselect: Cell::new(0),
            config_msix: Cell::new(0xffff),
            queue_select: Cell::new(0),
        }
    }

    fn with_queue<T>(&self, f: impl FnOnce(&mut QueueRegs) -> T) -> Option<T> {
        self.queues.borrow_mut().get_mut(self.queue_select.get() as usize).map(f)
    }

    fn vector(&self, v: u16) -> u16 {
        if v < self.vectors { v } else { 0xffff }
    }
}

impl Mmio for CommonDevice {
    fn read8(&self, offset: usize) -> u8 {
        match offset {
            0x14 => {
                if self.reset_delay.get() > 0 {
                    self.reset_delay.set(self.reset_delay.get() - 1);
                    return 0xff;
                }
                self.status.get()
            }
            0x15 => {
                let g = self.generation.get();
                if self.generation_churn.get() > 0 {
                    self.generation_churn.set(self.generation_churn.get() - 1);
                    self.generation.set(g.wrapping_add(1));
                }
                g
            }
            _ => panic!("8-bit read at {offset:#x}"),
        }
    }

    fn read16(&self, offset: usize) -> u16 {
        match offset {
            0x10 => self.config_msix.get(),
            0x12 => self.queues.borrow().len() as u16,
            0x16 => self.queue_select.get(),
            0x18 => self.with_queue(|q| q.size).unwrap_or(0),
            0x1a => self.with_queue(|q| q.msix).unwrap_or(0xffff),
            0x1c => self.with_queue(|q| q.enable).unwrap_or(0),
            0x1e => self.with_queue(|q| q.notify_off).unwrap_or(0),
            _ => panic!("16-bit read at {offset:#x}"),
        }
    }

    fn read32(&self, offset: usize) -> u32 {
        match offset {
            0x00 => self.dfselect.get(),
            0x04 => match self.dfselect.get() {
                0 => self.offered as u32,
                1 => (self.offered >> 32) as u32,
                _ => 0,
            },
            0x08 => self.gfselect.get(),
            _ => panic!("32-bit read at {offset:#x}"),
        }
    }

    fn write8(&self, offset: usize, value: u8) {
        assert_eq!(offset, 0x14, "8-bit write at {offset:#x}");
        self.status_writes.borrow_mut().push(value);
        if value == 0 {
            self.status.set(0);
            self.driver_features.set(0);
            for q in self.queues.borrow_mut().iter_mut() {
                *q = QueueRegs {
                    max_size: q.max_size,
                    size: q.max_size,
                    msix: 0xffff,
                    notify_off: q.notify_off,
                    ..QueueRegs::default()
                };
            }
            return;
        }
        let mut value = value;
        if value & status::FEATURES_OK != 0 && self.status.get() & status::FEATURES_OK == 0 {
            let f = self.driver_features.get();
            if f & !self.offered != 0 || !(self.accepts)(f) {
                value &= !status::FEATURES_OK;
            }
        }
        self.status.set(value);
    }

    fn write16(&self, offset: usize, value: u16) {
        match offset {
            0x10 => self.config_msix.set(self.vector(value)),
            0x16 => self.queue_select.set(value),
            0x18 => {
                self.with_queue(|q| q.size = value);
            }
            0x1a => {
                let v = self.vector(value);
                self.with_queue(|q| q.msix = v);
            }
            0x1c => {
                self.with_queue(|q| q.enable = value);
            }
            _ => panic!("16-bit write at {offset:#x}"),
        }
    }

    fn write32(&self, offset: usize, value: u32) {
        let lo = |old: u64| (old & !0xffff_ffff) | value as u64;
        let hi = |old: u64| (old & 0xffff_ffff) | (value as u64) << 32;
        match offset {
            0x00 => self.dfselect.set(value),
            0x08 => self.gfselect.set(value),
            0x0c => {
                let old = self.driver_features.get();
                self.driver_features.set(if self.gfselect.get() == 0 { lo(old) } else { hi(old) });
            }
            0x20 => drop(self.with_queue(|q| q.desc = lo(q.desc))),
            0x24 => drop(self.with_queue(|q| q.desc = hi(q.desc))),
            0x28 => drop(self.with_queue(|q| q.driver = lo(q.driver))),
            0x2c => drop(self.with_queue(|q| q.driver = hi(q.driver))),
            0x30 => drop(self.with_queue(|q| q.device = lo(q.device))),
            0x34 => drop(self.with_queue(|q| q.device = hi(q.device))),
            _ => panic!("32-bit write at {offset:#x}"),
        }
    }
}

/// A byte-addressed register block (ISR, notification area) that logs
/// 16-bit writes.
#[derive(Default)]
pub struct Bytes {
    pub mem: RefCell<Vec<u8>>,
    pub writes: RefCell<Vec<(usize, u16)>>,
    /// Clear the first byte when it is read (the ISR's behaviour).
    pub read_clears: bool,
}

impl Mmio for Bytes {
    fn read8(&self, offset: usize) -> u8 {
        let v = self.mem.borrow()[offset];
        if self.read_clears {
            self.mem.borrow_mut()[offset] = 0;
        }
        v
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
        self.mem.borrow_mut()[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        self.writes.borrow_mut().push((offset, value));
    }
    fn write32(&self, _: usize, _: u32) {
        unimplemented!()
    }
}

/// One function's 256-byte configuration space, plain memory.
pub struct Config {
    pub bytes: RefCell<[u8; 256]>,
}

impl Config {
    pub fn new() -> Config {
        Config { bytes: RefCell::new([0; 256]) }
    }

    pub fn put(&self, offset: u8, data: &[u8]) {
        let o = offset as usize;
        self.bytes.borrow_mut()[o..o + data.len()].copy_from_slice(data);
    }
}

impl ConfigSpace for Config {
    fn read32(&self, _: Bdf, offset: u16) -> u32 {
        let o = offset as usize;
        u32::from_le_bytes(self.bytes.borrow()[o..o + 4].try_into().unwrap())
    }
    fn write32(&self, _: Bdf, offset: u16, value: u32) {
        self.put(offset as u8, &value.to_le_bytes());
    }
    fn write16(&self, _: Bdf, offset: u16, value: u16) {
        self.put(offset as u8, &value.to_le_bytes());
    }
    fn write8(&self, _: Bdf, offset: u16, value: u8) {
        self.put(offset as u8, &[value]);
    }
}
