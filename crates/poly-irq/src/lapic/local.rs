//! The local APIC itself, its timer and the periodic-rate windows.

use super::*;

/// The local APIC of the CPU running this code.
#[derive(Debug)]
pub struct LocalApic<R> {
    regs: R,
    tsc_deadline: bool,
    icr_spins: u32,
}

impl<R: ApicRegisters> LocalApic<R> {
    /// Polls of the xAPIC delivery status bit before giving up on an IPI.
    pub const DEFAULT_ICR_SPINS: u32 = 1 << 20;

    /// SVR bit 8: APIC software enable.
    pub const SVR_ENABLE: u32 = 1 << 8;

    /// Wraps the registers. Changes nothing. `features` says whether the
    /// timer has TSC-deadline mode.
    pub fn new(regs: R, features: &CpuFeatures) -> LocalApic<R> {
        LocalApic { regs, tsc_deadline: features.tsc_deadline, icr_spins: Self::DEFAULT_ICR_SPINS }
    }

    pub fn registers(&self) -> &R {
        &self.regs
    }

    pub fn set_icr_spin_limit(&mut self, spins: u32) {
        self.icr_spins = spins;
    }

    pub fn is_x2apic(&self) -> bool {
        self.regs.is_x2apic()
    }

    /// The APIC ID: bits 31:24 of the ID register in xAPIC mode, all 32 bits
    /// in x2APIC mode (§11.4.6, §11.12.5.1).
    pub fn id(&self) -> u32 {
        let id = self.regs.read(reg::ID);
        if self.is_x2apic() { id } else { id >> 24 }
    }

    pub fn version(&self) -> Version {
        Version(self.regs.read(reg::VERSION))
    }

    /// Software-enables the APIC and sets the spurious interrupt vector
    /// (§11.9, figure 11-23). Other SVR bits (focus processor checking,
    /// EOI-broadcast suppression) are kept. On P6 and Pentium processors bits
    /// 3:0 of the vector are hardwired to 1, so 0xff is the portable choice.
    pub fn enable(&self, spurious_vector: u8) {
        let svr = self.regs.read(reg::SVR) & !0xff;
        self.regs.write(reg::SVR, svr | Self::SVR_ENABLE | spurious_vector as u32);
    }

    /// Software-disables the APIC; it masks every LVT entry (§11.4.7.2).
    pub fn disable(&self) {
        let svr = self.regs.read(reg::SVR);
        self.regs.write(reg::SVR, svr & !Self::SVR_ENABLE);
    }

    pub fn is_enabled(&self) -> bool {
        self.regs.read(reg::SVR) & Self::SVR_ENABLE != 0
    }

    pub fn spurious_vector(&self) -> u8 {
        self.regs.read(reg::SVR) as u8
    }

    /// Interrupts with a priority class (vector / 16) at or below
    /// `priority >> 4` are held back (§11.8.3.1).
    pub fn set_task_priority(&self, priority: u8) {
        self.regs.write(reg::TPR, priority as u32);
    }

    pub fn task_priority(&self) -> u8 {
        self.regs.read(reg::TPR) as u8
    }

    pub fn processor_priority(&self) -> u8 {
        self.regs.read(reg::PPR) as u8
    }

    /// Signals the end of the interrupt being serviced (§11.8.5). Not for
    /// the spurious vector, which is never in service. x2APIC mode requires
    /// the value 0.
    pub fn eoi(&self) {
        self.regs.write(reg::EOI, 0);
    }

    fn vector_bit(&self, base: u16, vector: u8) -> bool {
        let r = self.regs.read(base + 0x10 * (vector as u16 / 32));
        r & (1 << (vector % 32)) != 0
    }

    pub fn is_in_service(&self, vector: u8) -> bool {
        self.vector_bit(reg::ISR, vector)
    }

    pub fn is_requested(&self, vector: u8) -> bool {
        self.vector_bit(reg::IRR, vector)
    }

    pub fn lvt(&self, which: LvtReg) -> Result<Lvt, ApicError> {
        if !which.is_present(self.version()) {
            return Err(ApicError::NotPresent);
        }
        Ok(Lvt(self.regs.read(which.offset())))
    }

    /// Writes an LVT entry; its read-only bits are cleared first. An
    /// unmasked fixed entry needs a vector of 16 or more. While the APIC is
    /// software-disabled the hardware keeps every entry masked.
    pub fn set_lvt(&self, which: LvtReg, lvt: Lvt) -> Result<(), ApicError> {
        if !which.is_present(self.version()) {
            return Err(ApicError::NotPresent);
        }
        if !lvt.is_masked() && lvt.delivery() == Some(DeliveryMode::Fixed) && lvt.vector() < 16 {
            return Err(ApicError::InvalidVector);
        }
        self.regs.write(which.offset(), lvt.bits() & !Lvt::READ_ONLY);
        Ok(())
    }

