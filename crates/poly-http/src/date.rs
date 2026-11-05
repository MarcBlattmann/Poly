//! HTTP dates (RFC 9110 section 5.6.7, formerly RFC 2616 section 3.3):
//! the form `WinHttpTimeFromSystemTime` writes
//! (`Sun, 06 Nov 1994 08:49:37 GMT`) and the three forms
//! `WinHttpTimeToSystemTime` reads: that one, RFC 850's
//! (`Sunday, 06-Nov-94 08:49:37 GMT`) and ANSI C's asctime
//! (`Sun Nov  6 08:49:37 1994`).
//!
//! The day of the week is always worked out from the date, never trusted
//! from the caller: `SYSTEMTIME.wDayOfWeek` is redundant with the date and
//! a wrong one cannot be represented in the string anyway.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// A date and time as `SYSTEMTIME` carries it (`minwinbase.h`): UTC, no
/// time zone. `millis` takes no part in an HTTP date (the grammar has no
/// fraction) and is only carried for the round trip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DateTime {
    pub year: u16,
    pub month: u16,
    /// 0 is Sunday, 6 Saturday (`SYSTEMTIME.wDayOfWeek`). Written by
    /// [`parse_http_date`]; ignored by [`format_http_date`].
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub millis: u16,
}

/// The preferred HTTP date, `Sun, 06 Nov 1994 08:49:37 GMT`; `None` for a
/// date that does not exist (month 13, 30 February, hour 24, ...).
pub fn format_http_date(t: &DateTime) -> Option<String> {
    if t.month < 1 || t.month > 12 || t.day < 1 || t.day > days_in_month(t.year, t.month) {
        return None;
    }
    if t.hour > 23 || t.minute > 59 || t.second > 60 {
        return None;
    }
    let weekday = weekday(t.year, t.month, t.day);
    Some(format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        WEEKDAYS[weekday],
        t.day,
        MONTHS[(t.month - 1) as usize],
        t.year,
        t.hour,
        t.minute,
        t.second
    ))
}

/// Any of the three HTTP date forms; `None` when the string is not one, or
/// names a date that does not exist.
pub fn parse_http_date(s: &str) -> Option<DateTime> {
    let words: Vec<&str> = s.split_whitespace().collect();
    match words.len() {
        6 => rfc1123(&words),
        4 => rfc850(&words),
        5 => asctime(&words),
        _ => None,
    }
}

/// `Sun, 06 Nov 1994 08:49:37 GMT`.
fn rfc1123(w: &[&str]) -> Option<DateTime> {
    let weekday = weekday_name(w[0])?;
    let day = number(w[1], 1, 2)?;
    let month = month_name(w[2])?;
    let year = number(w[3], 4, 4)?;
    let (hour, minute, second) = time(w[4])?;
    zone(w[5])?;
    date(year, month, day, hour, minute, second, weekday)
}

/// `Sunday, 06-Nov-94 08:49:37 GMT`.
fn rfc850(w: &[&str]) -> Option<DateTime> {
    let weekday = weekday_name(w[0])?;
    let mut parts = w[1].split('-');
    let day = number(parts.next()?, 1, 2)?;
    let month = month_name(parts.next()?)?;
    let year = match number(parts.next()?, 2, 4)? {
        // RFC 9110 section 5.6.7: a two-digit year more than 50 years ahead
        // is the most recent year in the past with those digits.
        y if (70..100).contains(&y) => y + 1900,
        y if y < 100 => y + 2000,
        y => y,
    };
    if parts.next().is_some() {
        return None;
    }
    let (hour, minute, second) = time(w[2])?;
    zone(w[3])?;
    date(year, month, day, hour, minute, second, weekday)
}

/// `Sun Nov  6 08:49:37 1994`.
fn asctime(w: &[&str]) -> Option<DateTime> {
    let weekday = weekday_name(w[0])?;
    let month = month_name(w[1])?;
    let day = number(w[2], 1, 2)?;
    let (hour, minute, second) = time(w[3])?;
    let year = number(w[4], 4, 4)?;
    date(year, month, day, hour, minute, second, weekday)
}

#[allow(clippy::too_many_arguments)]
fn date(year: u16, month: u16, day: u16, hour: u16, minute: u16, second: u16, _weekday: u16) -> Option<DateTime> {
    if year < 1 || day < 1 || day > days_in_month(year, month) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some(DateTime { year, month, day, day_of_week: weekday(year, month, day) as u16, hour, minute, second, millis: 0 })
}

/// `hh:mm:ss`.
fn time(s: &str) -> Option<(u16, u16, u16)> {
    let b = s.as_bytes();
    if b.len() != 8 || !s.is_ascii() || b[2] != b':' || b[5] != b':' {
        return None;
    }
    Some((number(&s[0..2], 2, 2)?, number(&s[3..5], 2, 2)?, number(&s[6..8], 2, 2)?))
}

fn zone(s: &str) -> Option<()> {
    if s.eq_ignore_ascii_case("GMT") || s.eq_ignore_ascii_case("UTC") { Some(()) } else { None }
}

