//! Wall-clock time from the PC's real-time clock: the date and time
//! registers of an MC146818-compatible CMOS RTC (what QEMU and PC firmware
//! provide) decoded into seconds since the Unix epoch.
//!
//! Register layout and encodings from the MC146818 data sheet: seconds,
//! minutes, hours, day of month, month and year at indexes 0x00, 0x02,
//! 0x04, 0x07, 0x08 and 0x09; status register A bit 7 "update in
//! progress"; status register B bit 1 "24-hour mode" and bit 2 "binary
//! (not BCD) data"; in 12-hour mode bit 7 of the hours means PM. The year
//! register holds only two digits: a century register (its index comes
//! from the ACPI FADT `CENTURY` field, when the firmware has one) gives the
//! rest, otherwise years 70-99 are taken as 1970-1999 and 00-69 as
//! 2000-2069.
//!
//! The kernel reads the registers (`arch/x86_64/rtc.rs`); this crate only
//! decodes them, so it is tested on the host.

#![no_std]

/// Register indexes (MC146818).
pub mod reg {
    pub const SECONDS: u8 = 0x00;
    pub const MINUTES: u8 = 0x02;
    pub const HOURS: u8 = 0x04;
    pub const DAY: u8 = 0x07;
    pub const MONTH: u8 = 0x08;
    pub const YEAR: u8 = 0x09;
    pub const STATUS_A: u8 = 0x0a;
    pub const STATUS_B: u8 = 0x0b;
}

/// Status register A: an update of the time registers is in progress.
pub const STATUS_A_UPDATE_IN_PROGRESS: u8 = 1 << 7;
/// Status register B: hours count 0-23 (else 1-12 with a PM bit).
pub const STATUS_B_24_HOUR: u8 = 1 << 1;
/// Status register B: values are binary (else BCD).
pub const STATUS_B_BINARY: u8 = 1 << 2;

/// The raw time registers, as read from the chip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Registers {
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    pub day: u8,
    pub month: u8,
    pub year: u8,
    /// The century register, if the platform names one.
    pub century: Option<u8>,
    pub status_b: u8,
}

/// A calendar date and time (UTC, proleptic Gregorian calendar).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub year: u32,
    /// 1-12.
    pub month: u32,
    /// 1-31.
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// A register value as a number: BCD unless the chip is in binary mode.
const fn value(raw: u8, binary: bool) -> u32 {
    if binary { raw as u32 } else { (raw >> 4) as u32 * 10 + (raw & 0x0f) as u32 }
}

impl Registers {
    /// The date and time the registers hold, or `None` if they are out of
    /// range (a chip with a dead battery, garbage).
    pub fn decode(&self) -> Option<DateTime> {
        let binary = self.status_b & STATUS_B_BINARY != 0;
        let pm = self.hours & 0x80 != 0;
        let mut hour = value(self.hours & 0x7f, binary);
        if self.status_b & STATUS_B_24_HOUR == 0 {
            // 12-hour mode: 12 AM is midnight, 12 PM noon.
            if !(1..=12).contains(&hour) {
                return None;
            }
            hour = (hour % 12) + if pm { 12 } else { 0 };
        }
        let short_year = value(self.year, binary);
        let year = match self.century {
            Some(c) => value(c, binary) * 100 + short_year,
            None if short_year >= 70 => 1900 + short_year,
            None => 2000 + short_year,
        };
        let t = DateTime {
            year,
            month: value(self.month, binary),
            day: value(self.day, binary),
            hour,
            minute: value(self.minutes, binary),
            second: value(self.seconds, binary),
        };
        t.is_valid().then_some(t)
    }
}

