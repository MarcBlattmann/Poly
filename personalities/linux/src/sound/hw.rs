//! Hardware parameters: `struct snd_pcm_hw_params` and the refinement
//! behind `SNDRV_PCM_IOCTL_HW_REFINE` and `SNDRV_PCM_IOCTL_HW_PARAMS`.
//!
//! A hardware parameter set is three masks (access, format, subformat)
//! and twelve intervals (sample bits, frame bits, channels, rate, period
//! time/size/bytes, periods, buffer time/size/bytes, tick time). Each
//! interval is a set of reals `[min, max]` whose ends may be open, possibly
//! restricted to integers (`struct snd_interval`). Refining narrows every
//! mask and interval to the values that can still be part of a
//! configuration the device supports, using the relations between them
//! (ALSA's PCM documentation: "the hardware parameters are refined by the
//! rules until nothing changes"):
//!
//! ```text
//! frame_bits   = sample_bits * channels
//! period_bytes = period_size * frame_bits / 8      buffer_bytes = buffer_size * frame_bits / 8
//! buffer_size  = period_size * periods            buffer_bytes = period_bytes * periods
//! period_time  = period_size * 1000000 / rate       buffer_time  = buffer_size * 1000000 / rate
//! ```
//!
//! plus the device's limits ([`Caps`]): its formats, its rates (a list,
//! not a range), channels, period and buffer sizes. Only interleaved
//! read/write access is offered (no mmap). [`choose`] then picks one
//! configuration out of what is left, the way `HW_PARAMS` must.

use super::abi::{self, HW_PARAMS_LEN, access, format, info, param};
use crate::errno::Errno;

/// `struct snd_mask`: 256 bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mask {
    pub bits: [u32; 8],
}

impl Mask {
    pub const fn full() -> Mask {
        Mask { bits: [u32::MAX; 8] }
    }

    pub fn of(values: &[u32]) -> Mask {
        let mut m = Mask::default();
        for &v in values {
            m.set(v);
        }
        m
    }

    pub fn test(&self, v: u32) -> bool {
        v < 256 && self.bits[(v / 32) as usize] & (1 << (v % 32)) != 0
    }

    pub fn set(&mut self, v: u32) {
        if v < 256 {
            self.bits[(v / 32) as usize] |= 1 << (v % 32);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|&w| w == 0)
    }

    pub fn first(&self) -> Option<u32> {
        (0..256).find(|&v| self.test(v))
    }

    pub fn single(&self) -> Option<u32> {
        let n: u32 = self.bits.iter().map(|w| w.count_ones()).sum();
        if n == 1 { self.first() } else { None }
    }

    pub fn values(&self) -> impl Iterator<Item = u32> + '_ {
        (0..256).filter(|&v| self.test(v))
    }

    /// Keep only the values also in `other`; true if anything went.
    pub fn intersect(&mut self, other: &Mask) -> bool {
        let before = *self;
        for (a, b) in self.bits.iter_mut().zip(other.bits) {
            *a &= b;
        }
        *self != before
    }

    fn decode(b: &[u8]) -> Mask {
        Mask { bits: core::array::from_fn(|i| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap_or([0; 4]))) }
    }

    fn encode(&self, b: &mut [u8]) {
        for (i, w) in self.bits.iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
    }
}

/// The interval is empty: no configuration is possible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Empty;

/// `struct snd_interval`: reals from `min` to `max`, each end open or
/// closed, integers only if `integer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub min: u32,
    pub max: u32,
    pub openmin: bool,
    pub openmax: bool,
    pub integer: bool,
    pub empty: bool,
}

impl Default for Interval {
    fn default() -> Interval {
        Interval::full()
    }
}

impl Interval {
    /// Every value.
    pub const fn full() -> Interval {
        Interval { min: 0, max: u32::MAX, openmin: false, openmax: false, integer: false, empty: false }
    }

    /// Exactly `v`.
    pub const fn single_value(v: u32) -> Interval {
        Interval { min: v, max: v, openmin: false, openmax: false, integer: true, empty: false }
    }

    /// The integers from `min` to `max`.
    pub const fn range(min: u32, max: u32) -> Interval {
        Interval { min, max, openmin: false, openmax: false, integer: true, empty: false }
    }

