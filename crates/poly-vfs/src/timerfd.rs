//! Timers that notify through a file, as timerfd_create(2) describes
//! them.
//!
//! A [`TimerFd`] measures one clock ([`TimerClock`]). It is armed with a
//! first expiration (relative, or absolute with `TFD_TIMER_ABSTIME`) and
//! an interval; reading it returns how many times it expired since the
//! last read (an 8-byte count, `EINVAL` for a shorter buffer) and would
//! block while that is 0. Expirations are counted from the clock whenever
//! the timer is looked at, so a late wake-up never loses any; the kernel's
//! [`TimerService`] calls [`TimerTarget::expire`] at the expiration time so
//! that waiters (a blocked `read`, `poll`, an epoll set) hear of it.

use crate::object::FileObject;
use crate::poll::{PollEvents, WaitQueue};
use crate::sync::SpinMutex;
use crate::{Errno, Result};
use alloc::sync::{Arc, Weak};

/// Which clock a timer measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerClock {
    /// Wall-clock time (`CLOCK_REALTIME`, `CLOCK_REALTIME_ALARM`).
    Realtime,
    /// Time since boot (`CLOCK_MONOTONIC`, and `CLOCK_BOOTTIME`, which is
    /// the same while the machine never suspends).
    Monotonic,
}

/// Called back when a timer is due.
pub trait TimerTarget: Send + Sync {
    fn expire(&self);
}

/// The clocks and the one-shot callbacks timers need, from the kernel.
pub trait TimerService: Send + Sync {
    /// Nanoseconds on the monotonic clock.
    fn monotonic_ns(&self) -> u64;

    /// Nanoseconds since the Unix epoch.
    fn realtime_ns(&self) -> u64;

    /// Call `target.expire()` once the monotonic clock reads `at_ns` or
    /// later, from a thread that holds no locks (not from an interrupt
    /// handler), unless [`cancel`](Self::cancel)led first or `target` is
    /// gone. Returns an id for `cancel`. Never calls `expire` from within
    /// `arm` or `cancel` (the timer calls them with its state locked).
    fn arm(&self, at_ns: u64, target: Weak<dyn TimerTarget>) -> u64;

    /// Forget timer `id` (it may have fired already).
    fn cancel(&self, id: u64);
}

/// A timer setting: the time to the first expiration (or its absolute
/// time) and the interval after it, in nanoseconds. All zero: disarmed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimerSpec {
    pub value_ns: u64,
    pub interval_ns: u64,
}

/// One timer.
pub struct TimerFd {
    clock: TimerClock,
    service: Arc<dyn TimerService>,
    state: SpinMutex<State>,
    queue: Arc<WaitQueue>,
    me: Weak<TimerFd>,
}

#[derive(Default)]
struct State {
    /// The next expiration, on the timer's clock; `None` when disarmed.
    next: Option<u64>,
    interval: u64,
    /// Expirations not yet read.
    ticks: u64,
    /// The callback armed with the service.
    armed: Option<u64>,
}

impl TimerFd {
    /// A disarmed timer on `clock`.
    pub fn new(clock: TimerClock, service: Arc<dyn TimerService>) -> Arc<TimerFd> {
        Arc::new_cyclic(|me| TimerFd {
            clock,
            service,
            state: SpinMutex::new(State::default()),
            queue: Arc::new(WaitQueue::new()),
            me: me.clone(),
        })
    }

    pub fn clock(&self) -> TimerClock {
        self.clock
    }

    fn now(&self) -> u64 {
        match self.clock {
            TimerClock::Realtime => self.service.realtime_ns(),
            TimerClock::Monotonic => self.service.monotonic_ns(),
        }
    }

    /// Count the expirations up to `now`; true if there were new ones.
    fn update(s: &mut State, now: u64) -> bool {
        let Some(next) = s.next else { return false };
        if now < next {
            return false;
        }
        match (now - next).checked_div(s.interval) {
            None => {
                s.ticks = s.ticks.saturating_add(1);
                s.next = None;
            }
            Some(periods) => {
                let n = periods + 1;
                s.ticks = s.ticks.saturating_add(n);
                s.next = Some(next.saturating_add(n.saturating_mul(s.interval)));
            }
        }
        true
    }

