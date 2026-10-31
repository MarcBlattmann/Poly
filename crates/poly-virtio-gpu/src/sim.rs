//! A simulated virtio GPU for the unit tests, written from the device
//! requirements of VIRTIO 1.2 (§2.7 split virtqueues, §4.1.4.3 common
//! configuration, §5.7 GPU device), not from the driver code.
//!
//! One shared state sits behind three register blocks: the common
//! configuration, the device configuration and the notification region.
//! Requests on the control queue are carried out when the driver notifies:
//! resources are host-side images, `TRANSFER_TO_HOST_2D` copies from their
//! guest backing (a scatter list of host addresses here), and
//! `RESOURCE_FLUSH` copies into the "screen" of every scanout showing the
//! resource, which the tests then look at.

extern crate std;

use core::cell::{Cell, RefCell};
use core::sync::atomic::{Ordering, fence};
use poly_pci::Mmio;
use poly_virtio::status;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::{alloc, vec, vec::Vec};

// The constants are spelled out here from the spec rather than taken from
// the crate, so a wrong constant there is caught.
const GET_DISPLAY_INFO: u32 = 0x100;
const RESOURCE_CREATE_2D: u32 = 0x101;
const RESOURCE_UNREF: u32 = 0x102;
const SET_SCANOUT: u32 = 0x103;
const RESOURCE_FLUSH: u32 = 0x104;
const TRANSFER_TO_HOST_2D: u32 = 0x105;
const RESOURCE_ATTACH_BACKING: u32 = 0x106;
const RESOURCE_DETACH_BACKING: u32 = 0x107;
const GET_EDID: u32 = 0x10a;
pub const OK_NODATA: u32 = 0x1100;
const OK_DISPLAY_INFO: u32 = 0x1101;
const OK_EDID: u32 = 0x1104;
pub const ERR_UNSPEC: u32 = 0x1200;
pub const ERR_INVALID_SCANOUT_ID: u32 = 0x1202;
pub const ERR_INVALID_RESOURCE_ID: u32 = 0x1203;
pub const ERR_INVALID_PARAMETER: u32 = 0x1205;
const FLAG_FENCE: u32 = 1;
/// The formats §5.7.6.8 lists, all 4 bytes per pixel.
const FORMATS: [u32; 8] = [1, 2, 3, 4, 67, 68, 121, 134];

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

    pub fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: `len` bytes allocated in `new`, ours alone.
        unsafe { core::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
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

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn le64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
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

/// A scanout: what the "monitor" reports and what it shows.
#[derive(Clone, Debug, Default)]
pub struct Scanout {
    pub enabled: bool,
    pub width: u32,
    pub height: u32,
    /// The resource shown (0: none) and the rectangle of it.
    pub resource: u32,
    pub rect: (u32, u32, u32, u32),
    /// What the monitor shows, `rect.2` × `rect.3` pixels of 4 bytes, as of
    /// the last flush.
    pub screen: Vec<u8>,
    pub edid: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct Resource {
    pub format: u32,
    pub width: u32,
    pub height: u32,
    /// (address, length) pieces, empty without backing.
    pub backing: Vec<(u64, u32)>,
    /// The host-side copy, `width * height * 4` bytes.
    pub image: Vec<u8>,
}

impl Resource {
    /// Read `len` bytes at `offset` of the backing, across pieces.
    fn read_backing(&self, mut offset: u64, out: &mut [u8]) -> bool {
        let mut done = 0;
        for &(addr, len) in &self.backing {
            if done == out.len() {
                break;
            }
            if offset >= len as u64 {
                offset -= len as u64;
                continue;
            }
            let n = ((len as u64 - offset) as usize).min(out.len() - done);
            for (i, b) in out[done..done + n].iter_mut().enumerate() {
                *b = rd::<u8>(addr + offset + i as u64);
            }
            done += n;
            offset = 0;
        }
        done == out.len()
    }
}

/// A command the device carried out (or refused), for the tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Logged {
    pub ty: u32,
    /// The request after the header.
    pub body: Vec<u8>,
    pub response: u32,
}

#[derive(Debug, Default)]
pub struct Device {
    pub scanouts: Vec<Scanout>,
    pub resources: BTreeMap<u32, Resource>,
    pub log: Vec<Logged>,
    /// Do not answer when notified (a hung device).
    pub hold: bool,
    /// Answer the next request with this type instead.
    pub fail_next: Option<u32>,
    /// Answer with the wrong fence ID / without the fence flag.
    pub bad_fence: bool,
    /// Hand back this id in the next used element (a broken device).
    pub bogus_id: Option<u32>,
    /// Resources destroyed while a scanout still showed them.
    pub unref_while_shown: u32,
    /// Backing detached while a scanout still showed the resource.
    pub detach_while_shown: u32,
    pub notifies: u32,
    last_avail: u16,
    used_idx: u16,
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
    pub generation: Cell<u8>,
    pub events: Cell<u32>,
    pub num_scanouts: Cell<u32>,
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

impl Sim {
    /// A device offering `offered`, with `num_scanouts` scanouts of which
    /// those in `monitors` (index, width, height) are enabled, and queues
    /// of at most `max_queue` entries.
    pub fn new(offered: u64, num_scanouts: u32, monitors: &[(usize, u32, u32)], max_queue: u16) -> Sim {
        let mut scanouts = vec![Scanout::default(); 16];
        for &(i, w, h) in monitors {
            scanouts[i] = Scanout { enabled: true, width: w, height: h, ..Scanout::default() };
        }
        let q = QueueRegs { max_size: max_queue, size: max_queue, msix: 0xffff, ..Default::default() };
        Sim(Rc::new(Inner {
            offered,
            status: Cell::new(0),
            driver_features: Cell::new(0),
            dfselect: Cell::new(0),
            gfselect: Cell::new(0),
            queue_select: Cell::new(0),
            queues: RefCell::new(vec![q; 2]),
            generation: Cell::new(0),
            events: Cell::new(0),
            num_scanouts: Cell::new(num_scanouts),
            dev: RefCell::new(Device { scanouts, ..Device::default() }),
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

    /// The monitor on scanout `i` changes (or goes away, with `None`): the
    /// display event is raised (§5.7.4.2).
    pub fn replug(&self, i: usize, size: Option<(u32, u32)>) {
        {
            let mut d = self.dev();
            let s = &mut d.scanouts[i];
            s.enabled = size.is_some();
            (s.width, s.height) = size.unwrap_or((0, 0));
        }
        self.0.events.set(self.0.events.get() | 1);
        self.0.generation.set(self.0.generation.get().wrapping_add(1));
    }

    /// Pixel (x, y) of what scanout `i` shows, as stored (B, G, R, X).
    pub fn screen_pixel(&self, i: usize, x: u32, y: u32) -> [u8; 4] {
        let d = self.0.dev.borrow();
        let s = &d.scanouts[i];
        let at = ((y * s.rect.2 + x) * 4) as usize;
        s.screen[at..at + 4].try_into().unwrap()
    }

    pub fn types(&self) -> Vec<u32> {
        self.0.dev.borrow().log.iter().map(|l| l.ty).collect()
    }

    fn reset_device(&self) {
        for q in self.0.queues.borrow_mut().iter_mut() {
            *q = QueueRegs { max_size: q.max_size, size: q.max_size, msix: 0xffff, ..Default::default() };
        }
        self.0.driver_features.set(0);
        let mut d = self.dev();
        d.last_avail = 0;
        d.used_idx = 0;
        d.resources.clear();
        for s in d.scanouts.iter_mut() {
            s.resource = 0;
        }
    }

    /// Carry out every request on the control queue.
    fn process(&self) {
        let q = self.queue(0);
        if q.enable == 0 || self.0.status.get() & status::DRIVER_OK == 0 || self.dev().hold {
            return;
        }
        let event_idx = self.negotiated(poly_virtio::feature::EVENT_IDX);
        loop {
            let last = self.dev().last_avail;
            if rd::<u16>(q.driver + 2) == last {
                break;
            }
            fence(Ordering::Acquire);
            let head = rd::<u16>(q.driver + 4 + 2 * (last % q.size) as u64);
            self.dev().last_avail = last.wrapping_add(1);
            if event_idx {
                // Notify me about the next one too (§2.7.10).
                wr(q.device + 4 + 8 * q.size as u64, last.wrapping_add(1));
            }
            // §2.7.4.2: device-readable descriptors come first.
            let (mut req, mut out) = (Vec::new(), Vec::new());
            let mut i = head;
            for _ in 0..q.size {
                assert!(i < q.size, "descriptor index out of range");
                let d = q.desc + 16 * i as u64;
                let (addr, len, flags, next) = (rd::<u64>(d), rd::<u32>(d + 8), rd::<u16>(d + 12), rd::<u16>(d + 14));
                if flags & 2 != 0 {
                    out.push((addr, len));
                } else {
                    assert!(out.is_empty(), "a device-readable descriptor after a device-writable one");
                    req.extend((0..len as u64).map(|k| rd::<u8>(addr + k)));
                }
                if flags & 1 == 0 {
                    break;
                }
                i = next;
            }
            let resp = self.execute(&req);
            // Write the response into the writable buffers.
            let mut written = 0usize;
            for (addr, len) in out {
                let n = (len as usize).min(resp.len() - written);
                for k in 0..n {
                    wr(addr + k as u64, resp[written + k]);
                }
                written += n;
            }
            assert_eq!(written, resp.len(), "the driver's response buffer is too short");
            let mut d = self.dev();
            let id = d.bogus_id.take().unwrap_or(head as u32);
            let e = q.device + 4 + 8 * (d.used_idx % q.size) as u64;
            wr(e, id);
            wr(e + 4, written as u32);
            d.used_idx = d.used_idx.wrapping_add(1);
            fence(Ordering::Release);
            wr(q.device + 2, d.used_idx);
        }
    }

    /// Carry out one request; the response.
    fn execute(&self, req: &[u8]) -> Vec<u8> {
        assert!(req.len() >= 24, "request shorter than its header");
        let (ty, flags, fence_id) = (le32(req, 0), le32(req, 4), le64(req, 8));
        let body = &req[24..];
        let mut guard = self.dev();
        let d = &mut *guard;
        let n = self.0.num_scanouts.get();
        let mut payload = Vec::new();
        let result = match d.fail_next.take() {
            Some(t) => t,
            None => match ty {
                GET_DISPLAY_INFO => {
                    for i in 0..16 {
                        let s = &d.scanouts[i];
                        let on = (i as u32) < n && s.enabled;
                        let (w, h) = if on { (s.width, s.height) } else { (0, 0) };
                        for v in [0, 0, w, h, on as u32, 0] {
                            payload.extend_from_slice(&v.to_le_bytes());
                        }
                    }
                    OK_DISPLAY_INFO
                }
                GET_EDID => {
                    let scanout = le32(body, 0);
                    if scanout >= n {
                        ERR_INVALID_PARAMETER
                    } else {
                        let e = d.scanouts[scanout as usize].edid.clone();
                        payload.extend_from_slice(&(e.len() as u32).to_le_bytes());
                        payload.extend_from_slice(&[0; 4]);
                        payload.extend_from_slice(&e);
                        payload.resize(8 + 1024, 0);
                        OK_EDID
                    }
                }
                RESOURCE_CREATE_2D => {
                    let (id, format, w, h) = (le32(body, 0), le32(body, 4), le32(body, 8), le32(body, 12));
                    if id == 0 || d.resources.contains_key(&id) {
                        ERR_INVALID_RESOURCE_ID
                    } else if !FORMATS.contains(&format) || w == 0 || h == 0 {
                        ERR_INVALID_PARAMETER
                    } else {
                        let image = vec![0; (w * h * 4) as usize];
                        d.resources.insert(id, Resource { format, width: w, height: h, backing: Vec::new(), image });
                        OK_NODATA
                    }
                }
                RESOURCE_UNREF => {
                    let id = le32(body, 0);
                    if d.resources.remove(&id).is_none() {
                        ERR_INVALID_RESOURCE_ID
                    } else {
                        if d.scanouts.iter().any(|s| s.resource == id) {
                            d.unref_while_shown += 1;
                        }
                        OK_NODATA
                    }
                }
                SET_SCANOUT => {
                    let (x, y, w, h) = (le32(body, 0), le32(body, 4), le32(body, 8), le32(body, 12));
                    let (scanout, id) = (le32(body, 16), le32(body, 20));
                    if scanout >= n {
                        ERR_INVALID_SCANOUT_ID
                    } else if id == 0 {
                        let s = &mut d.scanouts[scanout as usize];
                        (s.resource, s.rect, s.screen) = (0, (0, 0, 0, 0), Vec::new());
                        OK_NODATA
                    } else {
                        match d.resources.get(&id).map(|r| (r.width, r.height)) {
                            None => ERR_INVALID_RESOURCE_ID,
                            Some((rw, rh)) if x as u64 + w as u64 > rw as u64 || y as u64 + h as u64 > rh as u64 => {
                                ERR_INVALID_PARAMETER
                            }
                            Some(_) if w == 0 || h == 0 => ERR_INVALID_PARAMETER,
                            Some(_) => {
                                let s = &mut d.scanouts[scanout as usize];
                                (s.resource, s.rect, s.screen) = (id, (x, y, w, h), vec![0; (w * h * 4) as usize]);
                                OK_NODATA
                            }
                        }
                    }
                }
                RESOURCE_FLUSH => {
                    let (x, y, w, h, id) =
                        (le32(body, 0), le32(body, 4), le32(body, 8), le32(body, 12), le32(body, 16));
                    match d.resources.get(&id).cloned() {
                        None => ERR_INVALID_RESOURCE_ID,
                        Some(r) if x as u64 + w as u64 > r.width as u64 || y as u64 + h as u64 > r.height as u64 => {
                            ERR_INVALID_PARAMETER
                        }
                        Some(r) => {
                            for s in d.scanouts.iter_mut().filter(|s| s.resource == id) {
                                let (sx, sy, sw, sh) = s.rect;
                                for py in y.max(sy)..(y + h).min(sy + sh) {
                                    for px in x.max(sx)..(x + w).min(sx + sw) {
                                        let src = ((py * r.width + px) * 4) as usize;
                                        let dst = (((py - sy) * sw + (px - sx)) * 4) as usize;
                                        s.screen[dst..dst + 4].copy_from_slice(&r.image[src..src + 4]);
                                    }
                                }
                            }
                            OK_NODATA
                        }
                    }
                }
                TRANSFER_TO_HOST_2D => {
                    let (x, y, w, h) = (le32(body, 0), le32(body, 4), le32(body, 8), le32(body, 12));
                    let (offset, id) = (le64(body, 16), le32(body, 24));
                    match d.resources.get_mut(&id) {
                        None => ERR_INVALID_RESOURCE_ID,
                        Some(r) if x as u64 + w as u64 > r.width as u64 || y as u64 + h as u64 > r.height as u64 => {
                            ERR_INVALID_PARAMETER
                        }
                        Some(r) if r.backing.is_empty() => ERR_UNSPEC,
                        Some(r) => {
                            let stride = r.width as u64 * 4;
                            let mut row = vec![0u8; w as usize * 4];
                            let mut ok = true;
                            for line in 0..h {
                                ok &= r.read_backing(offset + line as u64 * stride, &mut row);
                                let dst = (((y + line) * r.width + x) * 4) as usize;
                                r.image[dst..dst + row.len()].copy_from_slice(&row);
                            }
                            if ok { OK_NODATA } else { ERR_INVALID_PARAMETER }
                        }
                    }
                }
                RESOURCE_ATTACH_BACKING => {
                    let (id, count) = (le32(body, 0), le32(body, 4) as usize);
                    assert!(body.len() >= 8 + 16 * count, "attach request shorter than its entries");
                    match d.resources.get_mut(&id) {
                        None => ERR_INVALID_RESOURCE_ID,
                        Some(r) if !r.backing.is_empty() => ERR_UNSPEC,
                        Some(r) => {
                            r.backing = (0..count).map(|i| (le64(body, 8 + 16 * i), le32(body, 16 + 16 * i))).collect();
                            OK_NODATA
                        }
                    }
                }
                RESOURCE_DETACH_BACKING => {
                    let id = le32(body, 0);
                    let shown = d.scanouts.iter().any(|s| s.resource == id);
                    match d.resources.get_mut(&id) {
                        None => ERR_INVALID_RESOURCE_ID,
                        Some(r) => {
                            r.backing.clear();
                            if shown {
                                d.detach_while_shown += 1;
                            }
                            OK_NODATA
                        }
                    }
                }
                _ => ERR_UNSPEC,
            },
        };
        if result != OK_DISPLAY_INFO && result != OK_EDID {
            payload.clear();
        }
        d.log.push(Logged { ty, body: body.to_vec(), response: result });
        let mut resp = Vec::with_capacity(24 + payload.len());
        resp.extend_from_slice(&result.to_le_bytes());
        let (rflags, rfence) = if flags & FLAG_FENCE == 0 {
            (0, 0)
        } else if d.bad_fence {
            (FLAG_FENCE, fence_id + 1000)
        } else {
            (FLAG_FENCE, fence_id)
        };
        resp.extend_from_slice(&rflags.to_le_bytes());
        resp.extend_from_slice(&rfence.to_le_bytes());
        resp.extend_from_slice(&[0; 8]);
        resp.extend_from_slice(&payload);
        resp
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
            0x10 => 0xffff,
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
            // No MSI-X entries: every vector is refused.
            0x10 => {}
            0x16 => self.0.queue_select.set(value),
            0x18 => qs[sel].size = value,
            0x1a => qs[sel].msix = 0xffff,
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
        panic!("8-bit configuration read at {offset:#x}");
    }

    fn read16(&self, offset: usize) -> u16 {
        panic!("16-bit configuration read at {offset:#x}");
    }

    /// `events_read`, `events_clear` (reads 0), `num_scanouts`,
    /// `num_capsets` (0: no 3D).
    fn read32(&self, offset: usize) -> u32 {
        match offset {
            0 => self.0.events.get(),
            4 => 0,
            8 => self.0.num_scanouts.get(),
            12 => 0,
            _ => panic!("configuration read at {offset:#x}"),
        }
    }

    fn write8(&self, offset: usize, _: u8) {
        panic!("8-bit configuration write at {offset:#x}");
    }

    fn write16(&self, offset: usize, _: u16) {
        panic!("16-bit configuration write at {offset:#x}");
    }

    /// Only `events_clear` is writable: it clears the bits written.
    fn write32(&self, offset: usize, value: u32) {
        assert_eq!(offset, 4, "configuration write at {offset:#x}");
        self.0.events.set(self.0.events.get() & !value);
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
        assert_eq!(value, 0, "notification for queue {value}: only the control queue is used");
        let sim = Sim(self.0.clone());
        sim.dev().notifies += 1;
        sim.process();
    }

    fn write32(&self, _: usize, _: u32) {
        unimplemented!()
    }
}
