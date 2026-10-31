//! The ALSA kernel/user ABI for PCM and control devices: numbers, versions
//! and structure layouts from the public uapi header `sound/asound.h`
//! (x86-64 layout: `long` and pointers 8 bytes, `struct timespec` 16).
//! Offsets were checked against the header with `offsetof`.

/// `SNDRV_PROTOCOL_VERSION(major, minor, subminor)`.
pub const fn protocol_version(major: u32, minor: u32, sub: u32) -> u32 {
    (major << 16) | (minor << 8) | sub
}

/// `SNDRV_PCM_VERSION`: 2.0.17.
pub const PCM_VERSION: u32 = protocol_version(2, 0, 17);
/// `SNDRV_CTL_VERSION`: 2.0.9.
pub const CTL_VERSION: u32 = protocol_version(2, 0, 9);

/// The character device major of ALSA (Linux `devices.txt`: 116).
pub const MAJOR: u32 = 116;
/// Poly's minor numbers under [`MAJOR`] for card `c`: the control device
/// at `32c`, PCM playback device `d` at `32c + 16 + d`, capture at
/// `32c + 24 + d` (eight devices of each per card).
pub const fn control_minor(card: u32) -> u32 {
    card * 32
}
pub const fn playback_minor(card: u32, device: u32) -> u32 {
    card * 32 + 16 + device
}
pub const fn capture_minor(card: u32, device: u32) -> u32 {
    card * 32 + 24 + device
}

/// `ioctl` request numbers (`_IOR('A', ...)` and friends, from
/// `sound/asound.h`).
pub mod ioctl {
    pub const PCM_PVERSION: u32 = 0x8004_4100;
    pub const PCM_INFO: u32 = 0x8120_4101;
    pub const PCM_TSTAMP: u32 = 0x4004_4102;
    pub const PCM_TTSTAMP: u32 = 0x4004_4103;
    pub const PCM_USER_PVERSION: u32 = 0x4004_4104;
    pub const PCM_HW_REFINE: u32 = 0xc260_4110;
    pub const PCM_HW_PARAMS: u32 = 0xc260_4111;
    pub const PCM_HW_FREE: u32 = 0x0000_4112;
    pub const PCM_SW_PARAMS: u32 = 0xc088_4113;
    pub const PCM_STATUS: u32 = 0x8098_4120;
    pub const PCM_DELAY: u32 = 0x8008_4121;
    pub const PCM_HWSYNC: u32 = 0x0000_4122;
    pub const PCM_SYNC_PTR: u32 = 0xc088_4123;
    pub const PCM_STATUS_EXT: u32 = 0xc098_4124;
    pub const PCM_CHANNEL_INFO: u32 = 0x8018_4132;
    pub const PCM_PREPARE: u32 = 0x0000_4140;
    pub const PCM_RESET: u32 = 0x0000_4141;
    pub const PCM_START: u32 = 0x0000_4142;
    pub const PCM_DROP: u32 = 0x0000_4143;
    pub const PCM_DRAIN: u32 = 0x0000_4144;
    pub const PCM_PAUSE: u32 = 0x4004_4145;
    pub const PCM_REWIND: u32 = 0x4008_4146;
    pub const PCM_RESUME: u32 = 0x0000_4147;
    pub const PCM_XRUN: u32 = 0x0000_4148;
    pub const PCM_FORWARD: u32 = 0x4008_4149;
    pub const PCM_WRITEI_FRAMES: u32 = 0x4018_4150;
    pub const PCM_READI_FRAMES: u32 = 0x8018_4151;
    pub const PCM_LINK: u32 = 0x4004_4160;
    pub const PCM_UNLINK: u32 = 0x0000_4161;

