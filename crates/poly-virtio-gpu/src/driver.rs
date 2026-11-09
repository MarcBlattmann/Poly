//! The driver: device initialisation and fenced commands on the control
//! queue, waited for synchronously or collected later.
//!
//! Every request sets VIRTIO_GPU_FLAG_FENCE with a fresh fence ID, so its
//! response comes only once the command is complete (§5.7.6.5), which
//! matters before the memory behind a resource is reused; the response must
//! echo the fence. Requests live in [`SLOTS`] slots of the command memory
//! (the request, then room for its response); each one in flight is a
//! [`Ticket`].
//!
//! - **Synchronous** commands (everything 2D, contexts, resources, capsets)
//!   go out one or a few at a time (a transfer and its flush share one
//!   notification) and the driver polls the used ring for their responses:
//!   the kernel console draws with interrupts off, so it cannot sleep until
//!   an interrupt says the device is done, and these commands are short.
//! - **Asynchronous** commands (3D command submission and 3D transfers,
//!   whose fence completes when the host's GPU work does) return their
//!   ticket at once; [`VirtioGpu::poll`] collects the response later. With
//!   an interrupt vector ([`Setup::queue_vector`]) the device interrupts
//!   when it gives buffers back, and the kernel polls then.
//!
//! Responses may come back in any order (a 3D fence completes long after
//! the commands behind it were answered); [`VirtioGpu::reap`] files each
//! used buffer under its slot, whoever is waiting.
//!
//! A device that does not answer a synchronous command within the
//! [`Clock`]'s timeout, or that breaks the queue protocol, still owns the
//! buffers in flight; the driver then refuses every further command
//! ([`Error::Broken`]) and the caller must never reuse the command memory
//! (nor the data of tickets still in flight).

use crate::modes::Plan;
use crate::protocol::{
    ATTACH_BACKING_LEN, BYTES_PER_PIXEL, CapsetInfo, Command, Create3d, CreateBlob, DisplayInfo, EdidBlob, HEADER_LEN,
    MEM_ENTRY_LEN, MemEntry, RESPONSE_MAX, Rect, Response, ResponseError, Transfer3d, decode_expected, format,
};
use crate::{CONTROL_QUEUE, ConfigError, GpuConfig, cfg, feature, select_features};
use core::ptr::NonNull;
use poly_pci::Mmio;
use poly_virtio::{
    AddError, Buffer, CommonCfg, Configuring, DescState, Layout, Notifier, PciTransport, SetupError, SplitQueue,
    UsedError, begin,
};

/// Polls of the status register while waiting for a reset to finish.
const RESET_POLLS: u32 = 1000;
/// Requests in flight at once.
pub const SLOTS: usize = 16;
/// Bytes of command memory per slot: the request, then the response.
const SLOT_LEN: usize = 4096;
/// Where a slot's response starts.
const RESPONSE_OFFSET: usize = 2048;
/// Bytes of command memory the driver needs ([`Setup::commands`]).
pub const COMMAND_MEMORY_LEN: usize = SLOTS * SLOT_LEN;
/// The most scatter-list entries one `RESOURCE_ATTACH_BACKING` can carry
/// here (they must fit a slot's request part).
pub const MAX_BACKING_ENTRIES: usize = (RESPONSE_OFFSET - ATTACH_BACKING_LEN) / MEM_ENTRY_LEN;
/// The most device-readable buffers that may follow one request (a 3D
/// command stream in pieces).
pub const MAX_DATA_BUFFERS: usize = 4;
/// The largest control queue the driver sets up, and the smallest it works
/// with (two requests of two descriptors).
const MAX_QUEUE: u16 = 64;
const MIN_QUEUE: u16 = 4;

const _: () = assert!(RESPONSE_OFFSET + RESPONSE_MAX <= SLOT_LEN);
const _: () = assert!(SLOTS <= u8::MAX as usize);

/// Memory shared with the device: its CPU address, bus address and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaRegion {
    pub ptr: NonNull<u8>,
    pub bus: u64,
    pub len: usize,
}

/// How the driver tells time while it waits for the device.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    /// A monotonic clock in nanoseconds.
    pub now_ns: fn() -> u64,
    /// How long one synchronous command may take before the device is
    /// given up on.
    pub timeout_ns: u64,
}

/// What [`init`] needs.
#[derive(Debug)]
pub struct Setup<'a, M, C, N> {
    /// The common configuration registers; kept, to reset the device.
    pub common: CommonCfg<M>,
    /// The device configuration region and its length; kept, for events.
    pub device_cfg: C,
    pub device_cfg_len: u32,
    pub transport: PciTransport,
    /// The notification region.
    pub notify: N,
    /// The control queue's ring (see [`Layout`], 16-byte aligned), for up to
    /// 64 entries; at least 4.
    pub ring: DmaRegion,
    /// At least [`COMMAND_MEMORY_LEN`] bytes for requests and responses.
    pub commands: DmaRegion,
    /// The control queue's descriptor bookkeeping, one per queue entry.
    pub desc: &'a mut [DescState],
    pub clock: Clock,
    /// The MSI-X table entry the control queue interrupts through, or
    /// `None` to poll (no interrupts asked for).
    pub queue_vector: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitError {
    Virtio(poly_virtio::InitError),
    Config(ConfigError),
    Queue(SetupError),
    /// The control queue's notification register lies outside the
    /// notification region.
    NoNotifier,
    /// Not enough ring or command memory, or table entries, for a control
    /// queue of 4 entries.
    NoRoom,
}

