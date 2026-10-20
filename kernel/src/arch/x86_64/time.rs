//! Time on x86, through `poly-irq`: a reference timer (the HPET, or the ACPI
//! PM timer on machines without one), the TSC calibrated against it, the
//! monotonic clock built from the two, and the local APIC timer, calibrated
//! against the same reference and run periodically as the scheduler tick.
//!
//! The clock reads the TSC only when CPUID reports it invariant and it
//! calibrated cleanly (ADR 0013); otherwise it reads the reference timer,
//! which is slower to read but runs at a fixed rate. QEMU's default CPU model
//! does not report an invariant TSC, so under QEMU the clock is the HPET.
//!
//! The tick is 1 kHz ([`TICK_HZ`]): fine enough that `thread::sleep` wakes
//! within a millisecond of its deadline and a 10-tick time slice is 10 ms,
//! cheap enough (one interrupt per millisecond) to run unconditionally until
//! the kernel goes tickless. The local APIC timer keeps counting in C1
//! (`hlt`), which is the only idle state the kernel uses; deeper C-states
//! would need ARAT (`CpuFeatures::arat`) or the TSC-deadline mode.

use super::apic::Apic;
use super::cpu;
use super::irq::TIMER_VECTOR;
use super::mmio::MmioRegion;
use super::port;
use crate::acpi::Platform;
use crate::kprintln;
use crate::sync::SetOnce;
use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};
use poly_irq::lapic::{TimerMode, timer_count_for};
use poly_irq::time::{ClockSource, NANOS_PER_SEC, calibrate, mask};
use poly_irq::{Calibration, CalibrationConfig, CalibrationError, Counter, CpuFeatures, Divide, Hpet, MonotonicClock};

/// Scheduler ticks per second.
pub const TICK_HZ: u64 = 1000;
pub const NANOS_PER_TICK: u64 = NANOS_PER_SEC / TICK_HZ;

/// The PM timer's fixed rate, 3.579545 MHz (ACPI 6.5 §4.8.3.3, "Power
/// Management Timer").
pub const PM_TIMER_HZ: u64 = 3_579_545;

/// The time-stamp counter.
#[derive(Clone, Copy, Debug)]
pub struct Tsc;

impl Counter for Tsc {
    fn read(&self) -> u64 {
        cpu::rdtsc()
    }
}

/// The ACPI PM timer: a free-running 24- or 32-bit counter in an I/O port.
#[derive(Clone, Copy, Debug)]
pub struct PmTimer {
    port: u16,
    bits: u32,
}

impl Counter for PmTimer {
    fn read(&self) -> u64 {
        // SAFETY: the FADT names this port as the PM timer's counter, a
        // read-only register whose reads have no side effects.
        u64::from(unsafe { port::inl(self.port) }) & mask(self.bits)
    }

    fn bits(&self) -> u32 {
        self.bits
    }
}

/// The timer everything else is measured against.
#[derive(Debug)]
pub enum Reference {
    Hpet(Hpet<MmioRegion>),
    PmTimer(PmTimer),
}

impl Reference {
    pub fn name(&self) -> &'static str {
        match self {
            Reference::Hpet(_) => "HPET",
            Reference::PmTimer(_) => "ACPI PM timer",
        }
    }

    pub fn hz(&self) -> u64 {
        match self {
            Reference::Hpet(h) => h.capabilities().frequency_hz(),
            Reference::PmTimer(_) => PM_TIMER_HZ,
        }
    }
}

impl Counter for Reference {
    fn read(&self) -> u64 {
        match self {
            Reference::Hpet(h) => h.counter(),
            Reference::PmTimer(p) => p.read(),
        }
    }

    fn bits(&self) -> u32 {
        match self {
            Reference::Hpet(h) => h.bits(),
            Reference::PmTimer(p) => p.bits(),
        }
    }
}

type Clock = MonotonicClock<Tsc, Reference>;

static CLOCK: SetOnce<Clock> = SetOnce::new();
static TICKS: AtomicU64 = AtomicU64::new(0);

/// What [`init`] measured, for tests.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub tsc_hz: Option<u64>,
    pub reference_hz: u64,
    /// Local APIC timer rate (after its divider) and the initial count that
    /// makes its period one tick.
    pub apic_timer_hz: u64,
    pub apic_timer_count: u32,
}

