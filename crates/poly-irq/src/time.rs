//! Counters, tick / nanosecond conversion, calibration and a monotonic clock.
//!
//! A [`Counter`] is anything that counts up at a fixed rate: the TSC, the
//! HPET main counter, the ACPI PM timer, or the APIC timer seen through
//! [`crate::lapic::ApicTimerCounter`]. [`calibrate`] measures one counter's
//! rate against another whose rate is known, [`Scale`] converts between rates
//! without overflow, and [`MonotonicClock`] turns a counter into nanoseconds
//! since boot.
//!
//! The TSC is the preferred time base only when it is invariant (SDM Vol. 3B
//! §18.17.1): otherwise its rate follows the core frequency and it can stop
//! in deep C-states, and the clock uses the reference counter instead.

use core::hint::spin_loop;
use core::sync::atomic::{AtomicU64, Ordering};

pub const NANOS_PER_SEC: u64 = 1_000_000_000;
pub const FEMTOS_PER_SEC: u64 = 1_000_000_000_000_000;

/// `value * mul / div`, rounded down, with a 128-bit intermediate so the
/// product cannot overflow. Saturates at `u64::MAX`; `div == 0` also gives
/// `u64::MAX`. A 128-bit division is a library call, so hot paths use
/// [`Scale`] instead.
pub const fn mul_div(value: u64, mul: u64, div: u64) -> u64 {
    if div == 0 {
        return u64::MAX;
    }
    let q = value as u128 * mul as u128 / div as u128;
    if q > u64::MAX as u128 { u64::MAX } else { q as u64 }
}

/// A fixed-point rate conversion: `value * to_hz / from_hz` computed as
/// `(value * mult) >> shift`, one 64 x 64 -> 128-bit multiply and a shift.
///
/// `shift` is chosen as large as possible (at most 63) with `mult` still
/// fitting 64 bits, which bounds the error: the result is never above the
/// exact quotient and at most 2 below it, for every input that does not
/// saturate. Results saturate at `u64::MAX`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scale {
    mult: u64,
    shift: u32,
}

impl Scale {
    pub const MAX_SHIFT: u32 = 63;

    /// Conversion from a counter at `from_hz` to one at `to_hz`. `None` if
    /// either is 0.
    pub const fn new(from_hz: u64, to_hz: u64) -> Option<Scale> {
        if from_hz == 0 || to_hz == 0 {
            return None;
        }
        // `to_hz / from_hz < 2^64`, so shift 0 always fits and the loop ends.
        let mut shift = Self::MAX_SHIFT;
        loop {
            let mult = ((to_hz as u128) << shift) / from_hz as u128;
            if mult <= u64::MAX as u128 {
                return Some(Scale { mult: mult as u64, shift });
            }
            shift -= 1;
        }
    }

    /// Ticks of a counter at `hz` to nanoseconds.
    pub const fn ticks_to_ns(hz: u64) -> Option<Scale> {
        Scale::new(hz, NANOS_PER_SEC)
    }

    /// Nanoseconds to ticks of a counter at `hz`.
    pub const fn ns_to_ticks(hz: u64) -> Option<Scale> {
        Scale::new(NANOS_PER_SEC, hz)
    }

    pub const fn apply(self, value: u64) -> u64 {
        let p = (value as u128 * self.mult as u128) >> self.shift;
        if p > u64::MAX as u128 { u64::MAX } else { p as u64 }
    }

    pub const fn mult(self) -> u64 {
        self.mult
    }

    pub const fn shift(self) -> u32 {
        self.shift
    }
}

/// A free-running counter that counts up at a fixed rate and wraps to 0
/// after `2^bits() - 1`.
///
/// Reads take `&self`: counters are hardware state that changes on its own.
pub trait Counter {
    fn read(&self) -> u64;

    /// Width in bits, 1 to 64. The HPET main counter can be 32 bits wide and
    /// the ACPI PM timer 24 or 32 bits.
    fn bits(&self) -> u32 {
        64
    }
}

impl<T: Counter + ?Sized> Counter for &T {
    fn read(&self) -> u64 {
        (**self).read()
    }

