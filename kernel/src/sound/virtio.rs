//! virtio-sound over PCI: the kernel's side of the `poly-virtio-sound`
//! driver, as `net/virtio.rs` is for virtio-net (VIRTIO 1.2 §3.1.1 up to
//! where [`poly_virtio_sound::init`] takes over), and the device as the
//! audio server's hardware ([`VirtioHw`], ADR 0109):
//!
//! - finds the function (vendor 0x1af4, virtio device type 25; QEMU's
//!   `virtio-sound-pci` is PCI device 0x1059), locates its structures and
//!   maps them;
//! - turns on memory decoding, resets the device and only then bus
//!   mastering;
//! - gives it DMA memory ([`DmaBuffer`]): the four queue rings, a page for
//!   control requests, the event buffers, the per-message headers and
//!   status of both directions, and the PCM memory ([`PCM_BYTES`]: output
//!   ring in the lower half, input periods in the upper); bookkeeping
//!   tables come from the heap. Everything lives as long as the kernel.
//!
//! With MSI-X the function is enabled masked before `init`, which maps the
//! event, transmit and receive queues and configuration changes to table
//! entry 0; [`connect_interrupt`] later points it at the audio thread's
//! wakeup. A device that refuses the vector is brought up again without it
//! and is then polled.
//!
//! [`VirtioHw`] drives one output and one input stream in the fixed
//! formats the server mixes in: whole periods go out through the period
//! ring (`poly_virtio_sound::ring`), captured periods come back on the
//! receive queue and are lent again once copied out.

use super::{HwFormat, patience};
use crate::dma::DmaBuffer;
use crate::pci::{self, Device, IrqError, MapError, MmioRegion, MsixVectors};
use crate::{kprintln, thread, time};
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use poly_audio::{Format, Sample};
use poly_pci::{Bdf, command};
use poly_virtio::{CommonCfg, DescState, InitError, Layout, PciTransport, Region, TransportError, device_id};
use poly_virtio_sound::proto::{ChmapInfo, Event, JackInfo, PcmInfo, SetParams, code, direction, format as vfmt};
use poly_virtio_sound::ring::{Chunk, PeriodRing};
use poly_virtio_sound::stream::{Command, StreamState, Wanted, check_params};
use poly_virtio_sound::{DmaRegion, Memory, RX_SLOT_LEN, Setup, Storage, TX_SLOT_LEN, TxError};

/// The driver as the kernel instantiates it.
pub type Driver = poly_virtio_sound::VirtioSound<'static, MmioRegion, MmioRegion>;

/// Entries per queue the driver gets memory for (fewer if the device
/// offers fewer): 21 messages of three descriptors each per direction.
const QUEUE_ENTRIES: u16 = 64;
/// The PCM memory: output ring in the lower half, input periods in the
/// upper.
pub const PCM_BYTES: usize = 256 * 1024;
const IN_BASE: u32 = (PCM_BYTES / 2) as u32;
/// Status reads after a reset before giving up on the device.
const RESET_POLLS: u32 = 1_000_000;
/// The MSI-X table entry the queues and configuration changes use.
pub const MSIX_ENTRY: u16 = 0;
/// The most items of each kind read from the device.
const MAX_ITEMS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Transport(TransportError),
    Bar(MapError),
    NoDeviceConfig,
    Reset(InitError),
    OutOfMemory,
    Init(poly_virtio_sound::InitError),
}

fn map_region(dev: &Device, r: Region) -> Result<MmioRegion, Error> {
    let bar = dev.map_bar(r.bar as usize).map_err(Error::Bar)?;
    bar.subregion(r.offset as usize, r.length as usize).ok_or(Error::Transport(TransportError::OutsideBar))
}

fn region(b: &DmaBuffer) -> DmaRegion {
    DmaRegion { ptr: b.ptr(), bus: b.phys(), len: b.len() }
}

/// A table that lives as long as the kernel (the driver keeps it).
fn table<T: Clone>(entry: T) -> &'static mut [T] {
    Box::leak(vec![entry; QUEUE_ENTRIES as usize].into_boxed_slice())
}

