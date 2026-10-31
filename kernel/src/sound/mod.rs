//! Sound (ADR 0104): the virtio-sound device as ALSA card 0.
//!
//! ```text
//!  /dev/snd/pcmC0D<n>p ─┐ ioctl / write / poll            MSI-X entry 0
//!  /dev/snd/controlC0  ─┤ (alsa.rs)                             │ on_interrupt: WAKE += 1, wake the thread
//!                       ▼                                       ▼
//!          Card { Mutex<Inner { Driver, [PcmSlot] }> } ◄── audio thread: returned messages advance the
//!                       │ PcmRuntime (personality)               ring and the runtime's hw pointer, the next
//!                       │ over Glue: PcmBackend                  periods go out, waiters are woken
//!                       ▼
//!          poly-virtio-sound: control requests, output messages, events
//! ```
//!
//! - **The device** is found and brought up by [`virtio`]; its jacks,
//!   streams and channel maps are queried once and logged. Every output
//!   stream becomes a PCM playback device whose number is the stream id
//!   (`/dev/snd/pcmC0D0p` for QEMU's stream 0); input streams are listed
//!   but not opened yet (no capture).
//! - **The ALSA logic** (hardware parameter refinement, states, pointers,
//!   thresholds) is the host-tested `poly_personality_linux::sound`; the
//!   device side of each stream ([`Glue`]) maps its calls onto the virtio
//!   command lifecycle and the period ring (`poly_virtio_sound::ring`).
//! - **The audio thread** collects returned output messages and events,
//!   when the device interrupts or, without MSI-X, every
//!   [`POLL_INTERVAL_NS`] while a stream plays. Each returned message is a
//!   period played: the ring and the runtime advance, the next periods go
//!   to the device, an empty ring is an underrun, and writers and pollers
//!   on the card's wait queue are woken.
//! - **Control requests** are polled for while the card is locked (the
//!   device answers them at once; [`CONTROL_TIMEOUT_NS`] at most).

pub mod alsa;
mod tests;
mod virtio;

use crate::boot_info::BootInfo;
use crate::pci::MsixVectors;
use crate::sync::{Mutex, SetOnce};
use crate::thread::{self, WaitQueue as ThreadQueue};
use crate::{kprintln, time};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use poly_pci::Bdf;
use poly_personality_linux::errno::Errno;
use poly_personality_linux::sound::abi::{self, Timespec};
use poly_personality_linux::sound::hw::{Caps, HwConfig, Mask};
use poly_personality_linux::sound::pcm::{PcmBackend, PcmRuntime};
use poly_virtio_sound::proto::{ChmapInfo, JackInfo, PcmInfo, code, direction, format as vfmt};
use poly_virtio_sound::ring::PeriodRing;
use poly_virtio_sound::stream::{Command, StreamState, Wanted, check_params};
use poly_virtio_sound::{CtlError, TxError};

pub use virtio::Driver;

/// How long a control request may take.
pub const CONTROL_TIMEOUT_NS: u64 = time::NANOS_PER_SEC;
/// How often the audio thread polls a device without interrupts while a
/// stream plays (and, as a safety net, one with them).
pub const POLL_INTERVAL_NS: u64 = 2 * time::NANOS_PER_MILLI;
const SAFETY_POLL_NS: u64 = 50 * time::NANOS_PER_MILLI;
/// The most items of each kind read from the device.
const MAX_ITEMS: usize = 16;
/// The ALSA card number.
pub const CARD: u32 = 0;