    fn bits(&self) -> u32 {
        (**self).bits()
    }
}

/// All-ones in the low `bits` bits.
pub const fn mask(bits: u32) -> u64 {
    if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 }
}

/// Ticks from `earlier` to `later` on a counter `bits` wide, across at most
/// one wrap.
pub const fn delta(earlier: u64, later: u64, bits: u32) -> u64 {
    later.wrapping_sub(earlier) & mask(bits)
}

/// Extends a counter narrower than 64 bits to 64 bits.
///
/// Every call must come less than one wrap period after the previous one
/// (`2^32` HPET ticks is about 5 minutes at 14.3 MHz; `2^24` PM timer ticks is
/// 4.7 seconds), which a periodic tick guarantees.
///
/// Safe to share between CPUs because [`Extender::extend`] loads the stored
/// value *before* it reads the counter: the reading is then never older than
/// the stored value, so it can never be mistaken for one a whole wrap ahead.
/// (A raw value read first and extended afterwards could be, if another CPU
/// stored a newer one in between.) The stored value only grows.
#[derive(Debug)]
pub struct Extender {
    last: AtomicU64,
    bits: u32,
}

impl Extender {
    /// `initial` is a raw reading taken now; extended values continue from it.
    pub const fn new(bits: u32, initial: u64) -> Extender {
        Extender { last: AtomicU64::new(initial & mask(bits)), bits }
    }

    /// Calls `read_raw` to read the counter and returns its extended value.
    /// The load before the counter read is an acquire, and x86 does not
    /// reorder loads with loads (SDM Vol. 3A §10.2.2), which keeps an MMIO
    /// or port read of the counter after it.
    pub fn extend(&self, read_raw: impl FnOnce() -> u64) -> u64 {
        if self.bits >= 64 {
            return read_raw();
        }
        let last = self.last.load(Ordering::Acquire);
        let raw = read_raw();
        let now = last.wrapping_add(delta(last, raw, self.bits));
        self.last.fetch_max(now, Ordering::AcqRel);
        now
    }
}

/// Most measurements [`calibrate`] takes.
pub const MAX_SAMPLES: usize = 32;

/// How [`calibrate`] measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalibrationConfig {
    /// Measurements to take, 1 to [`MAX_SAMPLES`].
    pub samples: usize,
    /// Length of each measurement in reference ticks. Must be under half the
    /// reference counter's wrap period.
    pub window: u64,
    /// Reference reads per measurement before giving up on a reference that
    /// does not advance.
    pub max_spins: u64,
    /// A measurement is kept only if its own read uncertainty and its
    /// distance from the median are both within this many parts per million.
    pub tolerance_ppm: u64,
}

impl CalibrationConfig {
    /// Six 10 ms measurements, 500 ppm tolerance: about 60 ms of boot time.
    pub const fn for_reference(reference_hz: u64) -> CalibrationConfig {
        let window = if reference_hz / 100 == 0 { 1 } else { reference_hz / 100 };
        CalibrationConfig {
            samples: 6,
            window,
            max_spins: window.saturating_mul(64).saturating_add(100_000),
            tolerance_ppm: 500,
        }
    }
}

/// Result of [`calibrate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Calibration {
    /// The measured counter's rate.
    pub hz: u64,
    /// Measurements averaged into `hz`.
    pub used: usize,
    /// Measurements taken.
    pub taken: usize,
    /// Spread (max - min) of the measurements used, in ppm of `hz`.
    pub spread_ppm: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibrationError {
    /// Bad sample count, empty window, a window too long for the reference
    /// counter's width, or a reference rate of 0.
    InvalidConfig,
    /// The reference counter did not advance by a window within `max_spins`
    /// reads.
    ReferenceStuck,
    /// The measured counter did not advance during a window (or, for the APIC
    /// timer, ran out).
    TargetStuck,
    /// No majority of the measurements agreed within the tolerance.
    Inconsistent,
}

/// One target / reference / target reading triple.
#[derive(Clone, Copy)]
struct Bracket {
    /// Target value half-way between the two target reads.
    mid: u64,
    reference: u64,
    /// Target ticks between the two target reads: how far off `mid` can be.
    width: u64,
}

