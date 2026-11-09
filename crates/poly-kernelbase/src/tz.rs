//! Time zones (Chrome batch 2, ADR 0155): Learn's `timezoneapi.h` pages
//! (`TIME_ZONE_INFORMATION`, `DYNAMIC_TIME_ZONE_INFORMATION`,
//! `GetTimeZoneInformation`, `SystemTimeToTzSpecificLocalTime`,
//! `TzSpecificLocalTimeToSystemTime`) as host-tested code.
//!
//! A zone is a bias (minutes, UTC = local + Bias) and, when it has daylight
//! saving time, two transition dates: `DaylightDate` is when daylight time
//! begins, as local *standard* time; `StandardDate` when standard time
//! resumes, as local *daylight* time. A transition date with `wYear` 0 is
//! relative: month `wMonth`, the `wDay`th (1 to 5, 5 meaning the last)
//! `wDayOfWeek` (0 = Sunday) of it at `wHour:wMinute:wSecond`; with a
//! year it is that absolute time. Poly's clock and its local time are UTC
//! ([`crate::time::to_local`]), so the active zone is [`utc`].

use crate::time::{SystemTime, to_file_time, to_system_time};
use alloc::vec::Vec;

/// `TIME_ZONE_ID_UNKNOWN`, `TIME_ZONE_ID_STANDARD`, `TIME_ZONE_ID_DAYLIGHT`
/// (`timezoneapi.h`); `TIME_ZONE_ID_INVALID` is `0xFFFFFFFF`.
pub const TIME_ZONE_ID_UNKNOWN: u32 = 0;
pub const TIME_ZONE_ID_STANDARD: u32 = 1;
pub const TIME_ZONE_ID_DAYLIGHT: u32 = 2;
pub const TIME_ZONE_ID_INVALID: u32 = 0xFFFF_FFFF;

/// `sizeof(TIME_ZONE_INFORMATION)`: `Bias` 0, `StandardName[32]` 4,
/// `StandardDate` 68, `StandardBias` 84, `DaylightName[32]` 88,
/// `DaylightDate` 152, `DaylightBias` 168.
pub const TZI_SIZE: usize = 172;
/// `sizeof(DYNAMIC_TIME_ZONE_INFORMATION)`: the same, then
/// `TimeZoneKeyName[128]` at 172 and `DynamicDaylightTimeDisabled` at 428.
pub const DTZI_SIZE: usize = 432;

const FILETIME_PER_MINUTE: i64 = 600_000_000;

/// A `TIME_ZONE_INFORMATION`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tzi {
    pub bias: i32,
    pub standard_name: [u16; 32],
    pub standard_date: SystemTime,
    pub standard_bias: i32,
    pub daylight_name: [u16; 32],
    pub daylight_date: SystemTime,
    pub daylight_bias: i32,
}

fn name32(s: &str) -> [u16; 32] {
    let mut out = [0u16; 32];
    for (i, c) in s.encode_utf16().take(31).enumerate() {
        out[i] = c;
    }
    out
}

const ZERO: SystemTime =
    SystemTime { year: 0, month: 0, day_of_week: 0, day: 0, hour: 0, minute: 0, second: 0, milliseconds: 0 };

/// The zone Poly runs in: Coordinated Universal Time, no daylight saving
/// time (the names Windows gives its `UTC` zone).
pub fn utc() -> Tzi {
    Tzi {
        bias: 0,
        standard_name: name32("Coordinated Universal Time"),
        standard_date: ZERO,
        standard_bias: 0,
        daylight_name: name32("Coordinated Universal Time"),
        daylight_date: ZERO,
        daylight_bias: 0,
    }
}

/// The registry key name of [`utc`] (`DYNAMIC_TIME_ZONE_INFORMATION.TimeZoneKeyName`).
pub const UTC_KEY_NAME: &str = "UTC";

impl Tzi {
    pub fn to_bytes(&self) -> [u8; TZI_SIZE] {
        let mut b = [0u8; TZI_SIZE];
        b[0..4].copy_from_slice(&self.bias.to_le_bytes());
        for (i, c) in self.standard_name.iter().enumerate() {
            b[4 + 2 * i..6 + 2 * i].copy_from_slice(&c.to_le_bytes());
        }
        b[68..84].copy_from_slice(&self.standard_date.to_bytes());
        b[84..88].copy_from_slice(&self.standard_bias.to_le_bytes());
        for (i, c) in self.daylight_name.iter().enumerate() {
            b[88 + 2 * i..90 + 2 * i].copy_from_slice(&c.to_le_bytes());
        }
        b[152..168].copy_from_slice(&self.daylight_date.to_bytes());
        b[168..172].copy_from_slice(&self.daylight_bias.to_le_bytes());
        b
    }

