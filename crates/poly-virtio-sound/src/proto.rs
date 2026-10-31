//! The messages of the sound device (VIRTIO 1.2 §5.14.6), as plain structs
//! with little-endian encoders and decoders. Every field a device returns
//! is untrusted: the decoders only reinterpret bytes, and callers check
//! ranges.

/// Request codes and event codes (§5.14.6, `enum` in the spec).
pub mod code {
    pub const JACK_INFO: u32 = 1;
    pub const JACK_REMAP: u32 = 2;
    pub const PCM_INFO: u32 = 0x0100;
    pub const PCM_SET_PARAMS: u32 = 0x0101;
    pub const PCM_PREPARE: u32 = 0x0102;
    pub const PCM_RELEASE: u32 = 0x0103;
    pub const PCM_START: u32 = 0x0104;
    pub const PCM_STOP: u32 = 0x0105;
    pub const CHMAP_INFO: u32 = 0x0200;
    /// Events (§5.14.6.10 and the event codes of §5.14.6).
    pub const EVT_JACK_CONNECTED: u32 = 0x1000;
    pub const EVT_JACK_DISCONNECTED: u32 = 0x1001;
    pub const EVT_PCM_PERIOD_ELAPSED: u32 = 0x1100;
    pub const EVT_PCM_XRUN: u32 = 0x1101;
}

/// Status codes in responses (§5.14.6).
pub mod status {
    pub const OK: u32 = 0x8000;
    pub const BAD_MSG: u32 = 0x8001;
    pub const NOT_SUPP: u32 = 0x8002;
    pub const IO_ERR: u32 = 0x8003;
}

/// Data flow directions (§5.14.6).
pub mod direction {
    pub const OUTPUT: u8 = 0;
    pub const INPUT: u8 = 1;
}

/// PCM stream feature bits (§5.14.6.6.2, `VIRTIO_SND_PCM_F_*`).
pub mod pcm_feature {
    pub const SHMEM_HOST: u32 = 1 << 0;
    pub const SHMEM_GUEST: u32 = 1 << 1;
    pub const MSG_POLLING: u32 = 1 << 2;
    pub const EVT_SHMEM_PERIODS: u32 = 1 << 3;
    pub const EVT_XRUNS: u32 = 1 << 4;
}

/// Sample formats (§5.14.6.6.2, `VIRTIO_SND_PCM_FMT_*`): bit numbers in
/// [`PcmInfo::formats`], values in [`SetParams::format`].
pub mod format {
    pub const IMA_ADPCM: u8 = 0;
    pub const MU_LAW: u8 = 1;
    pub const A_LAW: u8 = 2;
    pub const S8: u8 = 3;
    pub const U8: u8 = 4;
    pub const S16: u8 = 5;
    pub const U16: u8 = 6;
    pub const S18_3: u8 = 7;
    pub const U18_3: u8 = 8;
    pub const S20_3: u8 = 9;
    pub const U20_3: u8 = 10;
    pub const S24_3: u8 = 11;
    pub const U24_3: u8 = 12;
    pub const S20: u8 = 13;
    pub const U20: u8 = 14;
    pub const S24: u8 = 15;
    pub const U24: u8 = 16;
    pub const S32: u8 = 17;
    pub const U32: u8 = 18;
    pub const FLOAT: u8 = 19;
    pub const FLOAT64: u8 = 20;
    pub const DSD_U8: u8 = 21;
    pub const DSD_U16: u8 = 22;
    pub const DSD_U32: u8 = 23;
    pub const IEC958_SUBFRAME: u8 = 24;
    /// One past the last format the specification defines.
    pub const COUNT: u8 = 25;

    /// Bits a sample of `fmt` takes in memory (the spec's "physical
    /// width"), `None` for formats that are not whole bytes or unknown.
    pub const fn physical_bits(fmt: u8) -> Option<u32> {
        match fmt {
            MU_LAW | A_LAW | S8 | U8 | DSD_U8 => Some(8),
            S16 | U16 | DSD_U16 => Some(16),
            S18_3 | U18_3 | S20_3 | U20_3 | S24_3 | U24_3 => Some(24),
            S20 | U20 | S24 | U24 | S32 | U32 | FLOAT | DSD_U32 | IEC958_SUBFRAME => Some(32),
            FLOAT64 => Some(64),
            _ => None,
        }
    }

