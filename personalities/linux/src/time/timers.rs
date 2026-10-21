//! The timers of a process: the three interval timers of setitimer(2)
//! (alarm(2) uses `ITIMER_REAL`) and POSIX timers (timer_create(2)).
//!
//! The state is part of [`LinuxProcess`] (`timers`), so it is locked with
//! the signal state it feeds. Times are nanoseconds on a [`TimerClock`]:
//! the monotonic clock, or a CPU clock. The kernel does the waiting: after
//! every change the personality tells it the process's earliest expiry
//! ([`Timers::wakeup`], `LinuxKernel::timers_changed`), and when that
//! time comes (or, for CPU clocks, as CPU time passes) the kernel calls
//! [`fire`] with the clocks' current values, under the process's lock.
//! [`fire`] queues the signals of the timers that expired, rearms the
//! periodic ones and says which threads to notify.
//!
//! - **Interval timers.** `ITIMER_REAL` counts on the monotonic clock and
//!   sends `SIGALRM`, `ITIMER_VIRTUAL` counts the process's user CPU time
//!   (`SIGVTALRM`), `ITIMER_PROF` its user and system CPU time (`SIGPROF`)
//!   (setitimer(2)). Their signals carry `si_code` `SI_KERNEL` (Poly's
//!   choice; to be confirmed by the diff runner). fork(2): "The child does
//!   not inherit timers"; execve(2) keeps them.
//! - **POSIX timers** have ids from 0 up (the lowest free one), a clock,
//!   and a notification: none (`SIGEV_NONE`), a signal to the process
//!   (`SIGEV_SIGNAL`) or to one of its threads (`SIGEV_THREAD_ID`), with
//!   `si_code` `SI_TIMER`, the timer id, the overrun count and the
//!   `sigev_value` (`asm-generic/siginfo.h` `_timer`). timer_create(2): a
//!   timer's signal is queued once; expirations while it is still pending
//!   only raise its overrun count, which the queued siginfo and
//!   timer_getoverrun(2) report. execve(2) deletes them, fork(2) does not
//!   copy them.
//! - `CLOCK_REALTIME` timers count on the monotonic clock: an absolute
//!   time is converted when the timer is armed, so setting the wall clock
//!   later does not move it (Linux moves it; to be done when something
//!   needs it).

use super::abi::{DELAYTIMER_MAX, ITIMER_PROF, ITIMER_REAL, ITIMER_VIRTUAL, SI_TIMER};
use crate::abi::SigInfo;
use crate::abi::signal::{SI_KERNEL, SIGALRM, SIGPROF, SIGVTALRM};
use crate::errno::Errno;
use crate::signal::{Dest, LinuxProcess};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// The most POSIX timers one process may have (timer_create(2): `EAGAIN`
/// "Temporary error during kernel allocation of timer structures"; the
/// limit is Poly's).
pub const MAX_POSIX_TIMERS: usize = 4096;

/// What a timer counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerClock {
    /// The monotonic clock (`CLOCK_MONOTONIC`, `CLOCK_BOOTTIME`,
    /// `CLOCK_REALTIME`, `ITIMER_REAL`).
    Monotonic,
    /// The process's user and system CPU time (`CLOCK_PROCESS_CPUTIME_ID`,
    /// `ITIMER_PROF`).
    ProcessCpu,
    /// The process's user CPU time (`ITIMER_VIRTUAL`).
    ProcessUser,
    /// The CPU time of thread `tid` (`CLOCK_THREAD_CPUTIME_ID`).
    ThreadCpu(i32),
}

impl TimerClock {
    pub const fn is_cpu(self) -> bool {
        !matches!(self, TimerClock::Monotonic)
    }
}

/// How a timer tells the process it expired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notify {
    /// `SIGEV_NONE`: not at all (timer_gettime shows the progress).
    None,
    /// A signal, to the process or (`SIGEV_THREAD_ID`) to one thread.
    Signal { signo: u32, value: u64, thread: Option<i32> },
}