    pub const CTL_PVERSION: u32 = 0x8004_5500;
    pub const CTL_CARD_INFO: u32 = 0x8178_5501;
    pub const CTL_ELEM_LIST: u32 = 0xc050_5510;
    pub const CTL_SUBSCRIBE_EVENTS: u32 = 0xc004_5516;
    pub const CTL_PCM_NEXT_DEVICE: u32 = 0x8004_5530;
    pub const CTL_PCM_INFO: u32 = 0xc120_5531;
    pub const CTL_PCM_PREFER_SUBDEVICE: u32 = 0x4004_5532;
}

/// `SNDRV_PCM_STREAM_*`.
pub mod stream {
    pub const PLAYBACK: i32 = 0;
    pub const CAPTURE: i32 = 1;
}

/// `SNDRV_PCM_ACCESS_*`.
pub mod access {
    pub const MMAP_INTERLEAVED: u32 = 0;
    pub const MMAP_NONINTERLEAVED: u32 = 1;
    pub const MMAP_COMPLEX: u32 = 2;
    pub const RW_INTERLEAVED: u32 = 3;
    pub const RW_NONINTERLEAVED: u32 = 4;
}

/// `SNDRV_PCM_FORMAT_*` (the little-endian ones and the byte formats).
pub mod format {
    pub const S8: u32 = 0;
    pub const U8: u32 = 1;
    pub const S16_LE: u32 = 2;
    pub const U16_LE: u32 = 4;
    pub const S24_LE: u32 = 6;
    pub const U24_LE: u32 = 8;
    pub const S32_LE: u32 = 10;
    pub const U32_LE: u32 = 12;
    pub const FLOAT_LE: u32 = 14;
    pub const FLOAT64_LE: u32 = 16;
    pub const IEC958_SUBFRAME_LE: u32 = 18;
    pub const MU_LAW: u32 = 20;
    pub const A_LAW: u32 = 21;
    pub const S20_LE: u32 = 25;
    pub const U20_LE: u32 = 27;
    pub const S24_3LE: u32 = 32;
    pub const U24_3LE: u32 = 34;
    pub const S20_3LE: u32 = 36;
    pub const U20_3LE: u32 = 38;
    pub const S18_3LE: u32 = 40;
    pub const U18_3LE: u32 = 42;
    pub const DSD_U8: u32 = 48;
    pub const DSD_U16_LE: u32 = 49;
    pub const DSD_U32_LE: u32 = 50;

    /// `(physical bits, significant bits, unsigned)` of the formats above
    /// (`sound/asound.h` comments and the ALSA format descriptions);
    /// `None` for any other format.
    pub const fn describe(f: u32) -> Option<(u32, u32, bool)> {
        Some(match f {
            S8 => (8, 8, false),
            U8 => (8, 8, true),
            S16_LE => (16, 16, false),
            U16_LE => (16, 16, true),
            S24_LE => (32, 24, false),
            U24_LE => (32, 24, true),
            S32_LE => (32, 32, false),
            U32_LE => (32, 32, true),
            FLOAT_LE => (32, 32, false),
            FLOAT64_LE => (64, 64, false),
            IEC958_SUBFRAME_LE => (32, 32, false),
            MU_LAW | A_LAW => (8, 8, false),
            S20_LE => (32, 20, false),
            U20_LE => (32, 20, true),
            S24_3LE => (24, 24, false),
            U24_3LE => (24, 24, true),
            S20_3LE => (24, 20, false),
            U20_3LE => (24, 20, true),
            S18_3LE => (24, 18, false),
            U18_3LE => (24, 18, true),
            DSD_U8 => (8, 8, false),
            DSD_U16_LE => (16, 16, false),
            DSD_U32_LE => (32, 32, false),
            _ => return None,
        })
    }

