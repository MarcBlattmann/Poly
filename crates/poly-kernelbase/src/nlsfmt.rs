//! The rules of the locale-name and date/time functions of `kernelbase` (spec sheets
//! `specs/windows/kernel32.dll/{GetTimeFormatW,GetDateFormatW,EnumSystemLocalesW,
//! GetUserPreferredUILanguages}.yaml` and `specs/windows/kernelbase.dll/{GetTimeFormatEx,
//! GetDateFormatEx,EnumSystemLocalesEx,LCIDToLocaleName,LocaleNameToLCID}.yaml`): which locale
//! an argument names, which flags are valid, what text comes out, and the buffer protocol. The
//! locale table and the pictures are `poly_nls::lcid`'s and `poly_nls::datefmt`'s. Constants are
//! from `winnls.h` and `winerror.h`.
//!
//! Poly's user, system and UI locale is en-US ([`crate::locale::LCID_EN_US`]).

use crate::error::{ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_FLAGS, ERROR_INVALID_PARAMETER};
use crate::locale::{LCID_EN_US, LOCALE_NOUSEROVERRIDE, LOCALE_USE_CP_ACP};
use alloc::vec::Vec;
use poly_nls::datefmt::{self, Profile};
use poly_nls::lcid::{self, Entry, Kind, LOCALE_ALLOW_NEUTRAL_NAMES, LOCALE_CUSTOM_UNSPECIFIED, Named};

/// `LOCALE_NAME_SYSTEM_DEFAULT` (`winnls.h`).
pub const LOCALE_NAME_SYSTEM_DEFAULT: &str = "!x-sys-default-locale";

/// `DATE_*` (`winnls.h`).
pub const DATE_SHORTDATE: u32 = 0x1;
pub const DATE_LONGDATE: u32 = 0x2;
pub const DATE_USE_ALT_CALENDAR: u32 = 0x4;
pub const DATE_YEARMONTH: u32 = 0x8;
pub const DATE_LTRREADING: u32 = 0x10;
pub const DATE_RTLREADING: u32 = 0x20;
pub const DATE_AUTOLAYOUT: u32 = 0x40;

/// `LCID_INSTALLED`, `LCID_SUPPORTED`, `LCID_ALTERNATE_SORTS` (`winnls.h`).
pub const LCID_INSTALLED: u32 = 0x1;
pub const LCID_SUPPORTED: u32 = 0x2;
pub const LCID_ALTERNATE_SORTS: u32 = 0x4;

/// `LOCALE_WINDOWS`, `LOCALE_SUPPLEMENTAL`, `LOCALE_ALTERNATE_SORTS`, `LOCALE_NEUTRALDATA`,
/// `LOCALE_SPECIFICDATA` (`winnls.h`).
pub const LOCALE_WINDOWS: u32 = 0x1;
pub const LOCALE_SUPPLEMENTAL: u32 = 0x2;
pub const LOCALE_ALTERNATE_SORTS: u32 = 0x4;
pub const LOCALE_NEUTRALDATA: u32 = 0x10;
pub const LOCALE_SPECIFICDATA: u32 = 0x20;

/// The user's locale: what `LOCALE_USER_DEFAULT` and the other default identifiers mean.
fn user_default() -> &'static Entry {
    lcid::by_lcid(LCID_EN_US).expect("en-US is in the table")
}

/// The system's locale (`LOCALE_SYSTEM_DEFAULT`, `!x-sys-default-locale`): en-US as well.
fn system_default() -> &'static Entry {
    user_default()
}

/// The locale an identifier names: `LOCALE_NEUTRAL` 0, `LOCALE_USER_DEFAULT` 0x400,
/// `LOCALE_CUSTOM_DEFAULT` 0xC00, `LOCALE_CUSTOM_UNSPECIFIED` 0x1000 and `LOCALE_CUSTOM_UI_DEFAULT`
/// 0x1400 (the UI language, en-US too) and 0x2000 are the user's, `LOCALE_SYSTEM_DEFAULT` 0x800
/// the system's, any other a locale of the table; an identifier the table lacks names none.
pub fn locale_of_lcid(lcid: u32) -> Option<&'static Entry> {
    match lcid {
        0 | 0x400 | 0xC00 | 0x1000 | 0x1400 | 0x2000 => Some(user_default()),
        0x800 => Some(system_default()),
        _ => lcid::by_lcid(lcid),
    }
}

