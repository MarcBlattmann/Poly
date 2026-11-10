//! The logic of our `winmm.dll` timer functions (`mmsystem.h`, `timeapi.h`; Learn: `timeGetTime`,
//! `timeBeginPeriod`, `timeEndPeriod`). Chrome calls them when it starts (a high-resolution
//! timer request around its message loop), and `timeGetTime` for timestamps.

/// `TIMERR_NOERROR` (`mmsystem.h`: `TIMERR_BASE` 96 + 0).
pub const TIMERR_NOERROR: u32 = 0;
/// `TIMERR_NOCANDO` (`mmsystem.h`: `TIMERR_BASE` 96 + 1): the period is out of range.
pub const TIMERR_NOCANDO: u32 = 97;

/// `timeGetTime`: Learn: "the system time, in milliseconds ... the time elapsed since Windows
/// was started", a `DWORD`, so it wraps around to 0 after 2^32 ms (49.71 days).
pub fn time_get_time(uptime_ms: u64) -> u32 {
    uptime_ms as u32
}

/// `timeBeginPeriod` / `timeEndPeriod`: `TIMERR_NOERROR` for a period in the range the
/// timer-device capabilities allow, `TIMERR_NOCANDO` for one outside it. A period of 0 is
/// outside it; Poly's clock has a 1 ms resolution, so any period from 1 ms on is granted
/// (and changes nothing).
pub fn period_result(period_ms: u32) -> u32 {
    if period_ms == 0 { TIMERR_NOCANDO } else { TIMERR_NOERROR }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_is_milliseconds_since_start_and_wraps_at_2_pow_32() {
        assert_eq!(time_get_time(0), 0);
        assert_eq!(time_get_time(1234), 1234);
        assert_eq!(time_get_time(u64::from(u32::MAX)), u32::MAX);
        assert_eq!(time_get_time(1 << 32), 0);
        assert_eq!(time_get_time((1 << 32) + 5), 5);
    }

    #[test]
    fn periods_of_at_least_one_millisecond_are_granted() {
        assert_eq!(period_result(1), TIMERR_NOERROR);
        assert_eq!(period_result(15), TIMERR_NOERROR);
        assert_eq!(period_result(0), TIMERR_NOCANDO);
        assert_eq!((TIMERR_NOERROR, TIMERR_NOCANDO), (0, 97));
    }
}