    fn decode(b: &[u8]) -> Interval {
        let w = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap_or([0; 4]));
        let f = w(8);
        Interval {
            min: w(0),
            max: w(4),
            openmin: f & 1 != 0,
            openmax: f & 2 != 0,
            integer: f & 4 != 0,
            empty: f & 8 != 0,
        }
    }

    fn encode(&self, b: &mut [u8]) {
        b[0..4].copy_from_slice(&self.min.to_le_bytes());
        b[4..8].copy_from_slice(&self.max.to_le_bytes());
        let f =
            self.openmin as u32 | (self.openmax as u32) << 1 | (self.integer as u32) << 2 | (self.empty as u32) << 3;
        b[8..12].copy_from_slice(&f.to_le_bytes());
    }

    /// The one value left, if only one is.
    pub fn single(&self) -> Option<u32> {
        (!self.empty && self.min == self.max && !self.openmin && !self.openmax).then_some(self.min)
    }

    /// Make the ends tight (integers: an open end moves to the next
    /// integer) and check that something is left.
    fn normalize(&mut self) -> Result<(), Empty> {
        if self.empty {
            return Err(Empty);
        }
        if self.integer {
            if self.openmin {
                self.min = self.min.checked_add(1).ok_or(Empty)?;
                self.openmin = false;
            }
            if self.openmax {
                self.max = self.max.checked_sub(1).ok_or(Empty)?;
                self.openmax = false;
            }
        }
        if self.min > self.max || (self.min == self.max && (self.openmin || self.openmax)) {
            self.empty = true;
            return Err(Empty);
        }
        Ok(())
    }

    /// Narrow to what is also in `other`; true if anything changed.
    pub fn refine(&mut self, other: &Interval) -> Result<bool, Empty> {
        if other.empty {
            self.empty = true;
            return Err(Empty);
        }
        let before = *self;
        if other.min > self.min {
            self.min = other.min;
            self.openmin = other.openmin;
        } else if other.min == self.min && other.openmin {
            self.openmin = true;
        }
        if other.max < self.max {
            self.max = other.max;
            self.openmax = other.openmax;
        } else if other.max == self.max && other.openmax {
            self.openmax = true;
        }
        self.integer |= other.integer;
        self.normalize()?;
        Ok(*self != before)
    }

    /// Lower and upper ends as `(value, open)` pairs over 64 bits.
    fn lo(&self) -> (u64, bool) {
        (self.min as u64, self.openmin)
    }

    fn hi(&self) -> (u64, bool) {
        (self.max as u64, self.openmax)
    }
}

/// The interval with ends `lo` and `hi` (64-bit values clamped to 32
/// bits: an upper end past `u32::MAX` means "no limit", closed).
fn from_ends(lo: (u64, bool), hi: (u64, bool), integer: bool) -> Interval {
    let (min, openmin) = if lo.0 > u32::MAX as u64 { (u32::MAX, true) } else { (lo.0 as u32, lo.1) };
    let (max, openmax) = if hi.0 > u32::MAX as u64 { (u32::MAX, false) } else { (hi.0 as u32, hi.1) };
    Interval { min, max, openmin, openmax, integer, empty: false }
}

/// `n / d` rounded down, open if not exact.
fn div_down(n: u64, d: u64, open: bool) -> (u64, bool) {
    match n.checked_div(d) {
        Some(q) => (q, open || !n.is_multiple_of(d)),
        None => (u64::MAX, false),
    }
}

/// `n / d` rounded up, open if not exact.
fn div_up(n: u64, d: u64, open: bool) -> (u64, bool) {
    if d == 0 { (u64::MAX, false) } else { (n.div_ceil(d), open || !n.is_multiple_of(d)) }
}

/// `a * b`.
pub fn mul(a: &Interval, b: &Interval) -> Interval {
    let (lo, hi) = (
        (a.lo().0.saturating_mul(b.lo().0), a.openmin || b.openmin),
        (a.hi().0.saturating_mul(b.hi().0), a.openmax || b.openmax),
    );
    from_ends(lo, hi, a.integer && b.integer)
}

/// `a / b`.
pub fn div(a: &Interval, b: &Interval) -> Interval {
    let lo = div_down(a.lo().0, b.hi().0, a.openmin || b.openmax);
    let hi = div_up(a.hi().0, b.lo().0, a.openmax || b.openmin);
    from_ends(lo, hi, false)
}

/// `a * b / k`.
pub fn muldivk(a: &Interval, b: &Interval, k: u32) -> Interval {
    let k = k as u64;
    let lo = div_down(a.lo().0.saturating_mul(b.lo().0), k, a.openmin || b.openmin);
    let hi = div_up(a.hi().0.saturating_mul(b.hi().0), k, a.openmax || b.openmax);
    from_ends(lo, hi, false)
}

