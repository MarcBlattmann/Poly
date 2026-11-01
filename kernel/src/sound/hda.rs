//! Intel High Definition Audio over PCI (ADR 0109): the kernel's side of
//! the `poly-hda` driver, and the controller's output path as the audio
//! server's hardware ([`HdaHw`]).
//!
//! - finds the first function of class 04h, subclass 03h (an HDA
//!   controller: QEMU's `intel-hda` and `ich9-intel-hda`), maps BAR0,
//!   turns on memory decoding and bus mastering;
//! - gives it DMA memory: the CORB, the RIRB, the buffer descriptor list
//!   and the output ring buffer (all page-aligned, so 128-byte aligned);
//! - `poly_hda::init` resets the controller, finds the codecs, walks the
//!   widget graph to one output path (DAC to pin) and powers it up;
//! - plain MSI (one vector) wakes the audio thread; without it the device
//!   is polled.
//!
//! The ring buffer is [`PERIODS`] periods the DMA engine plays round and
//! round. Its position (LPIB) says how far it got; a period it has left
//! behind is free for the next mixed period. Interrupts (one per period,
//! IOC on every buffer descriptor) only wake the thread, which reads the
//! position. Capture on HDA is not done.

use super::HwFormat;
use crate::dma::DmaBuffer;
use crate::pci::{self, MmioRegion, MsiVector};
use crate::{kprintln, time};
use alloc::vec::Vec;
use poly_audio::{Format, Sample};
use poly_hda::{DmaRegion, Hda, Memory, StreamFormat};
use poly_pci::{Bdf, command};

/// Periods in the ring buffer, 20 ms each.
const PERIODS: u32 = 4;
const PERIOD_MS: u32 = 20;
const BUFFER_BYTES: usize = 64 * 1024;

/// Waits for the driver: spin on the clock.
fn delay(us: u32) {
    let end = time::now_ns() + us as u64 * 1000;
    while time::now_ns() < end {
        core::hint::spin_loop();
    }
}

fn region(b: &DmaBuffer) -> DmaRegion {
    DmaRegion { ptr: b.ptr(), bus: b.phys(), len: b.len() }
}

/// The controller as the server's hardware.
pub struct HdaHw {
    pub bdf: Bdf,
    hda: Hda<MmioRegion>,
    fmt: HwFormat,
    period_bytes: u32,
    /// Periods written since the stream started, and bytes the DMA engine
    /// has moved past (from the position).
    written: u64,
    played: u64,
    last_pos: u32,
    running: bool,
    interrupts: bool,
}

impl core::fmt::Debug for HdaHw {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HdaHw").field("bdf", &self.bdf).field("fmt", &self.fmt).finish_non_exhaustive()
    }
}