impl From<poly_virtio::InitError> for InitError {
    fn from(e: poly_virtio::InitError) -> InitError {
        InitError::Virtio(e)
    }
}

/// Why a command failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The device answered with an error, or an answer that does not fit
    /// the request.
    Response(ResponseError),
    /// No answer to `request` within the timeout. The driver is broken.
    Timeout { request: u32 },
    /// The device broke the queue protocol. The driver is broken.
    Queue(UsedError),
    /// The device gave back a buffer twice or one never sent. The driver
    /// is broken.
    Protocol,
    /// The driver gave up on the device earlier.
    Broken,
    /// More scatter-list entries than fit a request, or none; a request
    /// too long for a slot; too many data buffers.
    BadBacking,
    /// The backing is smaller than the framebuffer it should hold.
    BackingTooSmall { have: u64, need: u64 },
    /// `GET_EDID` without VIRTIO_GPU_F_EDID.
    NoEdid,
    /// A rectangle outside its resource, or a resource of size 0.
    BadRect,
    /// Every slot (or queue entry) is in use: collect finished tickets and
    /// try again.
    Busy,
    /// A ticket the driver does not know (already collected).
    BadTicket,
    /// A 3D or blob command without the feature it needs negotiated.
    Unsupported,
}

impl From<ResponseError> for Error {
    fn from(e: ResponseError) -> Error {
        Error::Response(e)
    }
}

/// Counters, for logs and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Commands sent.
    pub commands: u64,
    /// Commands the device answered with an error.
    pub errors: u64,
    /// Notifications sent.
    pub notifies: u64,
    /// `TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH` pairs.
    pub updates: u64,
    /// Display-change events taken.
    pub display_events: u64,
    /// `SUBMIT_3D` commands sent.
    pub submits: u64,
}

/// A request in flight, to collect with [`VirtioGpu::poll`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket {
    slot: u8,
    serial: u32,
    /// The request's fence ID.
    pub fence: u64,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    head: u16,
    request: u32,
    ok_type: u32,
    fence: u64,
    /// Whether the request carried its fence (VIRTIO_GPU_FLAG_FENCE).
    fenced: bool,
    /// Where the response goes if not in the slot.
    response: Option<DmaRegion>,
    /// Bytes the device wrote, once it gave the buffers back.
    done: Option<u32>,
    /// Nobody will collect it: the slot is freed when the device is done.
    discard: bool,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    pending: Option<Pending>,
    serial: u32,
}

/// The largest power-of-two queue size, at most `max` (the device's
/// limit), `room` (table entries) and [`MAX_QUEUE`], whose ring fits
/// `ring_len` bytes.
fn queue_size(max: u16, room: usize, ring_len: usize) -> u16 {
    let room = (max as usize).min(room).min(MAX_QUEUE as usize);
    (1..=MAX_QUEUE.trailing_zeros())
        .rev()
        .map(|k| 1u16 << k)
        .find(|&s| s as usize <= room && Layout::new(s).is_some_and(|l| l.total() <= ring_len))
        .unwrap_or(0)
}

/// Initialise a virtio GPU device (§3.1.1, §5.7.5) and return the running
/// driver: features negotiated (EDID, 3D, blobs and context init when
/// offered), the configuration read and checked, the control queue set up
/// (interrupting through [`Setup::queue_vector`] if given, else polled),
/// DRIVER_OK set. The cursor queue stays disabled. On failure the device
/// is left marked FAILED (or, if it refused the reset, untouched).
///
/// # Safety
///
/// The ring and command memory must be valid for reads and writes for
/// `'a`, reachable by the device at their bus addresses, and used by
/// nothing but the driver and the device. `common`, `device_cfg`,
/// `transport` and `notify` must all belong to the same device.
pub unsafe fn init<'a, M: Mmio, C: Mmio, N: Mmio>(
    setup: Setup<'a, M, C, N>,
) -> Result<VirtioGpu<'a, M, C, N>, InitError> {
    let Setup { common, device_cfg, device_cfg_len, transport, notify, ring, commands, desc, clock, queue_vector } =
        setup;
    let negotiation = begin(&common, RESET_POLLS)?;
    let wanted = select_features(negotiation.offered());
    let cfg = negotiation.accept(wanted)?;
    macro_rules! fail {
        ($e:expr) => {{
            cfg.fail();
            return Err($e);
        }};
    }
    let config = match GpuConfig::read(&common, &device_cfg, device_cfg_len) {
        Ok(c) => c,
        Err(e) => fail!(InitError::Config(e)),
    };
    if commands.len < COMMAND_MEMORY_LEN {
        fail!(InitError::NoRoom);
    }
    let max = match cfg.queue_max_size(CONTROL_QUEUE) {
        Ok(m) => m,
        Err(e) => fail!(InitError::Virtio(e)),
    };
    let size = queue_size(max, desc.len(), ring.len);
    if size < MIN_QUEUE {
        fail!(InitError::NoRoom);
    }
    // SAFETY: our caller vouches for the ring memory; the layout fits in it
    // (the size was chosen so) and the queue is not enabled yet.
    let (queue, notifier) = match unsafe { setup_queue(&cfg, &transport, size, ring, desc, queue_vector) } {
        Ok(q) => q,
        Err(e) => fail!(e),
    };
    let features = cfg.driver_ok();
    let mut gpu = VirtioGpu {
        common,
        device_cfg,
        notify,
        notifier,
        queue,
        cmd: commands,
        features,
        config,
        clock,
        next_fence: 1,
        broken: false,
        stats: Stats::default(),
        slots: [Slot { pending: None, serial: 0 }; SLOTS],
        interrupts: queue_vector.is_some(),
    };
    if gpu.interrupts {
        gpu.queue.enable_interrupts();
    } else {
        // Polled: ask the device not to interrupt (a hint; no vector anyway).
        gpu.queue.disable_interrupts();
    }
    Ok(gpu)
}

