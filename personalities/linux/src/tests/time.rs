//! The time system calls against the fake kernel: clocks, setting them,
//! sleeps and their restarts, interval and POSIX timers (arming and
//! firing), process times (ADR 0044).

extern crate std;

use super::fake::*;
use crate::abi::signal::*;
use crate::abi::{SigAction, SigInfo, Timespec, clock as id};
use crate::errno::Errno;
use crate::kernel::{Clock, CpuTimes, Deadline, SigSet};
use crate::signal::restart::RestartBlock;
use crate::signal::{Dest, ExitToUser, LinuxProcess};
use crate::time::abi::*;
use crate::time::timers::{self, MAX_POSIX_TIMERS, Notify, TimerClock};
use crate::time::{TimerWakeup, fire};
use crate::{Linux, nr};
use std::vec::Vec;

const TS: u64 = DATA + 0x200;
const OUT: u64 = DATA + 0x300;
const OUT2: u64 = DATA + 0x400;
const EV: u64 = DATA + 0x500;
const SEC: u64 = 1_000_000_000;

fn put_ts(k: &mut FakeKernel, at: u64, sec: i64, nsec: i64) -> u64 {
    k.poke(at, &Timespec { sec, nsec }.to_bytes());
    at
}

fn ts_at(k: &FakeKernel, at: u64) -> Timespec {
    Timespec::from_bytes(&k.peek(at, 16).try_into().unwrap())
}

fn tv_at(k: &FakeKernel, at: u64) -> Timeval {
    Timeval::from_bytes(&k.peek(at, 16).try_into().unwrap())
}

fn root() -> FakeKernel {
    let mut k = FakeKernel::new();
    k.creds.euid = 0;
    k
}

/// The fake's monotonic clock in nanoseconds (42.999999999 s).
fn mono(k: &FakeKernel) -> u64 {
    timespec_ns(k.monotonic)
}

fn set_mono(k: &mut FakeKernel, ns: u64) {
    k.monotonic = Timespec::from_nanos(ns);
}

fn install(k: &mut FakeKernel, sig: u32, flags: u64) {
    let act = SigAction { handler: 0x40_5000, flags: flags | SA_RESTORER, restorer: 0x40_6000, mask: 0 };
    k.poke(DATA + 0x800, &act.to_bytes());
    sys(k, nr::RT_SIGACTION, &[sig as u64, DATA + 0x800, 0, 8]).unwrap();
}

fn interrupt_with(k: &mut FakeKernel, sig: u32) {
    k.add_thread(101);
    k.meanwhile(move |k| {
        k.as_thread(101, |k| sys(k, nr::TGKILL, &[100, 100, sig as u64]).unwrap());
    });
}

/// The saved `rip` and `rax` in the frame of the handler now running.
fn saved_rip_rax(k: &FakeKernel) -> (u64, u64) {
    let uc = k.ctx.rdx + crate::signal::frame::UC_MCONTEXT as u64;
    (k.peek_u64(uc + 128), k.peek_u64(uc + 104))
}

// ---- clocks ------------------------------------------------------------

#[test]
fn clock_getres_reports_resolutions() {
    let mut k = FakeKernel::new();
    for clock in [id::CLOCK_REALTIME, id::CLOCK_MONOTONIC, id::CLOCK_MONOTONIC_RAW, id::CLOCK_BOOTTIME] {
        assert_eq!(sys(&mut k, nr::CLOCK_GETRES, &[clock as u64, OUT]), Ok(0));
        assert_eq!(ts_at(&k, OUT), Timespec { sec: 0, nsec: 1 }, "clock {clock}");
    }
    for clock in [id::CLOCK_REALTIME_COARSE, id::CLOCK_MONOTONIC_COARSE] {
        assert_eq!(sys(&mut k, nr::CLOCK_GETRES, &[clock as u64, OUT]), Ok(0));
        assert_eq!(ts_at(&k, OUT), Timespec { sec: 0, nsec: 4_000_000 }, "the tick");
    }
    assert_eq!(sys(&mut k, nr::CLOCK_GETRES, &[id::CLOCK_MONOTONIC as u64, 0]), Ok(0), "res may be null");
    assert_eq!(sys(&mut k, nr::CLOCK_GETRES, &[99, OUT]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::CLOCK_GETRES, &[id::CLOCK_MONOTONIC as u64, UNMAPPED]), Err(Errno::EFAULT));
    // CPU clocks: only where the kernel accounts CPU time.
    assert_eq!(sys(&mut k, nr::CLOCK_GETRES, &[id::CLOCK_PROCESS_CPUTIME_ID as u64, OUT]), Err(Errno::EINVAL));
    k.cpu = Some([CpuTimes::default(); 3]);
    assert_eq!(sys(&mut k, nr::CLOCK_GETRES, &[id::CLOCK_THREAD_CPUTIME_ID as u64, OUT]), Ok(0));
    assert_eq!(ts_at(&k, OUT), Timespec { sec: 0, nsec: 1 });
}

#[test]
fn cpu_time_clocks_read_the_kernel_accounting() {
    let mut k = FakeKernel::new();
    let thread = CpuTimes { user_ns: 3 * SEC, system_ns: 5 };
    let process = CpuTimes { user_ns: 7 * SEC, system_ns: SEC };
    k.cpu = Some([thread, process, CpuTimes::default()]);
    sys(&mut k, nr::CLOCK_GETTIME, &[id::CLOCK_THREAD_CPUTIME_ID as u64, OUT]).unwrap();
    assert_eq!(ts_at(&k, OUT), Timespec { sec: 3, nsec: 5 });
    sys(&mut k, nr::CLOCK_GETTIME, &[id::CLOCK_PROCESS_CPUTIME_ID as u64, OUT]).unwrap();
    assert_eq!(ts_at(&k, OUT), Timespec { sec: 8, nsec: 0 });
}