fn bracket<T: Counter, R: Counter>(target: &T, reference: &R) -> Bracket {
    let bits = target.bits();
    let mut best: Option<Bracket> = None;
    // The tightest of three: an interrupt or SMI between the reads widens one
    // bracket, rarely all three.
    for _ in 0..3 {
        let a = target.read();
        let r = reference.read();
        let b = target.read();
        let width = delta(a, b, bits);
        let mid = a.wrapping_add(width / 2) & mask(bits);
        if best.is_none_or(|best| width < best.width) {
            best = Some(Bracket { mid, reference: r, width });
        }
    }
    best.unwrap_or(Bracket { mid: 0, reference: 0, width: u64::MAX })
}

/// Measures the rate of `target` against `reference`, which counts at
/// `reference_hz`.
///
/// Each measurement brackets a reference read between two target reads at
/// both ends of a window of `config.window` reference ticks. Measurements
/// whose brackets are too wide (an interrupt, SMI or vCPU preemption hit
/// them) are dropped, the rest are compared against their median, and the
/// ones within `config.tolerance_ppm` of it are averaged if they are a
/// majority of all measurements taken.
///
/// Run it with interrupts disabled. Every wait is bounded by
/// `config.max_spins`.
pub fn calibrate<T: Counter, R: Counter>(
    target: &T,
    reference: &R,
    reference_hz: u64,
    config: &CalibrationConfig,
) -> Result<Calibration, CalibrationError> {
    let (tbits, rbits) = (target.bits(), reference.bits());
    if reference_hz == 0
        || config.samples == 0
        || config.samples > MAX_SAMPLES
        || config.window == 0
        || config.window > mask(rbits) / 2
    {
        return Err(CalibrationError::InvalidConfig);
    }
    let tolerance = config.tolerance_ppm as u128;
    let mut hz = [0u64; MAX_SAMPLES];
    let mut n = 0;
    for _ in 0..config.samples {
        let start = bracket(target, reference);
        let mut spins = 0u64;
        while delta(start.reference, reference.read(), rbits) < config.window {
            spins += 1;
            if spins >= config.max_spins {
                return Err(CalibrationError::ReferenceStuck);
            }
            spin_loop();
        }
        let end = bracket(target, reference);
        let ref_ticks = delta(start.reference, end.reference, rbits);
        let ticks = delta(start.mid, end.mid, tbits);
        if ticks == 0 {
            return Err(CalibrationError::TargetStuck);
        }
        let uncertainty = (start.width as u128 + end.width as u128) / 2;
        if uncertainty * 1_000_000 > ticks as u128 * tolerance {
            continue;
        }
        hz[n] = mul_div(ticks, reference_hz, ref_ticks);
        n += 1;
    }
    let hz = &mut hz[..n];
    hz.sort_unstable();
    let median = match n {
        0 => return Err(CalibrationError::Inconsistent),
        _ if n % 2 == 0 => ((hz[n / 2 - 1] as u128 + hz[n / 2] as u128) / 2) as u64,
        _ => hz[n / 2],
    };
    let close = |f: u64| (f.abs_diff(median) as u128) * 1_000_000 <= median as u128 * tolerance;
    let (mut used, mut sum, mut min, mut max) = (0usize, 0u128, u64::MAX, 0u64);
    for &f in hz.iter().filter(|&&f| close(f)) {
        used += 1;
        sum += f as u128;
        min = min.min(f);
        max = max.max(f);
    }
    if used * 2 <= config.samples {
        return Err(CalibrationError::Inconsistent);
    }
    let mean = (sum / used as u128) as u64;
    Ok(Calibration { hz: mean, used, taken: config.samples, spread_ppm: mul_div(max - min, 1_000_000, mean) })
}

/// Which counter a [`MonotonicClock`] reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockSource {
    Tsc,
    Reference,
}

