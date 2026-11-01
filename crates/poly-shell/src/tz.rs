//! Time zones for the ridge's clock (ADR 0117): the POSIX `TZ` rule
//! format (POSIX.1-2017, XBD 8.3 "Other Environment Variables", `TZ`:
//! `std offset [dst [offset] [,start[/time],end[/time]]]`, with the `Jn`,
//! `n` and `Mm.w.d` date forms) and a short list of named zones with the
//! rules their regions use today (the clock rules of those countries,
//! public facts; no time zone database is read).
//!
//! Offsets in the format are west of Greenwich (`CET-1` is one hour
//! *east*); [`TimeZone::offset_minutes`] returns minutes east of UTC, as
//! the clock wants.

use alloc::string::String;

/// The zones the settings offer: (name, label, POSIX rule).
pub const ZONES: &[(&str, &str, &str)] = &[
    ("UTC", "UTC", "UTC0"),
    ("Europe/Zurich", "Zurich", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Europe/Berlin", "Berlin", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Europe/London", "London", "GMT0BST,M3.5.0/1,M10.5.0"),
    ("America/New_York", "New York", "EST5EDT,M3.2.0,M11.1.0"),
    ("America/Los_Angeles", "Los Angeles", "PST8PDT,M3.2.0,M11.1.0"),
    ("Asia/Tokyo", "Tokyo", "JST-9"),
    ("Asia/Kolkata", "Kolkata", "IST-5:30"),
    ("Australia/Sydney", "Sydney", "AEST-10AEDT,M10.1.0,M4.1.0/3"),
];

/// A day of the year a rule names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Day {
    /// `Jn`: 1 to 365, February 29 never counted.
    Julian1(u32),
    /// `n`: 0 to 365, February 29 counted.
    Julian0(u32),
    /// `Mm.w.d`: day `d` (0 Sunday) of week `w` (5: the last) of month `m`.
    Month { m: u32, w: u32, d: u32 },
}

/// A transition: the day and the local time of day (seconds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rule {
    day: Day,
    time: i64,
}

/// A parsed `TZ` rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeZone {
    pub std_name: String,
    /// Seconds east of UTC (the format's sign flipped).
    std_east: i64,
    pub dst_name: Option<String>,
    dst_east: i64,
    rules: Option<(Rule, Rule)>,
}

struct Cursor<'a> {
    s: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn number(&mut self) -> Option<i64> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        core::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()
    }

    /// A zone abbreviation: three or more letters, or `<...>`.
    fn name(&mut self) -> Option<String> {
        let start = self.i;
        if self.eat(b'<') {
            while self.peek().is_some_and(|c| c != b'>') {
                self.i += 1;
            }
            let name = core::str::from_utf8(&self.s[start + 1..self.i]).ok()?.into();
            return self.eat(b'>').then_some(name);
        }
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.i += 1;
        }
        (self.i - start >= 3).then(|| String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }

    /// `[+-]hh[:mm[:ss]]` in seconds.
    fn duration(&mut self) -> Option<i64> {
        let sign = if self.eat(b'-') {
            -1
        } else {
            self.eat(b'+');
            1
        };
        let mut secs = self.number()? * 3600;
        if self.eat(b':') {
            secs += self.number()? * 60;
            if self.eat(b':') {
                secs += self.number()?;
            }
        }
        Some(sign * secs)
    }

    fn rule(&mut self) -> Option<Rule> {
        let day = if self.eat(b'J') {
            Day::Julian1(self.number()? as u32)
        } else if self.eat(b'M') {
            let m = self.number()? as u32;
            self.eat(b'.').then_some(())?;
            let w = self.number()? as u32;
            self.eat(b'.').then_some(())?;
            let d = self.number()? as u32;
            if !(1..=12).contains(&m) || !(1..=5).contains(&w) || d > 6 {
                return None;
            }
            Day::Month { m, w, d }
        } else {
            Day::Julian0(self.number()? as u32)
        };
        let time = if self.eat(b'/') { self.duration()? } else { 2 * 3600 };
        Some(Rule { day, time })
    }
}