#[test]
fn clock_settime_sets_the_wall_clock_only() {
    let mut k = root();
    let t = put_ts(&mut k, TS, 1_800_000_000, 5);
    assert_eq!(sys(&mut k, nr::CLOCK_SETTIME, &[id::CLOCK_REALTIME as u64, t]), Ok(0));
    assert_eq!(k.realtime, Timespec { sec: 1_800_000_000, nsec: 5 });
    for clock in [id::CLOCK_MONOTONIC, id::CLOCK_BOOTTIME, id::CLOCK_REALTIME_COARSE, id::CLOCK_PROCESS_CPUTIME_ID] {
        assert_eq!(sys(&mut k, nr::CLOCK_SETTIME, &[clock as u64, t]), Err(Errno::EINVAL), "clock {clock}");
    }
    assert_eq!(sys(&mut k, nr::CLOCK_SETTIME, &[1234, t]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::CLOCK_SETTIME, &[0, UNMAPPED]), Err(Errno::EFAULT));
    for (sec, nsec) in [(5, 1_000_000_000), (5, -1), (-1, 0)] {
        let t = put_ts(&mut k, TS, sec, nsec);
        assert_eq!(sys(&mut k, nr::CLOCK_SETTIME, &[0, t]), Err(Errno::EINVAL), "{sec}.{nsec}");
    }
    // Below the monotonic clock (42.999999999 s): EINVAL.
    let t = put_ts(&mut k, TS, 40, 0);
    assert_eq!(sys(&mut k, nr::CLOCK_SETTIME, &[0, t]), Err(Errno::EINVAL));
    // Unprivileged: EPERM, the clock unchanged.
    let mut k = FakeKernel::new();
    let t = put_ts(&mut k, TS, 1_800_000_000, 0);
    assert_eq!(sys(&mut k, nr::CLOCK_SETTIME, &[0, t]), Err(Errno::EPERM));
    assert_eq!(k.realtime.sec, 1_700_000_000);
}

#[test]
fn gettimeofday_time_and_settimeofday() {
    let mut k = root();
    k.realtime = Timespec { sec: 1_700_000_123, nsec: 456_789_999 };
    k.timezone = (-60, 1);
    assert_eq!(sys(&mut k, nr::GETTIMEOFDAY, &[OUT, OUT2]), Ok(0));
    assert_eq!(tv_at(&k, OUT), Timeval { sec: 1_700_000_123, usec: 456_789 });
    assert_eq!(k.peek(OUT2, 8), Timezone { minuteswest: -60, dsttime: 1 }.to_bytes());
    assert_eq!(sys(&mut k, nr::GETTIMEOFDAY, &[0, 0]), Ok(0), "both may be null");
    assert_eq!(sys(&mut k, nr::GETTIMEOFDAY, &[UNMAPPED, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::GETTIMEOFDAY, &[0, UNMAPPED]), Err(Errno::EFAULT));

    assert_eq!(sys(&mut k, nr::TIME, &[0]), Ok(1_700_000_123));
    assert_eq!(sys(&mut k, nr::TIME, &[OUT]), Ok(1_700_000_123));
    assert_eq!(k.peek_u64(OUT), 1_700_000_123);
    assert_eq!(sys(&mut k, nr::TIME, &[UNMAPPED]), Err(Errno::EFAULT));

    k.poke(OUT, &Timeval { sec: 1_900_000_000, usec: 250_000 }.to_bytes());
    k.poke(OUT2, &Timezone { minuteswest: 120, dsttime: 0 }.to_bytes());
    assert_eq!(sys(&mut k, nr::SETTIMEOFDAY, &[OUT, OUT2]), Ok(0));
    assert_eq!(k.realtime, Timespec { sec: 1_900_000_000, nsec: 250_000_000 });
    assert_eq!(k.timezone, (120, 0));
    assert_eq!(sys(&mut k, nr::SETTIMEOFDAY, &[0, 0]), Ok(0));
    // Only the time zone.
    k.poke(OUT2, &Timezone { minuteswest: -300, dsttime: 0 }.to_bytes());
    assert_eq!(sys(&mut k, nr::SETTIMEOFDAY, &[0, OUT2]), Ok(0));
    assert_eq!((k.realtime.sec, k.timezone), (1_900_000_000, (-300, 0)));
    // Out of range.
    k.poke(OUT, &Timeval { sec: 5, usec: 1_000_000 }.to_bytes());
    assert_eq!(sys(&mut k, nr::SETTIMEOFDAY, &[OUT, 0]), Err(Errno::EINVAL));
    k.poke(OUT2, &Timezone { minuteswest: 901, dsttime: 0 }.to_bytes());
    assert_eq!(sys(&mut k, nr::SETTIMEOFDAY, &[0, OUT2]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SETTIMEOFDAY, &[UNMAPPED, 0]), Err(Errno::EFAULT));
    k.poke(OUT, &Timeval { sec: 10, usec: 0 }.to_bytes());
    assert_eq!(sys(&mut k, nr::SETTIMEOFDAY, &[OUT, 0]), Err(Errno::EINVAL), "below the monotonic clock");
    // Unprivileged.
    let mut k = FakeKernel::new();
    k.poke(OUT, &Timeval { sec: 1_900_000_000, usec: 0 }.to_bytes());
    assert_eq!(sys(&mut k, nr::SETTIMEOFDAY, &[OUT, 0]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::SETTIMEOFDAY, &[0, 0]), Ok(0), "setting nothing needs no privilege");
}

// ---- sleeping ------------------------------------------------------------

#[test]
fn nanosleep_blocks_until_a_monotonic_deadline() {
    let mut k = FakeKernel::new();
    let t = put_ts(&mut k, TS, 1, 500_000_000);
    assert_eq!(sys(&mut k, nr::NANOSLEEP, &[t, OUT]), Ok(0));
    let deadline = Deadline { clock: Clock::Monotonic, at: Timespec { sec: 44, nsec: 499_999_999 } };
    assert_eq!(k.deadlines, [Some(deadline)]);
    let t = put_ts(&mut k, TS, 0, 0);
    assert_eq!(sys(&mut k, nr::NANOSLEEP, &[t, 0]), Ok(0), "zero sleeps");
    for (sec, nsec) in [(0, 1_000_000_000), (0, -1), (-1, 0)] {
        let t = put_ts(&mut k, TS, sec, nsec);
        assert_eq!(sys(&mut k, nr::NANOSLEEP, &[t, 0]), Err(Errno::EINVAL));
    }
    assert_eq!(sys(&mut k, nr::NANOSLEEP, &[UNMAPPED, 0]), Err(Errno::EFAULT));
    // A spurious wake-up does not end it.
    k.unblocked.insert(100);
    let t = put_ts(&mut k, TS, 2, 0);
    let before = k.blocks;
    assert_eq!(sys(&mut k, nr::NANOSLEEP, &[t, 0]), Ok(0));
    assert_eq!(k.blocks - before, 2);
}

#[test]
fn nanosleep_interrupted_by_a_handler_reports_the_time_left() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, SA_RESTART);
    k.add_thread(101);
    // The signal comes a quarter second in.
    k.meanwhile(|k| {
        k.monotonic = Timespec { sec: 43, nsec: 249_999_999 };
        k.as_thread(101, |k| sys(k, nr::TGKILL, &[100, 100, SIGUSR1 as u64]).unwrap());
    });
    let t = put_ts(&mut k, TS, 1, 0);
    let (exit, ctx) = run(&mut k, nr::NANOSLEEP, &[t, OUT]);
    assert_eq!(exit, ExitToUser::Return);
    assert_eq!(ctx.rip, 0x40_5000, "the handler runs");
    assert_eq!(saved_rip_rax(&k), (SYSCALL_RIP, Errno::EINTR.to_rax()), "EINTR even with SA_RESTART");
    assert_eq!(ts_at(&k, OUT), Timespec { sec: 0, nsec: 750_000_000 });
    assert_eq!(k.thread_state().restart_block, None);
    // A bad rem: EFAULT instead.
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, 0);
    interrupt_with(&mut k, SIGUSR1);
    let t = put_ts(&mut k, TS, 1, 0);
    run(&mut k, nr::NANOSLEEP, &[t, UNMAPPED]);
    assert_eq!(saved_rip_rax(&k), (SYSCALL_RIP, Errno::EFAULT.to_rax()));
}

