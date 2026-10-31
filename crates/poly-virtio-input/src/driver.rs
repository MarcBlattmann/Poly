//! The driver: events in from the event queue, which is kept full, and
//! events out (LEDs) through the status queue.

use crate::{
    ConfigError, DRIVER_FEATURES, EVENT_LEN, EVENT_QUEUE, RawEvent, STATUS_QUEUE, event_bytes, parse_event, read_info,
};
use alloc::vec;
use alloc::vec::Vec;
use core::ptr::NonNull;
use poly_input::hub::DeviceInfo;
use poly_pci::Mmio;
use poly_virtio::{
    Buffer, CommonCfg, Configuring, DescState, Layout, Notifier, PciTransport, SetupError, SplitQueue, begin, status,
};

/// Polls of the status register while waiting for a reset to finish.
const RESET_POLLS: u32 = 1000;
/// The most entries the event queue gets: room for a burst of events
/// between two interrupts.
pub const MAX_EVENT_ENTRIES: u16 = 64;
/// The most entries the status queue gets (an LED update is 4 events).
pub const MAX_STATUS_ENTRIES: u16 = 16;

/// Memory shared with the device: its CPU address, bus address and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaRegion {
    pub ptr: NonNull<u8>,
    pub bus: u64,
    pub len: usize,
}

/// The memory of one virtqueue: the ring (see [`Layout`], 16-byte aligned)
/// and its buffers, [`EVENT_LEN`] bytes per queue entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueMemory {
    pub ring: DmaRegion,
    pub buffers: DmaRegion,
}

/// Caller-provided descriptor tables; each queue gets no more entries than
/// its table has.
#[derive(Debug)]
pub struct Storage<'a> {
    pub event_desc: &'a mut [DescState],
    pub status_desc: &'a mut [DescState],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitError {
    Virtio(poly_virtio::InitError),
    Config(ConfigError),
    Queue(SetupError),
    /// A queue's notification register lies outside the notification
    /// region.
    NoNotifier,
    /// Not enough memory or table space for queues of 2 entries.
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
    /// The common configuration registers; kept, to reset the device if it
    /// misbehaves.
    pub common: CommonCfg<M>,
    /// The device configuration region and its length.
    pub device_cfg: C,
    pub device_cfg_len: u32,
    pub transport: PciTransport,
    /// The notification region.
    pub notify: N,
    pub event: QueueMemory,
    pub status: QueueMemory,
    pub storage: Storage<'a>,
    /// The MSI-X table entry both queues and configuration changes use
    /// (`None`: no interrupt). MSI-X must be enabled on the function.
    pub vector: Option<u16>,
}

/// Counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub events: u64,
    /// Buffers handed back shorter than an event.
    pub short: u64,
    /// Events sent on the status queue.
    pub sent: u64,
    pub kicks: u64,
}

/// Why [`VirtioInput::send`] did not send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// Every status buffer is in flight; try again later.
    Busy,
    /// More events than the status queue holds.
    TooMany,
    /// The device was reset after breaking the protocol.
    Failed,
}

/// The largest power-of-two queue size, at most `max`, `cap` and `room`
/// table entries, whose ring and buffers fit `mem`.
fn queue_size(max: u16, cap: u16, room: usize, mem: &QueueMemory) -> u16 {
    let room = (max.min(cap) as usize).min(room);
    (1..=15)
        .rev()
        .map(|k| 1u16 << k)
        .find(|&s| {
            s as usize <= room
                && Layout::new(s).is_some_and(|l| l.total() <= mem.ring.len)
                && s as usize * EVENT_LEN <= mem.buffers.len
        })
        .unwrap_or(0)
}

/// Set up queue `index` of `size` entries.
///
/// # Safety
///
/// As for [`init`]: the ring memory is the caller's to give to the device.
unsafe fn setup_queue<'a, M: Mmio>(
    cfg: &Configuring<'_, M>,
    transport: &PciTransport,
    index: u16,
    size: u16,
    mem: &QueueMemory,
    desc: &'a mut [DescState],
    vector: Option<u16>,
) -> Result<(SplitQueue<'a>, Notifier), InitError> {
    let layout = Layout::new(size).ok_or(InitError::NoRoom)?;
    let event_idx = cfg.has(poly_virtio::feature::EVENT_IDX);
    // SAFETY: our caller vouches for the memory; the layout fits in it, and
    // the queue is not enabled yet.
    let queue =
        unsafe { SplitQueue::new(mem.ring.ptr, mem.ring.bus, layout, desc, event_idx) }.map_err(InitError::Queue)?;
    let notify_off = cfg.enable_queue(index, size, queue.addresses(), vector)?;
    let notifier = transport.notifier(index, notify_off).ok_or(InitError::NoNotifier)?;
    Ok((queue, notifier))
}

