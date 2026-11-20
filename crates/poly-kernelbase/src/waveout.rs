//! The logic of our `winmm.dll` wave output (`waveOut*`; the sheets in `specs/windows/winmm.dll/`,
//! constants and layouts from `mmsystem.h`, `mmeapi.h`, `mmreg.h` and `ksmedia.h`): which formats
//! a query and an open accept, what prepare, unprepare and write do to a header's flags, the
//! order buffers (and loops) play in and when each is done, the error texts, and the
//! capabilities structures. The DLL keeps the handles, feeds `\Device\PolyAudio` (ADR 0244) and
//! calls back.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use poly_audio::{Format, Sample};

// ----- results (`mmsystem.h`, `mmeapi.h`)

pub const MMSYSERR_NOERROR: u32 = 0;
pub const MMSYSERR_BADDEVICEID: u32 = 2;
pub const MMSYSERR_INVALHANDLE: u32 = 5;
pub const MMSYSERR_NOMEM: u32 = 7;
pub const MMSYSERR_NOTSUPPORTED: u32 = 8;
pub const MMSYSERR_BADERRNUM: u32 = 9;
pub const MMSYSERR_INVALFLAG: u32 = 10;
pub const MMSYSERR_INVALPARAM: u32 = 11;
pub const WAVERR_BADFORMAT: u32 = 32;
pub const WAVERR_STILLPLAYING: u32 = 33;
pub const WAVERR_UNPREPARED: u32 = 34;

// ----- ids, flags and messages

/// `WAVE_MAPPER`.
pub const WAVE_MAPPER: u32 = 0xFFFF_FFFF;
pub const WAVE_FORMAT_QUERY: u32 = 0x1;
pub const WAVE_ALLOWSYNC: u32 = 0x2;
pub const WAVE_MAPPED: u32 = 0x4;
pub const WAVE_FORMAT_DIRECT: u32 = 0x8;
pub const CALLBACK_TYPEMASK: u32 = 0x0007_0000;
pub const CALLBACK_NULL: u32 = 0;
pub const CALLBACK_WINDOW: u32 = 0x0001_0000;
pub const CALLBACK_THREAD: u32 = 0x0002_0000;
pub const CALLBACK_FUNCTION: u32 = 0x0003_0000;
pub const CALLBACK_EVENT: u32 = 0x0005_0000;
/// The flags `waveOutOpen` accepts; any other bit (`WAVE_MAPPED_DEFAULT_COMMUNICATION_DEVICE`,
/// 0x10, among them) is `MMSYSERR_INVALFLAG`.
pub const OPEN_FLAGS: u32 = WAVE_FORMAT_QUERY | WAVE_ALLOWSYNC | WAVE_MAPPED | WAVE_FORMAT_DIRECT | CALLBACK_TYPEMASK;

pub const WOM_OPEN: u32 = 0x3BB;
pub const WOM_CLOSE: u32 = 0x3BC;
pub const WOM_DONE: u32 = 0x3BD;

pub const WHDR_DONE: u32 = 0x1;
pub const WHDR_PREPARED: u32 = 0x2;
pub const WHDR_BEGINLOOP: u32 = 0x4;
pub const WHDR_ENDLOOP: u32 = 0x8;
pub const WHDR_INQUEUE: u32 = 0x10;
/// The flags a header may have; any other bit makes prepare and unprepare `MMSYSERR_INVALPARAM`.
pub const WHDR_VALID: u32 = 0x1F;

pub const TIME_SAMPLES: u32 = 0x2;
pub const TIME_BYTES: u32 = 0x4;

/// `DRVM_MAPPER_PREFERRED_GET` (`mmddk.h`: `DRVM_MAPPER` 0x2000 + 21).
pub const DRVM_MAPPER_PREFERRED_GET: u32 = 0x2015;

// ----- formats (`mmreg.h`, `ksmedia.h`)

pub const WAVE_FORMAT_PCM: u16 = 1;
pub const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
pub const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
/// `WAVEFORMATEX` without `cbSize`'s extra bytes: 18 bytes.
pub const WAVEFORMATEX_SIZE: usize = 18;
/// `WAVEFORMATEXTENSIBLE`'s extra bytes (`cbSize`): 22.
pub const EXTENSIBLE_EXTRA: u16 = 22;
/// `KSDATAFORMAT_SUBTYPE_PCM` and `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT` (`ksmedia.h`): the format
/// tag in the first two bytes of `{0000xxxx-0000-0010-8000-00aa00389b71}`.
const SUBTYPE_TAIL: [u8; 14] = [0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71];