/// virtio format codes and the ALSA formats they are (§5.14.6.6.2 against
/// `sound/asound.h`; all little-endian).
const FORMATS: [(u8, u32); 24] = [
    (vfmt::MU_LAW, abi::format::MU_LAW),
    (vfmt::A_LAW, abi::format::A_LAW),
    (vfmt::S8, abi::format::S8),
    (vfmt::U8, abi::format::U8),
    (vfmt::S16, abi::format::S16_LE),
    (vfmt::U16, abi::format::U16_LE),
    (vfmt::S18_3, abi::format::S18_3LE),
    (vfmt::U18_3, abi::format::U18_3LE),
    (vfmt::S20_3, abi::format::S20_3LE),
    (vfmt::U20_3, abi::format::U20_3LE),
    (vfmt::S24_3, abi::format::S24_3LE),
    (vfmt::U24_3, abi::format::U24_3LE),
    (vfmt::S20, abi::format::S20_LE),
    (vfmt::U20, abi::format::U20_LE),
    (vfmt::S24, abi::format::S24_LE),
    (vfmt::U24, abi::format::U24_LE),
    (vfmt::S32, abi::format::S32_LE),
    (vfmt::U32, abi::format::U32_LE),
    (vfmt::FLOAT, abi::format::FLOAT_LE),
    (vfmt::FLOAT64, abi::format::FLOAT64_LE),
    (vfmt::DSD_U8, abi::format::DSD_U8),
    (vfmt::DSD_U16, abi::format::DSD_U16_LE),
    (vfmt::DSD_U32, abi::format::DSD_U32_LE),
    (vfmt::IEC958_SUBFRAME, abi::format::IEC958_SUBFRAME_LE),
];

fn alsa_format(v: u8) -> Option<u32> {
    FORMATS.iter().find(|f| f.0 == v).map(|f| f.1)
}

fn virtio_format(a: u32) -> Option<u8> {
    FORMATS.iter().find(|f| f.1 == a).map(|f| f.0)
}

/// What ALSA may ask of a stream: its formats, rates and channels, and
/// period and buffer sizes the PCM memory and the transmit queue can
/// carry (a buffer of `periods` periods needs `periods + 1` messages in
/// flight: see `PeriodRing`).
fn caps_of(info: &PcmInfo, tx_slots: u16, pcm_bytes: usize) -> Caps {
    let mut formats = Mask::default();
    for f in info.usable_formats().filter_map(alsa_format) {
        formats.set(f);
    }
    let mut rates = [0u32; 16];
    let mut rate_count = 0;
    for r in info.rates_hz().take(16) {
        rates[rate_count] = r;
        rate_count += 1;
    }
    let pcm = pcm_bytes.min(u32::MAX as usize) as u32;
    Caps {
        formats,
        rates,
        rate_count,
        channels_min: info.channels_min as u32,
        channels_max: info.channels_max as u32,
        period_bytes_min: 64,
        period_bytes_max: pcm / 2,
        periods_min: 2,
        periods_max: (tx_slots.saturating_sub(1) as u32).clamp(2, 32),
        buffer_bytes_max: pcm,
    }
}

/// A PCM device of the card, as user space sees it.
#[derive(Debug)]
pub struct PcmDesc {
    /// The ALSA device number (the virtio stream id).
    pub device: u32,
    pub caps: Caps,
    pub name: String,
}

/// The device side of one stream.
#[derive(Debug)]
pub struct DevStream {
    id: u32,
    info: PcmInfo,
    state: StreamState,
    ring: Option<PeriodRing>,
    frame_bytes: u32,
    running: bool,
    draining: bool,
    silence: ([u8; 8], usize),
    /// Output messages that came back with an error status.
    pub errors: u64,
}

/// One playback substream: its runtime and its device side.
#[derive(Debug)]
pub struct PcmSlot {
    pub open: bool,
    pub rt: PcmRuntime,
    pub dev: DevStream,
}

/// The card's mutable state, behind its lock.
#[derive(Debug)]
pub struct Inner {
    pub driver: Driver,
    pub pcms: Vec<PcmSlot>,
}