static INFO: SetOnce<Info> = SetOnce::new();

/// Nanoseconds since the clock was set up during boot; 0 before that.
/// Never goes backwards, also across threads (and, later, CPUs).
pub fn now_ns() -> u64 {
    CLOCK.get().map_or(0, |c| c.now_ns())
}

/// Timer interrupts so far.
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub fn info() -> Option<Info> {
    INFO.get().copied()
}

/// The reference timer, once [`init`] ran.
pub fn reference() -> Option<&'static Reference> {
    CLOCK.get().map(|c| c.reference())
}

/// While the clock reads an invariant TSC: how it converts TSC readings
/// to its nanoseconds (for the vDSO). `None` on the reference timer.
pub fn tsc_conversion() -> Option<poly_irq::time::TscConversion> {
    CLOCK.get()?.tsc_conversion()
}

/// The timer interrupt, before its EOI.
pub(super) fn on_tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
    // A narrow reference counter (32-bit HPET, 24-bit PM timer) must be
    // read at least once per wrap for the clock to extend it; reading it
    // every tick guarantees that while interrupts flow.
    if let Some(c) = CLOCK.get().filter(|c| c.source() == ClockSource::Reference) {
        c.now_ns();
    }
}

/// Find and start the reference timer, calibrate the TSC and the local APIC
/// timer against it, start the clock and the periodic tick on
/// [`TIMER_VECTOR`]. Run with interrupts disabled: calibration measures
/// short intervals and must not be interrupted.
pub(super) fn init(platform: Option<&Platform>, features: &CpuFeatures, apic: &Apic) -> Result<(), &'static str> {
    let reference = reference_timer(platform)?;
    let ref_hz = reference.hz();
    let name = reference.name();

    let tsc = if features.tsc {
        match calibrate_with_retries(ref_hz, |cfg| calibrate(&Tsc, &reference, ref_hz, cfg)) {
            Ok(c) => {
                kprintln!(
                    "time: TSC {}, calibrated against the {name} ({} of {} measurements, spread {} ppm), invariant: {}",
                    Mhz(c.hz),
                    c.used,
                    c.taken,
                    c.spread_ppm,
                    if features.invariant_tsc { "yes" } else { "no" }
                );
                Some(c.hz)
            }
            Err(e) => {
                kprintln!("time: TSC calibration against the {name} failed: {e:?}");
                None
            }
        }
    } else {
        None
    };

    // The APIC timer counts down from u32::MAX during calibration; divide
    // its input clock until that lasts the whole measurement.
    let mut apic_timer = Err(CalibrationError::TargetStuck);
    let mut divide = Divide::By16;
    for d in [Divide::By16, Divide::By128] {
        divide = d;
        apic_timer = calibrate_with_retries(ref_hz, |cfg| apic.calibrate_timer(&reference, ref_hz, d, cfg));
        if apic_timer != Err(CalibrationError::TargetStuck) {
            break;
        }
    }
    let apic_timer = apic_timer.map_err(|e| {
        kprintln!("time: APIC timer calibration failed: {e:?}");
        "the local APIC timer could not be calibrated"
    })?;

    let clock = MonotonicClock::new(Tsc, reference, ref_hz, tsc, features.invariant_tsc)
        .map_err(|_| "no usable clock source")?;
    let tsc_clock = clock.source() == ClockSource::Tsc;
    kprintln!(
        "time: clock source {}{}",
        if tsc_clock { "TSC" } else { name },
        if tsc_clock || !features.tsc {
            ""
        } else if features.invariant_tsc {
            " (TSC calibration failed)"
        } else {
            " (TSC not invariant)"
        }
    );
    CLOCK.set(clock).map_err(|_| "clock initialised twice")?;

    let count = timer_count_for(apic_timer.hz, NANOS_PER_TICK).ok_or("APIC timer too fast or slow for the tick")?;
    apic.start_timer(TIMER_VECTOR, TimerMode::Periodic, divide, count).map_err(|_| "could not start the APIC timer")?;
    kprintln!(
        "time: APIC timer {} (input clock / {}), periodic tick {TICK_HZ} Hz on vector {TIMER_VECTOR:#x}",
        Mhz(apic_timer.hz),
        divide.divisor()
    );
    let info = Info { tsc_hz: tsc, reference_hz: ref_hz, apic_timer_hz: apic_timer.hz, apic_timer_count: count };
    INFO.set(info).map_err(|_| "time initialised twice")?;
    Ok(())
}