fn number(s: &str, min_digits: usize, max_digits: usize) -> Option<u16> {
    if s.len() < min_digits || s.len() > max_digits || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `Sun`, `Sunday`, any case; the trailing comma of the first two forms is
/// part of the word.
fn weekday_name(s: &str) -> Option<u16> {
    let s = s.strip_suffix(',').unwrap_or(s);
    WEEKDAYS
        .iter()
        .position(|w| s.len() >= 3 && s.is_char_boundary(3) && w.eq_ignore_ascii_case(&s[..3]))
        .map(|i| i as u16)
}

fn month_name(s: &str) -> Option<u16> {
    MONTHS.iter().position(|m| s.len() == 3 && m.eq_ignore_ascii_case(s)).map(|i| i as u16 + 1)
}

fn days_in_month(year: u16, month: u16) -> u16 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn leap(year: u16) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Sakamoto's method: 0 is Sunday, as `SYSTEMTIME.wDayOfWeek` counts.
fn weekday(year: u16, month: u16, day: u16) -> usize {
    const SHIFT: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = i32::from(year);
    if month < 3 {
        y -= 1;
    }
    let m = SHIFT[(month - 1) as usize];
    ((y + y / 4 - y / 100 + y / 400 + m + i32::from(day)) % 7) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(year: u16, month: u16, day: u16, h: u16, m: u16, s: u16) -> DateTime {
        DateTime { year, month, day, day_of_week: 0, hour: h, minute: m, second: s, millis: 0 }
    }

    #[test]
    fn formats_the_preferred_form() {
        assert_eq!(format_http_date(&t(1994, 11, 6, 8, 49, 37)).as_deref(), Some("Sun, 06 Nov 1994 08:49:37 GMT"));
        assert_eq!(format_http_date(&t(2000, 1, 1, 0, 0, 0)).as_deref(), Some("Sat, 01 Jan 2000 00:00:00 GMT"));
        assert_eq!(format_http_date(&t(2024, 2, 29, 23, 59, 59)).as_deref(), Some("Thu, 29 Feb 2024 23:59:59 GMT"));
    }

    #[test]
    fn refuses_dates_that_do_not_exist() {
        assert_eq!(format_http_date(&t(2023, 2, 29, 0, 0, 0)), None);
        assert_eq!(format_http_date(&t(1994, 13, 1, 0, 0, 0)), None);
        assert_eq!(format_http_date(&t(1994, 4, 31, 0, 0, 0)), None);
        assert_eq!(format_http_date(&t(1994, 1, 1, 24, 0, 0)), None);
        assert_eq!(format_http_date(&t(1994, 1, 1, 0, 60, 0)), None);
    }

    #[test]
    fn parses_all_three_forms() {
        let want = t(1994, 11, 6, 8, 49, 37);
        for s in [
            "Sun, 06 Nov 1994 08:49:37 GMT",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
            "sun, 06 nov 1994 08:49:37 gmt",
        ] {
            let got = parse_http_date(s).unwrap_or_else(|| panic!("{s}"));
            assert_eq!(
                (got.year, got.month, got.day, got.hour, got.minute, got.second),
                (want.year, want.month, want.day, want.hour, want.minute, want.second),
                "{s}"
            );
            assert_eq!(got.day_of_week, 0, "{s}");
        }
        assert_eq!(parse_http_date("Tue, 21 Nov 2000 01:06:53 GMT").unwrap().day_of_week, 2);
    }

    #[test]
    fn two_digit_years() {
        assert_eq!(parse_http_date("Sunday, 06-Nov-94 08:49:37 GMT").unwrap().year, 1994);
        assert_eq!(parse_http_date("Sunday, 06-Nov-69 08:49:37 GMT").unwrap().year, 2069);
        assert_eq!(parse_http_date("Sunday, 06-Nov-70 08:49:37 GMT").unwrap().year, 1970);
    }

    #[test]
    fn refuses_what_is_not_an_http_date() {
        for s in [
            "",
            "Sun, 06 Nov 1994 08:49:37",
            "Sun, 06 Nov 1994 08:49:37 PST",
            "Sun, 32 Nov 1994 08:49:37 GMT",
            "Sun, 06 Foo 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 08:49 GMT",
            "Sun, 06 Nov 1994 08:49:37 GMT extra",
            "Sunday, 06-Nov-1994-1 08:49:37 GMT",
        ] {
            assert_eq!(parse_http_date(s), None, "{s}");
        }
    }

    #[test]
    fn round_trips() {
        for (y, m, d, hh, mm, ss) in
            [(1994, 11, 6, 8, 49, 37), (1970, 1, 1, 0, 0, 0), (2000, 2, 29, 12, 0, 0), (2026, 11, 3, 23, 59, 59)]
        {
            let text = format_http_date(&t(y, m, d, hh, mm, ss)).unwrap();
            let back = parse_http_date(&text).unwrap();
            assert_eq!((back.year, back.month, back.day, back.hour, back.minute, back.second), (y, m, d, hh, mm, ss));
        }
    }
}
