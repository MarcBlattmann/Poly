//! Sound (ADR 0104, ADR 0109): the machine's sound device as ALSA card 0,
//! with an audio server in front of it.
//!
//! ```text
//!  /dev/snd/pcmC0D0p ─┐ ioctl / write / read / mmap / poll           device interrupt
//!  /dev/snd/pcmC0D0c ─┤ (alsa.rs; up to 8 opens each: subdevices)           │ on_interrupt: wake the thread
//!  /dev/snd/controlC0 ┤                                                     ▼
//!                     ▼                                      audio thread: every device period, mix every
//!   Card { Mutex<Inner { Hw, [Sub], Controls }> }  ◄───────  playing client into one period (poly-audio), apply
//!                     │ each Sub: PcmRuntime (personality)    the master volume, hand it to the device; hand each
//!                     │ over its own ring buffer (buffer.rs)  captured period to every recording client
//!                     ▼
//!   Hw: virtio-sound (poly-virtio-sound) or Intel HDA (poly-hda)
//! ```
//!
//! - **Clients.** Every open of a PCM device is a substream of its own
//!   (ALSA's subdevices: a card with "hardware" mixing): its own
//!   [`PcmRuntime`] (the host-tested ALSA logic) and its own ring buffer
//!   in shared memory ([`buffer::SharedBuf`], which `mmap(2)` maps), in the
//!   format the client chose. [`SoftGlue`] is the runtime's backend: it
//!   only fills and reads that buffer and marks the client active.
//! - **The server** (ADR 0109) runs in the audio thread. The device plays
//!   one stream in one fixed format ([`HwFormat`]); whenever it has room
//!   for a period, the thread mixes one from every active playback client
//!   (`poly_audio::server::mix_source`: converted, resampled, summed),
//!   applies the "Master Playback" controls, and hands it over. What a
//!   client gave moves its hardware pointer, so its runtime sees
//!   underruns and the end of a drain as before. Captured periods go to
//!   every active capture client (`poly_audio::server::deliver`), after
//!   the "Capture" controls. The device stream starts with the first
//!   client and stops [`IDLE_NS`] after the last.
//! - **The hardware** ([`Hw`]) is virtio-sound (`virtio.rs`) if there is
//!   one, else an Intel HDA controller (`hda.rs`, output only).
//! - **The audio thread** wakes on the device's interrupt, when a client
//!   starts, and on a timer while the device runs ([`POLL_INTERVAL_NS`]
//!   without interrupts, [`SAFETY_POLL_NS`] with them).

pub mod alsa;
pub mod buffer;
mod hda;
mod tests;
mod virtio;

use crate::boot_info::BootInfo;
use crate::pci::{MsiVector, MsixVectors};
use crate::sync::{Mutex, SetOnce};
use crate::thread::{self, WaitQueue as ThreadQueue};
use crate::{kprintln, time};
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use buffer::SharedBuf;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use poly_audio::convert::render;
use poly_audio::server::{Sink, Source, deliver, mix_source};
use poly_audio::{Converter, Format, MAX_CHANNELS, Sample, UNITY};
use poly_pci::Bdf;
use poly_personality_linux::errno::Errno;
use poly_personality_linux::sound::abi::{self, MmapControl, Timespec};
use poly_personality_linux::sound::ctl::Controls;
use poly_personality_linux::sound::hw::{self, Caps, HwConfig, Mask};
use poly_personality_linux::sound::pcm::{Dir, PcmBackend, PcmRuntime};

/// How long a control request may take.
pub const CONTROL_TIMEOUT_NS: u64 = time::NANOS_PER_SEC;
/// How often the audio thread looks at a device without interrupts while
/// it runs.
pub const POLL_INTERVAL_NS: u64 = 2 * time::NANOS_PER_MILLI;
/// ... and at one with interrupts, as a safety net.
pub const SAFETY_POLL_NS: u64 = 50 * time::NANOS_PER_MILLI;
/// The device stream keeps running (playing silence) this long after the
/// last client stopped, so back-to-back streams do not restart it.
pub const IDLE_NS: u64 = 300 * time::NANOS_PER_MILLI;
/// The ALSA card number.
pub const CARD: u32 = 0;
/// Substreams (concurrent opens) per PCM device.
pub const SUBDEVICES: usize = 8;
/// Bytes of a client's source frames converted at a time.
const SCRATCH_BYTES: usize = 16 * 1024;

