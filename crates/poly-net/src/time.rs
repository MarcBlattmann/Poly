//! Time as the stack sees it: a monotonic millisecond count the caller
//! passes in. There is no global clock; every timer is a deadline compared
//! against the `now` given to [`crate::Stack::poll`].

use core::fmt;
use core::ops::{Add, Sub};
use core::time::Duration;

/// A point on the caller's monotonic clock, in milliseconds since an
/// arbitrary origin (boot, say).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Instant(u64);

impl Instant {
    /// The clock's origin.
    pub const ZERO: Instant = Instant(0);

    /// `ms` milliseconds after the origin.
    pub const fn from_millis(ms: u64) -> Instant {
        Instant(ms)
    }

    /// Milliseconds since the origin.
    pub const fn millis(self) -> u64 {
        self.0
    }

    /// Time from `earlier` to `self`, zero if `earlier` is later.
    pub fn since(self, earlier: Instant) -> Duration {
        Duration::from_millis(self.0.saturating_sub(earlier.0))
    }
}

/// Milliseconds in `d`, saturating.
pub(crate) fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

impl Add<Duration> for Instant {
    type Output = Instant;
    fn add(self, d: Duration) -> Instant {
        Instant(self.0.saturating_add(ms(d)))
    }
}

impl Sub<Duration> for Instant {
    type Output = Instant;
    fn sub(self, d: Duration) -> Instant {
        Instant(self.0.saturating_sub(ms(d)))
    }
}

impl fmt::Debug for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:03}s", self.0 / 1000, self.0 % 1000)
    }
}

/// The earlier of two optional deadlines.
pub(crate) fn earliest(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_saturates() {
        let t = Instant::from_millis(5);
        assert_eq!((t - Duration::from_millis(10)).millis(), 0);
        assert_eq!((Instant::from_millis(u64::MAX) + Duration::from_secs(1)).millis(), u64::MAX);
        assert_eq!(t.since(Instant::from_millis(9)), Duration::ZERO);
        assert_eq!(Instant::from_millis(1500).since(t), Duration::from_millis(1495));
    }

    #[test]
    fn earliest_picks_the_smaller() {
        let a = Some(Instant::from_millis(3));
        let b = Some(Instant::from_millis(2));
        assert_eq!(earliest(a, b), b);
        assert_eq!(earliest(a, None), a);
        assert_eq!(earliest(None, None), None);
    }
}