/// The HPET if the firmware has one that works, else the PM timer.
fn reference_timer(platform: Option<&Platform>) -> Result<Reference, &'static str> {
    if let Some(phys) = platform.and_then(|p| p.hpet) {
        // The register block is 1 KiB (IA-PC HPET 1.0a §2.3.1).
        match MmioRegion::map(phys, 0x400).map(Hpet::new) {
            Some(Ok(hpet)) => {
                // Firmware may have left timers running, or legacy
                // replacement routing on (which steals IRQ 0 and 8).
                hpet.disable();
                hpet.stop_all_timers();
                let _ = hpet.set_legacy_replacement(false);
                hpet.enable();
                let caps = hpet.capabilities();
                kprintln!(
                    "time: HPET at {phys:#x}: {}, {}-bit counter, {} timers",
                    Mhz(caps.frequency_hz()),
                    if caps.counter_64bit() { 64 } else { 32 },
                    caps.timer_count()
                );
                return Ok(Reference::Hpet(hpet));
            }
            Some(Err(e)) => kprintln!("time: HPET at {phys:#x} unusable: {e:?}"),
            None => kprintln!("time: could not map the HPET at {phys:#x}"),
        }
    }
    let (port, bits) = platform.and_then(|p| p.pm_timer).ok_or("no HPET and no ACPI PM timer to measure time with")?;
    kprintln!("time: ACPI PM timer at port {port:#x}, {bits}-bit");
    Ok(Reference::PmTimer(PmTimer { port, bits: bits.into() }))
}

/// Calibrate with the standard settings; if the measurements disagree
/// (an emulator or a busy host can stall one), try again, then once more
/// with a looser tolerance.
fn calibrate_with_retries(
    reference_hz: u64,
    mut measure: impl FnMut(&CalibrationConfig) -> Result<Calibration, CalibrationError>,
) -> Result<Calibration, CalibrationError> {
    let strict = CalibrationConfig::for_reference(reference_hz);
    let loose = CalibrationConfig { tolerance_ppm: 5_000, ..strict };
    let mut result = Err(CalibrationError::Inconsistent);
    for config in [strict, strict, loose] {
        result = measure(&config);
        if result != Err(CalibrationError::Inconsistent) {
            break;
        }
    }
    result
}

/// A frequency for the boot log.
struct Mhz(u64);

impl fmt::Display for Mhz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:03} MHz", self.0 / 1_000_000, self.0 / 1_000 % 1_000)
    }
}

// --- Tests ------------------------------------------------------------------

use super::{apic, irq};
use poly_irq::hpet::Route;
use poly_irq::{Polarity, Trigger};

static HPET_INTERRUPTS: AtomicU64 = AtomicU64::new(0);

fn count_hpet_interrupt(_: usize) {
    HPET_INTERRUPTS.fetch_add(1, Ordering::Relaxed);
}

