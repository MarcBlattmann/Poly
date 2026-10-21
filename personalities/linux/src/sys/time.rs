//! Time (ADR 0044): clock_getres(2), clock_settime(2), gettimeofday(2),
//! settimeofday(2), time(2), nanosleep(2), clock_nanosleep(2), alarm(2),
//! getitimer(2) / setitimer(2), timer_create(2), timer_settime(2),
//! timer_gettime(2), timer_getoverrun(2), timer_delete(2), times(2),
//! getrusage(2) and getcpu(2). clock_gettime(2) is in
//! [`crate::sys::system`].
//!
//! The clocks: `CLOCK_REALTIME` is the settable wall clock;
//! `CLOCK_MONOTONIC`, `CLOCK_MONOTONIC_RAW` and `CLOCK_BOOTTIME` are one
//! monotonic clock (Poly never slews it and never suspends); the coarse
//! clocks read the same values but report the tick as their resolution;
//! the CPU-time clocks count what the kernel accounts per thread
//! ([`LinuxKernel::cpu_times`]).
//!
//! Sleeps block with a deadline ([`LinuxKernel::block`]). A relative
//! sleep counts on the monotonic clock whatever clock it names
//! (clock_nanosleep(2): setting the wall clock does not change a relative
//! sleep) and an absolute `CLOCK_REALTIME` sleep checks the wall clock
//! again when it wakes, so setting the clock back makes it sleep longer.
//! Interrupted by a signal handler, both fail with `EINTR` (signal(7)
//! lists them as never restarted after a handler) and a relative one
//! reports the time left; after a stop and `SIGCONT` (no handler) they go
//! on: a relative one through restart_syscall(2) with the deadline it
//! had, an absolute one by running again.
//!
//! The order of the error checks is not documented; where it matters the
//! handler says what Poly does, to be confirmed by the diff runner.

use crate::abi::signal::NSIG;
use crate::abi::{Timespec, clock as id};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::{Clock, CpuTimesOf, Deadline, LinuxKernel, UserPtr, Wakeup};
use crate::signal::deliver::with_own;
use crate::signal::restart::RestartBlock;
use crate::sys::system::clock_for;
use crate::table::{Handler, nr};
use crate::time::abi::*;
use crate::time::timers::{self, Notify, TimerClock};

/// The time system calls, registered with the others in
/// [`crate::table`].
pub const CALLS: &[(u64, Handler)] = &[
    (nr::NANOSLEEP, nanosleep),
    (nr::GETITIMER, getitimer),
    (nr::ALARM, alarm),
    (nr::SETITIMER, setitimer),
    (nr::GETTIMEOFDAY, gettimeofday),
    (nr::GETRUSAGE, getrusage),
    (nr::TIMES, times),
    (nr::SETTIMEOFDAY, settimeofday),
    (nr::TIME, time),
    (nr::TIMER_CREATE, timer_create),
    (nr::TIMER_SETTIME, timer_settime),
    (nr::TIMER_GETTIME, timer_gettime),
    (nr::TIMER_GETOVERRUN, timer_getoverrun),
    (nr::TIMER_DELETE, timer_delete),
    (nr::CLOCK_SETTIME, clock_settime),
    (nr::CLOCK_GETRES, clock_getres),
    (nr::CLOCK_NANOSLEEP, clock_nanosleep),
    (nr::GETCPU, getcpu),
];

/// The widest of `tz_minuteswest` settimeofday accepts: 15 hours either
/// way (Poly's bound; to be confirmed by the diff runner).
pub const MAX_MINUTESWEST: i32 = 15 * 60;

fn read<const N: usize>(k: &mut dyn LinuxKernel, at: UserPtr) -> Result<[u8; N], Errno> {
    let mut b = [0; N];
    k.copy_from_user(at, &mut b)?;
    Ok(b)
}

fn read_timespec(k: &mut dyn LinuxKernel, at: UserPtr) -> Result<Timespec, Errno> {
    Ok(Timespec::from_bytes(&read(k, at)?))
}

fn now_ns(k: &dyn LinuxKernel, clock: Clock) -> Result<u64, Errno> {
    Ok(timespec_ns(k.clock(clock)?))
}