/// An open's format as the device stream takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaveFormat {
    pub format: Format,
    /// Bytes per frame (`nBlockAlign` as the format implies it).
    pub block: u32,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// The integer sample of `bits` bits (`valid` of them significant), or none.
fn int_sample(bits: u16, valid: u16) -> Option<Sample> {
    match (bits, valid) {
        (8, 8) => Some(Sample::U8),
        (16, 16) => Some(Sample::S16),
        (24, 24) => Some(Sample::S24_3),
        // 24 significant bits at the top of 32: what S32 reads.
        (32, 32) | (32, 24) => Some(Sample::S32),
        _ => None,
    }
}

/// Check `fmt` (a `WAVEFORMATEX`, and its extra bytes for `WAVE_FORMAT_EXTENSIBLE`) for a query
/// (`query`) or an open. `waveOutOpen.yaml`: a tag other than PCM, float or extensible, and an
/// unknown sub-format, are `WAVERR_BADFORMAT`; an extensible format with too few extra bytes is
/// `MMSYSERR_INVALPARAM`; a width that is not a sample's, other than one or two channels, a rate
/// of 0 and more valid bits than container bits are `WAVERR_BADFORMAT` as a query and
/// `MMSYSERR_INVALPARAM` as an open; fewer valid bits than the container's pass the query and
/// are `WAVERR_BADFORMAT` as an open. `fmt` holds as many bytes as the caller could give (at
/// least [`WAVEFORMATEX_SIZE`]).
pub fn check_format(fmt: &[u8], query: bool) -> Result<WaveFormat, u32> {
    if fmt.len() < WAVEFORMATEX_SIZE {
        return Err(MMSYSERR_INVALPARAM);
    }
    let tag = u16_at(fmt, 0);
    let channels = u16_at(fmt, 2);
    let rate = u32_at(fmt, 4);
    let bits = u16_at(fmt, 14);
    let shape = if query { WAVERR_BADFORMAT } else { MMSYSERR_INVALPARAM };
    let (float, valid) = match tag {
        WAVE_FORMAT_PCM => (false, bits),
        WAVE_FORMAT_IEEE_FLOAT => (true, bits),
        WAVE_FORMAT_EXTENSIBLE => {
            if u16_at(fmt, 16) < EXTENSIBLE_EXTRA || fmt.len() < WAVEFORMATEX_SIZE + EXTENSIBLE_EXTRA as usize {
                return Err(MMSYSERR_INVALPARAM);
            }
            let sub = &fmt[24..40];
            if sub[2..] != SUBTYPE_TAIL {
                return Err(WAVERR_BADFORMAT);
            }
            let float = match u16_at(sub, 0) {
                WAVE_FORMAT_PCM => false,
                WAVE_FORMAT_IEEE_FLOAT => true,
                _ => return Err(WAVERR_BADFORMAT),
            };
            let valid = match u16_at(fmt, 18) {
                0 => bits,
                v => v,
            };
            if valid > bits {
                return Err(shape);
            }
            (float, valid)
        }
        _ => return Err(WAVERR_BADFORMAT),
    };
    if rate == 0 || !(1..=2).contains(&channels) {
        return Err(shape);
    }
    let sample = if float {
        match bits {
            32 => Sample::Float,
            64 => Sample::Float64,
            _ => return Err(shape),
        }
    } else {
        if !matches!(bits, 8 | 16 | 24 | 32) {
            return Err(shape);
        }
        match int_sample(bits, valid) {
            Some(s) => s,
            // Fewer significant bits than the container: the query takes it, the open does not.
            None if query => Sample::S16,
            None => return Err(WAVERR_BADFORMAT),
        }
    };
    let format = Format::new(sample, channels as u8, rate);
    Ok(WaveFormat { format, block: u32::from(channels) * u32::from(bits / 8) })
}

/// `waveOutOpen`'s flags: `MMSYSERR_INVALFLAG` for a bit it does not know, and for a callback
/// kind other than null, window, thread, function and event.
pub fn check_open_flags(flags: u32) -> Result<(), u32> {
    if flags & !OPEN_FLAGS != 0 {
        return Err(MMSYSERR_INVALFLAG);
    }
    match flags & CALLBACK_TYPEMASK {
        CALLBACK_NULL | CALLBACK_WINDOW | CALLBACK_THREAD | CALLBACK_FUNCTION | CALLBACK_EVENT => Ok(()),
        _ => Err(MMSYSERR_INVALFLAG),
    }
}