const fn is_leap(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

const fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl DateTime {
    /// Whether every field is in range (seconds up to 59: the RTC has no
    /// leap seconds).
    pub const fn is_valid(&self) -> bool {
        self.year >= 1970
            && self.month >= 1
            && self.month <= 12
            && self.day >= 1
            && self.day <= days_in_month(self.year, self.month)
            && self.hour < 24
            && self.minute < 60
            && self.second < 60
    }

    /// Seconds since 1970-01-01 00:00:00 UTC (POSIX time: every day has
    /// 86400 seconds).
    pub fn unix_seconds(&self) -> u64 {
        let mut days: u64 = 0;
        for y in 1970..self.year {
            days += if is_leap(y) { 366 } else { 365 };
        }
        for m in 1..self.month {
            days += u64::from(days_in_month(self.year, m));
        }
        days += u64::from(self.day - 1);
        ((days * 24 + u64::from(self.hour)) * 60 + u64::from(self.minute)) * 60 + u64::from(self.second)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regs(sec: u8, min: u8, hours: u8, day: u8, month: u8, year: u8, status_b: u8) -> Registers {
        Registers { seconds: sec, minutes: min, hours, day, month, year, century: None, status_b }
    }

    #[test]
    fn bcd_24_hour() {
        // 2024-02-29 23:59:58 in BCD, 24-hour mode.
        let r = regs(0x58, 0x59, 0x23, 0x29, 0x02, 0x24, STATUS_B_24_HOUR);
        let t = r.decode().unwrap();
        assert_eq!(t, DateTime { year: 2024, month: 2, day: 29, hour: 23, minute: 59, second: 58 });
        assert_eq!(t.unix_seconds(), 1_709_251_198);
    }

    #[test]
    fn binary_12_hour() {
        let b = STATUS_B_BINARY;
        // 12 AM is midnight, 12 PM noon, 1 PM 13:00.
        assert_eq!(regs(0, 0, 12, 1, 1, 70, b).decode().unwrap().hour, 0);
        assert_eq!(regs(0, 0, 0x80 | 12, 1, 1, 70, b).decode().unwrap().hour, 12);
        assert_eq!(regs(0, 0, 0x80 | 1, 1, 1, 70, b).decode().unwrap().hour, 13);
        assert_eq!(regs(0, 0, 0, 1, 1, 70, b).decode(), None, "hour 0 in 12-hour mode");
        assert_eq!(regs(0, 0, 12, 1, 1, 70, b).decode().unwrap().unix_seconds(), 0);
    }

    #[test]
    fn centuries() {
        let b = STATUS_B_BINARY | STATUS_B_24_HOUR;
        assert_eq!(regs(0, 0, 0, 1, 1, 99, b).decode().unwrap().year, 1999);
        assert_eq!(regs(0, 0, 0, 1, 1, 26, b).decode().unwrap().year, 2026);
        let with_century = Registers { century: Some(0x21), ..regs(0, 0, 0, 1, 1, 0x05, STATUS_B_24_HOUR) };
        assert_eq!(with_century.decode().unwrap().year, 2105);
    }

    #[test]
    fn garbage_is_refused() {
        let b = STATUS_B_BINARY | STATUS_B_24_HOUR;
        assert_eq!(regs(0, 0, 0, 30, 2, 24, b).decode(), None, "30 February");
        assert_eq!(regs(0, 0, 0, 29, 2, 23, b).decode(), None, "29 February 2023");
        assert_eq!(regs(60, 0, 0, 1, 1, 24, b).decode(), None);
        assert_eq!(regs(0, 0, 24, 1, 1, 24, b).decode(), None);
        assert_eq!(regs(0, 0, 0, 1, 13, 24, b).decode(), None);
        assert_eq!(regs(0, 0, 0, 0xff, 0xff, 0xff, 0).decode(), None);
    }

    #[test]
    fn known_dates() {
        let at = |year, month, day| DateTime { year, month, day, hour: 0, minute: 0, second: 0 }.unix_seconds();
        assert_eq!(at(1970, 1, 1), 0);
        assert_eq!(at(2000, 3, 1), 951_868_800);
        assert_eq!(at(2020, 1, 1), 1_577_836_800);
        assert_eq!(at(2100, 3, 1), 4_107_542_400, "2100 is not a leap year");
    }
}
