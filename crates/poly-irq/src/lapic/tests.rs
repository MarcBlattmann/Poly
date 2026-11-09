//! Tests.

extern crate std;
use super::*;
use crate::sim::{SimApic, SimClock, SimHpet};
use std::vec::Vec;

const ALL: CpuFeatures = CpuFeatures {
    tsc: true,
    apic: true,
    x2apic: true,
    tsc_deadline: true,
    arat: true,
    invariant_tsc: true,
    hypervisor: false,
};

#[test]
fn x2apic_msr_numbers() {
    assert_eq!(x2apic_msr(reg::ID), 0x802);
    assert_eq!(x2apic_msr(reg::EOI), 0x80b);
    assert_eq!(x2apic_msr(reg::SVR), 0x80f);
    assert_eq!(x2apic_msr(reg::ESR), 0x828);
    assert_eq!(x2apic_msr(reg::ICR_LOW), 0x830);
    assert_eq!(x2apic_msr(reg::LVT_TIMER), 0x832);
    assert_eq!(x2apic_msr(reg::TIMER_DIVIDE), 0x83e);
    assert_eq!(x2apic_msr(reg::SELF_IPI), 0x83f);
}

#[test]
fn lvt_encoding_round_trips() {
    let l = Lvt::new(0x40)
        .with_delivery(DeliveryMode::Nmi)
        .with_polarity(Polarity::ActiveLow)
        .with_trigger(Trigger::Level)
        .with_mask(true);
    assert_eq!(l.bits(), 0x40 | 0b100 << 8 | 1 << 13 | 1 << 15 | 1 << 16);
    assert_eq!(l.vector(), 0x40);
    assert_eq!(l.delivery(), Some(DeliveryMode::Nmi));
    assert_eq!(l.polarity(), Polarity::ActiveLow);
    assert_eq!(l.trigger(), Trigger::Level);
    assert!(l.is_masked());
    let l = l.with_mask(false).with_polarity(Polarity::ActiveHigh).with_trigger(Trigger::Edge);
    assert_eq!(l.bits(), 0x40 | 0b100 << 8);
    for mode in [TimerMode::OneShot, TimerMode::Periodic, TimerMode::TscDeadline] {
        assert_eq!(Lvt::new(0x20).with_timer_mode(mode).timer_mode(), Some(mode));
    }
    assert_eq!(Lvt::from_bits(0b11 << 17).timer_mode(), None);
    assert_eq!(Lvt::from_bits(0b011 << 8).delivery(), None);
    assert!(Lvt::from_bits(1 << 12).is_pending());
    assert!(Lvt::from_bits(1 << 14).remote_irr());
    assert_eq!(Lvt::new(1).with_vector(0xfe).vector(), 0xfe);
}

#[test]
fn divide_encoding_round_trips() {
    let all =
        [Divide::By1, Divide::By2, Divide::By4, Divide::By8, Divide::By16, Divide::By32, Divide::By64, Divide::By128];
    for (i, d) in all.into_iter().enumerate() {
        assert_eq!(Divide::from_bits(d.bits()), d);
        assert_eq!(d.divisor(), 1 << i);
        assert_eq!(d.bits() & 0b0100, 0);
    }
    // Bit 2 is reserved and ignored.
    assert_eq!(Divide::from_bits(0b0111), Divide::By16);
}

#[test]
fn ipi_encoding() {
    // The SDM's broadcast INIT and SIPI values (Vol. 3A §10.4.4.1).
    assert_eq!(Ipi::init(Destination::AllExcludingSelf).encode(), (0, 0x000c_4500));
    assert_eq!(Ipi::startup(Destination::AllExcludingSelf, 0x08).encode(), (0, 0x000c_4608));
    assert_eq!(Ipi::fixed(Destination::Physical(3), 0x30).encode(), (3, 0x4030));
    assert_eq!(Ipi::fixed(Destination::Logical(0b101), 0x31).encode(), (0b101, 0x4831));
    assert_eq!(Ipi::fixed(Destination::ToSelf, 0x32).encode(), (0, 0x4_4032));
    assert_eq!(Ipi::fixed(Destination::AllIncludingSelf, 0x33).encode(), (0, 0x8_4033));
    assert_eq!(Ipi::nmi(Destination::Physical(1)).encode(), (1, 0x4400));
    let deassert = Ipi { assert: false, trigger: Trigger::Level, ..Ipi::init(Destination::Physical(2)) };
    assert_eq!(deassert.encode(), (2, 0x8500));
    assert_eq!(Ipi::startup_at(Destination::Physical(1), 0x8000), Some(Ipi::startup(Destination::Physical(1), 8)));
    assert_eq!(Ipi::startup_at(Destination::Physical(1), 0x8001), None);
    assert_eq!(Ipi::startup_at(Destination::Physical(1), 0x10_0000), None);
}