/// Buffer `slot` of `mem`.
fn buffer(mem: &DmaRegion, slot: u16) -> Buffer {
    Buffer { addr: mem.bus + (slot as usize * EVENT_LEN) as u64, len: EVENT_LEN as u32 }
}

/// Initialise a virtio input device (§3.1.1, §5.8.5) and return the
/// running driver: features negotiated, the configuration read into a
/// device description, the event queue filled with buffers, the status
/// queue ready, DRIVER_OK set. On failure the device is left marked
/// FAILED (or, if it refused the reset, untouched).
///
/// # Safety
///
/// The ring and buffer memory of `event` and `status` must be valid for
/// reads and writes for `'a`, reachable by the device at their bus
/// addresses, and used by nothing but the driver and the device. `common`,
/// `device_cfg`, `transport` and `notify` must all belong to the same
/// device.
pub unsafe fn init<'a, M: Mmio, C: Mmio, N: Mmio>(
    setup: Setup<'a, M, C, N>,
) -> Result<VirtioInput<'a, M, N>, InitError> {
    let Setup { common, device_cfg, device_cfg_len, transport, notify, event, status, storage, vector } = setup;
    let negotiation = begin(&common, RESET_POLLS)?;
    let offered = negotiation.offered();
    let cfg = negotiation.accept(offered & DRIVER_FEATURES)?;
    macro_rules! fail {
        ($e:expr) => {{
            cfg.fail();
            return Err($e);
        }};
    }
    let info = match read_info(&common, &device_cfg, device_cfg_len) {
        Ok(i) => i,
        Err(e) => fail!(InitError::Config(e)),
    };
    if let Some(v) = vector
        && let Err(e) = cfg.set_config_vector(v)
    {
        fail!(InitError::Virtio(e));
    }
    let Storage { event_desc, status_desc } = storage;
    let (event_max, status_max) = match (cfg.queue_max_size(EVENT_QUEUE), cfg.queue_max_size(STATUS_QUEUE)) {
        (Ok(e), Ok(s)) => (e, s),
        (Err(e), _) | (_, Err(e)) => fail!(InitError::Virtio(e)),
    };
    let event_size = queue_size(event_max, MAX_EVENT_ENTRIES, event_desc.len(), &event);
    let status_size = queue_size(status_max, MAX_STATUS_ENTRIES, status_desc.len(), &status);
    if event_size < 2 || status_size < 2 {
        fail!(InitError::NoRoom);
    }
    // SAFETY: forwarded from our caller.
    let (mut event_queue, event_notifier) =
        match unsafe { setup_queue(&cfg, &transport, EVENT_QUEUE, event_size, &event, event_desc, vector) } {
            Ok(q) => q,
            Err(e) => fail!(e),
        };
    // SAFETY: forwarded from our caller.
    let (status_queue, status_notifier) =
        match unsafe { setup_queue(&cfg, &transport, STATUS_QUEUE, status_size, &status, status_desc, vector) } {
            Ok(q) => q,
            Err(e) => fail!(e),
        };
    // §5.8.5: fill the event queue before DRIVER_OK; notify after it.
    let mut event_owner = vec![0u16; event_size as usize];
    for slot in 0..event_size {
        match event_queue.add(&[], &[buffer(&event.buffers, slot)]) {
            Ok(head) => event_owner[head as usize] = slot,
            Err(_) => fail!(InitError::NoRoom),
        }
    }
    let features = cfg.driver_ok();
    let mut dev = VirtioInput {
        common,
        notify,
        events: event_queue,
        status: status_queue,
        event_notifier,
        status_notifier,
        event_mem: event.buffers,
        status_mem: status.buffers,
        event_owner,
        status_owner: vec![0u16; status_size as usize],
        status_free: (0..status_size).collect(),
        info,
        features,
        stats: Stats::default(),
        failed: false,
        reset_done: false,
    };
    dev.kick_events();
    Ok(dev)
}

/// A running virtio input device. See the [crate documentation](crate).
pub struct VirtioInput<'a, M, N> {
    common: CommonCfg<M>,
    notify: N,
    events: SplitQueue<'a>,
    status: SplitQueue<'a>,
    event_notifier: Notifier,
    status_notifier: Notifier,
    event_mem: DmaRegion,
    status_mem: DmaRegion,
    /// For each descriptor: the event buffer whose chain starts there.
    event_owner: Vec<u16>,
    status_owner: Vec<u16>,
    /// The status buffers not in flight.
    status_free: Vec<u16>,
    info: DeviceInfo,
    features: u64,
    stats: Stats,
    failed: bool,
    reset_done: bool,
}

// SAFETY: the queue and buffer memory belong to the driver alone (the
// contract of `init`), so moving the driver moves their only CPU-side user
// with it; the rest is plain data or the caller's own types.
unsafe impl<M: Send, N: Send> Send for VirtioInput<'_, M, N> {}

