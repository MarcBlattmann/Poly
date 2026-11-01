//! The driver: controller bring-up, codec commands, output path setup and
//! the output stream.

use crate::codec::{
    self, FG_AUDIO, PinCaps, WidgetCaps, WidgetType, amp, command, conn, eapd, long, param, pinctl, power, short, verb,
};
use crate::format::StreamFormat;
use crate::graph::{self, MAX_WIDGETS, OutputPath, Widget};
use crate::regs::{
    self, corbctl, corbrp, corbsts, gctl, int, ringsize, rirb_ex, rirbctl, rirbsts, rirbwp, sd, sdctl, sdsts,
};
use crate::{Delay, regs::BDL_ENTRY};
use core::ptr::{self, NonNull};
use core::sync::atomic::{Ordering, fence};
use poly_pci::Mmio;

/// Entries in the buffer descriptor list at most (§3.6.2 allows 256).
pub const MAX_BDL_ENTRIES: usize = 32;
/// The stream tag of the output stream (SDnCTL bits 23:20 and the codec's
/// Converter Stream, Channel verb). Tag 0 is reserved (§3.3, SDnCTL).
pub const OUTPUT_STREAM_TAG: u8 = 1;
/// Microseconds waited after leaving reset before STATESTS is read: codecs
/// request their status change within 521 µs of the link coming up (§4.3).
pub const CODEC_WAKE_US: u32 = 1000;

/// Wait between polls of a register bit, and how many polls at most.
const POLL_US: u32 = 10;
const POLLS: u32 = 1000;
/// How long the controller is held in reset, at least (§4.2: the link reset
/// must last long enough for codecs to see it; 100 µs is generous).
const RESET_HOLD_US: u32 = 100;
/// Polls of the RIRB for one verb's response, `VERB_POLL_US` apart.
const VERB_POLLS: u32 = 1000;
const VERB_POLL_US: u32 = 10;
/// Polls of Get Power State for the AFG to reach D0, `POWER_POLL_US` apart.
const POWER_POLLS: u32 = 20;
const POWER_POLL_US: u32 = 500;
/// Function groups looked at per codec.
const MAX_FUNCTION_GROUPS: u32 = 8;
/// SDI lines: STATESTS bits 14:0 (codec address 15 is not a codec).
const MAX_CODECS: u8 = 15;

/// Memory shared with the device: its CPU address, bus address and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaRegion {
    pub ptr: NonNull<u8>,
    pub bus: u64,
    pub len: usize,
}

/// All memory the controller sees. Every region starts on a 128-byte
/// boundary (bus address).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    /// The command ring: 1024 bytes for 256 entries (smaller rings are used
    /// if the region or the controller is smaller: 64 bytes, 8 bytes).
    pub corb: DmaRegion,
    /// The response ring: 2048 bytes for 256 entries (or 128, 16).
    pub rirb: DmaRegion,
    /// The buffer descriptor list: 16 bytes per entry, up to
    /// [`MAX_BDL_ENTRIES`].
    pub bdl: DmaRegion,
    /// The output ring buffer.
    pub buffer: DmaRegion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitError {
    /// The registers read all ones: no controller behind the BAR.
    NotPresent,
    /// A region's bus address is not 128-byte aligned.
    Misaligned,
    /// A region lies above 4 GiB and the controller cannot address it
    /// (GCAP 64OK clear).
    Address,
    /// A region is too small (CORB, RIRB for the smallest ring the
    /// controller supports, BDL for two entries).
    NoRoom,
    /// The controller has no output stream descriptor.
    NoOutputStream,
    /// No codec reported its presence after reset.
    NoCodec,
    /// The controller did not enter or leave reset, a DMA engine did not
    /// start or stop, or no codec answered a command.
    Timeout,
    /// No codec has a usable output path.
    NoOutputPath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The controller or codec did not respond in time.
    Timeout,
    /// An argument is out of range: codec address, verb field, period size
    /// or count, the buffer or BDL is too small.
    Range,
    /// The output path cannot play this format.
    Unsupported,
    /// The output stream is not configured.
    NotConfigured,
}

/// Global Capabilities, decoded (§3.3, GCAP).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gcap {
    /// Output, input and bidirectional stream descriptors.
    pub output_streams: u8,
    pub input_streams: u8,
    pub bidir_streams: u8,
    /// Serial data out signals (1, 2 or 4).
    pub sdo_lines: u8,
    /// 64-bit addresses are supported.
    pub addr64: bool,
}

impl Gcap {
    pub fn decode(v: u16) -> Gcap {
        Gcap {
            output_streams: ((v >> 12) & 0xF) as u8,
            input_streams: ((v >> 8) & 0xF) as u8,
            bidir_streams: ((v >> 3) & 0x1F) as u8,
            sdo_lines: match (v >> 1) & 3 {
                0 => 1,
                1 => 2,
                _ => 4,
            },
            addr64: v & 1 != 0,
        }
    }

    /// Stream descriptors in all.
    pub fn streams(&self) -> usize {
        self.input_streams as usize + self.output_streams as usize + self.bidir_streams as usize
    }
}

/// What [`Hda::take_interrupt`] found (and cleared).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Interrupts {
    /// INTSTS as read.
    pub intsts: u32,
    /// The output stream completed a BDL entry with IOC set (a period).
    pub buffer_complete: bool,
    pub fifo_error: bool,
    pub descriptor_error: bool,
    /// RIRB response interrupt / overrun flags were set.
    pub response: bool,
    pub response_overrun: bool,
    /// CORB memory error.
    pub corb_error: bool,
    /// Codecs that requested a status change (STATESTS).
    pub codec_change: u16,
    /// The registers read all ones: the controller is gone.
    pub gone: bool,
}