/// The sound card.
#[derive(Debug)]
pub struct Card {
    pub bdf: Bdf,
    pub inner: Mutex<Inner>,
    /// Woken (from the audio thread) whenever a stream's readiness may
    /// have changed.
    pub queue: Arc<poly_vfs::WaitQueue>,
    /// One per playback stream, in the order of `Inner::pcms`.
    pub pcms: Vec<PcmDesc>,
    pub longname: String,
}

impl Card {
    /// Run `f` on playback slot `i` with its device side as a backend.
    pub fn with_pcm<R>(&self, i: usize, f: impl FnOnce(&mut PcmRuntime, &mut Glue<'_>) -> R) -> R {
        let mut g = self.inner.lock();
        let Inner { driver, pcms } = &mut *g;
        let slot = &mut pcms[i];
        let mut glue = Glue { driver, dev: &mut slot.dev };
        f(&mut slot.rt, &mut glue)
    }

    /// Wake everyone waiting on a stream of this card.
    pub fn notify(&self) {
        self.queue.notify(poly_vfs::PollEvents::WRITABLE | poly_vfs::PollEvents::ERR);
    }
}

/// A `wait` for control requests: yield until the deadline.
fn patience() -> impl FnMut() -> bool {
    let deadline = time::now_ns() + CONTROL_TIMEOUT_NS;
    move || {
        if time::now_ns() >= deadline {
            false
        } else {
            thread::yield_now();
            true
        }
    }
}

fn ctl_errno(e: CtlError) -> Errno {
    match e {
        CtlError::Status(code) if code == poly_virtio_sound::proto::status::NOT_SUPP => Errno::EINVAL,
        CtlError::State => Errno::EBADFD,
        _ => Errno::EIO,
    }
}

/// The monotonic clock as a timespec.
pub fn monotonic() -> Timespec {
    Timespec::from_ns(time::now_ns())
}

/// A stream's device side as the ALSA runtime's backend.
pub struct Glue<'a> {
    pub driver: &'a mut Driver,
    pub dev: &'a mut DevStream,
}

impl Glue<'_> {
    fn goto(&mut self, goal: Command, params: Option<&poly_virtio_sound::proto::SetParams>) -> Result<(), Errno> {
        self.driver.pcm_goto(&mut self.dev.state, self.dev.id, goal, params, &mut patience()).map_err(ctl_errno)
    }

    /// Hand the device what the ring has ready, as long as there are
    /// message slots.
    fn pump(&mut self) {
        let Some(ring) = self.dev.ring.as_mut() else { return };
        while self.driver.tx_in_flight() < self.driver.tx_slots() {
            let Some(c) = ring.next_chunk(self.dev.draining) else { break };
            match self.driver.submit(self.dev.id, c) {
                Ok(()) => ring.mark_submitted(c),
                Err(TxError::Busy) => break,
                Err(e) => {
                    kprintln!("sound: stream {}: output message refused: {e:?}", self.dev.id);
                    break;
                }
            }
        }
    }

    /// Stop the stream if it plays and take back every message (§5.14.6.6.5:
    /// RELEASE completes them), so the ring can be reused.
    fn halt(&mut self) {
        if matches!(self.dev.state, StreamState::Started | StreamState::Stopped)
            && let Err(e) = self.goto(Command::Release, None)
        {
            kprintln!("sound: stream {}: stop/release failed: {e:?}", self.dev.id);
        }
        let deadline = time::now_ns() + CONTROL_TIMEOUT_NS;
        while self.driver.tx_in_flight() > 0 && !self.driver.is_failed() {
            // Messages of this stream only: the card drives one stream at
            // a time (ADR 0104).
            self.driver.poll_tx(|_| {});
            if self.driver.tx_in_flight() == 0 {
                break;
            }
            if time::now_ns() >= deadline {
                kprintln!(
                    "sound: stream {}: {} output messages not returned after release",
                    self.dev.id,
                    self.driver.tx_in_flight()
                );
                break;
            }
            thread::yield_now();
        }
        if let Some(r) = self.dev.ring.as_mut() {
            r.reset();
        }
        self.dev.running = false;
        self.dev.draining = false;
    }
}

