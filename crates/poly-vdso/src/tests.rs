extern crate std;

use super::*;
use poly_irq::time::{Scale, mul_div};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::vec::Vec;

/// The conversion the kernel's clock uses for a counter at `hz`.
fn conversion(hz: u64, origin: u64, offset_ns: u64) -> Conversion {
    let s = Scale::ticks_to_ns(hz).unwrap();
    Conversion { origin, mult: s.mult(), shift: s.shift(), offset_ns }
}

fn tsc_snapshot(hz: u64) -> Snapshot {
    Snapshot {
        mode: Mode::Tsc,
        conversion: conversion(hz, 1_000_000, 5_000),
        realtime_offset_ns: 1_700_000_000 * NANOS_PER_SEC,
        coarse_resolution_ns: 1_000_000,
        timezone: Timezone { minuteswest: -60, dsttime: 1 },
    }
}

#[test]
fn conversion_matches_the_kernel_clock_arithmetic() {
    // Poly-irq's `Scale` is what `MonotonicClock` applies; the vDSO must
    // give the same nanoseconds for the same reading, which are never
    // above the exact value and at most 2 below.
    let mut x: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for hz in [1_000_000, 14_318_180, 100_000_000, 2_699_992_000, 3_000_000_000, 10_000_000_000] {
        let scale = Scale::ticks_to_ns(hz).unwrap();
        let c = conversion(hz, 12_345, 0);
        for _ in 0..2_000 {
            // Up to about ten years of ticks.
            let ticks = next() % (hz * 3600 * 24 * 3650);
            let got = c.ns(12_345 + ticks);
            assert_eq!(got, scale.apply(ticks), "the same formula as the kernel's clock");
            let exact = mul_div(ticks, NANOS_PER_SEC, hz);
            assert!(got <= exact && exact - got <= 2, "{hz} Hz, {ticks} ticks: {got} vs {exact}");
        }
    }
}

#[test]
fn conversion_is_monotonic_and_saturates() {
    let c = conversion(2_500_000_000, 1_000, 777);
    assert_eq!(c.ns(1_000), 777);
    assert_eq!(c.ns(999), 777, "a reading before the origin counts as the origin");
    assert_eq!(c.ns(0), 777);
    let mut last = 0;
    for t in (1_000..1_000_000_000).step_by(9_973_331) {
        let ns = c.ns(t);
        assert!(ns >= last);
        last = ns;
    }
    let huge = Conversion { origin: 0, mult: u64::MAX, shift: 0, offset_ns: 1 };
    assert_eq!(huge.ns(i64::MAX as u64), u64::MAX);
    let late = Conversion { offset_ns: u64::MAX - 1, ..c };
    assert_eq!(late.ns(u64::MAX / 4), u64::MAX);
}

#[test]
fn clocks_from_a_snapshot() {
    let s = tsc_snapshot(1_000_000_000);
    let counter = 1_000_000 + 3 * NANOS_PER_SEC;
    let mono = 5_000 + 3 * NANOS_PER_SEC;
    assert_eq!(s.monotonic_ns(counter), Some(mono));
    assert_eq!(s.clock_ns(VClock::Monotonic, counter), Some(mono));
    assert_eq!(s.clock_ns(VClock::MonotonicCoarse, counter), Some(mono));
    assert_eq!(s.clock_ns(VClock::Realtime, counter), Some(mono + 1_700_000_000 * NANOS_PER_SEC));
    assert_eq!(s.clock_ns(VClock::RealtimeCoarse, counter), Some(mono + 1_700_000_000 * NANOS_PER_SEC));
    let syscall = Snapshot { mode: Mode::Syscall, ..s };
    assert_eq!(syscall.monotonic_ns(counter), None);
    assert_eq!(syscall.clock_ns(VClock::Realtime, counter), None);
    assert_eq!(s.resolution_ns(VClock::Monotonic), 1);
    assert_eq!(s.resolution_ns(VClock::Realtime), 1);
    assert_eq!(s.resolution_ns(VClock::RealtimeCoarse), 1_000_000);
    assert_eq!(Snapshot::default().resolution_ns(VClock::MonotonicCoarse), 1);
}