#[test]
fn apic_base_transitions() {
    let clock = SimClock::new(1);
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    let base = read_apic_base(&&sim);
    assert!(base.is_bsp() && base.is_enabled() && !base.is_x2apic());
    assert_eq!(base.address(), 0xfee0_0000);
    assert_eq!(enable_x2apic(&&sim, &CpuFeatures::default()), Err(ApicError::Unsupported));
    let base = enable_x2apic(&&sim, &ALL).unwrap();
    assert!(base.is_x2apic() && base.is_enabled());
    assert_eq!(enable_xapic(&&sim), Err(ApicError::IllegalTransition));
    // Idempotent once in x2APIC mode.
    assert_eq!(enable_x2apic(&&sim, &ALL), Ok(base));

    // From globally disabled: EN first, then EXTD (the sim rejects
    // EXTD without EN).
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    sim.set_apic_base(0xfee0_0100);
    assert!(!read_apic_base(&&sim).is_enabled());
    assert!(enable_xapic(&&sim).unwrap().is_enabled());
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    sim.set_apic_base(0xfee0_0100);
    assert!(enable_x2apic(&&sim, &ALL).unwrap().is_x2apic());
}

/// Everything a driver can observe, collected while exercising most of
/// the interface.
fn scenario<R: ApicRegisters>(apic: &LocalApic<R>) -> Vec<u64> {
    let mut seen = Vec::new();
    apic.mask_all_lvts();
    apic.enable(0xff);
    apic.set_task_priority(0);
    seen.push(apic.id() as u64);
    seen.push(apic.version().0 as u64);
    seen.push(apic.is_enabled() as u64);
    seen.push(apic.spurious_vector() as u64);
    apic.set_lvt(LvtReg::Error, Lvt::new(0xfe)).unwrap();
    apic.set_lvt(LvtReg::Lint0, Lvt::masked().with_delivery(DeliveryMode::ExtInt)).unwrap();
    apic.set_lvt(LvtReg::Lint1, Lvt::new(0).with_delivery(DeliveryMode::Nmi)).unwrap();
    seen.push(apic.error_status().0 as u64);
    apic.start_timer(0x40, TimerMode::Periodic, Divide::By16, 100_000).unwrap();
    seen.push(apic.timer_divide().divisor() as u64);
    seen.push(apic.timer_initial_count() as u64);
    apic.send_ipi(&Ipi::fixed(Destination::Physical(1), 0x50)).unwrap();
    apic.send_ipi(&Ipi::init(Destination::AllExcludingSelf)).unwrap();
    apic.send_ipi(&Ipi::startup(Destination::AllExcludingSelf, 8)).unwrap();
    // An illegal vector caught by the hardware rather than by us.
    apic.registers().write_icr(1, 0x4005);
    seen.push(apic.error_status().0 as u64);
    seen.push(apic.error_status().0 as u64);
    apic.eoi();
    apic.set_task_priority(0x20);
    seen.push(apic.task_priority() as u64);
    seen.push(apic.processor_priority() as u64);
    for which in LvtReg::ALL {
        seen.push(apic.lvt(which).map_or(u64::MAX, |l| l.bits() as u64));
    }
    apic.stop_timer();
    seen.push(apic.timer_initial_count() as u64);
    seen
}

#[test]
fn xapic_and_x2apic_behave_the_same() {
    let clock = SimClock::new(1);
    for version in [0x0004_0011, 0x0005_0014, 0x0106_0015] {
        let (x, x2) = (SimApic::new(&clock, 3, version), SimApic::new(&clock, 3, version));
        enable_x2apic(&&x2, &ALL).unwrap();
        let a = LocalApic::new(XApic(&x), &ALL);
        let b = LocalApic::new(X2Apic(&x2), &ALL);
        assert!(!a.is_x2apic() && b.is_x2apic());
        let sa = scenario(&a);
        let sb = scenario(&b);
        assert_eq!(sa, sb, "version {version:#x}");
        assert_eq!(sa[0], 3);
        assert_eq!(x.register_writes(), x2.register_writes());
        assert_eq!(x.ipis(), x2.ipis());
        assert_eq!(x.ipis().len(), 4);
        assert_eq!(x.eois(), 1);
        assert_eq!(x2.eois(), 1);
        // The illegal-vector IPI was reported once, then cleared.
        assert_eq!(sa[4], 0);
        assert_eq!(sa[7], ErrorStatus::SEND_ILLEGAL_VECTOR as u64);
        assert_eq!(sa[8], 0);
    }
}