/// `a * k / b`.
pub fn mulkdiv(a: &Interval, k: u32, b: &Interval) -> Interval {
    let k = k as u64;
    let lo = div_down(a.lo().0.saturating_mul(k), b.hi().0, a.openmin || b.openmax);
    let hi = div_up(a.hi().0.saturating_mul(k), b.lo().0, a.openmax || b.openmin);
    from_ends(lo, hi, false)
}

/// `struct snd_pcm_hw_params`, the parts this implementation reads and
/// writes; everything else of the caller's bytes is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HwParams {
    pub flags: u32,
    /// Access, format and subformat.
    pub masks: [Mask; 3],
    /// [`param::SAMPLE_BITS`] to [`param::TICK_TIME`].
    pub intervals: [Interval; 12],
    pub rmask: u32,
    pub cmask: u32,
    pub info: u32,
    pub msbits: u32,
    pub rate_num: u32,
    pub rate_den: u32,
    pub fifo_size: u64,
    raw: [u8; HW_PARAMS_LEN],
}

const MASKS_AT: usize = 4;
const INTERVALS_AT: usize = 260;

impl Default for HwParams {
    fn default() -> HwParams {
        HwParams::any()
    }
}

impl HwParams {
    /// Everything open (what `snd_pcm_hw_params_any` starts from).
    pub fn any() -> HwParams {
        HwParams {
            flags: 0,
            masks: [Mask::full(); 3],
            intervals: [Interval::full(); 12],
            rmask: u32::MAX,
            cmask: 0,
            info: 0,
            msbits: 0,
            rate_num: 0,
            rate_den: 0,
            fifo_size: 0,
            raw: [0; HW_PARAMS_LEN],
        }
    }

    pub fn decode(b: &[u8; HW_PARAMS_LEN]) -> HwParams {
        let w = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap_or([0; 4]));
        HwParams {
            flags: w(0),
            masks: core::array::from_fn(|i| Mask::decode(&b[MASKS_AT + i * 32..])),
            intervals: core::array::from_fn(|i| Interval::decode(&b[INTERVALS_AT + i * 12..])),
            rmask: w(512),
            cmask: w(516),
            info: w(520),
            msbits: w(524),
            rate_num: w(528),
            rate_den: w(532),
            fifo_size: u64::from_le_bytes(b[536..544].try_into().unwrap_or([0; 8])),
            raw: *b,
        }
    }

    pub fn encode(&self) -> [u8; HW_PARAMS_LEN] {
        let mut b = self.raw;
        b[0..4].copy_from_slice(&self.flags.to_le_bytes());
        for (i, m) in self.masks.iter().enumerate() {
            m.encode(&mut b[MASKS_AT + i * 32..MASKS_AT + i * 32 + 32]);
        }
        for (i, iv) in self.intervals.iter().enumerate() {
            iv.encode(&mut b[INTERVALS_AT + i * 12..INTERVALS_AT + i * 12 + 12]);
        }
        for (at, v) in [(512, self.rmask), (516, self.cmask), (520, self.info), (524, self.msbits)] {
            b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[528..532].copy_from_slice(&self.rate_num.to_le_bytes());
        b[532..536].copy_from_slice(&self.rate_den.to_le_bytes());
        b[536..544].copy_from_slice(&self.fifo_size.to_le_bytes());
        b
    }

    pub fn mask(&self, p: usize) -> &Mask {
        &self.masks[p]
    }

    pub fn mask_mut(&mut self, p: usize) -> &mut Mask {
        &mut self.masks[p]
    }

    pub fn interval(&self, p: usize) -> &Interval {
        &self.intervals[p - param::FIRST_INTERVAL]
    }

    pub fn interval_mut(&mut self, p: usize) -> &mut Interval {
        &mut self.intervals[p - param::FIRST_INTERVAL]
    }

    /// Narrow interval `p` to `with`; true if it changed.
    fn narrow(&mut self, p: usize, with: Interval) -> Result<bool, Empty> {
        self.interval_mut(p).refine(&with)
    }
}