/// The value of a timer's clock now (`EINVAL` for a CPU clock the kernel
/// does not account).
fn timer_clock_now(k: &dyn LinuxKernel, clock: TimerClock) -> Result<u64, Errno> {
    let cpu = |of| k.cpu_times(of).ok_or(Errno::EINVAL);
    match clock {
        TimerClock::Monotonic => now_ns(k, Clock::Monotonic),
        TimerClock::ProcessCpu => cpu(CpuTimesOf::Process).map(|t| t.total_ns()),
        TimerClock::ProcessUser => cpu(CpuTimesOf::Process).map(|t| t.user_ns),
        TimerClock::ThreadCpu(tid) => cpu(CpuTimesOf::Thread(tid)).map(|t| t.total_ns()),
    }
}

/// Tell the kernel when to look at the calling process's timers.
fn timers_changed(k: &mut dyn LinuxKernel) {
    let next = with_own(k, |p| p.timers.wakeup());
    k.timers_changed(next);
}

/// `CAP_SYS_TIME` in the calling thread's effective set (ADR 0045).
fn may_set_time(k: &mut dyn LinuxKernel) -> bool {
    // The clocks are the machine's (ADR 0054).
    crate::cred::current(k).capable_initial(crate::cred::cap::SYS_TIME)
}

/// clock_getres(clockid, res): the resolution of a clock (`res` may be
/// null). 1 ns for the precise and the CPU-time clocks, the tick for the
/// coarse ones. `EINVAL` for an unknown clock, `EFAULT`.
pub fn clock_getres(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let clockid = a.int(0);
    if let Some(cpu) = crate::time::cpuclock::decode(k, clockid) {
        // A CPU-time clock named by id: 1 ns, like the caller's own.
        cpu?;
    } else {
        let clock = clock_for(clockid).ok_or(Errno::EINVAL)?;
        if matches!(clock, Clock::ProcessCpuTime | Clock::ThreadCpuTime) && k.cpu_times(CpuTimesOf::Process).is_none()
        {
            return Err(Errno::EINVAL);
        }
    }
    let res = match clockid {
        id::CLOCK_REALTIME_COARSE | id::CLOCK_MONOTONIC_COARSE => k.tick_ns(),
        _ => 1,
    };
    if !a.ptr(1).is_null() {
        k.copy_to_user(a.ptr(1), &Timespec::from_nanos(res).to_bytes())?;
    }
    Ok(0)
}

/// clock_settime(clockid, tp): set the wall clock. clock_settime(2):
/// `EINVAL` for a clock that does not exist or cannot be set (every clock
/// but `CLOCK_REALTIME`), a `tp` out of range, or a time below the
/// monotonic clock; `EPERM` without privilege; `EFAULT`. Poly checks the
/// clock, copies, checks the clock is settable and the value, then the
/// privilege.
pub fn clock_settime(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let clockid = a.int(0);
    clock_for(clockid).ok_or(Errno::EINVAL)?;
    let t = read_timespec(k, a.ptr(1))?;
    if clockid != id::CLOCK_REALTIME || !t.is_valid() {
        return Err(Errno::EINVAL);
    }
    if !may_set_time(k) {
        return Err(Errno::EPERM);
    }
    k.set_realtime(t)?;
    Ok(0)
}

/// gettimeofday(tv, tz): the wall clock in microseconds and the stored
/// time zone; either pointer may be null. `EFAULT`.
pub fn gettimeofday(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if !a.ptr(0).is_null() {
        let now = k.clock(Clock::Realtime)?;
        let tv = Timeval { sec: now.sec, usec: now.nsec / 1000 };
        k.copy_to_user(a.ptr(0), &tv.to_bytes())?;
    }
    if !a.ptr(1).is_null() {
        let (minuteswest, dsttime) = k.timezone();
        k.copy_to_user(a.ptr(1), &Timezone { minuteswest, dsttime }.to_bytes())?;
    }
    Ok(0)
}