/// Bring the device up and hand it to `poly-virtio-sound`, with its queues
/// on MSI-X entry `vector` if given.
fn bring_up(dev: &Device, vector: Option<u16>) -> Result<Driver, Error> {
    let cfg = dev.config();
    let bdf = dev.bdf();
    let t = PciTransport::discover(cfg, &dev.header).map_err(Error::Transport)?;
    t.check_bars(&dev.bars).map_err(Error::Transport)?;
    let device_cfg = map_region(dev, t.device.ok_or(Error::NoDeviceConfig)?)?;
    let common = map_region(dev, t.common)?;
    let notify = map_region(dev, t.notify)?;

    // Decoding on and INTx off first, bus mastering only after a reset made
    // the device forget anything firmware set up.
    poly_pci::enable(cfg, bdf, command::MEMORY_SPACE | command::INTX_DISABLE);
    CommonCfg::new(common).reset(RESET_POLLS).map_err(Error::Reset)?;
    poly_pci::enable(cfg, bdf, command::BUS_MASTER);

    let layout = Layout::new(QUEUE_ENTRIES).unwrap_or_else(|| unreachable!("a valid queue size"));
    let mut memory = Vec::new();
    for len in [
        layout.total(),
        layout.total(),
        layout.total(),
        layout.total(),
        4096,
        QUEUE_ENTRIES as usize * Event::LEN,
        QUEUE_ENTRIES as usize * TX_SLOT_LEN,
        QUEUE_ENTRIES as usize * RX_SLOT_LEN,
        PCM_BYTES,
    ] {
        memory.push(DmaBuffer::new(len).ok_or(Error::OutOfMemory)?);
    }
    let regions: Vec<DmaRegion> = memory.iter().map(region).collect();
    // If bring-up fails these tables leak; it happens once, at boot.
    let storage = Storage {
        desc: [
            table(DescState::default()),
            table(DescState::default()),
            table(DescState::default()),
            table(DescState::default()),
        ],
        event_owner: table(0u16),
        tx_owner: table(0u16),
        tx_free: table(0u16),
        tx_len: table(0u32),
        rx_owner: table(0u16),
        rx_free: table(0u16),
        rx_len: table(0u32),
    };
    let setup = Setup {
        common: CommonCfg::new(common),
        device_cfg,
        device_cfg_len: device_cfg.len() as u32,
        transport: t,
        notify,
        memory: Memory {
            rings: [regions[0], regions[1], regions[2], regions[3]],
            control: regions[4],
            events: regions[5],
            tx_meta: regions[6],
            rx_meta: regions[7],
            pcm: regions[8],
        },
        storage,
        vector,
    };
    // SAFETY: the DMA buffers are fresh, zeroed, direct-mapped blocks of
    // frames that the device reaches at their physical addresses (no
    // IOMMU); on success they are leaked below, so they stay valid and used
    // by nothing else for the kernel's lifetime ('static). The register
    // regions all come from this one function's capabilities.
    match unsafe { poly_virtio_sound::init(setup) } {
        Ok(mut driver) => {
            memory.into_iter().for_each(core::mem::forget);
            driver.disable_interrupts();
            Ok(driver)
        }
        Err(e) => {
            // The device may know the queues' addresses by now: take the
            // memory back with a reset, or never reuse it.
            if CommonCfg::new(common).reset(RESET_POLLS).is_err() {
                memory.into_iter().for_each(core::mem::forget);
            }
            Err(Error::Init(e))
        }
    }
}

pub fn is_virtio_sound(d: &Device) -> bool {
    poly_virtio::device_id_of(&d.header) == Some(device_id::SOUND)
}

/// A virtio-sound function brought up by [`probe`].
#[derive(Debug)]
pub struct Found {
    pub bdf: Bdf,
    pub driver: Driver,
    /// MSI-X, on but masked, with everything on entry 0; `None` if the
    /// device must be polled.
    pub msix: Option<MsixVectors>,
}

/// Find the first virtio-sound function and bring it up. Logs what it
/// found; `None` if there is none or it cannot be used.
pub fn probe() -> Option<Found> {
    let mut found = pci::devices().iter().filter(|d| is_virtio_sound(d));
    let dev = found.next()?;
    for other in found {
        kprintln!("sound: {}: ignored, only one virtio-sound device is driven for now", other.bdf());
    }
    let bdf = dev.bdf();
    let mut msix = match dev.msix() {
        Some(Ok(_)) => match dev.enable_msix() {
            Ok(v) => Some(v),
            Err(e) => {
                kprintln!("sound: {bdf}: cannot enable MSI-X ({e:?}); polling");
                None
            }
        },
        Some(Err(e)) => {
            kprintln!("sound: {bdf}: MSI-X capability unusable ({e:?}); polling");
            None
        }
        None => None,
    };
    let mut result = bring_up(dev, msix.as_ref().map(|_| MSIX_ENTRY));
    if let Err(e) = &result
        && let Some(m) = msix.take()
    {
        kprintln!("sound: {bdf}: bring-up with MSI-X failed ({e:?}); trying again, polled");
        m.disable();
        result = bring_up(dev, None);
    }
    match result {
        Ok(driver) => Some(Found { bdf, driver, msix }),
        Err(e) => {
            kprintln!("sound: {bdf}: bring-up failed: {e:?}");
            if let Some(m) = msix {
                m.disable();
            }
            None
        }
    }
}