/// What the device can do, in ALSA's terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    /// `SNDRV_PCM_ACCESS_*` values offered.
    pub access: Mask,
    /// `SNDRV_PCM_INFO_*` bits of every configuration.
    pub info: u32,
    /// `SNDRV_PCM_FORMAT_*` values, each one [`format::describe`] knows.
    pub formats: Mask,
    /// Rates in hertz, increasing; `rates[..rate_count]` are used.
    pub rates: [u32; 16],
    pub rate_count: usize,
    pub channels_min: u32,
    pub channels_max: u32,
    pub period_bytes_min: u32,
    pub period_bytes_max: u32,
    pub periods_min: u32,
    pub periods_max: u32,
    pub buffer_bytes_max: u32,
}

impl Caps {
    pub fn rates(&self) -> &[u32] {
        &self.rates[..self.rate_count.min(16)]
    }
}

/// `SNDRV_PCM_INFO_*` of a device read and written in blocks of whole
/// periods: interleaved frames.
pub const INFO: u32 = info::INTERLEAVED | info::BLOCK_TRANSFER | info::BATCH;

/// `SNDRV_PCM_INFO_*` of a stream whose ring buffer may also be mapped
/// (the audio server's: ADR 0109).
pub const INFO_MMAP: u32 = INFO | info::MMAP | info::MMAP_VALID;

/// The access modes of a stream that may be mapped: interleaved, through
/// read/write or mmap.
pub fn access_rw_and_mmap() -> Mask {
    Mask::of(&[access::RW_INTERLEAVED, access::MMAP_INTERLEAVED])
}

/// Parameters that only take integers.
const INTEGER_PARAMS: [usize; 9] = [
    param::SAMPLE_BITS,
    param::FRAME_BITS,
    param::CHANNELS,
    param::RATE,
    param::PERIOD_SIZE,
    param::PERIOD_BYTES,
    param::PERIODS,
    param::BUFFER_SIZE,
    param::BUFFER_BYTES,
];

/// The physical sample sizes of the formats in `m`, as an interval.
fn sample_bits_of(m: &Mask) -> Option<Interval> {
    let bits = m.values().filter_map(|f| format::describe(f).map(|d| d.0));
    let (lo, hi) = bits.fold((u32::MAX, 0), |(lo, hi), b| (lo.min(b), hi.max(b)));
    (lo <= hi).then(|| Interval::range(lo, hi))
}

/// Narrow the rate to the device's list.
fn snap_rate(rate: &Interval, rates: &[u32]) -> Result<Interval, Empty> {
    let inside = |r: &&u32| {
        let r = **r;
        (r > rate.min || (r == rate.min && !rate.openmin)) && (r < rate.max || (r == rate.max && !rate.openmax))
    };
    let lo = *rates.iter().find(inside).ok_or(Empty)?;
    let hi = *rates.iter().rev().find(inside).ok_or(Empty)?;
    Ok(Interval::range(lo, hi))
}