impl Interrupts {
    /// Was anything there?
    pub fn any(&self) -> bool {
        self.buffer_complete
            || self.fifo_error
            || self.descriptor_error
            || self.response
            || self.response_overrun
            || self.corb_error
            || self.codec_change != 0
            || self.gone
    }
}

/// Counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub verbs: u64,
    pub timeouts: u64,
    /// Unsolicited responses skipped.
    pub unsolicited: u64,
    /// Solicited responses that were not for the current verb (another
    /// codec, or late answers to a verb that timed out).
    pub stale: u64,
    /// `take_interrupt` calls that found something.
    pub interrupts: u64,
    pub buffer_completions: u64,
    /// FIFO and descriptor errors of the output stream.
    pub stream_errors: u64,
}

/// The output stream as configured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StreamSetup {
    format: StreamFormat,
    cbl: u32,
}

/// The controller with its codecs, the chosen output path and one output
/// stream.
#[derive(Debug)]
pub struct Hda<M> {
    regs: M,
    mem: Memory,
    gcap: Gcap,
    codecs: u16,
    corb_entries: u16,
    rirb_entries: u16,
    corb_wp: u16,
    rirb_rp: u16,
    output: OutputPath,
    vendor: u32,
    widgets: [Widget; MAX_WIDGETS],
    widget_count: usize,
    /// Which AFG `widgets` holds.
    table_of: Option<(u8, u8)>,
    stream: Option<StreamSetup>,
    irq: bool,
    stats: Stats,
}

// SAFETY: the DMA pointers refer to memory lent to the driver for its whole
// lifetime (see `init`); nothing else in the CPU uses it, so moving the
// driver to another thread moves sole ownership of it.
unsafe impl<M: Send> Send for Hda<M> {}

/// Poll `cond` up to `polls` times, `us` apart.
fn poll(delay: &mut dyn Delay, polls: u32, us: u32, mut cond: impl FnMut() -> bool) -> bool {
    for _ in 0..polls {
        if cond() {
            return true;
        }
        delay.delay_us(us);
    }
    cond()
}

/// The ring size (entries, CORBSIZE/RIRBSIZE field value) to use: the
/// largest the controller supports whose ring fits `room` bytes.
fn ring_size(sizereg: u8, entry: usize, room: usize) -> Option<(u16, u8)> {
    let caps = sizereg & 0xF0;
    let options = [
        (ringsize::CAP_256, 256u16, ringsize::SIZE_256),
        (ringsize::CAP_16, 16, ringsize::SIZE_16),
        (ringsize::CAP_2, 2, ringsize::SIZE_2),
    ];
    if caps == 0 {
        // No capability bits: the size in use is the only one.
        let (_, n, code) = options.iter().copied().find(|o| o.2 == sizereg & ringsize::SIZE_MASK)?;
        return (n as usize * entry <= room).then_some((n, code));
    }
    options.iter().find(|o| caps & o.0 != 0 && o.1 as usize * entry <= room).map(|o| (o.1, o.2))
}

fn region_ok(r: &DmaRegion, addr64: bool) -> Result<(), InitError> {
    if !r.bus.is_multiple_of(regs::DMA_ALIGN) {
        return Err(InitError::Misaligned);
    }
    let end = r.bus.checked_add(r.len as u64).ok_or(InitError::Address)?;
    if !addr64 && end > 1 << 32 {
        return Err(InitError::Address);
    }
    Ok(())
}

