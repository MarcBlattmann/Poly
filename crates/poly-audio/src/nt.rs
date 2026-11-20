//! Poly's own protocol between the Windows side's sound DLLs and
//! `\Device\PolyAudio`, the NT personality's way into the audio server
//! (ADR 0244), and the accounting of the streams behind it ([`Ring`]).
//!
//! A program's stream is a file object on `\Device\PolyAudio` (opened with
//! `NtCreateFile`, closed with `NtClose`); everything else is
//! `NtDeviceIoControlFile` with the codes below. Windows' own interface
//! below its sound APIs is not documented, so these codes and buffers are
//! Poly's, chosen like the console's and AFD's: `CTL_CODE(FILE_DEVICE_SOUND,
//! function, METHOD_NEITHER, FILE_ANY_ACCESS)` with functions from 0x800,
//! the range Learn ("Defining I/O Control Codes") leaves to vendors.
//! Buffers are little-endian with fixed offsets.
//!
//! | Code | Input | Output | `Information` |
//! |---|---|---|---|
//! | [`DEVICE`] | | [`DeviceInfo`] | its size |
//! | [`OPEN`] | [`OpenReq`] | | |
//! | [`WRITE`] | whole frames in the stream's format | | frames taken |
//! | [`START`], [`STOP`], [`RESET`] | | | |
//! | [`SET_GAIN`] | 1 to [`MAX_CHANNELS`] gains (u32, 16.16) | | |
//! | [`QUERY`] | | [`StreamInfo`] | its size |
//! | [`WAIT`] | a played-frame count (u64) | the played-frame count (u64) | 8 |
//!
//! - [`DEVICE`] answers on any handle; `STATUS_NO_SUCH_DEVICE` when the
//!   machine has no sound card.
//! - [`OPEN`] makes the handle's stream, once (`STATUS_INVALID_DEVICE_STATE`
//!   the second time; every other stream request before it, too).
//! - [`WRITE`] never waits: it takes as many whole frames as the stream's
//!   buffer has room for, maybe none.
//! - [`START`] lets the mixer take the stream's frames, [`STOP`] holds it
//!   where it is, [`RESET`] stops it, drops its frames, sets its counters to
//!   zero and cancels its waits (`STATUS_CANCELLED`).
//! - [`WAIT`] finishes when the mixer has taken that many frames
//!   ([`StreamInfo::played`]), at once if it has; else it is pending until
//!   then, or until the stream is reset, the request cancelled or the
//!   handle closed. A synchronous handle waits in the call.
//!
//! A running stream that runs out of frames plays silence for the rest of
//! its share of a period: [`StreamInfo::silence`] counts those frames, and
//! [`StreamInfo::underruns`] each time it ran dry.

use crate::{Format, MAX_CHANNELS, Sample};

/// The device the sound DLLs open.
pub const DEVICE_NAME: &str = "\\Device\\PolyAudio";

/// `FILE_DEVICE_SOUND` (`devioctl.h`).
pub const FILE_DEVICE_SOUND: u32 = 0x1d;
/// `METHOD_NEITHER` (`devioctl.h`).
pub const METHOD_NEITHER: u32 = 3;

pub const fn code(function: u32) -> u32 {
    (FILE_DEVICE_SOUND << 16) | (function << 2) | METHOD_NEITHER
}

pub const DEVICE: u32 = code(0x800);
pub const OPEN: u32 = code(0x801);
pub const WRITE: u32 = code(0x802);
pub const START: u32 = code(0x803);
pub const STOP: u32 = code(0x804);
pub const RESET: u32 = code(0x805);
pub const SET_GAIN: u32 = code(0x806);
pub const QUERY: u32 = code(0x807);
pub const WAIT: u32 = code(0x808);

/// The largest stream buffer, in bytes (two seconds of 8 channels of
/// 64-bit samples at 48 kHz is less).
pub const MAX_BUFFER_BYTES: usize = 8 << 20;

/// The sample formats by their number in [`OpenReq`] and [`DeviceInfo`].
pub const SAMPLES: [Sample; 12] = [
    Sample::U8,
    Sample::S8,
    Sample::S16,
    Sample::U16,
    Sample::S24,
    Sample::U24,
    Sample::S24_3,
    Sample::U24_3,
    Sample::S32,
    Sample::U32,
    Sample::Float,
    Sample::Float64,
];