/// One pass of every rule; true if anything changed.
fn apply_rules(p: &mut HwParams, caps: &Caps) -> Result<bool, Empty> {
    use param::*;
    let mut changed = false;
    // Formats and their sample size.
    let sb = sample_bits_of(p.mask(FORMAT)).ok_or(Empty)?;
    changed |= p.narrow(SAMPLE_BITS, sb)?;
    let allowed = *p.interval(SAMPLE_BITS);
    let mut keep = Mask::default();
    for f in p.mask(FORMAT).values() {
        if format::describe(f).is_some_and(|d| d.0 >= allowed.min && d.0 <= allowed.max) {
            keep.set(f);
        }
    }
    changed |= p.mask_mut(FORMAT).intersect(&keep);
    if p.mask(FORMAT).is_empty() {
        return Err(Empty);
    }
    let rate = snap_rate(p.interval(RATE), caps.rates())?;
    changed |= p.narrow(RATE, rate)?;

    let iv = |p: &HwParams, k| *p.interval(k);
    // frame_bits = sample_bits * channels
    changed |= p.narrow(FRAME_BITS, mul(&iv(p, SAMPLE_BITS), &iv(p, CHANNELS)))?;
    changed |= p.narrow(SAMPLE_BITS, div(&iv(p, FRAME_BITS), &iv(p, CHANNELS)))?;
    changed |= p.narrow(CHANNELS, div(&iv(p, FRAME_BITS), &iv(p, SAMPLE_BITS)))?;
    // bytes = size * frame_bits / 8, for periods and the buffer
    for (size, bytes) in [(PERIOD_SIZE, PERIOD_BYTES), (BUFFER_SIZE, BUFFER_BYTES)] {
        changed |= p.narrow(bytes, muldivk(&iv(p, size), &iv(p, FRAME_BITS), 8))?;
        changed |= p.narrow(size, mulkdiv(&iv(p, bytes), 8, &iv(p, FRAME_BITS)))?;
        changed |= p.narrow(FRAME_BITS, mulkdiv(&iv(p, bytes), 8, &iv(p, size)))?;
    }
    // buffer_size = period_size * periods
    changed |= p.narrow(BUFFER_SIZE, mul(&iv(p, PERIOD_SIZE), &iv(p, PERIODS)))?;
    changed |= p.narrow(PERIOD_SIZE, div(&iv(p, BUFFER_SIZE), &iv(p, PERIODS)))?;
    changed |= p.narrow(PERIODS, div(&iv(p, BUFFER_SIZE), &iv(p, PERIOD_SIZE)))?;
    // buffer_bytes = period_bytes * periods
    changed |= p.narrow(BUFFER_BYTES, mul(&iv(p, PERIOD_BYTES), &iv(p, PERIODS)))?;
    changed |= p.narrow(PERIOD_BYTES, div(&iv(p, BUFFER_BYTES), &iv(p, PERIODS)))?;
    changed |= p.narrow(PERIODS, div(&iv(p, BUFFER_BYTES), &iv(p, PERIOD_BYTES)))?;
    // time = size * 1000000 / rate, for periods and the buffer
    for (size, time) in [(PERIOD_SIZE, PERIOD_TIME), (BUFFER_SIZE, BUFFER_TIME)] {
        changed |= p.narrow(time, mulkdiv(&iv(p, size), 1_000_000, &iv(p, RATE)))?;
        changed |= p.narrow(size, muldivk(&iv(p, time), &iv(p, RATE), 1_000_000))?;
        changed |= p.narrow(RATE, mulkdiv(&iv(p, size), 1_000_000, &iv(p, time)))?;
    }
    Ok(changed)
}

/// The changes `before` -> `after` as a `cmask` (one bit per parameter).
fn changed_params(before: &HwParams, after: &HwParams) -> u32 {
    let mut m = 0;
    for i in 0..3 {
        if before.masks[i] != after.masks[i] {
            m |= 1 << i;
        }
    }
    for i in 0..12 {
        if before.intervals[i] != after.intervals[i] {
            m |= 1 << (param::FIRST_INTERVAL + i);
        }
    }
    m
}

/// `SNDRV_PCM_IOCTL_HW_REFINE`: narrow `p` to what `caps` allows. `cmask`
/// says what changed; `info`, `msbits`, `rate_num`/`rate_den` and
/// `fifo_size` describe the result as far as it is determined. `EINVAL`
/// if nothing is left.
pub fn refine(p: &mut HwParams, caps: &Caps) -> Result<(), Errno> {
    use param::*;
    let before = *p;
    let fail = |_: Empty| Errno::EINVAL;
    if p.intervals.iter().any(|i| i.empty) {
        return Err(Errno::EINVAL);
    }
    p.mask_mut(ACCESS).intersect(&caps.access);
    p.mask_mut(FORMAT).intersect(&caps.formats);
    p.mask_mut(SUBFORMAT).intersect(&Mask::of(&[abi::SUBFORMAT_STD]));
    if p.masks.iter().any(Mask::is_empty) {
        return Err(Errno::EINVAL);
    }
    for k in INTEGER_PARAMS {
        let mut i = *p.interval(k);
        i.integer = true;
        p.narrow(k, i).map_err(fail)?;
    }
    p.narrow(CHANNELS, Interval::range(caps.channels_min, caps.channels_max)).map_err(fail)?;
    p.narrow(PERIOD_BYTES, Interval::range(caps.period_bytes_min, caps.period_bytes_max)).map_err(fail)?;
    p.narrow(PERIODS, Interval::range(caps.periods_min, caps.periods_max)).map_err(fail)?;
    p.narrow(BUFFER_BYTES, Interval::range(1, caps.buffer_bytes_max)).map_err(fail)?;
    // The rules only ever narrow, over finite integers, so this ends; the
    // bound is a guard.
    for _ in 0..64 {
        if !apply_rules(p, caps).map_err(fail)? {
            break;
        }
    }
    p.cmask = changed_params(&before, p);
    p.info = caps.info;
    p.msbits = match p.mask(FORMAT).single().and_then(format::describe) {
        Some((_, width, _)) => width,
        None => 0,
    };
    match p.interval(RATE).single() {
        Some(r) => (p.rate_num, p.rate_den) = (r, 1),
        None => (p.rate_num, p.rate_den) = (0, 0),
    }
    p.fifo_size = 0;
    Ok(())
}