    pub fn set_masked(&self, which: LvtReg, masked: bool) -> Result<(), ApicError> {
        let lvt = self.lvt(which)?;
        self.set_lvt(which, lvt.with_mask(masked))
    }

    /// Masks every LVT entry this APIC has.
    pub fn mask_all_lvts(&self) {
        let version = self.version();
        for which in LvtReg::ALL {
            if which.is_present(version) {
                let lvt = Lvt(self.regs.read(which.offset()));
                self.regs.write(which.offset(), lvt.with_mask(true).bits() & !Lvt::READ_ONLY);
            }
        }
    }

    /// Errors seen since the last call. The ESR is updated by a write and
    /// then read (§11.5.3); x2APIC mode requires the written value to be 0.
    pub fn error_status(&self) -> ErrorStatus {
        self.regs.write(reg::ESR, 0);
        ErrorStatus(self.regs.read(reg::ESR))
    }

    pub fn timer_divide(&self) -> Divide {
        Divide::from_bits(self.regs.read(reg::TIMER_DIVIDE))
    }

    pub fn set_timer_divide(&self, divide: Divide) {
        let keep = self.regs.read(reg::TIMER_DIVIDE) & !0b1011;
        self.regs.write(reg::TIMER_DIVIDE, keep | divide.bits());
    }

    /// Starts the timer counting down from `initial_count` at the bus /
    /// crystal clock divided by `divide`, interrupting at `vector` when it
    /// reaches 0, once or (periodic) every time (§11.5.4). The order follows
    /// the SDM: divider, LVT, then the initial count, which starts it.
    pub fn start_timer(
        &self,
        vector: u8,
        mode: TimerMode,
        divide: Divide,
        initial_count: u32,
    ) -> Result<(), ApicError> {
        if mode == TimerMode::TscDeadline {
            return Err(ApicError::InvalidTimerMode);
        }
        self.set_timer_divide(divide);
        self.set_lvt(LvtReg::Timer, Lvt::new(vector).with_timer_mode(mode))?;
        self.regs.write(reg::TIMER_INITIAL, initial_count);
        Ok(())
    }

    /// Stops the timer: masks it and writes an initial count of 0. In
    /// TSC-deadline mode, also write a deadline of 0.
    pub fn stop_timer(&self) {
        let lvt = Lvt(self.regs.read(reg::LVT_TIMER));
        self.regs.write(reg::LVT_TIMER, lvt.with_mask(true).bits() & !Lvt::READ_ONLY);
        self.regs.write(reg::TIMER_INITIAL, 0);
    }

    pub fn timer_initial_count(&self) -> u32 {
        self.regs.read(reg::TIMER_INITIAL)
    }

    pub fn timer_current_count(&self) -> u32 {
        self.regs.read(reg::TIMER_CURRENT)
    }

    /// Puts the timer in TSC-deadline mode at `vector` (§11.5.4.1), where it
    /// interrupts when the TSC reaches the value in `IA32_TSC_DEADLINE`.
    pub fn enable_tsc_deadline(&self, vector: u8) -> Result<(), ApicError> {
        if !self.tsc_deadline {
            return Err(ApicError::Unsupported);
        }
        self.set_lvt(LvtReg::Timer, Lvt::new(vector).with_timer_mode(TimerMode::TscDeadline))
    }

    /// Arms (or with 0 disarms) the TSC deadline. The MSR exists in both
    /// modes. The fence orders an earlier memory-mapped LVT write before the
    /// `wrmsr`, which §11.5.4.1 requires in xAPIC mode.
    pub fn set_tsc_deadline<S: Msr>(&self, msr: &S, deadline: u64) -> Result<(), ApicError> {
        if !self.tsc_deadline {
            return Err(ApicError::Unsupported);
        }
        fence(Ordering::SeqCst);
        msr.write(IA32_TSC_DEADLINE, deadline);
        Ok(())
    }

    fn wait_icr_idle(&self) -> Result<(), ApicError> {
        for _ in 0..self.icr_spins {
            if self.regs.read_icr_low() & Ipi::DELIVERY_STATUS == 0 {
                return Ok(());
            }
            spin_loop();
        }
        Err(ApicError::Timeout)
    }