/// A sample format's number.
pub fn sample_code(s: Sample) -> u32 {
    SAMPLES.iter().position(|&x| x == s).unwrap_or(0) as u32
}

/// The sample format of number `code`.
pub fn sample_of(code: u32) -> Option<Sample> {
    SAMPLES.get(code as usize).copied()
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

/// The sound card's output stream, which every stream is mixed into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub format: Format,
    /// Frames of one device period, and periods the device holds.
    pub period_frames: u32,
    pub periods: u32,
}

impl DeviceInfo {
    pub const SIZE: usize = 20;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        put32(&mut b, 0, sample_code(self.format.sample));
        put32(&mut b, 4, self.format.channels as u32);
        put32(&mut b, 8, self.format.rate);
        put32(&mut b, 12, self.period_frames);
        put32(&mut b, 16, self.periods);
        b
    }

    pub fn decode(b: &[u8]) -> Option<DeviceInfo> {
        let sample = sample_of(u32_at(b, 0)?)?;
        let channels = u8::try_from(u32_at(b, 4)?).ok()?;
        Some(DeviceInfo {
            format: Format::new(sample, channels, u32_at(b, 8)?),
            period_frames: u32_at(b, 12)?,
            periods: u32_at(b, 16)?,
        })
    }
}

/// [`OPEN`]'s input: the stream's format and its buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenReq {
    pub format: Format,
    pub buffer_frames: u32,
}

impl OpenReq {
    pub const SIZE: usize = 16;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        put32(&mut b, 0, sample_code(self.format.sample));
        put32(&mut b, 4, self.format.channels as u32);
        put32(&mut b, 8, self.format.rate);
        put32(&mut b, 12, self.buffer_frames);
        b
    }

    /// The request, if it is one a stream can have: a known sample format,
    /// 1 to [`MAX_CHANNELS`] channels, a rate, and a buffer of at least one
    /// frame and at most [`MAX_BUFFER_BYTES`].
    pub fn decode(b: &[u8]) -> Option<OpenReq> {
        let sample = sample_of(u32_at(b, 0)?)?;
        let channels = u8::try_from(u32_at(b, 4)?).ok()?;
        let format = Format::new(sample, channels, u32_at(b, 8)?);
        let buffer_frames = u32_at(b, 12)?;
        let bytes = (buffer_frames as usize).checked_mul(format.frame_bytes())?;
        (format.is_valid() && buffer_frames > 0 && bytes <= MAX_BUFFER_BYTES)
            .then_some(OpenReq { format, buffer_frames })
    }
}

/// [`SET_GAIN`]'s input as a gain per channel: one value is every
/// channel's; with fewer than the channels, the last one repeats.
pub fn gains_of(b: &[u8]) -> Option<[u32; MAX_CHANNELS]> {
    let n = b.len() / 4;
    if n == 0 || n > MAX_CHANNELS || !b.len().is_multiple_of(4) {
        return None;
    }
    let mut g = [0u32; MAX_CHANNELS];
    for (c, v) in g.iter_mut().enumerate() {
        *v = u32_at(b, c.min(n - 1) * 4)?;
    }
    Some(g)
}

/// [`QUERY`]'s output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamInfo {
    /// Frames the program wrote since the stream was opened or reset.
    pub written: u64,
    /// Frames of them the device has played: the mixer took them and the device finished the
    /// period they went out in.
    pub played: u64,
    /// Frames of silence the stream played while running without frames.
    pub silence: u64,
    /// Times it ran dry while running.
    pub underruns: u32,
    pub running: bool,
    pub buffer_frames: u32,
    /// Frames written and not yet taken by the mixer (the buffer's room is the rest).
    pub queued: u32,
    /// The device's output stream ([`DeviceInfo`]): its rate, period and
    /// periods, so the program can tell how far behind the speaker the
    /// mixer is.
    pub device_rate: u32,
    pub device_period_frames: u32,
    pub device_periods: u32,
}

