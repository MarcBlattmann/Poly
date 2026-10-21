//! Calendar time (C11 7.27, Learn's `time`, `gmtime`, `localtime`,
//! `mktime`, `asctime`, `strftime` pages): `time_t` (seconds since
//! 1970-01-01 00:00:00 UTC) ↔ `struct tm` in the proleptic Gregorian
//! calendar, `asctime`'s text and `strftime` in the "C" locale.
//!
//! - `_gmtime64`/`_localtime64` accept 0 … 32535215999 (Learn: "up to
//!   23:59:59, December 31, 3000, UTC"); `_gmtime32` 0 … 0x7FFFFFFF.
//!   Anything else is `EINVAL` and NULL (Learn: "Negative values of
//!   sourceTime ... return NULL").
//! - `mktime` normalises every field (C11 7.27.2.3) and fails (−1) outside
//!   the same range.
//! - Poly's local time is UTC (ADR 0079), so `tm_isdst` is 0 and the time
//!   zone is "UTC" with a bias of 0 (to be confirmed by the spec agent:
//!   Windows shows the zone's full name in `%Z`).
//! - `asctime`: `"Wed Jan 02 02:03:55 1980\n"`, the day of the month with
//!   two digits as Learn's example shows.
//! - `strftime` in the "C" locale: `%c` is `%x %X`, `%x` is `%m/%d/%y`, `%X`
//!   is `%H:%M:%S` (to be confirmed: C11 gives `%a %b %e %T %Y` for `%c`),
//!   with the `#` flag removing leading zeros or giving the long forms
//!   (`%#c`: "Tuesday, March 14, 1995, 12:41:29", `%#x`: "Tuesday, March
//!   14, 1995").

use crate::unit::Unit;
use alloc::vec::Vec;

/// `struct tm` (`time.h`): nine `int`s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Tm {
    pub sec: i32,
    pub min: i32,
    pub hour: i32,
    pub mday: i32,
    pub mon: i32,
    pub year: i32,
    pub wday: i32,
    pub yday: i32,
    pub isdst: i32,
}

/// The last second `_gmtime64` accepts: 3000-12-31 23:59:59 UTC.
pub const MAX_TIME64: i64 = 32_535_215_999;
/// The last second of a 32-bit `time_t`.
pub const MAX_TIME32: i64 = 0x7FFF_FFFF;

/// Seconds between 1601-01-01 (the `FILETIME` epoch) and 1970-01-01.
pub const FILETIME_UNIX_DIFF: i64 = 11_644_473_600;