    /// Sends an IPI (§11.6.1). In xAPIC mode, waits (bounded) for the
    /// previous IPI to leave and for this one to be accepted, using the
    /// delivery status bit; x2APIC mode has no such bit.
    ///
    /// INIT-SIPI-SIPI timing (10 ms after INIT, 200 us between start-ups)
    /// is the caller's job.
    pub fn send_ipi(&self, ipi: &Ipi) -> Result<(), ApicError> {
        match ipi.delivery {
            DeliveryMode::ExtInt => return Err(ApicError::InvalidIpi),
            DeliveryMode::Fixed | DeliveryMode::LowestPriority if ipi.vector < 16 => {
                return Err(ApicError::InvalidVector);
            }
            _ => {}
        }
        if ipi.destination == Destination::ToSelf && ipi.delivery != DeliveryMode::Fixed {
            return Err(ApicError::InvalidIpi);
        }
        let (dest, low) = ipi.encode();
        if self.is_x2apic() {
            self.regs.write_icr(dest, low);
            return Ok(());
        }
        if dest > 0xff {
            return Err(ApicError::InvalidDestination);
        }
        self.wait_icr_idle()?;
        self.regs.write_icr(dest, low);
        self.wait_icr_idle()
    }

    /// Measures the timer's rate at `divide` against `reference` (counting at
    /// `reference_hz`) and leaves the timer stopped. Run with interrupts
    /// disabled. The countdown starts at `u32::MAX`; if it reaches 0 before
    /// the measurements end, the result is [`CalibrationError::TargetStuck`]
    /// and a larger divider is needed.
    pub fn calibrate_timer<C: Counter>(
        &self,
        reference: &C,
        reference_hz: u64,
        divide: Divide,
        config: &CalibrationConfig,
    ) -> Result<Calibration, CalibrationError> {
        self.calibrate_timer_report(reference, reference_hz, divide, config).0
    }

    /// [`LocalApic::calibrate_timer`], and the measurements it took.
    ///
    /// The reference can be any counter of known rate: the HPET or PM
    /// timer, or a TSC already calibrated against one. Under a hypervisor
    /// the TSC is the better one, since reading it is not a VM exit.
    pub fn calibrate_timer_report<C: Counter>(
        &self,
        reference: &C,
        reference_hz: u64,
        divide: Divide,
        config: &CalibrationConfig,
    ) -> (Result<Calibration, CalibrationError>, CalibrationReport) {
        self.set_timer_divide(divide);
        let lvt = Lvt::masked().with_vector(0xff).with_timer_mode(TimerMode::OneShot);
        self.regs.write(reg::LVT_TIMER, lvt.bits());
        self.regs.write(reg::TIMER_INITIAL, u32::MAX);
        let (result, report) = calibrate_report(&ApicTimerCounter(self), reference, reference_hz, config);
        let expired = self.timer_current_count() == 0;
        self.stop_timer();
        (if expired { Err(CalibrationError::TargetStuck) } else { result }, report)
    }
}

/// The APIC timer counting down from `u32::MAX`, seen as a counter that
/// counts up.
#[derive(Debug)]
pub struct ApicTimerCounter<'a, R>(pub &'a LocalApic<R>);

impl<R: ApicRegisters> Counter for ApicTimerCounter<'_, R> {
    fn read(&self) -> u64 {
        !self.0.timer_current_count() as u64
    }

    fn bits(&self) -> u32 {
        32
    }
}

/// One way [`LocalApic::measure_periodic_rate`] measures: a window of
/// `window_permille` thousandths of the timer's period, kept only if the
/// reads bracketing the timer's two count reads took at most
/// `max_brackets_permille` thousandths of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeriodicWindow {
    pub window_permille: u64,
    pub max_brackets_permille: u64,
}

/// What [`LocalApic::measure_periodic_rate`] tries, in order: half a period
/// with brackets of at most 2% (a few microseconds at a 1 ms period), most
/// of a period with up to 5%, then four and a half periods with up to 5%
/// (225 us at 1 ms: a KVM guest on a busy or nested host can take 15 us
/// for one read of the count, which is an exit to the hypervisor).
pub const PERIODIC_WINDOWS: [PeriodicWindow; 3] = [
    PeriodicWindow { window_permille: 500, max_brackets_permille: 20 },
    PeriodicWindow { window_permille: 850, max_brackets_permille: 50 },
    PeriodicWindow { window_permille: 4500, max_brackets_permille: 50 },
];

