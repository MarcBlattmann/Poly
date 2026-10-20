//! High Precision Event Timer (IA-PC HPET Specification 1.0a).
//!
//! One timer block is a 1 KiB register window holding a free-running main
//! counter and up to 32 comparators. The ACPI HPET table gives its address
//! (`poly_acpi::Hpet::base`). The driver uses only 32-bit accesses, which
//! every HPET supports (§2.3.2 "Register Access"), so it works over the
//! same [`Mmio`] as everything else; the 64-bit main counter is then read
//! high, low, high so a carry between the halves cannot tear the value.
//!
//! ```text
//! 0x000  General Capabilities and ID (RO)    §2.3.4
//! 0x010  General Configuration               §2.3.5
//! 0x020  General Interrupt Status (W1C)      §2.3.6
//! 0x0f0  Main Counter Value                  §2.3.7
//! 0x100 + 0x20 * n  Timer n Configuration and Capabilities  §2.3.8
//! 0x108 + 0x20 * n  Timer n Comparator Value               §2.3.9
//! 0x110 + 0x20 * n  Timer n FSB Interrupt Route            §2.3.10
//! ```

use crate::time::{Counter, FEMTOS_PER_SEC, Scale, delta, mask};
use crate::{Mmio, Trigger};

pub mod reg {
    pub const CAPABILITIES: usize = 0x000;
    pub const CONFIGURATION: usize = 0x010;
    pub const INTERRUPT_STATUS: usize = 0x020;
    pub const MAIN_COUNTER: usize = 0x0f0;

    pub const fn timer_config(n: u8) -> usize {
        0x100 + 0x20 * n as usize
    }

    pub const fn timer_comparator(n: u8) -> usize {
        0x108 + 0x20 * n as usize
    }

    pub const fn timer_fsb_route(n: u8) -> usize {
        0x110 + 0x20 * n as usize
    }
}

/// General Configuration register bits (§2.3.5).
pub mod config {
    /// ENABLE_CNF: the main counter runs and timers may interrupt.
    pub const ENABLE: u64 = 1 << 0;
    /// LEG_RT_CNF: timer 0 takes over IRQ0 / GSI 2 and timer 1 IRQ8 / GSI 8.
    pub const LEGACY_REPLACEMENT: u64 = 1 << 1;
}

/// Timer n Configuration and Capabilities bits (§2.3.8).
pub mod timer {
    /// Tn_INT_TYPE_CNF: level-triggered (otherwise edge).
    pub const LEVEL: u64 = 1 << 1;
    /// Tn_INT_ENB_CNF.
    pub const INT_ENABLE: u64 = 1 << 2;
    /// Tn_TYPE_CNF: periodic (otherwise one-shot).
    pub const PERIODIC: u64 = 1 << 3;
    /// Tn_PER_INT_CAP (RO): periodic mode is available.
    pub const PERIODIC_CAPABLE: u64 = 1 << 4;
    /// Tn_SIZE_CAP (RO): the comparator is 64 bits wide.
    pub const SIZE_64: u64 = 1 << 5;
    /// Tn_VAL_SET_CNF: the next comparator write sets the periodic
    /// accumulator directly. Clears itself.
    pub const VAL_SET: u64 = 1 << 6;
    /// Tn_32MODE_CNF: run a 64-bit timer as a 32-bit one.
    pub const MODE_32: u64 = 1 << 8;
    /// Tn_INT_ROUTE_CNF (bits 13:9): the I/O APIC input to signal.
    pub const ROUTE_SHIFT: u32 = 9;
    pub const ROUTE_MASK: u64 = 0x1f << ROUTE_SHIFT;
    /// Tn_FSB_EN_CNF: signal with an FSB (MSI-like) message instead.
    pub const FSB_ENABLE: u64 = 1 << 14;
    /// Tn_FSB_INT_DEL_CAP (RO).
    pub const FSB_CAPABLE: u64 = 1 << 15;
    /// Tn_INT_ROUTE_CAP (RO, bits 63:32): bit n set if I/O APIC input n is
    /// allowed.
    pub const ROUTE_CAP_SHIFT: u32 = 32;
}

/// The General Capabilities and ID register (§2.3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities(pub u64);

impl Capabilities {
    /// The largest period the specification allows: 100 ns.
    pub const MAX_PERIOD_FS: u32 = 100_000_000;

    pub const fn revision(self) -> u8 {
        self.0 as u8
    }

