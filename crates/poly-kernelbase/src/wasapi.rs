//! The logic of our `mmdevapi.dll`'s shared-mode audio (WASAPI; the sheets in
//! `specs/windows/mmdevapi.dll/`, constants and layouts from `mmdeviceapi.h`, `audioclient.h`,
//! `audiopolicy.h`, `mmreg.h`, `ksmedia.h` and `winerror.h`): the device id, the argument checks of
//! the enumerator, which formats a shared-mode client takes, the buffer sizes, and the state of a
//! client (its buffer, its outstanding `GetBuffer`, start, stop, reset, the event handle) and of a
//! session's registrations. The DLL is the COM shell around it and its engine thread feeds
//! `\Device\PolyAudio` (ADR 0244, ADR 0254).

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use poly_audio::{Format, Sample};

// ----- results (`winerror.h`, `audioclient.h`: `AUDCLNT_ERR(n)` is 0x88890000 + n)

pub const S_OK: u32 = 0;
pub const S_FALSE: u32 = 1;
pub const E_POINTER: u32 = 0x8000_4003;
pub const E_INVALIDARG: u32 = 0x8007_0057;
pub const E_NOINTERFACE: u32 = 0x8000_4002;
pub const E_UNEXPECTED: u32 = 0x8000_FFFF;
pub const E_NOTIMPL: u32 = 0x8000_4001;
pub const E_OUTOFMEMORY: u32 = 0x8007_000E;
/// `HRESULT_FROM_WIN32(ERROR_NOT_FOUND)`.
pub const E_NOTFOUND: u32 = 0x8007_0490;
/// `HRESULT_FROM_WIN32(RPC_X_NULL_REF_POINTER)`.
pub const RPC_X_NULL_REF_POINTER: u32 = 0x8007_06F4;
pub const CLASS_E_NOAGGREGATION: u32 = 0x8004_0110;
pub const AUDCLNT_E_NOT_INITIALIZED: u32 = 0x8889_0001;
pub const AUDCLNT_E_ALREADY_INITIALIZED: u32 = 0x8889_0002;
pub const AUDCLNT_E_WRONG_ENDPOINT_TYPE: u32 = 0x8889_0003;
pub const AUDCLNT_E_NOT_STOPPED: u32 = 0x8889_0005;
pub const AUDCLNT_E_BUFFER_TOO_LARGE: u32 = 0x8889_0006;
pub const AUDCLNT_E_OUT_OF_ORDER: u32 = 0x8889_0007;
pub const AUDCLNT_E_UNSUPPORTED_FORMAT: u32 = 0x8889_0008;
pub const AUDCLNT_E_INVALID_SIZE: u32 = 0x8889_0009;
pub const AUDCLNT_E_BUFFER_OPERATION_PENDING: u32 = 0x8889_000B;
pub const AUDCLNT_E_EVENTHANDLE_NOT_EXPECTED: u32 = 0x8889_0011;
pub const AUDCLNT_E_EVENTHANDLE_NOT_SET: u32 = 0x8889_0014;
pub const AUDCLNT_E_INVALID_DEVICE_PERIOD: u32 = 0x8889_0020;
pub const AUDCLNT_E_INVALID_STREAM_FLAG: u32 = 0x8889_0021;

// ----- enumerations and flags

/// `EDataFlow`: `eRender`, `eCapture`, `eAll`.
pub const E_RENDER: u32 = 0;
pub const E_CAPTURE: u32 = 1;
pub const E_ALL: u32 = 2;
/// `ERole`: `eConsole`, `eMultimedia`, `eCommunications`.
pub const ROLES: u32 = 3;
pub const DEVICE_STATE_ACTIVE: u32 = 1;
pub const DEVICE_STATEMASK_ALL: u32 = 0xF;
pub const AUDCLNT_SHAREMODE_SHARED: u32 = 0;
pub const AUDCLNT_SHAREMODE_EXCLUSIVE: u32 = 1;
pub const AUDCLNT_STREAMFLAGS_EVENTCALLBACK: u32 = 0x0004_0000;
pub const AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM: u32 = 0x8000_0000;
pub const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x2;
pub const AUDIOCLOCK_CHARACTERISTIC_FIXED_FREQ: u32 = 1;
/// `AudioSessionState`.
pub const SESSION_INACTIVE: u32 = 0;
pub const SESSION_ACTIVE: u32 = 1;