// ----- headers

/// `waveOutPrepareHeader`'s new flags for a header with `flags` (`waveOutPrepareHeader.yaml`):
/// an undefined bit is `MMSYSERR_INVALPARAM`; a prepared header stays exactly as it is; else
/// `WHDR_PREPARED` with the loop flags kept and done and in-queue cleared.
pub fn prepare(flags: u32) -> Result<u32, u32> {
    if flags & !WHDR_VALID != 0 {
        return Err(MMSYSERR_INVALPARAM);
    }
    if flags & WHDR_PREPARED != 0 {
        return Ok(flags);
    }
    Ok(flags & (WHDR_BEGINLOOP | WHDR_ENDLOOP) | WHDR_PREPARED)
}

/// What `waveOutUnprepareHeader` does with a header of `flags` (`waveOutUnprepareHeader.yaml`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unprepare {
    /// Not prepared: nothing to do (the handle is not looked at).
    NotPrepared,
    /// Look at the handle, then: still queued (`WAVERR_STILLPLAYING`) or these new flags.
    Prepared { queued: bool, flags: u32 },
}

pub fn unprepare(flags: u32) -> Result<Unprepare, u32> {
    if flags & !WHDR_VALID != 0 {
        return Err(MMSYSERR_INVALPARAM);
    }
    if flags & WHDR_PREPARED == 0 {
        return Ok(Unprepare::NotPrepared);
    }
    Ok(Unprepare::Prepared { queued: flags & WHDR_INQUEUE != 0, flags: flags & !WHDR_PREPARED })
}

/// `waveOutWrite`'s new flags (`waveOutWrite.yaml`): not prepared is `WAVERR_UNPREPARED`, queued
/// is `WAVERR_STILLPLAYING`; else done cleared and in-queue set.
pub fn write(flags: u32) -> Result<u32, u32> {
    if flags & WHDR_PREPARED == 0 {
        return Err(WAVERR_UNPREPARED);
    }
    if flags & WHDR_INQUEUE != 0 {
        return Err(WAVERR_STILLPLAYING);
    }
    Ok(flags & !WHDR_DONE | WHDR_INQUEUE)
}

/// A finished header's flags: done set, in-queue cleared.
pub fn finished(flags: u32) -> u32 {
    flags & !WHDR_INQUEUE | WHDR_DONE
}

// ----- the play order

/// A header the application wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Queued {
    /// The application's header (its address).
    pub id: usize,
    pub len: u32,
    pub begin_loop: bool,
    pub end_loop: bool,
    /// `dwLoops` (of a header that begins a loop).
    pub loops: u32,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    q: Queued,
    /// Where its last byte is in the played stream (bytes), once its last pass is fed.
    end: Option<u64>,
}

/// The next bytes to feed: `len` bytes of header `id` from `offset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub id: usize,
    pub offset: u32,
    pub len: u32,
}

/// The order the queued headers play in (`waveOutWrite.yaml`, `waveOutBreakLoop.yaml`): one after
/// the other; a loop (the header with `WHDR_BEGINLOOP` up to the one with `WHDR_ENDLOOP`) waits
/// until its end is written, then plays `dwLoops` times (once for 0), or up to the end of its
/// current pass after a break. The stream is counted in bytes from the open or the last reset;
/// a header is done once the stream played up to its end in its last pass.
#[derive(Debug, Default)]
pub struct Player {
    queue: VecDeque<Entry>,
    /// The header (index in `queue`) and offset the next byte comes from.
    at: usize,
    offset: u32,
    /// The loop being played: its first header's index and the passes still to go after this one.
    looping: Option<(usize, u32)>,
    fed: u64,
}

