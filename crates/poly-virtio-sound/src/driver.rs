//! The driver: device initialisation, control requests, the event queue
//! and output messages on the transmit queue.

use crate::proto::{
    CONFIG_LEN, ChmapInfo, DeviceConfig, Event, HDR_LEN, JackInfo, PcmInfo, PcmStatus, SetParams, XFER_LEN, code,
    id_request, query_info, response_status, status as snd_status,
};
use crate::ring::Chunk;
use crate::stream::{Command, StreamState};
use crate::{CONTROL_QUEUE, EVENT_QUEUE, RX_QUEUE, TX_QUEUE};
use core::ptr::{self, NonNull};
use poly_pci::Mmio;
use poly_virtio::{
    Buffer, CommonCfg, Configuring, DescState, Layout, Notifier, PciTransport, SetupError, SplitQueue, begin, status,
};

/// Polls of the status register while waiting for a reset to finish.
const RESET_POLLS: u32 = 1000;
/// Tries at a consistent configuration read before giving up.
const CONFIG_TRIES: u32 = 16;

/// Bytes at the start of the control buffer for the request; the response
/// follows.
pub const CONTROL_REQUEST_MAX: usize = 64;
/// The smallest control buffer: a request and a response with room for a
/// few query answers.
pub const CONTROL_MIN: usize = 256;
/// Bytes per transmit slot in the message memory: the `virtio_snd_pcm_xfer`
/// header at 0, the `virtio_snd_pcm_status` at 8.
pub const TX_SLOT_LEN: usize = 16;
/// Descriptors per output message: header, data, status.
pub const TX_DESCRIPTORS: u16 = 3;
/// Bytes per receive slot in the message memory: the header at 0, the
/// status at 8 (as for [`TX_SLOT_LEN`]).
pub const RX_SLOT_LEN: usize = 16;
/// Descriptors per input message: header (device-readable), data and
/// status (device-writable).
pub const RX_DESCRIPTORS: u16 = 3;

/// Features this driver uses when offered. The sound device defines none
/// of its own in VIRTIO 1.2 (§5.14.3). VIRTIO_F_ACCESS_PLATFORM is accepted
/// because a device that offers it may refuse to work without it (§6.1).
pub const DRIVER_FEATURES: u64 = poly_virtio::feature::EVENT_IDX | poly_virtio::feature::ACCESS_PLATFORM;

/// Memory shared with the device: its CPU address, bus address and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaRegion {
    pub ptr: NonNull<u8>,
    pub bus: u64,
    pub len: usize,
}

/// All memory the device sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    /// The rings of the control, event, transmit and receive queues (see
    /// [`Layout`]; 16-byte aligned).
    pub rings: [DmaRegion; 4],
    /// Requests and responses, one at a time (at least [`CONTROL_MIN`]).
    pub control: DmaRegion,
    /// Event buffers, [`Event::LEN`] bytes each.
    pub events: DmaRegion,
    /// Headers and status of output messages, [`TX_SLOT_LEN`] per slot.
    pub tx_meta: DmaRegion,
    /// Headers and status of input messages, [`RX_SLOT_LEN`] per slot.
    pub rx_meta: DmaRegion,
    /// The PCM memory output and input messages point into (the caller
    /// keeps the two directions' ring buffers apart).
    pub pcm: DmaRegion,
}

/// Caller-provided tables.
#[derive(Debug)]
pub struct Storage<'a> {
    /// Descriptor state of the control, event, transmit and receive
    /// queues; a queue gets no more entries than its table has.
    pub desc: [&'a mut [DescState]; 4],
    /// For each event descriptor: the event buffer behind it.
    pub event_owner: &'a mut [u16],
    /// For each transmit descriptor: the slot whose message starts there.
    pub tx_owner: &'a mut [u16],
    /// Free transmit slots (a stack), and each slot's data length.
    pub tx_free: &'a mut [u16],
    pub tx_len: &'a mut [u32],
    /// The same for input messages on the receive queue.
    pub rx_owner: &'a mut [u16],
    pub rx_free: &'a mut [u16],
    pub rx_len: &'a mut [u32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitError {
    Virtio(poly_virtio::InitError),
    /// The device configuration is shorter than `struct virtio_snd_config`
    /// or kept changing while read.
    Config,
    Queue(SetupError),
    /// A queue's notification register lies outside the notification
    /// region.
    NoNotifier,
    /// Not enough memory or table space for the queues.
    NoRoom,
}

impl From<poly_virtio::InitError> for InitError {
    fn from(e: poly_virtio::InitError) -> InitError {
        InitError::Virtio(e)
    }
}