/// settimeofday(tv, tz): set the wall clock and/or the time zone (either
/// may be null: settimeofday(2), "If either tv or tz is NULL, the
/// corresponding structure is not set"). `EFAULT`; `EINVAL` for a `tv`
/// out of range (negative seconds, microseconds outside 0-999999), a time
/// below the monotonic clock, or a time zone beyond
/// [`MAX_MINUTESWEST`]; `EPERM` without privilege. Poly copies both,
/// checks both, then the privilege.
pub fn settimeofday(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let tv = match a.ptr(0).is_null() {
        true => None,
        false => Some(Timeval::from_bytes(&read(k, a.ptr(0))?)),
    };
    let tz = match a.ptr(1).is_null() {
        true => None,
        false => Some(Timezone::from_bytes(&read(k, a.ptr(1))?)),
    };
    if tv.is_some_and(|t| !t.is_valid()) || tz.is_some_and(|z| z.minuteswest.abs() > MAX_MINUTESWEST) {
        return Err(Errno::EINVAL);
    }
    if (tv.is_some() || tz.is_some()) && !may_set_time(k) {
        return Err(Errno::EPERM);
    }
    if let Some(tv) = tv {
        k.set_realtime(Timespec { sec: tv.sec, nsec: tv.usec * 1000 })?;
    }
    if let Some(tz) = tz {
        k.set_timezone(tz.minuteswest, tz.dsttime);
    }
    Ok(0)
}

/// time(tloc): seconds since the epoch, also stored at `tloc` if it is not
/// null (`EFAULT`).
pub fn time(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let sec = k.clock(Clock::Realtime)?.sec;
    if !a.ptr(0).is_null() {
        k.copy_to_user(a.ptr(0), &sec.to_le_bytes())?;
    }
    Ok(sec as u64)
}

/// Block until `deadline`. `rem` is set for a relative sleep: where to
/// report the time left (if not null) when a signal interrupts it.
pub fn sleep(k: &mut dyn LinuxKernel, deadline: Deadline, rem: Option<UserPtr>) -> SysResult {
    loop {
        match k.block(Some(deadline)) {
            Wakeup::TimedOut => {
                // The wall clock may have been set back meanwhile.
                if deadline.clock == Clock::Realtime && now_ns(k, Clock::Realtime)? < timespec_ns(deadline.at) {
                    continue;
                }
                return Ok(0);
            }
            Wakeup::Woken => {}
            Wakeup::Signal => {
                let Some(rem) = rem else { return Err(Errno::RESTART_IF_NO_HANDLER) };
                if !rem.is_null() {
                    let left = timespec_ns(deadline.at).saturating_sub(now_ns(k, deadline.clock)?);
                    k.copy_to_user(rem, &Timespec::from_nanos(left).to_bytes())?;
                }
                let tid = k.tid();
                let block = RestartBlock::Sleep { deadline, rem: rem.0 };
                with_own(k, |p| p.thread_mut(tid).restart_block = Some(block));
                return Err(Errno::RESTART_WITH_BLOCK);
            }
        }
    }
}

/// nanosleep(req, rem): sleep at least `req` on the monotonic clock
/// (nanosleep(2): "Linux measures the time using the CLOCK_MONOTONIC
/// clock"). `EFAULT`; `EINVAL` for a `req` out of range; `EINTR` with the
/// time left in `rem` (if not null) when a handler interrupts it.
pub fn nanosleep(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let req = read_timespec(k, a.ptr(0))?;
    if !req.is_valid() {
        return Err(Errno::EINVAL);
    }
    let at = k.clock(Clock::Monotonic)?.saturating_add(req);
    sleep(k, Deadline { clock: Clock::Monotonic, at }, Some(a.ptr(1)))
}

