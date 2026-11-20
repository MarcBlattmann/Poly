//! Calendar time (C11 7.27, POSIX): `struct tm` as the SDK lays it out
//! (`<time.h>`: nine `int`s, then `long tm_gmtoff` and `char *tm_zone`),
//! the conversions between seconds since the Epoch and the proleptic
//! Gregorian calendar, the normalisation `mktime` and `timegm` do, and
//! `strftime` in the C locale.
//!
//! Poly has no time zone database yet: local time is UTC, named "UTC".

/// `struct tm` (`<time.h>`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub gmtoff: i64,
    pub zone: *const u8,
}

impl Default for Tm {
    fn default() -> Self {
        Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: core::ptr::null() }
    }
}

const DAYS_BEFORE: [i32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

fn leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Days from 1970-01-01 to `year-month-day` (month 1..=12), proleptic
/// Gregorian.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    // Count from 0000-03-01 so the leap day ends the year.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of day `z` (days since 1970-01-01): (year, month 1..=12, day).
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

/// `gmtime`: the broken-down UTC time of `t`; `None` if the year does not
/// fit an `int` (`EOVERFLOW`).
pub fn gmtime(t: i64) -> Option<Tm> {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400) as i32;
    let (y, m, d) = civil_from_days(days);
    let year = i32::try_from(y - 1900).ok()?;
    let yday = DAYS_BEFORE[m as usize - 1] + d as i32 - 1 + i32::from(m > 2 && leap(y));
    Some(Tm {
        sec: secs % 60,
        min: secs / 60 % 60,
        hour: secs / 3600,
        mday: d as i32,
        mon: m as i32 - 1,
        year,
        wday: (days + 4).rem_euclid(7) as i32,
        yday,
        isdst: 0,
        gmtoff: 0,
        zone: core::ptr::null(),
    })
}

/// `timegm` (and `mktime` in UTC): the seconds of `tm`, whose fields may
/// be out of their ranges; `tm` is normalised as `mktime` does (the
/// weekday and day of the year filled in). `None` if out of range.
pub fn timegm(tm: &mut Tm) -> Option<i64> {
    let mon = i64::from(tm.mon);
    let year = i64::from(tm.year) + 1900 + mon.div_euclid(12);
    let mon = mon.rem_euclid(12);
    if year.abs() > 1 << 40 {
        return None;
    }
    let days = days_from_civil(year, mon as u32 + 1, 1) + i64::from(tm.mday) - 1;
    let t = days
        .checked_mul(86_400)?
        .checked_add(i64::from(tm.hour) * 3600 + i64::from(tm.min) * 60 + i64::from(tm.sec))?;
    let out = gmtime(t)?;
    *tm = Tm { zone: tm.zone, ..out };
    Some(t)
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
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

/// Where `strftime` writes: a buffer it must not overrun.
struct Out<'a> {
    buf: &'a mut [u8],
    len: usize,
    full: bool,
}

impl Out<'_> {
    fn put(&mut self, s: &[u8]) {
        if self.len + s.len() > self.buf.len() {
            self.full = true;
            return;
        }
        self.buf[self.len..self.len + s.len()].copy_from_slice(s);
        self.len += s.len();
    }

    fn num(&mut self, v: i64, width: usize, pad: u8) {
        let mut tmp = [0u8; 24];
        let mut n = 0;
        let neg = v < 0;
        let mut u = v.unsigned_abs();
        loop {
            tmp[n] = b'0' + (u % 10) as u8;
            n += 1;
            u /= 10;
            if u == 0 {
                break;
            }
        }
        if neg {
            self.put(b"-");
        }
        for _ in n..width {
            self.put(&[pad]);
        }
        tmp[..n].reverse();
        self.put(&tmp[..n]);
    }
}