#[test]
fn mask_all_skips_missing_entries() {
    let clock = SimClock::new(1);
    // Max LVT 3: no perf, thermal or CMCI entry. The sim panics on any
    // access to one, like x2APIC hardware raising #GP.
    let sim = SimApic::new(&clock, 0, 0x0003_0014);
    enable_x2apic(&&sim, &ALL).unwrap();
    let apic = LocalApic::new(X2Apic(&sim), &ALL);
    apic.mask_all_lvts();
    assert_eq!(apic.lvt(LvtReg::Thermal), Err(ApicError::NotPresent));
    assert_eq!(apic.set_lvt(LvtReg::Cmci, Lvt::masked()), Err(ApicError::NotPresent));
    assert_eq!(apic.set_masked(LvtReg::Perf, true), Err(ApicError::NotPresent));
    assert!(apic.lvt(LvtReg::Lint0).unwrap().is_masked());
    let v = Version(0x0105_0014);
    assert!(v.is_integrated() && v.eoi_broadcast_suppression() && v.max_lvt() == 5 && v.version() == 0x14);
    assert!(LvtReg::Thermal.is_present(v) && !LvtReg::Cmci.is_present(v));
}

#[test]
fn lvt_masks_stick_while_software_disabled() {
    let clock = SimClock::new(1);
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    let apic = LocalApic::new(XApic(&sim), &ALL);
    assert!(!apic.is_enabled());
    apic.set_lvt(LvtReg::Lint1, Lvt::new(0x22)).unwrap();
    assert!(apic.lvt(LvtReg::Lint1).unwrap().is_masked());
    apic.enable(0xff);
    apic.set_masked(LvtReg::Lint1, false).unwrap();
    assert!(!apic.lvt(LvtReg::Lint1).unwrap().is_masked());
    apic.disable();
    assert!(!apic.is_enabled());
    assert!(apic.lvt(LvtReg::Lint1).unwrap().is_masked());
}

#[test]
fn read_only_bits_are_not_written() {
    let clock = SimClock::new(1);
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    let apic = LocalApic::new(XApic(&sim), &ALL);
    apic.enable(0xff);
    // A level-triggered LINT0 with remote IRR and delivery status set by
    // the hardware.
    sim.poke(reg::LVT_LINT0, 0x0000_d030 | 1 << 16);
    let lvt = apic.lvt(LvtReg::Lint0).unwrap();
    assert!(lvt.remote_irr() && lvt.is_pending());
    apic.set_masked(LvtReg::Lint0, false).unwrap();
    assert_eq!(sim.last_write(reg::LVT_LINT0), Some(0x0000_8030));
    // The hardware's bits are still there; ours changed.
    let lvt = apic.lvt(LvtReg::Lint0).unwrap();
    assert!(lvt.remote_irr() && !lvt.is_masked());
    // Writes to read-only registers are ignored in xAPIC mode.
    apic.registers().write(reg::VERSION, 0);
    assert_eq!(apic.version().0, 0x0005_0014);
}

#[test]
fn rejects_bad_vectors_and_destinations() {
    let clock = SimClock::new(1);
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    let apic = LocalApic::new(XApic(&sim), &ALL);
    apic.enable(0xff);
    assert_eq!(apic.set_lvt(LvtReg::Timer, Lvt::new(15)), Err(ApicError::InvalidVector));
    assert_eq!(apic.set_lvt(LvtReg::Timer, Lvt::new(15).with_mask(true)), Ok(()));
    assert_eq!(apic.set_lvt(LvtReg::Lint1, Lvt::new(0).with_delivery(DeliveryMode::Nmi)), Ok(()));
    assert_eq!(apic.send_ipi(&Ipi::fixed(Destination::Physical(1), 2)), Err(ApicError::InvalidVector));
    assert_eq!(apic.send_ipi(&Ipi::fixed(Destination::Physical(256), 0x40)), Err(ApicError::InvalidDestination));
    assert_eq!(apic.send_ipi(&Ipi::nmi(Destination::ToSelf)), Err(ApicError::InvalidIpi));
    let extint = Ipi { delivery: DeliveryMode::ExtInt, ..Ipi::fixed(Destination::Physical(1), 0x40) };
    assert_eq!(apic.send_ipi(&extint), Err(ApicError::InvalidIpi));
    assert!(sim.ipis().is_empty());
    assert_eq!(apic.start_timer(0x40, TimerMode::TscDeadline, Divide::By1, 1), Err(ApicError::InvalidTimerMode));

    // x2APIC IDs are 32 bits.
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    enable_x2apic(&&sim, &ALL).unwrap();
    let apic = LocalApic::new(X2Apic(&sim), &ALL);
    apic.send_ipi(&Ipi::fixed(Destination::Physical(0x1_0000), 0x40)).unwrap();
    assert_eq!(sim.ipis(), [(0x1_0000, 0x4040)]);
}

