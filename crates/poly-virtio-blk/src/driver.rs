//! The driver: block requests in, descriptor chains out, status bytes back.

use crate::{BlkConfig, ConfigError, Dma, HEADER_LEN, ID_LEN, SECTOR_SIZE, req_status, req_type, select_features};
use core::ptr::{self, NonNull};
use poly_block::{BlockDevice, Completion, DeviceInfo, IoBuf, IoError, Op, Request, RequestId, SubmitError};
use poly_pci::Mmio;
use poly_virtio::{
    AddError, Buffer, CommonCfg, DescState, Layout, Notifier, PciTransport, SetupError, SplitQueue, begin, feature,
    status,
};

/// Bytes of the request area per queue entry: the header at 0, the status
/// byte at 16, room for a device ID at 32.
pub const SLOT_LEN: usize = 64;
const STATUS_AT: usize = HEADER_LEN;
const ID_AT: usize = 32;
/// Written to the status byte before a request goes out; no device writes
/// it, so seeing it back means the device never answered.
const NO_STATUS: u8 = 0xff;
/// Polls of the status register while waiting for a reset to finish.
const RESET_POLLS: u32 = 1000;
const NIL: u16 = u16::MAX;

/// Bytes of request area a queue of `queue_size` entries needs.
pub const fn request_area_len(queue_size: u16) -> usize {
    queue_size as usize * SLOT_LEN
}

/// Memory shared with the device: its CPU address, bus address and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaRegion {
    pub ptr: NonNull<u8>,
    pub bus: u64,
    pub len: usize,
}

/// Bus address = CPU address, for memory the device sees where the CPU
/// does (and for the tests).
#[derive(Clone, Copy, Debug, Default)]
pub struct IdentityDma;

impl Dma for IdentityDma {
    fn bus_address(&self, buf: IoBuf) -> Option<u64> {
        Some(buf.ptr.as_ptr() as u64)
    }
}

/// Driver-private bookkeeping for one queue entry. The caller provides one
/// per entry (`[ReqSlot::EMPTY; N]`) and otherwise leaves them alone.
#[derive(Clone, Copy, Debug)]
pub struct ReqSlot {
    // As a request slot (its index picks its place in the request area):
    id: RequestId,
    busy: bool,
    next_free: u16,
    // As the entry for descriptor `index`: the request slot whose chain
    // starts there.
    owner: u16,
}

impl ReqSlot {
    pub const EMPTY: ReqSlot = ReqSlot { id: RequestId(0), busy: false, next_free: NIL, owner: NIL };
}

/// Caller-provided tables, each at least one entry per queue entry.
#[derive(Debug)]
pub struct Storage<'a> {
    pub desc_state: &'a mut [DescState],
    pub slots: &'a mut [ReqSlot],
    /// Scratch space for building descriptor chains.
    pub buffers: &'a mut [Buffer],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitError {
    Virtio(poly_virtio::InitError),
    Config(ConfigError),
    Queue(SetupError),
    /// The queue's notification register lies outside the notification
    /// region.
    NoNotifier,
    /// Not enough memory or storage for a queue of 4 entries.
    NoRoom,
    /// The request area is not 8-byte aligned.
    Misaligned,
}

impl From<poly_virtio::InitError> for InitError {
    fn from(e: poly_virtio::InitError) -> InitError {
        InitError::Virtio(e)
    }
}

/// What [`init`] needs.
#[derive(Debug)]
pub struct Setup<'a, M, C, N, T> {
    /// The common configuration registers; the driver keeps them, to reset
    /// the device if it misbehaves.
    pub common: CommonCfg<M>,
    /// The device configuration region and its length.
    pub device_cfg: C,
    pub device_cfg_len: u32,
    pub transport: PciTransport,
    /// The notification region.
    pub notify: N,
    pub dma: T,
    /// Memory for the virtqueue (see [`Layout`]), 16-byte aligned.
    pub queue_mem: DmaRegion,
    /// Memory for headers and status bytes (see [`request_area_len`]),
    /// 8-byte aligned.
    pub area: DmaRegion,
    pub storage: Storage<'a>,
    /// MSI-X vector for the request queue, if interrupts are wanted that way.
    pub vector: Option<u16>,
}

