//! The ridge's clock: a time on the Unix clock as Alpine Shell's clock
//! shows it, `21:41 Sep 12` (24-hour `HH:mm`, then the month's English
//! abbreviation and the day).

use alloc::format;
use alloc::string::String;

/// A calendar date and a time of day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    pub year: i64,
    /// 1 to 12.
    pub month: u32,
    /// 1 to 31.
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The proleptic Gregorian date of `secs` seconds after 1970-01-01 00:00
/// UTC, shifted by `offset_minutes` (the local time zone; 0 for UTC).
pub fn civil(secs: i64, offset_minutes: i32) -> Civil {
    let t = secs + i64::from(offset_minutes) * 60;
    let days = t.div_euclid(86_400);
    let rem = t.rem_euclid(86_400) as u32;
    // Days to a date by 400-year eras of 146097 days, counting years from
    // March so that the leap day is the era's last.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    Civil { year, month, day, hour: rem / 3600, minute: rem / 60 % 60, second: rem % 60 }
}

/// `HH:mm MMM d`, as the ridge shows it.
pub fn ridge_text(c: &Civil) -> String {
    format!("{:02}:{:02} {} {}", c.hour, c.minute, MONTHS[(c.month as usize - 1) % 12], c.day)
}

/// Seconds until the minute shown changes (when to draw the clock again).
pub fn until_next_minute(secs: i64) -> i64 {
    60 - secs.rem_euclid(60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        let c = civil(0, 0);
        assert_eq!((c.year, c.month, c.day, c.hour, c.minute), (1970, 1, 1, 0, 0));
        // 2000-02-29 12:34:56 UTC = 951827696.
        let c = civil(951_827_696, 0);
        assert_eq!((c.year, c.month, c.day, c.hour, c.minute, c.second), (2000, 2, 29, 12, 34, 56));
        // 2026-10-31 23:59:59 UTC = 1793491199, and one second later.
        assert_eq!(ridge_text(&civil(1_793_491_199, 0)), "23:59 Oct 31");
        assert_eq!(ridge_text(&civil(1_793_491_200, 0)), "00:00 Nov 1");
        // A time zone east of UTC.
        assert_eq!(ridge_text(&civil(1_793_491_200, 60)), "01:00 Nov 1");
        // Before 1970.
        let c = civil(-1, 0);
        assert_eq!((c.year, c.month, c.day, c.hour, c.minute, c.second), (1969, 12, 31, 23, 59, 59));
        assert_eq!(until_next_minute(1_793_491_199), 1);
        assert_eq!(until_next_minute(1_793_491_200), 60);
    }
}
