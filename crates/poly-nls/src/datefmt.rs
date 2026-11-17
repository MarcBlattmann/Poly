//! Date and time formats of the locales and the format pictures of `GetDateFormatW` and
//! `GetTimeFormatW` (Learn: "Day, Month, Year, and Era Format Pictures", "Hour, Minute, and
//! Second Format Pictures", "GetDateFormatEx", "GetTimeFormatEx").
//!
//! The patterns and names of the locales are the ones the spec sheets
//! `specs/windows/kernel32.dll/{GetDateFormatW,GetTimeFormatW}.yaml` record (en-US, en-GB,
//! de-DE, de-CH, fr-FR, it-IT, ja-JP, zh-CN and the invariant locale); a locale without
//! formats of its own uses its language's, else the invariant locale's (Poly's choice).

use alloc::vec::Vec;

/// Month and day names and the era of a language.
#[derive(Debug)]
pub struct Names {
    pub months: [&'static str; 12],
    pub abbrev_months: [&'static str; 12],
    /// Sunday first.
    pub days: [&'static str; 7],
    pub abbrev_days: [&'static str; 7],
    /// The Gregorian era (`g`).
    pub era: &'static str,
}

/// The formats of a locale: pictures for the short date, the long date, the year and month,
/// and the time.
#[derive(Debug)]
pub struct Profile {
    pub name: &'static str,
    pub short_date: &'static str,
    pub long_date: &'static str,
    pub year_month: &'static str,
    pub time: &'static str,
    pub names: &'static Names,
}

static EN: Names = Names {
    months: [
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
    ],
    abbrev_months: ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
    days: ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"],
    abbrev_days: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
    era: "A.D.",
};

static DE: Names = Names {
    months: [
        "Januar",
        "Februar",
        "M\u{E4}rz",
        "April",
        "Mai",
        "Juni",
        "Juli",
        "August",
        "September",
        "Oktober",
        "November",
        "Dezember",
    ],
    abbrev_months: ["Jan", "Feb", "M\u{E4}r", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez"],
    days: ["Sonntag", "Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag"],
    abbrev_days: ["So", "Mo", "Di", "Mi", "Do", "Fr", "Sa"],
    era: "n. Chr.",
};

static FR: Names = Names {
    months: [
        "janvier",
        "f\u{E9}vrier",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "ao\u{FB}t",
        "septembre",
        "octobre",
        "novembre",
        "d\u{E9}cembre",
    ],
    abbrev_months: [
        "janv.",
        "f\u{E9}vr.",
        "mars",
        "avr.",
        "mai",
        "juin",
        "juil.",
        "ao\u{FB}t",
        "sept.",
        "oct.",
        "nov.",
        "d\u{E9}c.",
    ],
    days: ["dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi"],
    abbrev_days: ["dim.", "lun.", "mar.", "mer.", "jeu.", "ven.", "sam."],
    era: "ap. J.-C.",
};

static IT: Names = Names {
    months: [
        "gennaio",
        "febbraio",
        "marzo",
        "aprile",
        "maggio",
        "giugno",
        "luglio",
        "agosto",
        "settembre",
        "ottobre",
        "novembre",
        "dicembre",
    ],
    abbrev_months: ["gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic"],
    days: ["domenica", "luned\u{EC}", "marted\u{EC}", "mercoled\u{EC}", "gioved\u{EC}", "venerd\u{EC}", "sabato"],
    abbrev_days: ["dom", "lun", "mar", "mer", "gio", "ven", "sab"],
    era: "d.C.",
};

static JA: Names = Names {
    months: [
        "1\u{6708}",
        "2\u{6708}",
        "3\u{6708}",
        "4\u{6708}",
        "5\u{6708}",
        "6\u{6708}",
        "7\u{6708}",
        "8\u{6708}",
        "9\u{6708}",
        "10\u{6708}",
        "11\u{6708}",
        "12\u{6708}",
    ],
    abbrev_months: ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12"],
    days: [
        "\u{65E5}\u{66DC}\u{65E5}",
        "\u{6708}\u{66DC}\u{65E5}",
        "\u{706B}\u{66DC}\u{65E5}",
        "\u{6C34}\u{66DC}\u{65E5}",
        "\u{6728}\u{66DC}\u{65E5}",
        "\u{91D1}\u{66DC}\u{65E5}",
        "\u{571F}\u{66DC}\u{65E5}",
    ],
    abbrev_days: ["\u{65E5}", "\u{6708}", "\u{706B}", "\u{6C34}", "\u{6728}", "\u{91D1}", "\u{571F}"],
    era: "\u{897F}\u{66A6}",
};

static ZH: Names = Names {
    months: [
        "\u{4E00}\u{6708}",
        "\u{4E8C}\u{6708}",
        "\u{4E09}\u{6708}",
        "\u{56DB}\u{6708}",
        "\u{4E94}\u{6708}",
        "\u{516D}\u{6708}",
        "\u{4E03}\u{6708}",
        "\u{516B}\u{6708}",
        "\u{4E5D}\u{6708}",
        "\u{5341}\u{6708}",
        "\u{5341}\u{4E00}\u{6708}",
        "\u{5341}\u{4E8C}\u{6708}",
    ],
    abbrev_months: [
        "1\u{6708}",
        "2\u{6708}",
        "3\u{6708}",
        "4\u{6708}",
        "5\u{6708}",
        "6\u{6708}",
        "7\u{6708}",
        "8\u{6708}",
        "9\u{6708}",
        "10\u{6708}",
        "11\u{6708}",
        "12\u{6708}",
    ],
    days: [
        "\u{661F}\u{671F}\u{65E5}",
        "\u{661F}\u{671F}\u{4E00}",
        "\u{661F}\u{671F}\u{4E8C}",
        "\u{661F}\u{671F}\u{4E09}",
        "\u{661F}\u{671F}\u{56DB}",
        "\u{661F}\u{671F}\u{4E94}",
        "\u{661F}\u{671F}\u{516D}",
    ],
    abbrev_days: [
        "\u{5468}\u{65E5}",
        "\u{5468}\u{4E00}",
        "\u{5468}\u{4E8C}",
        "\u{5468}\u{4E09}",
        "\u{5468}\u{56DB}",
        "\u{5468}\u{4E94}",
        "\u{5468}\u{516D}",
    ],
    era: "\u{516C}\u{5143}",
};

/// The invariant locale's formats.
pub static INVARIANT: Profile = Profile {
    name: "",
    short_date: "MM/dd/yyyy",
    long_date: "dddd, dd MMMM yyyy",
    year_month: "yyyy MMMM",
    time: "HH:mm:ss",
    names: &EN,
};

static PROFILES: &[Profile] = &[
    Profile {
        name: "en-US",
        short_date: "M/d/yyyy",
        long_date: "dddd, MMMM d, yyyy",
        year_month: "MMMM yyyy",
        time: "h:mm:ss tt",
        names: &EN,
    },
    Profile {
        name: "en-GB",
        short_date: "dd/MM/yyyy",
        long_date: "dd MMMM yyyy",
        year_month: "MMMM yyyy",
        time: "HH:mm:ss",
        names: &EN,
    },
    Profile {
        name: "de-DE",
        short_date: "dd.MM.yyyy",
        long_date: "dddd, d. MMMM yyyy",
        year_month: "MMMM yyyy",
        time: "HH:mm:ss",
        names: &DE,
    },
    Profile {
        name: "fr-FR",
        short_date: "dd/MM/yyyy",
        long_date: "dddd d MMMM yyyy",
        year_month: "MMMM yyyy",
        time: "HH:mm:ss",
        names: &FR,
    },
    Profile {
        name: "it-IT",
        short_date: "dd/MM/yyyy",
        long_date: "dddd d MMMM yyyy",
        year_month: "MMMM yyyy",
        time: "HH:mm:ss",
        names: &IT,
    },
    Profile {
        name: "ja-JP",
        short_date: "yyyy/MM/dd",
        long_date: "yyyy'\u{5E74}'M'\u{6708}'d'\u{65E5}'",
        year_month: "yyyy'\u{5E74}'M'\u{6708}'",
        time: "H:mm:ss",
        names: &JA,
    },
    Profile {
        name: "zh-CN",
        short_date: "yyyy/M/d",
        long_date: "yyyy'\u{5E74}'M'\u{6708}'d'\u{65E5}'",
        year_month: "yyyy'\u{5E74}'M'\u{6708}'",
        time: "H:mm:ss",
        names: &ZH,
    },
];

/// The formats of the locale named `name` (a specific locale's name, "" for the invariant
/// locale): its own, else the first of its language's (`de-CH` uses `de-DE`'s; English
/// outside the United States uses `en-GB`'s), else the invariant locale's.
pub fn profile(name: &str) -> &'static Profile {
    if let Some(p) = PROFILES.iter().find(|p| p.name == name) {
        return p;
    }
    let language = name.split(['-', '_']).next().unwrap_or("");
    let language = if language == "en" { "en-GB" } else { language };
    PROFILES
        .iter()
        .find(|p| p.name == language || p.name.split('-').next() == Some(language))
        .unwrap_or(&INVARIANT)
}

/// The days of a month (Gregorian).
pub fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Whether a date can be formatted: years 1601 to 30827 (the range of a `FILETIME`), a month
/// and a day that exist.
pub fn is_valid_date(year: u32, month: u32, day: u32) -> bool {
    (1601..=30827).contains(&year) && (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month)
}

/// The day of the week of a Gregorian date, Sunday 0.
pub fn weekday(year: u32, month: u32, day: u32) -> u32 {
    const T: [u32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    (y + y / 4 - y / 100 + y / 400 + T[(month as usize - 1) % 12] + day) % 7
}

/// One piece of a picture: literal text, or an element (its letter and how many times it is
/// repeated).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    Text(Vec<u16>),
    Element(u8, usize),
}

/// Split a picture into literal text and elements made of the letters `letters`: text between
/// single quotes is literal (two quotes inside it are one quote; a quote that is not closed
/// runs to the end), a run of one of the letters is an element, anything else is literal.
fn parse(picture: &[u16], letters: &[u8]) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let push_text = |out: &mut Vec<Piece>, c: u16| match out.last_mut() {
        Some(Piece::Text(t)) => t.push(c),
        _ => out.push(Piece::Text(alloc::vec![c])),
    };
    let quote = u16::from(b'\'');
    let mut i = 0;
    while i < picture.len() {
        let c = picture[i];
        if c == quote {
            i += 1;
            let mut any = false;
            while i < picture.len() {
                if picture[i] == quote {
                    if picture.get(i + 1) == Some(&quote) {
                        push_text(&mut out, quote);
                        any = true;
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                push_text(&mut out, picture[i]);
                any = true;
                i += 1;
            }
            if !any && !matches!(out.last(), Some(Piece::Text(_))) {
                out.push(Piece::Text(Vec::new()));
            }
            continue;
        }
        if let Some(&l) = letters.iter().find(|&&l| u16::from(l) == c) {
            let start = i;
            while i < picture.len() && picture[i] == c {
                i += 1;
            }
            out.push(Piece::Element(l, i - start));
            continue;
        }
        push_text(&mut out, c);
        i += 1;
    }
    out
}

fn push_str(out: &mut Vec<u16>, s: &str) {
    out.extend(s.encode_utf16());
}

fn push_number(out: &mut Vec<u16>, n: u32, width: usize) {
    let mut digits = [0u16; 10];
    let mut len = 0;
    let mut v = n;
    loop {
        digits[len] = u16::from(b'0') + (v % 10) as u16;
        len += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    for _ in len..width {
        out.push(u16::from(b'0'));
    }
    out.extend(digits[..len].iter().rev());
}

/// `TIME_NOMINUTESORSECONDS`, `TIME_NOSECONDS`, `TIME_NOTIMEMARKER`, `TIME_FORCE24HOURFORMAT`
/// (`winnls.h`).
pub const TIME_NOMINUTESORSECONDS: u32 = 0x1;
pub const TIME_NOSECONDS: u32 = 0x2;
pub const TIME_NOTIMEMARKER: u32 = 0x4;
pub const TIME_FORCE24HOURFORMAT: u32 = 0x8;

/// Remove the elements `drop` names together with the literal text that leads up to each (the
/// text after it when it comes first).
fn drop_elements(pieces: &mut Vec<Piece>, drop: impl Fn(u8) -> bool) {
    let mut i = 0;
    while i < pieces.len() {
        let Piece::Element(l, _) = pieces[i] else {
            i += 1;
            continue;
        };
        if !drop(l) {
            i += 1;
            continue;
        }
        let earlier = pieces[..i].iter().any(|p| matches!(p, Piece::Element(..)));
        if earlier {
            let mut start = i;
            while start > 0 && matches!(pieces[start - 1], Piece::Text(_)) {
                start -= 1;
            }
            pieces.drain(start..=i);
            i = start;
        } else {
            let mut end = i + 1;
            while end < pieces.len() && matches!(pieces[end], Piece::Text(_)) {
                end += 1;
            }
            pieces.drain(i..end);
        }
    }
}

/// Format a time with a time picture (`h hh H HH m mm s ss t tt`; longer runs act as two
/// letters, three or more `t` as the marker's first letter in lower case) and the `TIME_*`
/// flags, which apply to any picture. The marker is "AM" or "PM" in every locale.
pub fn format_time(picture: &[u16], hour: u32, minute: u32, second: u32, flags: u32) -> Vec<u16> {
    let mut pieces = parse(picture, b"hHmst");
    drop_elements(&mut pieces, |l| match l {
        b'm' => flags & TIME_NOMINUTESORSECONDS != 0,
        b's' => flags & (TIME_NOMINUTESORSECONDS | TIME_NOSECONDS) != 0,
        b't' => flags & TIME_NOTIMEMARKER != 0,
        _ => false,
    });
    let marker = if hour < 12 { "AM" } else { "PM" };
    let mut out = Vec::new();
    for p in &pieces {
        match *p {
            Piece::Text(ref t) => out.extend_from_slice(t),
            Piece::Element(l, n) => {
                let width = if n >= 2 { 2 } else { 0 };
                match l {
                    b'h' if flags & TIME_FORCE24HOURFORMAT == 0 => {
                        push_number(&mut out, if hour % 12 == 0 { 12 } else { hour % 12 }, width)
                    }
                    b'h' | b'H' => push_number(&mut out, hour, width),
                    b'm' => push_number(&mut out, minute, width),
                    b's' => push_number(&mut out, second, width),
                    _ => match n {
                        1 => push_str(&mut out, &marker[..1]),
                        2 => push_str(&mut out, marker),
                        _ => out.push(u16::from(marker.as_bytes()[0].to_ascii_lowercase())),
                    },
                }
            }
        }
    }
    out
}

/// Format a date with a date picture (`d dd ddd dddd`, `M MM MMM MMMM`, `y yy yyyy`, `g gg`;
/// longer runs act as the longest form) in the names of `names`. `mark` is put before each
/// element (U+200E or U+200F for the reading-order flags). The weekday comes from the date.
pub fn format_date(picture: &[u16], names: &Names, year: u32, month: u32, day: u32, mark: Option<u16>) -> Vec<u16> {
    let pieces = parse(picture, b"dMyg");
    let mut out = Vec::new();
    let m = (month as usize).clamp(1, 12) - 1;
    let wd = weekday(year, month, day) as usize;
    for p in &pieces {
        match *p {
            Piece::Text(ref t) => out.extend_from_slice(t),
            Piece::Element(l, n) => {
                if let Some(mark) = mark {
                    out.push(mark);
                }
                match (l, n) {
                    (b'd', 1) => push_number(&mut out, day, 0),
                    (b'd', 2) => push_number(&mut out, day, 2),
                    (b'd', 3) => push_str(&mut out, names.abbrev_days[wd]),
                    (b'd', _) => push_str(&mut out, names.days[wd]),
                    (b'M', 1) => push_number(&mut out, month, 0),
                    (b'M', 2) => push_number(&mut out, month, 2),
                    (b'M', 3) => push_str(&mut out, names.abbrev_months[m]),
                    (b'M', _) => push_str(&mut out, names.months[m]),
                    (b'y', 1 | 2) => push_number(&mut out, year % 100, 2),
                    (b'y', _) => push_number(&mut out, year, 4),
                    _ => push_str(&mut out, names.era),
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn time(pic: &str, h: u32, m: u32, s: u32, flags: u32) -> String {
        String::from_utf16(&format_time(&w(pic), h, m, s, flags)).unwrap()
    }

    fn date(pic: &str, p: &Profile, y: u32, m: u32, d: u32) -> String {
        String::from_utf16(&format_date(&w(pic), p.names, y, m, d, None)).unwrap()
    }

    #[test]
    fn time_pictures() {
        assert_eq!(time("h:mm:ss tt", 7, 8, 9, 0), "7:08:09 AM");
        assert_eq!(time("h:mm:ss tt", 0, 0, 0, 0), "12:00:00 AM");
        assert_eq!(time("hh 'o''clock' tt", 15, 30, 45, 0), "03 o'clock PM");
        assert_eq!(time("'unterminated", 15, 30, 45, 0), "unterminated");
        assert_eq!(time("ttt", 15, 0, 0, 0), "p");
        assert_eq!(time("t", 15, 0, 0, 0), "P");
        assert_eq!(time("hhh:mmm:sss", 15, 30, 45, 0), "03:30:45");
        assert_eq!(time("dd.MM.yyyy HH:mm", 15, 30, 0, 0), "dd.MM.yyyy 15:30");
        assert_eq!(time("", 15, 30, 0, 0), "");
    }

    #[test]
    fn time_flags() {
        assert_eq!(time("h:mm:ss tt", 7, 8, 9, TIME_NOMINUTESORSECONDS), "7 AM");
        assert_eq!(time("h:mm:ss tt", 7, 8, 9, TIME_NOSECONDS), "7:08 AM");
        assert_eq!(time("h:mm:ss tt", 7, 8, 9, TIME_NOTIMEMARKER), "7:08:09");
        assert_eq!(time("h:mm:ss tt", 15, 30, 45, TIME_FORCE24HOURFORMAT), "15:30:45 PM");
        assert_eq!(time("h:mm:ss tt", 0, 0, 0, TIME_FORCE24HOURFORMAT), "0:00:00 AM");
        assert_eq!(time("h:mm:ss tt", 7, 8, 9, TIME_NOSECONDS | TIME_NOTIMEMARKER), "7:08");
        assert_eq!(time("HH:mm", 7, 8, 9, TIME_NOMINUTESORSECONDS), "07");
        assert_eq!(time("HH:mm", 7, 8, 9, TIME_NOTIMEMARKER), "07:08");
    }

    #[test]
    fn date_layouts() {
        let us = profile("en-US");
        assert_eq!(date(us.short_date, us, 2024, 3, 5), "3/5/2024");
        assert_eq!(date(us.long_date, us, 2024, 3, 5), "Tuesday, March 5, 2024");
        let de = profile("de-CH");
        assert_eq!(date(de.long_date, de, 2024, 3, 5), "Dienstag, 5. M\u{E4}rz 2024");
        assert_eq!(date(INVARIANT.year_month, &INVARIANT, 2024, 3, 5), "2024 March");
        let ja = profile("ja-JP");
        assert_eq!(date(ja.long_date, ja, 2024, 3, 5), "2024\u{5E74}3\u{6708}5\u{65E5}");
        assert_eq!(date("gg 'x' d h:mm tt", us, 2024, 3, 5), "A.D. x 5 h:mm tt");
        assert_eq!(date("ddd dddd MMM yy", profile("fr-FR"), 2024, 7, 14), "dim. dimanche juil. 24");
        let marked = format_date(&w("M/d/yyyy"), us.names, 2024, 3, 5, Some(0x200E));
        assert_eq!(String::from_utf16(&marked).unwrap(), "\u{200E}3/\u{200E}5/\u{200E}2024");
        assert_eq!(profile("de").name, "de-DE");
        assert_eq!(profile("en-AU").name, "en-GB");
        assert_eq!(profile("ru-RU").name, "");
    }

    #[test]
    fn dates() {
        assert!(is_valid_date(2000, 2, 29) && !is_valid_date(1900, 2, 29) && !is_valid_date(2023, 2, 29));
        assert!(!is_valid_date(1600, 1, 1) && is_valid_date(1601, 1, 1) && !is_valid_date(30828, 1, 1));
        assert!(!is_valid_date(2024, 4, 31) && !is_valid_date(2024, 0, 1) && !is_valid_date(2024, 13, 1));
        assert_eq!(weekday(2024, 7, 14), 0);
        assert_eq!(weekday(1601, 1, 1), 1);
        assert_eq!(weekday(2025, 11, 5), 3);
    }
}
