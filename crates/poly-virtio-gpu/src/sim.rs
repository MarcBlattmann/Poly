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
//!
//! The 3D side (§5.7.6.10) is modelled as far as the driver can tell it
//! apart: capsets with their data, contexts (and which resources each may
//! use), 3D resources as host byte arrays copied to and from their guest
//! backing by the 3D transfers (box, stride, offset), a toy command stream
//! for `SUBMIT_3D` (the real ones are Mesa's business, not the driver's:
//! here a stream of `FILL resource byte` dwords), blob resources and the
//! host-visible region they map into. Fenced 3D requests can be held back
//! and answered later in any order ([`Sim::retire`]), as a host GPU
//! finishing work does, and the queue's MSI-X vector is recorded.

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
const GET_CAPSET_INFO: u32 = 0x108;
const GET_CAPSET: u32 = 0x109;
const RESOURCE_CREATE_BLOB: u32 = 0x10c;
const CTX_CREATE: u32 = 0x200;
const CTX_DESTROY: u32 = 0x201;
const CTX_ATTACH_RESOURCE: u32 = 0x202;
const CTX_DETACH_RESOURCE: u32 = 0x203;
const RESOURCE_CREATE_3D: u32 = 0x204;
const TRANSFER_TO_HOST_3D: u32 = 0x205;
const TRANSFER_FROM_HOST_3D: u32 = 0x206;
const SUBMIT_3D: u32 = 0x207;
const RESOURCE_MAP_BLOB: u32 = 0x208;
const RESOURCE_UNMAP_BLOB: u32 = 0x209;
const OK_CAPSET_INFO: u32 = 0x1102;
const OK_CAPSET: u32 = 0x1103;
const OK_MAP_INFO: u32 = 0x1106;
pub const ERR_INVALID_CONTEXT_ID: u32 = 0x1204;
const FLAG_INFO_RING_IDX: u32 = 2;
/// The toy command stream's one command: `FILL resource byte`.
pub const TOY_FILL: u32 = 1;
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
    /// The header's flags, context and ring.
    pub flags: u32,
    pub ctx_id: u32,
    pub ring_idx: u8,
}

/// A 3D (or blob) resource: host bytes, maybe guest backing.
#[derive(Clone, Debug, Default)]
pub struct Resource3d {
    /// Width, height, depth, bytes per texel (1 for buffers).
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub cpp: u32,
    pub backing: Vec<(u64, u32)>,
    pub host: Vec<u8>,
    /// A blob: (blob_mem, blob_flags, blob_id).
    pub blob: Option<(u32, u32, u64)>,
    /// Where in the host-visible region it is mapped.
    pub mapped: Option<u64>,
}

/// A 3D context: its capset and the resources it may use.
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub capset: u32,
    pub name: Vec<u8>,
    pub attached: Vec<u32>,
}

/// A response written later (a fenced 3D request whose work is not done).
#[derive(Clone, Debug)]
struct Held {
    head: u16,
    out: Vec<(u64, u32)>,
    resp: Vec<u8>,
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
    /// Capsets: (id, max_version, data).
    pub capsets: Vec<(u32, u32, Vec<u8>)>,
    pub contexts: BTreeMap<u32, Context>,
    pub resources_3d: BTreeMap<u32, Resource3d>,
    /// Hold back the responses of fenced `SUBMIT_3D` and 3D transfers until
    /// [`Sim::retire`].
    pub hold_3d: bool,
    held: Vec<Held>,
    /// The host-visible region's size.
    pub shm_len: u64,
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
    /// Whether the function has MSI-X entries (vectors written are kept).
    pub msix_entries: Cell<bool>,
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
            msix_entries: Cell::new(false),
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

    /// Answer the held-back 3D requests, in the order of `order` (indices
    /// into the held list; the rest after, in order).
    pub fn retire(&self, order: &[usize]) {
        let q = self.queue(0);
        let mut held = core::mem::take(&mut self.dev().held);
        let mut sorted = Vec::new();
        for &i in order {
            if i < held.len() {
                sorted.push(held[i].clone());
            }
        }
        for (i, h) in held.drain(..).enumerate() {
            if !order.contains(&i) {
                sorted.push(h);
            }
        }
        for h in sorted {
            self.answer(q, h.head, &h.out, &h.resp);
        }
    }

    /// How many responses are held back.
    pub fn held(&self) -> usize {
        self.0.dev.borrow().held.len()
    }