/// One timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timer {
    pub clock: TimerClock,
    /// The next expiry on `clock`, in nanoseconds; `None` while disarmed.
    pub deadline: Option<u64>,
    /// Re-arm this long after each expiry (0: one shot).
    pub interval: u64,
    pub notify: Notify,
    /// A `CLOCK_REALTIME` timer: an absolute time given to timer_settime
    /// is a wall-clock time.
    pub realtime: bool,
    /// The overrun count fixed when the timer's last signal was taken
    /// (timer_getoverrun(2)); 0 after arming.
    overrun: u32,
    /// The POSIX timer's signal is pending ([`fire`] queued it). Its
    /// `deadline` then stays at the first expiry after the one that queued
    /// it, even once that is past: the expiries while the signal is pending
    /// are counted when it is taken ([`count_taken`]), not one by one.
    signal_pending: bool,
    /// The pending signal has been dequeued and its count is being fixed
    /// ([`signal_taken`], then [`count_taken`]).
    taking: bool,
    /// While the signal is pending: when [`fire`] looks at the timer again
    /// (on its clock), to notice a signal that went without being taken
    /// (discarded when its action became `SIG_IGN`, say).
    recheck: u64,
}

/// While a POSIX timer's signal is pending, [`fire`] looks at the timer
/// this often (or once an interval, if that is longer): 10 ms. The
/// expiries in between cost nothing, however short the interval.
pub const PENDING_RECHECK_NS: u64 = 10_000_000;

impl Timer {
    pub const fn new(clock: TimerClock, notify: Notify) -> Timer {
        Timer {
            clock,
            deadline: None,
            interval: 0,
            notify,
            realtime: false,
            overrun: 0,
            signal_pending: false,
            taking: false,
            recheck: 0,
        }
    }

    /// Arm the timer to expire at `deadline` (on its clock) and every
    /// `interval` after that, or disarm it (`None`; the interval is then
    /// dropped too). The overrun count starts again; a POSIX timer's
    /// pending signal is the caller's to discard ([`discard_queued`]).
    pub fn arm(&mut self, deadline: Option<u64>, interval: u64) {
        self.deadline = deadline;
        self.interval = if deadline.is_some() { interval } else { 0 };
        self.overrun = 0;
        self.signal_pending = false;
        self.taking = false;
    }

    /// `(time left, interval)` at `now`, as timer_gettime(2) reports it
    /// for a POSIX timer: the time to the next expiry that is still to come
    /// (a periodic timer whose expiries are due but not fired yet counts on
    /// from them), 0 for a one-shot timer whose time has come (an absolute
    /// time in the past "expires at once").
    pub fn left(&self, now: u64) -> (u64, u64) {
        match self.deadline {
            Some(d) if now < d => (d - now, self.interval),
            Some(d) if self.interval > 0 => (self.interval - (now - d) % self.interval, self.interval),
            Some(_) => (0, 0),
            None => (0, 0),
        }
    }

    /// `(time left, interval)` at `now`, as timer_gettime(2) and
    /// getitimer(2) report it. A timer that is due but has not been fired
    /// yet reports 1 ns left, so that an armed timer never reads as
    /// disarmed.
    pub fn remaining(&self, now: u64) -> (u64, u64) {
        match self.deadline {
            Some(d) => (d.saturating_sub(now).max(1), self.interval),
            None => (0, 0),
        }
    }

    /// The expirations due at `now` (0 if none), moving the deadline past
    /// `now` (or disarming a one-shot timer).
    fn expire(&mut self, now: u64) -> u64 {
        let Some(d) = self.deadline else { return 0 };
        if now < d {
            return 0;
        }
        if self.interval == 0 {
            self.deadline = None;
            return 1;
        }
        let n = (now - d) / self.interval + 1;
        self.deadline = Some(d.saturating_add(n.saturating_mul(self.interval)));
        n
    }
}

/// When the kernel must next look at a process's timers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimerWakeup {
    /// The earliest expiry of a timer on the monotonic clock.
    pub monotonic: Option<u64>,
    /// Some timer counts CPU time: check as the process runs.
    pub cpu: bool,
}

/// A process's timers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timers {
    itimers: [Timer; 3],
    posix: BTreeMap<i32, Timer>,
    /// Where the search for the next POSIX timer id starts: ids are handed
    /// out in increasing order, so a deleted one is not given out again at
    /// once (wrapping around after `i32::MAX`).
    next_id: i32,
}