/// The device period `GetDevicePeriod` gives, in 100 ns units: 10 ms by default, 3 ms at least.
pub const DEFAULT_PERIOD_HNS: i64 = 100_000;
pub const MINIMUM_PERIOD_HNS: i64 = 30_000;

// ----- the endpoint

/// The one render endpoint: Poly's sound card. Its id has the form of an endpoint id
/// (`{0.0.0.00000000}.{guid}`, 55 characters); the GUID is Poly's own.
pub const RENDER_ID: &str = "{0.0.0.00000000}.{50ae3c1d-2f6b-4c0e-9a51-706f6c796175}";

/// What `GetDevice` makes of an id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdKind {
    /// The render endpoint.
    Ours,
    /// An endpoint id that names no endpoint here (`E_NOTFOUND`).
    Elsewhere,
    /// Not an endpoint id (`E_INVALIDARG`).
    Malformed,
}

fn is_hex(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

/// Is `s` a GUID in braces, `{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}`?
fn is_braced_guid(s: &[u8]) -> bool {
    s.len() == 38
        && s[0] == b'{'
        && s[37] == b'}'
        && s[1..37].iter().enumerate().all(|(i, &c)| if matches!(i, 8 | 13 | 18 | 23) { c == b'-' } else { is_hex(c) })
}

/// `GetDevice`'s id (UTF-16 as the caller gave it).
pub fn classify_id(id: &[u16]) -> IdKind {
    let Ok(s) = alloc::string::String::from_utf16(id) else { return IdKind::Malformed };
    if s.eq_ignore_ascii_case(RENDER_ID) {
        return IdKind::Ours;
    }
    let b = s.as_bytes();
    // `{0.0.F.00000000}.` with a flow digit, then a braced GUID.
    let head_ok = b.len() == 55
        && b.starts_with(b"{0.0.")
        && matches!(b[5], b'0' | b'1')
        && &b[6..16] == b".00000000}"
        && b[16] == b'.';
    if head_ok && is_braced_guid(&b[17..]) { IdKind::Elsewhere } else { IdKind::Malformed }
}

/// `EnumAudioEndpoints(flow, mask)`: whether the arguments are good, and whether the render
/// endpoint (always active) is in the list.
pub fn enum_endpoints(flow: u32, mask: u32) -> Result<bool, u32> {
    if flow > E_ALL || mask == 0 || mask & !DEVICE_STATEMASK_ALL != 0 {
        return Err(E_INVALIDARG);
    }
    Ok(flow != E_CAPTURE && mask & DEVICE_STATE_ACTIVE != 0)
}

/// `GetDefaultAudioEndpoint(flow, role)`'s arguments: render only (no capture endpoint here: not
/// found), any of the three roles.
pub fn default_endpoint(flow: u32, role: u32) -> Result<(), u32> {
    if flow >= E_ALL || role >= ROLES {
        return Err(E_INVALIDARG);
    }
    if flow == E_CAPTURE { Err(E_NOTFOUND) } else { Ok(()) }
}

// ----- formats

/// The mix format: 48000 Hz, 2 channels, 32-bit float, extensible, channel mask 3.
pub const MIX_RATE: u32 = 48_000;
pub const MIX_CHANNELS: u16 = 2;
/// `WAVE_FORMAT_*` and the extensible part (`mmreg.h`, `ksmedia.h`).
pub const WAVE_FORMAT_PCM: u16 = 1;
pub const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
pub const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const SUBTYPE_TAIL: [u8; 14] = [0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71];
pub const WAVEFORMATEX_SIZE: usize = 18;
pub const EXTENSIBLE_SIZE: usize = 40;

/// `GetMixFormat`'s `WAVEFORMATEXTENSIBLE` (40 bytes).
pub fn mix_format() -> [u8; EXTENSIBLE_SIZE] {
    let mut b = [0u8; EXTENSIBLE_SIZE];
    b[0..2].copy_from_slice(&WAVE_FORMAT_EXTENSIBLE.to_le_bytes());
    b[2..4].copy_from_slice(&MIX_CHANNELS.to_le_bytes());
    b[4..8].copy_from_slice(&MIX_RATE.to_le_bytes());
    b[8..12].copy_from_slice(&(MIX_RATE * 8).to_le_bytes());
    b[12..14].copy_from_slice(&8u16.to_le_bytes());
    b[14..16].copy_from_slice(&32u16.to_le_bytes());
    b[16..18].copy_from_slice(&22u16.to_le_bytes());
    b[18..20].copy_from_slice(&32u16.to_le_bytes());
    b[20..24].copy_from_slice(&3u32.to_le_bytes());
    b[24..26].copy_from_slice(&WAVE_FORMAT_IEEE_FLOAT.to_le_bytes());
    b[26..40].copy_from_slice(&SUBTYPE_TAIL);
    b
}

/// A format the stream can be fed in: its samples, channels, rate and frame size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamFormat {
    pub format: Format,
    pub block: u32,
}

