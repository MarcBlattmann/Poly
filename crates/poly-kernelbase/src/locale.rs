//! The locale and character classification rules behind `kernelbase`'s
//! `GetLocaleInfoW`, `GetStringTypeW`, `LCMapStringW` and `CompareStringW`
//! (ADR 0155). Constants are from `winnls.h` and `winnt.h`; the values for
//! English (United States) are the ones Learn's locale pages give. Poly has
//! one locale for the system, the user and the thread.

/// `MAKELCID(0x0409, SORT_DEFAULT)`: English (United States).
pub const LCID_EN_US: u32 = 0x0409;
/// The locale name for it (BCP 47).
pub const NAME_EN_US: &str = "en-US";

/// `LOCALE_NOUSEROVERRIDE`: ignored (there is no user override).
pub const LOCALE_NOUSEROVERRIDE: u32 = 0x8000_0000;
/// `LOCALE_RETURN_NUMBER`: a numeric value is returned as a `DWORD`.
pub const LOCALE_RETURN_NUMBER: u32 = 0x2000_0000;

/// `LOCALE_USER_DEFAULT`, `LOCALE_SYSTEM_DEFAULT`, `LOCALE_NEUTRAL`,
/// `LOCALE_INVARIANT` (`winnls.h`), and the neutral English language.
const VALID_LCIDS: [u32; 6] = [LCID_EN_US, 0x0009, 0x0400, 0x0800, 0x0000, 0x007F];

/// `IsValidLocale`: `LCID_INSTALLED` (1) and `LCID_SUPPORTED` (2) both
/// accept the locales Poly has.
pub fn is_valid_lcid(lcid: u32, flags: u32) -> bool {
    flags & !3 == 0 && VALID_LCIDS.contains(&lcid)
}

/// An answer of `GetLocaleInfoW`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Info {
    Text(&'static str),
    /// A value Learn gives as a number in a string ("0", "1", "437"),
    /// which `LOCALE_RETURN_NUMBER` asks for as a `DWORD`.
    Number(u32),
}

/// The `LCTYPE` values (`winnls.h`) Poly answers, for English (United
/// States). Others are `None` (`ERROR_INVALID_FLAGS`).
pub fn info(lctype: u32) -> Option<Info> {
    use Info::{Number as N, Text as T};
    // The flag bits are not part of the type.
    Some(match lctype & !LOCALE_RETURN_NUMBER {
        0x0001 => T("0409"),                            // LOCALE_ILANGUAGE
        0x0002 => T("English (United States)"),         // LOCALE_SLANGUAGE
        0x0003 => T("ENU"),                             // LOCALE_SABBREVLANGNAME
        0x0004 => T("English"),                         // LOCALE_SNATIVELANGNAME
        0x0005 => N(1),                                 // LOCALE_ICOUNTRY
        0x0006 | 0x1002 | 0x0008 => T("United States"), // LOCALE_SCOUNTRY, SENGCOUNTRY, SNATIVECTRYNAME
        0x0007 => T("USA"),                             // LOCALE_SABBREVCTRYNAME
        0x0009 => T("0409"),                            // LOCALE_IDEFAULTLANGUAGE
        0x000A => N(1),                                 // LOCALE_IDEFAULTCOUNTRY
        0x000B => N(437),                               // LOCALE_IDEFAULTCODEPAGE
        0x000C => T(","),                               // LOCALE_SLIST
        0x000D => N(1),                                 // LOCALE_IMEASURE (US customary)
        0x000E => T("."),                               // LOCALE_SDECIMAL
        0x000F => T(","),                               // LOCALE_STHOUSAND
        0x0010 => T("3;0"),                             // LOCALE_SGROUPING
        0x0011 => N(2),                                 // LOCALE_IDIGITS
        0x0012 => N(1),                                 // LOCALE_ILZERO
        0x1010 => N(1),                                 // LOCALE_INEGNUMBER
        0x0013 => T("0123456789"),                      // LOCALE_SNATIVEDIGITS
        0x0014 => T("$"),                               // LOCALE_SCURRENCY
        0x0015 => T("USD"),                             // LOCALE_SINTLSYMBOL
        0x0016 => T("."),                               // LOCALE_SMONDECIMALSEP
        0x0017 => T(","),                               // LOCALE_SMONTHOUSANDSEP
        0x0018 => T("3;0"),                             // LOCALE_SMONGROUPING
        0x0019 | 0x001A => N(2),                        // LOCALE_ICURRDIGITS, IINTLCURRDIGITS
        0x001B => N(0),                                 // LOCALE_ICURRENCY
        0x001C => N(0),                                 // LOCALE_INEGCURR
        0x001D => T("/"),                               // LOCALE_SDATE
        0x001E => T(":"),                               // LOCALE_STIME
        0x001F => T("M/d/yyyy"),                        // LOCALE_SSHORTDATE
        0x0020 => T("dddd, MMMM d, yyyy"),              // LOCALE_SLONGDATE
        0x0021 | 0x0022 => N(0),                        // LOCALE_IDATE, ILDATE (month day year)
        0x0023 => N(0),                                 // LOCALE_ITIME (12 hour)
        0x0025 => N(0),                                 // LOCALE_ITLZERO
        0x0028 => T("AM"),                              // LOCALE_S1159
        0x0029 => T("PM"),                              // LOCALE_S2359
        0x1003 => T("h:mm:ss tt"),                      // LOCALE_STIMEFORMAT
        0x1004 => N(1252),                              // LOCALE_IDEFAULTANSICODEPAGE
        0x1001 => T("English"),                         // LOCALE_SENGLANGUAGE
        0x1009 => N(1),                                 // LOCALE_ICALENDARTYPE
        0x100A => N(1),                                 // LOCALE_IPAPERSIZE (Letter)
        0x100C => N(6),                                 // LOCALE_IFIRSTDAYOFWEEK (Sunday)
        0x100D => N(0),                                 // LOCALE_IFIRSTWEEKOFYEAR
        0x0050 => T(""),                                // LOCALE_SPOSITIVESIGN
        0x0051 => T("-"),                               // LOCALE_SNEGATIVESIGN
        0x0059 => T("en"),                              // LOCALE_SISO639LANGNAME
        0x005A => T("US"),                              // LOCALE_SISO3166CTRYNAME
        0x005C => T(NAME_EN_US),                        // LOCALE_SNAME
        0x0069 => T("NaN"),                             // LOCALE_SNAN
        0x006A => T("Infinity"),                        // LOCALE_SPOSINFINITY
        0x006B => T("-Infinity"),                       // LOCALE_SNEGINFINITY
        _ => return None,
    })
}