/// What [`init`] needs.
#[derive(Debug)]
pub struct Setup<'a, M, C, N> {
    /// The common configuration registers (kept, to reset the device if it
    /// misbehaves).
    pub common: CommonCfg<M>,
    /// The device configuration region and its length (read once).
    pub device_cfg: C,
    pub device_cfg_len: u32,
    pub transport: PciTransport,
    /// The notification region.
    pub notify: N,
    pub memory: Memory,
    pub storage: Storage<'a>,
    /// MSI-X table entry for the event and transmit queues and
    /// configuration changes (`None`: no interrupt). The control queue is
    /// polled and never interrupts.
    pub vector: Option<u16>,
}

/// Errors of control requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtlError {
    /// The device broke the protocol earlier and was reset.
    Failed,
    /// The device did not answer in time; it was reset, since it might
    /// still write the response later.
    Timeout,
    /// The request or response does not fit the control buffer.
    TooBig,
    /// The response was shorter than it has to be.
    Short,
    /// The device answered with this status (`VIRTIO_SND_S_*`, not OK).
    Status(u32),
    /// The command is not allowed in the stream's state.
    State,
}

/// Errors handing an output message to the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxError {
    Failed,
    /// Every transmit slot is in flight; retry once messages came back.
    Busy,
    /// The chunk is empty or not inside the PCM memory.
    Range,
}

/// An output message the device returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxDone {
    /// The stream the message belonged to (from its header).
    pub stream: u32,
    /// Data bytes the message carried.
    pub len: u32,
    /// The device's status; `IO_ERR` if it wrote less than a whole status.
    pub status: PcmStatus,
}

/// An input message the device filled and returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RxDone {
    pub stream: u32,
    /// Where its data is in the PCM memory, and how many bytes the device
    /// wrote there (see [`VirtioSound::poll_rx`]).
    pub offset: u32,
    pub len: u32,
    /// The device's status.
    pub status: PcmStatus,
}

/// Counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub control_requests: u64,
    pub control_errors: u64,
    /// Responses whose used length was shorter than the response (taken:
    /// see `check_status`).
    pub short_responses: u64,
    pub tx_messages: u64,
    pub tx_bytes: u64,
    /// Messages that came back with a status other than OK.
    pub tx_errors: u64,
    pub tx_kicks: u64,
    pub events: u64,
    pub period_elapsed: u64,
    pub xruns: u64,
    pub jack_events: u64,
    pub rx_messages: u64,
    pub rx_bytes: u64,
    /// Input messages that came back with a status other than OK.
    pub rx_errors: u64,
    /// Input messages whose used length did not cover the status (taken
    /// as full: see `poll_rx`).
    pub rx_short: u64,
    pub rx_kicks: u64,
}

/// The largest power-of-two queue size, at most `max` (the device's limit)
/// and `room` (table entries), whose ring fits `ring`.
fn queue_size(max: u16, room: usize, ring: &DmaRegion) -> u16 {
    let room = (max as usize).min(room).min(Layout::MAX_SIZE as usize);
    (1..=15)
        .rev()
        .map(|k| 1u16 << k)
        .find(|&s| s as usize <= room && Layout::new(s).is_some_and(|l| l.total() <= ring.len))
        .unwrap_or(0)
}

/// Set up queue `index` of `size` entries in `ring`.
///
/// # Safety
///
/// As for [`init`]: the ring memory is the caller's to give to the device.
unsafe fn setup_queue<'a, M: Mmio>(
    cfg: &Configuring<'_, M>,
    transport: &PciTransport,
    index: u16,
    size: u16,
    ring: &DmaRegion,
    desc: &'a mut [DescState],
    vector: Option<u16>,
) -> Result<(SplitQueue<'a>, Notifier), InitError> {
    let layout = Layout::new(size).ok_or(InitError::NoRoom)?;
    let event_idx = cfg.has(poly_virtio::feature::EVENT_IDX);
    // SAFETY: our caller vouches for the memory; the layout fits in it (the
    // size was chosen so), and the queue is not enabled yet.
    let queue = unsafe { SplitQueue::new(ring.ptr, ring.bus, layout, desc, event_idx) }.map_err(InitError::Queue)?;
    let notify_off = cfg.enable_queue(index, size, queue.addresses(), vector)?;
    let notifier = transport.notifier(index, notify_off).ok_or(InitError::NoNotifier)?;
    Ok((queue, notifier))
}

/// Read `struct virtio_snd_config` (§5.14.4) consistently.
fn read_config<M: Mmio, C: Mmio>(common: &CommonCfg<M>, cfg: &C, len: u32) -> Result<DeviceConfig, InitError> {
    if (len as usize) < CONFIG_LEN {
        return Err(InitError::Config);
    }
    common
        .read_config(CONFIG_TRIES, || DeviceConfig {
            jacks: cfg.read32(0),
            streams: cfg.read32(4),
            chmaps: cfg.read32(8),
        })
        .ok_or(InitError::Config)
}