#[test]
fn nanosleep_continues_after_a_stop() {
    let mut k = FakeKernel::new();
    interrupt_with(&mut k, SIGTSTP);
    let t = put_ts(&mut k, TS, 5, 0);
    let (_, ctx) = run(&mut k, nr::NANOSLEEP, &[t, OUT]);
    assert_eq!(k.stops, [SIGTSTP]);
    assert_eq!((ctx.rip, ctx.rax), (SYSCALL_RIP - 2, nr::RESTART_SYSCALL));
    let deadline = Deadline { clock: Clock::Monotonic, at: Timespec { sec: 47, nsec: 999_999_999 } };
    assert_eq!(k.thread_state().restart_block, Some(RestartBlock::Sleep { deadline, rem: OUT }));
    // restart_syscall sleeps on until the same deadline.
    assert_eq!(run(&mut k, nr::RESTART_SYSCALL, &[]).1.rax, 0);
    assert_eq!(k.deadlines.last(), Some(&Some(deadline)));
    assert_eq!(k.thread_state().restart_block, None);
}

#[test]
fn clock_nanosleep_absolute_and_relative() {
    let mut k = FakeKernel::new();
    // Absolute on the wall clock: the deadline as given.
    k.meanwhile(|k| k.realtime = Timespec { sec: 1_700_000_100, nsec: 0 });
    let t = put_ts(&mut k, TS, 1_700_000_050, 0);
    let abs = TIMER_ABSTIME as u64;
    assert_eq!(sys(&mut k, nr::CLOCK_NANOSLEEP, &[id::CLOCK_REALTIME as u64, abs, t, OUT]), Ok(0));
    let wall = Deadline { clock: Clock::Realtime, at: Timespec { sec: 1_700_000_050, nsec: 0 } };
    assert_eq!(k.deadlines.last(), Some(&Some(wall)));
    // Absolute on the monotonic clock and on CLOCK_BOOTTIME.
    for clock in [id::CLOCK_MONOTONIC, id::CLOCK_BOOTTIME] {
        let t = put_ts(&mut k, TS, 50, 0);
        assert_eq!(sys(&mut k, nr::CLOCK_NANOSLEEP, &[clock as u64, abs, t, 0]), Ok(0));
        let d = Deadline { clock: Clock::Monotonic, at: Timespec { sec: 50, nsec: 0 } };
        assert_eq!(k.deadlines.last(), Some(&Some(d)));
    }
    // Relative, whatever the clock: on the monotonic clock.
    let t = put_ts(&mut k, TS, 2, 0);
    assert_eq!(sys(&mut k, nr::CLOCK_NANOSLEEP, &[0, 0, t, OUT]), Ok(0));
    let d = Deadline { clock: Clock::Monotonic, at: Timespec { sec: 44, nsec: 999_999_999 } };
    assert_eq!(k.deadlines.last(), Some(&Some(d)));
    // Errors.
    let call = |k: &mut FakeKernel, clock: i32| sys(k, nr::CLOCK_NANOSLEEP, &[clock as u64, 0, TS, 0]);
    assert_eq!(call(&mut k, id::CLOCK_THREAD_CPUTIME_ID), Err(Errno::EINVAL));
    assert_eq!(call(&mut k, 77), Err(Errno::EINVAL));
    assert_eq!(call(&mut k, id::CLOCK_PROCESS_CPUTIME_ID), Err(Errno::EOPNOTSUPP));
    let t = put_ts(&mut k, TS, 0, 2_000_000_000);
    assert_eq!(sys(&mut k, nr::CLOCK_NANOSLEEP, &[1, 0, t, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::CLOCK_NANOSLEEP, &[1, 0, UNMAPPED, 0]), Err(Errno::EFAULT));
}

#[test]
fn an_absolute_sleep_interrupted_by_a_handler_fails_and_restarts_after_a_stop() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, SA_RESTART);
    interrupt_with(&mut k, SIGUSR1);
    k.poke(OUT, &[0xaa; 16]);
    let t = put_ts(&mut k, TS, 99, 0);
    run(&mut k, nr::CLOCK_NANOSLEEP, &[1, TIMER_ABSTIME as u64, t, OUT]);
    assert_eq!(saved_rip_rax(&k), (SYSCALL_RIP, Errno::EINTR.to_rax()));
    assert_eq!(k.peek(OUT, 16), [0xaa; 16], "remain is not written for an absolute sleep");
    // After a stop: the same call again.
    let mut k = FakeKernel::new();
    interrupt_with(&mut k, SIGTSTP);
    let t = put_ts(&mut k, TS, 99, 0);
    let (_, ctx) = run(&mut k, nr::CLOCK_NANOSLEEP, &[1, TIMER_ABSTIME as u64, t, OUT]);
    assert_eq!((ctx.rip, ctx.rax), (SYSCALL_RIP - 2, nr::CLOCK_NANOSLEEP));
}