/// Reset and bring up the controller, find the codecs, choose and set up an
/// output path.
///
/// 1. Stop the CORB, RIRB and stream DMA engines; reset the controller
///    (GCTL.CRST to 0, wait for it to read 0, hold 100 µs, back to 1, wait
///    for 1; §4.2) and wait [`CODEC_WAKE_US`] for codecs to announce
///    themselves in STATESTS (§4.3).
/// 2. Set up the CORB (§4.4.1) and RIRB (§4.4.2): engines stopped, the
///    largest ring that fits, base addresses, CORBRP reset handshake,
///    RIRBWP reset, RINTCNT 1, the response interrupt flag on (cleared with
///    every response taken; the controller interrupt stays off), engines
///    started.
/// 3. Walk every present codec's widget graph (§7.3.4) and keep the best
///    output path (see [`crate::graph`]).
/// 4. Configure it: AFG and path widgets to D0, output amplifiers unmuted
///    at 0 dB, the path's input amplifiers unmuted, connection selects of
///    selectors and pins with several inputs set, the pin's widget control
///    to OUT_EN (+HP_EN for headphone pins), EAPD on where the pin has it.
///
/// Unsolicited responses are accepted (GCTL.UNSOL) and skipped. Interrupts
/// stay off until [`Hda::enable_interrupts`].
///
/// # Safety
///
/// The memory regions are valid for their lengths, reachable by the
/// controller at their bus addresses, and used only by the driver and the
/// controller until the driver is dropped after [`Hda::shutdown`] (or
/// forever). `regs` is the controller's memory BAR (BAR0, at least 0x80 +
/// 0x20 × streams bytes), with memory decoding and bus mastering enabled.
pub unsafe fn init<M: Mmio>(regs: M, memory: Memory, delay: &mut dyn Delay) -> Result<Hda<M>, InitError> {
    let gcap_raw = regs.read16(regs::GCAP);
    if gcap_raw == 0xFFFF && regs.read8(regs::VMAJ) == 0xFF {
        return Err(InitError::NotPresent);
    }
    let gcap = Gcap::decode(gcap_raw);
    for r in [&memory.corb, &memory.rirb, &memory.bdl, &memory.buffer] {
        region_ok(r, gcap.addr64)?;
    }
    if memory.bdl.len < 2 * BDL_ENTRY {
        return Err(InitError::NoRoom);
    }
    if gcap.output_streams == 0 {
        return Err(InitError::NoOutputStream);
    }

    // Stop every DMA engine before the reset.
    regs.write8(regs::CORBCTL, 0);
    regs.write8(regs::RIRBCTL, 0);
    for i in 0..gcap.streams() {
        regs.write8(regs::sd(i) + sd::CTL, 0);
    }
    poll(delay, POLLS, POLL_US, || {
        regs.read8(regs::CORBCTL) & corbctl::RUN == 0
            && regs.read8(regs::RIRBCTL) & rirbctl::DMAEN == 0
            && (0..gcap.streams()).all(|i| regs.read8(regs::sd(i) + sd::CTL) & sdctl::RUN == 0)
    });

    // §4.2: controller reset.
    let g = regs.read32(regs::GCTL);
    regs.write32(regs::GCTL, g & !gctl::CRST);
    if !poll(delay, POLLS, POLL_US, || regs.read32(regs::GCTL) & gctl::CRST == 0) {
        return Err(InitError::Timeout);
    }
    delay.delay_us(RESET_HOLD_US);
    let g = regs.read32(regs::GCTL);
    regs.write32(regs::GCTL, g | gctl::CRST);
    if !poll(delay, POLLS, POLL_US, || regs.read32(regs::GCTL) & gctl::CRST != 0) {
        return Err(InitError::Timeout);
    }
    // §4.3: codecs signal their presence within 521 µs.
    delay.delay_us(CODEC_WAKE_US);
    let codecs = regs.read16(regs::STATESTS) & 0x7FFF;
    regs.write16(regs::STATESTS, codecs);
    if codecs == 0 {
        return Err(InitError::NoCodec);
    }
    let g = regs.read32(regs::GCTL);
    regs.write32(regs::GCTL, g | gctl::UNSOL);
    regs.write32(regs::INTCTL, 0);
    regs.write16(regs::WAKEEN, 0);

    let (corb_entries, corb_code) =
        ring_size(regs.read8(regs::CORBSIZE), regs::CORB_ENTRY, memory.corb.len).ok_or(InitError::NoRoom)?;
    let (rirb_entries, rirb_code) =
        ring_size(regs.read8(regs::RIRBSIZE), regs::RIRB_ENTRY, memory.rirb.len).ok_or(InitError::NoRoom)?;

    // §4.4.1: the CORB.
    regs.write8(regs::CORBCTL, 0);
    if !poll(delay, POLLS, POLL_US, || regs.read8(regs::CORBCTL) & corbctl::RUN == 0) {
        return Err(InitError::Timeout);
    }
    regs.write8(regs::CORBSIZE, corb_code);
    regs.write32(regs::CORBLBASE, memory.corb.bus as u32);
    regs.write32(regs::CORBUBASE, (memory.corb.bus >> 32) as u32);
    regs.write16(regs::CORBWP, 0);
    // CORBRP reset: write 1, it reads 1 once done, write 0, reads 0. Some
    // controllers never show the 1; only failing to clear is fatal.
    regs.write16(regs::CORBRP, corbrp::RST);
    poll(delay, POLLS, POLL_US, || regs.read16(regs::CORBRP) & corbrp::RST != 0);
    regs.write16(regs::CORBRP, 0);
    if !poll(delay, POLLS, POLL_US, || regs.read16(regs::CORBRP) & corbrp::RST == 0) {
        return Err(InitError::Timeout);
    }
    regs.write8(regs::CORBSTS, corbsts::CMEI);

    // §4.4.2: the RIRB.
    regs.write8(regs::RIRBCTL, 0);
    if !poll(delay, POLLS, POLL_US, || regs.read8(regs::RIRBCTL) & rirbctl::DMAEN == 0) {
        return Err(InitError::Timeout);
    }
    regs.write8(regs::RIRBSIZE, rirb_code);
    regs.write32(regs::RIRBLBASE, memory.rirb.bus as u32);
    regs.write32(regs::RIRBUBASE, (memory.rirb.bus >> 32) as u32);
    regs.write16(regs::RIRBWP, rirbwp::RST);
    regs.write16(regs::RINTCNT, 1);
    regs.write8(regs::RIRBSTS, rirbsts::RINTFL | rirbsts::OIS);

    // Responses must have somewhere to go before commands flow.
    // RINTCTL on: RINTFL is set after every RINTCNT (1) responses, and the
    // driver clears it as it takes each response. QEMU's controller stops
    // taking commands once RINTCNT responses are waiting with RINTFL set,
    // and with RINTCTL off it never sets RINTFL, so its CORB stalled after
    // the first response (observed, ADR 0109). INTCTL.CIE stays off, so
    // this raises no interrupt.
    regs.write8(regs::RIRBCTL, rirbctl::DMAEN | rirbctl::RINTCTL);
    regs.write8(regs::CORBCTL, corbctl::RUN);
    let started = poll(delay, POLLS, POLL_US, || {
        regs.read8(regs::RIRBCTL) & rirbctl::DMAEN != 0 && regs.read8(regs::CORBCTL) & corbctl::RUN != 0
    });
    if !started {
        return Err(InitError::Timeout);
    }

    let empty_path = OutputPath {
        codec: 0,
        afg: 0,
        dac: 0,
        pin: 0,
        nodes: [0; graph::MAX_PATH],
        len: 0,
        inputs: [0; graph::MAX_PATH],
        rates: 0,
        sizes: 0,
        max_channels: 0,
        rank: 0,
    };
    let mut hda = Hda {
        regs,
        mem: memory,
        gcap,
        codecs,
        corb_entries,
        rirb_entries,
        corb_wp: 0,
        rirb_rp: 0,
        output: empty_path,
        vendor: 0,
        widgets: [Widget::EMPTY; MAX_WIDGETS],
        widget_count: 0,
        table_of: None,
        stream: None,
        irq: false,
        stats: Stats::default(),
    };
    hda.discover(delay)?;
    hda.configure_path(delay).map_err(|_| InitError::Timeout)?;
    Ok(hda)
}