#[test]
fn xapic_ipis_poll_delivery_status() {
    let clock = SimClock::new(1);
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    let mut apic = LocalApic::new(XApic(&sim), &ALL);
    sim.set_busy_reads(5);
    apic.send_ipi(&Ipi::fixed(Destination::Physical(1), 0x40)).unwrap();
    // The second send waited for the first to drain.
    apic.send_ipi(&Ipi::fixed(Destination::Physical(2), 0x41)).unwrap();
    assert!(sim.icr_reads() >= 10);
    assert_eq!(sim.ipis(), [(1, 0x4040), (2, 0x4041)]);
    // A delivery that never completes times out instead of hanging.
    sim.set_busy_reads(u32::MAX);
    apic.set_icr_spin_limit(100);
    assert_eq!(apic.send_ipi(&Ipi::fixed(Destination::Physical(3), 0x42)), Err(ApicError::Timeout));
    assert_eq!(apic.send_ipi(&Ipi::fixed(Destination::Physical(4), 0x43)), Err(ApicError::Timeout));
    assert_eq!(sim.ipis().len(), 3, "nothing is sent while the previous IPI is still pending");
}

#[test]
fn isr_and_irr_bits() {
    let clock = SimClock::new(1);
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    let apic = LocalApic::new(XApic(&sim), &ALL);
    sim.poke(reg::ISR + 0x10 * 2, 1 << 1); // vector 65
    sim.poke(reg::IRR + 0x10 * 7, 1 << 31); // vector 255
    assert!(apic.is_in_service(65) && !apic.is_in_service(64));
    assert!(apic.is_requested(255) && !apic.is_requested(65));
}

#[test]
fn timer_one_shot_and_periodic() {
    let clock = SimClock::new(1);
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    let apic = LocalApic::new(XApic(&sim), &ALL);
    apic.enable(0xff);
    // Sim bus clock: 1 GHz. Divide by 4: 250 MHz, 4 ns per tick.
    apic.start_timer(0x40, TimerMode::OneShot, Divide::By4, 1000).unwrap();
    assert_eq!(apic.timer_current_count(), 1000);
    clock.advance_ps(400_000); // 400 ns: 100 ticks
    assert_eq!(apic.timer_current_count(), 900);
    clock.advance_ps(10_000_000);
    assert_eq!(apic.timer_current_count(), 0);
    let lvt = apic.lvt(LvtReg::Timer).unwrap();
    assert_eq!((lvt.vector(), lvt.timer_mode(), lvt.is_masked()), (0x40, Some(TimerMode::OneShot), false));
    apic.start_timer(0x41, TimerMode::Periodic, Divide::By1, 1000).unwrap();
    clock.advance_ps(2_500_000); // 2500 ticks: 2.5 periods
    assert_eq!(apic.timer_current_count(), 500);
    // The current count register is read-only.
    apic.registers().write(reg::TIMER_CURRENT, 7);
    assert_eq!(apic.timer_current_count(), 500);
    apic.stop_timer();
    assert_eq!(apic.timer_current_count(), 0);
    assert!(apic.lvt(LvtReg::Timer).unwrap().is_masked());
}

#[test]
fn tsc_deadline_mode() {
    let clock = SimClock::new(1);
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    let none = LocalApic::new(XApic(&sim), &CpuFeatures::default());
    assert_eq!(none.enable_tsc_deadline(0x40), Err(ApicError::Unsupported));
    assert_eq!(none.set_tsc_deadline(&&sim, 5), Err(ApicError::Unsupported));
    let apic = LocalApic::new(XApic(&sim), &ALL);
    apic.enable(0xff);
    apic.enable_tsc_deadline(0x40).unwrap();
    assert_eq!(apic.lvt(LvtReg::Timer).unwrap().timer_mode(), Some(TimerMode::TscDeadline));
    apic.set_tsc_deadline(&&sim, 123_456).unwrap();
    assert_eq!(sim.tsc_deadline(), 123_456);
    apic.set_tsc_deadline(&&sim, 0).unwrap();
    assert_eq!(sim.tsc_deadline(), 0);
}

