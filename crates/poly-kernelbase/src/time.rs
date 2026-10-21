//! File times and calendar times: `FileTimeToSystemTime`,
//! `SystemTimeToFileTime`, `CompareFileTime`, and the local time.
//!
//! Learn ("File Times", "FILETIME", "SYSTEMTIME"): a `FILETIME` counts
//! 100-nanosecond intervals since January 1, 1601 (UTC); `SYSTEMTIME` is
//! eight `WORD`s (`wYear`, `wMonth`, `wDayOfWeek` with 0 = Sunday, `wDay`,
//! `wHour`, `wMinute`, `wSecond`, `wMilliseconds`, `minwinbase.h`), valid
//! for years 1601 to 30827. `FileTimeToSystemTime` fails for a value with
//! the high bit set, `SystemTimeToFileTime` for a field out of range
//! (`ERROR_INVALID_PARAMETER`); the latter ignores `wDayOfWeek`.
//!
//! **Local time.** Poly runs its NT processes in UTC until time zones exist
//! (to be confirmed by the spec agent against the oracle's zone): local
//! times equal UTC times.

/// 100 ns intervals per second, per day.
const TICKS_PER_SECOND: u64 = 10_000_000;
const TICKS_PER_DAY: u64 = 86_400 * TICKS_PER_SECOND;

/// `SYSTEMTIME`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemTime {
    pub year: u16,
    pub month: u16,
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub milliseconds: u16,
}

impl SystemTime {
    /// The structure's 16 bytes.
    pub fn to_bytes(&self) -> [u8; 16] {
        let f =
            [self.year, self.month, self.day_of_week, self.day, self.hour, self.minute, self.second, self.milliseconds];
        let mut b = [0u8; 16];
        for (i, v) in f.iter().enumerate() {
            b[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    pub fn from_bytes(b: &[u8; 16]) -> SystemTime {
        let w = |i: usize| u16::from_le_bytes([b[i * 2], b[i * 2 + 1]]);
        SystemTime {
            year: w(0),
            month: w(1),
            day_of_week: w(2),
            day: w(3),
            hour: w(4),
            minute: w(5),
            second: w(6),
            milliseconds: w(7),
        }
    }
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

fn days_in_month(y: u64, m: u64) -> u64 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn days_in_year(y: u64) -> u64 {
    if is_leap(y) { 366 } else { 365 }
}

/// `FileTimeToSystemTime`.
pub fn to_system_time(ft: u64) -> Option<SystemTime> {
    if ft > i64::MAX as u64 {
        return None;
    }
    let mut days = ft / TICKS_PER_DAY;
    let rest = ft % TICKS_PER_DAY;
    // 1601-01-01 was a Monday.
    let day_of_week = ((days + 1) % 7) as u16;
    // Whole 400-year cycles (146,097 days) first.
    let mut year = 1601 + 400 * (days / 146_097);
    days %= 146_097;
    while days >= days_in_year(year) {
        days -= days_in_year(year);
        year += 1;
    }
    let mut month = 1;
    while days >= days_in_month(year, month) {
        days -= days_in_month(year, month);
        month += 1;
    }
    let secs = rest / TICKS_PER_SECOND;
    Some(SystemTime {
        year: year as u16,
        month: month as u16,
        day_of_week,
        day: days as u16 + 1,
        hour: (secs / 3600) as u16,
        minute: (secs / 60 % 60) as u16,
        second: (secs % 60) as u16,
        milliseconds: (rest % TICKS_PER_SECOND / 10_000) as u16,
    })
}

/// `SystemTimeToFileTime`.
pub fn to_file_time(st: &SystemTime) -> Option<u64> {
    let (y, m, d) = (u64::from(st.year), u64::from(st.month), u64::from(st.day));
    if !(1601..=30827).contains(&y)
        || !(1..=12).contains(&m)
        || d == 0
        || d > days_in_month(y, m)
        || st.hour > 23
        || st.minute > 59
        || st.second > 59
        || st.milliseconds > 999
    {
        return None;
    }
    let cycles = (y - 1601) / 400;
    let mut days = cycles * 146_097;
    for year in 1601 + cycles * 400..y {
        days += days_in_year(year);
    }
    for month in 1..m {
        days += days_in_month(y, month);
    }
    days += d - 1;
    let secs = u64::from(st.hour) * 3600 + u64::from(st.minute) * 60 + u64::from(st.second);
    Some(days * TICKS_PER_DAY + secs * TICKS_PER_SECOND + u64::from(st.milliseconds) * 10_000)
}

/// `CompareFileTime`: -1, 0 or 1.
pub fn compare(a: u64, b: u64) -> i32 {
    match a.cmp(&b) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

/// The local time of a UTC `FILETIME` (see the module documentation).
pub fn to_local(utc: u64) -> u64 {
    utc
}

/// The UTC time of a local `FILETIME`.
pub fn from_local(local: u64) -> u64 {
    local
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(year: u16, month: u16, day: u16, hour: u16, minute: u16, second: u16, ms: u16) -> SystemTime {
        SystemTime { year, month, day_of_week: 0, day, hour, minute, second, milliseconds: ms }
    }

    #[test]
    fn known_dates() {
        let epoch = to_system_time(0).unwrap();
        assert_eq!(epoch, SystemTime { day_of_week: 1, ..st(1601, 1, 1, 0, 0, 0, 0) });
        // The Unix epoch: 11644473600 seconds after 1601 (a Thursday).
        let unix = 116_444_736_000_000_000;
        assert_eq!(to_system_time(unix).unwrap(), SystemTime { day_of_week: 4, ..st(1970, 1, 1, 0, 0, 0, 0) });
        assert_eq!(to_file_time(&st(1970, 1, 1, 0, 0, 0, 0)), Some(unix));
        // 2000-02-29 12:34:56.789, a Tuesday.
        let t = to_file_time(&st(2000, 2, 29, 12, 34, 56, 789)).unwrap();
        let back = to_system_time(t + 1234).unwrap();
        assert_eq!(back, SystemTime { day_of_week: 2, ..st(2000, 2, 29, 12, 34, 56, 789) });
        assert_eq!(to_system_time(i64::MAX as u64).unwrap().year, 30828);
        assert_eq!(to_system_time(1 << 63), None);
    }

    #[test]
    fn round_trips_and_validation() {
        for ft in (0..200).map(|i: u64| i * 0x0001_2345_6789_ABCD % (i64::MAX as u64 / 4)) {
            let s = to_system_time(ft).unwrap();
            assert_eq!(to_file_time(&s), Some(ft - ft % 10_000), "{ft}");
        }
        assert_eq!(to_file_time(&st(2023, 2, 29, 0, 0, 0, 0)), None);
        assert_eq!(to_file_time(&st(1600, 12, 31, 0, 0, 0, 0)), None);
        assert_eq!(to_file_time(&st(2023, 13, 1, 0, 0, 0, 0)), None);
        assert_eq!(to_file_time(&st(2023, 1, 1, 24, 0, 0, 0)), None);
        assert_eq!(to_file_time(&st(2023, 1, 1, 0, 0, 0, 1000)), None);
        // wDayOfWeek is ignored.
        let mut s = st(2024, 1, 1, 0, 0, 0, 0);
        s.day_of_week = 6;
        assert!(to_file_time(&s).is_some());
        assert_eq!(SystemTime::from_bytes(&s.to_bytes()), s);
        assert_eq!(compare(1, 2), -1);
        assert_eq!(compare(2, 2), 0);
        assert_eq!(compare(3, 2), 1);
    }
}