/// Days from 1970-01-01 to `y`-`m`-`d` (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn month_days(y: i64, m: u32) -> u32 {
    match m {
        2 if leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl Day {
    /// The day's number since 1970-01-01 in year `y`.
    fn in_year(self, y: i64) -> i64 {
        let jan1 = days_from_civil(y, 1, 1);
        match self {
            Day::Julian0(n) => jan1 + i64::from(n),
            Day::Julian1(n) => jan1 + i64::from(n) - 1 + i64::from(leap(y) && n >= 60),
            Day::Month { m, w, d } => {
                let first = days_from_civil(y, m, 1);
                // 1970-01-01 was a Thursday (4).
                let wd1 = (first + 4).rem_euclid(7) as u32;
                let mut day = 1 + (d + 7 - wd1) % 7 + 7 * (w - 1);
                while day > month_days(y, m) {
                    day -= 7;
                }
                first + i64::from(day) - 1
            }
        }
    }
}

impl TimeZone {
    /// Parse a POSIX `TZ` rule (a leading `:` is not supported).
    pub fn parse(text: &str) -> Option<TimeZone> {
        let mut c = Cursor { s: text.as_bytes(), i: 0 };
        let std_name = c.name()?;
        let std_east = -c.duration()?;
        let mut tz = TimeZone { std_name, std_east, dst_name: None, dst_east: std_east + 3600, rules: None };
        if c.peek().is_none() {
            return Some(tz);
        }
        tz.dst_name = Some(c.name()?);
        if c.peek().is_some_and(|ch| ch != b',') {
            tz.dst_east = -c.duration()?;
        }
        if c.eat(b',') {
            let start = c.rule()?;
            c.eat(b',').then_some(())?;
            let end = c.rule()?;
            tz.rules = Some((start, end));
        } else {
            // No rules: the United States' current ones are the usual
            // default; ours is simply no daylight saving time.
            tz.dst_name = None;
        }
        (c.peek().is_none()).then_some(tz)
    }

    /// A named zone of [`ZONES`], or a POSIX rule.
    pub fn named(name: &str) -> Option<TimeZone> {
        let rule = ZONES.iter().find(|z| z.0 == name).map_or(name, |z| z.2);
        TimeZone::parse(rule)
    }

    /// Whether daylight saving time is in effect at `utc` (Unix seconds).
    pub fn is_dst(&self, utc: i64) -> bool {
        let Some((start, end)) = self.rules else { return false };
        // The year in local standard time.
        let local = utc + self.std_east;
        let year = crate::clock::civil(local, 0).year;
        // Start is given in standard time, end in daylight time.
        let at = |r: Rule, east: i64| r.day.in_year(year) * 86_400 + r.time - east;
        let (s, e) = (at(start, self.std_east), at(end, self.dst_east));
        if s < e { utc >= s && utc < e } else { !(utc >= e && utc < s) }
    }

    /// Minutes east of UTC at `utc` (Unix seconds).
    pub fn offset_minutes(&self, utc: i64) -> i32 {
        let east = if self.is_dst(utc) { self.dst_east } else { self.std_east };
        (east / 60) as i32
    }

    /// The abbreviation in effect at `utc`.
    pub fn abbreviation(&self, utc: i64) -> &str {
        match (&self.dst_name, self.is_dst(utc)) {
            (Some(d), true) => d,
            _ => &self.std_name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unix seconds of a UTC date and time.
    fn utc(y: i64, m: u32, d: u32, h: i64, min: i64) -> i64 {
        days_from_civil(y, m, d) * 86_400 + h * 3600 + min * 60
    }

    #[test]
    fn days() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        // The last Sunday of March 2026 is the 29th; of October the 25th.
        let m = Day::Month { m: 3, w: 5, d: 0 };
        assert_eq!(m.in_year(2026), days_from_civil(2026, 3, 29));
        assert_eq!(Day::Month { m: 10, w: 5, d: 0 }.in_year(2026), days_from_civil(2026, 10, 25));
        // The second Sunday of March 2026: the 8th; the first of November: the 1st.
        assert_eq!(Day::Month { m: 3, w: 2, d: 0 }.in_year(2026), days_from_civil(2026, 3, 8));
        assert_eq!(Day::Month { m: 11, w: 1, d: 0 }.in_year(2026), days_from_civil(2026, 11, 1));
        assert_eq!(Day::Julian1(60).in_year(2024), days_from_civil(2024, 3, 1), "J60 skips Feb 29");
        assert_eq!(Day::Julian0(59).in_year(2024), days_from_civil(2024, 2, 29));
    }

    #[test]
    fn central_europe() {
        let z = TimeZone::named("Europe/Zurich").unwrap();
        assert_eq!(z.offset_minutes(utc(2026, 1, 15, 12, 0)), 60);
        assert_eq!(z.offset_minutes(utc(2026, 7, 1, 12, 0)), 120);
        assert_eq!(z.abbreviation(utc(2026, 7, 1, 12, 0)), "CEST");
        // Summer time starts at 01:00 UTC on 2026-03-29, ends at 01:00 UTC on 2026-10-25.
        assert_eq!(z.offset_minutes(utc(2026, 3, 29, 0, 59)), 60);
        assert_eq!(z.offset_minutes(utc(2026, 3, 29, 1, 0)), 120);
        assert_eq!(z.offset_minutes(utc(2026, 10, 25, 0, 59)), 120);
        assert_eq!(z.offset_minutes(utc(2026, 10, 25, 1, 0)), 60);
        // The ridge shows local time.
        let t = utc(2026, 11, 1, 0, 0);
        assert_eq!(crate::clock::ridge_text(&crate::clock::civil(t, z.offset_minutes(t))), "01:00 Nov 1");
    }

    #[test]
    fn other_zones() {
        let ny = TimeZone::named("America/New_York").unwrap();
        assert_eq!(ny.offset_minutes(utc(2026, 1, 1, 12, 0)), -300);
        assert_eq!(ny.offset_minutes(utc(2026, 3, 8, 6, 59)), -300);
        assert_eq!(ny.offset_minutes(utc(2026, 3, 8, 7, 0)), -240, "02:00 EST");
        let syd = TimeZone::named("Australia/Sydney").unwrap();
        assert_eq!(syd.offset_minutes(utc(2026, 1, 1, 0, 0)), 660, "southern summer");
        assert_eq!(syd.offset_minutes(utc(2026, 7, 1, 0, 0)), 600);
        assert_eq!(TimeZone::named("Asia/Kolkata").unwrap().offset_minutes(0), 330);
        assert_eq!(TimeZone::named("UTC").unwrap().offset_minutes(12345), 0);
        // Raw rules, and nonsense.
        assert_eq!(TimeZone::parse("<+03>-3").unwrap().offset_minutes(0), 180);
        assert!(TimeZone::parse("X1").is_none(), "a name of three letters at least");
        assert!(TimeZone::parse("CET-1CEST,M13.5.0,M10.5.0").is_none());
        assert!(TimeZone::named("Mars/Olympus").is_none());
        for (name, _, _) in ZONES {
            assert!(TimeZone::named(name).is_some(), "{name}");
        }
    }
}