impl<M: Mmio> Hda<M> {
    /// The chosen output path.
    pub fn output(&self) -> &OutputPath {
        &self.output
    }

    pub fn gcap(&self) -> Gcap {
        self.gcap
    }

    /// STATESTS at init: bit n for a codec at address n.
    pub fn codecs(&self) -> u16 {
        self.codecs
    }

    /// Vendor / device ID (parameter 0x00) of the codec with the output path.
    pub fn vendor_id(&self) -> u32 {
        self.vendor
    }

    /// The widgets of the AFG that holds the output path, in NID order.
    pub fn widgets(&self) -> &[Widget] {
        &self.widgets[..self.widget_count.min(MAX_WIDGETS)]
    }

    /// Call `each(codec, nid, widget_type, caps)` for every widget of the
    /// AFG that holds the output path.
    pub fn each_widget(&self, mut each: impl FnMut(u8, u8, WidgetType, u32)) {
        for w in self.widgets() {
            each(self.output.codec, w.nid, w.kind(), w.caps.0);
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The register block.
    pub fn regs(&self) -> &M {
        &self.regs
    }

    /// Byte offset of the output stream descriptor: the first output
    /// stream, after the input streams.
    fn osd(&self) -> usize {
        regs::sd(self.gcap.input_streams as usize)
    }

    /// The interrupt bit (INTCTL SIE / INTSTS SIS) of the output stream.
    pub fn output_stream_bit(&self) -> u32 {
        1 << self.gcap.input_streams
    }

    /// The next response in the RIRB, if the controller wrote one.
    fn next_response(&mut self) -> Option<(u32, u32)> {
        let n = self.rirb_entries.max(1);
        let wp = (self.regs.read16(regs::RIRBWP) & 0xFF) % n;
        if wp == self.rirb_rp {
            return None;
        }
        self.rirb_rp = (self.rirb_rp + 1) % n;
        fence(Ordering::Acquire);
        let off = self.rirb_rp as usize * regs::RIRB_ENTRY;
        if off + regs::RIRB_ENTRY > self.mem.rirb.len {
            return None;
        }
        // SAFETY: `off + 8 <= rirb.len` (checked above) and the region is
        // lent to the driver (see `init`); the entry was written by the
        // controller before it moved RIRBWP past it.
        let (resp, ex) = unsafe {
            let p = self.mem.rirb.ptr.as_ptr().add(off);
            (ptr::read_volatile(p as *const u32), ptr::read_volatile(p.add(4) as *const u32))
        };
        // Each response sets RINTFL (RINTCNT 1, RINTCTL on); a controller
        // may wait for it to be cleared before it takes the next command
        // (see `init`).
        self.regs.write8(regs::RIRBSTS, rirbsts::RINTFL | rirbsts::OIS);
        Some((u32::from_le(resp), u32::from_le(ex)))
    }

    /// Skip whatever responses are already in the RIRB.
    fn drain(&mut self) {
        for _ in 0..=self.rirb_entries {
            let Some((_, ex)) = self.next_response() else { break };
            if ex & rirb_ex::UNSOL != 0 {
                self.stats.unsolicited += 1;
            } else {
                self.stats.stale += 1;
            }
        }
    }

    /// Send one verb and wait for its response (§4.4.1, §4.4.2, §7.3).
    ///
    /// `verb` is the 20-bit verb field: `id << 8 | payload` for 12-bit verbs
    /// ([`codec::short`]), `id << 16 | payload` for 4-bit verbs
    /// ([`codec::long`]); the command word is `codec << 28 | nid << 20 |
    /// verb`. Waits up to 1000 × 10 µs. Unsolicited responses and answers
    /// not from `codec` are skipped and counted.
    pub fn verb(&mut self, codec: u8, nid: u8, verb: u32, delay: &mut dyn Delay) -> Result<u32, Error> {
        if codec >= MAX_CODECS || verb > 0xF_FFFF {
            return Err(Error::Range);
        }
        self.drain();
        let n = self.corb_entries.max(1);
        let wp = (self.corb_wp + 1) % n;
        let off = wp as usize * regs::CORB_ENTRY;
        if off + regs::CORB_ENTRY > self.mem.corb.len {
            return Err(Error::Range);
        }
        // A full ring means the controller stopped fetching.
        if (self.regs.read16(regs::CORBRP) & 0xFF) % n == wp {
            self.stats.timeouts += 1;
            return Err(Error::Timeout);
        }
        // SAFETY: `off + 4 <= corb.len` (checked above), the region is lent
        // to the driver, and the controller does not read entry `wp` before
        // CORBWP is moved to it below.
        unsafe {
            ptr::write_volatile(self.mem.corb.ptr.as_ptr().add(off) as *mut u32, command(codec, nid, verb).to_le())
        };
        fence(Ordering::SeqCst);
        self.regs.write16(regs::CORBWP, wp);
        self.corb_wp = wp;
        self.stats.verbs += 1;
        for _ in 0..=VERB_POLLS {
            while let Some((resp, ex)) = self.next_response() {
                if ex & rirb_ex::UNSOL != 0 {
                    self.stats.unsolicited += 1;
                } else if (ex & rirb_ex::CODEC_MASK) as u8 != codec {
                    self.stats.stale += 1;
                } else {
                    return Ok(resp);
                }
            }
            delay.delay_us(VERB_POLL_US);
        }
        self.stats.timeouts += 1;
        Err(Error::Timeout)
    }

    fn param(&mut self, codec: u8, nid: u8, p: u8, delay: &mut dyn Delay) -> Result<u32, Error> {
        self.verb(codec, nid, short(verb::GET_PARAMETER, p), delay)
    }

    /// Find codecs' output paths; keep the best (lowest rank, first found).
    fn discover(&mut self, delay: &mut dyn Delay) -> Result<(), InitError> {
        let mut best: Option<(OutputPath, u32)> = None;
        let mut timed_out = false;
        for c in 0..MAX_CODECS {
            if self.codecs & (1 << c) == 0 {
                continue;
            }
            match self.scan_codec(c, &mut best, delay) {
                Ok(()) => {}
                Err(Error::Timeout) => timed_out = true,
                Err(_) => {}
            }
        }
        let Some((mut path, vendor)) = best else {
            return Err(if timed_out { InitError::Timeout } else { InitError::NoOutputPath });
        };
        if self.table_of != Some((path.codec, path.afg)) {
            // The table holds a later AFG: read the winner again.
            self.scan_afg(path.codec, path.afg, delay).map_err(|_| InitError::Timeout)?;
            path = graph::find_output(self.widgets(), path.codec, path.afg).ok_or(InitError::NoOutputPath)?;
        }
        self.output = path;
        self.vendor = vendor;
        Ok(())
    }

    fn scan_codec(&mut self, c: u8, best: &mut Option<(OutputPath, u32)>, delay: &mut dyn Delay) -> Result<(), Error> {
        let vendor = self.param(c, 0, param::VENDOR_ID, delay)?;
        let nodes = self.param(c, 0, param::NODE_COUNT, delay)?;
        let start = (nodes >> 16) & 0xFF;
        let count = (nodes & 0xFF).min(MAX_FUNCTION_GROUPS);
        for fg in start..(start + count).min(256) {
            let fg = fg as u8;
            let t = self.param(c, fg, param::FUNCTION_GROUP_TYPE, delay)?;
            if t & 0xFF != FG_AUDIO {
                continue;
            }
            self.scan_afg(c, fg, delay)?;
            if let Some(p) = graph::find_output(self.widgets(), c, fg)
                && best.as_ref().is_none_or(|(b, _)| p.rank < b.rank)
            {
                *best = Some((p, vendor));
            }
        }
        Ok(())
    }

    /// Read the widgets of one AFG into the table.
    fn scan_afg(&mut self, c: u8, afg: u8, delay: &mut dyn Delay) -> Result<(), Error> {
        self.table_of = None;
        self.widget_count = 0;
        let afg_pcm = self.param(c, afg, param::PCM, delay)?;
        let afg_formats = self.param(c, afg, param::STREAM_FORMATS, delay)?;
        let afg_in = self.param(c, afg, param::IN_AMP_CAPS, delay)?;
        let afg_out = self.param(c, afg, param::OUT_AMP_CAPS, delay)?;
        let nodes = self.param(c, afg, param::NODE_COUNT, delay)?;
        let start = (nodes >> 16) & 0xFF;
        let count = (nodes & 0xFF).min(MAX_WIDGETS as u32);
        for nid in start..(start + count).min(256) {
            let nid = nid as u8;
            let caps = WidgetCaps(self.param(c, nid, param::WIDGET_CAPS, delay)?);
            let mut w = Widget::new(nid, caps);
            if matches!(caps.kind(), WidgetType::AudioOutput | WidgetType::AudioInput) {
                if caps.format_override() {
                    w.pcm = self.param(c, nid, param::PCM, delay)?;
                    w.stream_formats = self.param(c, nid, param::STREAM_FORMATS, delay)?;
                } else {
                    w.pcm = afg_pcm;
                    w.stream_formats = afg_formats;
                }
            }
            if caps.in_amp() {
                w.in_amp = codec::AmpCaps(if caps.amp_override() {
                    self.param(c, nid, param::IN_AMP_CAPS, delay)?
                } else {
                    afg_in
                });
            }
            if caps.out_amp() {
                w.out_amp = codec::AmpCaps(if caps.amp_override() {
                    self.param(c, nid, param::OUT_AMP_CAPS, delay)?
                } else {
                    afg_out
                });
            }
            if caps.kind() == WidgetType::PinComplex {
                w.pin_caps = PinCaps(self.param(c, nid, param::PIN_CAPS, delay)?);
                w.config = codec::ConfigDefault(self.verb(c, nid, short(verb::GET_CONFIG_DEFAULT, 0), delay)?);
            }
            if caps.conn_list() {
                self.read_connections(c, &mut w, delay)?;
            }
            let i = self.widget_count;
            if i < MAX_WIDGETS {
                self.widgets[i] = w;
                self.widget_count = i + 1;
            }
        }
        self.table_of = Some((c, afg));
        Ok(())
    }

    /// Read a widget's connection list (§7.3.3 Get Connection List Entry,
    /// §7.3.4.11), expanding ranges, at most `MAX_CONNECTIONS` entries.
    fn read_connections(&mut self, c: u8, w: &mut Widget, delay: &mut dyn Delay) -> Result<(), Error> {
        let lp = self.param(c, w.nid, param::CONN_LIST_LEN, delay)?;
        let len = lp & conn::LEN_MASK;
        let long_form = lp & conn::LONG_FORM != 0;
        let (per, width, range_bit, nid_mask) = if long_form {
            (2, 16, conn::LONG_RANGE, conn::LONG_NID)
        } else {
            (4, 8, conn::SHORT_RANGE, conn::SHORT_NID)
        };
        let mut prev: Option<u32> = None;
        let mut idx = 0;
        while idx < len && !w.conn_truncated {
            let resp = self.verb(c, w.nid, short(verb::GET_CONNECTION_LIST_ENTRY, idx as u8), delay)?;
            for k in 0..per {
                if idx + k >= len {
                    break;
                }
                let e = (resp >> (k * width)) & ((1 << width) - 1);
                let n = e & nid_mask;
                match prev {
                    Some(p) if e & range_bit != 0 && n > p => {
                        for x in p + 1..=n {
                            if !w.push_connection(if x <= 0xFF { x as u8 } else { 0 }) {
                                break;
                            }
                        }
                    }
                    _ => {
                        w.push_connection(if n <= 0xFF { n as u8 } else { 0 });
                    }
                }
                prev = Some(n);
            }
            idx += per;
        }
        if len as usize > crate::graph::MAX_CONNECTIONS {
            w.conn_truncated = true;
        }
        Ok(())
    }

    fn widget(&self, nid: u8) -> Option<Widget> {
        graph::position(self.widgets(), nid).map(|i| self.widgets[i])
    }

    /// Power up and unmute the chosen path.
    fn configure_path(&mut self, delay: &mut dyn Delay) -> Result<(), Error> {
        let p = self.output;
        let c = p.codec;
        self.verb(c, p.afg, short(verb::SET_POWER_STATE, power::D0), delay)?;
        for _ in 0..POWER_POLLS {
            let s = self.verb(c, p.afg, short(verb::GET_POWER_STATE, 0), delay)?;
            if (s >> 4) & 0xF == power::D0 as u32 {
                break;
            }
            delay.delay_us(POWER_POLL_US);
        }
        for i in 0..p.len.min(graph::MAX_PATH) {
            let Some(w) = self.widget(p.nodes[i]) else { continue };
            if w.caps.power_control() {
                self.verb(c, w.nid, short(verb::SET_POWER_STATE, power::D0), delay)?;
            }
            if w.caps.out_amp() {
                self.verb(
                    c,
                    w.nid,
                    long(verb::SET_AMP_GAIN_MUTE, amp::set(true, 0, false, w.out_amp.zero_db())),
                    delay,
                )?;
            }
            if i == 0 {
                continue;
            }
            let input = p.inputs[i];
            if w.caps.in_amp() {
                let payload = amp::set(false, input, false, w.in_amp.zero_db());
                self.verb(c, w.nid, long(verb::SET_AMP_GAIN_MUTE, payload), delay)?;
            }
            if w.kind() != WidgetType::Mixer && w.connections().len() > 1 {
                self.verb(c, w.nid, short(verb::SET_CONNECTION_SELECT, input), delay)?;
            }
        }
        if let Some(pin) = self.widget(p.pin) {
            let ctl = pinctl::OUT_EN | if pin.is_headphone() { pinctl::HP_EN } else { 0 };
            self.verb(c, pin.nid, short(verb::SET_PIN_WIDGET_CONTROL, ctl), delay)?;
            if pin.pin_caps.eapd() {
                self.verb(c, pin.nid, short(verb::SET_EAPD_BTL, eapd::EAPD), delay)?;
            }
        }
        Ok(())
    }

    /// Clear RUN and wait for the engine to stop; false on timeout.
    fn halt(&mut self, delay: &mut dyn Delay) -> bool {
        let ctl = self.osd() + sd::CTL;
        let v = self.regs.read8(ctl);
        self.regs.write8(ctl, v & !(sdctl::RUN | sdctl::IOCE | sdctl::FEIE | sdctl::DEIE));
        let regs = &self.regs;
        poll(delay, POLLS, POLL_US, || regs.read8(ctl) & sdctl::RUN == 0)
    }

    /// Program the output stream descriptor and the DAC (§3.3 SDn
    /// registers, §3.6 BDL, §4.5.1 stream setup).
    ///
    /// The stream is stopped and reset (SRST handshake), then: a BDL of
    /// `periods` entries of `period_bytes` each over the start of the
    /// buffer, IOC on every entry; CBL = `period_bytes × periods`; LVI =
    /// `periods − 1`; SDnFMT; stream tag [`OUTPUT_STREAM_TAG`]. The DAC gets
    /// the same format (Set Converter Format) and the stream tag with
    /// channel 0 (Set Converter Stream, Channel).
    ///
    /// Requires `periods` in 2..=[`MAX_BDL_ENTRIES`] (and BDL room for
    /// them), `period_bytes` a non-zero multiple of 128 (so every BDL buffer
    /// is 128-byte aligned, §3.6.2) and of the frame size, the total within
    /// the buffer: else `Range`. A format the path does not support is
    /// `Unsupported`. On error the stream stays stopped and unconfigured.
    pub fn configure_output(
        &mut self,
        fmt: StreamFormat,
        period_bytes: u32,
        periods: u32,
        delay: &mut dyn Delay,
    ) -> Result<(), Error> {
        let code = fmt.encode().ok_or(Error::Unsupported)?;
        if !self.output.supports(&fmt) {
            return Err(Error::Unsupported);
        }
        let frame = fmt.frame_bytes().ok_or(Error::Unsupported)?;
        if !(2..=MAX_BDL_ENTRIES as u32).contains(&periods)
            || periods as usize * BDL_ENTRY > self.mem.bdl.len
            || period_bytes == 0
            || !period_bytes.is_multiple_of(regs::DMA_ALIGN as u32)
            || !period_bytes.is_multiple_of(frame)
        {
            return Err(Error::Range);
        }
        let cbl = period_bytes.checked_mul(periods).ok_or(Error::Range)?;
        if cbl as usize > self.mem.buffer.len {
            return Err(Error::Range);
        }

        self.stream = None;
        let base = self.osd();
        if !self.halt(delay) {
            return Err(Error::Timeout);
        }
        // SRST handshake (§3.3, SDnCTL): 1 and wait for 1, 0 and wait for 0.
        let ctl = base + sd::CTL;
        self.regs.write8(ctl, sdctl::SRST);
        let regs = &self.regs;
        if !poll(delay, POLLS, POLL_US, || regs.read8(ctl) & sdctl::SRST != 0) {
            return Err(Error::Timeout);
        }
        self.regs.write8(ctl, 0);
        let regs = &self.regs;
        if !poll(delay, POLLS, POLL_US, || regs.read8(ctl) & sdctl::SRST == 0) {
            return Err(Error::Timeout);
        }

        for i in 0..periods as usize {
            let addr = self.mem.buffer.bus + (i as u64) * period_bytes as u64;
            let off = i * BDL_ENTRY;
            // SAFETY: `off + 16 <= periods * 16 <= bdl.len` (checked above),
            // the BDL is lent to the driver, and the stream is stopped, so
            // the controller does not read it now.
            unsafe {
                let p = self.mem.bdl.ptr.as_ptr().add(off);
                ptr::write_volatile(p as *mut u64, addr.to_le());
                ptr::write_volatile(p.add(8) as *mut u32, period_bytes.to_le());
                ptr::write_volatile(p.add(12) as *mut u32, regs::BDL_IOC.to_le());
            }
        }
        fence(Ordering::SeqCst);

        self.regs.write32(base + sd::BDPL, self.mem.bdl.bus as u32);
        self.regs.write32(base + sd::BDPU, (self.mem.bdl.bus >> 32) as u32);
        self.regs.write32(base + sd::CBL, cbl);
        self.regs.write16(base + sd::LVI, (periods - 1) as u16);
        self.regs.write16(base + sd::FMT, code);
        self.regs.write8(base + sd::CTL_HI, OUTPUT_STREAM_TAG << sdctl::STRM_SHIFT);
        self.regs.write8(base + sd::STS, sdsts::CLEAR);

        let (c, dac) = (self.output.codec, self.output.dac);
        self.verb(c, dac, long(verb::SET_CONVERTER_FORMAT, code), delay)?;
        self.verb(c, dac, short(verb::SET_CONVERTER_STREAM_CHANNEL, OUTPUT_STREAM_TAG << 4), delay)?;
        self.stream = Some(StreamSetup { format: fmt, cbl });
        Ok(())
    }

    /// The configured format, if any.
    pub fn output_format(&self) -> Option<StreamFormat> {
        self.stream.map(|s| s.format)
    }

    /// The cyclic buffer length configured (0 if none).
    pub fn cyclic_len(&self) -> u32 {
        self.stream.map_or(0, |s| s.cbl)
    }

    /// Start the output stream: SDnCTL RUN with the completion, FIFO error
    /// and descriptor error interrupt enables. Does nothing if the stream is
    /// not configured. Whether those reach the CPU is up to
    /// [`Hda::enable_interrupts`].
    pub fn start_output(&mut self) {
        if self.stream.is_none() {
            return;
        }
        let ctl = self.osd() + sd::CTL;
        self.regs.write8(self.osd() + sd::STS, sdsts::CLEAR);
        self.regs.write8(ctl, sdctl::RUN | sdctl::IOCE | sdctl::FEIE | sdctl::DEIE);
    }

    /// Stop the output stream: clear RUN and wait (up to 10 ms) for it to
    /// read back 0. The position is kept; [`Hda::configure_output`] resets
    /// it.
    pub fn stop_output(&mut self, delay: &mut dyn Delay) {
        self.halt(delay);
    }

    pub fn is_running(&self) -> bool {
        self.regs.read8(self.osd() + sd::CTL) & sdctl::RUN != 0
    }

    /// SDnLPIB: bytes the DMA engine has fetched within the cyclic buffer,
    /// 0..CBL. A value the controller reports at or beyond CBL is reduced
    /// modulo CBL; 0 when the stream is not configured.
    pub fn position(&self) -> u32 {
        let Some(s) = self.stream else { return 0 };
        let v = self.regs.read32(self.osd() + sd::LPIB);
        if s.cbl == 0 { 0 } else { v % s.cbl }
    }

    /// Interrupts on or off: INTCTL GIE and the output stream's SIE. The
    /// controller interrupt (CIE) stays off: the rings are polled.
    pub fn enable_interrupts(&mut self, on: bool) {
        let bit = self.output_stream_bit();
        let v = self.regs.read32(regs::INTCTL) & !(int::CONTROLLER | int::GLOBAL | bit);
        self.regs.write32(regs::INTCTL, if on { v | int::GLOBAL | bit } else { v });
        self.irq = on;
    }

    pub fn interrupts_enabled(&self) -> bool {
        self.irq
    }

    /// Read and clear the interrupt causes: the output stream's SDnSTS
    /// (BCIS, FIFOE, DESE), RIRBSTS, CORBSTS and STATESTS. Safe to call at
    /// any time; reports nothing when nothing happened.
    pub fn take_interrupt(&mut self) -> Interrupts {
        let intsts = self.regs.read32(regs::INTSTS);
        if intsts == u32::MAX {
            let r = Interrupts { intsts, gone: true, ..Interrupts::default() };
            self.stats.interrupts += 1;
            return r;
        }
        let mut r = Interrupts { intsts, ..Interrupts::default() };
        let sts_reg = self.osd() + sd::STS;
        let sts = self.regs.read8(sts_reg) & sdsts::CLEAR;
        if sts != 0 {
            self.regs.write8(sts_reg, sts);
        }
        r.buffer_complete = sts & sdsts::BCIS != 0;
        r.fifo_error = sts & sdsts::FIFOE != 0;
        r.descriptor_error = sts & sdsts::DESE != 0;
        let rs = self.regs.read8(regs::RIRBSTS) & (rirbsts::RINTFL | rirbsts::OIS);
        if rs != 0 {
            self.regs.write8(regs::RIRBSTS, rs);
        }
        r.response = rs & rirbsts::RINTFL != 0;
        r.response_overrun = rs & rirbsts::OIS != 0;
        let cs = self.regs.read8(regs::CORBSTS) & corbsts::CMEI;
        if cs != 0 {
            self.regs.write8(regs::CORBSTS, cs);
        }
        r.corb_error = cs != 0;
        let st = self.regs.read16(regs::STATESTS) & 0x7FFF;
        if st != 0 {
            self.regs.write16(regs::STATESTS, st);
        }
        r.codec_change = st;
        if r.any() {
            self.stats.interrupts += 1;
        }
        if r.buffer_complete {
            self.stats.buffer_completions += 1;
        }
        if r.fifo_error || r.descriptor_error {
            self.stats.stream_errors += 1;
        }
        r
    }

    pub fn buffer_len(&self) -> usize {
        self.mem.buffer.len
    }

    /// Copy `data` into the output buffer at `offset`; false (nothing
    /// written) if it does not fit.
    pub fn buffer_write(&mut self, offset: usize, data: &[u8]) -> bool {
        if offset.checked_add(data.len()).is_none_or(|end| end > self.mem.buffer.len) {
            return false;
        }
        // SAFETY: `offset + data.len() <= buffer.len` (checked above); the
        // buffer is lent to the driver and the controller only reads it.
        unsafe { ptr::copy_nonoverlapping(data.as_ptr(), self.mem.buffer.ptr.as_ptr().add(offset), data.len()) };
        true
    }

    /// Fill `len` bytes of the output buffer at `offset` with `byte`; false
    /// (nothing written) if out of range.
    pub fn buffer_fill(&mut self, offset: usize, len: usize, byte: u8) -> bool {
        if offset.checked_add(len).is_none_or(|end| end > self.mem.buffer.len) {
            return false;
        }
        // SAFETY: as in `buffer_write`.
        unsafe { ptr::write_bytes(self.mem.buffer.ptr.as_ptr().add(offset), byte, len) };
        true
    }

    /// Stop the stream and the CORB / RIRB engines and put the controller in
    /// reset; afterwards the controller no longer touches the memory and it
    /// may be reused. Returns the register block.
    pub fn shutdown(mut self, delay: &mut dyn Delay) -> M {
        self.enable_interrupts(false);
        self.halt(delay);
        self.regs.write8(regs::CORBCTL, 0);
        self.regs.write8(regs::RIRBCTL, 0);
        let regs = &self.regs;
        poll(delay, POLLS, POLL_US, || {
            regs.read8(regs::CORBCTL) & corbctl::RUN == 0 && regs.read8(regs::RIRBCTL) & rirbctl::DMAEN == 0
        });
        let g = self.regs.read32(regs::GCTL);
        self.regs.write32(regs::GCTL, g & !gctl::CRST);
        let regs = &self.regs;
        poll(delay, POLLS, POLL_US, || regs.read32(regs::GCTL) & gctl::CRST == 0);
        self.regs
    }
}
