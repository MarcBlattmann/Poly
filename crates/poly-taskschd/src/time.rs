//! Times: the task schema's date-times (`YYYY-MM-DDTHH:MM:SS` with an optional fraction and zone,
//! Learn: "StartBoundary (triggerBaseType)") and durations (`PnYnMnWnDTnHnMnS`, Learn:
//! "IRepetitionPattern::Interval"), and automation `DATE`s (days since 1899-12-30, Learn: "DATE").
//!
//! Times are kept as seconds since 1970-01-01 in local time; Poly's local time is UTC (its default
//! time zone, `TimeZoneKeyName` UTC), so a zone in a boundary is converted to UTC.

use alloc::format;
use alloc::string::String;

/// `IRegisteredTask::LastRunTime` of a task that never ran: 1999-11-30 (the sheet's DATE 36494.0).
pub const NEVER_RUN: f64 = 36494.0;

/// The `DATE` of 1970-01-01.
const UNIX_EPOCH_DATE: f64 = 25569.0;

/// Days from 1970-01-01 to the given date of the proleptic Gregorian calendar.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    // Count from 0000-03-01 so that the leap day is the last day of the year.
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * i64::from(m) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day number (the inverse of [`days_from_civil`]).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

fn digits(s: &str, n: usize) -> Option<u32> {
    if s.len() != n || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// A schema date-time as seconds since 1970 (UTC); `None` when it is not one.
pub fn parse_datetime(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once('T')?;
    let mut dp = date.split('-');
    let y = digits(dp.next()?, 4)?;
    let mo = digits(dp.next()?, 2)?;
    let d = digits(dp.next()?, 2)?;
    if dp.next().is_some() || !(1..=12).contains(&mo) || d == 0 || d > days_in_month(i64::from(y), mo) {
        return None;
    }
    // The zone: `Z`, `+HH:MM` or `-HH:MM` after the time.
    let (time, offset) = if let Some(t) = rest.strip_suffix('Z') {
        (t, 0i64)
    } else if let Some(i) = rest.rfind(['+', '-']) {
        let (t, z) = rest.split_at(i);
        let sign = if z.starts_with('-') { -1 } else { 1 };
        let (zh, zm) = z[1..].split_once(':')?;
        let (zh, zm) = (digits(zh, 2)?, digits(zm, 2)?);
        if zh > 14 || zm > 59 {
            return None;
        }
        (t, sign * (i64::from(zh) * 3600 + i64::from(zm) * 60))
    } else {
        (rest, 0)
    };
    let (time, _fraction) = match time.split_once('.') {
        Some((t, f)) if !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()) => (t, f),
        Some(_) => return None,
        None => (time, ""),
    };
    let mut tp = time.split(':');
    let h = digits(tp.next()?, 2)?;
    let mi = digits(tp.next()?, 2)?;
    let se = digits(tp.next()?, 2)?;
    if tp.next().is_some() || h > 23 || mi > 59 || se > 59 {
        return None;
    }
    let days = days_from_civil(i64::from(y), mo, d);
    Some(days * 86_400 + i64::from(h) * 3600 + i64::from(mi) * 60 + i64::from(se) - offset)
}

/// Seconds since 1970 as a schema date-time (`2030-01-01T00:00:00`).
pub fn format_datetime(t: i64) -> String {
    let (y, m, d) = civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// A schema duration in seconds; `None` when it is not one (Learn: `PnYnMnWnDTnHnMnS`; a month is
/// counted as 30 days and a year as 365 for scheduling).
pub fn parse_duration(s: &str) -> Option<u64> {
    let rest = s.strip_prefix('P')?;
    if rest.is_empty() {
        return None;
    }
    let mut total: u64 = 0;
    let mut in_time = false;
    let mut num: Option<u64> = None;
    let mut parts = 0;
    let mut after_t = false;
    for c in rest.chars() {
        match c {
            '0'..='9' => {
                let d = u64::from(c as u8 - b'0');
                num = Some(num.unwrap_or(0).checked_mul(10)?.checked_add(d)?);
            }
            'T' if !in_time && num.is_none() => {
                in_time = true;
                after_t = true;
            }
            _ => {
                let n = num.take()?;
                let unit = match (in_time, c) {
                    (false, 'Y') => 365 * 86_400,
                    (false, 'M') => 30 * 86_400,
                    (false, 'W') => 7 * 86_400,
                    (false, 'D') => 86_400,
                    (true, 'H') => 3600,
                    (true, 'M') => 60,
                    (true, 'S') => 1,
                    _ => return None,
                };
                total = total.checked_add(n.checked_mul(unit)?)?;
                parts += 1;
                after_t = false;
            }
        }
    }
    if num.is_some() || parts == 0 || after_t {
        return None;
    }
    Some(total)
}

/// Seconds since 1970 as an automation `DATE`.
pub fn date_from_unix(t: i64) -> f64 {
    t as f64 / 86_400.0 + UNIX_EPOCH_DATE
}