/// `true` when `name` is `!x-sys-default-locale` in any case.
fn is_system_default_name(name: &[u16]) -> bool {
    name.len() == LOCALE_NAME_SYSTEM_DEFAULT.len()
        && LOCALE_NAME_SYSTEM_DEFAULT
            .bytes()
            .zip(name)
            .all(|(a, &b)| u8::try_from(b).is_ok_and(|b| b.to_ascii_lowercase() == a))
}

/// The formats a locale name selects (`None` for the user default): the user's, the system's
/// for `!x-sys-default-locale`, the locale's own (a neutral one's default's), a custom name's
/// locale before its sort suffix, else the invariant locale's; a malformed name is
/// `ERROR_INVALID_PARAMETER`.
pub fn formats_of_name(name: Option<&[u16]>) -> Result<&'static Profile, u32> {
    let entry = match name {
        None => user_default(),
        Some(n) if is_system_default_name(n) => system_default(),
        Some(n) => match lcid::lookup(n) {
            Some(Named::Known(e)) => e,
            Some(Named::Custom { base: Some(e) }) => e,
            Some(Named::Custom { base: None }) => return Ok(&datefmt::INVARIANT),
            None => return Err(ERROR_INVALID_PARAMETER),
        },
    };
    Ok(datefmt::profile(lcid::format_locale(entry).name))
}

/// The formats an identifier selects; an unknown one is `ERROR_INVALID_PARAMETER`.
pub fn formats_of_lcid(lcid: u32) -> Result<&'static Profile, u32> {
    let e = locale_of_lcid(lcid).ok_or(ERROR_INVALID_PARAMETER)?;
    Ok(datefmt::profile(lcid::format_locale(e).name))
}

/// The parts of a `SYSTEMTIME` (`minwinbase.h`): year, month, day of week, day, hour, minute,
/// second, milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemTime(pub [u16; 8]);

impl SystemTime {
    fn year(&self) -> u32 {
        u32::from(self.0[0])
    }
    fn month(&self) -> u32 {
        u32::from(self.0[1])
    }
    fn day(&self) -> u32 {
        u32::from(self.0[3])
    }
    fn hour(&self) -> u32 {
        u32::from(self.0[4])
    }
    fn minute(&self) -> u32 {
        u32::from(self.0[5])
    }
    fn second(&self) -> u32 {
        u32::from(self.0[6])
    }
    fn milliseconds(&self) -> u32 {
        u32::from(self.0[7])
    }
}

/// The flags `GetTimeFormatW`/`Ex` accept: the four `TIME_*` flags, `LOCALE_NOUSEROVERRIDE`
/// and `LOCALE_USE_CP_ACP`.
const TIME_FLAGS: u32 = datefmt::TIME_NOMINUTESORSECONDS
    | datefmt::TIME_NOSECONDS
    | datefmt::TIME_NOTIMEMARKER
    | datefmt::TIME_FORCE24HOURFORMAT
    | LOCALE_NOUSEROVERRIDE
    | LOCALE_USE_CP_ACP;

/// The text of `GetTimeFormatW`/`Ex`: flags other than [`TIME_FLAGS`] are `ERROR_INVALID_FLAGS`;
/// an hour of 24 or more, a minute of 60 or more or milliseconds of 1000 or more are
/// `ERROR_INVALID_PARAMETER` (the seconds and the date are not looked at); a picture replaces
/// the locale's pattern and the flags still apply to it.
pub fn time_text(p: &Profile, flags: u32, time: &SystemTime, picture: Option<&[u16]>) -> Result<Vec<u16>, u32> {
    if flags & !TIME_FLAGS != 0 {
        return Err(ERROR_INVALID_FLAGS);
    }
    if time.hour() >= 24 || time.minute() >= 60 || time.milliseconds() >= 1000 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let own: Vec<u16>;
    let picture = match picture {
        Some(pic) => pic,
        None => {
            own = p.time.encode_utf16().collect();
            &own
        }
    };
    Ok(datefmt::format_time(picture, time.hour(), time.minute(), time.second(), flags))
}