/// Point MSI-X entry 0 at `handler(data)` and let the function interrupt.
/// Returns the kernel vector.
pub fn connect_interrupt(
    msix: &mut MsixVectors,
    handler: crate::arch::irq::Handler,
    data: usize,
) -> Result<u8, IrqError> {
    let vector = msix.attach(MSIX_ENTRY, handler, data)?;
    msix.set_function_masked(false);
    Ok(vector)
}

// ---- the device as the server's hardware ------------------------------------

/// virtio sample formats the server can mix in, best first, and what they
/// are to it.
const SAMPLES: [(u8, Sample); 6] = [
    (vfmt::S16, Sample::S16),
    (vfmt::S32, Sample::S32),
    (vfmt::FLOAT, Sample::Float),
    (vfmt::S24, Sample::S24),
    (vfmt::S24_3, Sample::S24_3),
    (vfmt::U8, Sample::U8),
];

/// Hardware periods: 20 ms, four of them in flight.
const PERIOD_MS: u32 = 20;
const PERIODS: u32 = 4;

/// The fixed format the server uses on stream `info`: S16 (else the next
/// best it offers), stereo where allowed, 48 kHz (else 44.1 kHz, else its
/// fastest rate); 20 ms periods.
fn choose(info: &PcmInfo, base: u32) -> Option<(SetParams, HwFormat)> {
    let (vf, sample) = SAMPLES.iter().copied().find(|(f, _)| info.has_format(*f))?;
    let channels = 2u8.clamp(info.channels_min, info.channels_max);
    let rates: Vec<u32> = info.rates_hz().collect();
    let rate = [48000, 44100].into_iter().find(|r| rates.contains(r)).or_else(|| rates.iter().copied().max())?;
    let fmt = Format::new(sample, channels, rate);
    let period_frames = rate * PERIOD_MS / 1000;
    let period_bytes = period_frames * fmt.frame_bytes() as u32;
    let want = Wanted { format: vf, rate_hz: rate, channels, period_bytes, buffer_bytes: period_bytes * PERIODS };
    if want.buffer_bytes > IN_BASE {
        return None;
    }
    let params = check_params(info, &want).ok()?;
    let _ = base;
    Some((params, HwFormat { format: fmt, period_frames, periods: PERIODS }))
}

/// One stream the server uses.
#[derive(Debug)]
struct VStream {
    id: u32,
    params: SetParams,
    hw: HwFormat,
    state: StreamState,
    /// Where its bytes start in the PCM memory.
    base: u32,
    ring: PeriodRing,
    started: bool,
}

impl VStream {
    fn period_bytes(&self) -> u32 {
        self.params.period_bytes
    }
}

/// The virtio-sound device as the audio server's hardware.
#[derive(Debug)]
pub struct VirtioHw {
    pub bdf: Bdf,
    pub driver: Driver,
    out: Option<VStream>,
    inp: Option<VStream>,
    /// Filled input periods: offset in the PCM memory and length.
    filled: VecDeque<(u32, u32)>,
    /// Output messages that came back with an error status.
    pub errors: u64,
}