/// What a format is, before the shared mode's rules: `None` for a format of no known kind (an
/// unknown tag or sub-format, a width no sample has); an extensible format with too few extra
/// bytes is `E_INVALIDARG`. `fmt` holds the `WAVEFORMATEX` and, for an extensible format, its
/// extra 22 bytes when the caller gave them.
fn parse(fmt: &[u8]) -> Result<Option<StreamFormat>, u32> {
    if fmt.len() < WAVEFORMATEX_SIZE {
        return Err(E_POINTER);
    }
    let u16_at = |at: usize| u16::from_le_bytes([fmt[at], fmt[at + 1]]);
    let tag = u16_at(0);
    let channels = u16_at(2);
    let rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
    let bits = u16_at(14);
    let float = match tag {
        WAVE_FORMAT_PCM => false,
        WAVE_FORMAT_IEEE_FLOAT => true,
        WAVE_FORMAT_EXTENSIBLE => {
            if u16_at(16) < 22 || fmt.len() < EXTENSIBLE_SIZE {
                return Err(E_INVALIDARG);
            }
            if fmt[26..40] != SUBTYPE_TAIL {
                return Ok(None);
            }
            match u16_at(24) {
                WAVE_FORMAT_PCM => false,
                WAVE_FORMAT_IEEE_FLOAT => true,
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };
    let sample = match (float, bits) {
        (true, 32) => Sample::Float,
        (true, 64) => Sample::Float64,
        (false, 8) => Sample::U8,
        (false, 16) => Sample::S16,
        (false, 24) => Sample::S24_3,
        (false, 32) => Sample::S32,
        _ => return Ok(None),
    };
    if channels == 0 || channels > 8 || rate == 0 {
        return Ok(None);
    }
    let format = Format::new(sample, channels as u8, rate);
    Ok(Some(StreamFormat { format, block: u32::from(channels) * u32::from(bits / 8) }))
}

/// Does the mix take `f` as it is: the mix rate and channels, and a sample of 16, 24 or 32 bits or
/// float 32 (`IAudioClient.Format.yaml`, `is_format_supported`)?
fn native(f: &StreamFormat) -> bool {
    f.format.rate == MIX_RATE
        && f.format.channels as u16 == MIX_CHANNELS
        && matches!(f.format.sample, Sample::S16 | Sample::S24_3 | Sample::S32 | Sample::Float)
}

/// `IsFormatSupported(mode, fmt, closest)`: `S_OK` for a format the mix takes as it is, `S_FALSE`
/// (the closest match being the mix format) for another PCM or float format, the errors of the
/// sheet otherwise. `closest` says whether the caller gave a closest-match pointer.
pub fn is_format_supported(mode: u32, fmt: Option<&[u8]>, closest: bool) -> u32 {
    let Some(fmt) = fmt else { return E_POINTER };
    if mode != AUDCLNT_SHAREMODE_SHARED {
        return AUDCLNT_E_UNSUPPORTED_FORMAT;
    }
    match parse(fmt) {
        Err(e) => e,
        Ok(Some(f)) if native(&f) => S_OK,
        Ok(Some(_)) if !closest => E_POINTER,
        Ok(Some(_)) => S_FALSE,
        Ok(None) => AUDCLNT_E_UNSUPPORTED_FORMAT,
    }
}

/// Stream flags `Initialize` refuses: bit 0x20 is `E_INVALIDARG`, any other of the low 16 bits
/// `AUDCLNT_E_INVALID_STREAM_FLAG` (0x1 is the sheet's case); the documented flags above them
/// (event, autoconvert, loopback, rate adjust and the rest) are taken.
pub fn check_stream_flags(flags: u32) -> Result<(), u32> {
    if flags & 0x20 != 0 {
        return Err(E_INVALIDARG);
    }
    if flags & 0xFFFF != 0 {
        return Err(AUDCLNT_E_INVALID_STREAM_FLAG);
    }
    Ok(())
}

/// The frames of one engine period at `rate` (10 ms).
pub fn period_frames(rate: u32) -> u32 {
    (rate / 100).max(1)
}

/// The buffer `Initialize` gives a stream at `rate` for a duration (100 ns units): two engine
/// periods and 96 frames at least (1056 at 48000 Hz), else the duration's frames.
pub fn buffer_frames(rate: u32, duration_hns: i64) -> u32 {
    let least = 2 * period_frames(rate) + 96;
    let asked = (duration_hns.max(0) as u128 * u128::from(rate) / 10_000_000) as u64;
    asked.clamp(u64::from(least), u64::from(u32::MAX)) as u32
}

/// `Initialize(mode, flags, duration, periodicity, fmt, session)`'s checks: the stream's format.
pub fn check_initialize(mode: u32, flags: u32, duration_hns: i64, fmt: Option<&[u8]>) -> Result<StreamFormat, u32> {
    if mode > AUDCLNT_SHAREMODE_EXCLUSIVE {
        return Err(E_INVALIDARG);
    }
    check_stream_flags(flags)?;
    let fmt = fmt.ok_or(E_POINTER)?;
    if duration_hns < 0 {
        return Err(E_INVALIDARG);
    }
    if mode == AUDCLNT_SHAREMODE_EXCLUSIVE {
        return Err(AUDCLNT_E_UNSUPPORTED_FORMAT);
    }
    let f = parse(fmt)?.ok_or(AUDCLNT_E_UNSUPPORTED_FORMAT)?;
    // A format the mix does not take as it is needs AUTOCONVERTPCM (and then any PCM or float
    // format of one or two channels).
    if native(&f) || (flags & AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM != 0 && f.format.channels <= 2) {
        Ok(f)
    } else {
        Err(AUDCLNT_E_UNSUPPORTED_FORMAT)
    }
}

// ----- a client's stream

/// One client's shared-mode stream as the application sees it: the frames waiting in its buffer
/// (the padding), a `GetBuffer` outstanding, whether it runs, the event handle, and the frames the
/// engine consumed (the clock). The bytes are the DLL's.
///
/// The engine **sends** released frames to the device ahead of time ([`Stream::send`]: they can
/// no longer change), and **consumes** them, one period for each period the device played
/// ([`Stream::consume`]): the padding falls when the device moves on, not when frames are sent.
#[derive(Debug)]
pub struct Stream {
    pub buffer: u32,
    pub period: u32,
    pub block: u32,
    pub event_flag: bool,
    pub event: Option<usize>,
    pub running: bool,
    outstanding: Option<u32>,
    /// The frames released and not yet consumed, oldest first (their bytes; silent ones are zero).
    queue: VecDeque<u8>,
    /// Bytes at the front of `queue` the engine sent already.
    sent: usize,
    /// Frames the engine consumed since the last reset.
    consumed: u64,
}

impl Stream {
    pub fn new(f: &StreamFormat, flags: u32, duration_hns: i64) -> Stream {
        Stream {
            buffer: buffer_frames(f.format.rate, duration_hns),
            period: period_frames(f.format.rate),
            block: f.block,
            event_flag: flags & AUDCLNT_STREAMFLAGS_EVENTCALLBACK != 0,
            event: None,
            running: false,
            outstanding: None,
            queue: VecDeque::new(),
            sent: 0,
            consumed: 0,
        }
    }

    /// `GetCurrentPadding`: frames written and not yet consumed.
    pub fn padding(&self) -> u32 {
        (self.queue.len() / self.block as usize) as u32
    }

    /// `GetBuffer(n)`: `Ok(true)` for a buffer of `n` frames, `Ok(false)` for none (`n` = 0).
    pub fn get_buffer(&mut self, n: u32) -> Result<bool, u32> {
        if self.outstanding.is_some() {
            return Err(AUDCLNT_E_OUT_OF_ORDER);
        }
        if n == 0 {
            return Ok(false);
        }
        if n > self.buffer - self.padding() {
            return Err(AUDCLNT_E_BUFFER_TOO_LARGE);
        }
        self.outstanding = Some(n);
        Ok(true)
    }

    /// `ReleaseBuffer(n, flags)` of the frames in `data` (the buffer `GetBuffer` gave; not read
    /// when silent).
    pub fn release_buffer(&mut self, n: u32, flags: u32, data: &[u8]) -> Result<(), u32> {
        if n == 0 {
            self.outstanding = None;
            return Ok(());
        }
        if flags & !AUDCLNT_BUFFERFLAGS_SILENT != 0 {
            return Err(E_INVALIDARG);
        }
        let asked = self.outstanding.ok_or(AUDCLNT_E_OUT_OF_ORDER)?;
        if n > asked {
            return Err(AUDCLNT_E_INVALID_SIZE);
        }
        let bytes = (n * self.block) as usize;
        if flags & AUDCLNT_BUFFERFLAGS_SILENT != 0 || data.len() < bytes {
            self.queue.extend(core::iter::repeat_n(0u8, bytes));
        } else {
            self.queue.extend(&data[..bytes]);
        }
        self.outstanding = None;
        Ok(())
    }

    /// `Start`.
    pub fn start(&mut self) -> Result<(), u32> {
        if self.running {
            return Err(AUDCLNT_E_NOT_STOPPED);
        }
        if self.event_flag && self.event.is_none() {
            return Err(AUDCLNT_E_EVENTHANDLE_NOT_SET);
        }
        self.running = true;
        Ok(())
    }

    /// `Stop`: `S_FALSE` when it was not running.
    pub fn stop(&mut self) -> u32 {
        if !self.running {
            return S_FALSE;
        }
        self.running = false;
        S_OK
    }

    /// `Reset`: the buffer empties and the clock counts from 0.
    pub fn reset(&mut self) -> Result<(), u32> {
        if self.running {
            return Err(AUDCLNT_E_NOT_STOPPED);
        }
        if self.outstanding.is_some() {
            return Err(AUDCLNT_E_BUFFER_OPERATION_PENDING);
        }
        self.queue.clear();
        self.sent = 0;
        self.consumed = 0;
        Ok(())
    }

    /// `SetEventHandle(h)`.
    pub fn set_event(&mut self, h: usize) -> Result<(), u32> {
        if !self.event_flag {
            return Err(AUDCLNT_E_EVENTHANDLE_NOT_EXPECTED);
        }
        if h == 0 {
            return Err(E_INVALIDARG);
        }
        if self.event.is_some() {
            return Err(E_UNEXPECTED);
        }
        self.event = Some(h);
        Ok(())
    }

    /// The next frames to send to the device, at most `frames` of them (none when everything
    /// released is sent).
    pub fn send(&mut self, frames: u32) -> Vec<u8> {
        let n = (frames as usize * self.block as usize).min(self.queue.len() - self.sent);
        let out = self.queue.range(self.sent..self.sent + n).copied().collect();
        self.sent += n;
        out
    }

    /// Frames released and not yet sent.
    pub fn unsent(&self) -> u32 {
        ((self.queue.len() - self.sent) / self.block as usize) as u32
    }

    /// The device played `frames` more of the stream: as many frames leave the buffer (all that
    /// are there, if fewer). Returns how many left.
    pub fn consume(&mut self, frames: u64) -> u32 {
        let n = (frames.min(u64::from(self.padding())) as usize) * self.block as usize;
        self.queue.drain(..n);
        // Frames consumed before they were sent are not sent any more.
        self.sent = self.sent.saturating_sub(n);
        self.consumed += (n / self.block as usize) as u64;
        (n / self.block as usize) as u32
    }

    /// `IAudioClock::GetPosition`: the bytes the engine consumed.
    pub fn position(&self) -> u64 {
        self.consumed * u64::from(self.block)
    }
}

// ----- a session's registrations

/// The objects registered with `RegisterAudioSessionNotification`: once per call (two calls, two
/// entries), removed one at a time.
#[derive(Debug, Default)]
pub struct Registrations {
    pub list: Vec<usize>,
}

impl Registrations {
    pub fn register(&mut self, p: usize) -> u32 {
        if p == 0 {
            return E_POINTER;
        }
        self.list.push(p);
        S_OK
    }

    /// `Ok(true)` when the last registration of `p` went (the DLL lets the object go).
    pub fn unregister(&mut self, p: usize) -> Result<bool, u32> {
        if p == 0 {
            return Err(E_POINTER);
        }
        let i = self.list.iter().position(|&x| x == p).ok_or(E_NOTFOUND)?;
        self.list.remove(i);
        Ok(!self.list.contains(&p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn device_ids() {
        assert_eq!(RENDER_ID.len(), 55);
        assert_eq!(classify_id(&w(RENDER_ID)), IdKind::Ours);
        assert_eq!(classify_id(&w(&RENDER_ID.to_uppercase())), IdKind::Ours);
        assert_eq!(classify_id(&w("{0.0.0.00000000}.{11111111-2222-3333-4444-555555555555}")), IdKind::Elsewhere);
        assert_eq!(classify_id(&w("xy")), IdKind::Malformed);
        assert_eq!(classify_id(&w("")), IdKind::Malformed);
        assert_eq!(classify_id(&w("{0.0.0.00000000}.{11111111-2222-3333-4444-55555555555x}")), IdKind::Malformed);
    }

    #[test]
    fn enumerator_arguments() {
        assert_eq!(enum_endpoints(E_RENDER, DEVICE_STATE_ACTIVE), Ok(true));
        assert_eq!(enum_endpoints(E_ALL, DEVICE_STATE_ACTIVE), Ok(true));
        assert_eq!(enum_endpoints(E_RENDER, 0x2), Ok(false));
        assert_eq!(enum_endpoints(E_CAPTURE, DEVICE_STATE_ACTIVE), Ok(false));
        assert_eq!(enum_endpoints(E_RENDER, 0), Err(E_INVALIDARG));
        assert_eq!(enum_endpoints(E_RENDER, 0x10), Err(E_INVALIDARG));
        assert_eq!(enum_endpoints(3, 1), Err(E_INVALIDARG));
        assert_eq!(default_endpoint(E_RENDER, 2), Ok(()));
        assert_eq!(default_endpoint(E_ALL, 0), Err(E_INVALIDARG));
        assert_eq!(default_endpoint(E_RENDER, 3), Err(E_INVALIDARG));
    }

    fn pcm(tag: u16, ch: u16, rate: u32, bits: u16) -> Vec<u8> {
        let block = ch * bits / 8;
        let mut b = vec![0u8; 18];
        b[0..2].copy_from_slice(&tag.to_le_bytes());
        b[2..4].copy_from_slice(&ch.to_le_bytes());
        b[4..8].copy_from_slice(&rate.to_le_bytes());
        b[8..12].copy_from_slice(&(rate * u32::from(block)).to_le_bytes());
        b[12..14].copy_from_slice(&block.to_le_bytes());
        b[14..16].copy_from_slice(&bits.to_le_bytes());
        b
    }

    #[test]
    fn formats() {
        let mix = mix_format();
        assert_eq!(is_format_supported(0, Some(&mix), true), S_OK);
        assert_eq!(is_format_supported(0, Some(&pcm(1, 2, 48000, 16)), true), S_OK);
        assert_eq!(is_format_supported(0, Some(&pcm(1, 2, 48000, 24)), true), S_OK);
        assert_eq!(is_format_supported(0, Some(&pcm(3, 2, 44100, 32)), true), S_FALSE);
        assert_eq!(is_format_supported(0, Some(&pcm(1, 1, 48000, 16)), true), S_FALSE);
        assert_eq!(is_format_supported(0, Some(&pcm(1, 1, 8000, 8)), true), S_FALSE);
        assert_eq!(is_format_supported(0, Some(&pcm(1, 1, 8000, 8)), false), E_POINTER);
        assert_eq!(is_format_supported(0, None, true), E_POINTER);
        let mut no_extra = mix;
        no_extra[16] = 0;
        assert_eq!(is_format_supported(0, Some(&no_extra), true), E_INVALIDARG);
        assert_eq!(is_format_supported(5, Some(&mix), true), AUDCLNT_E_UNSUPPORTED_FORMAT);
        // Initialize.
        assert_eq!(check_initialize(0, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, 0, Some(&mix)).map(|f| f.block), Ok(8));
        assert_eq!(check_initialize(2, 0, 0, Some(&mix)).err(), Some(E_INVALIDARG));
        assert_eq!(check_initialize(0, 0x20, 0, Some(&mix)).err(), Some(E_INVALIDARG));
        assert_eq!(check_initialize(0, 0x1, 0, Some(&mix)).err(), Some(AUDCLNT_E_INVALID_STREAM_FLAG));
        assert_eq!(check_initialize(0, 0, 0, None).err(), Some(E_POINTER));
        assert_eq!(check_initialize(0, 0, -1, Some(&mix)).err(), Some(E_INVALIDARG));
        assert_eq!(check_initialize(0, 0, 0, Some(&pcm(3, 2, 44100, 32))).err(), Some(AUDCLNT_E_UNSUPPORTED_FORMAT));
        let conv = check_initialize(0, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, 0, Some(&pcm(1, 1, 44100, 16))).unwrap();
        assert_eq!((conv.format.rate, conv.block), (44100, 2));
    }

    #[test]
    fn buffer_sizes() {
        assert_eq!(buffer_frames(48000, 0), 1056);
        assert_eq!(buffer_frames(48000, 100_000), 1056);
        assert_eq!(buffer_frames(48000, 1_000_000), 4800);
        assert_eq!(buffer_frames(48000, 2_000_000), 9600);
        assert_eq!(buffer_frames(48000, 200_000_000), 960_000);
        assert!(buffer_frames(44100, 0) >= 441);
        assert_eq!(period_frames(48000), 480);
    }

    fn stream(flags: u32) -> Stream {
        let f = check_initialize(0, flags, 0, Some(&mix_format())).unwrap();
        Stream::new(&f, flags, 0)
    }

    #[test]
    fn get_and_release() {
        let mut s = stream(0);
        assert_eq!(s.release_buffer(10, 0, &[]), Err(AUDCLNT_E_OUT_OF_ORDER));
        assert_eq!(s.get_buffer(100), Ok(true));
        assert_eq!(s.get_buffer(10), Err(AUDCLNT_E_OUT_OF_ORDER));
        assert_eq!(s.release_buffer(101, 0, &[]), Err(AUDCLNT_E_INVALID_SIZE));
        assert_eq!(s.release_buffer(100, 4, &[]), Err(E_INVALIDARG));
        assert_eq!(s.release_buffer(100, 1, &[]), Err(E_INVALIDARG));
        assert_eq!(s.reset(), Err(AUDCLNT_E_BUFFER_OPERATION_PENDING));
        assert_eq!(s.release_buffer(50, AUDCLNT_BUFFERFLAGS_SILENT, &[]), Ok(()));
        assert_eq!(s.padding(), 50);
        // Too much for the free space, and nothing on a full buffer.
        assert_eq!(s.get_buffer(1056 - 50 + 1), Err(AUDCLNT_E_BUFFER_TOO_LARGE));
        assert_eq!(s.get_buffer(1006), Ok(true));
        let data = vec![7u8; 1006 * 8];
        assert_eq!(s.release_buffer(1006, 0, &data), Ok(()));
        assert_eq!(s.padding(), 1056);
        assert_eq!(s.get_buffer(1), Err(AUDCLNT_E_BUFFER_TOO_LARGE));
        assert_eq!(s.get_buffer(0), Ok(false));
        assert_eq!(s.release_buffer(0, 0, &[]), Ok(()));
        // Sending reads ahead without consuming: silence first, then the data.
        let p = s.send(960);
        assert_eq!((p.len(), p[0], p[50 * 8]), (960 * 8, 0, 7));
        assert_eq!((s.padding(), s.unsent(), s.position()), (1056, 96, 0));
        // The device played one period: one leaves the buffer.
        assert_eq!(s.consume(480), 480);
        assert_eq!((s.padding(), s.unsent(), s.position()), (576, 96, 480 * 8));
        let rest = s.send(480);
        assert_eq!((rest.len(), s.unsent()), (96 * 8, 0));
        assert!(s.send(480).is_empty());
        // More than is there: what is there.
        assert_eq!(s.consume(960), 576);
        assert_eq!((s.padding(), s.position()), (0, 8448));
        assert_eq!(s.consume(480), 0);
        // Consumed before it was sent: it is not sent any more.
        assert_eq!(s.get_buffer(100), Ok(true));
        assert_eq!(s.release_buffer(100, AUDCLNT_BUFFERFLAGS_SILENT, &[]), Ok(()));
        assert_eq!(s.consume(40), 40);
        assert_eq!(s.send(480).len(), 60 * 8);
        assert_eq!(s.reset(), Ok(()));
        assert_eq!((s.padding(), s.unsent(), s.position()), (0, 0, 0));
    }

    #[test]
    fn start_stop_reset_and_the_event() {
        let mut s = stream(AUDCLNT_STREAMFLAGS_EVENTCALLBACK);
        assert_eq!(s.stop(), S_FALSE);
        assert_eq!(s.reset(), Ok(()));
        assert_eq!(s.start(), Err(AUDCLNT_E_EVENTHANDLE_NOT_SET));
        assert_eq!(s.set_event(0), Err(E_INVALIDARG));
        assert_eq!(s.set_event(0x44), Ok(()));
        assert_eq!(s.set_event(0x48), Err(E_UNEXPECTED));
        assert_eq!(s.event, Some(0x44));
        assert_eq!(s.start(), Ok(()));
        assert_eq!(s.start(), Err(AUDCLNT_E_NOT_STOPPED));
        assert_eq!(s.reset(), Err(AUDCLNT_E_NOT_STOPPED));
        assert_eq!(s.stop(), S_OK);
        assert_eq!(s.stop(), S_FALSE);
        let mut plain = stream(0);
        assert_eq!(plain.set_event(0x44), Err(AUDCLNT_E_EVENTHANDLE_NOT_EXPECTED));
        assert_eq!(plain.start(), Ok(()));
    }

    #[test]
    fn session_registrations() {
        let mut r = Registrations::default();
        assert_eq!(r.register(0), E_POINTER);
        assert_eq!(r.unregister(0), Err(E_POINTER));
        assert_eq!(r.unregister(0x10), Err(E_NOTFOUND));
        assert_eq!(r.register(0x10), S_OK);
        assert_eq!(r.register(0x10), S_OK);
        assert_eq!(r.unregister(0x10), Ok(false));
        assert_eq!(r.unregister(0x10), Ok(true));
        assert_eq!(r.unregister(0x10), Err(E_NOTFOUND));
    }

    #[test]
    fn the_mix_format() {
        let m = mix_format();
        assert_eq!(&m[..16], &[0xFE, 0xFF, 2, 0, 0x80, 0xBB, 0, 0, 0, 0xDC, 5, 0, 8, 0, 32, 0]);
        let f = parse(&m).unwrap().unwrap();
        assert_eq!((f.format.sample, f.block), (Sample::Float, 8));
    }
}