    /// One sample of silence in format `f` (at most 8 bytes; the first
    /// `physical bits / 8` are used): zero, the middle value for unsigned
    /// PCM, and the encodings' own silence for mu-law (0xff), A-law (0x55)
    /// and DSD (0x69).
    pub fn silence(f: u32) -> ([u8; 8], usize) {
        let Some((bits, width, unsigned)) = describe(f) else { return ([0; 8], 1) };
        let n = (bits / 8) as usize;
        let mut s = [0u8; 8];
        match f {
            MU_LAW => s[0] = 0xff,
            A_LAW => s[0] = 0x55,
            DSD_U8 | DSD_U16_LE | DSD_U32_LE => s[..n].fill(0x69),
            _ if unsigned => {
                // The top bit of the significant part, little-endian.
                let mid = 1u64 << (width - 1);
                s[..n].copy_from_slice(&mid.to_le_bytes()[..n]);
            }
            _ => {}
        }
        (s, n)
    }
}

/// `SNDRV_PCM_SUBFORMAT_STD`.
pub const SUBFORMAT_STD: u32 = 0;

/// `SNDRV_PCM_STATE_*`.
pub mod state {
    pub const OPEN: u32 = 0;
    pub const SETUP: u32 = 1;
    pub const PREPARED: u32 = 2;
    pub const RUNNING: u32 = 3;
    pub const XRUN: u32 = 4;
    pub const DRAINING: u32 = 5;
    pub const PAUSED: u32 = 6;
    pub const SUSPENDED: u32 = 7;
    pub const DISCONNECTED: u32 = 8;
}

/// `SNDRV_PCM_INFO_*` bits this implementation reports.
pub mod info {
    pub const INTERLEAVED: u32 = 0x0000_0100;
    pub const BLOCK_TRANSFER: u32 = 0x0001_0000;
    pub const BATCH: u32 = 0x0000_0010;
}

/// `SNDRV_PCM_HW_PARAM_*`: masks 0-2, intervals 8-19.
pub mod param {
    pub const ACCESS: usize = 0;
    pub const FORMAT: usize = 1;
    pub const SUBFORMAT: usize = 2;
    pub const SAMPLE_BITS: usize = 8;
    pub const FRAME_BITS: usize = 9;
    pub const CHANNELS: usize = 10;
    pub const RATE: usize = 11;
    pub const PERIOD_TIME: usize = 12;
    pub const PERIOD_SIZE: usize = 13;
    pub const PERIOD_BYTES: usize = 14;
    pub const PERIODS: usize = 15;
    pub const BUFFER_TIME: usize = 16;
    pub const BUFFER_SIZE: usize = 17;
    pub const BUFFER_BYTES: usize = 18;
    pub const TICK_TIME: usize = 19;
    pub const FIRST_INTERVAL: usize = SAMPLE_BITS;
    pub const LAST_INTERVAL: usize = TICK_TIME;
}

/// `SNDRV_PCM_SYNC_PTR_*`.
pub mod sync_ptr {
    pub const HWSYNC: u32 = 1 << 0;
    pub const APPL: u32 = 1 << 1;
    pub const AVAIL_MIN: u32 = 1 << 2;
}

/// `SNDRV_PCM_TSTAMP_TYPE_*`.
pub mod tstamp_type {
    pub const GETTIMEOFDAY: u32 = 0;
    pub const MONOTONIC: u32 = 1;
    pub const MONOTONIC_RAW: u32 = 2;
}

/// Structure sizes.
pub const PCM_INFO_LEN: usize = 288;
pub const HW_PARAMS_LEN: usize = 608;
pub const SW_PARAMS_LEN: usize = 136;
pub const STATUS_LEN: usize = 152;
pub const SYNC_PTR_LEN: usize = 136;
pub const XFERI_LEN: usize = 24;
pub const CARD_INFO_LEN: usize = 376;
pub const ELEM_LIST_LEN: usize = 80;

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn get32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn get64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(core::array::from_fn(|i| b[at + i]))
}

fn put_str(b: &mut [u8], at: usize, len: usize, s: &str) {
    let n = s.len().min(len - 1);
    b[at..at + n].copy_from_slice(&s.as_bytes()[..n]);
}

/// A `struct timespec` value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timespec {
    pub sec: i64,
    pub nsec: i64,
}