/// A stream format the device runs in, fixed while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HwFormat {
    pub format: Format,
    pub period_frames: u32,
    pub periods: u32,
}

impl HwFormat {
    pub fn period_bytes(&self) -> usize {
        self.period_frames as usize * self.format.frame_bytes()
    }
}

/// The sound device.
#[derive(Debug)]
pub enum Hw {
    // Both boxed: the drivers are large (queues and tables, the codec's
    // widgets) and the card keeps one.
    Virtio(alloc::boxed::Box<virtio::VirtioHw>),
    Hda(alloc::boxed::Box<hda::HdaHw>),
}

macro_rules! each_hw {
    ($self:expr, $h:ident => $e:expr) => {
        match $self {
            Hw::Virtio($h) => $e,
            Hw::Hda($h) => $e,
        }
    };
}

impl Hw {
    pub fn output(&self) -> Option<HwFormat> {
        each_hw!(self, h => h.output())
    }
    pub fn input(&self) -> Option<HwFormat> {
        each_hw!(self, h => h.input())
    }
    fn collect(&mut self) {
        each_hw!(self, h => h.collect())
    }
    fn out_start(&mut self) -> bool {
        each_hw!(self, h => h.out_start())
    }
    fn out_room(&mut self) -> usize {
        match self {
            Hw::Virtio(h) => h.out_room(),
            Hw::Hda(h) => h.out_room(),
        }
    }
    fn out_push(&mut self, period: &[u8]) {
        each_hw!(self, h => h.out_push(period))
    }
    fn out_kick(&mut self) {
        each_hw!(self, h => h.out_kick())
    }
    fn out_stop(&mut self) {
        each_hw!(self, h => h.out_stop())
    }
    fn in_start(&mut self) -> bool {
        each_hw!(self, h => h.in_start())
    }
    fn in_pop(&mut self, out: &mut [u8]) -> Option<usize> {
        each_hw!(self, h => h.in_pop(out))
    }
    fn in_stop(&mut self) {
        each_hw!(self, h => h.in_stop())
    }
    fn enable_interrupts(&mut self) -> bool {
        each_hw!(self, h => h.enable_interrupts())
    }
    pub fn is_failed(&self) -> bool {
        each_hw!(self, h => h.is_failed())
    }
    pub fn bdf(&self) -> Bdf {
        each_hw!(self, h => h.bdf)
    }
}

/// ALSA formats and what they are to the server.
const SAMPLES: [(u32, Sample); 12] = [
    (abi::format::U8, Sample::U8),
    (abi::format::S8, Sample::S8),
    (abi::format::S16_LE, Sample::S16),
    (abi::format::U16_LE, Sample::U16),
    (abi::format::S24_LE, Sample::S24),
    (abi::format::U24_LE, Sample::U24),
    (abi::format::S24_3LE, Sample::S24_3),
    (abi::format::U24_3LE, Sample::U24_3),
    (abi::format::S32_LE, Sample::S32),
    (abi::format::U32_LE, Sample::U32),
    (abi::format::FLOAT_LE, Sample::Float),
    (abi::format::FLOAT64_LE, Sample::Float64),
];

fn sample_of(alsa: u32) -> Option<Sample> {
    SAMPLES.iter().find(|s| s.0 == alsa).map(|s| s.1)
}

/// Rates clients may ask for; the server resamples.
const RATES: [u32; 9] = [8000, 11025, 16000, 22050, 32000, 44100, 48000, 88200, 96000];

/// What a client may ask of a PCM device: any of the formats and rates
/// above, mono or stereo, read/write or mmap access, periods and buffers
/// the server's memory allows.
fn client_caps() -> Caps {
    let mut formats = Mask::default();
    for (f, _) in SAMPLES {
        formats.set(f);
    }
    let mut rates = [0u32; 16];
    rates[..RATES.len()].copy_from_slice(&RATES);
    Caps {
        access: hw::access_rw_and_mmap(),
        info: hw::INFO_MMAP,
        formats,
        rates,
        rate_count: RATES.len(),
        channels_min: 1,
        channels_max: 2,
        period_bytes_min: 64,
        period_bytes_max: 256 * 1024,
        periods_min: 2,
        periods_max: 64,
        buffer_bytes_max: 512 * 1024,
    }
}

