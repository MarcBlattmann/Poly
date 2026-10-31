//! Certificate times (RFC 5280 4.1.2.5) as Windows `FILETIME` values: 100
//! ns intervals since 1601-01-01 00:00 UTC (Learn: "FILETIME structure"),
//! the form `CERT_INFO` holds them in.

use crate::der::{Error, tag};

/// FILETIME ticks per second.
pub const TICKS_PER_SECOND: u64 = 10_000_000;

/// Days from 1601-01-01 to the given proleptic Gregorian date.
fn days_since_1601(year: i64, month: u32, day: u32) -> i64 {
    // Days from 0000-03-01 (a March-based year, so the leap day is last).
    let (y, m) = if month <= 2 { (year - 1, month + 9) } else { (year, month - 3) };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * i64::from(m) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe;
    // 1601-01-01 in the same count.
    const EPOCH_1601: i64 = 584_694;
    days - EPOCH_1601
}

/// The FILETIME of a UTC date and time; None for dates before 1601.
pub fn filetime(year: i64, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> Option<u64> {
    let days = days_since_1601(year, month, day);
    if days < 0 {
        return None;
    }
    let secs = days as u64 * 86_400 + u64::from(hour) * 3600 + u64::from(minute) * 60 + u64::from(second);
    secs.checked_mul(TICKS_PER_SECOND)
}

/// The UTC date and time of a FILETIME: (year, month, day, hour, minute,
/// second).
pub fn civil(ft: u64) -> (i64, u32, u32, u32, u32, u32) {
    let secs = ft / TICKS_PER_SECOND;
    let days = (secs / 86_400) as i64 + 584_694;
    let rem = secs % 86_400;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, (rem / 3600) as u32, (rem / 60 % 60) as u32, (rem % 60) as u32)
}

fn digits(s: &[u8]) -> Option<u32> {
    if s.is_empty() || !s.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(s.iter().fold(0, |a, &d| a * 10 + u32::from(d - b'0')))
}

/// A UTCTime or GeneralizedTime element's value (by its tag) as a
/// FILETIME. RFC 5280 requires `YYMMDDHHMMSSZ` (years 50..99 are 19xx)
/// and `YYYYMMDDHHMMSSZ`; fractional seconds of a GeneralizedTime are
/// accepted and dropped, and so are a missing seconds field and a
/// `+hhmm`/`-hhmm` offset in UTCTime (BER forms some old certificates
/// have).
pub fn parse(t: u8, v: &[u8]) -> Result<u64, Error> {
    let (year, rest) = match t {
        tag::UTC_TIME => {
            let yy = digits(v.get(..2).ok_or(Error::Invalid)?).ok_or(Error::Invalid)?;
            (i64::from(if yy >= 50 { 1900 + yy } else { 2000 + yy }), &v[2..])
        }
        tag::GENERALIZED_TIME => (i64::from(digits(v.get(..4).ok_or(Error::Invalid)?).ok_or(Error::Invalid)?), &v[4..]),
        found => return Err(Error::UnexpectedTag { expected: tag::UTC_TIME, found }),
    };
    let field = |i: usize| rest.get(i..i + 2).and_then(digits);
    let (month, day, hour, minute) = (
        field(0).ok_or(Error::Invalid)?,
        field(2).ok_or(Error::Invalid)?,
        field(4).ok_or(Error::Invalid)?,
        field(6).ok_or(Error::Invalid)?,
    );
    let (second, mut tail) = match field(8) {
        Some(s) => (s, &rest[10..]),
        None => (0, &rest[8..]),
    };
    if t == tag::GENERALIZED_TIME
        && let [b'.' | b',', frac @ ..] = tail
    {
        let n = frac.iter().take_while(|b| b.is_ascii_digit()).count();
        tail = &frac[n..];
    }
    let offset: i64 = match tail {
        b"Z" => 0,
        [sign @ (b'+' | b'-'), hh1, hh2, mm1, mm2] => {
            let h = digits(&[*hh1, *hh2]).ok_or(Error::Invalid)?;
            let m = digits(&[*mm1, *mm2]).ok_or(Error::Invalid)?;
            let off = i64::from(h * 3600 + m * 60);
            if *sign == b'+' { off } else { -off }
        }
        _ => return Err(Error::Invalid),
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return Err(Error::Invalid);
    }
    let ft = filetime(year, month, day, hour, minute, second.min(59)).ok_or(Error::Invalid)?;
    let adjusted = ft as i64 - offset * TICKS_PER_SECOND as i64;
    u64::try_from(adjusted).map_err(|_| Error::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_filetimes() {
        // Learn: the Unix epoch is FILETIME 116444736000000000.
        assert_eq!(filetime(1970, 1, 1, 0, 0, 0), Some(116_444_736_000_000_000));
        assert_eq!(filetime(1601, 1, 1, 0, 0, 0), Some(0));
        assert_eq!(filetime(1600, 12, 31, 0, 0, 0), None);
        assert_eq!(civil(116_444_736_000_000_000), (1970, 1, 1, 0, 0, 0));
        let ft = filetime(2024, 2, 29, 23, 59, 58).unwrap();
        assert_eq!(civil(ft), (2024, 2, 29, 23, 59, 58));
        let ft = filetime(2100, 3, 1, 1, 2, 3).unwrap();
        assert_eq!(civil(ft), (2100, 3, 1, 1, 2, 3));
    }

    #[test]
    fn parses_both_forms() {
        let utc = parse(tag::UTC_TIME, b"491231235959Z").unwrap();
        assert_eq!(civil(utc), (2049, 12, 31, 23, 59, 59));
        assert_eq!(civil(parse(tag::UTC_TIME, b"500101000000Z").unwrap()).0, 1950);
        let gtime = parse(tag::GENERALIZED_TIME, b"20540101000000Z").unwrap();
        assert_eq!(civil(gtime), (2054, 1, 1, 0, 0, 0));
        let frac = parse(tag::GENERALIZED_TIME, b"20540101000000.123Z").unwrap();
        assert_eq!(frac, gtime);
        let off = parse(tag::UTC_TIME, b"2401010100+0100").unwrap();
        assert_eq!(civil(off), (2024, 1, 1, 0, 0, 0));
        assert!(parse(tag::UTC_TIME, b"241301000000Z").is_err());
        assert!(parse(tag::UTC_TIME, b"240101000000").is_err());
        assert!(parse(tag::INTEGER, b"240101000000Z").is_err());
    }
}