/// clock_nanosleep(clockid, flags, request, remain): sleep on
/// `CLOCK_REALTIME`, `CLOCK_MONOTONIC` or `CLOCK_BOOTTIME`, until an
/// absolute time with `TIMER_ABSTIME` (then `remain` is unused), else for
/// a relative time. clock_nanosleep(2): `EINVAL` for `CLOCK_THREAD_CPUTIME_ID`
/// and unknown clocks, `ENOTSUP` for a clock it cannot sleep on (Poly:
/// `CLOCK_PROCESS_CPUTIME_ID` and the coarse and raw clocks: to be
/// confirmed by the diff runner); `EINVAL` for a `request` out of range,
/// `EFAULT`, `EINTR`. Other `flags` bits are ignored.
pub fn clock_nanosleep(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    use crate::time::cpuclock::{Counts, CpuClock, decode};
    let cpu = |of| CpuClock { of, counts: Counts::All };
    let clockid = a.int(0);
    let clock = match clockid {
        // CLOCK_TAI is the wall clock (its offset is 0, `clock_for`).
        id::CLOCK_REALTIME | id::CLOCK_TAI | id::CLOCK_REALTIME_ALARM => Clock::Realtime,
        id::CLOCK_MONOTONIC | id::CLOCK_BOOTTIME | id::CLOCK_BOOTTIME_ALARM => Clock::Monotonic,
        id::CLOCK_PROCESS_CPUTIME_ID => return cpu_sleep(k, a, cpu(CpuTimesOf::Process)),
        id::CLOCK_THREAD_CPUTIME_ID
        | id::CLOCK_MONOTONIC_RAW
        | id::CLOCK_REALTIME_COARSE
        | id::CLOCK_MONOTONIC_COARSE => return Err(Errno::EOPNOTSUPP),
        // Another process's CPU-time clock; a thread's cannot be slept on.
        _ => match decode(k, clockid) {
            Some(Ok(c)) if !matches!(c.of, CpuTimesOf::Thread(_)) => return cpu_sleep(k, a, c),
            _ => return Err(Errno::EINVAL),
        },
    };
    let request = read_timespec(k, a.ptr(2))?;
    if !request.is_valid() {
        return Err(Errno::EINVAL);
    }
    if a.uint(1) & TIMER_ABSTIME != 0 {
        return sleep(k, Deadline { clock, at: request }, None);
    }
    let at = k.clock(Clock::Monotonic)?.saturating_add(request);
    sleep(k, Deadline { clock: Clock::Monotonic, at }, Some(a.ptr(3)))
}

/// clock_nanosleep(2) on a CPU-time clock: until the clock reaches the
/// requested time (`TIMER_ABSTIME`) or has advanced by it, looking once a
/// tick. A handler interrupts it with `EINTR` and, for a relative sleep,
/// the CPU time left in `remain`.
fn cpu_sleep(k: &mut dyn LinuxKernel, a: &Args, clock: crate::time::cpuclock::CpuClock) -> SysResult {
    let request = read_timespec(k, a.ptr(2))?;
    if !request.is_valid() {
        return Err(Errno::EINVAL);
    }
    let absolute = a.uint(1) & TIMER_ABSTIME != 0;
    let start = clock.read_ns(k)?;
    let target = if absolute { timespec_ns(request) } else { start.saturating_add(timespec_ns(request)) };
    loop {
        let now = clock.read_ns(k)?;
        if now >= target {
            return Ok(0);
        }
        let at = k.clock(Clock::Monotonic)?.saturating_add(Timespec::from_nanos(k.tick_ns()));
        if k.block(Some(Deadline { clock: Clock::Monotonic, at })) == Wakeup::Signal {
            let rem = a.ptr(3);
            if !absolute && !rem.is_null() {
                k.copy_to_user(rem, &Timespec::from_nanos(target - now).to_bytes())?;
            }
            return Err(Errno::RESTART_IF_NO_HANDLER);
        }
    }
}

/// The clock of interval timer `which`.
fn itimer_clock(which: i32) -> Option<TimerClock> {
    match which {
        ITIMER_REAL => Some(TimerClock::Monotonic),
        ITIMER_VIRTUAL => Some(TimerClock::ProcessUser),
        ITIMER_PROF => Some(TimerClock::ProcessCpu),
        _ => None,
    }
}

fn itimerval(left: u64, interval: u64) -> Itimerval {
    Itimerval { interval: Timeval::from_nanos_up(interval), value: Timeval::from_nanos_up(left) }
}