crate::kernel_test! {
    /// An HPET comparator routed through the I/O APIC to an allocated
    /// vector: the whole external interrupt path, end to end.
    fn hpet_interrupt_arrives_through_the_io_apic() -> TestResult {
        let Some(Reference::Hpet(hpet)) = reference() else {
            crate::kprintln!("time: no HPET, skipping the I/O APIC delivery check");
            return Ok(());
        };
        // An unused, non-ISA I/O APIC input (GSI 16 and up are PCI) that
        // some comparator can drive. GSI n is input n of the first I/O APIC,
        // which is what the HPET's route field numbers.
        let pick = (0..hpet.capabilities().timer_count()).rev().find_map(|n| {
            let caps = hpet.timer(n).ok()?;
            let input = (16..32u8).rev().find(|&i| {
                caps.can_route_to(i) && apic::gsi_entry(i.into()).is_some_and(|e| e.is_masked())
            })?;
            Some((n, input))
        });
        let Some((timer, input)) = pick else {
            crate::kprintln!("time: no HPET comparator can reach a free I/O APIC input, skipping");
            return Ok(());
        };
        let gsi = u32::from(input);
        HPET_INTERRUPTS.store(0, Ordering::Relaxed);
        let vector = irq::route_gsi(gsi, Polarity::ActiveHigh, Trigger::Edge, count_hpet_interrupt, 0)
            .map_err(|_| "could not route the HPET's GSI")?;
        let result = (|| {
            hpet.configure_timer(timer, Route::IoApic(input), Trigger::Edge).map_err(|_| "HPET routing refused")?;
            let deadline = hpet.counter().wrapping_add(hpet.capabilities().frequency_hz() / 1000);
            hpet.arm_one_shot(timer, deadline).map_err(|_| "HPET comparator refused")?;
            let start = now_ns();
            while HPET_INTERRUPTS.load(Ordering::Relaxed) == 0 {
                if now_ns() - start > NANOS_PER_SEC {
                    return Err("the HPET interrupt did not arrive through the I/O APIC");
                }
                core::hint::spin_loop();
            }
            crate::kprintln!("time: HPET timer {timer} -> I/O APIC GSI {gsi} -> vector {vector:#x}: delivered");
            Ok(())
        })();
        let _ = hpet.stop_timer(timer);
        irq::unroute_gsi(gsi, vector).map_err(|_| "could not mask the HPET's GSI again")?;
        result
    }
}

/// A clock seen as a counter of nanoseconds, so it can be measured against
/// the reference like any other counter.
struct Nanoseconds<'a, T: Counter, R: Counter>(&'a MonotonicClock<T, R>);

impl<T: Counter, R: Counter> Counter for Nanoseconds<'_, T, R> {
    fn read(&self) -> u64 {
        self.0.now_ns()
    }
}

/// Measure `counter` against the reference the way boot calibration does
/// (bracketed reads, outliers dropped), so a host that deschedules the
/// emulator for a moment does not fail the test.
fn measure_against_reference(counter: &impl Counter) -> Result<u64, &'static str> {
    let info = info().ok_or("time not initialised")?;
    let reference = reference().ok_or("no reference timer")?;
    let _irq = cpu::InterruptsOff::new();
    calibrate_with_retries(info.reference_hz, |cfg| calibrate(counter, reference, info.reference_hz, cfg))
        .map(|c| c.hz)
        .map_err(|_| "measurements against the reference disagree")
}

crate::kernel_test! {
    fn tsc_runs_at_its_calibrated_rate() -> TestResult {
        let Some(tsc_hz) = info().ok_or("time not initialised")?.tsc_hz else { return Ok(()) };
        // Well after boot, and with interrupts having run, the TSC still
        // runs at the rate calibration found (under emulation both follow
        // the host's clock).
        let hz = measure_against_reference(&Tsc)?;
        if hz.abs_diff(tsc_hz) > tsc_hz / 100 {
            crate::kprintln!("time: TSC now runs at {hz} Hz, calibrated {tsc_hz} Hz");
            return Err("the TSC rate differs from its calibration");
        }
        Ok(())
    }
}

crate::kernel_test! {
    /// The clock's TSC path, which QEMU's CPU models (no invariant TSC) do
    /// not use at boot: a second clock that reads the TSC must count
    /// nanoseconds at the reference's rate, and never go backwards.
    fn a_tsc_clock_agrees_with_the_reference() -> TestResult {
        let info = info().ok_or("time not initialised")?;
        let Some(tsc_hz) = info.tsc_hz else { return Ok(()) };
        let reference = reference().ok_or("no reference timer")?;
        let clock = MonotonicClock::new(Tsc, reference, info.reference_hz, Some(tsc_hz), true)
            .map_err(|_| "no TSC clock")?;
        if clock.source() != ClockSource::Tsc {
            return Err("a clock with an invariant, calibrated TSC does not read the TSC");
        }
        let mut last = 0;
        for _ in 0..100_000 {
            let t = clock.now_ns();
            if t < last {
                return Err("the TSC clock went backwards");
            }
            last = t;
        }
        let hz = measure_against_reference(&Nanoseconds(&clock))?;
        if hz.abs_diff(NANOS_PER_SEC) > NANOS_PER_SEC / 100 {
            crate::kprintln!("time: the TSC clock counts {hz} ns per {} second", reference.name());
            return Err("the TSC clock and the reference disagree");
        }
        Ok(())
    }
}