// ---- interval timers -------------------------------------------------------

fn itimerval(k: &mut FakeKernel, at: u64, value_us: u64, interval_us: u64) -> u64 {
    let tv = |us: u64| Timeval { sec: (us / 1_000_000) as i64, usec: (us % 1_000_000) as i64 };
    k.poke(at, &Itimerval { interval: tv(interval_us), value: tv(value_us) }.to_bytes());
    at
}

fn read_itimerval(k: &FakeKernel, at: u64) -> Itimerval {
    Itimerval::from_bytes(&k.peek(at, 32).try_into().unwrap())
}

#[test]
fn alarm_arms_itimer_real() {
    let mut k = FakeKernel::new();
    let now = mono(&k);
    assert_eq!(sys(&mut k, nr::ALARM, &[5]), Ok(0));
    assert_eq!(k.timer_wakeups.last(), Some(&TimerWakeup { monotonic: Some(now + 5 * SEC), cpu: false }));
    // 2.4 s later: 2.6 s left, reported as 3.
    set_mono(&mut k, now + 2_400_000_000);
    assert_eq!(sys(&mut k, nr::ALARM, &[10]), Ok(3));
    // Almost due: still at least 1.
    set_mono(&mut k, now + 2_400_000_000 + 10 * SEC - 1000);
    assert_eq!(sys(&mut k, nr::ALARM, &[0]), Ok(1), "cancelled, but it was pending");
    assert_eq!(k.timer_wakeups.last(), Some(&TimerWakeup::default()));
    assert_eq!(sys(&mut k, nr::ALARM, &[0]), Ok(0));
    // The upper half of the register is ignored (unsigned int).
    assert_eq!(sys(&mut k, nr::ALARM, &[0xffff_ffff_0000_0002]), Ok(0));
    sys(&mut k, nr::GETITIMER, &[ITIMER_REAL as u64, OUT]).unwrap();
    assert_eq!(read_itimerval(&k, OUT).value, Timeval { sec: 2, usec: 0 });
}