/// The flags `GetDateFormatW`/`Ex` accept (`DATE_MONTHDAY` is refused).
const DATE_FLAGS: u32 = DATE_SHORTDATE
    | DATE_LONGDATE
    | DATE_USE_ALT_CALENDAR
    | DATE_YEARMONTH
    | DATE_LTRREADING
    | DATE_RTLREADING
    | DATE_AUTOLAYOUT
    | LOCALE_NOUSEROVERRIDE
    | LOCALE_USE_CP_ACP;

/// The text of `GetDateFormatW`/`Ex`: flags outside [`DATE_FLAGS`], two of the layouts (short,
/// long, year and month), both reading orders, or a layout together with a picture are
/// `ERROR_INVALID_FLAGS`; a date that does not exist (or lies outside 1601 to 30827) is
/// `ERROR_INVALID_PARAMETER`. A reading-order flag (or `DATE_AUTOLAYOUT`, left to right for
/// every locale Poly formats) puts U+200E or U+200F before each element.
pub fn date_text(p: &Profile, flags: u32, date: &SystemTime, picture: Option<&[u16]>) -> Result<Vec<u16>, u32> {
    let layouts = flags & (DATE_SHORTDATE | DATE_LONGDATE | DATE_YEARMONTH);
    if flags & !DATE_FLAGS != 0
        || layouts.count_ones() > 1
        || flags & (DATE_LTRREADING | DATE_RTLREADING) == DATE_LTRREADING | DATE_RTLREADING
        || (picture.is_some() && layouts != 0)
    {
        return Err(ERROR_INVALID_FLAGS);
    }
    if !datefmt::is_valid_date(date.year(), date.month(), date.day()) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let mark = if flags & DATE_RTLREADING != 0 {
        Some(0x200F)
    } else if flags & (DATE_LTRREADING | DATE_AUTOLAYOUT) != 0 {
        Some(0x200E)
    } else {
        None
    };
    let own: Vec<u16>;
    let picture = match picture {
        Some(pic) => pic,
        None => {
            let pattern = match layouts {
                DATE_LONGDATE => p.long_date,
                DATE_YEARMONTH => p.year_month,
                _ => p.short_date,
            };
            own = pattern.encode_utf16().collect();
            &own
        }
    };
    Ok(datefmt::format_date(picture, p.names, date.year(), date.month(), date.day(), mark))
}

/// How a function that formats into a caller's buffer of `cch` units answers for `text`:
/// `Ok(None)` when only the size is asked for (`cch` 0), `Ok(Some(units))` with the text and
/// its NUL to copy, or the error: a negative size, or no buffer with a size, is
/// `ERROR_INVALID_PARAMETER`; a buffer too small is `ERROR_INSUFFICIENT_BUFFER` and is not
/// written. The result is the length with the NUL.
pub fn sized(text: Vec<u16>, cch: i32, has_buffer: bool) -> Result<(i32, Option<Vec<u16>>), u32> {
    let need = text.len() as i32 + 1;
    if cch == 0 {
        return Ok((need, None));
    }
    if cch < 0 || !has_buffer {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if cch < need {
        return Err(ERROR_INSUFFICIENT_BUFFER);
    }
    let mut out = text;
    out.push(0);
    Ok((need, Some(out)))
}

/// The buffer checks every formatting function makes first: a negative size, or no buffer
/// with a size, is `ERROR_INVALID_PARAMETER`.
pub fn check_buffer(cch: i32, has_buffer: bool) -> Result<(), u32> {
    if cch < 0 || (cch > 0 && !has_buffer) { Err(ERROR_INVALID_PARAMETER) } else { Ok(()) }
}

/// What `LCIDToLocaleName` answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameAnswer {
    /// The size the name needs with its NUL (`cch` 0).
    Size(i32),
    /// The name and its NUL; the result is its length.
    Name(Vec<u16>),
    /// `ERROR_INSUFFICIENT_BUFFER`: as much of the name as fits, ended by a NUL, is written.
    Truncated(Vec<u16>),
}