impl VirtioHw {
    /// Query the device, log what it has, and pick the first usable output
    /// and input streams.
    pub fn new(bdf: Bdf, mut driver: Driver) -> VirtioHw {
        let config = driver.config();
        kprintln!(
            "sound: {bdf}: virtio-sound, {} jacks, {} streams, {} channel maps; queues {:?}, {} output and {} input messages in flight",
            config.jacks,
            config.streams,
            config.chmaps,
            driver.queue_sizes(),
            driver.tx_slots(),
            driver.rx_slots()
        );
        let mut jacks = [JackInfo::default(); MAX_ITEMS];
        let mut infos = [PcmInfo::default(); MAX_ITEMS];
        let mut maps = [ChmapInfo::default(); MAX_ITEMS];
        let nj = driver.query_jacks(&mut jacks, &mut patience()).unwrap_or_else(|e| {
            kprintln!("sound: {bdf}: jack query failed: {e:?}");
            0
        });
        let ns = driver.query_streams(&mut infos, &mut patience()).unwrap_or_else(|e| {
            kprintln!("sound: {bdf}: stream query failed: {e:?}");
            0
        });
        let nm = driver.query_chmaps(&mut maps, &mut patience()).unwrap_or_else(|e| {
            kprintln!("sound: {bdf}: channel map query failed: {e:?}");
            0
        });
        if driver.stats().short_responses > 0 {
            kprintln!(
                "sound: {bdf}: the device reports control responses shorter than it writes; taken by their status"
            );
        }
        for (i, j) in jacks[..nj].iter().enumerate() {
            kprintln!(
                "sound: jack {i}: {}, defconf {:#x}, features {:#x}",
                if j.connected { "connected" } else { "not connected" },
                j.hda_reg_defconf,
                j.features
            );
        }
        let mut out = None;
        let mut inp = None;
        for (id, info) in infos[..ns].iter().enumerate() {
            let rates: Vec<u32> = info.rates_hz().collect();
            kprintln!(
                "sound: stream {id}: {}, {}-{} channels, rates {:?}, formats {:#x}, features {:#x}",
                direction_name(info.direction),
                info.channels_min,
                info.channels_max,
                rates,
                info.formats,
                info.features
            );
            if !info.is_usable() {
                kprintln!("sound: stream {id}: not usable; left out");
                continue;
            }
            let (slot, base) = if info.direction == direction::OUTPUT { (&mut out, 0) } else { (&mut inp, IN_BASE) };
            if slot.is_some() {
                kprintln!("sound: stream {id}: the server uses one stream per direction; left out");
                continue;
            }
            match choose(info, base) {
                Some((params, hw)) => {
                    let ring = PeriodRing::new(params.buffer_bytes, params.period_bytes)
                        .unwrap_or_else(|| unreachable!("whole periods"));
                    *slot = Some(VStream {
                        id: id as u32,
                        params,
                        hw,
                        state: StreamState::Initial,
                        base,
                        ring,
                        started: false,
                    });
                }
                None => kprintln!("sound: stream {id}: no format the server can use; left out"),
            }
        }
        for (i, m) in maps[..nm].iter().enumerate() {
            kprintln!("sound: channel map {i}: {}, positions {:?}", direction_name(m.direction), m.used_positions());
        }
        VirtioHw { bdf, driver, out, inp, filled: VecDeque::new(), errors: 0 }
    }

    pub fn output(&self) -> Option<HwFormat> {
        self.out.as_ref().map(|s| s.hw)
    }

    pub fn input(&self) -> Option<HwFormat> {
        self.inp.as_ref().map(|s| s.hw)
    }

    fn goto(driver: &mut Driver, s: &mut VStream, goal: Command) -> bool {
        let params = s.params;
        match driver.pcm_goto(&mut s.state, s.id, goal, Some(&params), &mut patience()) {
            Ok(()) => true,
            Err(e) => {
                kprintln!("sound: stream {}: {goal:?} failed: {e:?}", s.id);
                false
            }
        }
    }

    /// Collect returned messages and events.
    pub fn collect(&mut self) {
        let VirtioHw { driver, out, inp, filled, errors, .. } = self;
        driver.poll_events(|e| match e.code {
            code::EVT_PCM_XRUN => kprintln!("sound: stream {}: the device reports an xrun", e.data),
            code::EVT_JACK_CONNECTED | code::EVT_JACK_DISCONNECTED => kprintln!(
                "sound: jack {} {}",
                e.data,
                if e.code == code::EVT_JACK_CONNECTED { "connected" } else { "disconnected" }
            ),
            _ => {}
        });
        driver.poll_tx(|d| {
            if d.status.status != poly_virtio_sound::proto::status::OK {
                *errors += 1;
            }
            if let Some(s) = out.as_mut()
                && s.id == d.stream
            {
                s.ring.complete(d.len);
            }
        });
        driver.poll_rx(|d| {
            // Returned by a RELEASE (empty), or with an error: not lent again.
            if let Some(s) = inp.as_ref()
                && s.id == d.stream
                && s.started
                && d.len > 0
                && d.status.status == poly_virtio_sound::proto::status::OK
            {
                filled.push_back((d.offset, d.len));
            }
        });
    }

    // ---- output ----

    /// Ready the output stream to take periods from its start.
    pub fn out_start(&mut self) -> bool {
        self.out_halt();
        let Some(s) = self.out.as_mut() else { return false };
        Self::goto(&mut self.driver, s, Command::SetParams) && Self::goto(&mut self.driver, s, Command::Prepare)
    }

    /// Whole periods the device can take now.
    pub fn out_room(&self) -> usize {
        self.out
            .as_ref()
            .filter(|s| s.state == StreamState::Prepared || s.started)
            .map_or(0, |s| (s.ring.room() / s.period_bytes()) as usize)
    }