/// A PCM device of the card, as user space sees it.
#[derive(Debug)]
pub struct PcmDesc {
    pub dir: Dir,
    pub device: u32,
    pub caps: Caps,
    pub name: String,
    /// The device stream behind it.
    pub hw: HwFormat,
}

/// The device side of one client: its ring buffer and conversion.
#[derive(Debug)]
pub struct Soft {
    pub dir: Dir,
    /// The device stream's format.
    hw_fmt: Format,
    pub buf: Option<SharedBuf>,
    conv: Option<Converter>,
    /// Started: the server mixes from (or delivers to) it.
    pub active: bool,
    /// The mmap status and control pages, once mapped.
    pub status: Option<SharedBuf>,
    pub control: Option<SharedBuf>,
    /// The control page's last values the kernel saw or wrote.
    last_appl: u64,
    last_avail_min: u64,
}

impl Soft {
    fn new(dir: Dir, hw_fmt: Format) -> Soft {
        Soft {
            dir,
            hw_fmt,
            buf: None,
            conv: None,
            active: false,
            status: None,
            control: None,
            last_appl: 0,
            last_avail_min: 0,
        }
    }
}

/// One substream.
#[derive(Debug)]
pub struct Sub {
    pub open: bool,
    pub rt: PcmRuntime,
    pub soft: Soft,
}

/// Counters of the server.
#[derive(Clone, Copy, Debug, Default)]
pub struct ServerStats {
    pub periods_out: u64,
    pub periods_in: u64,
    pub device_starts: u64,
}

#[derive(Debug, Default)]
struct OutState {
    running: bool,
    idle_since: Option<u64>,
    acc: Vec<i32>,
    bytes: Vec<u8>,
}

#[derive(Debug, Default)]
struct InState {
    running: bool,
    period: Vec<u8>,
}

/// One open control device's pending events (`snd_ctl_event`s).
#[derive(Debug)]
pub struct CtlEvents {
    pub subscribed: AtomicBool,
    pub queue: Mutex<alloc::collections::VecDeque<[u8; abi::CTL_EVENT_LEN]>>,
}

impl Default for CtlEvents {
    fn default() -> CtlEvents {
        CtlEvents { subscribed: AtomicBool::new(false), queue: Mutex::new(alloc::collections::VecDeque::new()) }
    }
}

/// The card's mutable state, behind its lock.
#[derive(Debug)]
pub struct Inner {
    pub hw: Hw,
    /// `SUBDEVICES` per entry of `Card::pcms`, in that order.
    pub subs: Vec<Sub>,
    pub controls: Controls,
    pub listeners: Vec<Weak<CtlEvents>>,
    pub stats: ServerStats,
    out: OutState,
    inp: InState,
    scratch: Vec<u8>,
}

/// The sound card.
#[derive(Debug)]
pub struct Card {
    pub bdf: Bdf,
    pub id: &'static str,
    pub driver: &'static str,
    pub name: &'static str,
    pub longname: String,
    pub inner: Mutex<Inner>,
    /// Woken (from the audio thread) whenever a stream's readiness may
    /// have changed.
    pub queue: Arc<poly_vfs::WaitQueue>,
    /// Woken when a control event is queued.
    pub ctl_queue: Arc<poly_vfs::WaitQueue>,
    pub pcms: Vec<PcmDesc>,
}