impl Default for Timers {
    fn default() -> Self {
        Timers::new()
    }
}

/// The signal of each interval timer, by `which`.
const ITIMER_SIGNALS: [u32; 3] = [SIGALRM, SIGVTALRM, SIGPROF];

impl Timers {
    pub fn new() -> Timers {
        let itimer = |clock, which: usize| {
            Timer::new(clock, Notify::Signal { signo: ITIMER_SIGNALS[which], value: 0, thread: None })
        };
        Timers {
            itimers: [
                itimer(TimerClock::Monotonic, ITIMER_REAL as usize),
                itimer(TimerClock::ProcessUser, ITIMER_VIRTUAL as usize),
                itimer(TimerClock::ProcessCpu, ITIMER_PROF as usize),
            ],
            posix: BTreeMap::new(),
            next_id: 0,
        }
    }

    /// Interval timer `which` (`ITIMER_REAL`, `ITIMER_VIRTUAL`,
    /// `ITIMER_PROF`); `None` for any other value.
    pub fn itimer_mut(&mut self, which: i32) -> Option<&mut Timer> {
        usize::try_from(which).ok().and_then(|w| self.itimers.get_mut(w))
    }

    /// Create a POSIX timer: its id. `notify` is `None` for a null
    /// `sevp`, which timer_create(2) defines as `SIGEV_SIGNAL` with
    /// `SIGALRM` and the timer id as the value. `EAGAIN` beyond
    /// [`MAX_POSIX_TIMERS`].
    pub fn create(&mut self, clock: TimerClock, notify: Option<Notify>) -> Result<i32, Errno> {
        if self.posix.len() >= MAX_POSIX_TIMERS {
            return Err(Errno::EAGAIN);
        }
        let id = (self.next_id..=i32::MAX)
            .chain(0..self.next_id)
            .find(|id| !self.posix.contains_key(id))
            .expect("fewer timers than ids");
        self.next_id = id.checked_add(1).unwrap_or(0);
        let notify = notify.unwrap_or(Notify::Signal { signo: SIGALRM, value: id as u64, thread: None });
        self.posix.insert(id, Timer::new(clock, notify));
        Ok(id)
    }

    /// POSIX timer `id`.
    pub fn get(&self, id: i32) -> Option<&Timer> {
        self.posix.get(&id)
    }

    pub fn get_mut(&mut self, id: i32) -> Option<&mut Timer> {
        self.posix.get_mut(&id)
    }

    /// Delete POSIX timer `id`; false if there is none.
    pub fn delete(&mut self, id: i32) -> bool {
        self.posix.remove(&id).is_some()
    }

    /// The POSIX timers' ids, lowest first.
    pub fn ids(&self) -> impl Iterator<Item = i32> + '_ {
        self.posix.keys().copied()
    }

    /// execve(2): the POSIX timers go, the interval timers stay.
    pub fn exec(&mut self) {
        self.posix.clear();
        self.next_id = 0;
    }

    /// When the kernel must look at the timers next.
    pub fn wakeup(&self) -> TimerWakeup {
        let mut w = TimerWakeup::default();
        for t in self.itimers.iter().chain(self.posix.values()) {
            let Some(d) = t.deadline else { continue };
            // A timer whose signal is pending is only looked at again.
            let d = if t.signal_pending { t.recheck } else { d };
            if t.clock.is_cpu() {
                w.cpu = true;
            } else {
                w.monotonic = Some(w.monotonic.map_or(d, |m: u64| m.min(d)));
            }
        }
        w
    }
}

/// Whether `info` is the signal of POSIX timer `id`.
fn from_timer(info: &SigInfo, id: i32, signo: u32) -> bool {
    info.signal() == signo && info.code == SI_TIMER && info.fields[0..4] == id.to_le_bytes()
}

/// The siginfo of POSIX timer `id` (`_timer`: `si_tid` at offset 16,
/// `si_overrun` at 20, `si_value` at 24).
pub fn timer_siginfo(signo: u32, id: i32, overrun: u32, value: u64) -> SigInfo {
    let mut info = SigInfo::new(signo, SI_TIMER);
    info.fields[0..4].copy_from_slice(&id.to_le_bytes());
    info.fields[4..8].copy_from_slice(&overrun.to_le_bytes());
    info.fields[8..16].copy_from_slice(&value.to_le_bytes());
    info
}