/// Initialise a virtio sound device (§3.1.1, §5.14.5): features
/// negotiated, the configuration read, the four queues set up (each as
/// large as the device, the memory and the tables allow), every event
/// buffer handed to the device, DRIVER_OK set. The jacks, streams and
/// channel maps are then queried with [`VirtioSound::query_jacks`] and
/// friends. On failure the device is left marked FAILED (or, if it refused
/// the reset, untouched).
///
/// # Safety
///
/// Every region of `memory` must be valid for reads and writes for `'a`,
/// reachable by the device at its bus address, and used by nothing but
/// the driver and the device. `common`, `device_cfg`, `transport` and
/// `notify` must all belong to the same device.
pub unsafe fn init<'a, M: Mmio, C: Mmio, N: Mmio>(
    setup: Setup<'a, M, C, N>,
) -> Result<VirtioSound<'a, M, N>, InitError> {
    let Setup { common, device_cfg, device_cfg_len, transport, notify, memory, storage, vector } = setup;
    let negotiation = begin(&common, RESET_POLLS)?;
    let offered = negotiation.offered();
    let cfg = negotiation.accept(offered & DRIVER_FEATURES)?;
    macro_rules! fail {
        ($e:expr) => {{
            cfg.fail();
            return Err($e);
        }};
    }
    let config = match read_config(&common, &device_cfg, device_cfg_len) {
        Ok(c) => c,
        Err(e) => fail!(e),
    };
    if let Some(v) = vector
        && let Err(e) = cfg.set_config_vector(v)
    {
        fail!(InitError::Virtio(e));
    }
    if memory.control.len < CONTROL_MIN {
        fail!(InitError::NoRoom);
    }
    let Storage { desc, event_owner, tx_owner, tx_free, tx_len, rx_owner, rx_free, rx_len } = storage;
    let [control_desc, event_desc, tx_desc, rx_desc] = desc;
    let mut max = [0u16; 4];
    for (q, m) in max.iter_mut().enumerate() {
        match cfg.queue_max_size(q as u16) {
            Ok(v) => *m = v,
            Err(e) => fail!(InitError::Virtio(e)),
        }
    }
    let event_room = event_desc.len().min(event_owner.len()).min(memory.events.len / Event::LEN);
    let tx_room = tx_desc.len().min(tx_owner.len());
    let rx_room = rx_desc.len().min(rx_owner.len());
    let sizes = [
        queue_size(max[0], control_desc.len(), &memory.rings[0]),
        queue_size(max[1], event_room, &memory.rings[1]),
        queue_size(max[2], tx_room, &memory.rings[2]),
        queue_size(max[3], rx_room, &memory.rings[3]),
    ];
    let tx_slots = (sizes[2] / TX_DESCRIPTORS)
        .min(tx_free.len().min(tx_len.len()).min(memory.tx_meta.len / TX_SLOT_LEN).min(u16::MAX as usize) as u16);
    let rx_slots = (sizes[3] / RX_DESCRIPTORS)
        .min(rx_free.len().min(rx_len.len()).min(memory.rx_meta.len / RX_SLOT_LEN).min(u16::MAX as usize) as u16);
    if sizes[0] < 2 || sizes[1] < 1 || tx_slots < 2 || sizes[3] < 1 {
        fail!(InitError::NoRoom);
    }
    // SAFETY: forwarded from our caller.
    let r = unsafe { setup_queue(&cfg, &transport, CONTROL_QUEUE, sizes[0], &memory.rings[0], control_desc, None) };
    let (control, control_n) = match r {
        Ok(q) => q,
        Err(e) => fail!(e),
    };
    // SAFETY: forwarded from our caller.
    let r = unsafe { setup_queue(&cfg, &transport, EVENT_QUEUE, sizes[1], &memory.rings[1], event_desc, vector) };
    let (mut event, event_n) = match r {
        Ok(q) => q,
        Err(e) => fail!(e),
    };
    // SAFETY: forwarded from our caller.
    let r = unsafe { setup_queue(&cfg, &transport, TX_QUEUE, sizes[2], &memory.rings[2], tx_desc, vector) };
    let (tx, tx_n) = match r {
        Ok(q) => q,
        Err(e) => fail!(e),
    };
    // SAFETY: forwarded from our caller.
    let r = unsafe { setup_queue(&cfg, &transport, RX_QUEUE, sizes[3], &memory.rings[3], rx_desc, vector) };
    let (rx, rx_n) = match r {
        Ok(q) => q,
        Err(e) => fail!(e),
    };
    // §5.14.5 step 4: the event queue is populated before the device runs;
    // the notification waits for DRIVER_OK (§3.1.1).
    let event_slots = sizes[1];
    for slot in 0..event_slots {
        match event.add(&[], &[event_buffer(&memory.events, slot)]) {
            Ok(head) => event_owner[head as usize] = slot,
            Err(_) => fail!(InitError::NoRoom),
        }
    }
    for (i, f) in tx_free.iter_mut().take(tx_slots as usize).enumerate() {
        *f = i as u16;
    }
    for (i, f) in rx_free.iter_mut().take(rx_slots as usize).enumerate() {
        *f = i as u16;
    }
    let features = cfg.driver_ok();
    let mut snd = VirtioSound {
        common,
        notify,
        control,
        event,
        tx,
        rx,
        control_n,
        event_n,
        tx_n,
        rx_n,
        memory,
        event_owner,
        tx_owner,
        tx_free,
        tx_len,
        tx_free_len: tx_slots,
        tx_slots,
        rx_owner,
        rx_free,
        rx_len,
        rx_free_len: rx_slots,
        rx_slots,
        config,
        features,
        stats: Stats::default(),
        failed: false,
        reset_done: false,
    };
    snd.kick_events();
    // The control queue is polled: no interrupts for it.
    snd.control.disable_interrupts();
    Ok(snd)
}