/// `time_t` from a `FILETIME` (100 ns units since 1601).
pub fn from_filetime(ft: u64) -> i64 {
    (ft / 10_000_000) as i64 - FILETIME_UNIX_DIFF
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

const DAYS: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

fn days_in_month(y: i64, m: usize) -> i64 {
    if m == 1 && is_leap(y) { 29 } else { DAYS[m] }
}

/// Days from 1970-01-01 to the first of January of `y`.
fn days_before_year(y: i64) -> i64 {
    let y1 = y - 1;
    // Days from 0001-01-01 to y-01-01, minus those to 1970-01-01.
    365 * y1 + y1.div_euclid(4) - y1.div_euclid(100) + y1.div_euclid(400) - 719_162
}

/// Days from 1970-01-01 to `y`-`m`(0-based)-01.
fn days_before(y: i64, m: usize) -> i64 {
    days_before_year(y) + (0..m).map(|k| days_in_month(y, k)).sum::<i64>()
}

/// `gmtime`'s conversion of any `time_t` (no range check).
pub fn to_tm(t: i64) -> Tm {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    // The year: estimate, then correct.
    let mut y = 1970 + days.div_euclid(365);
    while days_before_year(y) > days {
        y -= 1;
    }
    while days_before_year(y + 1) <= days {
        y += 1;
    }
    let yday = days - days_before_year(y);
    let mut m = 0;
    let mut rest = yday;
    while rest >= days_in_month(y, m) {
        rest -= days_in_month(y, m);
        m += 1;
    }
    Tm {
        sec: (secs % 60) as i32,
        min: (secs / 60 % 60) as i32,
        hour: (secs / 3600) as i32,
        mday: rest as i32 + 1,
        mon: m as i32,
        year: (y - 1900) as i32,
        // 1970-01-01 was a Thursday.
        wday: (days + 4).rem_euclid(7) as i32,
        yday: yday as i32,
        isdst: 0,
    }
}

/// `_gmtime64` (and `_localtime64`, UTC in Poly): `None` is `EINVAL`.
pub fn gmtime(t: i64, max: i64) -> Option<Tm> {
    (0..=max).contains(&t).then(|| to_tm(t))
}

/// `mktime`: normalise `tm` (fields may be out of range) and return its
/// `time_t`, or `None` (−1) when it is outside 1970 … `max`.
pub fn mktime(tm: &mut Tm, max: i64) -> Option<i64> {
    // Carry seconds into minutes and so on, then months into years.
    let mut sec = i64::from(tm.sec);
    let mut min = i64::from(tm.min) + sec.div_euclid(60);
    sec = sec.rem_euclid(60);
    let mut hour = i64::from(tm.hour) + min.div_euclid(60);
    min = min.rem_euclid(60);
    let day = i64::from(tm.mday) + hour.div_euclid(24); // 1-based
    hour = hour.rem_euclid(24);
    let year = 1900 + i64::from(tm.year) + i64::from(tm.mon).div_euclid(12);
    let mon = i64::from(tm.mon).rem_euclid(12) as usize;
    if !(1..=100_000).contains(&year) {
        return None;
    }
    let days = days_before(year, mon) + day - 1;
    let t = days * 86_400 + hour * 3600 + min * 60 + sec;
    if !(0..=max).contains(&t) {
        return None;
    }
    *tm = to_tm(t);
    Some(t)
}

const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn short(s: &str) -> &str {
    &s[..3]
}

/// `asctime`: 25 characters and a newline.
pub fn asctime(tm: &Tm) -> Option<Vec<u8>> {
    let wday = WEEKDAYS.get(usize::try_from(tm.wday).ok()?)?;
    let mon = MONTHS.get(usize::try_from(tm.mon).ok()?)?;
    let s = alloc::format!(
        "{} {} {:02} {:02}:{:02}:{:02} {}\n",
        short(wday),
        short(mon),
        tm.mday,
        tm.hour,
        tm.min,
        tm.sec,
        1900 + tm.year
    );
    Some(s.into_bytes())
}

/// The time zone `strftime`'s `%Z` and `_tzname` show.
pub const ZONE: &str = "UTC";

/// ISO 8601 week-based year and week (`%G`, `%V`).
fn iso_week(tm: &Tm) -> (i64, i64) {
    let year = 1900 + i64::from(tm.year);
    let wday = i64::from((tm.wday + 6) % 7); // Monday = 0
    let yday = i64::from(tm.yday);
    // The Thursday of this week decides the year.
    let thursday = yday - wday + 3;
    let len = |y: i64| if is_leap(y) { 366 } else { 365 };
    if thursday < 0 {
        let py = year - 1;
        let t = thursday + len(py);
        (py, t / 7 + 1)
    } else if thursday >= len(year) {
        (year + 1, 1)
    } else {
        (year, thursday / 7 + 1)
    }
}

/// `strftime` / `wcsftime` in the "C" locale: the output without its NUL,
/// or `None` for an invalid conversion (the invalid parameter handler,
/// `EINVAL`; Learn: "If format contains an invalid formatting code").
pub fn strftime<C: Unit>(format: &[C], tm: &Tm) -> Option<Vec<C>> {
    let mut out: Vec<C> = Vec::new();
    let mut i = 0;
    let push = |out: &mut Vec<C>, s: &str| out.extend(s.bytes().map(C::ascii));
    while i < format.len() {
        let c = format[i];
        if c.to_u32() != u32::from(b'%') {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        let mut alt = false;
        if format.get(i).map(|c| c.to_u32()) == Some(u32::from(b'#')) {
            alt = true;
            i += 1;
        }
        let conv = format.get(i)?.as_ascii()?;
        i += 1;
        let year = 1900 + i64::from(tm.year);
        let num = |v: i64, width: usize| {
            if alt { alloc::format!("{v}") } else { alloc::format!("{v:0width$}") }
        };
        let hour12 = if tm.hour % 12 == 0 { 12 } else { tm.hour % 12 };
        let wday = WEEKDAYS.get(usize::try_from(tm.wday).ok()?.min(6)).copied().unwrap_or("?");
        let mon = MONTHS.get(usize::try_from(tm.mon).ok()?.min(11)).copied().unwrap_or("?");
        let s = match conv {
            b'a' => alloc::string::String::from(short(wday)),
            b'A' => alloc::string::String::from(wday),
            b'b' | b'h' => alloc::string::String::from(short(mon)),
            b'B' => alloc::string::String::from(mon),
            b'c' if alt => {
                alloc::format!("{wday}, {mon} {}, {year}, {:02}:{:02}:{:02}", tm.mday, tm.hour, tm.min, tm.sec)
            }
            b'c' => alloc::format!(
                "{:02}/{:02}/{:02} {:02}:{:02}:{:02}",
                tm.mon + 1,
                tm.mday,
                year.rem_euclid(100),
                tm.hour,
                tm.min,
                tm.sec
            ),
            b'C' => num(year.div_euclid(100), 2),
            b'd' => num(i64::from(tm.mday), 2),
            b'D' => alloc::format!("{:02}/{:02}/{:02}", tm.mon + 1, tm.mday, year.rem_euclid(100)),
            b'e' => alloc::format!("{:2}", tm.mday),
            b'F' => alloc::format!("{year:04}-{:02}-{:02}", tm.mon + 1, tm.mday),
            b'g' => num(iso_week(tm).0.rem_euclid(100), 2),
            b'G' => alloc::format!("{}", iso_week(tm).0),
            b'H' => num(i64::from(tm.hour), 2),
            b'I' => num(i64::from(hour12), 2),
            b'j' => num(i64::from(tm.yday) + 1, 3),
            b'm' => num(i64::from(tm.mon) + 1, 2),
            b'M' => num(i64::from(tm.min), 2),
            b'n' => alloc::string::String::from("\n"),
            b'p' => alloc::string::String::from(if tm.hour < 12 { "AM" } else { "PM" }),
            b'r' => {
                alloc::format!("{:02}:{:02}:{:02} {}", hour12, tm.min, tm.sec, if tm.hour < 12 { "AM" } else { "PM" })
            }
            b'R' => alloc::format!("{:02}:{:02}", tm.hour, tm.min),
            b'S' => num(i64::from(tm.sec), 2),
            b't' => alloc::string::String::from("\t"),
            b'T' | b'X' => alloc::format!("{:02}:{:02}:{:02}", tm.hour, tm.min, tm.sec),
            b'u' => alloc::format!("{}", if tm.wday == 0 { 7 } else { tm.wday }),
            b'U' => num((i64::from(tm.yday) + 7 - i64::from(tm.wday)) / 7, 2),
            b'V' => num(iso_week(tm).1, 2),
            b'w' => alloc::format!("{}", tm.wday),
            b'W' => num((i64::from(tm.yday) + 7 - i64::from((tm.wday + 6) % 7)) / 7, 2),
            b'x' if alt => alloc::format!("{wday}, {mon} {}, {year}", tm.mday),
            b'x' => alloc::format!("{:02}/{:02}/{:02}", tm.mon + 1, tm.mday, year.rem_euclid(100)),
            b'y' => num(year.rem_euclid(100), 2),
            b'Y' => alloc::format!("{year}"),
            b'z' => alloc::string::String::from("+0000"),
            b'Z' => alloc::string::String::from(ZONE),
            b'%' => alloc::string::String::from("%"),
            _ => return None,
        };
        push(&mut out, &s);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    fn tm(y: i32, mon: i32, d: i32, h: i32, mi: i32, s: i32) -> Tm {
        Tm { sec: s, min: mi, hour: h, mday: d, mon, year: y - 1900, ..Tm::default() }
    }

    #[test]
    fn epoch_and_known_dates() {
        let t = to_tm(0);
        assert_eq!((t.year, t.mon, t.mday, t.wday, t.yday), (70, 0, 1, 4, 0));
        let t = to_tm(951_782_400); // 2000-02-29 00:00:00
        assert_eq!((t.year, t.mon, t.mday, t.wday, t.yday), (100, 1, 29, 2, 59));
        let t = to_tm(MAX_TIME64);
        assert_eq!((t.year + 1900, t.mon, t.mday, t.hour, t.min, t.sec), (3000, 11, 31, 23, 59, 59));
        let t = to_tm(1_700_000_000); // 2023-11-14 22:13:20, a Tuesday
        assert_eq!((t.year + 1900, t.mon + 1, t.mday, t.hour, t.min, t.sec, t.wday), (2023, 11, 14, 22, 13, 20, 2));
        assert_eq!(gmtime(-1, MAX_TIME64), None);
        assert_eq!(gmtime(MAX_TIME64 + 1, MAX_TIME64), None);
        assert!(gmtime(MAX_TIME32, MAX_TIME32).is_some() && gmtime(MAX_TIME32 + 1, MAX_TIME32).is_none());
        assert_eq!(from_filetime(116_444_736_000_000_000), 0);
    }

    #[test]
    fn mktime_normalises() {
        let mut t = tm(2023, 10, 14, 22, 13, 20);
        assert_eq!(mktime(&mut t, MAX_TIME64), Some(1_700_000_000));
        assert_eq!((t.wday, t.yday), (2, 317));
        // 2024-01-31 + 1 month -> March 2.
        let mut t = tm(2024, 1, 31, 0, 0, 0);
        mktime(&mut t, MAX_TIME64).unwrap();
        assert_eq!((t.mon, t.mday), (2, 2));
        let mut t = tm(2000, 0, 1, 0, 0, -1);
        mktime(&mut t, MAX_TIME64).unwrap();
        assert_eq!((t.year, t.mon, t.mday, t.hour, t.min, t.sec), (99, 11, 31, 23, 59, 59));
        let mut t = tm(2000, -1, 1, 0, 0, 0);
        mktime(&mut t, MAX_TIME64).unwrap();
        assert_eq!((t.year, t.mon), (99, 11));
        assert_eq!(mktime(&mut tm(1969, 11, 31, 23, 59, 59), MAX_TIME64), None);
        assert_eq!(mktime(&mut tm(2038, 0, 19, 3, 14, 8), MAX_TIME32), None);
        assert_eq!(mktime(&mut tm(3001, 0, 1, 0, 0, 0), MAX_TIME64), None);
    }

    #[test]
    fn asctime_text() {
        let t = to_tm(315_626_635); // 1980-01-02 02:03:55, a Wednesday
        assert_eq!(asctime(&t).unwrap(), b"Wed Jan 02 02:03:55 1980\n");
        assert_eq!(asctime(&Tm { wday: 9, ..t }), None);
    }

    fn sf(f: &str, t: &Tm) -> String {
        String::from_utf8(strftime(f.as_bytes(), t).unwrap()).unwrap()
    }

    #[test]
    fn strftime_c_locale() {
        let t = to_tm(795_184_889); // 1995-03-14 12:41:29, a Tuesday
        assert_eq!(
            sf("%a %A %b %B %d %H %I %j %m %M %p %S %w %y %Y %%", &t),
            "Tue Tuesday Mar March 14 12 12 073 03 41 PM 29 2 95 1995 %"
        );
        assert_eq!(sf("%c|%x|%X", &t), "03/14/95 12:41:29|03/14/95|12:41:29");
        assert_eq!(sf("%#c|%#x", &t), "Tuesday, March 14, 1995, 12:41:29|Tuesday, March 14, 1995");
        assert_eq!(sf("%#d %#m %#j %#H", &to_tm(0)), "1 1 1 0");
        assert_eq!(
            sf("%D %F %R %T %e %C %u %z %Z %n%t", &t),
            "03/14/95 1995-03-14 12:41 12:41:29 14 19 2 +0000 UTC \n\t"
        );
        assert_eq!(sf("%U %W %V %G %g", &t), "11 11 11 1995 95");
        // 2021-01-01 is a Friday in ISO week 53 of 2020.
        let ny = to_tm(1_609_459_200);
        assert_eq!(sf("%V %G %U %W", &ny), "53 2020 00 00");
        assert_eq!(sf("%r", &to_tm(0)), "12:00:00 AM");
        assert_eq!(strftime(b"%Q", &t), None);
        assert_eq!(strftime(b"%", &t), None);
        let w: Vec<u16> = "%Y-%m".encode_utf16().collect();
        assert_eq!(String::from_utf16(&strftime(&w, &t).unwrap()).unwrap(), "1995-03");
    }
}