impl Timespec {
    pub const fn from_ns(ns: u64) -> Timespec {
        Timespec { sec: (ns / 1_000_000_000) as i64, nsec: (ns % 1_000_000_000) as i64 }
    }

    fn put(&self, b: &mut [u8], at: usize) {
        put64(b, at, self.sec as u64);
        put64(b, at + 8, self.nsec as u64);
    }
}

/// `struct snd_pcm_info`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmInfo<'a> {
    pub device: u32,
    pub subdevice: u32,
    pub stream: i32,
    pub card: i32,
    pub id: &'a str,
    pub name: &'a str,
    pub subname: &'a str,
    pub subdevices_count: u32,
    pub subdevices_avail: u32,
}

impl PcmInfo<'_> {
    pub fn encode(&self) -> [u8; PCM_INFO_LEN] {
        let mut b = [0u8; PCM_INFO_LEN];
        put32(&mut b, 0, self.device);
        put32(&mut b, 4, self.subdevice);
        put32(&mut b, 8, self.stream as u32);
        put32(&mut b, 12, self.card as u32);
        put_str(&mut b, 16, 64, self.id);
        put_str(&mut b, 80, 80, self.name);
        put_str(&mut b, 160, 32, self.subname);
        // dev_class GENERIC, dev_subclass GENERIC_MIX: both 0.
        put32(&mut b, 200, self.subdevices_count);
        put32(&mut b, 204, self.subdevices_avail);
        b
    }

    /// The `(device, subdevice, stream)` a `SNDRV_CTL_IOCTL_PCM_INFO`
    /// caller asks about.
    pub fn request(b: &[u8; PCM_INFO_LEN]) -> (u32, u32, i32) {
        (get32(b, 0), get32(b, 4), get32(b, 8) as i32)
    }
}

/// `struct snd_ctl_card_info`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardInfo<'a> {
    pub card: i32,
    pub id: &'a str,
    pub driver: &'a str,
    pub name: &'a str,
    pub longname: &'a str,
    pub mixername: &'a str,
    pub components: &'a str,
}

impl CardInfo<'_> {
    pub fn encode(&self) -> [u8; CARD_INFO_LEN] {
        let mut b = [0u8; CARD_INFO_LEN];
        put32(&mut b, 0, self.card as u32);
        put_str(&mut b, 8, 16, self.id);
        put_str(&mut b, 24, 16, self.driver);
        put_str(&mut b, 40, 32, self.name);
        put_str(&mut b, 72, 80, self.longname);
        put_str(&mut b, 168, 80, self.mixername);
        put_str(&mut b, 248, 128, self.components);
        b
    }
}

/// `struct snd_ctl_elem_list` with no elements: `used` and `count` 0, the
/// rest as the caller gave it.
pub fn empty_elem_list(b: &mut [u8; ELEM_LIST_LEN]) {
    put32(b, 8, 0);
    put32(b, 12, 0);
}

/// `struct snd_pcm_sw_params`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwParams {
    pub tstamp_mode: u32,
    pub period_step: u32,
    pub sleep_min: u32,
    pub avail_min: u64,
    pub xfer_align: u64,
    pub start_threshold: u64,
    pub stop_threshold: u64,
    pub silence_threshold: u64,
    pub silence_size: u64,
    pub boundary: u64,
    pub proto: u32,
    pub tstamp_type: u32,
}

impl SwParams {
    pub fn decode(b: &[u8; SW_PARAMS_LEN]) -> SwParams {
        SwParams {
            tstamp_mode: get32(b, 0),
            period_step: get32(b, 4),
            sleep_min: get32(b, 8),
            avail_min: get64(b, 16),
            xfer_align: get64(b, 24),
            start_threshold: get64(b, 32),
            stop_threshold: get64(b, 40),
            silence_threshold: get64(b, 48),
            silence_size: get64(b, 56),
            boundary: get64(b, 64),
            proto: get32(b, 72),
            tstamp_type: get32(b, 76),
        }
    }