impl PcmBackend for Glue<'_> {
    fn configure(&mut self, cfg: &HwConfig) -> Result<(), Errno> {
        let format = virtio_format(cfg.format).ok_or(Errno::EINVAL)?;
        let want = Wanted {
            format,
            rate_hz: cfg.rate,
            channels: u8::try_from(cfg.channels).map_err(|_| Errno::EINVAL)?,
            period_bytes: cfg.period_bytes(),
            buffer_bytes: cfg.buffer_bytes(),
        };
        if want.buffer_bytes as usize > self.driver.pcm_len() {
            return Err(Errno::EINVAL);
        }
        let params = check_params(&self.dev.info, &want).map_err(|_| Errno::EINVAL)?;
        self.halt();
        self.goto(Command::SetParams, Some(&params))?;
        self.dev.ring = PeriodRing::new(params.buffer_bytes, params.period_bytes);
        self.dev.frame_bytes = cfg.frame_bytes;
        self.dev.silence = abi::format::silence(cfg.format);
        Ok(())
    }

    fn prepare(&mut self) -> Result<(), Errno> {
        self.halt();
        self.goto(Command::Prepare, None)
    }

    fn start(&mut self) -> Result<(), Errno> {
        self.goto(Command::Start, None)?;
        self.dev.running = true;
        // A polled device needs the audio thread awake from now on.
        kick();
        self.pump();
        Ok(())
    }

    fn stop(&mut self) -> Result<(), Errno> {
        self.halt();
        Ok(())
    }

    fn write(&mut self, offset: usize, data: &[u8]) -> Result<(), Errno> {
        if self.driver.pcm_write(offset, data) { Ok(()) } else { Err(Errno::EIO) }
    }

    fn commit(&mut self, bytes: usize, running: bool) {
        if let Some(r) = self.dev.ring.as_mut() {
            r.commit(bytes as u32);
        }
        if running && self.dev.running {
            self.pump();
        }
    }

    fn drain(&mut self) {
        let pad = self.dev.ring.as_mut().and_then(PeriodRing::pad_to_period);
        if let Some(c) = pad {
            let (s, n) = self.dev.silence;
            self.driver.pcm_fill(c.offset as usize, c.len as usize, &s[..n.max(1)]);
        }
        self.dev.draining = true;
        self.pump();
    }

    fn release(&mut self) {
        self.halt();
        if self.dev.state == StreamState::Prepared {
            let _ = self.goto(Command::Release, None);
        }
        self.dev.ring = None;
    }
}

static CARD_0: SetOnce<Arc<Card>> = SetOnce::new();
static WAKE: AtomicU64 = AtomicU64::new(0);
static AUDIO_WAIT: ThreadQueue = ThreadQueue::new();
static INTERRUPT_DRIVEN: AtomicBool = AtomicBool::new(false);
static INTERRUPTS: AtomicU64 = AtomicU64::new(0);
static MSIX: SetOnce<Option<MsixVectors>> = SetOnce::new();

/// The card, once [`init`] found one.
pub fn card() -> Option<&'static Arc<Card>> {
    CARD_0.get()
}

/// Interrupts taken, and whether the card is interrupt-driven.
pub fn interrupts() -> (u64, bool) {
    (INTERRUPTS.load(SeqCst), INTERRUPT_DRIVEN.load(SeqCst))
}

fn on_interrupt(_: usize) {
    INTERRUPTS.fetch_add(1, SeqCst);
    kick();
}

/// Make the audio thread look at the device soon.
pub fn kick() {
    WAKE.fetch_add(1, SeqCst);
    AUDIO_WAIT.wake_all();
}