/// Event buffer `slot`, device-writable.
fn event_buffer(mem: &DmaRegion, slot: u16) -> Buffer {
    Buffer { addr: mem.bus + (slot as usize * Event::LEN) as u64, len: Event::LEN as u32 }
}

/// A running virtio sound device. See the [crate documentation](crate).
pub struct VirtioSound<'a, M, N> {
    common: CommonCfg<M>,
    notify: N,
    control: SplitQueue<'a>,
    event: SplitQueue<'a>,
    tx: SplitQueue<'a>,
    rx: SplitQueue<'a>,
    control_n: Notifier,
    event_n: Notifier,
    tx_n: Notifier,
    rx_n: Notifier,
    memory: Memory,
    event_owner: &'a mut [u16],
    tx_owner: &'a mut [u16],
    tx_free: &'a mut [u16],
    tx_len: &'a mut [u32],
    tx_free_len: u16,
    tx_slots: u16,
    rx_owner: &'a mut [u16],
    rx_free: &'a mut [u16],
    /// Per receive slot, the length of its data buffer. Its offset in the
    /// PCM memory is kept in the slot's message memory at byte 4, after
    /// the 4-byte header (bytes the device is never shown).
    rx_len: &'a mut [u32],
    rx_free_len: u16,
    rx_slots: u16,
    config: DeviceConfig,
    features: u64,
    stats: Stats,
    /// The device broke the protocol (or a request timed out).
    failed: bool,
    /// ... and has since been reset, so it no longer touches memory.
    reset_done: bool,
}

// SAFETY: the queue and buffer memory belong to the driver alone (the
// contract of `init`), so moving the driver moves their only CPU-side user
// with it; the rest is plain data or the caller's own types.
unsafe impl<M: Send, N: Send> Send for VirtioSound<'_, M, N> {}