    /// Significant bits of a sample of `fmt` (the spec's "width").
    pub const fn width_bits(fmt: u8) -> Option<u32> {
        match fmt {
            IMA_ADPCM => Some(4),
            S18_3 | U18_3 => Some(18),
            S20_3 | U20_3 | S20 | U20 => Some(20),
            S24_3 | U24_3 | S24 | U24 | IEC958_SUBFRAME => Some(24),
            _ => physical_bits(fmt),
        }
    }

    /// The byte pattern of silence for `fmt`: 0 for signed and float
    /// formats, the middle value for unsigned ones (only the high byte is
    /// not zero, so the pattern is per sample).
    pub const fn is_unsigned(fmt: u8) -> bool {
        matches!(fmt, U8 | U16 | U18_3 | U20_3 | U24_3 | U20 | U24 | U32)
    }
}

/// Frame rates (§5.14.6.6.2, `VIRTIO_SND_PCM_RATE_*`): bit numbers in
/// [`PcmInfo::rates`], values in [`SetParams::rate`].
pub mod rate {
    /// Hertz of each rate code, by code.
    pub const HZ: [u32; 14] =
        [5512, 8000, 11025, 16000, 22050, 32000, 44100, 48000, 64000, 88200, 96000, 176400, 192000, 384000];
    pub const R44100: u8 = 6;
    pub const R48000: u8 = 7;

    /// The code of `hz`, if the specification has one.
    pub fn code_of(hz: u32) -> Option<u8> {
        HZ.iter().position(|&r| r == hz).map(|i| i as u8)
    }

    /// The rate of `code` in hertz.
    pub fn hz_of(code: u8) -> Option<u32> {
        HZ.get(code as usize).copied()
    }
}

/// Channel positions in channel maps (§5.14.6.9, `VIRTIO_SND_CHMAP_*`).
pub mod chmap {
    pub const NONE: u8 = 0;
    pub const NA: u8 = 1;
    pub const MONO: u8 = 2;
    pub const FL: u8 = 3;
    pub const FR: u8 = 4;
    pub const RL: u8 = 5;
    pub const RR: u8 = 6;
    pub const FC: u8 = 7;
    pub const LFE: u8 = 8;
    /// Positions a channel map holds at most (`VIRTIO_SND_CHMAP_MAX_SIZE`).
    pub const MAX_SIZE: usize = 18;
}

/// Jack feature bits (§5.14.6.4.1).
pub mod jack_feature {
    pub const REMAP: u32 = 1 << 0;
}

/// `struct virtio_snd_config` (§5.14.4): how many of each item the device
/// has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceConfig {
    pub jacks: u32,
    pub streams: u32,
    pub chmaps: u32,
}

/// Length of [`DeviceConfig`] in the device configuration region.
pub const CONFIG_LEN: usize = 12;

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn le64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(core::array::from_fn(|i| b[at + i]))
}

/// `struct virtio_snd_hdr` (4 bytes): a request code, or a response's
/// status.
pub const HDR_LEN: usize = 4;

/// A request with no body beyond its header and a 32-bit id:
/// `struct virtio_snd_pcm_hdr` (8 bytes) for PREPARE, RELEASE, START and
/// STOP, `struct virtio_snd_jack_hdr` for jacks.
pub fn id_request(code: u32, id: u32) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[0..4].copy_from_slice(&code.to_le_bytes());
    b[4..8].copy_from_slice(&id.to_le_bytes());
    b
}

/// `struct virtio_snd_query_info` (16 bytes): ask for `count` items from
/// `start_id`, each answered in `size` bytes.
pub fn query_info(code: u32, start_id: u32, count: u32, size: u32) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..4].copy_from_slice(&code.to_le_bytes());
    b[4..8].copy_from_slice(&start_id.to_le_bytes());
    b[8..12].copy_from_slice(&count.to_le_bytes());
    b[12..16].copy_from_slice(&size.to_le_bytes());
    b
}