/// `LCIDToLocaleName(lcid, buffer of cch, flags)`: only 0 and `LOCALE_ALLOW_NEUTRAL_NAMES` are
/// valid flags; a negative size, no buffer with a size, an unknown identifier and any other
/// flag are `ERROR_INVALID_PARAMETER`.
pub fn lcid_to_name(lcid: u32, cch: i32, has_buffer: bool, flags: u32) -> Result<NameAnswer, u32> {
    if flags & !LOCALE_ALLOW_NEUTRAL_NAMES != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    check_buffer(cch, has_buffer)?;
    let e = locale_of_lcid(lcid).ok_or(ERROR_INVALID_PARAMETER)?;
    let mut name: Vec<u16> = lcid::name_of(e, flags & LOCALE_ALLOW_NEUTRAL_NAMES != 0).encode_utf16().collect();
    let need = name.len() as i32 + 1;
    if cch == 0 {
        return Ok(NameAnswer::Size(need));
    }
    if cch < need {
        name.truncate(cch as usize - 1);
        name.push(0);
        return Ok(NameAnswer::Truncated(name));
    }
    name.push(0);
    Ok(NameAnswer::Name(name))
}

/// `LocaleNameToLCID(name, flags)`: NULL is the user's locale, `!x-sys-default-locale` the
/// system's, a known name its identifier (a neutral one's default's unless
/// `LOCALE_ALLOW_NEUTRAL_NAMES`), a well-formed unknown one `LOCALE_CUSTOM_UNSPECIFIED`; a
/// malformed name and any flag but that one are `ERROR_INVALID_PARAMETER`.
pub fn name_to_lcid(name: Option<&[u16]>, flags: u32) -> Result<u32, u32> {
    if flags & !LOCALE_ALLOW_NEUTRAL_NAMES != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let neutral = flags & LOCALE_ALLOW_NEUTRAL_NAMES != 0;
    match name {
        None => Ok(user_default().lcid),
        Some(n) if is_system_default_name(n) => Ok(system_default().lcid),
        Some(n) => match lcid::lookup(n) {
            Some(Named::Known(e)) => Ok(lcid::lcid_of(e, neutral)),
            Some(Named::Custom { .. }) => Ok(LOCALE_CUSTOM_UNSPECIFIED),
            None => Err(ERROR_INVALID_PARAMETER),
        },
    }
}