    /// One period of frames (exactly a period long).
    pub fn out_push(&mut self, period: &[u8]) {
        let Some(s) = self.out.as_mut() else { return };
        let [span, _] = s.ring.write_spans(period.len() as u32);
        if span.len as usize != period.len() {
            return;
        }
        if self.driver.pcm_write((s.base + span.offset) as usize, period) {
            s.ring.commit(span.len);
        }
    }

    /// Start the stream once it has periods, and hand them to the device.
    pub fn out_kick(&mut self) {
        let Some(s) = self.out.as_mut() else { return };
        if !s.started && s.ring.pending() > 0 {
            if !Self::goto(&mut self.driver, s, Command::Start) {
                return;
            }
            s.started = true;
        }
        while s.started && self.driver.tx_in_flight() < self.driver.tx_slots() {
            let Some(c) = s.ring.next_chunk(false) else { break };
            match self.driver.submit(s.id, Chunk { offset: s.base + c.offset, len: c.len }) {
                Ok(()) => s.ring.mark_submitted(c),
                Err(TxError::Busy) => break,
                Err(e) => {
                    kprintln!("sound: stream {}: output message refused: {e:?}", s.id);
                    break;
                }
            }
        }
    }

    /// Stop and release the output stream, waiting for every message to
    /// come back (§5.14.6.6.5: RELEASE completes them).
    fn out_halt(&mut self) {
        let Some(s) = self.out.as_mut() else { return };
        if matches!(s.state, StreamState::Started | StreamState::Stopped | StreamState::Prepared) {
            Self::goto(&mut self.driver, s, Command::Release);
        }
        let deadline = time::now_ns() + super::CONTROL_TIMEOUT_NS;
        while self.driver.tx_in_flight() > 0 && !self.driver.is_failed() {
            self.driver.poll_tx(|_| {});
            if self.driver.tx_in_flight() == 0 {
                break;
            }
            if time::now_ns() >= deadline {
                kprintln!(
                    "sound: stream {}: {} output messages not returned after release",
                    s.id,
                    self.driver.tx_in_flight()
                );
                break;
            }
            thread::yield_now();
        }
        s.ring.reset();
        s.started = false;
    }

    pub fn out_stop(&mut self) {
        self.out_halt();
    }

    // ---- input ----

    /// Set the input stream up, lend it every period and start it.
    pub fn in_start(&mut self) -> bool {
        self.in_stop();
        let Some(s) = self.inp.as_mut() else { return false };
        if !(Self::goto(&mut self.driver, s, Command::SetParams) && Self::goto(&mut self.driver, s, Command::Prepare)) {
            return false;
        }
        let pb = s.period_bytes();
        for i in 0..s.hw.periods {
            if let Err(e) = self.driver.submit_rx(s.id, Chunk { offset: s.base + i * pb, len: pb }) {
                kprintln!("sound: stream {}: input message refused: {e:?}", s.id);
                break;
            }
        }
        s.started = Self::goto(&mut self.driver, s, Command::Start);
        s.started
    }

    /// The next captured period, copied into `out`; its buffer is lent to
    /// the device again. Returns its length.
    pub fn in_pop(&mut self, out: &mut [u8]) -> Option<usize> {
        let s = self.inp.as_mut()?;
        let (offset, len) = self.filled.pop_front()?;
        let n = (len as usize).min(out.len());
        let ok = self.driver.pcm_read(offset as usize, &mut out[..n]);
        if s.started {
            let _ = self.driver.submit_rx(s.id, Chunk { offset, len: s.period_bytes() });
        }
        ok.then_some(n)
    }

    /// Stop and release the input stream and take back its buffers.
    pub fn in_stop(&mut self) {
        let Some(s) = self.inp.as_mut() else { return };
        s.started = false;
        if matches!(s.state, StreamState::Started | StreamState::Stopped | StreamState::Prepared) {
            Self::goto(&mut self.driver, s, Command::Release);
        }
        let deadline = time::now_ns() + super::CONTROL_TIMEOUT_NS;
        while self.driver.rx_in_flight() > 0 && !self.driver.is_failed() && time::now_ns() < deadline {
            self.driver.poll_rx(|_| {});
            thread::yield_now();
        }
        self.filled.clear();
    }

    pub fn enable_interrupts(&mut self) -> bool {
        self.driver.enable_interrupts()
    }

    pub fn is_failed(&self) -> bool {
        self.driver.is_failed()
    }
}

fn direction_name(d: u8) -> &'static str {
    match d {
        direction::OUTPUT => "output",
        direction::INPUT => "input",
        _ => "unknown",
    }
}