impl<M, N> core::fmt::Debug for VirtioInput<'_, M, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VirtioInput")
            .field("name", &self.info.name)
            .field("features", &self.features)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl<M: Mmio, N: Mmio> VirtioInput<'_, M, N> {
    /// The device, as its configuration describes it.
    pub fn info(&self) -> &DeviceInfo {
        &self.info
    }

    pub const fn features(&self) -> u64 {
        self.features
    }

    pub const fn stats(&self) -> Stats {
        self.stats
    }

    pub const fn event_queue_size(&self) -> u16 {
        self.events.size()
    }

    pub const fn status_queue_size(&self) -> u16 {
        self.status.size()
    }

    /// The device broke the protocol and was reset.
    pub const fn is_failed(&self) -> bool {
        self.failed
    }

    /// Ask the device for an interrupt per event. Returns true if events
    /// arrived meanwhile (poll again before waiting).
    pub fn enable_interrupts(&mut self) -> bool {
        if self.failed {
            return false;
        }
        self.status.disable_interrupts();
        self.events.enable_interrupts()
    }

    pub fn disable_interrupts(&mut self) {
        self.events.disable_interrupts();
        self.status.disable_interrupts();
    }

    fn fail(&mut self) {
        self.failed = true;
        if !self.reset_done && self.common.reset(RESET_POLLS).is_ok() {
            self.reset_done = true;
            self.common.set_status(status::FAILED);
        }
    }

    fn kick_events(&mut self) {
        if self.events.should_notify() {
            self.event_notifier.notify(&self.notify);
            self.stats.kicks += 1;
        }
    }

    /// Take the events the device delivered, up to `out.len()`, giving
    /// their buffers back to it. Returns how many.
    pub fn poll(&mut self, out: &mut [RawEvent]) -> usize {
        let mut n = 0;
        let mut reposted = false;
        while n < out.len() && !self.failed {
            let u = match self.events.pop_used() {
                Ok(Some(u)) => u,
                Ok(None) => break,
                Err(_) => {
                    self.fail();
                    break;
                }
            };
            let slot = self.event_owner[u.head as usize];
            if u.len as usize >= EVENT_LEN {
                // SAFETY: the slot is inside the event memory (one
                // EVENT_LEN buffer per queue entry, `init`'s contract) and
                // the device handed it back (after the queue's Acquire
                // fence), so it no longer writes there until it is posted
                // again below.
                let bytes: [u8; EVENT_LEN] = unsafe {
                    core::ptr::read_volatile(self.event_mem.ptr.as_ptr().add(slot as usize * EVENT_LEN).cast())
                };
                out[n] = parse_event(&bytes);
                n += 1;
                self.stats.events += 1;
            } else {
                self.stats.short += 1;
            }
            match self.events.add(&[], &[buffer(&self.event_mem, slot)]) {
                Ok(head) => {
                    self.event_owner[head as usize] = slot;
                    reposted = true;
                }
                // One descriptor per buffer: a buffer back means a free
                // descriptor.
                Err(_) => self.fail(),
            }
        }
        if reposted && !self.failed {
            self.kick_events();
        }
        n
    }

    /// Take back the status buffers the device has read.
    fn reclaim(&mut self) {
        while !self.failed {
            match self.status.pop_used() {
                Ok(Some(u)) => {
                    let slot = self.status_owner[u.head as usize];
                    self.status_free.push(slot);
                }
                Ok(None) => return,
                Err(_) => self.fail(),
            }
        }
    }

    /// Send `events` to the device on the status queue (`EV_LED` events
    /// and a `SYN_REPORT`, say), all or none.
    pub fn send(&mut self, events: &[RawEvent]) -> Result<(), SendError> {
        self.reclaim();
        if self.failed {
            return Err(SendError::Failed);
        }
        if events.len() > self.status.size() as usize {
            return Err(SendError::TooMany);
        }
        if events.len() > self.status_free.len() {
            return Err(SendError::Busy);
        }
        for &e in events {
            let Some(slot) = self.status_free.pop() else { return Err(SendError::Busy) };
            let bytes = event_bytes(e);
            // SAFETY: the slot is inside the status memory and free: the
            // device handed it back or never had it. The queue's Release
            // fence orders the write before the device can see the buffer.
            unsafe {
                core::ptr::write_volatile(self.status_mem.ptr.as_ptr().add(slot as usize * EVENT_LEN).cast(), bytes);
            }
            match self.status.add(&[buffer(&self.status_mem, slot)], &[]) {
                Ok(head) => self.status_owner[head as usize] = slot,
                Err(_) => {
                    self.status_free.push(slot);
                    return Err(SendError::Busy);
                }
            }
            self.stats.sent += 1;
        }
        if self.status.should_notify() {
            self.status_notifier.notify(&self.notify);
            self.stats.kicks += 1;
        }
        Ok(())
    }
}