    /// Read the structure (the first `TZI_SIZE` bytes of a
    /// `DYNAMIC_TIME_ZONE_INFORMATION` are one).
    pub fn from_bytes(b: &[u8]) -> Option<Tzi> {
        let b = b.get(..TZI_SIZE)?;
        let i32_at = |at: usize| i32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let name = |at: usize| {
            let mut n = [0u16; 32];
            for (i, c) in n.iter_mut().enumerate() {
                *c = u16::from_le_bytes([b[at + 2 * i], b[at + 2 * i + 1]]);
            }
            n
        };
        Some(Tzi {
            bias: i32_at(0),
            standard_name: name(4),
            standard_date: SystemTime::from_bytes(b[68..84].try_into().unwrap()),
            standard_bias: i32_at(84),
            daylight_name: name(88),
            daylight_date: SystemTime::from_bytes(b[152..168].try_into().unwrap()),
            daylight_bias: i32_at(168),
        })
    }

    /// A zone with daylight saving time has both transition dates (a
    /// month of 0 means "none").
    pub fn has_dst(&self) -> bool {
        self.standard_date.month != 0 && self.daylight_date.month != 0
    }
}

/// A `DYNAMIC_TIME_ZONE_INFORMATION` for `tzi` with the key name
/// `key_name` and `DynamicDaylightTimeDisabled` 0.
pub fn dynamic_bytes(tzi: &Tzi, key_name: &str) -> [u8; DTZI_SIZE] {
    let mut b = [0u8; DTZI_SIZE];
    b[..TZI_SIZE].copy_from_slice(&tzi.to_bytes());
    for (i, c) in key_name.encode_utf16().take(127).enumerate() {
        b[172 + 2 * i..174 + 2 * i].copy_from_slice(&c.to_le_bytes());
    }
    b
}

fn days_in_month(year: u16, month: u16) -> u16 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) {
                29
            } else {
                28
            }
        }
    }
}

/// The wall-clock time (a `FILETIME` count, no zone) a transition date
/// names in `year`: `None` for an invalid date.
fn transition(date: &SystemTime, year: u16) -> Option<i64> {
    let (y, day) = if date.year == 0 {
        if !(1..=12).contains(&date.month) || !(1..=5).contains(&date.day) || date.day_of_week > 6 {
            return None;
        }
        let first = SystemTime {
            year,
            month: date.month,
            day_of_week: 0,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            milliseconds: 0,
        };
        let dow_first = to_system_time(to_file_time(&first)?)?.day_of_week;
        let mut day = 1 + (date.day_of_week + 7 - dow_first) % 7 + 7 * (date.day - 1);
        while day > days_in_month(year, date.month) {
            day -= 7;
        }
        (year, day)
    } else {
        (date.year, date.day)
    };
    let st = SystemTime {
        year: y,
        month: date.month,
        day_of_week: 0,
        day,
        hour: date.hour,
        minute: date.minute,
        second: date.second,
        milliseconds: date.milliseconds,
    };
    Some(to_file_time(&st)? as i64)
}

fn year_of(ft: i64) -> Option<u16> {
    Some(to_system_time(u64::try_from(ft).ok()?)?.year)
}

/// Is `utc` (a `FILETIME`) in daylight saving time in `tz`?
pub fn is_daylight(tz: &Tzi, utc: u64) -> bool {
    if !tz.has_dst() {
        return false;
    }
    // The year is the one of the local standard time.
    let local_std = utc as i64 - (i64::from(tz.bias) + i64::from(tz.standard_bias)) * FILETIME_PER_MINUTE;
    let Some(year) = year_of(local_std) else { return false };
    let (Some(start_local), Some(end_local)) =
        (transition(&tz.daylight_date, year), transition(&tz.standard_date, year))
    else {
        return false;
    };
    // Daylight time begins at `start_local` standard time and ends at
    // `end_local` daylight time.
    let start = start_local + (i64::from(tz.bias) + i64::from(tz.standard_bias)) * FILETIME_PER_MINUTE;
    let end = end_local + (i64::from(tz.bias) + i64::from(tz.daylight_bias)) * FILETIME_PER_MINUTE;
    let t = utc as i64;
    if start < end { t >= start && t < end } else { t >= start || t < end }
}

/// `GetTimeZoneInformation`'s result for the time `utc`.
pub fn zone_id(tz: &Tzi, utc: u64) -> u32 {
    if !tz.has_dst() {
        TIME_ZONE_ID_UNKNOWN
    } else if is_daylight(tz, utc) {
        TIME_ZONE_ID_DAYLIGHT
    } else {
        TIME_ZONE_ID_STANDARD
    }
}