/// alarm(seconds): `SIGALRM` after `seconds` (0 cancels), replacing the
/// `ITIMER_REAL` timer (alarm(2): "alarm() and setitimer(2) share the
/// same timer"). Returns the seconds the previous alarm had left, rounded
/// to the nearest second but at least 1 while one was pending (Poly's
/// rounding; to be confirmed by the diff runner), or 0.
pub fn alarm(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let seconds = u64::from(a.uint(0));
    let now = now_ns(k, Clock::Monotonic)?;
    let left = with_own(k, |p| {
        let t = p.timers.itimer_mut(ITIMER_REAL).expect("ITIMER_REAL exists");
        let (left, _) = t.remaining(now);
        t.arm((seconds != 0).then(|| now.saturating_add(seconds.saturating_mul(NANOS_PER_SEC))), 0);
        left
    });
    timers_changed(k);
    Ok(if left == 0 { 0 } else { (left.saturating_add(NANOS_PER_SEC / 2) / NANOS_PER_SEC).max(1) })
}

/// getitimer(which, curr_value): the time left and the interval of an
/// interval timer, the time rounded up to whole microseconds. `EINVAL`
/// for an unknown timer, `EFAULT`.
pub fn getitimer(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let which = a.int(0);
    let clock = itimer_clock(which).ok_or(Errno::EINVAL)?;
    let now = timer_clock_now(k, clock)?;
    let (left, interval) = with_own(k, |p| p.timers.itimer_mut(which).expect("valid which").remaining(now));
    k.copy_to_user(a.ptr(1), &itimerval(left, interval).to_bytes())?;
    Ok(0)
}

/// setitimer(which, new_value, old_value): arm (or with a zero value,
/// disarm) an interval timer; the old setting goes to `old_value` if it
/// is not null. setitimer(2): `EINVAL` for an unknown timer or a
/// `tv_usec` outside 0-999999 (and Poly: negative seconds), `EFAULT`
/// (also for a null `new_value`; to be confirmed by the diff runner). The
/// new setting is in place before `old_value` is written.
pub fn setitimer(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let which = a.int(0);
    let clock = itimer_clock(which).ok_or(Errno::EINVAL)?;
    let new = Itimerval::from_bytes(&read(k, a.ptr(1))?);
    if !new.value.is_valid() || !new.interval.is_valid() {
        return Err(Errno::EINVAL);
    }
    let now = timer_clock_now(k, clock)?;
    let (value, interval) = (new.value.to_nanos(), new.interval.to_nanos());
    let old = with_own(k, |p| {
        let t = p.timers.itimer_mut(which).expect("valid which");
        let old = t.remaining(now);
        t.arm((value != 0).then(|| now.saturating_add(value)), interval);
        old
    });
    timers_changed(k);
    if !a.ptr(2).is_null() {
        k.copy_to_user(a.ptr(2), &itimerval(old.0, old.1).to_bytes())?;
    }
    Ok(0)
}