/// What [`MonotonicClock::tsc_conversion`] reports: the clock reads
/// `offset_ns + ((tsc - origin) * mult) >> shift` (wrapping subtraction;
/// the TSC is 64 bits wide), before keeping its readings monotonic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TscConversion {
    pub origin: u64,
    pub mult: u64,
    pub shift: u32,
    pub offset_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockError {
    /// The TSC is unusable and the reference rate is 0.
    NoUsableSource,
}

/// Nanoseconds since the clock was created, from the TSC when it is
/// invariant and calibrated, otherwise from the reference counter (normally
/// the HPET).
///
/// Readings never go backwards, even across CPUs whose TSCs disagree
/// slightly: the clock remembers the largest value handed out. That costs
/// one atomic read-modify-write per reading on a shared cache line; per-CPU
/// clocks can come later if it shows up in profiles.
#[derive(Debug)]
pub struct MonotonicClock<T, R> {
    tsc: T,
    reference: R,
    reference_hz: u64,
    source: ClockSource,
    hz: u64,
    scale: Scale,
    extender: Extender,
    /// Extended counter value at which `offset_ns` was current.
    origin: u64,
    offset_ns: u64,
    last_ns: AtomicU64,
}

impl<T: Counter, R: Counter> MonotonicClock<T, R> {
    /// `tsc_hz` is the calibrated TSC rate (`None` if calibration failed).
    /// The TSC is used only if `invariant_tsc` (the TSC runs at a constant
    /// rate: [`crate::CpuFeatures::constant_rate_tsc`]) and `tsc_hz` is
    /// known.
    pub fn new(
        tsc: T,
        reference: R,
        reference_hz: u64,
        tsc_hz: Option<u64>,
        invariant_tsc: bool,
    ) -> Result<MonotonicClock<T, R>, ClockError> {
        let (source, hz) = match tsc_hz {
            Some(hz) if invariant_tsc && hz > 0 => (ClockSource::Tsc, hz),
            _ if reference_hz > 0 => (ClockSource::Reference, reference_hz),
            _ => return Err(ClockError::NoUsableSource),
        };
        let mut clock = MonotonicClock {
            tsc,
            reference,
            reference_hz,
            source,
            hz,
            scale: Scale { mult: 0, shift: 0 },
            extender: Extender::new(64, 0),
            origin: 0,
            offset_ns: 0,
            last_ns: AtomicU64::new(0),
        };
        clock.switch_to(source, hz, 0);
        Ok(clock)
    }

    fn switch_to(&mut self, source: ClockSource, hz: u64, offset_ns: u64) {
        let (raw, bits) = match source {
            ClockSource::Tsc => (self.tsc.read(), self.tsc.bits()),
            ClockSource::Reference => (self.reference.read(), self.reference.bits()),
        };
        self.source = source;
        self.hz = hz;
        // `hz` is non-zero on every path here.
        self.scale = Scale::ticks_to_ns(hz).unwrap_or(Scale { mult: 0, shift: 0 });
        self.extender = Extender::new(bits, raw);
        self.origin = raw & mask(bits);
        self.offset_ns = offset_ns;
    }

    pub fn source(&self) -> ClockSource {
        self.source
    }

    /// Rate of the counter in use.
    pub fn frequency(&self) -> u64 {
        self.hz
    }

    pub fn tsc(&self) -> &T {
        &self.tsc
    }

    /// While the clock reads the TSC: how it turns a TSC reading into its
    /// nanoseconds, so that code outside the kernel (the vDSO) can compute
    /// the same time. `None` on the reference counter.
    pub fn tsc_conversion(&self) -> Option<TscConversion> {
        (self.source == ClockSource::Tsc).then_some(TscConversion {
            origin: self.origin,
            mult: self.scale.mult,
            shift: self.scale.shift,
            offset_ns: self.offset_ns,
        })
    }

    pub fn reference(&self) -> &R {
        &self.reference
    }

    pub fn now_ns(&self) -> u64 {
        let raw = self.extender.extend(|| match self.source {
            ClockSource::Tsc => self.tsc.read(),
            ClockSource::Reference => self.reference.read(),
        });
        let ticks = raw.wrapping_sub(self.origin);
        let ns = self.offset_ns.saturating_add(self.scale.apply(ticks));
        let prev = self.last_ns.fetch_max(ns, Ordering::Relaxed);
        prev.max(ns)
    }