#[test]
fn setitimer_and_getitimer() {
    let mut k = FakeKernel::new();
    let now = mono(&k);
    let new = itimerval(&mut k, TS, 1_500_000, 250_000);
    assert_eq!(sys(&mut k, nr::SETITIMER, &[ITIMER_REAL as u64, new, OUT]), Ok(0));
    assert_eq!(read_itimerval(&k, OUT), Itimerval::default(), "nothing was armed");
    assert_eq!(k.timer_wakeups.last().unwrap().monotonic, Some(now + 1_500_000_000));
    set_mono(&mut k, now + 500_000_000);
    assert_eq!(sys(&mut k, nr::GETITIMER, &[ITIMER_REAL as u64, OUT]), Ok(0));
    let got = read_itimerval(&k, OUT);
    assert_eq!((got.value, got.interval), (Timeval { sec: 1, usec: 0 }, Timeval { sec: 0, usec: 250_000 }));
    // Disarm with a zero value; the old setting comes back.
    let zero = itimerval(&mut k, TS, 0, 100);
    assert_eq!(sys(&mut k, nr::SETITIMER, &[ITIMER_REAL as u64, zero, OUT]), Ok(0));
    assert_eq!(read_itimerval(&k, OUT).value, Timeval { sec: 1, usec: 0 });
    sys(&mut k, nr::GETITIMER, &[ITIMER_REAL as u64, OUT]).unwrap();
    assert_eq!(read_itimerval(&k, OUT), Itimerval::default(), "disarmed, interval dropped");
    // Errors.
    assert_eq!(sys(&mut k, nr::SETITIMER, &[3, new, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::GETITIMER, &[u64::MAX, OUT]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::GETITIMER, &[0, UNMAPPED]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::SETITIMER, &[0, UNMAPPED, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::SETITIMER, &[0, 0, 0]), Err(Errno::EFAULT));
    k.poke(TS + 24, &1_000_000i64.to_le_bytes());
    assert_eq!(sys(&mut k, nr::SETITIMER, &[0, TS, 0]), Err(Errno::EINVAL), "usec out of range");
    // The new value is in place even when the old one cannot be written.
    let new = itimerval(&mut k, TS, 3_000_000, 0);
    assert_eq!(sys(&mut k, nr::SETITIMER, &[0, new, UNMAPPED]), Err(Errno::EFAULT));
    sys(&mut k, nr::GETITIMER, &[0, OUT]).unwrap();
    assert_eq!(read_itimerval(&k, OUT).value.sec, 3);
}

#[test]
fn cpu_time_interval_timers() {
    let mut k = FakeKernel::new();
    let new = itimerval(&mut k, TS, 100_000, 0);
    assert_eq!(sys(&mut k, nr::SETITIMER, &[ITIMER_VIRTUAL as u64, new, 0]), Err(Errno::EINVAL), "not accounted");
    k.cpu = Some([CpuTimes::default(), CpuTimes { user_ns: SEC, system_ns: SEC }, CpuTimes::default()]);
    assert_eq!(sys(&mut k, nr::SETITIMER, &[ITIMER_VIRTUAL as u64, new, 0]), Ok(0));
    assert_eq!(k.timer_wakeups.last(), Some(&TimerWakeup { monotonic: None, cpu: true }));
    assert_eq!(sys(&mut k, nr::SETITIMER, &[ITIMER_PROF as u64, new, 0]), Ok(0));
    k.cpu.as_mut().unwrap()[1].user_ns += 40_000_000;
    sys(&mut k, nr::GETITIMER, &[ITIMER_VIRTUAL as u64, OUT]).unwrap();
    assert_eq!(read_itimerval(&k, OUT).value, Timeval { sec: 0, usec: 60_000 });
    // Firing: ITIMER_VIRTUAL counts user time, ITIMER_PROF both.
    let p = k.me();
    let fired = fire(p, &|c| Some(if c == TimerClock::ProcessUser { SEC + 100_000_000 } else { 0 }), false);
    assert_eq!(fired.expired, 1);
    assert!(p.pending(100).contains(SIGVTALRM) && !p.pending(100).contains(SIGPROF));
    let fired = fire(p, &|c| Some(if c == TimerClock::ProcessCpu { 2 * SEC + 100_000_000 } else { 0 }), false);
    assert_eq!(fired.expired, 1);
    assert!(p.pending(100).contains(SIGPROF));
    assert_eq!(fired.next, TimerWakeup::default());
}

fn always(now: u64) -> impl Fn(TimerClock) -> Option<u64> {
    move |_| Some(now)
}

#[test]
fn itimer_real_fires_sigalrm_and_rearms() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGALRM, 0);
    let now = mono(&k);
    let new = itimerval(&mut k, TS, 1_000_000, 500_000);
    sys(&mut k, nr::SETITIMER, &[0, new, 0]).unwrap();
    let p = k.me();
    let fired = fire(p, &always(now + SEC - 1), false);
    assert_eq!(fired.expired, 0, "not yet");
    assert_eq!(fired.next.monotonic, Some(now + SEC));
    let fired = fire(p, &always(now + SEC), false);
    assert_eq!((fired.expired, fired.notify.as_slice()), (1, &[100][..]));
    assert_eq!(fired.next.monotonic, Some(now + SEC + SEC / 2), "periodic");
    let info = p.dequeue(100).unwrap();
    assert_eq!((info.signal(), info.code), (SIGALRM, SI_KERNEL));
    // Late by more than a period: one signal, the next deadline after now.
    let fired = fire(p, &always(now + 3 * SEC + 1), false);
    assert_eq!(fired.expired, 1);
    assert_eq!(fired.next.monotonic, Some(now + 3 * SEC + SEC / 2));
    // A one-shot alarm disarms itself.
    let new = itimerval(&mut k, TS, 1_000_000, 0);
    sys(&mut k, nr::SETITIMER, &[0, new, 0]).unwrap();
    let fired = fire(k.me(), &always(now + 10 * SEC), false);
    assert_eq!((fired.expired, fired.next), (1, TimerWakeup::default()));
}

#[test]
fn timer_signals_obey_the_dispositions() {
    let mut k = FakeKernel::new();
    let now = mono(&k);
    let new = itimerval(&mut k, TS, 1_000, 0);
    sys(&mut k, nr::SETITIMER, &[0, new, 0]).unwrap();
    // SIGALRM's default action kills: every thread is told.
    k.add_thread(101);
    let fired = fire(k.me(), &always(now + SEC), false);
    assert_eq!(fired.notify, [100, 101]);
    // Ignored: nothing is queued.
    let mut k = FakeKernel::new();
    k.me().set_action(SIGALRM, SigAction { handler: SIG_IGN, ..SigAction::default() });
    let new = itimerval(&mut k, TS, 1_000, 0);
    sys(&mut k, nr::SETITIMER, &[0, new, 0]).unwrap();
    let fired = fire(k.me(), &always(now + SEC), false);
    assert_eq!((fired.expired, fired.notify.len()), (1, 0));
    assert!(k.me().pending(100).is_empty());
    // Process 1 only gets handled signals.
    let mut k = FakeKernel::new();
    let new = itimerval(&mut k, TS, 1_000, 0);
    sys(&mut k, nr::SETITIMER, &[0, new, 0]).unwrap();
    fire(k.me(), &always(now + SEC), true);
    assert!(k.me().pending(100).is_empty());
}

// ---- POSIX timers -----------------------------------------------------------

fn sigevent(k: &mut FakeKernel, notify: i32, signo: i32, value: u64, tid: i32) -> u64 {
    k.poke(EV, &Sigevent { value, signo, notify, tid }.to_bytes());
    EV
}

fn itimerspec(k: &mut FakeKernel, at: u64, value: u64, interval: u64) -> u64 {
    k.poke(at, &Itimerspec { interval: Timespec::from_nanos(interval), value: Timespec::from_nanos(value) }.to_bytes());
    at
}

fn read_itimerspec(k: &FakeKernel, at: u64) -> Itimerspec {
    Itimerspec::from_bytes(&k.peek(at, 32).try_into().unwrap())
}

fn create(k: &mut FakeKernel, clock: i32, sevp: u64) -> Result<i32, Errno> {
    sys(k, nr::TIMER_CREATE, &[clock as u64, sevp, OUT2])?;
    Ok(k.peek_u32(OUT2) as i32)
}