impl StreamInfo {
    pub const SIZE: usize = 56;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        put64(&mut b, 0, self.written);
        put64(&mut b, 8, self.played);
        put64(&mut b, 16, self.silence);
        put32(&mut b, 24, self.underruns);
        put32(&mut b, 28, u32::from(self.running));
        put32(&mut b, 32, self.buffer_frames);
        put32(&mut b, 36, self.queued);
        put32(&mut b, 40, self.device_rate);
        put32(&mut b, 44, self.device_period_frames);
        put32(&mut b, 48, self.device_periods);
        b
    }

    pub fn decode(b: &[u8]) -> Option<StreamInfo> {
        Some(StreamInfo {
            written: u64_at(b, 0)?,
            played: u64_at(b, 8)?,
            silence: u64_at(b, 16)?,
            underruns: u32_at(b, 24)?,
            running: u32_at(b, 28)? != 0,
            buffer_frames: u32_at(b, 32)?,
            queued: u32_at(b, 36)?,
            device_rate: u32_at(b, 40)?,
            device_period_frames: u32_at(b, 44)?,
            device_periods: u32_at(b, 48)?,
        })
    }
}

/// Where frames go in (or come from) a stream's buffer: up to two pieces,
/// the second from the start when the first reaches the end. Offsets and
/// lengths are in frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spans {
    pub first: (u32, u32),
    pub second: (u32, u32),
}

impl Spans {
    pub fn frames(&self) -> u32 {
        self.first.1 + self.second.1
    }
}

/// The accounting of one stream's ring buffer (the bytes are the
/// caller's): frames written, taken by the mixer and played by the device,
/// whether it runs, and the silence it played when it ran dry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ring {
    capacity: u32,
    written: u64,
    taken: u64,
    played: u64,
    silence: u64,
    underruns: u32,
    running: bool,
    dry: bool,
}

impl Ring {
    pub const fn new(capacity: u32) -> Ring {
        Ring { capacity, written: 0, taken: 0, played: 0, silence: 0, underruns: 0, running: false, dry: false }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn queued(&self) -> u32 {
        (self.written - self.taken) as u32
    }

    pub fn room(&self) -> u32 {
        self.capacity - self.queued()
    }

    pub fn running(&self) -> bool {
        self.running
    }

    /// Frames the mixer took.
    pub fn taken(&self) -> u64 {
        self.taken
    }

    /// Frames the device played.
    pub fn played(&self) -> u64 {
        self.played
    }

    fn spans(&self, from: u64, frames: u32) -> Spans {
        let at = (from % self.capacity as u64) as u32;
        let first = frames.min(self.capacity - at);
        Spans { first: (at, first), second: (0, frames - first) }
    }

    /// Take up to `frames` new frames (as many as there is room for):
    /// where they go.
    pub fn write(&mut self, frames: u32) -> Spans {
        let n = frames.min(self.room());
        let s = self.spans(self.written, n);
        self.written += n as u64;
        s
    }

    /// The next `frames` frames to play (at most those queued), not yet
    /// used up.
    pub fn peek(&self, frames: u32) -> Spans {
        self.spans(self.taken, frames.min(self.queued()))
    }

    /// The mixer used `frames` of them.
    pub fn consume(&mut self, frames: u32) {
        let n = frames.min(self.queued());
        self.taken += n as u64;
        if n > 0 {
            self.dry = false;
        }
    }

    /// The device finished playing `frames` more of what the mixer took.
    pub fn heard(&mut self, frames: u64) {
        self.played = (self.played + frames).min(self.taken);
    }

    /// The mixer's period ended `short` frames (in the stream's rate)
    /// before the stream's share of it: it played silence there. The first
    /// period of a dry spell counts an underrun.
    pub fn ran_short(&mut self, short: u32) {
        if !self.running || short == 0 {
            return;
        }
        self.silence += short as u64;
        if !self.dry {
            self.dry = true;
            self.underruns += 1;
        }
    }

    pub fn start(&mut self) {
        self.running = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.dry = false;
    }

    /// Stopped, empty and counted from zero again (the underruns stay).
    pub fn reset(&mut self) {
        *self = Ring { underruns: self.underruns, ..Ring::new(self.capacity) };
    }

    /// What [`QUERY`] reports of it (the device's fields left 0).
    pub fn info(&self) -> StreamInfo {
        StreamInfo {
            written: self.written,
            played: self.played,
            silence: self.silence,
            underruns: self.underruns,
            running: self.running,
            buffer_frames: self.capacity,
            queued: self.queued(),
            ..StreamInfo::default()
        }
    }
}