impl Player {
    pub fn push(&mut self, q: Queued) {
        self.queue.push_back(Entry { q, end: None });
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Bytes fed so far.
    pub fn fed(&self) -> u64 {
        self.fed
    }

    /// The end (in bytes) of the first header not yet done, when it is known.
    pub fn next_end(&self) -> Option<u64> {
        self.queue.front().and_then(|e| e.end)
    }

    /// Move over what is finished: zero-length headers and the ends of headers and loop passes.
    fn settle(&mut self) {
        while let Some(e) = self.queue.get(self.at).copied() {
            if e.q.begin_loop && self.offset == 0 && self.looping.is_none_or(|(start, _)| start != self.at) {
                // A loop starts here: only once its end is in the queue.
                if !self.queue.iter().skip(self.at).any(|e| e.q.end_loop) {
                    return;
                }
                self.looping = Some((self.at, e.q.loops.max(1) - 1));
            }
            if self.offset < e.q.len {
                return;
            }
            self.finish_header();
        }
    }

    /// The header at `at` was fed to its end in this pass.
    fn finish_header(&mut self) {
        let e = self.queue[self.at];
        self.offset = 0;
        match self.looping {
            Some((start, left)) if e.q.end_loop => {
                if left > 0 {
                    self.looping = Some((start, left - 1));
                    self.at = start;
                    // The first header's loop check is done: it plays again from here.
                    return;
                }
                for i in start..=self.at {
                    self.queue[i].end = Some(self.fed);
                }
                self.looping = None;
                self.at += 1;
            }
            Some(_) => self.at += 1,
            None => {
                self.queue[self.at].end = Some(self.fed);
                self.at += 1;
            }
        }
    }

    /// The next piece to feed, at most `max` bytes; `None` when there is nothing to feed now
    /// (all fed, or a loop waits for its end).
    pub fn next_piece(&mut self, max: u32) -> Option<Piece> {
        self.settle();
        let e = self.queue.get(self.at)?;
        if e.q.begin_loop && self.offset == 0 && self.looping.is_none_or(|(s, _)| s != self.at) {
            return None;
        }
        let len = (e.q.len - self.offset).min(max);
        (len > 0).then_some(Piece { id: e.q.id, offset: self.offset, len })
    }

    /// `len` bytes of the last piece were fed.
    pub fn fed_piece(&mut self, len: u32) {
        self.offset += len;
        self.fed += u64::from(len);
        if self.queue.get(self.at).is_some_and(|e| self.offset >= e.q.len) {
            self.finish_header();
        }
        self.settle();
    }

    /// The headers done once the stream played `played` bytes: whole frames of `block` bytes
    /// count, so a header that ends inside a frame is done when the frames before it are.
    pub fn done(&mut self, played: u64, block: u32) -> Vec<usize> {
        let mut out = Vec::new();
        while let Some(e) = self.queue.front() {
            let Some(end) = e.end else { break };
            let need = end - end % u64::from(block.max(1));
            if played < need {
                break;
            }
            out.push(e.q.id);
            self.queue.pop_front();
            self.at -= 1;
            if let Some((start, left)) = self.looping {
                self.looping = Some((start - 1, left));
            }
        }
        out
    }

    /// `waveOutBreakLoop`: the loop being played ends with its current pass.
    pub fn break_loop(&mut self) {
        if let Some((start, _)) = self.looping {
            self.looping = Some((start, 0));
        }
    }

    /// `waveOutReset`: every queued header, in order; the stream counts from 0 again.
    pub fn reset(&mut self) -> Vec<usize> {
        let out = self.queue.iter().map(|e| e.q.id).collect();
        *self = Player::default();
        out
    }
}

// ----- texts and capabilities

/// The texts of `waveOutGetErrorText` (`waveOutGetErrorTextW.yaml`).
pub fn error_text(code: u32) -> Option<&'static str> {
    Some(match code {
        0 => "The specified command was carried out.",
        1 => "Undefined external error.",
        2 => "A device ID has been used that is out of range for your system.",
        4 => "The specified device is already in use.  Wait until it is free, and then try again.",
        5 => "The specified device handle is invalid.",
        6 => "There is no driver installed on your system.",
        7 => {
            "There is not enough memory available for this task.  Quit one or more applications to increase available memory, and then try again."
        }
        8 => {
            "This function is not supported.  Use the Capabilities function to determine which functions and messages the driver supports."
        }
        9 => "An error number was specified that is not defined in the system.",
        10 => "An invalid flag was passed to a system function.",
        11 => "An invalid parameter was passed to a system function.",
        12 => "Handle being used simultaneously on another thread (eg callback).",
        20 => "The driver did not generate a valid OPEN callback.",
        32 => {
            "The specified format is not supported or cannot be translated.  Use the Capabilities function to determine the supported formats."
        }
        33 => {
            "Cannot perform this operation while media data is still playing.  Reset the device, or wait until the data is finished playing."
        }
        34 => "The wave header was not prepared.  Use the Prepare function to prepare the header, and then try again.",
        35 => "Cannot open the device without using the WAVE_ALLOWSYNC flag.  Use the flag, and then try again.",
        _ => return None,
    })
}