#[test]
fn calibrates_the_timer_against_the_hpet() {
    let clock = SimClock::new(5).read_cost(300_000, 400_000);
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    let hpet_sim = SimHpet::new(&clock, true);
    let hpet = crate::Hpet::new(&hpet_sim).unwrap();
    hpet.enable();
    let hz = hpet.capabilities().frequency_hz();
    for (backend_x2, divide) in [(false, Divide::By16), (true, Divide::By1)] {
        if backend_x2 {
            enable_x2apic(&&sim, &ALL).unwrap();
        }
        let backend: Backend<&SimApic, &SimApic> =
            if backend_x2 { Backend::X2Apic(X2Apic(&sim)) } else { Backend::XApic(XApic(&sim)) };
        let apic = LocalApic::new(backend, &ALL);
        assert_eq!(apic.is_x2apic(), backend_x2);
        let cal = apic.calibrate_timer(&hpet, hz, divide, &CalibrationConfig::for_reference(hz)).unwrap();
        let expected = 1_000_000_000 / divide.divisor() as u64;
        let err_ppm = mul_div(cal.hz.abs_diff(expected), 1_000_000, expected);
        assert!(err_ppm < 100, "{divide:?}: {cal:?}");
        assert_eq!(apic.timer_initial_count(), 0);
        assert!(apic.lvt(LvtReg::Timer).unwrap().is_masked());
        assert_eq!(timer_count_for(cal.hz, 1_000_000), Some((cal.hz / 1000) as u32));
    }
}

#[test]
fn timer_calibrates_against_the_tsc_where_device_reads_exit() {
    // Like KVM: reading the timer's current count is a VM exit (2 us),
    // reading the HPET one to user space (3 us per register, three per
    // 64-bit counter read); reading the TSC costs nothing much. Against
    // the HPET each bracket is about 11 us, too wide for the standard
    // tolerance; against a calibrated TSC it is the 2 us of one exit.
    const TSC_HZ: u64 = 2_445_428_000;
    let clock = SimClock::new(23).read_cost(20_000, 20_000);
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    sim.set_timer_read_cost_ps(2_000_000);
    let hpet_sim = SimHpet::new(&clock, true);
    hpet_sim.set_read_cost_ps(3_000_000);
    let hpet = crate::Hpet::new(&hpet_sim).unwrap();
    hpet.enable();
    let hz = hpet.capabilities().frequency_hz();
    let tsc = crate::sim::FakeTsc::new(&clock, TSC_HZ);
    enable_x2apic(&&sim, &ALL).unwrap();
    let apic = LocalApic::new(X2Apic(&sim), &ALL);
    let strict = CalibrationConfig::for_reference(hz);

    let (result, report) = apic.calibrate_timer_report(&hpet, hz, Divide::By16, &strict);
    assert_eq!(result, Err(CalibrationError::Inconsistent), "{report}");
    assert!(report.samples().iter().all(|s| !s.tight), "{report}");

    let strict = CalibrationConfig::for_reference(TSC_HZ);
    let cal = apic.calibrate_timer(&tsc, TSC_HZ, Divide::By16, &strict).unwrap();
    let expected = 1_000_000_000 / 16;
    let err_ppm = mul_div(cal.hz.abs_diff(expected), 1_000_000, expected);
    assert!(err_ppm < 100, "{cal:?}");
    assert_eq!(cal.used, cal.taken);
    assert_eq!(apic.timer_initial_count(), 0);

    // Against the HPET the retries get there too, with looser settings.
    let r = crate::time::calibrate_with_retries(hz, |cfg| apic.calibrate_timer_report(&hpet, hz, Divide::By16, cfg));
    let cal = r.result.unwrap();
    assert!(r.attempts > 2, "{r:?}");
    assert!(mul_div(cal.hz.abs_diff(expected), 1_000_000, expected) < 1000, "{cal:?}");
}