/// Entries sorted the way the enumerations deliver them: by name, ASCII case-insensitively.
fn sorted(mut list: Vec<&'static Entry>) -> Vec<&'static Entry> {
    list.sort_by_key(|e| e.name.to_ascii_lowercase());
    list
}

/// The identifiers `EnumSystemLocalesW(flags)` delivers, as eight lower-case hexadecimal
/// digits: `LCID_INSTALLED`, `LCID_SUPPORTED` and 0 the specific locales, `LCID_ALTERNATE_SORTS`
/// the alternate sorts, which may be added to either of the first two; both of them together
/// and any other flag are `ERROR_INVALID_FLAGS`.
pub fn enum_lcids(flags: u32) -> Result<Vec<Vec<u16>>, u32> {
    if flags & !(LCID_INSTALLED | LCID_SUPPORTED | LCID_ALTERNATE_SORTS) != 0
        || flags & (LCID_INSTALLED | LCID_SUPPORTED) == LCID_INSTALLED | LCID_SUPPORTED
    {
        return Err(ERROR_INVALID_FLAGS);
    }
    let normal = flags & (LCID_INSTALLED | LCID_SUPPORTED) != 0 || flags & LCID_ALTERNATE_SORTS == 0;
    let alt = flags & LCID_ALTERNATE_SORTS != 0;
    let list = sorted(
        lcid::ENTRIES
            .iter()
            .filter(|e| (normal && e.kind == Kind::Specific) || (alt && e.kind == Kind::AltSort))
            .collect(),
    );
    Ok(list.iter().map(|e| alloc::format!("{:08x}", e.lcid).encode_utf16().collect()).collect())
}

/// The names `EnumSystemLocalesEx(flags)` delivers, each with the flags its callback receives
/// (0x11 neutral, 0x21 specific, 4 an alternate sort). `LOCALE_WINDOWS` is every Windows
/// locale whatever the neutral and specific bits say; without it `LOCALE_NEUTRALDATA` and
/// `LOCALE_SPECIFICDATA` choose; `LOCALE_ALTERNATE_SORTS` adds the alternate sorts; 0 is all
/// of them; `LOCALE_SUPPLEMENTAL` adds nothing (there are no supplemental locales). Any other
/// flag is `ERROR_INVALID_FLAGS`.
pub fn enum_names(flags: u32) -> Result<Vec<(Vec<u16>, u32)>, u32> {
    let known = LOCALE_WINDOWS | LOCALE_SUPPLEMENTAL | LOCALE_ALTERNATE_SORTS | LOCALE_NEUTRALDATA | LOCALE_SPECIFICDATA;
    if flags & !known != 0 {
        return Err(ERROR_INVALID_FLAGS);
    }
    let all = flags == 0;
    let windows = all || flags & LOCALE_WINDOWS != 0;
    let neutral = windows || flags & LOCALE_NEUTRALDATA != 0;
    let specific = windows || flags & LOCALE_SPECIFICDATA != 0;
    let alt = all || flags & LOCALE_ALTERNATE_SORTS != 0;
    let list = sorted(
        lcid::ENTRIES
            .iter()
            .filter(|e| match e.kind {
                Kind::Neutral => neutral,
                Kind::Specific => specific,
                Kind::AltSort => alt,
                Kind::Pseudo | Kind::Alias => false,
            })
            .collect(),
    );
    Ok(list
        .iter()
        .map(|e| {
            let kind = match e.kind {
                Kind::Neutral => LOCALE_WINDOWS | LOCALE_NEUTRALDATA,
                Kind::Specific => LOCALE_WINDOWS | LOCALE_SPECIFICDATA,
                _ => LOCALE_ALTERNATE_SORTS,
            };
            (e.name.encode_utf16().collect(), kind)
        })
        .collect())
}

/// `GetUserPreferredUILanguages`' flags: `MUI_LANGUAGE_ID` 4 or `MUI_LANGUAGE_NAME` 8 (not
/// both), and the bit 2 that changes nothing; anything else is `ERROR_INVALID_PARAMETER`.
/// The list is the user's one UI language, en-US (as "0409" with `MUI_LANGUAGE_ID`).
pub fn user_preferred_ui_languages(flags: u32) -> Result<Vec<Vec<u16>>, u32> {
    use crate::uilang::{MUI_LANGUAGE_ID, MUI_LANGUAGE_NAME};
    if flags & !(MUI_LANGUAGE_ID | MUI_LANGUAGE_NAME | 0x2) != 0
        || flags & (MUI_LANGUAGE_ID | MUI_LANGUAGE_NAME) == MUI_LANGUAGE_ID | MUI_LANGUAGE_NAME
    {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let text = if flags & MUI_LANGUAGE_ID != 0 {
        alloc::format!("{:04X}", LCID_EN_US & 0xFFFF)
    } else {
        alloc::string::String::from(crate::locale::NAME_EN_US)
    };
    Ok(alloc::vec![text.encode_utf16().collect()])
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::string::String;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn st(y: u16, mo: u16, d: u16, h: u16, mi: u16, s: u16, ms: u16) -> SystemTime {
        SystemTime([y, mo, 0, d, h, mi, s, ms])
    }

    fn time(lcid: u32, flags: u32, t: &SystemTime) -> Result<String, u32> {
        time_text(formats_of_lcid(lcid)?, flags, t, None).map(|v| String::from_utf16(&v).unwrap())
    }

    fn date_ex(name: &str, flags: u32, t: &SystemTime) -> Result<String, u32> {
        date_text(formats_of_name(Some(&w(name)))?, flags, t, None).map(|v| String::from_utf16(&v).unwrap())
    }

    #[test]
    fn times_by_identifier() {
        let morning = st(2024, 3, 5, 7, 8, 9, 0);
        assert_eq!(time(0x409, 0, &morning).unwrap(), "7:08:09 AM");
        assert_eq!(time(0x807, 0, &morning).unwrap(), "07:08:09");
        assert_eq!(time(0x411, 0, &morning).unwrap(), "7:08:09");
        assert_eq!(time(0x9, 0, &morning).unwrap(), "7:08:09 AM");
        assert_eq!(time(0x1007F, 0, &morning).unwrap(), "07:08:09");
        assert_eq!(time(0x400, 0, &morning), time(0x409, 0, &morning));
        assert_eq!(time(0x1234, 0, &morning), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(time(0x10409, 0, &morning), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(time(0x409, 0x10, &morning), Err(ERROR_INVALID_FLAGS));
        assert_eq!(time(0x409, 0x8000_0000, &morning).unwrap(), "7:08:09 AM");
        assert_eq!(time(0x409, 0, &st(0, 0, 0, 24, 0, 0, 0)), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(time(0x409, 0, &st(0, 0, 0, 1, 1, 60, 0)).unwrap(), "1:01:60 AM");
        assert_eq!(time(0x409, 0, &st(0, 0, 0, 1, 1, 1, 1000)), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn dates_by_name() {
        let d = st(2025, 11, 5, 0, 0, 0, 0);
        assert_eq!(date_ex("en", DATE_LONGDATE, &d).unwrap(), "Wednesday, November 5, 2025");
        assert_eq!(date_ex("de", DATE_LONGDATE, &d).unwrap(), "Mittwoch, 5. November 2025");
        assert_eq!(date_ex("en-US_tradnl", DATE_LONGDATE, &d).unwrap(), "Wednesday, November 5, 2025");
        assert_eq!(date_ex("x-IV_mathan", DATE_LONGDATE, &d).unwrap(), "Wednesday, 05 November 2025");
        assert_eq!(date_ex("xx-XX", 0, &d).unwrap(), "11/05/2025");
        assert_eq!(date_ex("en_US", 0, &d).unwrap(), "11/5/2025");
        assert_eq!(date_ex("zh-Hans", DATE_LONGDATE, &d).unwrap(), "2025\u{5E74}11\u{6708}5\u{65E5}");
        assert_eq!(date_ex("en US", 0, &d), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(date_ex("en-US", 0x80, &d), Err(ERROR_INVALID_FLAGS));
        assert_eq!(date_ex("en-US", DATE_SHORTDATE | DATE_LONGDATE, &d), Err(ERROR_INVALID_FLAGS));
        assert_eq!(date_ex("en-US", 0x30, &d), Err(ERROR_INVALID_FLAGS));
        assert_eq!(date_ex("en-US", DATE_AUTOLAYOUT, &st(2024, 3, 5, 0, 0, 0, 0)).unwrap(), "\u{200E}3/\u{200E}5/\u{200E}2024");
        assert_eq!(date_ex("en-US", 0, &st(2023, 2, 29, 0, 0, 0, 0)), Err(ERROR_INVALID_PARAMETER));
        let p = formats_of_name(None).unwrap();
        assert_eq!(date_text(p, DATE_LONGDATE, &d, Some(&w("d"))), Err(ERROR_INVALID_FLAGS));
        assert_eq!(formats_of_name(Some(&w("!X-SYS-DEFAULT-LOCALE"))).unwrap().name, "en-US");
        assert_eq!(formats_of_name(Some(&w("!x-sys-default-locale "))).err(), Some(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn sizes() {
        assert_eq!(sized(w("ab"), 0, false), Ok((3, None)));
        assert_eq!(sized(w("ab"), 2, true), Err(ERROR_INSUFFICIENT_BUFFER));
        assert_eq!(sized(w("ab"), 3, true), Ok((3, Some(alloc::vec![97, 98, 0]))));
        assert_eq!(check_buffer(-1, true), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(check_buffer(10, false), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn names_and_identifiers() {
        assert_eq!(lcid_to_name(0x409, 0, false, 0), Ok(NameAnswer::Size(6)));
        assert_eq!(lcid_to_name(0x409, 5, true, 0), Ok(NameAnswer::Truncated(w("en-U\0"))));
        assert_eq!(lcid_to_name(0x7F, 1, true, 0), Ok(NameAnswer::Name(w("\0"))));
        assert_eq!(lcid_to_name(0x9, 10, true, LOCALE_ALLOW_NEUTRAL_NAMES), Ok(NameAnswer::Name(w("en\0"))));
        assert_eq!(lcid_to_name(0x409, 10, true, 1), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(lcid_to_name(0x10409, 10, true, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(lcid_to_name(0x409, 10, false, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(name_to_lcid(None, 0), Ok(0x409));
        assert_eq!(name_to_lcid(Some(&w("zh")), LOCALE_ALLOW_NEUTRAL_NAMES), Ok(0x7804));
        assert_eq!(name_to_lcid(Some(&w("en-001")), 0), Ok(0x1000));
        assert_eq!(name_to_lcid(Some(&w("")), 0), Ok(0x7F));
        assert_eq!(name_to_lcid(Some(&w("409")), 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(name_to_lcid(Some(&w("en")), 0x8000_0000), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn enumerations() {
        let installed = enum_lcids(LCID_INSTALLED).unwrap();
        assert_eq!(installed, enum_lcids(LCID_SUPPORTED).unwrap());
        assert!(installed.contains(&w("0000040c")) && !installed.contains(&w("00000009")));
        let alt = enum_lcids(LCID_ALTERNATE_SORTS).unwrap();
        assert!(alt.contains(&w("00010407")) && alt.contains(&w("0001007f")));
        assert_eq!(enum_lcids(LCID_SUPPORTED | LCID_ALTERNATE_SORTS).unwrap().len(), installed.len() + alt.len());
        assert_eq!(enum_lcids(3), Err(ERROR_INVALID_FLAGS));
        assert_eq!(enum_lcids(8), Err(ERROR_INVALID_FLAGS));
        let names = |f: u32| enum_names(f).unwrap();
        let neutral = names(LOCALE_NEUTRALDATA);
        let specific = names(LOCALE_SPECIFICDATA);
        assert_eq!(names(0x30).len(), neutral.len() + specific.len());
        assert_eq!(names(0x11), names(LOCALE_WINDOWS));
        assert!(names(LOCALE_WINDOWS).len() > 300 && specific.len() > neutral.len());
        assert!(names(LOCALE_SUPPLEMENTAL).is_empty());
        assert_eq!(neutral[0], (w(""), 0x11));
        assert!(names(0).contains(&(w("de-DE_phoneb"), 4)));
        assert!(!names(0).iter().any(|(n, _)| n == &w("qps-ploc") || n == &w("zh-Hans-CN")));
        let all = names(0);
        assert!(all.windows(2).all(|p| String::from_utf16(&p[0].0).unwrap().to_ascii_lowercase()
            <= String::from_utf16(&p[1].0).unwrap().to_ascii_lowercase()));
        assert_eq!(enum_names(8), Err(ERROR_INVALID_FLAGS));
    }

    #[test]
    fn preferred_ui_languages() {
        assert_eq!(user_preferred_ui_languages(0).unwrap(), [w("en-US")]);
        assert_eq!(user_preferred_ui_languages(4 | 2).unwrap(), [w("0409")]);
        assert_eq!(user_preferred_ui_languages(12), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(user_preferred_ui_languages(0x10), Err(ERROR_INVALID_PARAMETER));
    }
}