/// Initialise a virtio block device (§3.1.1, §5.2.5) and return the running
/// driver. The request queue gets the largest size that the device, the
/// memory and the storage all allow, up to 32768 entries. On failure the
/// device is left marked FAILED (or, if it refused the reset, untouched).
///
/// # Safety
///
/// `queue_mem` and `area` must be valid for reads and writes for `'a`,
/// reachable by the device at their bus addresses, and used by nothing but
/// the driver and the device. `common`, `device_cfg`, `transport` and
/// `notify` must all belong to the same device.
pub unsafe fn init<'a, M: Mmio, C: Mmio, N: Mmio, T: Dma>(
    setup: Setup<'a, M, C, N, T>,
) -> Result<VirtioBlk<'a, M, N, T>, InitError> {
    let Setup { common, device_cfg, device_cfg_len, transport, notify, dma, queue_mem, area, storage, vector } = setup;
    if !(area.ptr.as_ptr() as usize).is_multiple_of(8) {
        return Err(InitError::Misaligned);
    }
    let negotiation = begin(&common, RESET_POLLS)?;
    let wanted = select_features(negotiation.offered());
    let cfg = negotiation.accept(wanted)?;
    macro_rules! fail {
        ($e:expr) => {{
            cfg.fail();
            return Err($e);
        }};
    }
    let config = match BlkConfig::read(&common, &device_cfg, device_cfg_len, cfg.features()) {
        Ok(c) => c,
        Err(e) => fail!(InitError::Config(e)),
    };
    let max = match cfg.queue_max_size(0) {
        Ok(m) => m,
        Err(e) => fail!(InitError::Virtio(e)),
    };
    let Storage { desc_state, slots, buffers } = storage;
    let room = (max as usize).min(desc_state.len()).min(slots.len()).min(buffers.len()).min(Layout::MAX_SIZE as usize);
    let size = (2..=15)
        .rev()
        .map(|k| 1u16 << k)
        .find(|&s| {
            s as usize <= room
                && Layout::new(s).is_some_and(|l| l.total() <= queue_mem.len)
                && request_area_len(s) <= area.len
        })
        .unwrap_or(0);
    if size == 0 {
        fail!(InitError::NoRoom);
    }
    let info = match config.info(size) {
        Ok(i) => i,
        Err(e) => fail!(InitError::Config(e)),
    };
    let layout = Layout::new(size).unwrap_or_else(|| unreachable!("a power of two in range"));
    // SAFETY: our caller vouches for the memory; the layout fits in it, and
    // the queue is not enabled yet.
    let queue =
        match unsafe { SplitQueue::new(queue_mem.ptr, queue_mem.bus, layout, desc_state, cfg.has(feature::EVENT_IDX)) }
        {
            Ok(q) => q,
            Err(e) => fail!(InitError::Queue(e)),
        };
    let notify_off = match cfg.enable_queue(0, size, queue.addresses(), vector) {
        Ok(n) => n,
        Err(e) => fail!(InitError::Virtio(e)),
    };
    let Some(notifier) = transport.notifier(0, notify_off) else { fail!(InitError::NoNotifier) };
    let features = cfg.driver_ok();
    let parts = Parts { common, queue, notifier, notify, dma, area, slots, buffers, info, features };
    // SAFETY: the device is live with this queue as its queue 0; the area
    // is our caller's to give and big enough for the queue size.
    Ok(unsafe { VirtioBlk::new(parts) })
}