    /// NUM_TIM_CAP holds the index of the last timer.
    pub const fn timer_count(self) -> u8 {
        ((self.0 >> 8) & 0x1f) as u8 + 1
    }

    /// COUNT_SIZE_CAP.
    pub const fn counter_64bit(self) -> bool {
        self.0 & (1 << 13) != 0
    }

    /// LEG_RT_CAP.
    pub const fn legacy_replacement(self) -> bool {
        self.0 & (1 << 15) != 0
    }

    pub const fn vendor_id(self) -> u16 {
        (self.0 >> 16) as u16
    }

    /// COUNTER_CLK_PERIOD: main counter tick in femtoseconds.
    pub const fn period_fs(self) -> u32 {
        (self.0 >> 32) as u32
    }

    /// Main counter rate, rounded down (0 if the period is 0).
    pub const fn frequency_hz(self) -> u64 {
        match self.period_fs() {
            0 => 0,
            p => FEMTOS_PER_SEC / p as u64,
        }
    }

    /// Exact main-counter-ticks to nanoseconds conversion, from the period
    /// rather than the rounded frequency.
    pub const fn ticks_to_ns(self) -> Option<Scale> {
        Scale::new(1_000_000, self.period_fs() as u64)
    }
}

/// What one comparator can do, from its configuration register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerCaps {
    pub periodic: bool,
    /// The comparator is 64 bits wide.
    pub wide: bool,
    pub fsb: bool,
    /// Bit n set: the timer can be routed to I/O APIC input n.
    pub ioapic_routes: u32,
}

impl TimerCaps {
    pub const fn from_config(config: u64) -> TimerCaps {
        TimerCaps {
            periodic: config & timer::PERIODIC_CAPABLE != 0,
            wide: config & timer::SIZE_64 != 0,
            fsb: config & timer::FSB_CAPABLE != 0,
            ioapic_routes: (config >> timer::ROUTE_CAP_SHIFT) as u32,
        }
    }

    pub const fn can_route_to(self, input: u8) -> bool {
        input < 32 && self.ioapic_routes & (1 << input) != 0
    }
}