#[test]
fn periodic_rate_is_measured_however_slow_the_count_reads() {
    // A 1 ms tick: 62.5 MHz (divide 16), initial count 62500, measured
    // against a TSC. Each read of the count costs `read_us` on top:
    // 0 (TCG), 7 us (the 14.8 us pair seen on KVM with -cpu max), 30 us.
    const TSC_HZ: u64 = 2_596_197_000;
    const HZ: u64 = 62_500_000;
    for (read_us, attempt) in [(0, 1), (7, 2), (30, 3)] {
        let clock = SimClock::new(31).read_cost(20_000, 10_000);
        let sim = SimApic::new(&clock, 0, 0x0005_0014);
        sim.set_timer_read_cost_ps(read_us * 1_000_000);
        enable_x2apic(&&sim, &ALL).unwrap();
        let apic = LocalApic::new(X2Apic(&sim), &ALL);
        apic.start_timer(0xf0, TimerMode::Periodic, Divide::By16, 62_500).unwrap();
        let tsc = crate::sim::FakeTsc::new(&clock, TSC_HZ);
        let mut guards = 0;
        let rate = apic.measure_periodic_rate(&tsc, TSC_HZ, HZ, 5, || guards += 1).unwrap();
        assert_eq!(rate.attempt, attempt, "{read_us} us: {rate:?}");
        assert!(guards >= attempt, "one guard per measurement");
        let err_ppm = mul_div(rate.hz.abs_diff(HZ), 1_000_000, HZ);
        assert!(err_ppm <= rate.uncertainty_ppm + 100, "{read_us} us: {rate:?} off by {err_ppm} ppm");
        assert!(rate.uncertainty_ppm <= 50_000, "{rate:?}");

        // A calibration 10% off shows as a rate 10% away from it, even
        // over the windows of several periods.
        let off = apic.measure_periodic_rate(&tsc, TSC_HZ, HZ * 11 / 10, 5, || ()).unwrap();
        let err_ppm = mul_div(off.hz.abs_diff(HZ), 1_000_000, HZ);
        assert!(err_ppm <= off.uncertainty_ppm + 100, "{read_us} us: {off:?} off by {err_ppm} ppm");
    }

    // Reads too slow for every window: disturbed, with the narrowest
    // pair of brackets (two 300 us reads, in TSC ticks) reported.
    let clock = SimClock::new(31).read_cost(20_000, 0);
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    sim.set_timer_read_cost_ps(300_000_000);
    let apic = LocalApic::new(XApic(&sim), &ALL);
    let tsc = crate::sim::FakeTsc::new(&clock, TSC_HZ);
    assert_eq!(apic.measure_periodic_rate(&tsc, TSC_HZ, HZ, 2, || ()), Err(PeriodicRateError::NotRunning));
    apic.start_timer(0xf0, TimerMode::Periodic, Divide::By16, 62_500).unwrap();
    match apic.measure_periodic_rate(&tsc, TSC_HZ, HZ, 2, || ()) {
        Err(PeriodicRateError::Disturbed { narrowest }) => {
            let ns = mul_div(narrowest, 1_000_000_000, TSC_HZ);
            assert!((600_000..601_000).contains(&ns), "{ns} ns");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        apic.measure_periodic_rate(&crate::sim::Stuck(3), 1_000_000, HZ, 2, || ()),
        Err(PeriodicRateError::ReferenceStuck)
    );
}

#[test]
fn timer_calibration_detects_expiry() {
    // A 1 GHz timer at divide 1 runs out after 4.3 s; ask for 6 x 1 s.
    let clock = SimClock::new(5).read_cost(1_000_000, 0);
    let sim = SimApic::new(&clock, 0, 0x0005_0014);
    let hpet_sim = SimHpet::new(&clock, true);
    let hpet = crate::Hpet::new(&hpet_sim).unwrap();
    hpet.enable();
    let hz = hpet.capabilities().frequency_hz();
    let apic = LocalApic::new(XApic(&sim), &ALL);
    let config = CalibrationConfig { window: hz, max_spins: u64::MAX, ..CalibrationConfig::for_reference(hz) };
    assert_eq!(apic.calibrate_timer(&hpet, hz, Divide::By1, &config), Err(CalibrationError::TargetStuck));
}

#[test]
fn timer_counts() {
    assert_eq!(timer_count_for(1_000_000_000, 1_000_000), Some(1_000_000));
    assert_eq!(timer_count_for(62_500_000, 1), None);
    assert_eq!(timer_count_for(1_000_000_000, 5_000_000_000), None);
    assert_eq!(timer_count_for(u64::MAX, u64::MAX), None);
}