/// Everything a [`VirtioBlk`] is made of. [`init`] assembles it.
#[derive(Debug)]
pub struct Parts<'a, M, N, T> {
    pub common: CommonCfg<M>,
    pub queue: SplitQueue<'a>,
    pub notifier: Notifier,
    pub notify: N,
    pub dma: T,
    pub area: DmaRegion,
    pub slots: &'a mut [ReqSlot],
    pub buffers: &'a mut [Buffer],
    pub info: DeviceInfo,
    pub features: u64,
}

/// A running virtio block device. See the [crate documentation](crate).
pub struct VirtioBlk<'a, M, N, T> {
    common: CommonCfg<M>,
    queue: SplitQueue<'a>,
    notifier: Notifier,
    notify: N,
    dma: T,
    area: DmaRegion,
    slots: &'a mut [ReqSlot],
    buffers: &'a mut [Buffer],
    free: u16,
    /// Requests submitted and not yet handed back.
    busy: u16,
    info: DeviceInfo,
    features: u64,
    /// Requests added since the last notification decision.
    unkicked: bool,
    /// The device broke the queue protocol.
    failed: bool,
    /// ... and has since been reset, so it no longer touches memory.
    reset_done: bool,
}

// SAFETY: the request area and the queue memory belong to the driver alone
// (the contract of `new`), so moving the driver moves their only CPU-side
// user with it; the rest is plain data or the caller's own types.
unsafe impl<M: Send, N: Send, T: Send> Send for VirtioBlk<'_, M, N, T> {}

impl<M, N, T> core::fmt::Debug for VirtioBlk<'_, M, N, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VirtioBlk")
            .field("info", &self.info)
            .field("features", &self.features)
            .field("queue", &self.queue)
            .field("failed", &self.failed)
            .finish()
    }
}

