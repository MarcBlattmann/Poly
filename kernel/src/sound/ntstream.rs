//! The streams of `\Device\PolyAudio` (ADR 0244): the Windows side's
//! clients of the audio server, beside the ALSA substreams.
//!
//! A stream is the program's frames in a ring buffer of the kernel's
//! (`poly_audio::nt::Ring` keeps the count), written by its device
//! requests, never mapped. The card keeps a weak reference to each; every
//! output period the server mixes the running ones like ALSA clients
//! (converted to the device's format, with the stream's own gains), and a
//! running stream that ran out plays silence for the rest of its share.
//! A stream's frames count as played when the device finished the period
//! they went out in (each stream keeps its frames per period in flight),
//! not when the mixer took them: the device holds several periods. When
//! frames were played, the `nt-audio` worker (`crate::nt::audio`) is woken
//! to finish the waits that are due.
//!
//! Lock order: the card's lock, then a stream's.

use super::{card, kick};
use crate::sync::Mutex;
use crate::thread::WaitQueue as ThreadQueue;
use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use poly_audio::nt::{DeviceInfo, Ring, StreamInfo};
use poly_audio::server::{Source, mix_source};
use poly_audio::{Converter, Format, MAX_CHANNELS, UNITY};
use poly_ntobj::NtStatus;
use poly_personality_nt::audio::{NtAudioStream, STATUS_NO_SUCH_DEVICE};

/// Bumped whenever a stream's frames were played; the worker waits on
/// [`PLAYED`] for it to move.
pub static PLAYED_GEN: AtomicU64 = AtomicU64::new(0);
pub static PLAYED: ThreadQueue = ThreadQueue::new();
/// Frames were played since the audio thread last looked.
static HEARD: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
struct State {
    ring: Ring,
    bytes: Vec<u8>,
    gains: [u32; MAX_CHANNELS],
    /// From the stream's format to the device's.
    conv: Converter,
    /// Its frames in each device period in flight, oldest first.
    in_flight: VecDeque<u32>,
}

/// One stream.
#[derive(Debug)]
pub struct NtStream {
    format: Format,
    state: Mutex<State>,
}

/// The card's output stream.
pub fn device() -> Option<DeviceInfo> {
    let hw = card()?.inner.lock().hw.output()?;
    Some(DeviceInfo { format: hw.format, period_frames: hw.period_frames, periods: hw.periods })
}

/// A new stream of `format` with a buffer of `frames` (the request was
/// checked: `poly_audio::nt::OpenReq::decode`).
pub fn open(format: Format, frames: u32) -> Result<Arc<NtStream>, NtStatus> {
    let card = card().ok_or(STATUS_NO_SUCH_DEVICE)?;
    let mut g = card.inner.lock();
    let hw = g.hw.output().ok_or(STATUS_NO_SUCH_DEVICE)?;
    let conv = Converter::new(format, hw.format).ok_or(poly_ntobj::status::STATUS_INVALID_PARAMETER)?;
    let bytes = vec![0u8; frames as usize * format.frame_bytes()];
    // The periods already in flight carry none of its frames.
    let in_flight = core::iter::repeat_n(0, g.out.in_flight as usize).collect();
    let state = State { ring: Ring::new(frames), bytes, gains: [UNITY; MAX_CHANNELS], conv, in_flight };
    let s = Arc::new(NtStream { format, state: Mutex::new(state) });
    g.nt.retain(|w| w.strong_count() > 0);
    g.nt.push(Arc::downgrade(&s));
    Ok(s)
}

impl NtAudioStream for NtStream {
    fn write(&self, data: &[u8]) -> u32 {
        let fb = self.format.frame_bytes();
        let mut g = self.state.lock();
        let State { ring, bytes, .. } = &mut *g;
        let spans = ring.write((data.len() / fb) as u32);
        let mut from = 0;
        for (at, n) in [spans.first, spans.second] {
            let len = n as usize * fb;
            let at = at as usize * fb;
            bytes[at..at + len].copy_from_slice(&data[from..from + len]);
            from += len;
        }
        spans.frames()
    }

    fn start(&self) {
        self.state.lock().ring.start();
        // The device stream may have to start.
        kick();
    }

    fn stop(&self) {
        self.state.lock().ring.stop();
    }

    fn reset(&self) {
        let mut g = self.state.lock();
        g.ring.reset();
        g.conv.reset();
    }

    fn set_gains(&self, gains: [u32; MAX_CHANNELS]) {
        self.state.lock().gains = gains;
    }

    fn info(&self) -> StreamInfo {
        self.state.lock().ring.info()
    }

    fn format(&self) -> Format {
        self.format
    }
}

/// A running stream as the mixer's source.
struct Play<'a>(&'a mut State);

impl Source for Play<'_> {
    fn converter(&mut self) -> &mut Converter {
        &mut self.0.conv
    }

    fn gains(&self) -> [u32; MAX_CHANNELS] {
        self.0.gains
    }

    fn peek(&mut self, out: &mut [u8]) -> usize {
        let fb = self.0.conv.from().frame_bytes();
        let spans = self.0.ring.peek((out.len() / fb) as u32);
        let mut to = 0;
        for (at, n) in [spans.first, spans.second] {
            let len = n as usize * fb;
            let at = at as usize * fb;
            out[to..to + len].copy_from_slice(&self.0.bytes[at..at + len]);
            to += len;
        }
        spans.frames() as usize
    }

    fn consume(&mut self, frames: usize) {
        self.0.ring.consume(frames as u32);
    }
}

/// Is any stream running (the device stream must run)?
pub(super) fn any_running(nt: &[Weak<NtStream>]) -> bool {
    nt.iter().filter_map(Weak::upgrade).any(|s| s.state.lock().ring.running())
}

/// Mix every running stream into `acc` (one period of the device's
/// frames, about to be handed over); the ones that ran out count the
/// silence. Every stream notes its frames in this period.
pub(super) fn mix(nt: &mut Vec<Weak<NtStream>>, acc: &mut [i32], scratch: &mut [u8], device: Format) {
    nt.retain(|w| w.strong_count() > 0);
    let frames = acc.len() / device.channels as usize;
    for s in nt.iter().filter_map(Weak::upgrade) {
        let mut g = s.state.lock();
        if !g.ring.running() {
            g.in_flight.push_back(0);
            continue;
        }
        let m = mix_source(&mut Play(&mut g), acc, scratch);
        g.in_flight.push_back(m.consumed as u32);
        if m.produced < frames {
            // The rest of its share, in its own frames.
            let short = ((frames - m.produced) as u64 * s.format.rate as u64).div_ceil(device.rate as u64);
            g.ring.ran_short(short as u32);
        }
    }
}

/// The device finished `periods` periods (all of them: `None`, when it
/// stopped): their frames are played.
pub(super) fn completed(nt: &[Weak<NtStream>], periods: Option<u32>) {
    for s in nt.iter().filter_map(Weak::upgrade) {
        let mut g = s.state.lock();
        let n = periods.map_or(g.in_flight.len(), |p| (p as usize).min(g.in_flight.len()));
        let frames: u64 = g.in_flight.drain(..n).map(u64::from).sum();
        if frames > 0 {
            g.ring.heard(frames);
            HEARD.store(true, SeqCst);
        }
    }
}

/// After a round of the server: wake the worker if frames were played.
pub(super) fn after_round() {
    if HEARD.swap(false, SeqCst) {
        PLAYED_GEN.fetch_add(1, SeqCst);
        PLAYED.wake_all();
    }
}