    /// Write `resp` into the writable buffers `out` and put `head` on the
    /// used ring.
    fn answer(&self, q: QueueRegs, head: u16, out: &[(u64, u32)], resp: &[u8]) {
        let mut written = 0usize;
        for &(addr, len) in out {
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
            let ty = le32(&req, 0);
            let fenced = le32(&req, 4) & FLAG_FENCE != 0;
            let slow = matches!(ty, SUBMIT_3D | TRANSFER_TO_HOST_3D | TRANSFER_FROM_HOST_3D);
            if fenced && slow && self.dev().hold_3d {
                self.dev().held.push(Held { head, out, resp });
                continue;
            }
            self.answer(q, head, &out, &resp);
        }
    }

    /// Carry out one request; the response.
    fn execute(&self, req: &[u8]) -> Vec<u8> {
        assert!(req.len() >= 24, "request shorter than its header");
        let (ty, flags, fence_id) = (le32(req, 0), le32(req, 4), le64(req, 8));
        let (ctx_id, ring_idx) = (le32(req, 16), req[20]);
        let body = &req[24..];
        let virgl = self.negotiated(1 << 0);
        let blob = self.negotiated(1 << 3);
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
                GET_CAPSET_INFO if virgl => {
                    let index = le32(body, 0) as usize;
                    match d.capsets.get(index) {
                        None => ERR_INVALID_PARAMETER,
                        Some((id, v, data)) => {
                            for x in [*id, *v, data.len() as u32, 0] {
                                payload.extend_from_slice(&x.to_le_bytes());
                            }
                            OK_CAPSET_INFO
                        }
                    }
                }
                GET_CAPSET if virgl => {
                    let (id, version) = (le32(body, 0), le32(body, 4));
                    match d.capsets.iter().find(|c| c.0 == id) {
                        Some((_, max, data)) if version <= *max => {
                            payload.extend_from_slice(data);
                            OK_CAPSET
                        }
                        _ => ERR_INVALID_PARAMETER,
                    }
                }
                CTX_CREATE if virgl => {
                    let (nlen, init) = (le32(body, 0) as usize, le32(body, 4));
                    if ctx_id == 0 || d.contexts.contains_key(&ctx_id) || nlen > 64 {
                        ERR_INVALID_CONTEXT_ID
                    } else if init != 0 && !self.negotiated(1 << 4) {
                        ERR_INVALID_PARAMETER
                    } else {
                        let name = body[8..8 + nlen].to_vec();
                        d.contexts.insert(ctx_id, Context { capset: init & 0xff, name, attached: Vec::new() });
                        OK_NODATA
                    }
                }
                CTX_DESTROY if virgl => {
                    if d.contexts.remove(&ctx_id).is_some() {
                        OK_NODATA
                    } else {
                        ERR_INVALID_CONTEXT_ID
                    }
                }
                CTX_ATTACH_RESOURCE | CTX_DETACH_RESOURCE if virgl => {
                    let id = le32(body, 0);
                    let known = d.resources_3d.contains_key(&id) || d.resources.contains_key(&id);
                    match d.contexts.get_mut(&ctx_id) {
                        None => ERR_INVALID_CONTEXT_ID,
                        Some(_) if !known => ERR_INVALID_RESOURCE_ID,
                        Some(c) => {
                            c.attached.retain(|&r| r != id);
                            if ty == CTX_ATTACH_RESOURCE {
                                c.attached.push(id);
                            }
                            OK_NODATA
                        }
                    }
                }
                RESOURCE_CREATE_3D if virgl => {
                    let f: Vec<u32> = (0..11).map(|i| le32(body, 4 * i)).collect();
                    let (id, target, w, h, depth, array) = (f[0], f[1], f[4], f[5], f[6], f[7]);
                    if id == 0 || d.resources_3d.contains_key(&id) || d.resources.contains_key(&id) {
                        ERR_INVALID_RESOURCE_ID
                    } else if w == 0 || h == 0 {
                        ERR_INVALID_PARAMETER
                    } else {
                        // Target 0 is a buffer (bytes); the rest 4-byte texels.
                        let cpp = if target == 0 { 1 } else { 4 };
                        let depth = depth.max(1) * array.max(1);
                        let host = vec![0; (w * h * depth * cpp) as usize];
                        d.resources_3d
                            .insert(id, Resource3d { width: w, height: h, depth, cpp, host, ..Resource3d::default() });
                        OK_NODATA
                    }
                }
                TRANSFER_TO_HOST_3D | TRANSFER_FROM_HOST_3D if virgl => {
                    let b: Vec<u32> = (0..6).map(|i| le32(body, 4 * i)).collect();
                    let (offset, id, stride) = (le64(body, 24), le32(body, 32), le32(body, 40));
                    let attached = d.contexts.get(&ctx_id).map(|c| c.attached.contains(&id));
                    match (attached, d.resources_3d.get_mut(&id)) {
                        (None, _) => ERR_INVALID_CONTEXT_ID,
                        (_, None) | (Some(false), _) => ERR_INVALID_RESOURCE_ID,
                        (_, Some(r)) if r.backing.is_empty() => ERR_UNSPEC,
                        (_, Some(r)) if b[0] + b[3] > r.width || b[1] + b[4] > r.height || b[2] + b[5] > r.depth => {
                            ERR_INVALID_PARAMETER
                        }
                        (_, Some(r)) => {
                            let line = (b[3] * r.cpp) as usize;
                            let stride = if stride == 0 { r.width * r.cpp } else { stride } as u64;
                            let mut ok = true;
                            for z in 0..b[5] {
                                for y in 0..b[4] {
                                    let guest = offset + ((z * b[4] + y) as u64) * stride;
                                    let at = ((((b[2] + z) * r.height + b[1] + y) * r.width + b[0]) * r.cpp) as usize;
                                    if ty == TRANSFER_TO_HOST_3D {
                                        let mut row = vec![0u8; line];
                                        ok &= read_pieces(&r.backing, guest, &mut row);
                                        r.host[at..at + line].copy_from_slice(&row);
                                    } else {
                                        ok &= write_pieces(&r.backing, guest, &r.host[at..at + line]);
                                    }
                                }
                            }
                            if ok { OK_NODATA } else { ERR_INVALID_PARAMETER }
                        }
                    }
                }
                SUBMIT_3D if virgl => {
                    let size = le32(body, 0) as usize;
                    match d.contexts.get(&ctx_id).cloned() {
                        None => ERR_INVALID_CONTEXT_ID,
                        Some(_) if body.len() < 8 + size || !size.is_multiple_of(12) => ERR_INVALID_PARAMETER,
                        Some(c) => {
                            let mut result = OK_NODATA;
                            for cmd in body[8..8 + size].chunks_exact(12) {
                                let (op, id, value) = (le32(cmd, 0), le32(cmd, 4), le32(cmd, 8));
                                match d.resources_3d.get_mut(&id) {
                                    Some(r) if op == TOY_FILL && c.attached.contains(&id) => r.host.fill(value as u8),
                                    _ => result = ERR_INVALID_PARAMETER,
                                }
                            }
                            result
                        }
                    }
                }
                RESOURCE_CREATE_BLOB if blob => {
                    let (id, mem, bflags, n) = (le32(body, 0), le32(body, 4), le32(body, 8), le32(body, 12) as usize);
                    let (blob_id, size) = (le64(body, 16), le64(body, 24));
                    assert!(body.len() >= 32 + 16 * n, "blob request shorter than its entries");
                    let host3d = mem == 2 || mem == 3;
                    if id == 0 || d.resources_3d.contains_key(&id) || d.resources.contains_key(&id) {
                        ERR_INVALID_RESOURCE_ID
                    } else if !(1..=3).contains(&mem) || size == 0 || (mem == 1 && n == 0) || (mem == 2 && n != 0) {
                        ERR_INVALID_PARAMETER
                    } else if host3d && !d.contexts.contains_key(&ctx_id) {
                        ERR_INVALID_CONTEXT_ID
                    } else {
                        let backing = (0..n).map(|i| (le64(body, 32 + 16 * i), le32(body, 40 + 16 * i))).collect();
                        let r = Resource3d {
                            width: size as u32,
                            height: 1,
                            depth: 1,
                            cpp: 1,
                            backing,
                            host: vec![0; size as usize],
                            blob: Some((mem, bflags, blob_id)),
                            mapped: None,
                        };
                        d.resources_3d.insert(id, r);
                        OK_NODATA
                    }
                }
                RESOURCE_MAP_BLOB if blob => {
                    let (id, offset) = (le32(body, 0), le64(body, 8));
                    let shm = d.shm_len;
                    match d.resources_3d.get_mut(&id) {
                        None => ERR_INVALID_RESOURCE_ID,
                        Some(r) if r.mapped.is_some() || !r.blob.is_some_and(|b| b.0 != 1 && b.1 & 1 != 0) => {
                            ERR_INVALID_PARAMETER
                        }
                        Some(r) if offset % 4096 != 0 || offset + r.host.len() as u64 > shm => ERR_INVALID_PARAMETER,
                        Some(r) => {
                            r.mapped = Some(offset);
                            payload.extend_from_slice(&1u32.to_le_bytes());
                            payload.extend_from_slice(&[0; 4]);
                            OK_MAP_INFO
                        }
                    }
                }
                RESOURCE_UNMAP_BLOB if blob => match d.resources_3d.get_mut(&le32(body, 0)).map(|r| r.mapped.take()) {
                    Some(Some(_)) => OK_NODATA,
                    Some(None) => ERR_INVALID_PARAMETER,
                    None => ERR_INVALID_RESOURCE_ID,
                },
                _ => ERR_UNSPEC,
            },
        };
        // 3D resources share the 2D commands that apply to any resource.
        if result == ERR_INVALID_RESOURCE_ID
            && matches!(ty, RESOURCE_UNREF | RESOURCE_ATTACH_BACKING | RESOURCE_DETACH_BACKING)
        {
            let id = le32(body, 0);
            if let Some(r) = d.resources_3d.get_mut(&id) {
                result_3d(ty, body, r);
                if ty == RESOURCE_UNREF {
                    d.resources_3d.remove(&id);
                    for c in d.contexts.values_mut() {
                        c.attached.retain(|&x| x != id);
                    }
                }
                return self.respond(d, ty, flags, fence_id, ctx_id, ring_idx, body, OK_NODATA, Vec::new());
            }
        }
        if ![OK_DISPLAY_INFO, OK_EDID, OK_CAPSET_INFO, OK_CAPSET, OK_MAP_INFO].contains(&result) {
            payload.clear();
        }
        self.respond(d, ty, flags, fence_id, ctx_id, ring_idx, body, result, payload)
    }

    /// Log the request and make its response.
    #[allow(clippy::too_many_arguments)]
    fn respond(
        &self,
        d: &mut Device,
        ty: u32,
        flags: u32,
        fence_id: u64,
        ctx_id: u32,
        ring_idx: u8,
        body: &[u8],
        result: u32,
        payload: Vec<u8>,
    ) -> Vec<u8> {
        d.log.push(Logged { ty, body: body.to_vec(), response: result, flags, ctx_id, ring_idx });
        let mut resp = Vec::with_capacity(24 + payload.len());
        resp.extend_from_slice(&result.to_le_bytes());
        let (rflags, rfence) = if flags & FLAG_FENCE == 0 {
            (0, 0)
        } else if d.bad_fence {
            (FLAG_FENCE, fence_id + 1000)
        } else {
            (flags & (FLAG_FENCE | FLAG_INFO_RING_IDX), fence_id)
        };
        resp.extend_from_slice(&rflags.to_le_bytes());
        resp.extend_from_slice(&rfence.to_le_bytes());
        resp.extend_from_slice(&ctx_id.to_le_bytes());
        resp.extend_from_slice(&[ring_idx, 0, 0, 0]);
        resp.extend_from_slice(&payload);
        resp
    }
}