impl Card {
    /// Run `f` on substream `i` with its device side as a backend; the
    /// mapped control page is read before and the status page written
    /// after.
    pub fn with_sub<R>(&self, i: usize, f: impl FnOnce(&mut PcmRuntime, &mut SoftGlue<'_>) -> R) -> R {
        let mut g = self.inner.lock();
        let sub = &mut g.subs[i];
        pull_control(sub);
        let r = f(&mut sub.rt, &mut SoftGlue { soft: &mut sub.soft });
        push_pages(sub, monotonic());
        r
    }

    /// Wake everyone waiting on a stream of this card.
    pub fn notify(&self) {
        self.queue.notify(poly_vfs::PollEvents::WRITABLE | poly_vfs::PollEvents::READABLE | poly_vfs::PollEvents::ERR);
    }
}

/// mmap access: take what the application wrote to its control page.
fn pull_control(sub: &mut Sub) {
    let Sub { rt, soft, .. } = sub;
    let Some(page) = soft.control.as_ref() else { return };
    let mut b = [0u8; abi::MMAP_CONTROL_LEN];
    if !page.read(0, &mut b) {
        return;
    }
    let c = MmapControl::decode(&b);
    if c.appl_ptr != soft.last_appl {
        soft.last_appl = c.appl_ptr;
        let _ = rt.set_appl(c.appl_ptr, &mut SoftGlue { soft });
    }
    if c.avail_min != soft.last_avail_min {
        soft.last_avail_min = c.avail_min;
        rt.set_avail_min(c.avail_min);
    }
}

/// mmap access: publish the status, and the kernel's own moves of the
/// application pointer.
fn push_pages(sub: &mut Sub, now: Timespec) {
    let Sub { rt, soft, .. } = sub;
    if let Some(page) = soft.status.as_ref() {
        page.write(0, &rt.mmap_status(now).encode());
    }
    if let Some(page) = soft.control.as_ref() {
        let appl = rt.wrap(rt.appl());
        let avail_min = rt.sw_params().avail_min;
        if appl != soft.last_appl {
            page.write(0, &appl.to_le_bytes());
            soft.last_appl = appl;
        }
        if avail_min != soft.last_avail_min {
            page.write(8, &avail_min.to_le_bytes());
            soft.last_avail_min = avail_min;
        }
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

/// The monotonic clock as a timespec.
pub fn monotonic() -> Timespec {
    Timespec::from_ns(time::now_ns())
}

/// A client's device side as the ALSA runtime's backend: its own ring
/// buffer; the server does the rest.
pub struct SoftGlue<'a> {
    pub soft: &'a mut Soft,
}

impl PcmBackend for SoftGlue<'_> {
    fn configure(&mut self, cfg: &HwConfig) -> Result<(), Errno> {
        let sample = sample_of(cfg.format).ok_or(Errno::EINVAL)?;
        let channels = u8::try_from(cfg.channels).map_err(|_| Errno::EINVAL)?;
        let fmt = Format::new(sample, channels, cfg.rate);
        let conv = match self.soft.dir {
            Dir::Playback => Converter::new(fmt, self.soft.hw_fmt),
            Dir::Capture => Converter::new(self.soft.hw_fmt, fmt),
        }
        .ok_or(Errno::EINVAL)?;
        let buf = SharedBuf::new(cfg.buffer_bytes() as usize).ok_or(Errno::ENOMEM)?;
        let s = sample.silence();
        if !buf.is_empty() {
            buf.fill(0, buf.len(), &s[..sample.bytes()]);
        }
        self.soft.buf = Some(buf);
        self.soft.conv = Some(conv);
        self.soft.active = false;
        Ok(())
    }

    fn prepare(&mut self) -> Result<(), Errno> {
        self.soft.active = false;
        if let Some(c) = self.soft.conv.as_mut() {
            c.reset();
        }
        Ok(())
    }

    fn start(&mut self) -> Result<(), Errno> {
        if self.soft.buf.is_none() || self.soft.conv.is_none() {
            return Err(Errno::EBADFD);
        }
        self.soft.active = true;
        kick();
        Ok(())
    }

    fn stop(&mut self) -> Result<(), Errno> {
        self.soft.active = false;
        Ok(())
    }

    fn write(&mut self, offset: usize, data: &[u8]) -> Result<(), Errno> {
        match self.soft.buf.as_ref() {
            Some(b) if b.write(offset, data) => Ok(()),
            _ => Err(Errno::EIO),
        }
    }

    fn commit(&mut self, _bytes: usize, _running: bool) {}

    fn drain(&mut self) {
        kick();
    }

    fn release(&mut self) {
        self.soft.active = false;
        self.soft.buf = None;
        self.soft.conv = None;
    }

    fn read(&mut self, offset: usize, out: &mut [u8]) -> Result<(), Errno> {
        match self.soft.buf.as_ref() {
            Some(b) if b.read(offset, out) => Ok(()),
            _ => Err(Errno::EIO),
        }
    }
}

/// A playing client as the mixer's source.
struct Play<'a> {
    rt: &'a mut PcmRuntime,
    soft: &'a mut Soft,
    now: Timespec,
}