/// The still-queued signal of timer `id` going to `dest`, if any.
fn queued(p: &mut LinuxProcess, dest: Dest, id: i32, signo: u32) -> Option<&mut SigInfo> {
    let queue = match dest {
        Dest::Process => &mut p.shared,
        Dest::Thread(tid) => &mut p.thread_mut(tid).pending,
    };
    queue.find_mut(|info| from_timer(info, id, signo))
}

/// timer_delete(2) of timer `id`, which sent `signo`: its signal, if one
/// is still queued, is never delivered (the spec of timer_delete).
pub fn discard_queued(p: &mut LinuxProcess, id: i32, signo: u32) {
    p.shared.remove_where(|info| from_timer(info, id, signo));
    let tids: alloc::vec::Vec<i32> = p.thread_ids().collect();
    for tid in tids {
        p.thread_mut(tid).pending.remove_where(|info| from_timer(info, id, signo));
    }
}

/// Whether timer `id`'s signal is queued anywhere in the process.
fn is_queued(p: &LinuxProcess, id: i32, signo: u32) -> bool {
    let thread_queues = p.thread_ids().map(|tid| &p.thread(tid).pending);
    core::iter::once(&p.shared).chain(thread_queues).any(|q| q.iter().any(|info| from_timer(info, id, signo)))
}

/// timer_getoverrun(2) for timer `id`: the overrun count fixed when its
/// last signal was taken (0 if none was since it was armed; it stays so
/// while the next signal is pending), at most [`DELAYTIMER_MAX`]. `None`
/// if there is no such timer.
pub fn overrun(p: &LinuxProcess, id: i32) -> Option<u32> {
    p.timers.get(id).map(|t| t.overrun)
}

/// A POSIX timer's signal that was just taken: its overrun count is fixed
/// by [`count_taken`] with the time on [`clock`](Self::clock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakenTimer {
    pub id: i32,
    signo: u32,
    pub clock: TimerClock,
}

/// `info` was just dequeued (with the process's state locked, in the same
/// hold). If it is the pending signal of a POSIX timer, the timer waits
/// for [`count_taken`] (until then [`fire`] leaves it alone): the caller
/// reads the timer's clock and calls that, which fixes the count. Every
/// path that takes signals for a thread does this; one that does not
/// (a plain [`LinuxProcess::dequeue`]) only loses the count, as if the
/// signal had been discarded.
pub fn signal_taken(p: &mut LinuxProcess, info: &SigInfo) -> Option<TakenTimer> {
    if info.code != SI_TIMER {
        return None;
    }
    let id = i32::from_le_bytes(info.fields[0..4].try_into().expect("4 bytes"));
    let t = p.timers.get_mut(id)?;
    let Notify::Signal { signo, .. } = t.notify else { return None };
    if signo != info.signal() || !t.signal_pending {
        return None;
    }
    t.taking = true;
    Some(TakenTimer { id, signo, clock: t.clock })
}

/// The signal of [`signal_taken`] was taken at `now` on the timer's clock
/// (`None`: the clock is gone): fix the overrun count, the expiries due
/// from the one after the expiry that queued the signal up to `now`
/// (at most [`DELAYTIMER_MAX`]), the spec of timer_getoverrun. It goes
/// into `info` (`si_overrun`) if that is the signal, and a periodic timer
/// goes on from its first expiry after `now`. Returns whether the timer's
/// next expiry changed (the caller tells the kernel: [`Timers::wakeup`]).
pub fn count_taken(p: &mut LinuxProcess, taken: TakenTimer, now: Option<u64>, info: Option<&mut SigInfo>) -> bool {
    let Some(t) = p.timers.get_mut(taken.id) else { return false };
    if !t.taking || !matches!(t.notify, Notify::Signal { signo, .. } if signo == taken.signo) {
        // Re-armed, or deleted and made again, in between: nothing counts.
        return false;
    }
    t.taking = false;
    t.signal_pending = false;
    let due = match (now, t.deadline) {
        (Some(now), Some(d)) if t.interval > 0 && now >= d => (now - d) / t.interval + 1,
        _ => 0,
    };
    if due > 0 {
        let d = t.deadline.expect("due");
        t.deadline = Some(d.saturating_add(due.saturating_mul(t.interval)));
    }
    t.overrun = u32::try_from(due).unwrap_or(u32::MAX).min(DELAYTIMER_MAX);
    if let Some(info) = info.filter(|i| from_timer(i, taken.id, taken.signo)) {
        info.fields[4..8].copy_from_slice(&t.overrun.to_le_bytes());
    }
    true
}