/// The device's name (`szPname`) for device 0, and the mapper's.
pub const DEVICE_NAME: &str = "Speakers (Poly Audio)";
pub const MAPPER_NAME: &str = "Microsoft Sound Mapper";
/// `MM_MICROSOFT`, `MM_WAVE_MAPPER` and `MM_MSFT_WDMAUDIO_WAVEOUT` (`mmreg.h`).
const MM_MICROSOFT: u16 = 1;
const MM_WAVE_MAPPER: u16 = 2;
const MM_MSFT_WDMAUDIO_WAVEOUT: u16 = 100;
/// Every `WAVE_FORMAT_*` bit of `dwFormats`, and `WAVECAPS_PLAYBACKRATE | WAVECAPS_VOLUME |
/// WAVECAPS_LRVOLUME | WAVECAPS_SAMPLEACCURATE`.
const FORMATS: u32 = 0xFFFFF;
const SUPPORT: u32 = 0x2E;
/// The driver version reported (`vDriverVersion`: not compared by the sheet).
const DRIVER_VERSION: u32 = 0x0A00;
pub const CAPS_W_SIZE: usize = 84;
pub const CAPS_A_SIZE: usize = 52;

/// `WAVEOUTCAPSW` (84 bytes) or `WAVEOUTCAPSA` (52) of device 0 or the mapper.
pub fn caps(mapper: bool, wide: bool) -> Vec<u8> {
    let name = if mapper { MAPPER_NAME } else { DEVICE_NAME };
    let mut b = Vec::with_capacity(CAPS_W_SIZE);
    b.extend_from_slice(&MM_MICROSOFT.to_le_bytes());
    b.extend_from_slice(&(if mapper { MM_WAVE_MAPPER } else { MM_MSFT_WDMAUDIO_WAVEOUT }).to_le_bytes());
    b.extend_from_slice(&DRIVER_VERSION.to_le_bytes());
    for i in 0..32 {
        let c = name.as_bytes().get(i).copied().unwrap_or(0);
        if wide {
            b.extend_from_slice(&u16::from(c).to_le_bytes());
        } else {
            b.push(c);
        }
    }
    b.extend_from_slice(&FORMATS.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&SUPPORT.to_le_bytes());
    b
}