/// What [`LocalApic::measure_periodic_rate`] measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeriodicRate {
    /// The timer's count-down rate.
    pub hz: u64,
    /// How far off it can be from the bracketing alone, in ppm: compare
    /// with a tolerance of this plus whatever the check allows.
    pub uncertainty_ppm: u64,
    /// The [`PERIODIC_WINDOWS`] entry that measured it, from 1.
    pub attempt: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeriodicRateError {
    /// The timer is not running periodically (initial count 0, or no
    /// expected rate to size the windows with).
    NotRunning,
    /// The reference did not advance by a window within its spin budget.
    ReferenceStuck,
    /// Every measurement's brackets were too wide; the narrowest pair of
    /// brackets seen, in reference ticks.
    Disturbed { narrowest: u64 },
}

impl<R: ApicRegisters> LocalApic<R> {
    /// The rate a running periodic timer counts down at, measured against
    /// `reference` (counting at `reference_hz`) with each of
    /// [`PERIODIC_WINDOWS`] in turn, `tries` times each, until one
    /// measurement's brackets are narrow enough. `guard` is called before
    /// each measurement and its value dropped after it (the kernel disables
    /// interrupts with it).
    ///
    /// Each read of the count is bracketed by two reads of the reference.
    /// The count reloads from the initial count at 0, so a window only shows
    /// the count modulo the period; the whole periods in it are those that
    /// bring the count closest to what `expected_hz` (the calibrated rate)
    /// predicts. Over a window shorter than half a period that changes
    /// nothing; over `n` periods it hides rate errors near multiples of
    /// `1 / n` (22% for the longest window), which a rate check of a few
    /// percent still tells apart from small errors.
    pub fn measure_periodic_rate<C: Counter, G>(
        &self,
        reference: &C,
        reference_hz: u64,
        expected_hz: u64,
        tries: usize,
        mut guard: impl FnMut() -> G,
    ) -> Result<PeriodicRate, PeriodicRateError> {
        use crate::time::delta;
        let initial = u64::from(self.timer_initial_count());
        if initial == 0 || expected_hz == 0 || reference_hz == 0 {
            return Err(PeriodicRateError::NotRunning);
        }
        let bits = reference.bits();
        let period = mul_div(initial, reference_hz, expected_hz);
        let read = || {
            let r0 = reference.read();
            let c = u64::from(self.timer_current_count());
            let r1 = reference.read();
            let width = delta(r0, r1, bits);
            (r0.wrapping_add(width / 2), c, width)
        };
        let mut narrowest = u64::MAX;
        for (i, w) in PERIODIC_WINDOWS.iter().enumerate() {
            let window = mul_div(period, w.window_permille, 1000).max(1);
            let max_spins = window.saturating_mul(64).saturating_add(100_000);
            for _ in 0..tries {
                let _guard = guard();
                let (r0, c0, w0) = read();
                let mut spins = 0u64;
                while delta(r0, reference.read(), bits) < window {
                    spins += 1;
                    if spins >= max_spins {
                        return Err(PeriodicRateError::ReferenceStuck);
                    }
                    spin_loop();
                }
                let (r1, c1, w1) = read();
                let elapsed = delta(r0, r1, bits);
                let brackets = w0 + w1;
                if elapsed == 0 || brackets.saturating_mul(1000) > elapsed.saturating_mul(w.max_brackets_permille) {
                    narrowest = narrowest.min(brackets);
                    continue;
                }
                // Counted down within the last period, then the whole
                // periods that bring it closest to the expected count.
                let phase = (c0 + initial - c1 % initial) % initial;
                let expected = mul_div(elapsed, expected_hz, reference_hz);
                let periods = (expected + initial / 2).saturating_sub(phase) / initial;
                let counted = phase + periods * initial;
                return Ok(PeriodicRate {
                    hz: mul_div(counted, reference_hz, elapsed),
                    uncertainty_ppm: mul_div(brackets, 1_000_000, elapsed),
                    attempt: i + 1,
                });
            }
        }
        Err(PeriodicRateError::Disturbed { narrowest })
    }
}

/// The initial count for a timer period of `period_ns` at `timer_hz`
/// (as measured by [`LocalApic::calibrate_timer`]). `None` if it rounds to
/// 0 or does not fit 32 bits.
pub const fn timer_count_for(timer_hz: u64, period_ns: u64) -> Option<u32> {
    match mul_div(period_ns, timer_hz, crate::time::NANOS_PER_SEC) {
        0 => None,
        n if n > u32::MAX as u64 => None,
        n => Some(n as u32),
    }
}