#[test]
fn timer_create_ids_and_notifications() {
    let mut k = FakeKernel::new();
    assert_eq!(create(&mut k, id::CLOCK_MONOTONIC, 0), Ok(0));
    let ev = sigevent(&mut k, SIGEV_SIGNAL, SIGRTMIN as i32, 0xfeed, 0);
    assert_eq!(create(&mut k, id::CLOCK_REALTIME, ev), Ok(1));
    let ev = sigevent(&mut k, SIGEV_NONE, 0, 0, 0);
    assert_eq!(create(&mut k, id::CLOCK_BOOTTIME, ev), Ok(2));
    k.add_thread(101);
    let ev = sigevent(&mut k, SIGEV_THREAD_ID, SIGUSR1 as i32, 7, 101);
    assert_eq!(create(&mut k, id::CLOCK_MONOTONIC, ev), Ok(3));
    let p = k.me();
    let notify = |p: &LinuxProcess, timer| p.timers.get(timer).unwrap().notify;
    assert_eq!(notify(p, 0), Notify::Signal { signo: SIGALRM, value: 0, thread: None }, "the default");
    assert_eq!(notify(p, 1), Notify::Signal { signo: SIGRTMIN, value: 0xfeed, thread: None });
    assert_eq!(notify(p, 2), Notify::None);
    assert_eq!(notify(p, 3), Notify::Signal { signo: SIGUSR1, value: 7, thread: Some(101) });
    assert!(p.timers.get(1).unwrap().realtime && !p.timers.get(0).unwrap().realtime);
    // A deleted id is used again.
    assert_eq!(sys(&mut k, nr::TIMER_DELETE, &[1]), Ok(0));
    assert_eq!(create(&mut k, id::CLOCK_MONOTONIC, 0), Ok(1));
    assert_eq!(sys(&mut k, nr::TIMER_DELETE, &[1]), Ok(0));
    assert_eq!(sys(&mut k, nr::TIMER_DELETE, &[1]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::TIMER_DELETE, &[u64::MAX]), Err(Errno::EINVAL));
    // Errors.
    for clock in [id::CLOCK_MONOTONIC_RAW, id::CLOCK_REALTIME_COARSE, 8, 11, 99, -2] {
        assert_eq!(create(&mut k, clock, 0), Err(Errno::EINVAL), "clock {clock}");
    }
    assert_eq!(create(&mut k, id::CLOCK_PROCESS_CPUTIME_ID, 0), Err(Errno::EINVAL), "CPU time not accounted");
    for (notify, signo, tid) in [(SIGEV_THREAD, SIGALRM as i32, 0), (3, 10, 0), (SIGEV_SIGNAL, 0, 0), (0, 65, 0)] {
        let ev = sigevent(&mut k, notify, signo, 0, tid);
        assert_eq!(create(&mut k, 1, ev), Err(Errno::EINVAL), "notify {notify} signo {signo}");
    }
    let ev = sigevent(&mut k, SIGEV_THREAD_ID, 10, 0, 555);
    assert_eq!(create(&mut k, 1, ev), Err(Errno::EINVAL), "not a thread of the process");
    assert_eq!(sys(&mut k, nr::TIMER_CREATE, &[1, UNMAPPED, OUT2]), Err(Errno::EFAULT));
    // The id cannot be written: no timer is left behind.
    assert_eq!(sys(&mut k, nr::TIMER_CREATE, &[1, 0, UNMAPPED]), Err(Errno::EFAULT));
    assert_eq!(create(&mut k, id::CLOCK_MONOTONIC, 0), Ok(1));
    // CPU-time clocks, where accounted; the thread clock is the caller's.
    k.cpu = Some([CpuTimes::default(); 3]);
    let t = create(&mut k, id::CLOCK_THREAD_CPUTIME_ID, 0).unwrap();
    assert_eq!(k.me().timers.get(t).unwrap().clock, TimerClock::ThreadCpu(100));
    let t = create(&mut k, id::CLOCK_PROCESS_CPUTIME_ID, 0).unwrap();
    assert_eq!(k.me().timers.get(t).unwrap().clock, TimerClock::ProcessCpu);
}

#[test]
fn timer_creation_is_limited() {
    let mut k = FakeKernel::new();
    for i in 0..MAX_POSIX_TIMERS {
        assert_eq!(k.me().timers.create(TimerClock::Monotonic, None), Ok(i as i32));
    }
    assert_eq!(create(&mut k, id::CLOCK_MONOTONIC, 0), Err(Errno::EAGAIN));
}