impl Source for Play<'_> {
    fn converter(&mut self) -> &mut Converter {
        self.soft.conv.as_mut().unwrap_or_else(|| unreachable!("only clients with a converter are mixed"))
    }

    fn gains(&self) -> [u32; MAX_CHANNELS] {
        [UNITY; MAX_CHANNELS]
    }

    fn peek(&mut self, out: &mut [u8]) -> usize {
        let (Some(cfg), Some(buf)) = (self.rt.config(), self.soft.buf.as_ref()) else { return 0 };
        let fb = cfg.frame_bytes as usize;
        let buffer = cfg.buffer_frames as u64;
        let n = ((out.len() / fb) as u64).min(self.rt.queued());
        let at = self.rt.hw() % buffer;
        let first = n.min(buffer - at) as usize;
        let ok =
            buf.read(at as usize * fb, &mut out[..first * fb]) && buf.read(0, &mut out[first * fb..n as usize * fb]);
        if ok { n as usize } else { 0 }
    }

    fn consume(&mut self, frames: usize) {
        let total = self.rt.hw() + frames as u64;
        self.rt.on_played(total, &mut SoftGlue { soft: self.soft }, self.now);
    }
}

/// A recording client as the server's sink.
struct Rec<'a> {
    rt: &'a mut PcmRuntime,
    soft: &'a mut Soft,
    gains: [u32; MAX_CHANNELS],
    now: Timespec,
}

impl Sink for Rec<'_> {
    fn converter(&mut self) -> &mut Converter {
        self.soft.conv.as_mut().unwrap_or_else(|| unreachable!("only clients with a converter record"))
    }

    fn gains(&self) -> [u32; MAX_CHANNELS] {
        self.gains
    }

    fn room(&self) -> usize {
        self.rt.capture_room() as usize
    }

    fn push(&mut self, data: &[u8]) {
        let (Some(cfg), Some(buf)) = (self.rt.config(), self.soft.buf.as_ref()) else { return };
        let fb = cfg.frame_bytes as usize;
        let buffer = cfg.buffer_frames as u64;
        let n = (data.len() / fb) as u64;
        let at = self.rt.hw() % buffer;
        let first = n.min(buffer - at) as usize;
        buf.write(at as usize * fb, &data[..first * fb]);
        buf.write(0, &data[first * fb..n as usize * fb]);
        let total = self.rt.hw() + n;
        self.rt.on_captured(total, &mut SoftGlue { soft: self.soft }, self.now);
    }
}

fn gains_of(g: [u32; 2]) -> [u32; MAX_CHANNELS] {
    let mut out = [UNITY; MAX_CHANNELS];
    out[0] = g[0];
    out[1] = g[1];
    // Channels past the second follow the first two.
    for c in 2..MAX_CHANNELS {
        out[c] = g[c % 2];
    }
    out
}

/// Mix and hand over as many output periods as the device has room for.
fn serve_output(inner: &mut Inner, now: Timespec, now_ns: u64) -> bool {
    let Inner { hw, subs, controls, out, scratch, stats, .. } = inner;
    let Some(hwf) = hw.output() else { return false };
    let want = subs.iter().any(|s| s.soft.dir == Dir::Playback && s.soft.active);
    if want && !out.running {
        if !hw.out_start() {
            return false;
        }
        out.running = true;
        out.idle_since = None;
        stats.device_starts += 1;
    }
    if !out.running {
        return false;
    }
    let ch = hwf.format.channels as usize;
    out.acc.resize(hwf.period_frames as usize * ch, 0);
    out.bytes.resize(hwf.period_bytes(), 0);
    let gains = gains_of(controls.playback_gains());
    let mut changed = false;
    let mut pushed = 0;
    while pushed < hwf.periods && hw.out_room() > 0 {
        out.acc.fill(0);
        for sub in subs.iter_mut() {
            let Sub { rt, soft, .. } = sub;
            if soft.dir != Dir::Playback || !soft.active || soft.conv.is_none() || soft.buf.is_none() {
                continue;
            }
            let m = mix_source(&mut Play { rt, soft, now }, &mut out.acc, scratch);
            changed |= m.consumed > 0 || !soft.active;
        }
        render(&out.acc, ch, &gains, hwf.format.sample, &mut out.bytes);
        hw.out_push(&out.bytes);
        stats.periods_out += 1;
        pushed += 1;
    }
    hw.out_kick();
    let still = subs.iter().any(|s| s.soft.dir == Dir::Playback && s.soft.active);
    if still {
        out.idle_since = None;
    } else {
        let since = *out.idle_since.get_or_insert(now_ns);
        if now_ns.saturating_sub(since) >= IDLE_NS {
            hw.out_stop();
            out.running = false;
            out.idle_since = None;
        }
    }
    changed
}