impl<M, N> core::fmt::Debug for VirtioSound<'_, M, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VirtioSound")
            .field("config", &self.config)
            .field("features", &self.features)
            .field("tx_slots", &self.tx_slots)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl<M: Mmio, N: Mmio> VirtioSound<'_, M, N> {
    /// How many jacks, streams and channel maps the device has.
    pub const fn config(&self) -> DeviceConfig {
        self.config
    }

    pub const fn features(&self) -> u64 {
        self.features
    }

    pub const fn stats(&self) -> Stats {
        self.stats
    }

    pub const fn is_failed(&self) -> bool {
        self.failed
    }

    /// Output messages the device can hold at once.
    pub const fn tx_slots(&self) -> u16 {
        self.tx_slots
    }

    /// Output messages handed to the device and not back yet.
    pub const fn tx_in_flight(&self) -> u16 {
        self.tx_slots - self.tx_free_len
    }

    pub const fn queue_sizes(&self) -> [u16; 4] {
        [self.control.size(), self.event.size(), self.tx.size(), self.rx.size()]
    }

    /// Bytes of PCM memory.
    pub const fn pcm_len(&self) -> usize {
        self.memory.pcm.len
    }

    /// The device broke the protocol: reset it, so it stops using our
    /// memory, and mark it FAILED.
    fn fail(&mut self) {
        self.failed = true;
        if !self.reset_done && self.common.reset(RESET_POLLS).is_ok() {
            self.reset_done = true;
            self.common.set_status(status::FAILED);
        }
    }

    // ---- control requests --------------------------------------------------

    /// Send `req` and wait for a response of up to `resp_len` bytes,
    /// calling `wait` between looks at the used ring (false: give up).
    /// Returns the length the device wrote; the response is then in
    /// [`response`](Self::response).
    fn exchange(&mut self, req: &[u8], resp_len: usize, wait: &mut dyn FnMut() -> bool) -> Result<u32, CtlError> {
        if self.failed {
            return Err(CtlError::Failed);
        }
        if req.len() > CONTROL_REQUEST_MAX || resp_len < HDR_LEN || resp_len > self.response_room() {
            return Err(CtlError::TooBig);
        }
        let base = self.memory.control;
        // SAFETY: both pieces are inside the control buffer (checked
        // above), which no request is using (one at a time, and a request
        // that timed out reset the device): the device does not touch it
        // until the buffer is added below, after the queue's Release fence.
        unsafe {
            ptr::copy_nonoverlapping(req.as_ptr(), base.ptr.as_ptr(), req.len());
            ptr::write_bytes(base.ptr.as_ptr().add(CONTROL_REQUEST_MAX), 0, resp_len);
        }
        let r = Buffer { addr: base.bus, len: req.len() as u32 };
        let w = Buffer { addr: base.bus + CONTROL_REQUEST_MAX as u64, len: resp_len as u32 };
        if self.control.add(&[r], &[w]).is_err() {
            // One request at a time on a queue of at least 2 entries.
            self.fail();
            return Err(CtlError::Failed);
        }
        self.stats.control_requests += 1;
        if self.control.should_notify() {
            self.control_n.notify(&self.notify);
        }
        loop {
            match self.control.pop_used() {
                Ok(Some(u)) => return Ok(u.len),
                Ok(None) => {
                    if !wait() {
                        self.stats.control_errors += 1;
                        self.fail();
                        return Err(CtlError::Timeout);
                    }
                }
                Err(_) => {
                    self.fail();
                    return Err(CtlError::Failed);
                }
            }
        }
    }

    fn response_room(&self) -> usize {
        self.memory.control.len - CONTROL_REQUEST_MAX
    }

    /// The first `len` bytes of the last response (`len` at most what was
    /// asked for).
    fn response(&self, len: usize) -> &[u8] {
        let len = len.min(self.response_room());
        // SAFETY: inside the control buffer; the device handed the
        // response back (it came out of the used ring after an Acquire
        // fence) and no request is in flight, so nothing writes there.
        unsafe { core::slice::from_raw_parts(self.memory.control.ptr.as_ptr().add(CONTROL_REQUEST_MAX), len) }
    }

    /// A request answered by a bare status.
    fn simple(&mut self, req: &[u8], wait: &mut dyn FnMut() -> bool) -> Result<(), CtlError> {
        let len = self.exchange(req, HDR_LEN, wait)?;
        let status = response_status(self.response(HDR_LEN));
        self.check_status(status, len as usize, HDR_LEN)
    }

    /// Judge a response: its status decides. The response area was zeroed
    /// before the request, so a status of 0 was never written. Some devices
    /// (QEMU 8.2's) report a used length shorter than what they wrote;
    /// what was not written reads as zeros, so a short length with an OK
    /// status is taken (and counted).
    fn check_status(&mut self, status: Option<u32>, got: usize, want: usize) -> Result<(), CtlError> {
        match status {
            Some(snd_status::OK) => {
                if got < want {
                    self.stats.short_responses += 1;
                }
                Ok(())
            }
            Some(0) | None => {
                self.stats.control_errors += 1;
                Err(CtlError::Short)
            }
            Some(s) => {
                self.stats.control_errors += 1;
                Err(CtlError::Status(s))
            }
        }
    }

    /// An item query (§5.14.6.1): `count` items of `item_len` bytes from
    /// `start`, in as many requests as the control buffer needs; `each`
    /// gets every item's index (from 0) and bytes.
    fn query(
        &mut self,
        code: u32,
        item_len: usize,
        start: u32,
        count: usize,
        wait: &mut dyn FnMut() -> bool,
        mut each: impl FnMut(usize, &[u8]),
    ) -> Result<(), CtlError> {
        let per = ((self.response_room() - HDR_LEN) / item_len).max(1);
        let mut done = 0;
        while done < count {
            let n = (count - done).min(per);
            let req = query_info(code, start + done as u32, n as u32, item_len as u32);
            let resp_len = HDR_LEN + n * item_len;
            let got = self.exchange(&req, resp_len, wait)?;
            let status = response_status(self.response(resp_len));
            self.check_status(status, got as usize, resp_len)?;
            let resp = self.response(resp_len);
            for i in 0..n {
                let at = HDR_LEN + i * item_len;
                each(done + i, &resp[at..at + item_len]);
            }
            done += n;
        }
        Ok(())
    }

    /// The jacks (§5.14.6.4.1), as many as fit `out` and the device has;
    /// returns how many.
    pub fn query_jacks(&mut self, out: &mut [JackInfo], wait: &mut dyn FnMut() -> bool) -> Result<usize, CtlError> {
        let n = out.len().min(self.config.jacks as usize);
        self.query(code::JACK_INFO, JackInfo::LEN, 0, n, wait, |i, b| out[i] = JackInfo::decode(b))?;
        Ok(n)
    }

    /// The PCM streams (§5.14.6.6.2).
    pub fn query_streams(&mut self, out: &mut [PcmInfo], wait: &mut dyn FnMut() -> bool) -> Result<usize, CtlError> {
        let n = out.len().min(self.config.streams as usize);
        self.query(code::PCM_INFO, PcmInfo::LEN, 0, n, wait, |i, b| out[i] = PcmInfo::decode(b))?;
        Ok(n)
    }

    /// The channel maps (§5.14.6.9.1).
    pub fn query_chmaps(&mut self, out: &mut [ChmapInfo], wait: &mut dyn FnMut() -> bool) -> Result<usize, CtlError> {
        let n = out.len().min(self.config.chmaps as usize);
        self.query(code::CHMAP_INFO, ChmapInfo::LEN, 0, n, wait, |i, b| out[i] = ChmapInfo::decode(b))?;
        Ok(n)
    }

    /// Bring stream `id` from `state` to `goal` (§5.14.6.6.1), sending
    /// the commands in between ([`StreamState::path_to`]); `params` is
    /// what SET PARAMETERS sends when that is the goal. `state` follows
    /// every command the device accepted.
    pub fn pcm_goto(
        &mut self,
        state: &mut StreamState,
        id: u32,
        goal: Command,
        params: Option<&SetParams>,
        wait: &mut dyn FnMut() -> bool,
    ) -> Result<(), CtlError> {
        let path = state.path_to(goal);
        if path.is_empty() {
            return Err(CtlError::State);
        }
        for &cmd in path.as_slice() {
            match cmd {
                Command::SetParams => {
                    let p = params.ok_or(CtlError::State)?;
                    self.simple(&p.request(id), wait)?;
                }
                _ => self.simple(&id_request(cmd.code(), id), wait)?,
            }
            *state = state.after(cmd);
        }
        Ok(())
    }

    // ---- output messages ---------------------------------------------------

    /// Hand `chunk` of the PCM memory to the device as an output message
    /// of stream `id` (§5.14.6.8): header, data, status, in three
    /// descriptors. The caller must not change those bytes until the
    /// message comes back from [`poll_tx`](Self::poll_tx).
    pub fn submit(&mut self, id: u32, chunk: Chunk) -> Result<(), TxError> {
        if self.failed {
            return Err(TxError::Failed);
        }
        let (off, len) = (chunk.offset as usize, chunk.len as usize);
        if len == 0 || off.checked_add(len).is_none_or(|end| end > self.memory.pcm.len) {
            return Err(TxError::Range);
        }
        if self.tx_free_len == 0 {
            return Err(TxError::Busy);
        }
        self.tx_free_len -= 1;
        let slot = self.tx_free[self.tx_free_len as usize];
        let meta = slot as usize * TX_SLOT_LEN;
        // SAFETY: the slot is inside the message memory (`init` made no
        // more slots than fit) and free (the device returned it or never
        // had it), so nothing else accesses it; the queue's Release fence
        // orders these writes before the device can see the message.
        unsafe {
            let p = self.memory.tx_meta.ptr.as_ptr().add(meta);
            ptr::copy_nonoverlapping(id.to_le_bytes().as_ptr(), p, XFER_LEN);
            ptr::write_bytes(p.add(8), 0, PcmStatus::LEN);
        }
        let bus = self.memory.tx_meta.bus + meta as u64;
        let pieces = [
            Buffer { addr: bus, len: XFER_LEN as u32 },
            Buffer { addr: self.memory.pcm.bus + off as u64, len: len as u32 },
        ];
        let status = Buffer { addr: bus + 8, len: PcmStatus::LEN as u32 };
        match self.tx.add(&pieces, &[status]) {
            Ok(head) => {
                self.tx_owner[head as usize] = slot;
                self.tx_len[slot as usize] = len as u32;
            }
            Err(_) => {
                // A free slot means three free descriptors (`init` made no
                // more slots than that); keep going if that ever fails.
                self.tx_free[self.tx_free_len as usize] = slot;
                self.tx_free_len += 1;
                return Err(TxError::Busy);
            }
        }
        self.stats.tx_messages += 1;
        self.stats.tx_bytes += len as u64;
        if self.tx.should_notify() {
            self.tx_n.notify(&self.notify);
            self.stats.tx_kicks += 1;
        }
        Ok(())
    }

    /// Take back every output message the device returned, oldest first.
    pub fn poll_tx(&mut self, mut done: impl FnMut(TxDone)) {
        while !self.failed {
            match self.tx.pop_used() {
                Ok(Some(u)) => {
                    let slot = self.tx_owner[u.head as usize];
                    let meta = slot as usize * TX_SLOT_LEN;
                    let mut b = [0u8; TX_SLOT_LEN];
                    // SAFETY: inside the message memory; the device handed
                    // the message back (Acquire fence in `pop_used`).
                    unsafe {
                        ptr::copy_nonoverlapping(self.memory.tx_meta.ptr.as_ptr().add(meta), b.as_mut_ptr(), b.len())
                    };
                    let stream = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                    let status = if (u.len as usize) < PcmStatus::LEN {
                        PcmStatus { status: snd_status::IO_ERR, latency_bytes: 0 }
                    } else {
                        PcmStatus::decode(&b[8..])
                    };
                    if status.status != snd_status::OK {
                        self.stats.tx_errors += 1;
                    }
                    self.tx_free[self.tx_free_len as usize] = slot;
                    self.tx_free_len += 1;
                    done(TxDone { stream, len: self.tx_len[slot as usize], status });
                }
                Ok(None) => return,
                Err(_) => self.fail(),
            }
        }
    }

    // ---- input messages ----------------------------------------------------

    /// Input messages the device can hold at once.
    pub const fn rx_slots(&self) -> u16 {
        self.rx_slots
    }

    /// Input messages handed to the device and not back yet.
    pub const fn rx_in_flight(&self) -> u16 {
        self.rx_slots - self.rx_free_len
    }

    /// Lend `chunk` of the PCM memory to the device as an input message of
    /// stream `id` (§5.14.6.8): the header (device-readable), then the
    /// data buffer and the status (device-writable). The device returns
    /// it once it filled the buffer with captured frames
    /// ([`poll_rx`](Self::poll_rx)); until then the caller must not touch
    /// those bytes.
    pub fn submit_rx(&mut self, id: u32, chunk: Chunk) -> Result<(), TxError> {
        if self.failed {
            return Err(TxError::Failed);
        }
        let (off, len) = (chunk.offset as usize, chunk.len as usize);
        if len == 0 || off.checked_add(len).is_none_or(|end| end > self.memory.pcm.len) {
            return Err(TxError::Range);
        }
        if self.rx_free_len == 0 {
            return Err(TxError::Busy);
        }
        self.rx_free_len -= 1;
        let slot = self.rx_free[self.rx_free_len as usize];
        let meta = slot as usize * RX_SLOT_LEN;
        // SAFETY: the slot is inside the receive message memory (`init`
        // made no more slots than fit) and free, so nothing else uses it;
        // the queue's Release fence orders these writes before the device
        // sees the message.
        unsafe {
            let p = self.memory.rx_meta.ptr.as_ptr().add(meta);
            ptr::copy_nonoverlapping(id.to_le_bytes().as_ptr(), p, XFER_LEN);
            ptr::copy_nonoverlapping(chunk.offset.to_le_bytes().as_ptr(), p.add(4), 4);
            ptr::write_bytes(p.add(8), 0, PcmStatus::LEN);
        }
        let bus = self.memory.rx_meta.bus + meta as u64;
        let header = Buffer { addr: bus, len: XFER_LEN as u32 };
        let writable = [
            Buffer { addr: self.memory.pcm.bus + off as u64, len: len as u32 },
            Buffer { addr: bus + 8, len: PcmStatus::LEN as u32 },
        ];
        match self.rx.add(&[header], &writable) {
            Ok(head) => {
                self.rx_owner[head as usize] = slot;
                self.rx_len[slot as usize] = len as u32;
            }
            Err(_) => {
                self.rx_free[self.rx_free_len as usize] = slot;
                self.rx_free_len += 1;
                return Err(TxError::Busy);
            }
        }
        if self.rx.should_notify() {
            self.rx_n.notify(&self.notify);
            self.stats.rx_kicks += 1;
        }
        Ok(())
    }

    /// Take back every input message the device returned, oldest first.
    /// The used length counts the data and the status the device wrote
    /// (§5.14.6.8), so the data is that minus the status, at most the
    /// buffer. A length that does not even cover the status, with an OK
    /// status, is taken as a full buffer and counted (`rx_short`), as for
    /// QEMU's short control responses.
    pub fn poll_rx(&mut self, mut done: impl FnMut(RxDone)) {
        while !self.failed {
            match self.rx.pop_used() {
                Ok(Some(u)) => {
                    let slot = self.rx_owner[u.head as usize];
                    let meta = slot as usize * RX_SLOT_LEN;
                    let mut b = [0u8; RX_SLOT_LEN];
                    // SAFETY: inside the receive message memory; the device
                    // handed the message back (Acquire fence in `pop_used`).
                    unsafe {
                        ptr::copy_nonoverlapping(self.memory.rx_meta.ptr.as_ptr().add(meta), b.as_mut_ptr(), b.len())
                    };
                    let stream = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                    let offset = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                    let status = PcmStatus::decode(&b[8..]);
                    let posted = self.rx_len[slot as usize];
                    let len = if (u.len as usize) < PcmStatus::LEN {
                        self.stats.rx_short += 1;
                        posted
                    } else {
                        (u.len - PcmStatus::LEN as u32).min(posted)
                    };
                    if status.status != snd_status::OK {
                        self.stats.rx_errors += 1;
                    } else {
                        self.stats.rx_messages += 1;
                        self.stats.rx_bytes += len as u64;
                    }
                    self.rx_free[self.rx_free_len as usize] = slot;
                    self.rx_free_len += 1;
                    done(RxDone { stream, offset, len, status });
                }
                Ok(None) => return,
                Err(_) => self.fail(),
            }
        }
    }

    // ---- events ------------------------------------------------------------

    fn kick_events(&mut self) {
        if self.event.should_notify() {
            self.event_n.notify(&self.notify);
        }
    }

    /// Take every event the device sent (§5.14.6.10) and hand the buffers
    /// back.
    pub fn poll_events(&mut self, mut each: impl FnMut(Event)) {
        let mut reposted = false;
        while !self.failed {
            match self.event.pop_used() {
                Ok(Some(u)) => {
                    let slot = self.event_owner[u.head as usize];
                    let mut b = [0u8; Event::LEN];
                    // SAFETY: inside the event memory (one buffer per
                    // slot); the device handed it back.
                    unsafe {
                        ptr::copy_nonoverlapping(
                            self.memory.events.ptr.as_ptr().add(slot as usize * Event::LEN),
                            b.as_mut_ptr(),
                            Event::LEN,
                        )
                    };
                    if u.len as usize >= Event::LEN
                        && let Some(e) = Event::decode(&b)
                    {
                        self.stats.events += 1;
                        match e.code {
                            code::EVT_PCM_PERIOD_ELAPSED => self.stats.period_elapsed += 1,
                            code::EVT_PCM_XRUN => self.stats.xruns += 1,
                            code::EVT_JACK_CONNECTED | code::EVT_JACK_DISCONNECTED => self.stats.jack_events += 1,
                            _ => {}
                        }
                        each(e);
                    }
                    match self.event.add(&[], &[event_buffer(&self.memory.events, slot)]) {
                        Ok(head) => self.event_owner[head as usize] = slot,
                        Err(_) => self.fail(),
                    }
                    reposted = true;
                }
                Ok(None) => break,
                Err(_) => self.fail(),
            }
        }
        if reposted && !self.failed {
            self.kick_events();
        }
    }

    // ---- interrupts --------------------------------------------------------

    /// Ask for interrupts on returned output and input messages and on
    /// events. Returns true if some arrived meanwhile (poll before
    /// waiting).
    pub fn enable_interrupts(&mut self) -> bool {
        if self.failed {
            return false;
        }
        let a = self.tx.enable_interrupts();
        let b = self.event.enable_interrupts();
        let c = self.rx.enable_interrupts();
        a || b || c
    }

    /// Ask the device not to interrupt (a hint, §2.7.7).
    pub fn disable_interrupts(&mut self) {
        self.tx.disable_interrupts();
        self.event.disable_interrupts();
        self.rx.disable_interrupts();
    }

    /// Is there anything to collect?
    pub fn has_work(&self) -> bool {
        !self.failed && (self.tx.has_used() || self.event.has_used() || self.rx.has_used())
    }

    // ---- PCM memory --------------------------------------------------------

    fn pcm_range(&self, offset: usize, len: usize) -> bool {
        offset.checked_add(len).is_some_and(|end| end <= self.memory.pcm.len)
    }

    /// Copy `data` into the PCM memory at `offset`. False if it does not
    /// fit. The caller only writes bytes no message in flight points at.
    pub fn pcm_write(&mut self, offset: usize, data: &[u8]) -> bool {
        if !self.pcm_range(offset, data.len()) {
            return false;
        }
        // SAFETY: inside the PCM memory (checked); the caller writes only
        // bytes the device is not reading (see above).
        unsafe { ptr::copy_nonoverlapping(data.as_ptr(), self.memory.pcm.ptr.as_ptr().add(offset), data.len()) };
        true
    }

    /// Fill `len` bytes at `offset` with repeats of `pattern` (silence for
    /// a format, one sample long). False if it does not fit.
    pub fn pcm_fill(&mut self, offset: usize, len: usize, pattern: &[u8]) -> bool {
        if !self.pcm_range(offset, len) || pattern.is_empty() {
            return false;
        }
        for i in 0..len {
            // SAFETY: as for `pcm_write`.
            unsafe { self.memory.pcm.ptr.as_ptr().add(offset + i).write(pattern[i % pattern.len()]) };
        }
        true
    }

    /// Copy PCM memory at `offset` into `out`. False if out of range.
    pub fn pcm_read(&self, offset: usize, out: &mut [u8]) -> bool {
        if !self.pcm_range(offset, out.len()) {
            return false;
        }
        // SAFETY: inside the PCM memory; reading bytes the device may be
        // reading too is harmless (output messages are device-readable).
        unsafe { ptr::copy_nonoverlapping(self.memory.pcm.ptr.as_ptr().add(offset), out.as_mut_ptr(), out.len()) };
        true
    }
}