#[test]
fn timer_settime_and_gettime() {
    let mut k = FakeKernel::new();
    let now = mono(&k);
    let t = create(&mut k, id::CLOCK_MONOTONIC, 0).unwrap() as u64;
    let new = itimerspec(&mut k, TS, 2 * SEC, SEC / 10);
    assert_eq!(sys(&mut k, nr::TIMER_SETTIME, &[t, 0, new, OUT]), Ok(0));
    assert_eq!(read_itimerspec(&k, OUT), Itimerspec::default());
    assert_eq!(k.timer_wakeups.last().unwrap().monotonic, Some(now + 2 * SEC));
    set_mono(&mut k, now + SEC / 2);
    assert_eq!(sys(&mut k, nr::TIMER_GETTIME, &[t, OUT]), Ok(0));
    let got = read_itimerspec(&k, OUT);
    assert_eq!((timespec_ns(got.value), timespec_ns(got.interval)), (3 * SEC / 2, SEC / 10));
    // Absolute on the monotonic clock.
    let new = itimerspec(&mut k, TS, now + 5 * SEC, 0);
    sys(&mut k, nr::TIMER_SETTIME, &[t, TIMER_ABSTIME as u64, new, OUT]).unwrap();
    assert_eq!(timespec_ns(read_itimerspec(&k, OUT).value), 3 * SEC / 2, "the old setting");
    assert_eq!(k.timer_wakeups.last().unwrap().monotonic, Some(now + 5 * SEC));
    // Disarm.
    let zero = itimerspec(&mut k, TS, 0, SEC);
    sys(&mut k, nr::TIMER_SETTIME, &[t, 0, zero, 0]).unwrap();
    sys(&mut k, nr::TIMER_GETTIME, &[t, OUT]).unwrap();
    assert_eq!(read_itimerspec(&k, OUT), Itimerspec::default());
    assert_eq!(k.timer_wakeups.last(), Some(&TimerWakeup::default()));
    // A CLOCK_REALTIME timer: an absolute time is a wall-clock time.
    let rt = create(&mut k, id::CLOCK_REALTIME, 0).unwrap() as u64;
    let wall = timespec_ns(k.realtime);
    let new = itimerspec(&mut k, TS, wall + 3 * SEC, 0);
    sys(&mut k, nr::TIMER_SETTIME, &[rt, TIMER_ABSTIME as u64, new, 0]).unwrap();
    assert_eq!(k.timer_wakeups.last().unwrap().monotonic, Some(now + SEC / 2 + 3 * SEC));
    // In the past: due at once.
    let new = itimerspec(&mut k, TS, wall - SEC, 0);
    sys(&mut k, nr::TIMER_SETTIME, &[rt, TIMER_ABSTIME as u64, new, 0]).unwrap();
    assert_eq!(k.timer_wakeups.last().unwrap().monotonic, Some(now + SEC / 2));
    // Errors.
    assert_eq!(sys(&mut k, nr::TIMER_SETTIME, &[9, 0, new, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::TIMER_GETTIME, &[9, OUT]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::TIMER_SETTIME, &[t, 0, UNMAPPED, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::TIMER_GETTIME, &[t, UNMAPPED]), Err(Errno::EFAULT));
    put_ts(&mut k, TS + 16, 0, 1_000_000_000);
    assert_eq!(sys(&mut k, nr::TIMER_SETTIME, &[t, 0, TS, 0]), Err(Errno::EINVAL));
    put_ts(&mut k, TS + 16, 1, 0);
    put_ts(&mut k, TS, -1, 0);
    assert_eq!(sys(&mut k, nr::TIMER_SETTIME, &[t, 0, TS, 0]), Err(Errno::EINVAL), "a bad interval");
}

/// The timer id, overrun and value of a timer's siginfo.
fn timer_fields(info: &SigInfo) -> (i32, u32, u64) {
    let int = |o: usize| i32::from_le_bytes(info.fields[o..o + 4].try_into().unwrap());
    (int(0), int(4) as u32, info.value())
}

#[test]
fn posix_timers_fire_and_count_overruns() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGRTMIN, SA_SIGINFO);
    let now = mono(&k);
    let ev = sigevent(&mut k, SIGEV_SIGNAL, SIGRTMIN as i32, 0x5151, 0);
    let t = create(&mut k, 1, ev).unwrap();
    let new = itimerspec(&mut k, TS, SEC, SEC);
    sys(&mut k, nr::TIMER_SETTIME, &[t as u64, 0, new, 0]).unwrap();
    let p = k.me();
    // Due: one signal, overrun 0.
    let fired = fire(p, &always(now + SEC), false);
    assert_eq!((fired.expired, fired.notify.as_slice()), (1, &[100][..]));
    assert_eq!(fired.next.monotonic, Some(now + 2 * SEC));
    // Two more expirations, then 2.5 periods late, while the signal is
    // still pending: no second signal, the overrun grows.
    fire(p, &always(now + 2 * SEC), false);
    fire(p, &always(now + 5 * SEC + SEC / 2), false);
    assert_eq!(p.shared.len(), 1, "queued once");
    assert_eq!(timers::overrun(p, t), Some(0), "the previous notification's count while one is pending");
    let info = p.dequeue(100).unwrap();
    assert_eq!((info.signal(), info.code), (SIGRTMIN, SI_TIMER));
    assert_eq!(timer_fields(&info), (t, 4, 0x5151));
    assert_eq!(timers::overrun(p, t), Some(4), "delivered: its count");
    // Due again: a new signal with overrun 0.
    fire(p, &always(now + 6 * SEC), false);
    assert_eq!(timer_fields(&p.dequeue(100).unwrap()), (t, 0, 0x5151));
    assert_eq!(sys(&mut k, nr::TIMER_GETOVERRUN, &[t as u64]), Ok(0));
    assert_eq!(sys(&mut k, nr::TIMER_GETOVERRUN, &[77]), Err(Errno::EINVAL));
    // Late by several periods with nothing pending: overrun at once.
    fire(k.me(), &always(now + 9 * SEC), false);
    assert_eq!(timer_fields(&k.me().dequeue(100).unwrap()).1, 2);
    assert_eq!(sys(&mut k, nr::TIMER_GETOVERRUN, &[t as u64]), Ok(2));
}

#[test]
fn posix_timer_notifications_go_where_asked() {
    let mut k = FakeKernel::new();
    install(&mut k, SIGUSR1, 0);
    install(&mut k, SIGUSR2, 0);
    let now = mono(&k);
    k.add_thread(101);
    let ev = sigevent(&mut k, SIGEV_THREAD_ID, SIGUSR1 as i32, 1, 101);
    let to_thread = create(&mut k, 1, ev).unwrap();
    let ev = sigevent(&mut k, SIGEV_NONE, 0, 0, 0);
    let silent = create(&mut k, 1, ev).unwrap();
    for t in [to_thread, silent] {
        let new = itimerspec(&mut k, TS, SEC, 0);
        sys(&mut k, nr::TIMER_SETTIME, &[t as u64, 0, new, 0]).unwrap();
    }
    let p = k.me();
    let fired = fire(p, &always(now + SEC), false);
    assert_eq!((fired.expired, fired.notify.as_slice()), (2, &[101][..]));
    assert!(p.thread(101).pending.contains(SIGUSR1));
    assert!(p.shared.is_empty() && p.thread(100).pending.is_empty(), "SIGEV_NONE sends nothing");
    // A target thread that has gone: the process gets it.
    p.remove_thread(101);
    let new = itimerspec(&mut k, TS, 2 * SEC, 0);
    sys(&mut k, nr::TIMER_SETTIME, &[to_thread as u64, 0, new, 0]).unwrap();
    fire(k.me(), &always(now + 2 * SEC), false);
    assert!(k.me().shared.contains(SIGUSR1));
    // SIGEV_NONE: timer_gettime shows it disarmed after its one shot.
    sys(&mut k, nr::TIMER_GETTIME, &[silent as u64, OUT]).unwrap();
    assert_eq!(read_itimerspec(&k, OUT), Itimerspec::default());
}

