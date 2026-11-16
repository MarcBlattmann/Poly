//! Time (`_Xtime_get_ticks.yaml`, `_Thrd_create.yaml`).
//!
//! `_Xtime_get_ticks()` is the system time in 100 ns ticks since 1970-01-01: a `FILETIME`
//! (100 ns since 1601-01-01, Learn: "FILETIME structure") minus the ticks between the two
//! epochs. An `xtime` is `{ __time64_t sec; long nsec; }`, an ABSOLUTE time on that clock:
//! `_Thrd_sleep` returns when the system clock reaches it, at once for a time in the past.

/// 100 ns ticks from 1601-01-01 to 1970-01-01.
pub const EPOCH_DIFFERENCE: i64 = 116_444_736_000_000_000;
/// Ticks per second.
pub const TICKS_PER_SECOND: i64 = 10_000_000;
/// Ticks per millisecond.
pub const TICKS_PER_MS: i64 = 10_000;

/// `_Xtime_get_ticks()` for a `FILETIME` value.
pub fn ticks_from_filetime(filetime: u64) -> i64 {
    (filetime as i64).wrapping_sub(EPOCH_DIFFERENCE)
}

/// An `xtime` as ticks (saturating: a time far in the future stays far in the future).
pub fn xtime_ticks(sec: i64, nsec: i32) -> i64 {
    sec.saturating_mul(TICKS_PER_SECOND).saturating_add(i64::from(nsec) / 100)
}

/// The milliseconds to wait from `now` until `until` (both ticks), rounded up so that a wait
/// never ends early; 0 when `until` has passed; at most `u32::MAX - 1` (never `INFINITE`).
pub fn ms_until(until: i64, now: i64) -> u32 {
    if until <= now {
        return 0;
    }
    let ms = until.abs_diff(now).div_ceil(TICKS_PER_MS as u64);
    u32::try_from(ms).unwrap_or(u32::MAX - 1).min(u32::MAX - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch() {
        assert_eq!(ticks_from_filetime(EPOCH_DIFFERENCE as u64), 0);
        // 2026-01-01T00:00:00Z is 1_767_225_600 s after 1970.
        let ft = (1_767_225_600 * TICKS_PER_SECOND + EPOCH_DIFFERENCE) as u64;
        assert_eq!(ticks_from_filetime(ft), 1_767_225_600 * TICKS_PER_SECOND);
    }

    #[test]
    fn xtime_and_waits() {
        assert_eq!(xtime_ticks(2, 500_000_000), 25_000_000);
        assert_eq!(xtime_ticks(0, 99), 0);
        assert_eq!(xtime_ticks(i64::MAX, 0), i64::MAX);
        assert_eq!(ms_until(0, 100), 0, "the past");
        assert_eq!(ms_until(100, 100), 0);
        assert_eq!(ms_until(101, 100), 1, "rounded up");
        assert_eq!(ms_until(500_000, 0), 50);
        assert_eq!(ms_until(i64::MAX, 0), u32::MAX - 1, "never INFINITE");
    }
}