/// One configuration, all parameters decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HwConfig {
    pub access: u32,
    /// `SNDRV_PCM_FORMAT_*`.
    pub format: u32,
    pub channels: u32,
    pub rate: u32,
    pub period_frames: u32,
    pub periods: u32,
    pub buffer_frames: u32,
    /// Bytes per frame.
    pub frame_bytes: u32,
}

impl HwConfig {
    pub const fn period_bytes(&self) -> u32 {
        self.period_frames * self.frame_bytes
    }

    pub const fn buffer_bytes(&self) -> u32 {
        self.buffer_frames * self.frame_bytes
    }
}

/// Fix parameter `k` to `v` if that leaves a configuration, else to the
/// interval's lower end.
fn pick(p: &mut HwParams, caps: &Caps, k: usize, v: u32) -> Result<(), Errno> {
    let cur = *p.interval(k);
    if cur.single().is_some() {
        return Ok(());
    }
    let v = v.clamp(cur.min, cur.max);
    for candidate in [v, cur.min] {
        let mut trial = *p;
        if trial.narrow(k, Interval::single_value(candidate)).is_ok() && refine(&mut trial, caps).is_ok() {
            *p = trial;
            return Ok(());
        }
    }
    Err(Errno::EINVAL)
}

/// Fix mask `k` to `v` (if allowed) or its first value.
fn pick_mask(p: &mut HwParams, caps: &Caps, k: usize, prefer: u32) -> Result<(), Errno> {
    let m = *p.mask(k);
    if m.single().is_some() {
        return Ok(());
    }
    let order = [prefer].into_iter().filter(|&v| m.test(v)).chain(m.values());
    for v in order {
        let mut trial = *p;
        trial.masks[k] = Mask::of(&[v]);
        if refine(&mut trial, caps).is_ok() {
            *p = trial;
            return Ok(());
        }
    }
    Err(Errno::EINVAL)
}

/// `SNDRV_PCM_IOCTL_HW_PARAMS`: refine, then decide whatever the caller
/// left open, one parameter at a time, refining after each: access and
/// format (S16_LE when possible), stereo when possible, 48 kHz (else 44.1
/// kHz, else the lowest rate), 4 periods, periods of about 20 ms. Returns
/// the configuration; `p` holds it in ALSA's form.
pub fn choose(p: &mut HwParams, caps: &Caps) -> Result<HwConfig, Errno> {
    use param::*;
    let original = *p;
    refine(p, caps)?;
    pick_mask(p, caps, ACCESS, access::RW_INTERLEAVED)?;
    pick_mask(p, caps, FORMAT, format::S16_LE)?;
    pick_mask(p, caps, SUBFORMAT, abi::SUBFORMAT_STD)?;
    pick(p, caps, CHANNELS, 2)?;
    let rate = *p.interval(RATE);
    let preferred = [48000, 44100].into_iter().find(|&r| r >= rate.min && r <= rate.max).unwrap_or(rate.min);
    pick(p, caps, RATE, preferred)?;
    let rate = p.interval(RATE).single().ok_or(Errno::EINVAL)?;
    pick(p, caps, PERIODS, 4)?;
    pick(p, caps, PERIOD_SIZE, rate / 50)?;
    pick(p, caps, BUFFER_SIZE, 0)?;
    let get = |k| p.interval(k).single().ok_or(Errno::EINVAL);
    let fmt = p.mask(FORMAT).single().ok_or(Errno::EINVAL)?;
    let (bits, _, _) = format::describe(fmt).ok_or(Errno::EINVAL)?;
    let channels = get(CHANNELS)?;
    let cfg = HwConfig {
        access: p.mask(ACCESS).single().ok_or(Errno::EINVAL)?,
        format: fmt,
        channels,
        rate,
        period_frames: get(PERIOD_SIZE)?,
        periods: get(PERIODS)?,
        buffer_frames: get(BUFFER_SIZE)?,
        frame_bytes: bits / 8 * channels,
    };
    if cfg.period_frames.checked_mul(cfg.periods) != Some(cfg.buffer_frames) || cfg.frame_bytes == 0 {
        return Err(Errno::EINVAL);
    }
    p.cmask = changed_params(&original, p);
    Ok(cfg)
}
