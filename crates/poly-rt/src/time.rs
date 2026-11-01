//! Time: [`Instant`] on the monotonic clock, the wall clock, [`sleep`].

use crate::abi::Clock;
use crate::{Result, sys};
use core::time::Duration;

/// A point on the monotonic clock (nanoseconds since boot).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(u64);

impl Instant {
    pub fn now() -> Instant {
        Instant(sys::clock(Clock::Monotonic))
    }

    /// The instant `ns` nanoseconds after boot.
    pub const fn from_nanos(ns: u64) -> Instant {
        Instant(ns)
    }

    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// The time from `earlier` to `self` (zero if `earlier` is later).
    pub fn duration_since(self, earlier: Instant) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }

    pub fn elapsed(self) -> Duration {
        Instant::now().duration_since(self)
    }
}

impl core::ops::Add<Duration> for Instant {
    type Output = Instant;
    fn add(self, d: Duration) -> Instant {
        Instant(self.0.saturating_add(u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)))
    }
}

/// The wall clock: the time since 1970-01-01 00:00 UTC.
pub fn since_epoch() -> Duration {
    Duration::from_nanos(sys::clock(Clock::Realtime))
}

/// Sleep for `d`.
pub fn sleep(d: Duration) -> Result<()> {
    sys::sleep(u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instants_subtract_and_add() {
        let a = Instant::from_nanos(1_000);
        let b = a + Duration::from_micros(2);
        assert_eq!(b.as_nanos(), 3_000);
        assert_eq!(b.duration_since(a), Duration::from_nanos(2_000));
        assert_eq!(a.duration_since(b), Duration::ZERO);
        assert_eq!((a + Duration::MAX).as_nanos(), u64::MAX);
    }
}