impl<'a, M: Mmio, N: Mmio, T: Dma> VirtioBlk<'a, M, N, T> {
    /// A driver for a device that is live (DRIVER_OK) with `queue` enabled
    /// as its queue 0.
    ///
    /// # Safety
    ///
    /// `area` must be at least [`request_area_len`] of the queue size long,
    /// 8-byte aligned, valid for reads and writes for `'a`, reachable by the
    /// device at `area.bus`, and used by nothing but this driver and the
    /// device. `slots` and `buffers` need an entry per queue entry. `info`
    /// must come from [`BlkConfig::info`] for this queue size, `features`
    /// from the negotiation, and the other parts from the same device.
    ///
    /// # Panics
    ///
    /// If `slots` or `buffers` are too short.
    pub unsafe fn new(parts: Parts<'a, M, N, T>) -> VirtioBlk<'a, M, N, T> {
        let Parts { common, queue, notifier, notify, dma, area, slots, buffers, info, features } = parts;
        let n = queue.size() as usize;
        assert!(slots.len() >= n && buffers.len() >= n, "storage smaller than the queue");
        debug_assert!(area.len >= request_area_len(queue.size()));
        for (i, s) in slots.iter_mut().enumerate() {
            *s = ReqSlot { next_free: if i + 1 < n { i as u16 + 1 } else { NIL }, ..ReqSlot::EMPTY };
        }
        VirtioBlk {
            common,
            queue,
            notifier,
            notify,
            dma,
            area,
            slots,
            buffers,
            free: 0,
            busy: 0,
            info,
            features,
            unkicked: false,
            failed: false,
            reset_done: false,
        }
    }

    /// The negotiated features.
    pub const fn features(&self) -> u64 {
        self.features
    }

    pub const fn queue_size(&self) -> u16 {
        self.queue.size()
    }

    /// Requests submitted and not yet handed back by [`BlockDevice::poll`].
    pub const fn in_flight(&self) -> u16 {
        self.busy
    }

    /// The device broke the protocol; every request fails from now on.
    pub const fn is_failed(&self) -> bool {
        self.failed
    }

    /// Ask the device not to interrupt for completions (a hint).
    pub fn disable_interrupts(&mut self) {
        self.queue.disable_interrupts();
    }

    /// Ask for interrupts again. True if completions arrived meanwhile:
    /// poll before waiting for an interrupt.
    pub fn enable_interrupts(&mut self) -> bool {
        self.queue.enable_interrupts()
    }

    /// The device's ID string (GET_ID, §5.2.6), waiting for the answer. Only
    /// while nothing else is in flight: [`IoError::Busy`] otherwise.
    pub fn device_id(&mut self) -> Result<[u8; ID_LEN], IoError> {
        if self.in_flight() != 0 {
            return Err(IoError::Busy);
        }
        let id = RequestId(u64::MAX);
        let slot = self.start(id, req_type::GET_ID, 0, &[], false, true).map_err(|e| match e {
            SubmitError::Busy => IoError::Busy,
            SubmitError::Rejected(e) => e,
        })?;
        self.kick();
        loop {
            match self.poll() {
                Some(c) if c.id == id => {
                    c.result?;
                    let mut out = [0; ID_LEN];
                    for (i, b) in out.iter_mut().enumerate() {
                        *b = self.area_read8(slot, ID_AT + i);
                    }
                    return Ok(out);
                }
                Some(_) => {}
                None => core::hint::spin_loop(),
            }
        }
    }

    /// Put a request on the queue: the header and status in request slot
    /// `slot`'s part of the area, then `segments` (device-writable if
    /// `writable`), then for GET_ID (`id_buf`) the slot's ID room. Returns
    /// the request slot.
    fn start(
        &mut self,
        id: RequestId,
        ty: u32,
        sector: u64,
        segments: &[IoBuf],
        writable: bool,
        id_buf: bool,
    ) -> Result<u16, SubmitError> {
        if self.failed {
            return Err(SubmitError::Rejected(IoError::DeviceFailed));
        }
        let slot = self.free;
        if slot == NIL {
            return Err(SubmitError::Busy);
        }
        let base = self.area.bus + (slot as usize * SLOT_LEN) as u64;
        let mut n = 0;
        self.buffers[n] = Buffer { addr: base, len: HEADER_LEN as u32 };
        n += 1;
        for seg in segments {
            let Some(addr) = self.dma.bus_address(*seg) else {
                return Err(SubmitError::Rejected(IoError::BadBuffer));
            };
            // Segment lengths were checked against `max_segment_len`, a u32.
            self.buffers[n] = Buffer { addr, len: seg.len as u32 };
            n += 1;
        }
        if id_buf {
            self.buffers[n] = Buffer { addr: base + ID_AT as u64, len: ID_LEN as u32 };
            n += 1;
        }
        self.buffers[n] = Buffer { addr: base + STATUS_AT as u64, len: 1 };
        n += 1;
        let readable = if writable || id_buf { 1 } else { n - 1 };
        self.area_write_header(slot, ty, sector);
        self.area_write8(slot, STATUS_AT, NO_STATUS);
        let (r, w) = self.buffers[..n].split_at(readable);
        let head = match self.queue.add(r, w) {
            Ok(head) => head,
            Err(AddError::Full) => return Err(SubmitError::Busy),
            Err(AddError::TooLong | AddError::NoBuffers) => return Err(SubmitError::Rejected(IoError::BadRequest)),
        };
        let s = &mut self.slots[slot as usize];
        self.free = s.next_free;
        *s = ReqSlot { id, busy: true, next_free: NIL, owner: s.owner };
        self.slots[head as usize].owner = slot;
        self.busy += 1;
        self.unkicked = true;
        Ok(slot)
    }

    /// Hand a request slot back to the free list.
    fn release(&mut self, slot: u16) -> RequestId {
        let s = &mut self.slots[slot as usize];
        s.busy = false;
        s.next_free = self.free;
        self.free = slot;
        self.busy -= 1;
        s.id
    }

    /// The device broke the protocol: reset it, so it stops using our
    /// memory, and mark it FAILED.
    fn fail(&mut self) {
        self.failed = true;
        if self.common.reset(RESET_POLLS).is_ok() {
            self.reset_done = true;
            self.common.set_status(status::FAILED);
        }
    }

    // The request area. Offsets stay inside slot `slot`'s SLOT_LEN bytes,
    // and the slot is below the queue size, so every access is inside the
    // area (`new`'s contract); fields are naturally aligned because the area
    // is 8-byte aligned and SLOT_LEN a multiple of 8.

    fn area_at(&self, slot: u16, off: usize) -> *mut u8 {
        debug_assert!((slot as usize) < self.queue.size() as usize && off < SLOT_LEN);
        // SAFETY: inside the area (see above), which is one allocation.
        unsafe { self.area.ptr.as_ptr().add(slot as usize * SLOT_LEN + off) }
    }

    fn area_write_header(&self, slot: u16, ty: u32, sector: u64) {
        let p = self.area_at(slot, 0);
        // SAFETY: inside the area and aligned (see above). Volatile: the
        // device reads it.
        unsafe {
            ptr::write_volatile(p.cast::<u32>(), ty.to_le());
            ptr::write_volatile(p.add(4).cast::<u32>(), 0);
            ptr::write_volatile(p.add(8).cast::<u64>(), sector.to_le());
        }
    }

    fn area_write8(&self, slot: u16, off: usize, v: u8) {
        // SAFETY: as for `area_write_header`.
        unsafe { ptr::write_volatile(self.area_at(slot, off), v) }
    }

    fn area_read8(&self, slot: u16, off: usize) -> u8 {
        // SAFETY: as for `area_write_header`; the device writes it.
        unsafe { ptr::read_volatile(self.area_at(slot, off)) }
    }
}

impl<M: Mmio, N: Mmio, T: Dma> BlockDevice for VirtioBlk<'_, M, N, T> {
    fn info(&self) -> DeviceInfo {
        self.info
    }