    /// Stops using the TSC (it was found to drift or stop), continuing from
    /// the current time on the reference counter. Returns `false` if there is
    /// no reference to fall back to.
    pub fn fall_back_to_reference(&mut self) -> bool {
        if self.reference_hz == 0 {
            return false;
        }
        if self.source == ClockSource::Reference {
            return true;
        }
        let now = self.now_ns();
        self.switch_to(ClockSource::Reference, self.reference_hz, now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{FakeTsc, SimClock, SimHpet, Stuck, Xorshift};

    #[test]
    fn mul_div_edges() {
        assert_eq!(mul_div(0, 123, 7), 0);
        assert_eq!(mul_div(u64::MAX, 0, 7), 0);
        assert_eq!(mul_div(u64::MAX, u64::MAX, u64::MAX), u64::MAX);
        assert_eq!(mul_div(u64::MAX, 2, 1), u64::MAX);
        assert_eq!(mul_div(u64::MAX, 3, 4), u64::MAX / 4 * 3 + 2);
        assert_eq!(mul_div(5, 5, 0), u64::MAX);
        assert_eq!(mul_div(7, 1, 2), 3);
    }

    #[test]
    fn scale_rejects_zero_rates() {
        assert_eq!(Scale::new(0, 1), None);
        assert_eq!(Scale::new(1, 0), None);
        assert!(Scale::new(1, 1).is_some());
        assert!(Scale::new(1, u64::MAX).is_some());
        assert!(Scale::new(u64::MAX, 1).is_some());
    }

    #[test]
    fn scale_identity_and_round_numbers() {
        let one = Scale::new(1_000, 1_000).unwrap();
        for v in [0, 1, 2, 999, u32::MAX as u64, u64::MAX - 1, u64::MAX] {
            assert!(v - one.apply(v) <= 2, "{v}");
        }
        let ghz3 = Scale::ticks_to_ns(3_000_000_000).unwrap();
        assert_eq!(ghz3.apply(0), 0);
        assert!((999_999_998..=1_000_000_000).contains(&ghz3.apply(3_000_000_000)));
        // A 1 MHz counter: 1 tick = 1000 ns, and huge inputs saturate.
        let mhz = Scale::ticks_to_ns(1_000_000).unwrap();
        assert_eq!(mhz.apply(1), 1000);
        assert_eq!(mhz.apply(u64::MAX), u64::MAX);
        assert_eq!(mhz.apply(u64::MAX / 1000), u64::MAX / 1000 * 1000);
    }

    #[test]
    fn scale_matches_exact_math() {
        let mut rng = Xorshift(0x1234_5678_9abc_def0);
        let rates = [1, 3, 1_000, 32_768, 3_579_545, 14_318_180, 1_000_000_000, 2_712_345_678, 5_000_000_000, u64::MAX];
        for &from in &rates {
            for &to in &rates {
                let s = Scale::new(from, to).unwrap();
                for i in 0..200 {
                    // Mix full-range values with small ones.
                    let v = if i % 2 == 0 { rng.next() } else { rng.next() >> (rng.next() % 64) };
                    let exact = mul_div(v, to, from);
                    let got = s.apply(v);
                    if exact == u64::MAX {
                        assert!(got >= exact - 2, "{from}->{to} {v}");
                    } else {
                        assert!(got <= exact && exact - got <= 2, "{from}->{to} {v}: {got} vs {exact}");
                    }
                }
            }
        }
    }

    #[test]
    fn delta_wraps_at_the_counter_width() {
        assert_eq!(delta(0xffff_fff0, 0x10, 32), 0x20);
        assert_eq!(delta(0x00ff_fff0, 0x10, 24), 0x20);
        assert_eq!(delta(u64::MAX, 1, 64), 2);
        assert_eq!(delta(5, 5, 32), 0);
        assert_eq!(mask(0), 0);
        assert_eq!(mask(64), u64::MAX);
    }

    #[test]
    fn extender_counts_through_wraps() {
        let e = Extender::new(32, 0xffff_0000);
        assert_eq!(e.extend(|| 0xffff_8000), 0xffff_8000);
        assert_eq!(e.extend(|| 0x0000_1000), 0x1_0000_1000);
        assert_eq!(e.extend(|| 0xffff_ffff), 0x1_ffff_ffff);
        assert_eq!(e.extend(|| 0x0000_0000), 0x2_0000_0000);
        assert_eq!(e.extend(|| 0x0000_0000), 0x2_0000_0000);
        assert_eq!(e.extend(|| 0x0000_0001), 0x2_0000_0001);
        let e24 = Extender::new(24, 0xff_fffe);
        assert_eq!(e24.extend(|| 0x00_0001), 0x100_0001);
        let e64 = Extender::new(64, 5);
        assert_eq!(e64.extend(|| u64::MAX), u64::MAX);
    }

    #[test]
    fn extender_reads_after_loading() {
        // Reader A loads the stored value, then (inside its read) reader B
        // reads a later counter value and stores it. A's own reading still
        // extends correctly, and the stored value does not move back.
        let e = Extender::new(32, 0xffff_ff00);
        let a = e.extend(|| {
            assert_eq!(e.extend(|| 0x0000_0010), 0x1_0000_0010);
            0x0000_0020
        });
        assert_eq!(a, 0x1_0000_0020);
        let b = e.extend(|| {
            assert_eq!(e.extend(|| 0x0000_0040), 0x1_0000_0040);
            0x0000_0030
        });
        assert_eq!(b, 0x1_0000_0030);
        assert_eq!(e.extend(|| 0x0000_0041), 0x1_0000_0041);
    }

    const TSC_HZ: u64 = 2_712_345_678;

    fn hpet(clock: &SimClock, wide: bool) -> crate::Hpet<&SimHpet<'_>> {
        let sim = std::boxed::Box::leak(std::boxed::Box::new(SimHpet::new(clock, wide)));
        let h = crate::Hpet::new(&*sim).unwrap();
        h.enable();
        h
    }

    extern crate std;

    #[test]
    fn calibrates_a_jittery_tsc_against_the_hpet() {
        // Each read costs 0.6 to 1.4 us, like an HPET read under a hypervisor.
        let clock = SimClock::new(0xabcdef).read_cost(600_000, 800_000);
        let tsc = FakeTsc::new(&clock, TSC_HZ);
        let h = hpet(&clock, true);
        let hz = h.capabilities().frequency_hz();
        let cal = calibrate(&tsc, &h, hz, &CalibrationConfig::for_reference(hz)).unwrap();
        let err_ppm = mul_div(cal.hz.abs_diff(TSC_HZ), 1_000_000, TSC_HZ);
        assert!(err_ppm < 50, "{cal:?} off by {err_ppm} ppm");
        assert_eq!(cal.taken, 6);
        assert!(cal.used > 3);
    }

    #[test]
    fn rejects_measurements_hit_by_long_stalls() {
        // One read in 40 stalls for 300 us (an SMI or a descheduled vCPU).
        let clock = SimClock::new(42).read_cost(500_000, 200_000).stalls(40, 300_000_000);
        let tsc = FakeTsc::new(&clock, TSC_HZ);
        let h = hpet(&clock, false);
        let hz = h.capabilities().frequency_hz();
        let config = CalibrationConfig { samples: 16, ..CalibrationConfig::for_reference(hz) };
        let cal = calibrate(&tsc, &h, hz, &config).unwrap();
        let err_ppm = mul_div(cal.hz.abs_diff(TSC_HZ), 1_000_000, TSC_HZ);
        assert!(err_ppm < 100, "{cal:?} off by {err_ppm} ppm");
        assert!(cal.spread_ppm <= 1000);
    }

    #[test]
    fn calibration_across_a_32_bit_reference_wrap() {
        let clock = SimClock::new(7).read_cost(100_000, 50_000);
        let tsc = FakeTsc::new(&clock, TSC_HZ);
        let sim = SimHpet::new(&clock, false);
        let h = crate::Hpet::new(&sim).unwrap();
        h.set_counter(0xffff_0000).unwrap();
        h.enable();
        let hz = h.capabilities().frequency_hz();
        let cal = calibrate(&tsc, &h, hz, &CalibrationConfig::for_reference(hz)).unwrap();
        assert!(h.counter() < 0xffff_0000, "the counter wrapped during calibration");
        assert!(mul_div(cal.hz.abs_diff(TSC_HZ), 1_000_000, TSC_HZ) < 50);
    }

    #[test]
    fn calibration_errors() {
        let clock = SimClock::new(1).read_cost(1000, 0);
        let tsc = FakeTsc::new(&clock, TSC_HZ);
        let good = CalibrationConfig::for_reference(1_000_000);
        let stuck = Stuck(5);
        let small = CalibrationConfig { max_spins: 1000, ..good };
        assert_eq!(calibrate(&tsc, &stuck, 1_000_000, &small), Err(CalibrationError::ReferenceStuck));
        assert_eq!(calibrate(&stuck, &tsc, TSC_HZ, &good), Err(CalibrationError::TargetStuck));
        assert_eq!(calibrate(&tsc, &tsc, 0, &good), Err(CalibrationError::InvalidConfig));
        assert_eq!(
            calibrate(&tsc, &tsc, 1, &CalibrationConfig { samples: 0, ..good }),
            Err(CalibrationError::InvalidConfig)
        );
        assert_eq!(
            calibrate(&tsc, &tsc, 1, &CalibrationConfig { samples: 33, ..good }),
            Err(CalibrationError::InvalidConfig)
        );
        assert_eq!(
            calibrate(&tsc, &tsc, 1, &CalibrationConfig { window: 0, ..good }),
            Err(CalibrationError::InvalidConfig)
        );
        let h = hpet(&clock, false);
        let too_long = CalibrationConfig { window: 1 << 31, ..good };
        assert_eq!(calibrate(&tsc, &h, 1, &too_long), Err(CalibrationError::InvalidConfig));
        assert_eq!(CalibrationConfig::for_reference(0).window, 1);
    }

    #[test]
    fn inconsistent_measurements_are_refused() {
        // A TSC whose rate jumps by up to 20% every 3 ms, like one that
        // follows the core clock: the measurements do not agree.
        let clock = SimClock::new(9).read_cost(500_000, 0);
        let tsc = FakeTsc::new(&clock, TSC_HZ).wobble(3_000_000_000, TSC_HZ / 5);
        let h = hpet(&clock, true);
        let hz = h.capabilities().frequency_hz();
        assert_eq!(calibrate(&tsc, &h, hz, &CalibrationConfig::for_reference(hz)), Err(CalibrationError::Inconsistent));
    }

    #[test]
    fn measurements_with_wide_brackets_are_dropped() {
        // Every read stalls for 50 us: no bracket is tight enough.
        let clock = SimClock::new(9).read_cost(500_000, 0).stalls(1, 50_000_000);
        let tsc = FakeTsc::new(&clock, TSC_HZ);
        let h = hpet(&clock, true);
        let hz = h.capabilities().frequency_hz();
        assert_eq!(calibrate(&tsc, &h, hz, &CalibrationConfig::for_reference(hz)), Err(CalibrationError::Inconsistent));
    }

    #[test]
    fn clock_prefers_an_invariant_tsc() {
        let clock = SimClock::new(3).read_cost(20_000, 10_000);
        let h = hpet(&clock, true);
        let hz = h.capabilities().frequency_hz();
        let c = MonotonicClock::new(FakeTsc::new(&clock, TSC_HZ), &h, hz, Some(TSC_HZ), true).unwrap();
        assert_eq!(c.source(), ClockSource::Tsc);
        assert_eq!(c.frequency(), TSC_HZ);
        let start_ps = clock.now_ps();
        let t0 = c.now_ns();
        clock.advance_ps(1_000_000_000_000); // 1 s
        let t1 = c.now_ns();
        let real = (clock.now_ps() - start_ps) / 1000;
        assert!(t1 - t0 <= real && real - (t1 - t0) < 1000, "{} vs {real}", t1 - t0);
    }

    #[test]
    fn clock_falls_back_to_the_reference() {
        let clock = SimClock::new(3).read_cost(20_000, 0);
        let h = hpet(&clock, true);
        let hz = h.capabilities().frequency_hz();
        // Calibrated but not invariant.
        let c = MonotonicClock::new(FakeTsc::new(&clock, TSC_HZ), &h, hz, Some(TSC_HZ), false).unwrap();
        assert_eq!(c.source(), ClockSource::Reference);
        // Invariant but not calibrated.
        let c = MonotonicClock::new(FakeTsc::new(&clock, TSC_HZ), &h, hz, None, true).unwrap();
        assert_eq!(c.source(), ClockSource::Reference);
        let t0 = c.now_ns();
        clock.advance_ps(2_000_000_000_000);
        let elapsed = c.now_ns() - t0;
        assert!(elapsed.abs_diff(2_000_000_000) < 1000, "{elapsed}");
        // Nothing usable at all.
        let none = MonotonicClock::new(FakeTsc::new(&clock, TSC_HZ), &h, 0, None, true);
        assert_eq!(none.err(), Some(ClockError::NoUsableSource));
    }

    #[test]
    fn clock_stays_monotonic_across_a_fallback_and_a_32_bit_wrap() {
        let clock = SimClock::new(11).read_cost(30_000, 30_000);
        let sim = SimHpet::new(&clock, false);
        let h = crate::Hpet::new(&sim).unwrap();
        h.set_counter(0xff00_0000).unwrap();
        h.enable();
        let hz = h.capabilities().frequency_hz();
        let mut c = MonotonicClock::new(FakeTsc::new(&clock, TSC_HZ), &h, hz, Some(TSC_HZ), true).unwrap();
        let mut last = 0;
        for _ in 0..1000 {
            let t = c.now_ns();
            assert!(t >= last);
            last = t;
        }
        assert!(c.fall_back_to_reference());
        assert_eq!(c.source(), ClockSource::Reference);
        assert!(c.fall_back_to_reference());
        let start = c.now_ns();
        assert!(start >= last);
        // 20 s at 14.3 MHz takes the 32-bit counter through its wrap.
        for _ in 0..20 {
            clock.advance_ps(1_000_000_000_000);
            let t = c.now_ns();
            assert!(t > last);
            last = t;
        }
        assert!(h.counter() < 0xff00_0000);
        assert!((last - start).abs_diff(20_000_000_000) < 10_000, "{}", last - start);
    }

    #[test]
    fn tsc_conversion_reproduces_the_clock() {
        let clock = SimClock::new(5).read_cost(20_000, 0);
        let h = hpet(&clock, true);
        let hz = h.capabilities().frequency_hz();
        let mut c = MonotonicClock::new(FakeTsc::new(&clock, TSC_HZ), &h, hz, Some(TSC_HZ), true).unwrap();
        let conv = c.tsc_conversion().expect("the clock reads the TSC");
        for _ in 0..50 {
            clock.advance_ps(37_000_000_000);
            let tsc = c.tsc().read();
            let ns = c.now_ns();
            // What code outside the kernel computes from a TSC reading it
            // took just before the clock's own.
            let outside = conv
                .offset_ns
                .saturating_add(Scale { mult: conv.mult, shift: conv.shift }.apply(tsc.wrapping_sub(conv.origin)));
            assert!(outside <= ns && ns - outside < 100, "{outside} vs {ns}");
        }
        assert!(c.fall_back_to_reference());
        assert_eq!(c.tsc_conversion(), None, "the reference counter is not for user mode");
    }

    #[test]
    fn fall_back_without_a_reference_fails() {
        let clock = SimClock::new(3);
        let mut c =
            MonotonicClock::new(FakeTsc::new(&clock, TSC_HZ), FakeTsc::new(&clock, 1), 0, Some(TSC_HZ), true).unwrap();
        assert!(!c.fall_back_to_reference());
        assert_eq!(c.source(), ClockSource::Tsc);
        let _ = (c.tsc(), c.reference());
    }
}