/// timer_create(clockid, sevp, timerid): a POSIX timer on
/// `CLOCK_REALTIME`, `CLOCK_MONOTONIC`, `CLOCK_BOOTTIME`,
/// `CLOCK_PROCESS_CPUTIME_ID` or `CLOCK_THREAD_CPUTIME_ID` (the calling
/// thread's), notifying as `sevp` says (null: `SIGALRM` with the timer id
/// as value). timer_create(2): `EINVAL` for another clock, a
/// `sigev_notify` other than `SIGEV_NONE`, `SIGEV_SIGNAL` and
/// `SIGEV_THREAD_ID` (`SIGEV_THREAD` is the C library's), a signal number
/// out of range, or a thread that is not in the process; `EAGAIN` beyond
/// [`timers::MAX_POSIX_TIMERS`]; `EFAULT` (the timer is not kept then).
pub fn timer_create(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let clockid = a.int(0);
    let clock = match clockid {
        id::CLOCK_REALTIME
        | id::CLOCK_MONOTONIC
        | id::CLOCK_BOOTTIME
        | id::CLOCK_TAI
        | id::CLOCK_REALTIME_ALARM
        | id::CLOCK_BOOTTIME_ALARM => TimerClock::Monotonic,
        id::CLOCK_PROCESS_CPUTIME_ID => TimerClock::ProcessCpu,
        id::CLOCK_THREAD_CPUTIME_ID => TimerClock::ThreadCpu(k.tid()),
        // Clocks that exist but cannot drive a timer (timer_create(2)).
        id::CLOCK_MONOTONIC_RAW | id::CLOCK_REALTIME_COARSE | id::CLOCK_MONOTONIC_COARSE => {
            return Err(Errno::EOPNOTSUPP);
        }
        _ => return Err(Errno::EINVAL),
    };
    if clock.is_cpu() && k.cpu_times(CpuTimesOf::Process).is_none() {
        return Err(Errno::EINVAL);
    }
    let notify = if a.ptr(1).is_null() {
        None
    } else {
        let ev = Sigevent::from_bytes(&read(k, a.ptr(1))?);
        Some(match ev.notify {
            SIGEV_NONE => Notify::None,
            SIGEV_SIGNAL | SIGEV_THREAD_ID => {
                let signo = u32::try_from(ev.signo).ok().filter(|s| (1..=NSIG).contains(s)).ok_or(Errno::EINVAL)?;
                let thread = if ev.notify == SIGEV_THREAD_ID {
                    let tid = ev.tid;
                    if !with_own(k, |p| p.has_thread(tid)) {
                        return Err(Errno::EINVAL);
                    }
                    Some(tid)
                } else {
                    None
                };
                Notify::Signal { signo, value: ev.value, thread }
            }
            _ => return Err(Errno::EINVAL),
        })
    };
    let realtime = matches!(clockid, id::CLOCK_REALTIME | id::CLOCK_TAI | id::CLOCK_REALTIME_ALARM);
    let timer = with_own(k, |p| {
        let timer = p.timers.create(clock, notify)?;
        p.timers.get_mut(timer).expect("just created").realtime = realtime;
        Ok(timer)
    })?;
    if let Err(e) = k.copy_to_user(a.ptr(2), &timer.to_le_bytes()) {
        with_own(k, |p| p.timers.delete(timer));
        return Err(e);
    }
    Ok(0)
}

/// The clock of POSIX timer `timer` and whether it is a `CLOCK_REALTIME`
/// timer; `EINVAL` if there is none.
fn posix_timer(k: &mut dyn LinuxKernel, timer: i32) -> Result<(TimerClock, bool), Errno> {
    with_own(k, |p| p.timers.get(timer).map(|t| (t.clock, t.realtime))).ok_or(Errno::EINVAL)
}

fn itimerspec(left: u64, interval: u64) -> Itimerspec {
    Itimerspec { interval: Timespec::from_nanos(interval), value: Timespec::from_nanos(left) }
}

/// timer_settime(timerid, flags, new_value, old_value): arm (a zero
/// value disarms) a POSIX timer, at an absolute time on its clock with
/// `TIMER_ABSTIME` (a time already past expires at once), else relative;
/// the old setting goes to `old_value` if it is not null. timer_settime(2):
/// `EINVAL` for an unknown timer or a value out of range, `EFAULT`.
pub fn timer_settime(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let timer = a.int(0);
    let (clock, realtime) = posix_timer(k, timer)?;
    let new = Itimerspec::from_bytes(&read(k, a.ptr(2))?);
    if !new.value.is_valid() || !new.interval.is_valid() {
        return Err(Errno::EINVAL);
    }
    let now = timer_clock_now(k, clock)?;
    let value = timespec_ns(new.value);
    let deadline = if new.value.is_zero() {
        None
    } else if a.uint(1) & TIMER_ABSTIME == 0 {
        Some(now.saturating_add(value))
    } else if realtime {
        // An absolute wall-clock time, converted to the monotonic clock now.
        let wall = now_ns(k, Clock::Realtime)?;
        Some(now.saturating_add(value.saturating_sub(wall)))
    } else {
        Some(value)
    };
    let interval = timespec_ns(new.interval);
    let old = with_own(k, |p| {
        let t = p.timers.get_mut(timer)?;
        let old = t.remaining(now);
        t.arm(deadline, interval);
        Some(old)
    })
    .ok_or(Errno::EINVAL)?;
    timers_changed(k);
    if !a.ptr(3).is_null() {
        k.copy_to_user(a.ptr(3), &itimerspec(old.0, old.1).to_bytes())?;
    }
    Ok(0)
}