crate::kernel_test! {
    /// The fallback reference for machines without an HPET: the TSC
    /// measured against the ACPI PM timer matches the boot calibration.
    fn pm_timer_agrees_with_the_hpet() -> TestResult {
        let (Some(info), Some(Reference::Hpet(_))) = (info(), reference()) else { return Ok(()) };
        let (Some(tsc_hz), Some((port, bits))) = (info.tsc_hz, crate::acpi::platform().and_then(|p| p.pm_timer)) else {
            return Ok(());
        };
        let pm = PmTimer { port, bits: bits.into() };
        let _irq = cpu::InterruptsOff::new();
        let c = calibrate_with_retries(PM_TIMER_HZ, |cfg| calibrate(&Tsc, &pm, PM_TIMER_HZ, cfg))
            .map_err(|_| "the TSC could not be calibrated against the PM timer")?;
        if c.hz.abs_diff(tsc_hz) > tsc_hz / 200 {
            crate::kprintln!("time: TSC {} against the PM timer, {} against the HPET", Mhz(c.hz), Mhz(tsc_hz));
            return Err("the PM timer and the HPET disagree");
        }
        Ok(())
    }
}

crate::kernel_test! {
    /// The tick's period, from the timer itself rather than from counting
    /// interrupts (which a busy host can merge): the local APIC timer is
    /// periodic on the tick vector, reloads with the calibrated count, and
    /// counts down at the calibrated rate.
    fn apic_timer_period_is_one_tick() -> TestResult {
        use poly_irq::time::delta;
        let info = info().ok_or("time not initialised")?;
        let apic = apic::local_apic().ok_or("no local APIC")?;
        let lvt = apic.lvt(poly_irq::LvtReg::Timer).map_err(|_| "no timer LVT entry")?;
        if lvt.vector() != TIMER_VECTOR || lvt.is_masked() || lvt.timer_mode() != Some(TimerMode::Periodic) {
            return Err("the APIC timer is not periodic on the tick vector");
        }
        let initial = apic.timer_initial_count();
        if initial != info.apic_timer_count {
            return Err("the APIC timer's initial count is not the calibrated one");
        }
        let period_ns = poly_irq::time::mul_div(initial.into(), NANOS_PER_SEC, info.apic_timer_hz);
        if period_ns.abs_diff(NANOS_PER_TICK) > NANOS_PER_TICK / 100 {
            return Err("the APIC timer's period is not one tick");
        }
        // Count-down rate over half a period, against the reference. Both
        // reads of a pair are bracketed by reference reads; pairs the host
        // stretched (or that span a reload) are retried.
        let reference = reference().ok_or("no reference timer")?;
        let (hz, bits) = (info.reference_hz, reference.bits());
        let read = || {
            let r0 = reference.read();
            let c = apic.timer_current_count();
            let r1 = reference.read();
            (r0.wrapping_add(delta(r0, r1, bits) / 2), c, delta(r0, r1, bits))
        };
        let half_period = hz / TICK_HZ / 2;
        for _ in 0..50 {
            let _irq = cpu::InterruptsOff::new();
            let (r0, c0, w0) = read();
            while delta(r0, reference.read(), bits) < half_period {
                core::hint::spin_loop();
            }
            let (r1, c1, w1) = read();
            let elapsed = delta(r0, r1, bits);
            // Brackets within 2% of the window, and less than a period apart.
            if (w0 + w1) * 50 > elapsed || elapsed >= 2 * half_period {
                continue;
            }
            // A countdown that reloads from `initial` at 0.
            let counted = (u64::from(c0) + u64::from(initial) - u64::from(c1)) % u64::from(initial);
            let rate = poly_irq::time::mul_div(counted, hz, elapsed);
            if rate.abs_diff(info.apic_timer_hz) > info.apic_timer_hz / 20 {
                crate::kprintln!("time: APIC timer counts at {rate} Hz, calibrated {} Hz", info.apic_timer_hz);
                return Err("the APIC timer does not count at its calibrated rate");
            }
            return Ok(());
        }
        Err("could not measure the APIC timer: every measurement was disturbed")
    }
}