/// `SystemTimeToTzSpecificLocalTime`: the local `FILETIME` of `utc`.
pub fn utc_to_local(tz: &Tzi, utc: u64) -> Option<u64> {
    let bias = i64::from(tz.bias)
        + if is_daylight(tz, utc) { i64::from(tz.daylight_bias) } else { i64::from(tz.standard_bias) };
    u64::try_from(utc as i64 - bias * FILETIME_PER_MINUTE).ok()
}

/// `TzSpecificLocalTimeToSystemTime`: the UTC `FILETIME` of `local`. A
/// local time that happens twice (the hour daylight time ends) is taken as
/// standard time; one that does not happen (the hour it begins) as standard
/// time too (Learn does not say; this keeps the result inside the hour
/// after the change).
pub fn local_to_utc(tz: &Tzi, local: u64) -> Option<u64> {
    let std = local as i64 + (i64::from(tz.bias) + i64::from(tz.standard_bias)) * FILETIME_PER_MINUTE;
    let utc = if tz.has_dst() && is_daylight(tz, u64::try_from(std).ok()?) {
        let dst = local as i64 + (i64::from(tz.bias) + i64::from(tz.daylight_bias)) * FILETIME_PER_MINUTE;
        // Daylight only if that reading is itself daylight time (a time in
        // the fold is standard).
        if is_daylight(tz, u64::try_from(dst).ok()?) { dst } else { std }
    } else {
        std
    };
    u64::try_from(utc).ok()
}

/// A `SYSTEMTIME` pair for the two conversions (`None`: not a valid time).
pub fn convert(tz: &Tzi, st: &SystemTime, to_local: bool) -> Option<SystemTime> {
    let ft = to_file_time(st)?;
    let out = if to_local { utc_to_local(tz, ft)? } else { local_to_utc(tz, ft)? };
    to_system_time(out)
}