/// The status of a response (its `virtio_snd_hdr`).
pub fn response_status(b: &[u8]) -> Option<u32> {
    (b.len() >= HDR_LEN).then(|| le32(b, 0))
}

/// `struct virtio_snd_jack_info` (§5.14.6.4.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JackInfo {
    pub hda_fn_nid: u32,
    pub features: u32,
    pub hda_reg_defconf: u32,
    pub hda_reg_caps: u32,
    pub connected: bool,
}

impl JackInfo {
    pub const LEN: usize = 24;

    pub fn decode(b: &[u8]) -> JackInfo {
        JackInfo {
            hda_fn_nid: le32(b, 0),
            features: le32(b, 4),
            hda_reg_defconf: le32(b, 8),
            hda_reg_caps: le32(b, 12),
            connected: b[16] != 0,
        }
    }

    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut b = [0u8; Self::LEN];
        b[0..4].copy_from_slice(&self.hda_fn_nid.to_le_bytes());
        b[4..8].copy_from_slice(&self.features.to_le_bytes());
        b[8..12].copy_from_slice(&self.hda_reg_defconf.to_le_bytes());
        b[12..16].copy_from_slice(&self.hda_reg_caps.to_le_bytes());
        b[16] = self.connected as u8;
        b
    }
}

/// `struct virtio_snd_pcm_info` (§5.14.6.6.2): what a stream can do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PcmInfo {
    pub hda_fn_nid: u32,
    /// `VIRTIO_SND_PCM_F_*` bits.
    pub features: u32,
    /// One bit per [`format`] code.
    pub formats: u64,
    /// One bit per [`rate`] code.
    pub rates: u64,
    pub direction: u8,
    pub channels_min: u8,
    pub channels_max: u8,
}

impl PcmInfo {
    pub const LEN: usize = 32;

    pub fn decode(b: &[u8]) -> PcmInfo {
        PcmInfo {
            hda_fn_nid: le32(b, 0),
            features: le32(b, 4),
            formats: le64(b, 8),
            rates: le64(b, 16),
            direction: b[24],
            channels_min: b[25],
            channels_max: b[26],
        }
    }

    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut b = [0u8; Self::LEN];
        b[0..4].copy_from_slice(&self.hda_fn_nid.to_le_bytes());
        b[4..8].copy_from_slice(&self.features.to_le_bytes());
        b[8..16].copy_from_slice(&self.formats.to_le_bytes());
        b[16..24].copy_from_slice(&self.rates.to_le_bytes());
        b[24] = self.direction;
        b[25] = self.channels_min;
        b[26] = self.channels_max;
        b
    }

    pub const fn has_format(&self, fmt: u8) -> bool {
        fmt < 64 && self.formats & (1 << fmt) != 0
    }

    pub const fn has_rate(&self, code: u8) -> bool {
        code < 64 && self.rates & (1 << code) != 0
    }

    /// The rates in hertz this stream offers, in increasing order (the
    /// codes are ordered by rate).
    pub fn rates_hz(&self) -> impl Iterator<Item = u32> + '_ {
        (0..rate::HZ.len() as u8).filter(|&c| self.has_rate(c)).filter_map(rate::hz_of)
    }

    /// The formats this stream offers that the driver can carry (whole
    /// bytes per sample), in code order.
    pub fn usable_formats(&self) -> impl Iterator<Item = u8> + '_ {
        (0..format::COUNT).filter(|&f| self.has_format(f) && format::physical_bits(f).is_some())
    }

    /// Is the stream usable at all: a known direction, a sane channel
    /// range, and at least one usable format and one known rate?
    pub fn is_usable(&self) -> bool {
        self.direction <= direction::INPUT
            && self.channels_min >= 1
            && self.channels_min <= self.channels_max
            && self.usable_formats().next().is_some()
            && self.rates_hz().next().is_some()
    }
}

/// `struct virtio_snd_chmap_info` (§5.14.6.9.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChmapInfo {
    pub hda_fn_nid: u32,
    pub direction: u8,
    pub channels: u8,
    pub positions: [u8; chmap::MAX_SIZE],
}

impl ChmapInfo {
    pub const LEN: usize = 24;