/// `RESOURCE_ATTACH_BACKING` / `RESOURCE_DETACH_BACKING` on a 3D resource.
fn result_3d(ty: u32, body: &[u8], r: &mut Resource3d) {
    match ty {
        RESOURCE_ATTACH_BACKING => {
            let count = le32(body, 4) as usize;
            r.backing = (0..count).map(|i| (le64(body, 8 + 16 * i), le32(body, 16 + 16 * i))).collect();
        }
        RESOURCE_DETACH_BACKING => r.backing.clear(),
        _ => {}
    }
}

/// Read `out.len()` bytes at `offset` of a scatter list.
fn read_pieces(pieces: &[(u64, u32)], mut offset: u64, out: &mut [u8]) -> bool {
    let mut done = 0;
    for &(addr, len) in pieces {
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

/// Write `data` at `offset` of a scatter list.
fn write_pieces(pieces: &[(u64, u32)], mut offset: u64, data: &[u8]) -> bool {
    let mut done = 0;
    for &(addr, len) in pieces {
        if done == data.len() {
            break;
        }
        if offset >= len as u64 {
            offset -= len as u64;
            continue;
        }
        let n = ((len as u64 - offset) as usize).min(data.len() - done);
        for (i, b) in data[done..done + n].iter().enumerate() {
            wr(addr + offset + i as u64, *b);
        }
        done += n;
        offset = 0;
    }
    done == data.len()
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
            // Without MSI-X entries every vector is refused.
            0x10 => {}
            0x16 => self.0.queue_select.set(value),
            0x18 => qs[sel].size = value,
            0x1a => qs[sel].msix = if self.0.msix_entries.get() { value } else { 0xffff },
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
    /// `num_capsets`.
    fn read32(&self, offset: usize) -> u32 {
        match offset {
            0 => self.0.events.get(),
            4 => 0,
            8 => self.0.num_scanouts.get(),
            12 => self.0.dev.borrow().capsets.len() as u32,
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
