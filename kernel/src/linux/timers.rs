//! The timers of Linux processes (setitimer, alarm, POSIX timers) in the
//! kernel (ADR 0044).
//!
//! The personality keeps each process's timers in its `LinuxProcess` and
//! says when they are next due ([`TimerWakeup`]). The kernel keeps one
//! table of those wake-ups, by process id, and one kernel thread,
//! `linux-timers`, that sleeps until the earliest of them (or, while some
//! process has a timer on a CPU-time clock, one tick at a time, since CPU
//! time only moves while the process runs). When one comes due the thread
//! calls the personality's [`fire`] on that process's state, with the
//! clocks' values, and notifies the threads it names: their next return
//! to user mode (or the end of their wait) delivers the signal.
//!
//! The table is updated with the process's state locked (lock order: the
//! process's `LinuxState`, then the table), both by system calls that
//! change timers and by the thread after firing, so it never holds a
//! stale wake-up. The thread starts with the first timer anyone arms.

use crate::cputime;
use crate::kprintln;
use crate::process::{self, Process};
use crate::sync::SpinLock;
use crate::thread::{self, WaitQueue};
use crate::time;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use poly_personality_linux::time::{TimerClock, TimerWakeup, fire};

/// When each process's timers are next due.
static ARMED: SpinLock<BTreeMap<u32, TimerWakeup>> = SpinLock::new(BTreeMap::new());
/// Wakes the timer thread early (a timer was armed).
static KICK: WaitQueue = WaitQueue::new();
static KICKED: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
/// Timers the thread has fired (tests, debugging).
static FIRED: AtomicU64 = AtomicU64::new(0);

/// Record `next` for process `pid`. Call with the process's Linux state
/// locked (the lock order above).
fn store(pid: u32, next: TimerWakeup) {
    let mut armed = ARMED.lock();
    if next == TimerWakeup::default() {
        armed.remove(&pid);
    } else {
        armed.insert(pid, next);
    }
}

/// The timers of `process` changed: record when they are next due and
/// wake the timer thread to look again.
pub fn changed(process: &Process) {
    let Some(linux) = &process.linux else { return };
    linux.with(|p| store(process.pid, p.timers.wakeup()));
    start();
    KICKED.store(true, Ordering::SeqCst);
    KICK.wake_all();
}

/// Start the timer thread, once.
fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Err(e) = thread::spawn_named("linux-timers", timer_main, 0) {
        kprintln!("linux: no timer thread ({e:?}): timers will not fire");
        STARTED.store(false, Ordering::SeqCst);
    }
}

/// Timers fired so far.
pub fn fired() -> u64 {
    FIRED.load(Ordering::Relaxed)
}

fn timer_main(_: usize) -> usize {
    loop {
        let (earliest, cpu) = {
            let armed = ARMED.lock();
            let earliest = armed.values().filter_map(|w| w.monotonic).min();
            (earliest, armed.values().any(|w| w.cpu))
        };
        // CPU-time timers: look again every tick.
        let next_tick = time::now_ns().saturating_add(time::NANOS_PER_TICK);
        let deadline = if cpu { Some(earliest.map_or(next_tick, |e| e.min(next_tick))) } else { earliest };
        let kicked = || KICKED.swap(false, Ordering::SeqCst);
        match deadline {
            Some(d) => {
                KICK.wait_until_deadline(kicked, d);
            }
            None => KICK.wait_until(kicked),
        }
        let now = time::now_ns();
        let due: Vec<u32> = ARMED
            .lock()
            .iter()
            .filter(|(_, w)| w.cpu || w.monotonic.is_some_and(|d| d <= now))
            .map(|(&pid, _)| pid)
            .collect();
        for pid in due {
            fire_process(pid);
        }
    }
}

/// Fire what is due of process `pid`'s timers and notify its threads.
fn fire_process(pid: u32) {
    let Some(p) = process::find(pid) else {
        // The process has ended.
        ARMED.lock().remove(&pid);
        return;
    };
    let Some(linux) = &p.linux else {
        ARMED.lock().remove(&pid);
        return;
    };
    // The clocks, read before the state is locked.
    let now = time::now_ns();
    let own = cputime::process(&p);
    let threads = cputime::threads_of(pid);
    let clocks = |c: TimerClock| match c {
        TimerClock::Monotonic => Some(now),
        TimerClock::ProcessCpu => Some(own.total_ns()),
        TimerClock::ProcessUser => Some(own.user_ns),
        TimerClock::ThreadCpu(tid) => {
            threads.iter().find(|&&(t, _)| t as i32 == tid).map(|(_, times)| times.total_ns())
        }
    };
    let fired = linux.with(|lp| {
        let fired = fire(lp, &clocks, pid == 1);
        store(pid, fired.next);
        fired
    });
    FIRED.fetch_add(fired.expired as u64, Ordering::Relaxed);
    if fired.continue_process && p.cont() {
        kprintln!("linux: pid {pid} continued");
    }
    for tid in fired.notify {
        if let Some(flags) = p.thread_flags(tid as u32) {
            process::notify(&flags);
        }
    }
}
