//! The driver: device initialisation and synchronous, fenced commands on
//! the control queue.
//!
//! Commands go out one or two at a time (a transfer and its flush share one
//! notification) and the driver polls the used ring for their responses:
//! the kernel console draws with interrupts off, so it cannot sleep until
//! an interrupt says the device is done, and every command here is short.
//! Each request sets VIRTIO_GPU_FLAG_FENCE with a fresh fence ID, so its
//! response comes only once the command is complete (§5.7.6.5), which
//! matters before the memory behind a resource is reused. The response
//! must echo the fence.
//!
//! A device that does not answer within the [`Clock`]'s timeout, or that
//! breaks the queue protocol, still owns the buffers in flight; the driver
//! then refuses every further command ([`Error::Broken`]) and the caller
//! must never reuse the command memory.

use crate::modes::Plan;
use crate::protocol::{
    ATTACH_BACKING_LEN, BYTES_PER_PIXEL, Command, DisplayInfo, EdidBlob, MEM_ENTRY_LEN, MemEntry, RESPONSE_MAX, Rect,
    Response, ResponseError, decode_response, format,
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
/// Commands in flight at once.
const SLOTS: usize = 2;
/// Bytes of command memory per slot: the request, then the response.
const SLOT_LEN: usize = 4096;
/// Where a slot's response starts.
const RESPONSE_OFFSET: usize = 2048;
/// Bytes of command memory the driver needs ([`Setup::commands`]).
pub const COMMAND_MEMORY_LEN: usize = SLOTS * SLOT_LEN;
/// The most scatter-list entries one `RESOURCE_ATTACH_BACKING` can carry
/// here (they must fit a slot's request part).
pub const MAX_BACKING_ENTRIES: usize = (RESPONSE_OFFSET - ATTACH_BACKING_LEN) / MEM_ENTRY_LEN;
/// Queue entries the driver uses at most: two requests of two descriptors.
const QUEUE_ENTRIES: u16 = 4;

const _: () = assert!(RESPONSE_OFFSET + RESPONSE_MAX <= SLOT_LEN);

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
    /// How long one command may take before the device is given up on.
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
    /// The control queue's ring (see [`Layout`], 16-byte aligned).
    pub ring: DmaRegion,
    /// At least [`COMMAND_MEMORY_LEN`] bytes for requests and responses.
    pub commands: DmaRegion,
    /// The control queue's descriptor bookkeeping (4 entries are enough).
    pub desc: &'a mut [DescState],
    pub clock: Clock,
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
    /// More scatter-list entries than [`MAX_BACKING_ENTRIES`], or none.
    BadBacking,
    /// The backing is smaller than the framebuffer it should hold.
    BackingTooSmall { have: u64, need: u64 },
    /// `GET_EDID` without VIRTIO_GPU_F_EDID.
    NoEdid,
    /// A rectangle outside its resource, or a resource of size 0.
    BadRect,
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
}

/// The largest power-of-two queue size, at most `max` (the device's
/// limit), `room` (table entries) and [`QUEUE_ENTRIES`], whose ring fits
/// `ring_len` bytes.
fn queue_size(max: u16, room: usize, ring_len: usize) -> u16 {
    let room = (max as usize).min(room).min(QUEUE_ENTRIES as usize);
    (1..=2)
        .rev()
        .map(|k| 1u16 << k)
        .find(|&s| s as usize <= room && Layout::new(s).is_some_and(|l| l.total() <= ring_len))
        .unwrap_or(0)
}