/// `CT_CTYPE1`.
pub const CT_CTYPE1: u32 = 1;

// Character type 1 flags (`winnls.h`).
pub const C1_UPPER: u16 = 0x1;
pub const C1_LOWER: u16 = 0x2;
pub const C1_DIGIT: u16 = 0x4;
pub const C1_SPACE: u16 = 0x8;
pub const C1_PUNCT: u16 = 0x10;
pub const C1_CNTRL: u16 = 0x20;
pub const C1_BLANK: u16 = 0x40;
pub const C1_XDIGIT: u16 = 0x80;
pub const C1_ALPHA: u16 = 0x100;
pub const C1_DEFINED: u16 = 0x200;

/// The `CT_CTYPE1` flags of one UTF-16 unit. A unit that is half of a
/// surrogate pair has none; `C1_DEFINED` is for a character that has no
/// other type (Learn: "Defined, but not one of the other C1_* types").
pub fn ctype1(unit: u16) -> u16 {
    let Some(c) = char::from_u32(u32::from(unit)) else { return 0 };
    let mut f = 0;
    if c.is_uppercase() {
        f |= C1_UPPER;
    }
    if c.is_lowercase() {
        f |= C1_LOWER;
    }
    if c.is_ascii_digit() || (!c.is_ascii() && c.is_numeric() && !c.is_alphabetic()) {
        f |= C1_DIGIT;
    }
    if c.is_whitespace() {
        f |= C1_SPACE;
        if matches!(c, ' ' | '\t' | '\u{a0}' | '\u{2000}'..='\u{200a}' | '\u{3000}') {
            f |= C1_BLANK;
        }
    }
    if c.is_control() {
        f |= C1_CNTRL;
    }
    if c.is_ascii_hexdigit() {
        f |= C1_XDIGIT;
    }
    if c.is_alphabetic() {
        f |= C1_ALPHA;
    }
    let punct = if c.is_ascii() {
        c.is_ascii_punctuation()
    } else {
        !c.is_alphanumeric() && !c.is_whitespace() && !c.is_control() && !c.is_numeric()
    };
    if punct {
        f |= C1_PUNCT;
    }
    if f == 0 {
        f = C1_DEFINED;
    }
    f
}