/// The stream format: 16-bit stereo at 48 kHz if the path takes it, else
/// 44.1 kHz, else its first rate; periods of about 20 ms whose byte count
/// is a multiple of 128 (the BDL's alignment).
fn choose(path: &poly_hda::OutputPath) -> Option<(StreamFormat, HwFormat)> {
    let channels = 2u8.min(path.max_channels.max(1));
    let rates: Vec<u32> = path.rates_hz().collect();
    let rate = [48000, 44100]
        .into_iter()
        .find(|r| path.supports(&StreamFormat::new(*r, 16, channels)))
        .or_else(|| rates.iter().copied().find(|r| path.supports(&StreamFormat::new(*r, 16, channels))))?;
    let sf = StreamFormat::new(rate, 16, channels);
    let fmt = Format::new(Sample::S16, channels, rate);
    let fb = fmt.frame_bytes() as u32;
    let step = 128 / gcd(128, fb);
    let period_frames = (rate * PERIOD_MS / 1000).div_ceil(step) * step;
    Some((sf, HwFormat { format: fmt, period_frames, periods: PERIODS }))
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn is_hda(d: &pci::Device) -> bool {
    d.header.class.class == 0x04 && d.header.class.subclass == 0x03
}

/// Find the first HDA controller and bring it up, with `handler` on its
/// MSI if it has one. Logs what it found.
pub fn probe(handler: crate::arch::irq::Handler) -> Option<(HdaHw, Option<MsiVector>)> {
    let dev = pci::devices().iter().find(|d| is_hda(d))?;
    let bdf = dev.bdf();
    let regs = match dev.map_bar(0) {
        Ok(r) => r,
        Err(e) => {
            kprintln!("sound: {bdf}: HDA controller BAR0 not mapped: {e:?}");
            return None;
        }
    };
    poly_pci::enable(dev.config(), bdf, command::MEMORY_SPACE | command::BUS_MASTER);
    let mut memory = Vec::new();
    for len in [4096, 4096, 4096, BUFFER_BYTES] {
        memory.push(DmaBuffer::new(len)?);
    }
    let mem = Memory {
        corb: region(&memory[0]),
        rirb: region(&memory[1]),
        bdl: region(&memory[2]),
        buffer: region(&memory[3]),
    };
    let mut wait = delay;
    // SAFETY: the DMA buffers are fresh, zeroed, page-aligned and
    // direct-mapped, reached by the device at their physical addresses (no
    // IOMMU); they are leaked below on success, so they stay valid and used
    // by nothing else for the kernel's lifetime. `regs` is this
    // controller's BAR0.
    let mut hda = match unsafe { poly_hda::init(regs, mem, &mut wait) } {
        Ok(h) => h,
        Err(e) => {
            if let Ok(r) = dev.map_bar(0) {
                use poly_pci::Mmio;
                kprintln!(
                    "sound: {bdf}: HDA debug: GCTL {:#x} STATESTS {:#x} CORBWP {:#x} CORBRP {:#x} CORBCTL {:#x} RIRBWP {:#x} RIRBCTL {:#x} CORBSIZE {:#x} RIRBSIZE {:#x} RIRBSTS {:#x} RINTCNT {:#x} INTSTS {:#x}",
                    r.read32(0x08),
                    r.read16(0x0e),
                    r.read16(0x48),
                    r.read16(0x4a),
                    r.read8(0x4c),
                    r.read16(0x58),
                    r.read8(0x5c),
                    r.read8(0x4e),
                    r.read8(0x5e),
                    r.read8(0x5d),
                    r.read16(0x5a),
                    r.read32(0x24)
                );
            }
            kprintln!("sound: {bdf}: HDA controller not usable: {e:?}");
            // The controller may still know the ring addresses: never reuse
            // the memory.
            memory.into_iter().for_each(core::mem::forget);
            return None;
        }
    };
    memory.into_iter().for_each(core::mem::forget);
    let g = hda.gcap();
    let path = *hda.output();
    kprintln!(
        "sound: {bdf}: Intel HDA controller, {} output / {} input streams, codecs {:#06b}, codec {} vendor {:#010x}",
        g.output_streams,
        g.input_streams,
        hda.codecs(),
        path.codec,
        hda.vendor_id()
    );
    hda.each_widget(|codec, nid, ty, caps| {
        kprintln!("sound: hda codec {codec} widget {nid}: {ty:?}, caps {caps:#010x}")
    });
    kprintln!(
        "sound: hda output path {:?} (DAC {} to pin {}), rates {:#x}, sizes {:#x}, {} channels",
        path.nodes(),
        path.dac,
        path.pin,
        path.rates,
        path.sizes,
        path.max_channels
    );
    let Some((sf, fmt)) = choose(&path) else {
        kprintln!("sound: {bdf}: the HDA output path takes no 16-bit format; no sound");
        return None;
    };
    let period_bytes = fmt.period_bytes() as u32;
    if let Err(e) = hda.configure_output(sf, period_bytes, PERIODS, &mut wait) {
        kprintln!("sound: {bdf}: HDA output stream not configured: {e:?}");
        return None;
    }
    let msi = match dev.enable_msi(handler, 0) {
        Ok(m) => Some(m),
        Err(e) => {
            kprintln!("sound: {bdf}: no MSI ({e:?}); polled");
            None
        }
    };
    let hw =
        HdaHw { bdf, hda, fmt, period_bytes, written: 0, played: 0, last_pos: 0, running: false, interrupts: false };
    Some((hw, msi))
}

impl HdaHw {
    pub fn output(&self) -> Option<HwFormat> {
        Some(self.fmt)
    }

    pub fn input(&self) -> Option<HwFormat> {
        None
    }

    /// Let the controller interrupt (once MSI is connected).
    pub fn connect_interrupts(&mut self) {
        self.hda.enable_interrupts(true);
        self.interrupts = true;
    }

    /// Clear interrupt causes and move the play position on.
    pub fn collect(&mut self) {
        if self.interrupts {
            let i = self.hda.take_interrupt();
            if i.fifo_error || i.descriptor_error {
                kprintln!("sound: {}: HDA stream error ({i:?})", self.bdf);
            }
        }
        if self.running {
            let cbl = self.hda.cyclic_len().max(1);
            let pos = self.hda.position() % cbl;
            self.played += ((pos + cbl - self.last_pos) % cbl) as u64;
            self.last_pos = pos;
        }
    }

    pub fn out_start(&mut self) -> bool {
        if self.running {
            self.hda.stop_output(&mut delay);
            self.running = false;
        }
        let sf = StreamFormat::new(self.fmt.format.rate, 16, self.fmt.format.channels);
        if let Err(e) = self.hda.configure_output(sf, self.period_bytes, PERIODS, &mut delay) {
            kprintln!("sound: {}: HDA output stream not configured: {e:?}", self.bdf);
            return false;
        }
        self.written = 0;
        self.played = 0;
        self.last_pos = 0;
        true
    }

    /// Periods free: those the DMA engine has left behind.
    pub fn out_room(&mut self) -> usize {
        let done = self.played / self.period_bytes as u64;
        (done + PERIODS as u64).saturating_sub(self.written) as usize
    }

    pub fn out_push(&mut self, period: &[u8]) {
        let at = (self.written % PERIODS as u64) as usize * self.period_bytes as usize;
        if self.hda.buffer_write(at, &period[..period.len().min(self.period_bytes as usize)]) {
            self.written += 1;
        }
    }

    pub fn out_kick(&mut self) {
        if !self.running && self.written > 0 {
            self.hda.start_output();
            self.running = true;
        }
    }

    pub fn out_stop(&mut self) {
        if self.running {
            self.hda.stop_output(&mut delay);
            self.running = false;
        }
    }

    pub fn in_start(&mut self) -> bool {
        false
    }

    pub fn in_pop(&mut self, _out: &mut [u8]) -> Option<usize> {
        None
    }

    pub fn in_stop(&mut self) {}

    /// Interrupts stay on; nothing to re-arm.
    pub fn enable_interrupts(&mut self) -> bool {
        false
    }

    pub fn is_failed(&self) -> bool {
        false
    }
}