/// A zone's name, trimmed at the first NUL, for tests and logs.
pub fn name_of(name: &[u16; 32]) -> Vec<u16> {
    name.iter().copied().take_while(|&c| c != 0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(year: u16, month: u16, day: u16, hour: u16, minute: u16, second: u16) -> SystemTime {
        SystemTime { year, month, day_of_week: 0, day, hour, minute, second, milliseconds: 0 }
    }

    fn rule(month: u16, dow: u16, week: u16, hour: u16) -> SystemTime {
        SystemTime { year: 0, month, day_of_week: dow, day: week, hour, minute: 0, second: 0, milliseconds: 0 }
    }

    /// Pacific time: UTC-8; daylight time from the second Sunday of March
    /// 02:00 to the first Sunday of November 02:00.
    fn pacific() -> Tzi {
        Tzi {
            bias: 480,
            standard_name: name32("Pacific Standard Time"),
            standard_date: rule(11, 0, 1, 2),
            standard_bias: 0,
            daylight_name: name32("Pacific Daylight Time"),
            daylight_date: rule(3, 0, 2, 2),
            daylight_bias: -60,
        }
    }

    /// Sydney: UTC+10; daylight time from the first Sunday of October
    /// 02:00 to the first Sunday of April 03:00 (southern hemisphere).
    fn sydney() -> Tzi {
        Tzi {
            bias: -600,
            standard_name: name32("AUS Eastern Standard Time"),
            standard_date: rule(4, 0, 1, 3),
            standard_bias: 0,
            daylight_name: name32("AUS Eastern Daylight Time"),
            daylight_date: rule(10, 0, 1, 2),
            daylight_bias: -60,
        }
    }

    fn local(tz: &Tzi, utc: SystemTime) -> SystemTime {
        let mut r = convert(tz, &utc, true).unwrap();
        r.day_of_week = 0;
        r.milliseconds = 0;
        r
    }

    #[test]
    fn utc_zone() {
        let z = utc();
        let t = to_file_time(&st(2024, 7, 1, 12, 0, 0)).unwrap();
        assert_eq!(zone_id(&z, t), TIME_ZONE_ID_UNKNOWN);
        assert_eq!(utc_to_local(&z, t), Some(t));
        assert_eq!(local_to_utc(&z, t), Some(t));
        assert!(!z.has_dst());
        let b = z.to_bytes();
        assert_eq!(Tzi::from_bytes(&b), Some(z.clone()));
        assert_eq!(dynamic_bytes(&z, UTC_KEY_NAME).len(), 432);
        assert_eq!(&dynamic_bytes(&z, UTC_KEY_NAME)[172..180], &[b'U', 0, b'T', 0, b'C', 0, 0, 0]);
    }

    #[test]
    fn north_transitions() {
        let tz = pacific();
        // 2024: DST begins Sunday March 10 at 02:00 PST (10:00 UTC), ends
        // Sunday November 3 at 02:00 PDT (09:00 UTC).
        assert_eq!(local(&tz, st(2024, 3, 10, 9, 59, 59)), st(2024, 3, 10, 1, 59, 59));
        assert_eq!(local(&tz, st(2024, 3, 10, 10, 0, 0)), st(2024, 3, 10, 3, 0, 0));
        assert_eq!(local(&tz, st(2024, 11, 3, 8, 59, 59)), st(2024, 11, 3, 1, 59, 59));
        assert_eq!(local(&tz, st(2024, 11, 3, 9, 0, 0)), st(2024, 11, 3, 1, 0, 0));
        assert_eq!(local(&tz, st(2024, 7, 4, 19, 0, 0)), st(2024, 7, 4, 12, 0, 0));
        assert_eq!(local(&tz, st(2024, 1, 15, 8, 0, 0)), st(2024, 1, 15, 0, 0, 0));
        let ids = |t: SystemTime| zone_id(&tz, to_file_time(&t).unwrap());
        assert_eq!(ids(st(2024, 1, 1, 0, 0, 0)), TIME_ZONE_ID_STANDARD);
        assert_eq!(ids(st(2024, 7, 1, 0, 0, 0)), TIME_ZONE_ID_DAYLIGHT);
    }

    #[test]
    fn south_transitions() {
        let tz = sydney();
        // 2024: DST began Sunday October 6 at 02:00 AEST (Oct 5 16:00 UTC)
        // and ended Sunday April 7 at 03:00 AEDT (Apr 6 16:00 UTC).
        assert_eq!(local(&tz, st(2024, 10, 5, 15, 59, 59)), st(2024, 10, 6, 1, 59, 59));
        assert_eq!(local(&tz, st(2024, 10, 5, 16, 0, 0)), st(2024, 10, 6, 3, 0, 0));
        assert_eq!(local(&tz, st(2024, 4, 6, 15, 59, 59)), st(2024, 4, 7, 2, 59, 59));
        assert_eq!(local(&tz, st(2024, 4, 6, 16, 0, 0)), st(2024, 4, 7, 2, 0, 0));
        assert_eq!(local(&tz, st(2024, 1, 1, 0, 0, 0)), st(2024, 1, 1, 11, 0, 0), "January is summer");
        assert_eq!(local(&tz, st(2024, 7, 1, 0, 0, 0)), st(2024, 7, 1, 10, 0, 0));
        assert_eq!(zone_id(&tz, to_file_time(&st(2024, 1, 1, 0, 0, 0)).unwrap()), TIME_ZONE_ID_DAYLIGHT);
    }

    #[test]
    fn local_to_utc_round_trips() {
        let tz = pacific();
        for t in
            [st(2024, 1, 15, 0, 0, 0), st(2024, 7, 4, 12, 0, 0), st(2024, 3, 10, 1, 59, 59), st(2024, 11, 3, 0, 59, 59)]
        {
            let back = convert(&tz, &convert(&tz, &t, false).unwrap(), true).unwrap();
            assert_eq!(SystemTime { day_of_week: 0, ..back }, t, "{t:?}");
        }
        // 02:30 on March 10 does not exist: read as standard time (10:30 UTC, 03:30 PDT).
        let gap = convert(&tz, &st(2024, 3, 10, 2, 30, 0), false).unwrap();
        assert_eq!(SystemTime { day_of_week: 0, ..gap }, st(2024, 3, 10, 10, 30, 0));
        // 01:30 on November 3 happens twice: the standard one.
        let fold = convert(&tz, &st(2024, 11, 3, 1, 30, 0), false).unwrap();
        assert_eq!(SystemTime { day_of_week: 0, ..fold }, st(2024, 11, 3, 9, 30, 0));
    }

    #[test]
    fn rules_and_absolute_dates() {
        // The fifth Sunday is the last one: October 2024's last Sunday is the 27th.
        let last = transition(&rule(10, 0, 5, 1), 2024).unwrap();
        assert_eq!(last, to_file_time(&st(2024, 10, 27, 1, 0, 0)).unwrap() as i64);
        // February in a leap year.
        let feb = transition(&rule(2, 4, 5, 0), 2024).unwrap();
        assert_eq!(feb, to_file_time(&st(2024, 2, 29, 0, 0, 0)).unwrap() as i64);
        // An absolute transition.
        let abs =
            SystemTime { year: 2030, month: 5, day_of_week: 0, day: 6, hour: 7, minute: 8, second: 9, milliseconds: 0 };
        assert_eq!(transition(&abs, 2024), to_file_time(&st(2030, 5, 6, 7, 8, 9)).map(|t| t as i64));
        assert_eq!(transition(&rule(13, 0, 1, 0), 2024), None);
        assert_eq!(convert(&pacific(), &st(2023, 2, 30, 0, 0, 0), true), None, "invalid time");
        assert_eq!(name_of(&pacific().standard_name).len(), 21);
    }
}