/// Set up the control queue in `ring` and find its notification register.
///
/// # Safety
///
/// As for [`init`].
unsafe fn setup_queue<'a, M: Mmio>(
    cfg: &Configuring<'_, M>,
    transport: &PciTransport,
    size: u16,
    ring: DmaRegion,
    desc: &'a mut [DescState],
    vector: Option<u16>,
) -> Result<(SplitQueue<'a>, Notifier), InitError> {
    let layout = Layout::new(size).ok_or(InitError::NoRoom)?;
    let event_idx = cfg.has(poly_virtio::feature::EVENT_IDX);
    // SAFETY: passed on from our caller.
    let queue = unsafe { SplitQueue::new(ring.ptr, ring.bus, layout, desc, event_idx) }.map_err(InitError::Queue)?;
    let notify_off = cfg.enable_queue(CONTROL_QUEUE, size, queue.addresses(), vector)?;
    let notifier = transport.notifier(CONTROL_QUEUE, notify_off).ok_or(InitError::NoNotifier)?;
    Ok((queue, notifier))
}

/// A running virtio GPU.
pub struct VirtioGpu<'a, M, C, N> {
    common: CommonCfg<M>,
    device_cfg: C,
    notify: N,
    notifier: Notifier,
    queue: SplitQueue<'a>,
    cmd: DmaRegion,
    features: u64,
    config: GpuConfig,
    clock: Clock,
    next_fence: u64,
    broken: bool,
    stats: Stats,
    slots: [Slot; SLOTS],
    interrupts: bool,
}

impl<M, C, N> core::fmt::Debug for VirtioGpu<'_, M, C, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VirtioGpu")
            .field("features", &format_args!("{:#x}", self.features))
            .field("config", &self.config)
            .field("broken", &self.broken)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

// SAFETY: the command memory belongs to the driver alone (the contract of
// `init`), as does the queue; moving the driver to another CPU moves the
// only CPU-side user of both with it. The register blocks are moved with it
// and must be `Send` themselves. External response regions of tickets in
// flight are the caller's promise (`send`) and are only touched through the
// driver.
unsafe impl<M: Send, C: Send, N: Send> Send for VirtioGpu<'_, M, C, N> {}