    pub fn encode(&self) -> [u8; SW_PARAMS_LEN] {
        let mut b = [0u8; SW_PARAMS_LEN];
        put32(&mut b, 0, self.tstamp_mode);
        put32(&mut b, 4, self.period_step);
        put32(&mut b, 8, self.sleep_min);
        put64(&mut b, 16, self.avail_min);
        put64(&mut b, 24, self.xfer_align);
        put64(&mut b, 32, self.start_threshold);
        put64(&mut b, 40, self.stop_threshold);
        put64(&mut b, 48, self.silence_threshold);
        put64(&mut b, 56, self.silence_size);
        put64(&mut b, 64, self.boundary);
        put32(&mut b, 72, self.proto);
        put32(&mut b, 76, self.tstamp_type);
        b
    }
}

/// `struct snd_pcm_status`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub state: u32,
    pub trigger_tstamp: Timespec,
    pub tstamp: Timespec,
    pub appl_ptr: u64,
    pub hw_ptr: u64,
    pub delay: i64,
    pub avail: u64,
    pub avail_max: u64,
    pub suspended_state: u32,
}

impl Status {
    pub fn encode(&self) -> [u8; STATUS_LEN] {
        let mut b = [0u8; STATUS_LEN];
        put32(&mut b, 0, self.state);
        self.trigger_tstamp.put(&mut b, 8);
        self.tstamp.put(&mut b, 24);
        put64(&mut b, 40, self.appl_ptr);
        put64(&mut b, 48, self.hw_ptr);
        put64(&mut b, 56, self.delay as u64);
        put64(&mut b, 64, self.avail);
        put64(&mut b, 72, self.avail_max);
        put32(&mut b, 88, self.suspended_state);
        // The driver timestamp is the reference timestamp too.
        self.tstamp.put(&mut b, 112);
        b
    }

    pub fn decode_state(b: &[u8]) -> u32 {
        get32(b, 0)
    }
}

/// `struct snd_pcm_sync_ptr` (the 64-bit layout).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncPtr {
    pub flags: u32,
    pub state: u32,
    pub hw_ptr: u64,
    pub tstamp: Timespec,
    pub suspended_state: u32,
    pub appl_ptr: u64,
    pub avail_min: u64,
}

impl SyncPtr {
    pub fn decode(b: &[u8; SYNC_PTR_LEN]) -> SyncPtr {
        SyncPtr {
            flags: get32(b, 0),
            state: get32(b, 8),
            hw_ptr: get64(b, 16),
            tstamp: Timespec { sec: get64(b, 24) as i64, nsec: get64(b, 32) as i64 },
            suspended_state: get32(b, 40),
            appl_ptr: get64(b, 72),
            avail_min: get64(b, 80),
        }
    }

    pub fn encode(&self) -> [u8; SYNC_PTR_LEN] {
        let mut b = [0u8; SYNC_PTR_LEN];
        put32(&mut b, 0, self.flags);
        put32(&mut b, 8, self.state);
        put64(&mut b, 16, self.hw_ptr);
        self.tstamp.put(&mut b, 24);
        put32(&mut b, 40, self.suspended_state);
        put64(&mut b, 72, self.appl_ptr);
        put64(&mut b, 80, self.avail_min);
        b
    }
}

/// `struct snd_xferi`: `result`, the buffer address and the frame count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Xferi {
    pub result: i64,
    pub buf: u64,
    pub frames: u64,
}

impl Xferi {
    pub fn decode(b: &[u8; XFERI_LEN]) -> Xferi {
        Xferi { result: get64(b, 0) as i64, buf: get64(b, 8), frames: get64(b, 16) }
    }

    /// Just the `result` field, which the kernel writes back.
    pub fn result_bytes(result: i64) -> [u8; 8] {
        result.to_le_bytes()
    }
}