/// Collect returned messages and events, move the streams on; returns
/// whether a stream is playing and whether anything changed.
fn service(card: &Card) -> (bool, bool) {
    let mut g = card.inner.lock();
    let Inner { driver, pcms } = &mut *g;
    let mut changed = false;
    let now = monotonic();
    for _ in 0..8 {
        driver.poll_events(|e| match e.code {
            code::EVT_PCM_XRUN => kprintln!("sound: stream {}: the device reports an underrun", e.data),
            code::EVT_JACK_CONNECTED | code::EVT_JACK_DISCONNECTED => kprintln!(
                "sound: jack {} {}",
                e.data,
                if e.code == code::EVT_JACK_CONNECTED { "connected" } else { "disconnected" }
            ),
            _ => {}
        });
        let mut done = [(0u32, 0u32, true); 64];
        let mut n = 0;
        while n < done.len() {
            let before = n;
            driver.poll_tx(|d| {
                if n < done.len() {
                    done[n] = (d.stream, d.len, d.status.status == poly_virtio_sound::proto::status::OK);
                    n += 1;
                }
            });
            if n == before {
                break;
            }
        }
        for &(stream, len, ok) in &done[..n] {
            if let Some(slot) = pcms.iter_mut().find(|s| s.dev.id == stream) {
                if !ok {
                    slot.dev.errors += 1;
                }
                if slot.dev.running
                    && let Some(r) = slot.dev.ring.as_mut()
                {
                    r.complete(len);
                    changed = true;
                }
            }
        }
        for slot in pcms.iter_mut() {
            if !slot.dev.running {
                continue;
            }
            let fb = slot.dev.frame_bytes.max(1) as u64;
            let played = slot.dev.ring.map_or(0, |r| r.played() / fb);
            let mut glue = Glue { driver: &mut *driver, dev: &mut slot.dev };
            slot.rt.on_played(played, &mut glue, now);
            if glue.dev.running {
                glue.pump();
            }
        }
        if !INTERRUPT_DRIVEN.load(SeqCst) || !driver.enable_interrupts() {
            break;
        }
    }
    let active = pcms.iter().any(|s| s.dev.running);
    (active, changed)
}

/// The audio thread: service the card whenever it interrupts (or is
/// kicked), and on a timer while a stream plays.
fn audio_thread(_: usize) -> usize {
    let Some(card) = card() else { return 1 };
    loop {
        let seen = WAKE.load(SeqCst);
        let (active, changed) = service(card);
        if changed {
            card.notify();
        }
        let interval = if INTERRUPT_DRIVEN.load(SeqCst) { SAFETY_POLL_NS } else { POLL_INTERVAL_NS };
        if active {
            AUDIO_WAIT.wait_until_deadline(|| WAKE.load(SeqCst) != seen, time::now_ns() + interval);
        } else {
            AUDIO_WAIT.wait_until(|| WAKE.load(SeqCst) != seen);
        }
    }
}

fn direction_name(d: u8) -> &'static str {
    match d {
        direction::OUTPUT => "output",
        direction::INPUT => "input",
        _ => "unknown",
    }
}