/// Where a comparator's interrupt goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// An I/O APIC input (a GSI relative to the first I/O APIC).
    IoApic(u8),
    /// A front-side-bus message: the timer writes `data` to `address`, like
    /// MSI (on x86 the address picks a local APIC and the data a vector).
    Fsb { address: u32, data: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HpetError {
    /// The capabilities register reads all ones: nothing is mapped there.
    NotPresent,
    /// COUNTER_CLK_PERIOD is 0 or above 100 ns (§2.3.4 requires both).
    BadPeriod(u32),
    NoSuchTimer,
    /// The block or timer lacks the capability asked for (periodic mode,
    /// FSB delivery, legacy replacement, or that I/O APIC input).
    NotCapable,
    /// The main counter may only be written while it is halted.
    CounterRunning,
    /// A periodic period of 0, or above what a 32-bit comparator holds.
    InvalidPeriod,
}

/// One HPET timer block.
#[derive(Debug)]
pub struct Hpet<M> {
    mmio: M,
    caps: Capabilities,
}

impl<M: Mmio> Hpet<M> {
    /// Reads and checks the capabilities. Does not change any state.
    pub fn new(mmio: M) -> Result<Hpet<M>, HpetError> {
        let caps = Capabilities(read64(&mmio, reg::CAPABILITIES));
        if caps.0 == u64::MAX {
            return Err(HpetError::NotPresent);
        }
        let period = caps.period_fs();
        if period == 0 || period > Capabilities::MAX_PERIOD_FS {
            return Err(HpetError::BadPeriod(period));
        }
        Ok(Hpet { mmio, caps })
    }

    pub fn capabilities(&self) -> Capabilities {
        self.caps
    }

    pub fn mmio(&self) -> &M {
        &self.mmio
    }

    fn configuration(&self) -> u64 {
        read64(&self.mmio, reg::CONFIGURATION)
    }

    fn set_configuration_bit(&self, bit: u64, on: bool) {
        // Both defined bits are in the low dword; the rest is reserved.
        let c = self.mmio.read32(reg::CONFIGURATION);
        let c = if on { c | bit as u32 } else { c & !(bit as u32) };
        self.mmio.write32(reg::CONFIGURATION, c);
    }

    pub fn is_enabled(&self) -> bool {
        self.configuration() & config::ENABLE != 0
    }

    /// Starts the main counter.
    pub fn enable(&self) {
        self.set_configuration_bit(config::ENABLE, true);
    }

    /// Halts the main counter; timers stop interrupting.
    pub fn disable(&self) {
        self.set_configuration_bit(config::ENABLE, false);
    }

    pub fn set_legacy_replacement(&self, on: bool) -> Result<(), HpetError> {
        if on && !self.caps.legacy_replacement() {
            return Err(HpetError::NotCapable);
        }
        self.set_configuration_bit(config::LEGACY_REPLACEMENT, on);
        Ok(())
    }

    /// The main counter. A 32-bit counter wraps at `u32::MAX`; see
    /// [`crate::time::Extender`].
    pub fn counter(&self) -> u64 {
        if !self.caps.counter_64bit() {
            return self.mmio.read32(reg::MAIN_COUNTER) as u64;
        }
        let hi = self.mmio.read32(reg::MAIN_COUNTER + 4);
        let lo = self.mmio.read32(reg::MAIN_COUNTER);
        let hi2 = self.mmio.read32(reg::MAIN_COUNTER + 4);
        // If the high half moved, the low half wrapped around the time it
        // was read; a fresh read is consistent with `hi2`.
        let lo = if hi == hi2 { lo } else { self.mmio.read32(reg::MAIN_COUNTER) };
        (hi2 as u64) << 32 | lo as u64
    }

    /// Sets the main counter. Only while it is halted (§2.3.7).
    pub fn set_counter(&self, value: u64) -> Result<(), HpetError> {
        if self.is_enabled() {
            return Err(HpetError::CounterRunning);
        }
        self.mmio.write32(reg::MAIN_COUNTER, value as u32);
        if self.caps.counter_64bit() {
            self.mmio.write32(reg::MAIN_COUNTER + 4, (value >> 32) as u32);
        }
        Ok(())
    }

    fn check(&self, n: u8) -> Result<(), HpetError> {
        if n < self.caps.timer_count() { Ok(()) } else { Err(HpetError::NoSuchTimer) }
    }

    fn timer_config(&self, n: u8) -> u64 {
        read64(&self.mmio, reg::timer_config(n))
    }

    /// Only the low dword holds writable bits; the high one is the
    /// read-only route capability.
    fn set_timer_config(&self, n: u8, value: u64) {
        self.mmio.write32(reg::timer_config(n), value as u32);
    }

    pub fn timer(&self, n: u8) -> Result<TimerCaps, HpetError> {
        self.check(n)?;
        Ok(TimerCaps::from_config(self.timer_config(n)))
    }

    /// Routes timer `n` and sets its trigger mode, leaving it one-shot with
    /// its interrupt disabled. FSB delivery is edge-only in practice, but
    /// the trigger bit is written either way.
    pub fn configure_timer(&self, n: u8, route: Route, trigger: Trigger) -> Result<(), HpetError> {
        let caps = self.timer(n)?;
        let mut c = self.timer_config(n)
            & !(timer::INT_ENABLE
                | timer::PERIODIC
                | timer::VAL_SET
                | timer::ROUTE_MASK
                | timer::FSB_ENABLE
                | timer::LEVEL);
        match route {
            Route::IoApic(input) => {
                if !caps.can_route_to(input) {
                    return Err(HpetError::NotCapable);
                }
                c |= (input as u64) << timer::ROUTE_SHIFT;
            }
            Route::Fsb { address, data } => {
                if !caps.fsb {
                    return Err(HpetError::NotCapable);
                }
                // FSB_INT_VAL in the low dword, FSB_INT_ADDR in the high one.
                self.mmio.write32(reg::timer_fsb_route(n), data);
                self.mmio.write32(reg::timer_fsb_route(n) + 4, address);
                c |= timer::FSB_ENABLE;
            }
        }
        if trigger == Trigger::Level {
            c |= timer::LEVEL;
        }
        self.set_timer_config(n, c);
        Ok(())
    }

    /// Disables timer `n`'s interrupt and periodic mode.
    pub fn stop_timer(&self, n: u8) -> Result<(), HpetError> {
        self.check(n)?;
        let c = self.timer_config(n) & !(timer::INT_ENABLE | timer::PERIODIC | timer::VAL_SET);
        self.set_timer_config(n, c);
        Ok(())
    }

    /// Stops every timer, for a clean start (firmware may have left some
    /// running).
    pub fn stop_all_timers(&self) {
        for n in 0..self.caps.timer_count() {
            let _ = self.stop_timer(n);
        }
    }

    pub fn comparator(&self, n: u8) -> Result<u64, HpetError> {
        self.check(n)?;
        Ok(read64(&self.mmio, reg::timer_comparator(n)))
    }

    fn comparator_32bit(&self, n: u8) -> bool {
        let c = self.timer_config(n);
        c & timer::SIZE_64 == 0 || c & timer::MODE_32 != 0
    }

    fn write_comparator(&self, n: u8, value: u64) {
        self.mmio.write32(reg::timer_comparator(n), value as u32);
        if !self.comparator_32bit(n) {
            self.mmio.write32(reg::timer_comparator(n) + 4, (value >> 32) as u32);
        }
    }

    /// Arms timer `n` (already routed with [`Hpet::configure_timer`]) to
    /// interrupt once when the main counter reaches `deadline`.
    ///
    /// The comparator matches on equality, so a deadline the counter has
    /// already passed never fires. Returns `Ok(false)` in that case (the
    /// caller handles the event directly), `Ok(true)` if the deadline is
    /// still ahead.
    pub fn arm_one_shot(&self, n: u8, deadline: u64) -> Result<bool, HpetError> {
        self.check(n)?;
        let c = self.timer_config(n) & !(timer::PERIODIC | timer::VAL_SET);
        // Interrupt off while the two halves are written, so a half-written
        // comparator cannot match.
        self.set_timer_config(n, c & !timer::INT_ENABLE);
        self.write_comparator(n, deadline);
        self.set_timer_config(n, c | timer::INT_ENABLE);
        let bits = if self.comparator_32bit(n) { 32 } else { self.bits() };
        let now = self.counter() & mask(bits);
        let target = deadline & mask(bits);
        // Ahead of us if the distance forward is under half the range.
        let ahead = delta(now, target, bits);
        Ok(ahead != 0 && ahead <= mask(bits) / 2)
    }

    /// Makes timer `n` interrupt every `period` ticks, the first time
    /// `period` ticks from now.
    ///
    /// Follows the periodic set-up in §2.3.9.2.2: with Tn_TYPE_CNF and
    /// Tn_VAL_SET_CNF set, the first comparator write sets the time of the
    /// first interrupt and the second the period. A 64-bit timer is put in
    /// 32-bit mode first so each of those is one 32-bit write, which is why
    /// `period` is limited to `u32::MAX` ticks (5 minutes at 14.3 MHz).
    pub fn start_periodic(&self, n: u8, period: u64) -> Result<(), HpetError> {
        let caps = self.timer(n)?;
        if !caps.periodic {
            return Err(HpetError::NotCapable);
        }
        if period == 0 || period > u32::MAX as u64 {
            return Err(HpetError::InvalidPeriod);
        }
        let mut c = self.timer_config(n) & !timer::INT_ENABLE;
        if caps.wide {
            c |= timer::MODE_32;
        }
        self.set_timer_config(n, c);
        let first = self.counter().wrapping_add(period);
        self.set_timer_config(n, c | timer::PERIODIC | timer::VAL_SET);
        self.mmio.write32(reg::timer_comparator(n), first as u32);
        self.mmio.write32(reg::timer_comparator(n), period as u32);
        self.set_timer_config(n, c | timer::PERIODIC | timer::INT_ENABLE);
        Ok(())
    }

    /// Tn_INT_STS bits of level-triggered timers.
    pub fn interrupt_status(&self) -> u32 {
        self.mmio.read32(reg::INTERRUPT_STATUS)
    }

    /// Acknowledges a level-triggered interrupt of timer `n` (write one to
    /// clear).
    pub fn clear_interrupt(&self, n: u8) -> Result<(), HpetError> {
        self.check(n)?;
        self.mmio.write32(reg::INTERRUPT_STATUS, 1 << n);
        Ok(())
    }
}

impl<M: Mmio> Counter for Hpet<M> {
    fn read(&self) -> u64 {
        self.counter()
    }

    fn bits(&self) -> u32 {
        if self.caps.counter_64bit() { 64 } else { 32 }
    }
}

/// Reads a register that has no side effects on read as two dwords.
fn read64<M: Mmio>(mmio: &M, offset: usize) -> u64 {
    mmio.read32(offset) as u64 | (mmio.read32(offset + 4) as u64) << 32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{SimClock, SimHpet};

    #[test]
    fn capabilities_decode() {
        let caps = Capabilities(0x0429_b17f_8086_a201);
        assert_eq!(caps.revision(), 1);
        assert_eq!(caps.timer_count(), 3);
        assert!(caps.counter_64bit());
        assert!(caps.legacy_replacement());
        assert_eq!(caps.vendor_id(), 0x8086);
        assert_eq!(caps.period_fs(), 69_841_279);
        assert_eq!(caps.frequency_hz(), 14_318_179);
        // 14318180 ticks are 1.000000004 s: the exact period, not the
        // rounded frequency.
        assert!((1_000_000_002..=1_000_000_004).contains(&caps.ticks_to_ns().unwrap().apply(14_318_180)));
        assert_eq!(Capabilities(0).frequency_hz(), 0);
        assert_eq!(Capabilities(0).ticks_to_ns(), None);
    }

    #[test]
    fn rejects_absent_or_broken_blocks() {
        let clock = SimClock::new(1);
        let sim = SimHpet::new(&clock, true);
        sim.set_caps(u64::MAX);
        assert_eq!(Hpet::new(&sim).err(), Some(HpetError::NotPresent));
        sim.set_caps(0x0000_0000_8086_a201);
        assert_eq!(Hpet::new(&sim).err(), Some(HpetError::BadPeriod(0)));
        sim.set_caps(0x05f5_e101_8086_a201);
        assert_eq!(Hpet::new(&sim).err(), Some(HpetError::BadPeriod(100_000_001)));
        sim.set_caps(0x05f5_e100_8086_a201);
        assert!(Hpet::new(&sim).is_ok());
    }

    #[test]
    fn counter_runs_only_while_enabled() {
        let clock = SimClock::new(1);
        let sim = SimHpet::new(&clock, true);
        let h = Hpet::new(&sim).unwrap();
        assert!(!h.is_enabled());
        clock.advance_ps(1_000_000_000);
        assert_eq!(h.counter(), 0);
        h.enable();
        assert!(h.is_enabled());
        clock.advance_ps(1_000_000_000_000); // 1 s
        assert_eq!(h.counter(), 14_318_179);
        assert_eq!(h.set_counter(5), Err(HpetError::CounterRunning));
        h.disable();
        clock.advance_ps(1_000_000_000_000);
        assert_eq!(h.counter(), 14_318_179);
        h.set_counter(0x1_2345_6789).unwrap();
        assert_eq!(h.counter(), 0x1_2345_6789);
        // The capabilities register is read-only.
        sim.write_raw(reg::CAPABILITIES, 0);
        assert_eq!(Hpet::new(&sim).unwrap().capabilities(), h.capabilities());
    }

    #[test]
    fn wide_counter_read_does_not_tear() {
        let clock = SimClock::new(1);
        let sim = SimHpet::new(&clock, true);
        let h = Hpet::new(&sim).unwrap();
        // Each register read takes about one tick, so the low half wraps
        // between the reads of the high half.
        sim.set_read_cost_ps(70_000);
        for start in [0xffff_fffe, 0xffff_ffff, 0x1_ffff_fffd] {
            h.disable();
            h.set_counter(start).unwrap();
            h.enable();
            let v = h.counter();
            assert!(v >= start && v < start + 10, "{start:#x} -> {v:#x}");
        }
    }

    #[test]
    fn narrow_counter_wraps() {
        let clock = SimClock::new(1);
        let sim = SimHpet::new(&clock, false);
        let h = Hpet::new(&sim).unwrap();
        assert_eq!(Counter::bits(&h), 32);
        h.set_counter(0xffff_ff00).unwrap();
        h.enable();
        clock.advance_ps(1_000_000_000); // 1 ms: 14318 ticks
        let v = h.counter();
        assert_eq!(v, (0xffff_ff00u64 + 14_318) & 0xffff_ffff);
        assert!(v < 0x1_0000);
    }

    #[test]
    fn timer_caps_and_routing() {
        let clock = SimClock::new(1);
        let sim = SimHpet::new(&clock, true);
        let h = Hpet::new(&sim).unwrap();
        assert_eq!(h.timer(3), Err(HpetError::NoSuchTimer));
        let t0 = h.timer(0).unwrap();
        assert!(t0.periodic && t0.wide && !t0.fsb);
        assert!(t0.can_route_to(2) && !t0.can_route_to(3) && !t0.can_route_to(40));
        assert_eq!(h.configure_timer(0, Route::IoApic(3), Trigger::Edge), Err(HpetError::NotCapable));
        let fsb = Route::Fsb { address: 0xfee0_0000, data: 0x40 };
        assert_eq!(h.configure_timer(0, fsb, Trigger::Edge), Err(HpetError::NotCapable));
        h.configure_timer(0, Route::IoApic(2), Trigger::Level).unwrap();
        let c = sim.timer_config(0);
        assert_eq!((c & timer::ROUTE_MASK) >> timer::ROUTE_SHIFT, 2);
        assert!(c & timer::LEVEL != 0 && c & timer::INT_ENABLE == 0);
        // Timer 2 can use FSB delivery.
        h.configure_timer(2, Route::Fsb { address: 0xfee0_1000, data: 0x41 }, Trigger::Edge).unwrap();
        assert_eq!(sim.fsb_route(2), 0xfee0_1000_0000_0041);
        assert!(sim.timer_config(2) & timer::FSB_ENABLE != 0);
        // Read-only capability bits survive the writes.
        assert_eq!(h.timer(0).unwrap(), t0);
        assert!(h.timer(2).unwrap().fsb);
    }

    #[test]
    fn one_shot() {
        let clock = SimClock::new(1);
        let sim = SimHpet::new(&clock, true);
        let h = Hpet::new(&sim).unwrap();
        h.configure_timer(1, Route::IoApic(8), Trigger::Edge).unwrap();
        h.set_counter(0x1_0000_0000).unwrap();
        h.enable();
        assert_eq!(h.arm_one_shot(1, 0x1_0000_1000), Ok(true));
        assert_eq!(h.comparator(1), Ok(0x1_0000_1000));
        assert!(sim.timer_config(1) & timer::INT_ENABLE != 0);
        // The comparator was never live with only one half written.
        assert!(sim.comparator_writes_while_enabled() == 0);
        // Already in the past.
        assert_eq!(h.arm_one_shot(1, 0x0_ffff_0000), Ok(false));
        assert_eq!(h.arm_one_shot(9, 0), Err(HpetError::NoSuchTimer));
        // A 32-bit timer compares the low 32 bits only.
        h.configure_timer(2, Route::IoApic(20), Trigger::Edge).unwrap();
        assert_eq!(h.arm_one_shot(2, 0x1_0010_0000), Ok(true));
        assert_eq!(h.comparator(2), Ok(0x0010_0000));
        h.stop_timer(1).unwrap();
        assert!(sim.timer_config(1) & timer::INT_ENABLE == 0);
    }

    #[test]
    fn periodic() {
        let clock = SimClock::new(1);
        let sim = SimHpet::new(&clock, true);
        let h = Hpet::new(&sim).unwrap();
        h.configure_timer(0, Route::IoApic(2), Trigger::Edge).unwrap();
        h.set_counter(1000).unwrap();
        h.enable();
        assert_eq!(h.start_periodic(0, 0), Err(HpetError::InvalidPeriod));
        assert_eq!(h.start_periodic(0, 1 << 32), Err(HpetError::InvalidPeriod));
        assert_eq!(h.start_periodic(1, 100), Err(HpetError::NotCapable));
        h.start_periodic(0, 14_318).unwrap();
        let (accumulator, period) = sim.periodic_state(0);
        assert_eq!(period, 14_318);
        assert!((1000 + 14_318..1000 + 14_318 + 10).contains(&accumulator), "{accumulator}");
        let c = sim.timer_config(0);
        assert!(c & timer::PERIODIC != 0 && c & timer::INT_ENABLE != 0 && c & timer::MODE_32 != 0);
        assert!(c & timer::VAL_SET == 0);
        h.stop_all_timers();
        assert!(sim.timer_config(0) & (timer::PERIODIC | timer::INT_ENABLE) == 0);
    }

    #[test]
    fn legacy_replacement_and_interrupt_status() {
        let clock = SimClock::new(1);
        let sim = SimHpet::new(&clock, true);
        let h = Hpet::new(&sim).unwrap();
        h.set_legacy_replacement(true).unwrap();
        assert!(sim.configuration() & config::LEGACY_REPLACEMENT != 0);
        h.set_legacy_replacement(false).unwrap();
        assert!(sim.configuration() & config::LEGACY_REPLACEMENT == 0);
        sim.raise(0);
        sim.raise(2);
        assert_eq!(h.interrupt_status(), 0b101);
        h.clear_interrupt(2).unwrap();
        assert_eq!(h.interrupt_status(), 0b001);
        assert_eq!(h.clear_interrupt(3), Err(HpetError::NoSuchTimer));
        let narrow = SimHpet::new(&clock, false);
        assert_eq!(Hpet::new(&narrow).unwrap().set_legacy_replacement(true), Err(HpetError::NotCapable));
    }
}