#[test]
fn thread_cpu_timers_wait_for_their_clock() {
    let mut p = LinuxProcess::new(100);
    let t = p.timers.create(TimerClock::ThreadCpu(100), Some(Notify::None)).unwrap();
    p.timers.get_mut(t).unwrap().arm(Some(SEC), 0);
    assert_eq!(p.timers.wakeup(), TimerWakeup { monotonic: None, cpu: true });
    // The thread's clock is gone: nothing happens.
    assert_eq!(fire(&mut p, &|_| None, false).expired, 0);
    let clocks = |c| match c {
        TimerClock::ThreadCpu(100) => Some(SEC),
        _ => Some(0),
    };
    assert_eq!(fire(&mut p, &clocks, false).expired, 1);
}

#[test]
fn timers_across_fork_and_exec() {
    let mut k = FakeKernel::new();
    let new = itimerval(&mut k, TS, 1_000_000, 0);
    sys(&mut k, nr::SETITIMER, &[0, new, 0]).unwrap();
    create(&mut k, 1, 0).unwrap();
    let child = k.me().fork(100, 200);
    assert_eq!(child.timers.wakeup(), TimerWakeup::default(), "fork(2): timers are not inherited");
    assert_eq!(child.timers.get(0), None);
    let p = k.me();
    p.exec(100);
    assert_eq!(p.timers.get(0), None, "POSIX timers are deleted by execve");
    assert!(p.timers.wakeup().monotonic.is_some(), "interval timers survive it");
}

#[test]
fn queued_timer_siginfo_layout() {
    let info = timers::timer_siginfo(SIGRTMIN + 2, 7, 3, 0xabcd);
    let b = info.to_bytes();
    assert_eq!(&b[8..12], &SI_TIMER.to_le_bytes(), "si_code");
    assert_eq!(&b[16..20], &7i32.to_le_bytes(), "si_tid (si_timerid)");
    assert_eq!(&b[20..24], &3i32.to_le_bytes(), "si_overrun");
    assert_eq!(&b[24..32], &0xabcdu64.to_le_bytes(), "si_value");
    let mut p = LinuxProcess::new(1);
    p.thread_mut(1).mask = SigSet::FULL.without(SigSet::UNBLOCKABLE);
    p.send(info, Dest::Process, false).unwrap();
    assert!(p.pending(1).contains(SIGRTMIN + 2));
}

// ---- process times ------------------------------------------------------------

#[test]
fn times_and_getrusage() {
    let mut k = FakeKernel::new();
    let thread = CpuTimes { user_ns: SEC / 2, system_ns: 30_000_000 };
    let process = CpuTimes { user_ns: 3 * SEC + 990_000_000, system_ns: SEC / 4 };
    let children = CpuTimes { user_ns: 20_000_000, system_ns: 10_000_000 };
    k.cpu = Some([thread, process, children]);
    // 42.999999999 s of monotonic time: 4299 ticks of 10 ms.
    assert_eq!(sys(&mut k, nr::TIMES, &[OUT]), Ok(4299));
    let tms: Vec<u64> = (0..4).map(|i| k.peek_u64(OUT + 8 * i)).collect();
    assert_eq!(tms, [399, 25, 2, 1]);
    assert_eq!(sys(&mut k, nr::TIMES, &[0]), Ok(4299), "buf may be null");
    assert_eq!(sys(&mut k, nr::TIMES, &[UNMAPPED]), Err(Errno::EFAULT));

    let usage = |k: &mut FakeKernel, who: i32| {
        sys(k, nr::GETRUSAGE, &[who as u64, OUT]).map(|_| (tv_at(k, OUT), tv_at(k, OUT + 16)))
    };
    let tv = |sec, usec| Timeval { sec, usec };
    assert_eq!(usage(&mut k, RUSAGE_SELF), Ok((tv(3, 990_000), tv(0, 250_000))));
    assert_eq!(usage(&mut k, RUSAGE_THREAD), Ok((tv(0, 500_000), tv(0, 30_000))));
    assert_eq!(usage(&mut k, RUSAGE_CHILDREN), Ok((tv(0, 20_000), tv(0, 10_000))));
    assert_eq!(k.peek(OUT + 32, 112), [0; 112], "the counters are 0");
    assert_eq!(usage(&mut k, 2), Err(Errno::EINVAL));
    assert_eq!(usage(&mut k, -2), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::GETRUSAGE, &[0, UNMAPPED]), Err(Errno::EFAULT));
    // Without accounting: zeros.
    k.cpu = None;
    assert_eq!(usage(&mut k, RUSAGE_SELF), Ok((tv(0, 0), tv(0, 0))));
}

#[test]
fn getcpu_reports_cpu_and_node_zero() {
    let mut k = FakeKernel::new();
    k.poke(OUT, &[0xff; 8]);
    assert_eq!(sys(&mut k, nr::GETCPU, &[OUT, OUT + 4, 0]), Ok(0));
    assert_eq!(k.peek(OUT, 8), [0; 8]);
    assert_eq!(sys(&mut k, nr::GETCPU, &[0, 0, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::GETCPU, &[UNMAPPED, 0, 0]), Err(Errno::EFAULT));
}

#[test]
fn every_time_call_is_dispatched() {
    // The table routes each of them to a handler, by number.
    for &(number, _) in crate::sys::time::CALLS {
        let mut k = FakeKernel::new();
        let (outcome, _) = call(&mut k, number, &[u64::MAX, 0, 0, 0, 0, 0]);
        assert_eq!(outcome, poly_abi::Outcome::Handled);
    }
    let _ = Linux::dispatch_with;
}