/// What [`fire`] asks of the kernel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fired {
    /// Threads to notify of a signal.
    pub notify: Vec<i32>,
    /// Continue the process if it is stopped.
    pub continue_process: bool,
    /// When to look at the timers again.
    pub next: TimerWakeup,
    /// Timers that expired this time.
    pub expired: usize,
}

impl Fired {
    fn sent(&mut self, sent: crate::signal::Sent) {
        for tid in sent.notify {
            if !self.notify.contains(&tid) {
                self.notify.push(tid);
            }
        }
        self.continue_process |= sent.continue_process;
    }
}

/// Fire the timers of `p` that are due: `now(clock)` gives each clock's
/// current value (`None` for a clock that no longer exists: the thread
/// is gone). `is_init`: the process is process 1, whose signals obey
/// kill(2)'s rule for init (only handled signals are delivered).
pub fn fire(p: &mut LinuxProcess, now: &dyn Fn(TimerClock) -> Option<u64>, is_init: bool) -> Fired {
    let mut out = Fired::default();
    for (which, &signal) in ITIMER_SIGNALS.iter().enumerate() {
        let t = &mut p.timers.itimers[which];
        let Some(n) = now(t.clock).map(|now| t.expire(now)) else { continue };
        if n == 0 {
            continue;
        }
        out.expired += 1;
        let info = SigInfo::new(signal, SI_KERNEL);
        if let Ok(sent) = p.send(info, Dest::Process, is_init) {
            out.sent(sent);
        }
    }
    let ids: Vec<i32> = p.timers.ids().collect();
    for id in ids {
        let t = p.timers.get_mut(id).expect("listed");
        let Some(at) = now(t.clock) else { continue };
        let (clock_now, interval) = (at, t.interval);
        if let Notify::Signal { signo, .. } = t.notify
            && t.signal_pending
        {
            let taking = t.taking;
            if taking || is_queued(p, id, signo) {
                // Still pending (or being taken): look again later.
                let t = p.timers.get_mut(id).expect("listed");
                t.recheck = clock_now.saturating_add(interval.max(PENDING_RECHECK_NS));
                continue;
            }
            // The signal went without being taken: go on as if it had
            // never been queued.
            p.timers.get_mut(id).expect("listed").signal_pending = false;
        }
        let t = p.timers.get_mut(id).expect("listed");
        let Some(first) = t.deadline.filter(|&d| d <= clock_now) else { continue };
        out.expired += 1;
        let Notify::Signal { signo, value, thread } = t.notify else {
            t.expire(clock_now);
            continue;
        };
        let dest = match thread {
            Some(tid) if p.has_thread(tid) => Dest::Thread(tid),
            _ => Dest::Process,
        };
        // At most one signal of a timer is queued (timer_getoverrun(2)).
        let already = queued(p, dest, id, signo).is_some();
        if !already {
            // The count goes in when the signal is taken (count_taken).
            let info = timer_siginfo(signo, id, 0, value);
            if let Ok(sent) = p.send(info, dest, is_init) {
                out.sent(sent);
            }
        }
        let pending = queued(p, dest, id, signo).is_some();
        let t = p.timers.get_mut(id).expect("listed");
        if pending {
            // The expiries after `first` are counted when the signal is
            // taken: the deadline waits at the next one.
            t.signal_pending = true;
            t.taking = false;
            t.deadline = (t.interval > 0).then(|| first.saturating_add(t.interval));
            t.recheck = clock_now.saturating_add(t.interval.max(PENDING_RECHECK_NS));
        } else {
            // Not queued (ignored, or init's rule): the expiries are gone.
            t.expire(clock_now);
        }
    }
    out.next = p.timers.wakeup();
    out
}