/// Find and start the sound device, query it, register `/dev/snd/*`,
/// start the audio thread and connect the interrupt. Call once, after
/// `pci::init` and `fs::init`.
pub fn init(_boot: &BootInfo) {
    let Some(found) = virtio::probe() else {
        if crate::pci::devices().iter().any(virtio::is_virtio_sound) {
            kprintln!("sound: the virtio-sound device could not be used; no sound");
        } else {
            kprintln!("sound: no sound device");
        }
        return;
    };
    let virtio::Found { bdf, mut driver, msix } = found;
    let config = driver.config();
    kprintln!(
        "sound: {bdf}: virtio-sound, {} jacks, {} streams, {} channel maps; queues {:?}, {} output messages in flight",
        config.jacks,
        config.streams,
        config.chmaps,
        driver.queue_sizes(),
        driver.tx_slots()
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
        kprintln!("sound: {bdf}: the device reports control responses shorter than it writes; taken by their status");
    }
    for (i, j) in jacks[..nj].iter().enumerate() {
        kprintln!(
            "sound: jack {i}: {}, defconf {:#x}, features {:#x}",
            if j.connected { "connected" } else { "not connected" },
            j.hda_reg_defconf,
            j.features
        );
    }
    let mut pcms = Vec::new();
    let mut descs = Vec::new();
    let mut inputs = 0;
    for (id, info) in infos[..ns].iter().enumerate() {
        let rates: Vec<u32> = info.rates_hz().collect();
        let formats: Vec<u32> = info.usable_formats().filter_map(alsa_format).collect();
        kprintln!(
            "sound: stream {id}: {}, {}-{} channels, rates {:?}, ALSA formats {:?}, features {:#x}",
            direction_name(info.direction),
            info.channels_min,
            info.channels_max,
            rates,
            formats,
            info.features
        );
        if !info.is_usable() {
            kprintln!("sound: stream {id}: not usable; left out");
            continue;
        }
        if info.direction != direction::OUTPUT {
            inputs += 1;
            continue;
        }
        if id >= 8 {
            kprintln!("sound: stream {id}: only 8 playback devices per card; left out");
            continue;
        }
        let caps = caps_of(info, driver.tx_slots(), driver.pcm_len());
        descs.push(PcmDesc { device: id as u32, caps, name: alloc::format!("virtio-sound output {id}") });
        pcms.push(PcmSlot {
            open: false,
            rt: PcmRuntime::new(),
            dev: DevStream {
                id: id as u32,
                info: *info,
                state: StreamState::Initial,
                ring: None,
                frame_bytes: 0,
                running: false,
                draining: false,
                silence: ([0; 8], 1),
                errors: 0,
            },
        });
    }
    for (i, m) in maps[..nm].iter().enumerate() {
        kprintln!("sound: channel map {i}: {}, positions {:?}", direction_name(m.direction), m.used_positions());
    }
    driver.disable_interrupts();
    let card = Arc::new(Card {
        bdf,
        inner: Mutex::new(Inner { driver, pcms }),
        queue: Arc::new(poly_vfs::WaitQueue::new()),
        pcms: descs,
        longname: alloc::format!("Poly virtio sound at PCI {bdf}"),
    });
    if CARD_0.set(card.clone()).is_err() {
        return;
    }
    alsa::register(&card);
    if let Err(e) = thread::spawn_named("audio", audio_thread, 0) {
        kprintln!("sound: cannot start the audio thread: {e:?}; no sound");
        return;
    }
    let mut msix = msix;
    let mut vector = None;
    if let Some(m) = &mut msix {
        match virtio::connect_interrupt(m, on_interrupt, 0) {
            Ok(v) => {
                vector = Some(v);
                INTERRUPT_DRIVEN.store(true, SeqCst);
                card.inner.lock().driver.enable_interrupts();
            }
            Err(e) => kprintln!("sound: cannot connect the MSI-X interrupt ({e:?}); polled"),
        }
    }
    let _ = MSIX.set(msix);
    kick();
    let devices: Vec<String> = card.pcms.iter().map(|p| alloc::format!("pcmC{CARD}D{}p", p.device)).collect();
    match vector {
        Some(v) => kprintln!(
            "sound: card {CARD}: /dev/snd/controlC{CARD}, {}; MSI-X entry {} -> vector {v:#x}",
            devices.join(", "),
            virtio::MSIX_ENTRY
        ),
        None => kprintln!(
            "sound: card {CARD}: /dev/snd/controlC{CARD}, {}; polled every {} ms while playing",
            devices.join(", "),
            POLL_INTERVAL_NS / time::NANOS_PER_MILLI
        ),
    }
    if inputs > 0 {
        kprintln!("sound: card {CARD}: {inputs} input streams not offered (no capture yet)");
    }
}