/// Hand every captured period to the recording clients.
fn serve_input(inner: &mut Inner, now: Timespec) -> bool {
    let Inner { hw, subs, controls, inp, scratch, stats, .. } = inner;
    let Some(hwf) = hw.input() else { return false };
    let want = subs.iter().any(|s| s.soft.dir == Dir::Capture && s.soft.active);
    if want && !inp.running {
        if !hw.in_start() {
            return false;
        }
        inp.running = true;
    }
    if !inp.running {
        return false;
    }
    inp.period.resize(hwf.period_bytes(), 0);
    let gains = gains_of(controls.capture_gains());
    let mut changed = false;
    while let Some(n) = hw.in_pop(&mut inp.period) {
        stats.periods_in += 1;
        for sub in subs.iter_mut() {
            let Sub { rt, soft, .. } = sub;
            if soft.dir != Dir::Capture || !soft.active || soft.conv.is_none() || soft.buf.is_none() {
                continue;
            }
            deliver(&mut Rec { rt, soft, gains, now }, &inp.period[..n], scratch);
            changed = true;
        }
    }
    if !subs.iter().any(|s| s.soft.dir == Dir::Capture && s.soft.active) {
        hw.in_stop();
        inp.running = false;
    }
    changed
}

static CARD_0: SetOnce<Arc<Card>> = SetOnce::new();
static WAKE: AtomicU64 = AtomicU64::new(0);
static AUDIO_WAIT: ThreadQueue = ThreadQueue::new();
static INTERRUPT_DRIVEN: AtomicBool = AtomicBool::new(false);
static INTERRUPTS: AtomicU64 = AtomicU64::new(0);
static MSIX: SetOnce<Option<MsixVectors>> = SetOnce::new();
static MSI: SetOnce<Option<MsiVector>> = SetOnce::new();

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

/// One round of the server; returns whether the device runs and whether
/// anything changed for the clients.
fn service(card: &Card) -> (bool, bool) {
    let mut g = card.inner.lock();
    let inner = &mut *g;
    let now = monotonic();
    let now_ns = time::now_ns();
    let mut changed = false;
    for _ in 0..8 {
        inner.hw.collect();
        for sub in inner.subs.iter_mut().filter(|s| s.open) {
            pull_control(sub);
        }
        changed |= serve_output(inner, now, now_ns);
        changed |= serve_input(inner, now);
        for sub in inner.subs.iter_mut().filter(|s| s.open) {
            push_pages(sub, now);
        }
        if !INTERRUPT_DRIVEN.load(SeqCst) || !inner.hw.enable_interrupts() {
            break;
        }
    }
    (inner.out.running || inner.inp.running, changed)
}

/// The audio thread: serve the card whenever it interrupts (or is
/// kicked), and on a timer while the device runs.
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