#[test]
fn clock_ids() {
    use clock::*;
    assert_eq!(VClock::from_id(CLOCK_REALTIME), Some(VClock::Realtime));
    assert_eq!(VClock::from_id(CLOCK_MONOTONIC), Some(VClock::Monotonic));
    assert_eq!(VClock::from_id(CLOCK_MONOTONIC_RAW), Some(VClock::Monotonic));
    assert_eq!(VClock::from_id(CLOCK_BOOTTIME), Some(VClock::Monotonic));
    assert_eq!(VClock::from_id(CLOCK_REALTIME_COARSE), Some(VClock::RealtimeCoarse));
    assert_eq!(VClock::from_id(CLOCK_MONOTONIC_COARSE), Some(VClock::MonotonicCoarse));
    for id in [CLOCK_PROCESS_CPUTIME_ID, CLOCK_THREAD_CPUTIME_ID, 8, 11, 16, -1, -6, i32::MAX] {
        assert_eq!(VClock::from_id(id), None, "id {id} goes to the system call");
    }
}

#[test]
fn timespec_and_timeval() {
    assert_eq!(timespec(0), (0, 0));
    assert_eq!(timespec(1_999_999_999), (1, 999_999_999));
    assert_eq!(timespec(u64::MAX), (18_446_744_073, 709_551_615));
    assert_eq!(timeval(1_234_567_891), (1, 234_567));
    assert_eq!(timeval(999), (0, 0));
}

#[test]
fn a_zeroed_page_means_syscall() {
    let v = Vvar::new();
    assert_eq!(v.read(), Snapshot::default());
    assert_eq!(v.read().mode, Mode::Syscall);
    // Every byte zero is the same thing.
    // SAFETY: `Vvar` is atomics only, for which all-zero bytes are valid.
    let zeroed: Vvar = unsafe { core::mem::zeroed() };
    assert_eq!(zeroed.read(), Snapshot::default());
}

#[test]
fn publish_and_read_back() {
    let v = Vvar::new();
    let s = tsc_snapshot(3_000_000_000);
    v.publish(&s);
    assert_eq!(v.read(), s);
    assert_eq!(v.sequence(), 2);
    let t = Snapshot { realtime_offset_ns: 42, ..s };
    v.publish(&t);
    assert_eq!(v.try_read(), Some(t));
    assert_eq!(v.sequence(), 4);
}

#[test]
fn readers_back_off_during_a_write() {
    let v = Vvar::new();
    v.publish(&tsc_snapshot(1_000_000_000));
    v.begin_write();
    assert_eq!(v.try_read(), None, "odd sequence: a write is in progress");
    let s = tsc_snapshot(2_000_000_000);
    v.publish(&s);
    assert_eq!(v.sequence() & 1, 0);
    assert_eq!(v.try_read(), Some(s));
}

#[test]
fn the_sequence_wraps() {
    let v = Vvar::new();
    v.seq.store(u32::MAX - 1, Ordering::Relaxed);
    let s = tsc_snapshot(1_000_000_000);
    v.publish(&s);
    assert_eq!(v.sequence(), 0);
    assert_eq!(v.try_read(), Some(s));
}

/// A snapshot whose every field is derived from `i`, so a torn read shows.
fn numbered(i: u64) -> Snapshot {
    Snapshot {
        mode: if i.is_multiple_of(2) { Mode::Tsc } else { Mode::Syscall },
        conversion: Conversion { origin: i, mult: i.wrapping_mul(3), shift: (i % 64) as u32, offset_ns: !i },
        realtime_offset_ns: i.rotate_left(17),
        coarse_resolution_ns: i ^ 0x5555,
        timezone: Timezone { minuteswest: i as i32, dsttime: (i >> 32) as i32 },
    }
}

fn consistent(s: &Snapshot) -> bool {
    let i = s.conversion.origin;
    *s == numbered(i)
}

#[test]
fn concurrent_readers_never_see_a_torn_snapshot() {
    let v = Arc::new(Vvar::new());
    v.publish(&numbered(0));
    let stop = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..3)
        .map(|_| {
            let (v, stop) = (Arc::clone(&v), Arc::clone(&stop));
            std::thread::spawn(move || {
                let (mut reads, mut last) = (0u64, 0u64);
                while !stop.load(Ordering::Relaxed) {
                    let s = v.read();
                    assert!(consistent(&s), "torn snapshot {s:?}");
                    assert!(s.conversion.origin >= last, "an older snapshot after a newer one");
                    last = s.conversion.origin;
                    reads += 1;
                }
                reads
            })
        })
        .collect();
    for i in 1..200_000u64 {
        v.publish(&numbered(i));
    }
    stop.store(true, Ordering::Relaxed);
    let reads: u64 = readers.into_iter().map(|r| r.join().unwrap()).sum();
    assert!(reads > 0);
    assert_eq!(v.read(), numbered(199_999));
}