    /// Ask the service for a callback at the next expiration (cancelling
    /// the one before).
    fn arm(&self, s: &mut State, now: u64) {
        if let Some(id) = s.armed.take() {
            self.service.cancel(id);
        }
        if let Some(next) = s.next {
            let mono = self.service.monotonic_ns();
            let at = mono.saturating_add(next.saturating_sub(now));
            let me: Weak<dyn TimerTarget> = self.me.clone();
            s.armed = Some(self.service.arm(at, me));
        }
    }

    /// The setting as `timerfd_gettime` reports it: the time left to the
    /// next expiration (0: disarmed) and the interval.
    fn report(s: &State, now: u64) -> TimerSpec {
        match s.next {
            Some(next) => TimerSpec { value_ns: next.saturating_sub(now).max(1), interval_ns: s.interval },
            None => TimerSpec { value_ns: 0, interval_ns: s.interval },
        }
    }

    /// `timerfd_settime(2)`: arm with `new` (a value of 0 disarms; with
    /// `absolute`, the value is a time on the timer's clock, possibly
    /// past, which expires at once). Returns the old setting. Expirations
    /// not read yet are dropped (Poly's reading of "arms (starts) or
    /// disarms (stops) the timer"; to be confirmed by the diff runner).
    pub fn set(&self, absolute: bool, new: TimerSpec) -> TimerSpec {
        let notify = {
            let mut s = self.state.lock();
            let now = self.now();
            Self::update(&mut s, now);
            let old = Self::report(&s, now);
            s.ticks = 0;
            s.interval = new.interval_ns;
            s.next = match (new.value_ns, absolute) {
                (0, _) => None,
                (v, true) => Some(v),
                (v, false) => Some(now.saturating_add(v)),
            };
            let expired = Self::update(&mut s, now);
            self.arm(&mut s, now);
            (old, expired)
        };
        if notify.1 {
            self.queue.notify(PollEvents::READABLE);
        }
        notify.0
    }

    /// `timerfd_gettime(2)`.
    pub fn get(&self) -> TimerSpec {
        let mut s = self.state.lock();
        let now = self.now();
        let fresh = Self::update(&mut s, now);
        let spec = Self::report(&s, now);
        drop(s);
        if fresh {
            self.queue.notify(PollEvents::READABLE);
        }
        spec
    }

    /// Expirations not read yet, counted up to now.
    pub fn ticks(&self) -> u64 {
        let mut s = self.state.lock();
        let now = self.now();
        let fresh = Self::update(&mut s, now);
        let ticks = s.ticks;
        drop(s);
        if fresh {
            self.queue.notify(PollEvents::READABLE);
        }
        ticks
    }
}

impl TimerTarget for TimerFd {
    fn expire(&self) {
        let fresh = {
            let mut s = self.state.lock();
            let now = self.now();
            s.armed = None;
            let fresh = Self::update(&mut s, now);
            // Early (the realtime clock moved differently), or periodic:
            // ask again for the next one.
            self.arm(&mut s, now);
            fresh
        };
        if fresh {
            self.queue.notify(PollEvents::READABLE);
        }
    }
}

impl Drop for TimerFd {
    fn drop(&mut self) {
        if let Some(id) = self.state.lock().armed.take() {
            self.service.cancel(id);
        }
    }
}

impl core::fmt::Debug for TimerFd {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = self.state.lock();
        f.debug_struct("TimerFd")
            .field("clock", &self.clock)
            .field("next", &s.next)
            .field("interval", &s.interval)
            .field("ticks", &s.ticks)
            .finish_non_exhaustive()
    }
}

impl FileObject for TimerFd {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < 8 {
            return Err(Errno::EINVAL);
        }
        let ticks = {
            let mut s = self.state.lock();
            let now = self.now();
            Self::update(&mut s, now);
            core::mem::take(&mut s.ticks)
        };
        if ticks == 0 {
            return Err(Errno::EAGAIN);
        }
        buf[..8].copy_from_slice(&ticks.to_ne_bytes());
        Ok(8)
    }

    fn write(&self, _: &[u8]) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn poll(&self) -> PollEvents {
        if self.ticks() > 0 { PollEvents::READABLE } else { PollEvents::NONE }
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.queue)
    }
}