/// Find the sound device (virtio-sound, else Intel HDA), register
/// `/dev/snd/*`, start the audio thread and connect the interrupt. Call
/// once, after `pci::init` and `fs::init`.
pub fn init(_boot: &BootInfo) {
    let (hw, ids, irq) = if let Some(found) = virtio::probe() {
        let virtio::Found { bdf, driver, msix } = found;
        let hw = virtio::VirtioHw::new(bdf, driver);
        (Hw::Virtio(alloc::boxed::Box::new(hw)), ("Virtio", "poly-virtio-snd", "Poly virtio sound"), Irq::Msix(msix))
    } else if let Some((hw, msi)) = hda::probe(on_interrupt) {
        (Hw::Hda(alloc::boxed::Box::new(hw)), ("HDA", "poly-hda", "Poly HD Audio"), Irq::Msi(msi))
    } else {
        if crate::pci::devices().iter().any(virtio::is_virtio_sound) {
            kprintln!("sound: the virtio-sound device could not be used; no sound");
        } else {
            kprintln!("sound: no sound device");
        }
        return;
    };
    let bdf = hw.bdf();
    let mut pcms = Vec::new();
    if let Some(f) = hw.output() {
        pcms.push(PcmDesc {
            dir: Dir::Playback,
            device: 0,
            caps: client_caps(),
            name: alloc::format!("{} output", ids.2),
            hw: f,
        });
    }
    if let Some(f) = hw.input() {
        pcms.push(PcmDesc {
            dir: Dir::Capture,
            device: 0,
            caps: client_caps(),
            name: alloc::format!("{} input", ids.2),
            hw: f,
        });
    }
    if pcms.is_empty() {
        kprintln!("sound: {bdf}: no stream the server can use; no sound");
        return;
    }
    for p in &pcms {
        kprintln!(
            "sound: {} device stream: {:?}, {} channels, {} Hz, periods of {} frames x {}",
            if p.dir == Dir::Playback { "output" } else { "input" },
            p.hw.format.sample,
            p.hw.format.channels,
            p.hw.format.rate,
            p.hw.period_frames,
            p.hw.periods
        );
    }
    let mut subs = Vec::new();
    for p in &pcms {
        for _ in 0..SUBDEVICES {
            let rt = PcmRuntime::with_dir(p.dir);
            subs.push(Sub { open: false, rt, soft: Soft::new(p.dir, p.hw.format) });
        }
    }
    let controls = Controls::new(hw.output().is_some(), hw.input().is_some());
    let card = Arc::new(Card {
        bdf,
        id: ids.0,
        driver: ids.1,
        name: ids.2,
        longname: alloc::format!("{} at PCI {bdf}", ids.2),
        inner: Mutex::new(Inner {
            hw,
            subs,
            controls,
            listeners: Vec::new(),
            stats: ServerStats::default(),
            out: OutState::default(),
            inp: InState::default(),
            scratch: vec![0; SCRATCH_BYTES],
        }),
        queue: Arc::new(poly_vfs::WaitQueue::new()),
        ctl_queue: Arc::new(poly_vfs::WaitQueue::new()),
        pcms,
    });
    if CARD_0.set(card.clone()).is_err() {
        return;
    }
    alsa::register(&card);
    if let Err(e) = thread::spawn_named("audio", audio_thread, 0) {
        kprintln!("sound: cannot start the audio thread: {e:?}; no sound");
        return;
    }
    let how = match irq {
        Irq::Msix(Some(mut m)) => {
            let r = virtio::connect_interrupt(&mut m, on_interrupt, 0);
            let _ = MSIX.set(Some(m));
            match r {
                Ok(v) => {
                    INTERRUPT_DRIVEN.store(true, SeqCst);
                    card.inner.lock().hw.enable_interrupts();
                    alloc::format!("MSI-X entry {} -> vector {v:#x}", virtio::MSIX_ENTRY)
                }
                Err(e) => {
                    kprintln!("sound: cannot connect the MSI-X interrupt ({e:?}); polled");
                    polled()
                }
            }
        }
        Irq::Msi(Some(m)) => {
            let v = m.vector();
            let _ = MSI.set(Some(m));
            INTERRUPT_DRIVEN.store(true, SeqCst);
            if let Hw::Hda(h) = &mut card.inner.lock().hw {
                h.connect_interrupts();
            }
            alloc::format!("MSI -> vector {v:#x}")
        }
        _ => polled(),
    };
    kick();
    let devices: Vec<String> = card
        .pcms
        .iter()
        .map(|p| alloc::format!("pcmC{CARD}D{}{}", p.device, if p.dir == Dir::Playback { 'p' } else { 'c' }))
        .collect();
    kprintln!("sound: card {CARD}: /dev/snd/controlC{CARD}, {}; {how}", devices.join(", "));
}

fn polled() -> String {
    alloc::format!("polled every {} ms while playing", POLL_INTERVAL_NS / time::NANOS_PER_MILLI)
}

/// How the device interrupts.
enum Irq {
    Msix(Option<MsixVectors>),
    Msi(Option<MsiVector>),
}