/// A 16-bit volume as a gain (16.16): 0xFFFF is unity.
pub fn gain(v: u16) -> u32 {
    ((u64::from(v) << 16) / 0xFFFF) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn wfx(tag: u16, channels: u16, rate: u32, bits: u16, extra: &[u8]) -> Vec<u8> {
        let block = channels * bits / 8;
        let mut b = Vec::new();
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
        b.extend_from_slice(&block.to_le_bytes());
        b.extend_from_slice(&bits.to_le_bytes());
        b.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        b.extend_from_slice(extra);
        b
    }

    fn ext(channels: u16, rate: u32, bits: u16, valid: u16, sub: u16) -> Vec<u8> {
        let mut extra = Vec::new();
        extra.extend_from_slice(&valid.to_le_bytes());
        extra.extend_from_slice(&(if channels == 1 { 4u32 } else { 3 }).to_le_bytes());
        extra.extend_from_slice(&sub.to_le_bytes());
        extra.extend_from_slice(&SUBTYPE_TAIL);
        wfx(WAVE_FORMAT_EXTENSIBLE, channels, rate, bits, &extra)
    }

    fn both(f: &[u8]) -> (Result<WaveFormat, u32>, Result<WaveFormat, u32>) {
        (check_format(f, true), check_format(f, false))
    }

    #[test]
    fn formats_the_sheet_accepts() {
        let ok = |f: Vec<u8>, sample: Sample, block: u32| {
            let (q, o) = both(&f);
            assert_eq!(q.map(|w| w.format.sample), Ok(sample));
            assert_eq!(o.map(|w| (w.format.sample, w.block)), Ok((sample, block)));
        };
        ok(wfx(1, 2, 44100, 16, &[]), Sample::S16, 4);
        ok(wfx(1, 1, 8000, 16, &[]), Sample::S16, 2);
        ok(wfx(1, 2, 48000, 24, &[]), Sample::S24_3, 6);
        ok(wfx(1, 2, 48000, 32, &[]), Sample::S32, 8);
        ok(wfx(3, 2, 48000, 32, &[]), Sample::Float, 8);
        ok(ext(1, 44100, 32, 32, 3), Sample::Float, 4);
        ok(ext(2, 48000, 16, 16, 1), Sample::S16, 4);
    }

    #[test]
    fn formats_the_sheet_refuses() {
        let e = |q, o| (Err(q), Err(o));
        let r = |f: Vec<u8>| {
            let (q, o) = both(&f);
            (q.map(|_| ()), o.map(|_| ()))
        };
        for tag in [0u16, 2, 6, 0x55] {
            assert_eq!(r(wfx(tag, 2, 44100, 16, &[])), e(WAVERR_BADFORMAT, WAVERR_BADFORMAT));
        }
        assert_eq!(r(wfx(1, 2, 44100, 12, &[])), e(WAVERR_BADFORMAT, MMSYSERR_INVALPARAM));
        assert_eq!(r(wfx(1, 3, 44100, 16, &[])), e(WAVERR_BADFORMAT, MMSYSERR_INVALPARAM));
        assert_eq!(r(wfx(1, 2, 0, 16, &[])), e(WAVERR_BADFORMAT, MMSYSERR_INVALPARAM));
        assert_eq!(r(ext(2, 48000, 16, 16, 7)), e(WAVERR_BADFORMAT, WAVERR_BADFORMAT));
        assert_eq!(r(ext(2, 48000, 16, 24, 1)), e(WAVERR_BADFORMAT, MMSYSERR_INVALPARAM));
        assert_eq!(r(wfx(WAVE_FORMAT_EXTENSIBLE, 2, 48000, 16, &[])), e(MMSYSERR_INVALPARAM, MMSYSERR_INVALPARAM));
        // 12 valid bits of 16: the query takes it, the open does not.
        assert_eq!(r(ext(2, 48000, 16, 12, 1)), (Ok(()), Err(WAVERR_BADFORMAT)));
    }

    #[test]
    fn open_flags() {
        assert_eq!(check_open_flags(WAVE_FORMAT_DIRECT | WAVE_ALLOWSYNC | WAVE_MAPPED | CALLBACK_EVENT), Ok(()));
        assert_eq!(check_open_flags(0x10), Err(MMSYSERR_INVALFLAG));
        assert_eq!(check_open_flags(0x0004_0000), Err(MMSYSERR_INVALFLAG));
    }

    #[test]
    fn header_flags() {
        let p = |f| prepare(f).unwrap();
        assert_eq!(p(0), WHDR_PREPARED);
        assert_eq!(p(WHDR_DONE), WHDR_PREPARED);
        assert_eq!(p(WHDR_DONE | WHDR_BEGINLOOP), WHDR_PREPARED | WHDR_BEGINLOOP);
        assert_eq!(p(WHDR_INQUEUE), WHDR_PREPARED);
        assert_eq!(p(WHDR_PREPARED | WHDR_INQUEUE), WHDR_PREPARED | WHDR_INQUEUE);
        assert_eq!(prepare(0x20), Err(MMSYSERR_INVALPARAM));
        assert_eq!(unprepare(0), Ok(Unprepare::NotPrepared));
        assert_eq!(
            unprepare(WHDR_PREPARED | WHDR_DONE | WHDR_ENDLOOP),
            Ok(Unprepare::Prepared { queued: false, flags: WHDR_DONE | WHDR_ENDLOOP })
        );
        assert_eq!(
            unprepare(WHDR_PREPARED | WHDR_INQUEUE),
            Ok(Unprepare::Prepared { queued: true, flags: WHDR_INQUEUE })
        );
        assert_eq!(unprepare(0x8000_0000), Err(MMSYSERR_INVALPARAM));
        assert_eq!(write(0), Err(WAVERR_UNPREPARED));
        assert_eq!(write(WHDR_PREPARED | WHDR_INQUEUE), Err(WAVERR_STILLPLAYING));
        assert_eq!(write(WHDR_PREPARED | WHDR_DONE), Ok(WHDR_PREPARED | WHDR_INQUEUE));
        assert_eq!(finished(WHDR_PREPARED | WHDR_INQUEUE), WHDR_PREPARED | WHDR_DONE);
    }

    fn q(id: usize, len: u32) -> Queued {
        Queued { id, len, begin_loop: false, end_loop: false, loops: 0 }
    }

    /// Feed everything in pieces of at most `max`: the pieces' (id, offset, len).
    fn feed_all(p: &mut Player, max: u32) -> Vec<(usize, u32, u32)> {
        let mut out = Vec::new();
        while let Some(x) = p.next_piece(max) {
            out.push((x.id, x.offset, x.len));
            p.fed_piece(x.len);
        }
        out
    }

    #[test]
    fn buffers_play_in_order_and_finish_at_their_end() {
        let mut p = Player::default();
        p.push(q(1, 100));
        p.push(q(2, 0));
        p.push(q(3, 50));
        assert_eq!(feed_all(&mut p, 60), vec![(1, 0, 60), (1, 60, 40), (3, 0, 50)]);
        assert_eq!(p.fed(), 150);
        assert_eq!(p.done(99, 4), vec![]);
        // 100 is not a whole frame of 8: done with the frames before it.
        assert_eq!(p.done(96, 8), vec![1, 2]);
        assert_eq!(p.done(150, 1), vec![3]);
        assert!(p.is_empty());
        p.push(q(4, 10));
        assert_eq!(feed_all(&mut p, 100), vec![(4, 0, 10)]);
        assert_eq!(p.next_end(), Some(160));
        assert_eq!(p.reset(), vec![4]);
        assert_eq!(p.fed(), 0);
    }

    #[test]
    fn a_loop_plays_its_count_once_its_end_is_written() {
        let mut p = Player::default();
        p.push(Queued { id: 1, len: 10, begin_loop: true, end_loop: true, loops: 3 });
        assert_eq!(feed_all(&mut p, 100), vec![(1, 0, 10), (1, 0, 10), (1, 0, 10)]);
        assert_eq!(p.done(30, 1), vec![1]);
        // A loop over two headers waits for the second.
        p.push(Queued { id: 2, len: 4, begin_loop: true, end_loop: false, loops: 2 });
        assert_eq!(feed_all(&mut p, 100), vec![]);
        assert_eq!(p.done(1000, 1), vec![]);
        p.push(Queued { id: 3, len: 6, begin_loop: false, end_loop: true, loops: 0 });
        p.push(q(4, 5));
        assert_eq!(feed_all(&mut p, 100), vec![(2, 0, 4), (3, 0, 6), (2, 0, 4), (3, 0, 6), (4, 0, 5)]);
        assert_eq!(p.done(50, 1), vec![2, 3]);
        assert_eq!(p.done(55, 1), vec![4]);
    }

    #[test]
    fn a_break_ends_the_loop_with_its_pass() {
        let mut p = Player::default();
        p.push(Queued { id: 1, len: 10, begin_loop: true, end_loop: true, loops: 1000 });
        p.push(q(2, 5));
        for _ in 0..3 {
            let x = p.next_piece(10).unwrap();
            p.fed_piece(x.len);
        }
        let x = p.next_piece(4).unwrap();
        p.fed_piece(x.len);
        p.break_loop();
        assert_eq!(feed_all(&mut p, 100), vec![(1, 4, 6), (2, 0, 5)]);
        assert_eq!(p.done(40, 1), vec![1]);
        assert_eq!(p.done(45, 1), vec![2]);
    }

    #[test]
    fn texts_and_caps() {
        assert_eq!(error_text(34).map(|t| &t[..5]), Some("The w"));
        assert_eq!(error_text(21), None);
        let w = caps(false, true);
        assert_eq!(w.len(), CAPS_W_SIZE);
        assert_eq!((u16_at(&w, 0), u32_at(&w, 72), u16_at(&w, 76), u32_at(&w, 80)), (1, 0xFFFFF, 2, 0x2E));
        assert_eq!(u16_at(&w, 8), u16::from(b'S'));
        let a = caps(true, false);
        assert_eq!(a.len(), CAPS_A_SIZE);
        assert_eq!((u16_at(&a, 2), &a[8..30], u32_at(&a, 48)), (2, &b"Microsoft Sound Mapper"[..], 0x2E));
        assert_eq!(a[30], 0);
        assert_eq!((gain(0xFFFF), gain(0), gain(0x8000)), (0x10000, 0, 0x8000));
        let _ = vec![0u8];
    }
}
