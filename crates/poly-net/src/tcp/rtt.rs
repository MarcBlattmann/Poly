//! Retransmission timeout per RFC 6298: smoothed RTT and RTT variance,
//! RTO = SRTT + max(G, 4 * RTTVAR), clamped, doubled on each timeout.

use core::time::Duration;

/// Clock granularity G: the stack's clock counts milliseconds.
const G_US: u64 = 1000;

#[derive(Clone, Copy, Debug)]
pub struct Rtt {
    srtt_us: Option<u64>,
    rttvar_us: u64,
    rto_us: u64,
    min_us: u64,
    max_us: u64,
    backoff: u32,
}

fn us(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

impl Rtt {
    /// Before any measurement the RTO is `initial` (RFC 6298 2.1: 1 second).
    pub fn new(initial: Duration, min: Duration, max: Duration) -> Rtt {
        let (min_us, max_us) = (us(min), us(max).max(us(min)));
        Rtt { srtt_us: None, rttvar_us: 0, rto_us: us(initial).clamp(min_us, max_us), min_us, max_us, backoff: 0 }
    }

    /// Take a measurement (RFC 6298 2.2 and 2.3).
    pub fn sample(&mut self, r: Duration) {
        // Bounded so that a clock that jumps cannot overflow the arithmetic.
        let r = us(r).clamp(1, 3_600_000_000);
        match self.srtt_us {
            None => {
                self.srtt_us = Some(r);
                self.rttvar_us = r / 2;
            }
            Some(srtt) => {
                let diff = srtt.abs_diff(r);
                self.rttvar_us = (3 * self.rttvar_us + diff) / 4;
                self.srtt_us = Some((7 * srtt + r) / 8);
            }
        }
        let srtt = self.srtt_us.unwrap_or(r);
        self.rto_us = (srtt + G_US.max(4 * self.rttvar_us)).clamp(self.min_us, self.max_us);
        self.backoff = 0;
    }

    /// The current timeout, backoff included.
    pub fn rto(&self) -> Duration {
        let shifted = self.rto_us.checked_shl(self.backoff.min(32)).unwrap_or(u64::MAX);
        Duration::from_micros(shifted.min(self.max_us))
    }

    /// Double the timeout after an expiry (RFC 6298 5.5).
    pub fn back_off(&mut self) {
        if self.rto() < Duration::from_micros(self.max_us) {
            self.backoff += 1;
        }
    }

    /// Forget the backoff once new data is acknowledged.
    pub fn reset_backoff(&mut self) {
        self.backoff = 0;
    }

    /// RFC 6298 5.7: after a SYN timed out, data transfer starts with an RTO
    /// of at least 3 seconds if no measurement replaced it.
    pub fn at_least(&mut self, d: Duration) {
        self.rto_us = self.rto_us.max(us(d)).min(self.max_us);
    }

    /// Smoothed RTT, if measured.
    pub fn srtt(&self) -> Option<Duration> {
        self.srtt_us.map(Duration::from_micros)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn first_sample_and_smoothing() {
        let mut r = Rtt::new(ms(1000), ms(200), ms(60_000));
        assert_eq!(r.rto(), ms(1000));
        r.sample(ms(100));
        // SRTT = 100, RTTVAR = 50, RTO = 100 + 200 = 300.
        assert_eq!(r.rto(), ms(300));
        r.sample(ms(100));
        // RTTVAR = 37.5, RTO = 100 + 150.
        assert_eq!(r.rto(), ms(250));
        for _ in 0..50 {
            r.sample(ms(100));
        }
        assert_eq!(r.rto(), ms(200), "clamped to the minimum");
        assert_eq!(r.srtt(), Some(ms(100)));
    }

    #[test]
    fn absurd_samples_are_bounded() {
        let mut r = Rtt::new(ms(1000), ms(200), ms(60_000));
        r.sample(Duration::MAX);
        r.sample(Duration::MAX);
        assert_eq!(r.rto(), ms(60_000));
        r.sample(Duration::ZERO);
        assert!(r.srtt().is_some());
    }

    #[test]
    fn backoff_doubles_up_to_max() {
        let mut r = Rtt::new(ms(1000), ms(1000), ms(60_000));
        for expect in [2000, 4000, 8000, 16_000, 32_000, 60_000, 60_000] {
            r.back_off();
            assert_eq!(r.rto(), ms(expect));
        }
        r.reset_backoff();
        assert_eq!(r.rto(), ms(1000));
        r.at_least(ms(3000));
        assert_eq!(r.rto(), ms(3000));
    }
}