    pub fn decode(b: &[u8]) -> ChmapInfo {
        ChmapInfo {
            hda_fn_nid: le32(b, 0),
            direction: b[4],
            channels: b[5],
            positions: core::array::from_fn(|i| b[6 + i]),
        }
    }

    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut b = [0u8; Self::LEN];
        b[0..4].copy_from_slice(&self.hda_fn_nid.to_le_bytes());
        b[4] = self.direction;
        b[5] = self.channels;
        b[6..].copy_from_slice(&self.positions);
        b
    }

    /// The positions in use (at most [`chmap::MAX_SIZE`], however many
    /// channels the device claims).
    pub fn used_positions(&self) -> &[u8] {
        &self.positions[..(self.channels as usize).min(chmap::MAX_SIZE)]
    }
}

/// `struct virtio_snd_pcm_set_params` (§5.14.6.6.3) without its header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetParams {
    pub buffer_bytes: u32,
    pub period_bytes: u32,
    /// `VIRTIO_SND_PCM_F_*` bits the driver wants (only ones the stream
    /// offers).
    pub features: u32,
    pub channels: u8,
    /// A [`format`] code.
    pub format: u8,
    /// A [`rate`] code.
    pub rate: u8,
}

impl SetParams {
    /// The whole request, header and stream id included.
    pub const REQUEST_LEN: usize = 24;

    pub fn request(&self, stream_id: u32) -> [u8; Self::REQUEST_LEN] {
        let mut b = [0u8; Self::REQUEST_LEN];
        b[0..8].copy_from_slice(&id_request(code::PCM_SET_PARAMS, stream_id));
        b[8..12].copy_from_slice(&self.buffer_bytes.to_le_bytes());
        b[12..16].copy_from_slice(&self.period_bytes.to_le_bytes());
        b[16..20].copy_from_slice(&self.features.to_le_bytes());
        b[20] = self.channels;
        b[21] = self.format;
        b[22] = self.rate;
        b
    }

    /// Parse a whole request: `(stream_id, params)`.
    pub fn parse(b: &[u8]) -> Option<(u32, SetParams)> {
        if b.len() < Self::REQUEST_LEN || le32(b, 0) != code::PCM_SET_PARAMS {
            return None;
        }
        let p = SetParams {
            buffer_bytes: le32(b, 8),
            period_bytes: le32(b, 12),
            features: le32(b, 16),
            channels: b[20],
            format: b[21],
            rate: b[22],
        };
        Some((le32(b, 4), p))
    }

    /// Bytes per frame, if the format is whole bytes.
    pub fn frame_bytes(&self) -> Option<u32> {
        Some(format::physical_bits(self.format)? / 8 * self.channels as u32)
    }
}

/// `struct virtio_snd_event` (§5.14.6.10): 8 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub code: u32,
    /// The jack or stream id the event is about.
    pub data: u32,
}

impl Event {
    pub const LEN: usize = 8;

    pub fn decode(b: &[u8]) -> Option<Event> {
        (b.len() >= Self::LEN).then(|| Event { code: le32(b, 0), data: le32(b, 4) })
    }

    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut b = [0u8; Self::LEN];
        b[0..4].copy_from_slice(&self.code.to_le_bytes());
        b[4..8].copy_from_slice(&self.data.to_le_bytes());
        b
    }
}

/// `struct virtio_snd_pcm_xfer` (§5.14.6.8): the stream id in front of the
/// data of every I/O message.
pub const XFER_LEN: usize = 4;

/// `struct virtio_snd_pcm_status` (§5.14.6.8): the device's answer at the
/// end of every I/O message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PcmStatus {
    pub status: u32,
    pub latency_bytes: u32,
}

impl PcmStatus {
    pub const LEN: usize = 8;

    pub fn decode(b: &[u8]) -> PcmStatus {
        PcmStatus { status: le32(b, 0), latency_bytes: le32(b, 4) }
    }

    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut b = [0u8; Self::LEN];
        b[0..4].copy_from_slice(&self.status.to_le_bytes());
        b[4..8].copy_from_slice(&self.latency_bytes.to_le_bytes());
        b
    }
}