    unsafe fn submit(&mut self, req: &Request<'_>) -> Result<(), SubmitError> {
        if self.failed {
            return Err(SubmitError::Rejected(IoError::DeviceFailed));
        }
        req.check(&self.info).map_err(SubmitError::Rejected)?;
        let sector = req.lba * (self.info.block_size / SECTOR_SIZE) as u64;
        let (ty, writable) = match req.op {
            Op::Read => (req_type::IN, true),
            Op::Write => (req_type::OUT, false),
            Op::Flush => (req_type::FLUSH, false),
        };
        self.start(req.id, ty, sector, req.segments, writable, false).map(|_| ())
    }

    fn kick(&mut self) {
        if self.unkicked {
            self.unkicked = false;
            if self.queue.should_notify() {
                self.notifier.notify(&self.notify);
            }
        }
    }

    fn poll(&mut self) -> Option<Completion> {
        if !self.failed {
            match self.queue.pop_used() {
                Ok(Some(used)) => {
                    let slot = self.slots[used.head as usize].owner;
                    let result = match self.area_read8(slot, STATUS_AT) {
                        req_status::OK => Ok(()),
                        req_status::UNSUPP => Err(IoError::Unsupported),
                        // IOERR, and anything the specification does not
                        // define (including no answer at all).
                        _ => Err(IoError::Media),
                    };
                    let id = self.release(slot);
                    return Some(Completion { id, result });
                }
                Ok(None) => return None,
                Err(_) => self.fail(),
            }
        }
        // Requests in flight on a failed device come back only once it has
        // been reset; until then it may still write their memory.
        if !self.reset_done {
            self.fail();
            if !self.reset_done {
                return None;
            }
        }
        let slot = (0..self.queue.size()).find(|&s| self.slots[s as usize].busy)?;
        let id = self.release(slot);
        Some(Completion { id, result: Err(IoError::DeviceFailed) })
    }
}