/// timer_gettime(timerid, curr_value): the time left and the interval.
/// `EINVAL` for an unknown timer, `EFAULT`.
pub fn timer_gettime(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let timer = a.int(0);
    let (clock, _) = posix_timer(k, timer)?;
    let now = timer_clock_now(k, clock)?;
    let (left, interval) = with_own(k, |p| p.timers.get(timer).map(|t| t.remaining(now))).ok_or(Errno::EINVAL)?;
    k.copy_to_user(a.ptr(1), &itimerspec(left, interval).to_bytes())?;
    Ok(0)
}

/// timer_getoverrun(timerid): the overrun count of the timer's last
/// expiry notification ([`timers::overrun`]). `EINVAL` for an unknown
/// timer.
pub fn timer_getoverrun(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let timer = a.int(0);
    with_own(k, |p| timers::overrun(p, timer)).map(u64::from).ok_or(Errno::EINVAL)
}

/// timer_delete(timerid). `EINVAL` for an unknown timer. A signal of the
/// timer that is still pending stays pending (POSIX leaves it
/// unspecified).
pub fn timer_delete(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let timer = a.int(0);
    if !with_own(k, |p| p.timers.delete(timer)) {
        return Err(Errno::EINVAL);
    }
    timers_changed(k);
    Ok(0)
}

/// Nanoseconds as clock ticks of times(2) ([`USER_HZ`] a second).
const fn clock_ticks(ns: u64) -> i64 {
    (ns / (NANOS_PER_SEC / USER_HZ)) as i64
}

/// times(buf): the process's and its waited-for children's CPU times in
/// clock ticks (`buf` may be null: times(2), NOTES), and the elapsed real
/// time in clock ticks since boot (the "arbitrary point in the past").
/// `EFAULT`.
pub fn times(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    if !a.ptr(0).is_null() {
        let own = k.cpu_times(CpuTimesOf::Process).unwrap_or_default();
        let children = k.cpu_times(CpuTimesOf::Children).unwrap_or_default();
        let tms = Tms {
            utime: clock_ticks(own.user_ns),
            stime: clock_ticks(own.system_ns),
            cutime: clock_ticks(children.user_ns),
            cstime: clock_ticks(children.system_ns),
        };
        k.copy_to_user(a.ptr(0), &tms.to_bytes())?;
    }
    Ok(clock_ticks(now_ns(k, Clock::Monotonic)?) as u64)
}

/// getrusage(who, usage): user and system CPU time of the process
/// (`RUSAGE_SELF`), its waited-for children (`RUSAGE_CHILDREN`) or the
/// calling thread (`RUSAGE_THREAD`); the other fields are 0 for now.
/// `EINVAL` for another `who`, `EFAULT`.
pub fn getrusage(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let of = match a.int(0) {
        RUSAGE_SELF => CpuTimesOf::Process,
        RUSAGE_CHILDREN => CpuTimesOf::Children,
        RUSAGE_THREAD => CpuTimesOf::Thread(k.tid()),
        _ => return Err(Errno::EINVAL),
    };
    let t = k.cpu_times(of).unwrap_or_default();
    let usage = Rusage { utime: Timeval::from_nanos(t.user_ns), stime: Timeval::from_nanos(t.system_ns) };
    k.copy_to_user(a.ptr(1), &usage.to_bytes())?;
    Ok(0)
}

/// getcpu(cpu, node, tcache): the CPU and NUMA node the thread runs on,
/// each stored if its pointer is not null: the kernel's CPU number, and
/// node 0 (Poly has one NUMA node). `tcache` is unused (getcpu(2): "since
/// Linux 2.6.24, this argument is ignored"). `EFAULT`.
pub fn getcpu(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let cpu = k.current_cpu();
    for (i, value) in [(0, cpu), (1, 0u32)] {
        if !a.ptr(i).is_null() {
            k.copy_to_user(a.ptr(i), &value.to_le_bytes())?;
        }
    }
    Ok(0)
}