impl<M: Mmio, C: Mmio, N: Mmio> VirtioGpu<'_, M, C, N> {
    /// The negotiated features.
    pub const fn features(&self) -> u64 {
        self.features
    }

    /// The device configuration read at initialisation.
    pub const fn config(&self) -> GpuConfig {
        self.config
    }

    pub const fn num_scanouts(&self) -> u32 {
        self.config.num_scanouts
    }

    /// Whether `GET_EDID` works (VIRTIO_GPU_F_EDID negotiated).
    pub const fn has_edid(&self) -> bool {
        self.features & feature::EDID != 0
    }

    /// Whether 3D commands work (VIRTIO_GPU_F_VIRGL negotiated).
    pub const fn has_virgl(&self) -> bool {
        self.features & feature::VIRGL != 0
    }

    /// Whether blob resources work (VIRTIO_GPU_F_RESOURCE_BLOB).
    pub const fn has_blob(&self) -> bool {
        self.features & feature::RESOURCE_BLOB != 0
    }

    /// Whether contexts may name their capset and fence on rings
    /// (VIRTIO_GPU_F_CONTEXT_INIT).
    pub const fn has_context_init(&self) -> bool {
        self.features & feature::CONTEXT_INIT != 0
    }

    /// Whether the control queue interrupts when the device gives buffers
    /// back (else the driver must be polled).
    pub const fn interrupts(&self) -> bool {
        self.interrupts
    }

    pub const fn stats(&self) -> Stats {
        self.stats
    }

    /// Whether the driver gave up on the device.
    pub const fn is_broken(&self) -> bool {
        self.broken
    }

    /// The control queue's size.
    pub const fn queue_size(&self) -> u16 {
        self.queue.size()
    }

    /// Requests sent and not collected yet.
    pub fn in_flight(&self) -> usize {
        self.slots.iter().filter(|s| s.pending.is_some()).count()
    }

    fn slot(&mut self, i: usize, offset: usize, len: usize) -> (&mut [u8], u64) {
        let at = i * SLOT_LEN + offset;
        debug_assert!(at + len <= self.cmd.len);
        // SAFETY: `init` checked that the command memory holds every slot;
        // it is the driver's alone (the device only touches a slot between
        // `add` and the used element), and no two views of one slot part
        // are ever alive at once.
        let bytes = unsafe { core::slice::from_raw_parts_mut(self.cmd.ptr.as_ptr().add(at), len) };
        (bytes, self.cmd.bus + at as u64)
    }

    /// Put `cmd` on the queue in a free slot (without notifying), fenced on
    /// `ring` of its context if given, followed by the device-readable
    /// `data` and answered into `response` (else the slot). `Busy` if no
    /// slot or not enough queue entries are free.
    ///
    /// # Safety
    ///
    /// `data` and `response` must be reachable by the device at their bus
    /// addresses and stay valid (`response` writable and used by nothing
    /// else) until the ticket is collected or the device reset.
    unsafe fn enqueue(
        &mut self,
        cmd: &Command<'_>,
        ring: Option<u8>,
        data: &[Buffer],
        response: Option<DmaRegion>,
        fenced: bool,
    ) -> Result<Ticket, Error> {
        if self.broken {
            return Err(Error::Broken);
        }
        if cmd.len() > RESPONSE_OFFSET || data.len() > MAX_DATA_BUFFERS {
            return Err(Error::BadBacking);
        }
        let resp_len = cmd.response_len();
        if response.is_some_and(|r| r.len < resp_len) || (response.is_none() && resp_len > RESPONSE_MAX) {
            return Err(Error::BadBacking);
        }
        let i = self.slots.iter().position(|s| s.pending.is_none()).ok_or(Error::Busy)?;
        let fence = self.next_fence;
        let (req, req_bus) = self.slot(i, 0, RESPONSE_OFFSET);
        let len = cmd.encode_on_ring(fenced.then_some(fence), ring, req).ok_or(Error::BadBacking)?;
        let resp_bus = match response {
            Some(r) => {
                // SAFETY: our caller vouches for the region (at least
                // `resp_len` bytes, checked above, of which we clear the
                // header).
                unsafe { core::ptr::write_bytes(r.ptr.as_ptr(), 0, HEADER_LEN) };
                r.bus
            }
            None => {
                let (resp, bus) = self.slot(i, RESPONSE_OFFSET, RESPONSE_MAX);
                // A device that answers without writing is caught as a bad
                // type.
                resp[..HEADER_LEN].fill(0);
                bus
            }
        };
        let mut readable = [Buffer { addr: 0, len: 0 }; 1 + MAX_DATA_BUFFERS];
        readable[0] = Buffer { addr: req_bus, len: len as u32 };
        readable[1..=data.len()].copy_from_slice(data);
        let writable = [Buffer { addr: resp_bus, len: resp_len as u32 }];
        let head = self.queue.add(&readable[..=data.len()], &writable).map_err(|e| match e {
            AddError::Full | AddError::NoBuffers => Error::Busy,
            AddError::TooLong => Error::BadBacking,
        })?;
        self.next_fence += 1;
        self.stats.commands += 1;
        let slot = &mut self.slots[i];
        slot.serial = slot.serial.wrapping_add(1);
        slot.pending = Some(Pending {
            head,
            request: cmd.ty(),
            ok_type: cmd.ok_type(),
            fence,
            fenced,
            response,
            done: None,
            discard: false,
        });
        Ok(Ticket { slot: i as u8, serial: slot.serial, fence })
    }

    /// Notify the device if it wants to be.
    fn kick(&mut self) {
        if self.queue.should_notify() {
            self.notifier.notify(&self.notify);
            self.stats.notifies += 1;
        }
    }

    /// Collect what the device gave back: each used buffer is filed under
    /// its slot (slots nobody will collect are freed). Returns how many
    /// came back. A device that hands back something never sent is given
    /// up on.
    pub fn reap(&mut self) -> Result<usize, Error> {
        if self.broken {
            return Err(Error::Broken);
        }
        let mut n = 0;
        loop {
            let used = match self.queue.pop_used() {
                Ok(Some(u)) => u,
                Ok(None) => return Ok(n),
                Err(e) => return Err(self.give_up(Error::Queue(e))),
            };
            let found =
                self.slots.iter().position(|s| s.pending.is_some_and(|p| p.head == used.head && p.done.is_none()));
            let Some(i) = found else {
                return Err(self.give_up(Error::Protocol));
            };
            n += 1;
            let p = self.slots[i].pending.as_mut().unwrap_or_else(|| unreachable!());
            p.done = Some(used.len);
            if p.discard {
                let p = *p;
                if self.decode(i, &p).is_err() {
                    self.stats.errors += 1;
                }
                self.slots[i].pending = None;
            }
        }
    }

    /// Check the response of slot `i` (whose request `p` describes).
    fn decode(&mut self, i: usize, p: &Pending) -> Result<Response, Error> {
        let written = p.done.unwrap_or(0) as usize;
        let r = match p.response {
            Some(r) => {
                // SAFETY: the caller of `enqueue` keeps the region valid
                // until the ticket is collected, and the device is done
                // with it (its buffers came back).
                let b = unsafe { core::slice::from_raw_parts(r.ptr.as_ptr(), r.len) };
                decode_expected(p.request, p.ok_type, p.fenced.then_some(p.fence), b, written)
            }
            None => {
                let (resp, _) = self.slot(i, RESPONSE_OFFSET, RESPONSE_MAX);
                decode_expected(p.request, p.ok_type, p.fenced.then_some(p.fence), resp, written)
            }
        };
        r.map_err(Error::Response)
    }

    /// The response to `t` if the device has answered (the ticket is then
    /// spent), `None` if it is still working on it.
    fn take(&mut self, t: Ticket) -> Option<Result<Response, Error>> {
        let slot = self.slots.get(t.slot as usize)?;
        let Some(p) = slot.pending.filter(|_| slot.serial == t.serial) else {
            return Some(Err(Error::BadTicket));
        };
        p.done?;
        let r = self.decode(t.slot as usize, &p);
        self.slots[t.slot as usize].pending = None;
        if r.is_err() {
            self.stats.errors += 1;
        }
        Some(r)
    }

    /// Whether the device answered `t`: its result (the ticket is then
    /// spent), or `None` while it works. Collects whatever came back first.
    pub fn poll(&mut self, t: Ticket) -> Option<Result<(), Error>> {
        if let Err(e) = self.reap() {
            return Some(Err(e));
        }
        self.take(t).map(|r| r.map(drop))
    }

    /// Nobody will collect `t`: its slot is freed when the device is done.
    pub fn forget(&mut self, t: Ticket) {
        let Some(slot) = self.slots.get_mut(t.slot as usize) else { return };
        if slot.serial != t.serial {
            return;
        }
        if let Some(p) = slot.pending.as_mut() {
            if p.done.is_some() {
                slot.pending = None;
            } else {
                p.discard = true;
            }
        }
    }

    /// Put `cmds` on the queue, waiting (polling, within the timeout) for
    /// slots to free up, and notify the device once.
    fn send_all(&mut self, cmds: &[Command<'_>], tickets: &mut [Option<Ticket>], fenced: bool) -> Result<(), Error> {
        // A request that cannot fit sends none.
        if cmds.iter().any(|c| c.len() > RESPONSE_OFFSET || c.response_len() > RESPONSE_MAX) {
            return Err(Error::BadBacking);
        }
        let started = (self.clock.now_ns)();
        let mut result = Ok(());
        for (c, t) in cmds.iter().zip(tickets.iter_mut()) {
            loop {
                // SAFETY: no data buffers or external response.
                match unsafe { self.enqueue(c, None, &[], None, fenced) } {
                    Ok(ticket) => {
                        *t = Some(ticket);
                        break;
                    }
                    Err(Error::Busy) => {
                        self.kick();
                        self.reap()?;
                        if (self.clock.now_ns)().wrapping_sub(started) > self.clock.timeout_ns {
                            return Err(self.give_up(Error::Timeout { request: c.ty() }));
                        }
                        core::hint::spin_loop();
                    }
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                }
            }
            if result.is_err() {
                break;
            }
        }
        self.kick();
        result
    }

    /// Wait (polling, within the timeout) for every ticket, then collect
    /// them: the last response, or the first error.
    fn wait_all(&mut self, tickets: &[Option<Ticket>]) -> Result<Response, Error> {
        let started = (self.clock.now_ns)();
        loop {
            self.reap()?;
            let pending = tickets.iter().flatten().find(|t| {
                self.slots[t.slot as usize].pending.is_some_and(|p| p.done.is_none()) && {
                    self.slots[t.slot as usize].serial == t.serial
                }
            });
            let Some(&t) = pending else { break };
            if (self.clock.now_ns)().wrapping_sub(started) > self.clock.timeout_ns {
                let request = self.slots[t.slot as usize].pending.map_or(0, |p| p.request);
                return Err(self.give_up(Error::Timeout { request }));
            }
            core::hint::spin_loop();
        }
        let mut result = Ok(Response::NoData);
        for t in tickets.iter().flatten() {
            // The last response, unless an earlier one was an error.
            if let Some(r) = self.take(*t)
                && result.is_ok()
            {
                result = r;
            }
        }
        result
    }

    /// Send `cmds` (one to [`SLOTS`]) in one notification and wait for their
    /// responses. Returns the last command's response, or the first error.
    fn run(&mut self, cmds: &[Command<'_>]) -> Result<Response, Error> {
        if self.broken {
            return Err(Error::Broken);
        }
        assert!(!cmds.is_empty() && cmds.len() <= SLOTS, "one to SLOTS commands at a time");
        let mut tickets = [None; SLOTS];
        let sent = self.send_all(cmds, &mut tickets[..cmds.len()], true);
        // Whatever went out is waited for, even if a later one could not go.
        let answered = self.wait_all(&tickets[..cmds.len()]);
        sent?;
        answered
    }

    fn give_up(&mut self, e: Error) -> Error {
        self.broken = true;
        e
    }

    fn run_one(&mut self, cmd: Command<'_>) -> Result<Response, Error> {
        self.run(&[cmd])
    }

    /// `GET_DISPLAY_INFO`: each scanout's preferred mode and state.
    pub fn display_info(&mut self) -> Result<DisplayInfo, Error> {
        match self.run_one(Command::GetDisplayInfo)? {
            Response::DisplayInfo(d) => Ok(d),
            _ => Err(Error::Protocol),
        }
    }

    /// `GET_EDID` for `scanout` (VIRTIO_GPU_F_EDID only).
    pub fn edid(&mut self, scanout: u32) -> Result<EdidBlob, Error> {
        if !self.has_edid() {
            return Err(Error::NoEdid);
        }
        match self.run_one(Command::GetEdid { scanout })? {
            Response::Edid(e) => Ok(e),
            _ => Err(Error::Protocol),
        }
    }

    /// `RESOURCE_CREATE_2D`.
    pub fn create_2d(&mut self, resource_id: u32, format: u32, width: u32, height: u32) -> Result<(), Error> {
        self.run_one(Command::ResourceCreate2d { resource_id, format, width, height }).map(drop)
    }

    /// `RESOURCE_ATTACH_BACKING` with one to [`MAX_BACKING_ENTRIES`] pieces.
    pub fn attach_backing(&mut self, resource_id: u32, entries: &[MemEntry]) -> Result<(), Error> {
        if entries.is_empty() || entries.len() > MAX_BACKING_ENTRIES {
            return Err(Error::BadBacking);
        }
        self.run_one(Command::ResourceAttachBacking { resource_id, entries }).map(drop)
    }

    /// `RESOURCE_DETACH_BACKING`. Once it returns `Ok`, the device no longer
    /// uses the memory (the command was fenced).
    pub fn detach_backing(&mut self, resource_id: u32) -> Result<(), Error> {
        self.run_one(Command::ResourceDetachBacking { resource_id }).map(drop)
    }

    /// `RESOURCE_UNREF`.
    pub fn unref(&mut self, resource_id: u32) -> Result<(), Error> {
        self.run_one(Command::ResourceUnref { resource_id }).map(drop)
    }

    /// `SET_SCANOUT`: show `rect` of the resource on `scanout_id`
    /// (resource 0: show nothing).
    pub fn set_scanout(&mut self, scanout_id: u32, resource_id: u32, rect: Rect) -> Result<(), Error> {
        self.run_one(Command::SetScanout { scanout_id, resource_id, rect }).map(drop)
    }

    /// `TRANSFER_TO_HOST_2D`.
    pub fn transfer_to_host_2d(&mut self, resource_id: u32, rect: Rect, offset: u64) -> Result<(), Error> {
        self.run_one(Command::TransferToHost2d { resource_id, rect, offset }).map(drop)
    }

    /// `RESOURCE_FLUSH`.
    pub fn flush(&mut self, resource_id: u32, rect: Rect) -> Result<(), Error> {
        self.run_one(Command::ResourceFlush { resource_id, rect }).map(drop)
    }

    /// Put the damaged `rect` of `fb` on the screen (§5.7.6.2): copy it
    /// from guest memory to the host resource (`TRANSFER_TO_HOST_2D`, the
    /// offset of its first pixel in lines of `fb.stride` bytes) and flush
    /// it to the scanouts (`RESOURCE_FLUSH`), both in one notification.
    /// `rect` is clipped to the framebuffer; nothing is sent when that
    /// leaves nothing.
    pub fn update(&mut self, fb: &Framebuffer, rect: Rect) -> Result<(), Error> {
        let rect = rect.clip(fb.width, fb.height);
        if rect.is_empty() {
            return Ok(());
        }
        let offset = rect.y as u64 * fb.stride as u64 + rect.x as u64 * BYTES_PER_PIXEL as u64;
        let id = fb.resource_id;
        self.run(&[
            Command::TransferToHost2d { resource_id: id, rect, offset },
            Command::ResourceFlush { resource_id: id, rect },
        ])?;
        self.stats.updates += 1;
        Ok(())
    }

    /// Make a framebuffer for `plan` and show it (§5.7.6.1): create
    /// resource `resource_id` of the plan's size in B8G8R8X8 (lines of
    /// `width * 4` bytes), attach `backing` (guest memory holding at least
    /// `width * 4 * height` bytes, in order), and set it on every scanout
    /// the plan uses. Scanouts `old` showed and `plan` does not are turned
    /// off. On failure the resource is destroyed again (as far as the
    /// device lets it be).
    pub fn show(
        &mut self,
        resource_id: u32,
        plan: &Plan,
        old: Option<&Plan>,
        backing: &[MemEntry],
    ) -> Result<Framebuffer, Error> {
        if plan.width == 0 || plan.height == 0 {
            return Err(Error::BadRect);
        }
        let fb =
            Framebuffer { resource_id, width: plan.width, height: plan.height, stride: plan.width * BYTES_PER_PIXEL };
        let have: u64 = backing.iter().map(|e| e.length as u64).sum();
        let need = fb.stride as u64 * fb.height as u64;
        if have < need {
            return Err(Error::BackingTooSmall { have, need });
        }
        self.create_2d(resource_id, format::B8G8R8X8_UNORM, fb.width, fb.height)?;
        let shown = self.attach_backing(resource_id, backing).and_then(|()| self.set_scanouts(&fb, plan, old));
        if let Err(e) = shown {
            if !self.broken {
                let _ = self.detach_backing(resource_id);
                let _ = self.unref(resource_id);
            }
            return Err(e);
        }
        Ok(fb)
    }

    /// Set `fb` on every scanout of `plan` (with the rectangle the plan
    /// gives it, which must fit the framebuffer), and turn off the scanouts
    /// `old` used that `plan` does not.
    pub fn set_scanouts(&mut self, fb: &Framebuffer, plan: &Plan, old: Option<&Plan>) -> Result<(), Error> {
        for (scanout, rect) in plan.active() {
            if rect.is_empty() || !rect.fits(fb.width, fb.height) {
                return Err(Error::BadRect);
            }
            self.set_scanout(scanout, fb.resource_id, rect)?;
        }
        if let Some(old) = old {
            for scanout in plan.scanouts_to_disable(old) {
                self.set_scanout(scanout, 0, Rect::ZERO)?;
            }
        }
        Ok(())
    }

    /// Destroy a framebuffer no scanout shows any more: detach its memory
    /// and unref the resource. Once this returns `Ok`, the memory is the
    /// caller's again.
    pub fn destroy(&mut self, fb: &Framebuffer) -> Result<(), Error> {
        self.detach_backing(fb.resource_id)?;
        self.unref(fb.resource_id)
    }

    /// Pending device events (§5.7.4.2, [`crate::event`]), acknowledged:
    /// the bits read from `events_read` are written to `events_clear`.
    pub fn take_events(&mut self) -> u32 {
        let events = self.device_cfg.read32(cfg::EVENTS_READ);
        if events != 0 && events != u32::MAX {
            self.device_cfg.write32(cfg::EVENTS_CLEAR, events);
            if events & crate::event::DISPLAY != 0 {
                self.stats.display_events += 1;
            }
            return events;
        }
        0
    }

    /// Reset the device: it stops using every buffer it had (§2.4). The
    /// driver is unusable afterwards. Returns whether the reset finished.
    pub fn reset(&mut self) -> bool {
        self.broken = true;
        self.common.reset(RESET_POLLS).is_ok()
    }

    // --- 3D (VIRTIO_GPU_F_VIRGL), capsets, blobs --------------------------------

    fn need(&self, feature: u64) -> Result<(), Error> {
        if self.features & feature == 0 { Err(Error::Unsupported) } else { Ok(()) }
    }

    /// `GET_CAPSET_INFO` for capset number `index` (below the device's
    /// `num_capsets`).
    pub fn capset_info(&mut self, index: u32) -> Result<CapsetInfo, Error> {
        self.need(feature::VIRGL)?;
        if index >= self.config.num_capsets {
            return Err(Error::Unsupported);
        }
        match self.run_one(Command::GetCapsetInfo { index })? {
            Response::CapsetInfo(c) => Ok(c),
            _ => Err(Error::Protocol),
        }
    }

    /// `GET_CAPSET`: capset `id` at `version`, answered into `out` (the
    /// header, then up to `out.len - 24` bytes of the set, which should be
    /// its `max_size`). Returns how many bytes of the set the device wrote;
    /// they start [`HEADER_LEN`] bytes into `out`.
    ///
    /// # Safety
    ///
    /// `out` must be device-reachable memory, writable and used by nothing
    /// else until this returns (or, if it fails with the driver broken,
    /// forever).
    pub unsafe fn capset(&mut self, id: u32, version: u32, out: DmaRegion) -> Result<usize, Error> {
        self.need(feature::VIRGL)?;
        let size = out.len.checked_sub(HEADER_LEN).ok_or(Error::BadBacking)? as u32;
        let cmd = Command::GetCapset { id, version, size };
        // SAFETY: passed on from our caller.
        let t = unsafe { self.send(&cmd, None, &[], Some(out)) }?;
        match self.wait_all(&[Some(t)])? {
            Response::Capset(n) => Ok(n),
            _ => Err(Error::Protocol),
        }
    }

    /// `CTX_CREATE`: context `ctx_id` for the protocol of `capset_id` (0:
    /// the device's default; another needs VIRTIO_GPU_F_CONTEXT_INIT).
    pub fn ctx_create(&mut self, ctx_id: u32, capset_id: u32, name: &[u8]) -> Result<(), Error> {
        self.need(feature::VIRGL)?;
        if capset_id != 0 {
            self.need(feature::CONTEXT_INIT)?;
        }
        let context_init = capset_id & crate::protocol::CONTEXT_INIT_CAPSET_ID_MASK;
        self.run_one(Command::CtxCreate { ctx_id, context_init, name }).map(drop)
    }

    /// `CTX_DESTROY`.
    pub fn ctx_destroy(&mut self, ctx_id: u32) -> Result<(), Error> {
        self.need(feature::VIRGL)?;
        self.run_one(Command::CtxDestroy { ctx_id }).map(drop)
    }

    /// `CTX_ATTACH_RESOURCE`.
    pub fn ctx_attach(&mut self, ctx_id: u32, resource_id: u32) -> Result<(), Error> {
        self.need(feature::VIRGL)?;
        self.run_one(Command::CtxAttachResource { ctx_id, resource_id }).map(drop)
    }

    /// `CTX_DETACH_RESOURCE`.
    pub fn ctx_detach(&mut self, ctx_id: u32, resource_id: u32) -> Result<(), Error> {
        self.need(feature::VIRGL)?;
        self.run_one(Command::CtxDetachResource { ctx_id, resource_id }).map(drop)
    }

    /// `RESOURCE_CREATE_3D`.
    pub fn create_3d(&mut self, c: &Create3d) -> Result<(), Error> {
        self.need(feature::VIRGL)?;
        if c.resource_id == 0 {
            return Err(Error::BadRect);
        }
        self.run_one(Command::ResourceCreate3d(*c)).map(drop)
    }

    /// `TRANSFER_TO_HOST_3D` (`to_host`) or `TRANSFER_FROM_HOST_3D` in
    /// context `ctx_id`, waited for.
    pub fn transfer_3d(&mut self, to_host: bool, ctx_id: u32, transfer: Transfer3d) -> Result<(), Error> {
        self.need(feature::VIRGL)?;
        let cmd = if to_host {
            Command::TransferToHost3d { ctx_id, transfer }
        } else {
            Command::TransferFromHost3d { ctx_id, transfer }
        };
        self.run_one(cmd).map(drop)
    }

    /// Send one fenced request (on `ring` of its context if given) and
    /// notify, without waiting: [`VirtioGpu::poll`] collects the ticket.
    /// `Busy` when every slot is taken.
    ///
    /// # Safety
    ///
    /// `data` and `response` must be reachable by the device at their bus
    /// addresses and stay valid and unchanged (`response` writable, used by
    /// nothing else) until the ticket is collected or the device reset
    /// (forever, if the driver gives up on the device).
    pub unsafe fn send(
        &mut self,
        cmd: &Command<'_>,
        ring: Option<u8>,
        data: &[Buffer],
        response: Option<DmaRegion>,
    ) -> Result<Ticket, Error> {
        // SAFETY: passed on from our caller.
        let t = unsafe { self.enqueue(cmd, ring, data, response, true) };
        if t.is_ok() {
            self.kick();
        }
        t
    }

    /// Send `cmd` without a fence (and without data) and notify, without
    /// waiting. The device answers once it has carried the request out,
    /// with no fence to signal: for a request the device may finish only
    /// later and never signal the fence of (QEMU 9's `RESOURCE_UNMAP_BLOB`,
    /// answered once its memory map changed). [`VirtioGpu::poll`] collects
    /// the ticket.
    pub fn send_unfenced(&mut self, cmd: &Command<'_>) -> Result<Ticket, Error> {
        // SAFETY: no data buffers and no external response.
        let t = unsafe { self.enqueue(cmd, None, &[], None, false) };
        if t.is_ok() {
            self.kick();
        }
        t
    }

    /// `SUBMIT_3D`: run `size` bytes of context `ctx_id`'s command stream,
    /// the concatenation of `data`, fenced on `ring` if given. Returns at
    /// once; the ticket completes when the host has carried the commands
    /// out (its fence signalled).
    ///
    /// # Safety
    ///
    /// As for [`VirtioGpu::send`]: `data` stays valid until the ticket is
    /// collected.
    pub unsafe fn submit_3d(
        &mut self,
        ctx_id: u32,
        ring: Option<u8>,
        size: u32,
        data: &[Buffer],
    ) -> Result<Ticket, Error> {
        self.need(feature::VIRGL)?;
        if data.iter().map(|b| b.len as u64).sum::<u64>() != size as u64 {
            return Err(Error::BadBacking);
        }
        // SAFETY: passed on from our caller.
        let t = unsafe { self.send(&Command::Submit3d { ctx_id, size }, ring, data, None) }?;
        self.stats.submits += 1;
        Ok(t)
    }

    /// A 3D transfer without waiting (see [`VirtioGpu::transfer_3d`]); the
    /// ticket completes when the copy is done.
    pub fn send_transfer_3d(
        &mut self,
        to_host: bool,
        ctx_id: u32,
        ring: Option<u8>,
        transfer: Transfer3d,
    ) -> Result<Ticket, Error> {
        self.need(feature::VIRGL)?;
        let cmd = if to_host {
            Command::TransferToHost3d { ctx_id, transfer }
        } else {
            Command::TransferFromHost3d { ctx_id, transfer }
        };
        // SAFETY: no data buffers or external response.
        unsafe { self.send(&cmd, ring, &[], None) }
    }

    /// `RESOURCE_CREATE_BLOB` (VIRTIO_GPU_F_RESOURCE_BLOB) in context
    /// `ctx_id` (0 for guest blobs), with `entries` as its guest memory
    /// (for the `GUEST` kinds; none for `HOST3D`).
    pub fn create_blob(&mut self, ctx_id: u32, blob: &CreateBlob, entries: &[MemEntry]) -> Result<(), Error> {
        self.need(feature::RESOURCE_BLOB)?;
        if blob.resource_id == 0 || entries.len() > MAX_BLOB_ENTRIES {
            return Err(Error::BadBacking);
        }
        self.run_one(Command::ResourceCreateBlob { ctx_id, blob: *blob, entries }).map(drop)
    }

    /// `RESOURCE_MAP_BLOB`: map a mappable host blob at `offset` of the
    /// host-visible shared memory region. Returns its `map_info`
    /// ([`crate::protocol::map_cache`]).
    pub fn map_blob(&mut self, resource_id: u32, offset: u64) -> Result<u32, Error> {
        self.need(feature::RESOURCE_BLOB)?;
        match self.run_one(Command::ResourceMapBlob { resource_id, offset })? {
            Response::MapInfo(m) => Ok(m),
            _ => Err(Error::Protocol),
        }
    }

    /// `RESOURCE_UNMAP_BLOB`, waited for. Sent without a fence: QEMU 9
    /// answers it once its memory map changed and never signals a fence
    /// on it (see [`VirtioGpu::send_unfenced`]).
    pub fn unmap_blob(&mut self, resource_id: u32) -> Result<(), Error> {
        self.need(feature::RESOURCE_BLOB)?;
        let mut tickets = [None];
        let sent = self.send_all(&[Command::ResourceUnmapBlob { resource_id }], &mut tickets, false);
        let answered = self.wait_all(&tickets);
        sent?;
        answered.map(drop)
    }
}

/// The most scatter-list entries one `RESOURCE_CREATE_BLOB` carries here.
pub const MAX_BLOB_ENTRIES: usize = (RESPONSE_OFFSET - crate::protocol::CREATE_BLOB_LEN) / MEM_ENTRY_LEN;

/// A 2D resource in guest memory shown on scanouts: what [`VirtioGpu::show`]
/// made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Framebuffer {
    pub resource_id: u32,
    pub width: u32,
    pub height: u32,
    /// Bytes per line of the guest memory.
    pub stride: u32,
}