/// Initialise a virtio GPU device (§3.1.1, §5.7.5) and return the running
/// driver: features negotiated (EDID when offered; no 3D, no blobs), the
/// configuration read and checked, the control queue set up without an
/// interrupt (the driver polls), DRIVER_OK set. The cursor queue stays
/// disabled. On failure the device is left marked FAILED (or, if it
/// refused the reset, untouched).
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
    let Setup { common, device_cfg, device_cfg_len, transport, notify, ring, commands, desc, clock } = setup;
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
    if size < QUEUE_ENTRIES {
        fail!(InitError::NoRoom);
    }
    // SAFETY: our caller vouches for the ring memory; the layout fits in it
    // (the size was chosen so) and the queue is not enabled yet.
    let (queue, notifier) = match unsafe { setup_queue(&cfg, &transport, size, ring, desc) } {
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
    };
    // Polled: ask the device not to interrupt (a hint; no vector anyway).
    gpu.queue.disable_interrupts();
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
) -> Result<(SplitQueue<'a>, Notifier), InitError> {
    let layout = Layout::new(size).ok_or(InitError::NoRoom)?;
    let event_idx = cfg.has(poly_virtio::feature::EVENT_IDX);
    // SAFETY: passed on from our caller.
    let queue = unsafe { SplitQueue::new(ring.ptr, ring.bus, layout, desc, event_idx) }.map_err(InitError::Queue)?;
    let notify_off = cfg.enable_queue(CONTROL_QUEUE, size, queue.addresses(), None)?;
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
// and must be `Send` themselves.
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

    fn slot(&mut self, i: usize, offset: usize, len: usize) -> (&mut [u8], u64) {
        let at = i * SLOT_LEN + offset;
        debug_assert!(at + len <= self.cmd.len);
        // SAFETY: `init` checked that the command memory holds every slot;
        // it is the driver's alone (the device only touches a slot between
        // `add` and the used element, and `run` is the only user, never
        // holding two views of one slot part).
        let bytes = unsafe { core::slice::from_raw_parts_mut(self.cmd.ptr.as_ptr().add(at), len) };
        (bytes, self.cmd.bus + at as u64)
    }

    /// Send `cmds` (one or two) in one notification and wait for their
    /// responses. Returns the last command's response, or the first error.
    fn run(&mut self, cmds: &[Command<'_>]) -> Result<Response, Error> {
        if self.broken {
            return Err(Error::Broken);
        }
        assert!(!cmds.is_empty() && cmds.len() <= SLOTS, "one or two commands at a time");
        let mut heads = [None; SLOTS];
        let mut fences = [0u64; SLOTS];
        let mut buffers = [(Buffer { addr: 0, len: 0 }, Buffer { addr: 0, len: 0 }); SLOTS];
        // Every request is written before any goes to the device, so one
        // that does not fit sends none.
        for (i, c) in cmds.iter().enumerate() {
            let fence = self.next_fence + i as u64;
            fences[i] = fence;
            let (req, req_bus) = self.slot(i, 0, RESPONSE_OFFSET);
            let len = c.encode(Some(fence), req).ok_or(Error::BadBacking)?;
            let (resp, resp_bus) = self.slot(i, RESPONSE_OFFSET, RESPONSE_MAX);
            // A device that answers without writing is caught as a bad type.
            resp[..crate::protocol::HEADER_LEN].fill(0);
            buffers[i] =
                (Buffer { addr: req_bus, len: len as u32 }, Buffer { addr: resp_bus, len: c.response_len() as u32 });
        }
        self.next_fence += cmds.len() as u64;
        for (i, (r, w)) in buffers[..cmds.len()].iter().enumerate() {
            heads[i] = Some(self.queue.add(&[*r], &[*w]).map_err(|e| match e {
                // Never: the queue holds both slots and nothing else is in
                // flight between calls.
                AddError::Full | AddError::TooLong | AddError::NoBuffers => Error::Protocol,
            })?);
            self.stats.commands += 1;
        }
        if self.queue.should_notify() {
            self.notifier.notify(&self.notify);
            self.stats.notifies += 1;
        }
        let mut lens = [None; SLOTS];
        let started = (self.clock.now_ns)();
        let mut done = 0;
        while done < cmds.len() {
            match self.queue.pop_used() {
                Ok(Some(u)) => {
                    let Some(i) = heads[..cmds.len()].iter().position(|h| *h == Some(u.head)) else {
                        return Err(self.give_up(Error::Protocol));
                    };
                    if lens[i].replace(u.len as usize).is_some() {
                        return Err(self.give_up(Error::Protocol));
                    }
                    done += 1;
                }
                Ok(None) => {
                    if (self.clock.now_ns)().wrapping_sub(started) > self.clock.timeout_ns {
                        let pending = lens.iter().position(Option::is_none).unwrap_or(0);
                        return Err(self.give_up(Error::Timeout { request: cmds[pending].ty() }));
                    }
                    core::hint::spin_loop();
                }
                Err(e) => return Err(self.give_up(Error::Queue(e))),
            }
        }
        let mut result = Ok(Response::NoData);
        for (i, c) in cmds.iter().enumerate() {
            let (resp, _) = self.slot(i, RESPONSE_OFFSET, RESPONSE_MAX);
            match decode_response(c, Some(fences[i]), resp, lens[i].unwrap_or(0)) {
                Ok(r) => {
                    if result.is_ok() {
                        result = Ok(r);
                    }
                }
                Err(e) => {
                    self.stats.errors += 1;
                    if result.is_ok() {
                        result = Err(Error::Response(e));
                    }
                }
            }
        }
        result
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
}

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