fn name(list: &[&'static str], i: i32) -> &'static str {
    usize::try_from(i).ok().and_then(|i| list.get(i)).copied().unwrap_or("?")
}

/// The ISO 8601 week-based year and week of `tm` (for `%G`, `%g`, `%V`).
fn iso_week(tm: &Tm) -> (i64, i32) {
    let year = i64::from(tm.year) + 1900;
    let wday = (tm.wday + 6) % 7; // Monday = 0
    let week = (tm.yday - wday + 10) / 7;
    let weeks_in = |y: i64| {
        let jan1 = (days_from_civil(y, 1, 1) + 4).rem_euclid(7); // Sunday = 0
        let p = |d: i64| d == 4 || (d == 3 && leap(y));
        if p(jan1) { 53 } else { 52 }
    };
    if week < 1 {
        (year - 1, weeks_in(year - 1))
    } else if week > weeks_in(year) {
        (year + 1, 1)
    } else {
        (year, week)
    }
}

/// `strftime` in the C locale: the bytes written (without the NUL), or 0
/// if they and the NUL do not fit in `buf`. `zone` names the time zone
/// for `%Z`.
pub fn strftime(buf: &mut [u8], fmt: &[u8], tm: &Tm, zone: &[u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    let room = buf.len() - 1;
    let mut out = Out { buf: &mut buf[..room], len: 0, full: false };
    format(&mut out, fmt, tm, zone);
    let (full, n) = (out.full, out.len);
    if full {
        return 0;
    }
    buf[n] = 0;
    n
}

fn format(out: &mut Out<'_>, fmt: &[u8], tm: &Tm, zone: &[u8]) {
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        i += 1;
        if c != b'%' || i >= fmt.len() {
            out.put(&[c]);
            continue;
        }
        let mut spec = fmt[i];
        i += 1;
        // The E and O modifiers have no other form in the C locale.
        if (spec == b'E' || spec == b'O') && i < fmt.len() {
            spec = fmt[i];
            i += 1;
        }
        let year = i64::from(tm.year) + 1900;
        let hour12 = if tm.hour % 12 == 0 { 12 } else { tm.hour % 12 };
        match spec {
            b'a' => out.put(&name(&DAYS, tm.wday).as_bytes()[..3]),
            b'A' => out.put(name(&DAYS, tm.wday).as_bytes()),
            b'b' | b'h' => out.put(&name(&MONTHS, tm.mon).as_bytes()[..3]),
            b'B' => out.put(name(&MONTHS, tm.mon).as_bytes()),
            b'c' => format(out, b"%a %b %e %H:%M:%S %Y", tm, zone),
            b'C' => out.num(year.div_euclid(100), 2, b'0'),
            b'd' => out.num(tm.mday.into(), 2, b'0'),
            b'D' => format(out, b"%m/%d/%y", tm, zone),
            b'e' => out.num(tm.mday.into(), 2, b' '),
            b'F' => format(out, b"%Y-%m-%d", tm, zone),
            b'g' => out.num(iso_week(tm).0.rem_euclid(100), 2, b'0'),
            b'G' => out.num(iso_week(tm).0, 0, b'0'),
            b'H' => out.num(tm.hour.into(), 2, b'0'),
            b'I' => out.num(hour12.into(), 2, b'0'),
            b'j' => out.num((tm.yday + 1).into(), 3, b'0'),
            b'k' => out.num(tm.hour.into(), 2, b' '),
            b'l' => out.num(hour12.into(), 2, b' '),
            b'm' => out.num((tm.mon + 1).into(), 2, b'0'),
            b'M' => out.num(tm.min.into(), 2, b'0'),
            b'n' => out.put(b"\n"),
            b'p' => out.put(if tm.hour < 12 { b"AM" } else { b"PM" }),
            b'r' => format(out, b"%I:%M:%S %p", tm, zone),
            b'R' => format(out, b"%H:%M", tm, zone),
            b's' => {
                let mut copy = *tm;
                out.num(timegm(&mut copy).unwrap_or(0) - tm.gmtoff, 0, b'0');
            }
            b'S' => out.num(tm.sec.into(), 2, b'0'),
            b't' => out.put(b"\t"),
            b'T' => format(out, b"%H:%M:%S", tm, zone),
            b'u' => out.num(if tm.wday == 0 { 7 } else { tm.wday.into() }, 0, b'0'),
            b'U' => out.num(((tm.yday + 7 - tm.wday) / 7).into(), 2, b'0'),
            b'V' => out.num(iso_week(tm).1.into(), 2, b'0'),
            b'w' => out.num(tm.wday.into(), 0, b'0'),
            b'W' => out.num(((tm.yday + 7 - (tm.wday + 6) % 7) / 7).into(), 2, b'0'),
            b'x' => format(out, b"%m/%d/%y", tm, zone),
            b'X' => format(out, b"%H:%M:%S", tm, zone),
            b'y' => out.num(year.rem_euclid(100), 2, b'0'),
            b'Y' => out.num(year, 0, b'0'),
            b'z' => {
                let off = tm.gmtoff;
                out.put(if off < 0 { b"-" } else { b"+" });
                let a = off.abs();
                out.num(a / 3600 * 100 + a / 60 % 60, 4, b'0');
            }
            b'Z' => out.put(zone),
            b'%' => out.put(b"%"),
            other => out.put(&[b'%', other]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tm_of(t: i64) -> Tm {
        gmtime(t).unwrap()
    }

    fn fmt(f: &str, t: i64) -> alloc::string::String {
        let mut buf = [0u8; 128];
        let n = strftime(&mut buf, f.as_bytes(), &tm_of(t), b"UTC");
        alloc::string::String::from_utf8(buf[..n].to_vec()).unwrap()
    }

    #[test]
    fn epoch_and_round_trips() {
        let t = tm_of(0);
        assert_eq!((t.year, t.mon, t.mday, t.hour, t.wday, t.yday), (70, 0, 1, 0, 4, 0));
        // 2000-02-29 12:34:56 UTC is 951827696.
        let t = tm_of(951_827_696);
        assert_eq!((t.year, t.mon, t.mday, t.hour, t.min, t.sec, t.wday, t.yday), (100, 1, 29, 12, 34, 56, 2, 59));
        let t = tm_of(-1);
        assert_eq!((t.year, t.mon, t.mday, t.hour, t.min, t.sec), (69, 11, 31, 23, 59, 59));
        for &s in &[0i64, 1, 86_399, 951_827_696, -86_401, 4_102_444_800, -12_219_292_800, 253_402_300_799] {
            let mut tm = tm_of(s);
            assert_eq!(timegm(&mut tm), Some(s));
        }
        for d in (-800_000i64..800_000).step_by(997) {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
    }

    #[test]
    fn normalisation() {
        // 2026-13-01 is 2027-01-01; Jan 32 is Feb 1; -1 seconds.
        let mut tm = Tm { year: 126, mon: 12, mday: 1, ..Tm::default() };
        let t = timegm(&mut tm).unwrap();
        assert_eq!((tm.year, tm.mon, tm.mday), (127, 0, 1));
        assert_eq!(t, days_from_civil(2027, 1, 1) * 86_400);
        let mut tm = Tm { year: 126, mon: 0, mday: 32, sec: -1, ..Tm::default() };
        timegm(&mut tm).unwrap();
        assert_eq!((tm.mon, tm.mday, tm.hour, tm.min, tm.sec), (0, 31, 23, 59, 59));
    }

    #[test]
    fn strftime_c_locale() {
        let t = 951_827_696; // Tue Feb 29 12:34:56 2000
        assert_eq!(fmt("%c", t), "Tue Feb 29 12:34:56 2000");
        assert_eq!(fmt("%Y-%m-%d %H:%M:%S %j %a %A %b %B %p %I %y %C", t),
            "2000-02-29 12:34:56 060 Tue Tuesday Feb February PM 12 00 20");
        assert_eq!(fmt("%x %X %D %F %T %R %e|%k|%l", t), "02/29/00 12:34:56 02/29/00 2000-02-29 12:34:56 12:34 29|12|12");
        assert_eq!(fmt("%z %Z %% %u %w %s", t), "+0000 UTC % 2 2 951827696");
        // ISO weeks: 2021-01-03 (a Sunday) is in week 53 of 2020.
        let t = days_from_civil(2021, 1, 3) * 86_400;
        assert_eq!(fmt("%G %V %g %U %W", t), "2020 53 20 01 00");
        let t = days_from_civil(2026, 12, 31) * 86_400;
        assert_eq!(fmt("%G-W%V-%u", t), "2026-W53-4");
        // Too small: 0, and nothing past the buffer.
        let mut small = [0xffu8; 4];
        assert_eq!(strftime(&mut small, b"%Y-%m", &tm_of(t), b"UTC"), 0);
        let mut exact = [0u8; 5];
        assert_eq!(strftime(&mut exact, b"%Y", &tm_of(t), b"UTC"), 4);
        assert_eq!(&exact, b"2026\0");
    }
}