// `LCMapString` flags (`winnls.h`).
pub const LCMAP_LOWERCASE: u32 = 0x100;
pub const LCMAP_UPPERCASE: u32 = 0x200;
pub const LCMAP_LINGUISTIC_CASING: u32 = 0x0100_0000;

/// What an `LCMapString` call asks for, if Poly does it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapKind {
    Lower,
    Upper,
}

/// `LCMAP_LOWERCASE` or `LCMAP_UPPERCASE`, alone or with
/// `LCMAP_LINGUISTIC_CASING`; anything else is not done.
pub fn map_kind(flags: u32) -> Option<MapKind> {
    match flags & !LCMAP_LINGUISTIC_CASING {
        LCMAP_LOWERCASE => Some(MapKind::Lower),
        LCMAP_UPPERCASE => Some(MapKind::Upper),
        _ => None,
    }
}

/// One unit mapped; a unit whose mapping is more than one unit, and the
/// halves of surrogate pairs, stay as they are.
pub fn map_unit(kind: MapKind, unit: u16) -> u16 {
    let Some(c) = char::from_u32(u32::from(unit)) else { return unit };
    let mut m: alloc::vec::Vec<char> = match kind {
        MapKind::Lower => c.to_lowercase().collect(),
        MapKind::Upper => c.to_uppercase().collect(),
    };
    match (m.pop(), m.is_empty()) {
        (Some(one), true) => u16::try_from(u32::from(one)).unwrap_or(unit),
        _ => unit,
    }
}

// `CompareString` flags (`winnls.h`).
pub const NORM_IGNORECASE: u32 = 0x1;
/// The flags `CompareStringW` takes (Learn's list for `dwCmpFlags`).
pub const COMPARE_FLAGS_KNOWN: u32 = 0x1 | 0x2 | 0x4 | 0x10 | 0x20 | 0x1000 | 0x8000 | 0x10000 | 0x20000 | 0x0800_0000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_types_match_learns_flags() {
        let t = |c: char| ctype1(c as u16);
        assert_eq!(t('A'), C1_UPPER | C1_XDIGIT | C1_ALPHA);
        assert_eq!(t('a'), C1_LOWER | C1_XDIGIT | C1_ALPHA);
        assert_eq!(t('g'), C1_LOWER | C1_ALPHA);
        assert_eq!(t('7'), C1_DIGIT | C1_XDIGIT);
        assert_eq!(t(' '), C1_SPACE | C1_BLANK);
        assert_eq!(t('\n'), C1_SPACE | C1_CNTRL);
        assert_eq!(t('\t'), C1_SPACE | C1_BLANK | C1_CNTRL);
        assert_eq!(t('!'), C1_PUNCT);
        assert_eq!(t('\u{7}'), C1_CNTRL);
        assert_eq!(t('é'), C1_LOWER | C1_ALPHA);
        // Half of a surrogate pair has no type.
        assert_eq!(ctype1(0xD800), 0);
    }

    #[test]
    fn mapping_follows_the_flags() {
        assert_eq!(map_kind(LCMAP_LOWERCASE), Some(MapKind::Lower));
        assert_eq!(map_kind(LCMAP_UPPERCASE | LCMAP_LINGUISTIC_CASING), Some(MapKind::Upper));
        assert_eq!(map_kind(0), None);
        assert_eq!(map_kind(LCMAP_LOWERCASE | LCMAP_UPPERCASE), None);
        assert_eq!(map_kind(0x400), None, "sort keys are not done");
        assert_eq!(map_unit(MapKind::Lower, 'Q' as u16), 'q' as u16);
        assert_eq!(map_unit(MapKind::Upper, 'é' as u16), 'É' as u16);
        // 'ß' uppercases to two letters: left alone.
        assert_eq!(map_unit(MapKind::Upper, 'ß' as u16), 'ß' as u16);
        assert_eq!(map_unit(MapKind::Upper, 0xD83D), 0xD83D);
    }

    #[test]
    fn the_locale_is_english_united_states() {
        assert!(is_valid_lcid(0x0409, 1) && is_valid_lcid(0x0400, 0) && !is_valid_lcid(0x0407, 0));
        assert!(!is_valid_lcid(0x0409, 4), "unknown flags");
        assert_eq!(info(0x5C), Some(Info::Text("en-US")));
        assert_eq!(info(0x1004 | LOCALE_RETURN_NUMBER), Some(Info::Number(1252)));
        assert_eq!(info(0x0E), Some(Info::Text(".")));
        assert_eq!(info(0x7777), None);
    }
}
