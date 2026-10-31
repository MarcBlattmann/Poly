//! The locales Poly knows (ADR 0094): names, identifiers, code pages and
//! the number and currency formats the C runtime's `localeconv` reports.
//!
//! Sources (clean room): Learn's "Locale names, Languages, and
//! Country/Region strings", "Language strings", "Country/Region strings",
//! "Code Page Identifiers", the `LOCALE_*` constants' pages (`SDECIMAL`,
//! `STHOUSAND`, `SGROUPING`, `SCURRENCY`, `SINTLSYMBOL`, `ICURRENCY`,
//! `INEGCURR`, `ICURRDIGITS`, `IDEFAULTANSICODEPAGE`,
//! `IDEFAULTCODEPAGE`, `SENGLISHLANGUAGENAME`, `SENGLISHCOUNTRYNAME`,
//! `SABBREVLANGNAME`, `SABBREVCTRYNAME`), [MS-LCID] for the identifiers,
//! and the Unicode CLDR's published regional formats for the separators
//! and currency symbols. The values are Poly's table, to be confirmed by
//! the spec agent against Windows' (which changes them between releases,
//! e.g. the thousands separator of de-CH).
//!
//! Poly's user and system locale is English (United States), LCID 0x0409,
//! ANSI code page 1252, OEM code page 437 (ADR 0079).

/// One locale.
#[derive(Debug)]
pub struct Locale {
    /// The locale name (BCP-47, `LOCALE_SNAME`).
    pub name: &'static str,
    pub lcid: u32,
    /// `LOCALE_SENGLISHLANGUAGENAME`, `LOCALE_SENGLISHCOUNTRYNAME`.
    pub language: &'static str,
    pub country: &'static str,
    /// `LOCALE_SABBREVLANGNAME`, `LOCALE_SABBREVCTRYNAME`.
    pub abbrev_language: &'static str,
    pub abbrev_country: &'static str,
    /// `LOCALE_IDEFAULTANSICODEPAGE`, `LOCALE_IDEFAULTCODEPAGE`.
    pub ansi: u16,
    pub oem: u16,
    /// `LOCALE_SDECIMAL`, `LOCALE_STHOUSAND`.
    pub decimal: &'static str,
    pub thousands: &'static str,
    /// `LOCALE_SGROUPING` as C's `grouping` (`"3;0"` is `[3]`, `"3;2;0"`
    /// `[3, 2]`).
    pub grouping: &'static [u8],
    /// `LOCALE_SCURRENCY`, `LOCALE_SINTLSYMBOL`.
    pub currency: &'static str,
    pub intl_symbol: &'static str,
    /// `LOCALE_ICURRENCY` (positive currency format, 0-3),
    /// `LOCALE_INEGCURR` (negative, 0-15), `LOCALE_ICURRDIGITS`.
    pub pos_currency: u8,
    pub neg_currency: u8,
    pub currency_digits: u8,
}

/// Two locales are the same when their names are.
impl PartialEq for Locale {
    fn eq(&self, o: &Self) -> bool {
        self.name == o.name
    }
}
impl Eq for Locale {}

#[allow(clippy::too_many_arguments)]
const fn loc(
    name: &'static str,
    lcid: u32,
    language: &'static str,
    country: &'static str,
    abbrev: [&'static str; 2],
    cps: [u16; 2],
    numbers: [&'static str; 2],
    grouping: &'static [u8],
    currency: [&'static str; 2],
    formats: [u8; 3],
) -> Locale {
    Locale {
        name,
        lcid,
        language,
        country,
        abbrev_language: abbrev[0],
        abbrev_country: abbrev[1],
        ansi: cps[0],
        oem: cps[1],
        decimal: numbers[0],
        thousands: numbers[1],
        grouping,
        currency: currency[0],
        intl_symbol: currency[1],
        pos_currency: formats[0],
        neg_currency: formats[1],
        currency_digits: formats[2],
    }
}

const G3: &[u8] = &[3];
const G32: &[u8] = &[3, 2];
const NBSP: &str = "\u{A0}";

/// Every locale Poly has. The first of each language is its default
/// (what the language's name alone selects).
pub static LOCALES: &[Locale] = &[
    loc(
        "en-US",
        0x0409,
        "English",
        "United States",
        ["ENU", "USA"],
        [1252, 437],
        [".", ","],
        G3,
        ["$", "USD"],
        [0, 0, 2],
    ),
    loc(
        "en-GB",
        0x0809,
        "English",
        "United Kingdom",
        ["ENG", "GBR"],
        [1252, 850],
        [".", ","],
        G3,
        ["\u{A3}", "GBP"],
        [0, 1, 2],
    ),
    loc("en-AU", 0x0C09, "English", "Australia", ["ENA", "AUS"], [1252, 850], [".", ","], G3, ["$", "AUD"], [0, 1, 2]),
    loc("en-CA", 0x1009, "English", "Canada", ["ENC", "CAN"], [1252, 850], [".", ","], G3, ["$", "CAD"], [0, 1, 2]),
    loc(
        "en-NZ",
        0x1409,
        "English",
        "New Zealand",
        ["ENZ", "NZL"],
        [1252, 850],
        [".", ","],
        G3,
        ["$", "NZD"],
        [0, 1, 2],
    ),
    loc(
        "en-IE",
        0x1809,
        "English",
        "Ireland",
        ["ENI", "IRL"],
        [1252, 850],
        [".", ","],
        G3,
        ["\u{20AC}", "EUR"],
        [0, 1, 2],
    ),
    loc(
        "en-IN",
        0x4009,
        "English",
        "India",
        ["ENN", "IND"],
        [1252, 437],
        [".", ","],
        G32,
        ["\u{20B9}", "INR"],
        [2, 12, 2],
    ),
    loc(
        "de-DE",
        0x0407,
        "German",
        "Germany",
        ["DEU", "DEU"],
        [1252, 850],
        [",", "."],
        G3,
        ["\u{20AC}", "EUR"],
        [3, 8, 2],
    ),
    loc(
        "de-CH",
        0x0807,
        "German",
        "Switzerland",
        ["DES", "CHE"],
        [1252, 850],
        [".", "'"],
        G3,
        ["CHF", "CHF"],
        [2, 12, 2],
    ),
    loc(
        "de-AT",
        0x0C07,
        "German",
        "Austria",
        ["DEA", "AUT"],
        [1252, 850],
        [",", "."],
        G3,
        ["\u{20AC}", "EUR"],
        [2, 9, 2],
    ),
    loc(
        "de-LU",
        0x1007,
        "German",
        "Luxembourg",
        ["DEL", "LUX"],
        [1252, 850],
        [",", "."],
        G3,
        ["\u{20AC}", "EUR"],
        [3, 8, 2],
    ),
    loc(
        "de-LI",
        0x1407,
        "German",
        "Liechtenstein",
        ["DEC", "LIE"],
        [1252, 850],
        [".", "'"],
        G3,
        ["CHF", "CHF"],
        [2, 12, 2],
    ),
    loc(
        "fr-FR",
        0x040C,
        "French",
        "France",
        ["FRA", "FRA"],
        [1252, 850],
        [",", NBSP],
        G3,
        ["\u{20AC}", "EUR"],
        [3, 8, 2],
    ),
    loc(
        "fr-BE",
        0x080C,
        "French",
        "Belgium",
        ["FRB", "BEL"],
        [1252, 850],
        [",", "."],
        G3,
        ["\u{20AC}", "EUR"],
        [3, 8, 2],
    ),
    loc("fr-CA", 0x0C0C, "French", "Canada", ["FRC", "CAN"], [1252, 850], [",", NBSP], G3, ["$", "CAD"], [3, 15, 2]),
    loc(
        "fr-CH",
        0x100C,
        "French",
        "Switzerland",
        ["FRS", "CHE"],
        [1252, 850],
        [".", "'"],
        G3,
        ["CHF", "CHF"],
        [2, 12, 2],
    ),
    loc(
        "fr-LU",
        0x140C,
        "French",
        "Luxembourg",
        ["FRL", "LUX"],
        [1252, 850],
        [",", "."],
        G3,
        ["\u{20AC}", "EUR"],
        [3, 8, 2],
    ),
    loc(
        "it-IT",
        0x0410,
        "Italian",
        "Italy",
        ["ITA", "ITA"],
        [1252, 850],
        [",", "."],
        G3,
        ["\u{20AC}", "EUR"],
        [3, 8, 2],
    ),
    loc(
        "it-CH",
        0x0810,
        "Italian",
        "Switzerland",
        ["ITS", "CHE"],
        [1252, 850],
        [".", "'"],
        G3,
        ["CHF", "CHF"],
        [2, 12, 2],
    ),
    loc(
        "es-ES",
        0x0C0A,
        "Spanish",
        "Spain",
        ["ESN", "ESP"],
        [1252, 850],
        [",", "."],
        G3,
        ["\u{20AC}", "EUR"],
        [3, 8, 2],
    ),
    loc("es-MX", 0x080A, "Spanish", "Mexico", ["ESM", "MEX"], [1252, 850], [".", ","], G3, ["$", "MXN"], [0, 1, 2]),
    loc("pt-BR", 0x0416, "Portuguese", "Brazil", ["PTB", "BRA"], [1252, 850], [",", "."], G3, ["R$", "BRL"], [2, 9, 2]),
    loc(
        "pt-PT",
        0x0816,
        "Portuguese",
        "Portugal",
        ["PTG", "PRT"],
        [1252, 850],
        [",", NBSP],
        G3,
        ["\u{20AC}", "EUR"],
        [3, 8, 2],
    ),
    loc(
        "nl-NL",
        0x0413,
        "Dutch",
        "Netherlands",
        ["NLD", "NLD"],
        [1252, 850],
        [",", "."],
        G3,
        ["\u{20AC}", "EUR"],
        [2, 12, 2],
    ),
    loc(
        "nl-BE",
        0x0813,
        "Dutch",
        "Belgium",
        ["NLB", "BEL"],
        [1252, 850],
        [",", "."],
        G3,
        ["\u{20AC}", "EUR"],
        [3, 8, 2],
    ),
    loc("sv-SE", 0x041D, "Swedish", "Sweden", ["SVE", "SWE"], [1252, 850], [",", NBSP], G3, ["kr", "SEK"], [3, 8, 2]),
    loc(
        "sv-FI",
        0x081D,
        "Swedish",
        "Finland",
        ["SVF", "FIN"],
        [1252, 850],
        [",", NBSP],
        G3,
        ["\u{20AC}", "EUR"],
        [3, 8, 2],
    ),
    loc("da-DK", 0x0406, "Danish", "Denmark", ["DAN", "DNK"], [1252, 850], [",", "."], G3, ["kr.", "DKK"], [2, 12, 2]),
    loc(
        "nb-NO",
        0x0414,
        "Norwegian",
        "Norway",
        ["NOR", "NOR"],
        [1252, 850],
        [",", NBSP],
        G3,
        ["kr", "NOK"],
        [2, 12, 2],
    ),
    loc(
        "fi-FI",
        0x040B,
        "Finnish",
        "Finland",
        ["FIN", "FIN"],
        [1252, 850],
        [",", NBSP],
        G3,
        ["\u{20AC}", "EUR"],
        [3, 8, 2],
    ),
    loc(
        "is-IS",
        0x040F,
        "Icelandic",
        "Iceland",
        ["ISL", "ISL"],
        [1252, 861],
        [",", "."],
        G3,
        ["kr.", "ISK"],
        [3, 8, 0],
    ),
    loc(
        "pl-PL",
        0x0415,
        "Polish",
        "Poland",
        ["PLK", "POL"],
        [1250, 852],
        [",", NBSP],
        G3,
        ["z\u{142}", "PLN"],
        [3, 8, 2],
    ),
    loc(
        "cs-CZ",
        0x0405,
        "Czech",
        "Czech Republic",
        ["CSY", "CZE"],
        [1250, 852],
        [",", NBSP],
        G3,
        ["K\u{10D}", "CZK"],
        [3, 8, 2],
    ),
    loc(
        "sk-SK",
        0x041B,
        "Slovak",
        "Slovakia",
        ["SKY", "SVK"],
        [1250, 852],
        [",", NBSP],
        G3,
        ["\u{20AC}", "EUR"],
        [3, 8, 2],
    ),
    loc(
        "hu-HU",
        0x040E,
        "Hungarian",
        "Hungary",
        ["HUN", "HUN"],
        [1250, 852],
        [",", NBSP],
        G3,
        ["Ft", "HUF"],
        [3, 8, 2],
    ),
    loc(
        "ru-RU",
        0x0419,
        "Russian",
        "Russia",
        ["RUS", "RUS"],
        [1251, 866],
        [",", NBSP],
        G3,
        ["\u{20BD}", "RUB"],
        [3, 8, 2],
    ),
    loc(
        "uk-UA",
        0x0422,
        "Ukrainian",
        "Ukraine",
        ["UKR", "UKR"],
        [1251, 866],
        [",", NBSP],
        G3,
        ["\u{20B4}", "UAH"],
        [3, 8, 2],
    ),
    loc(
        "el-GR",
        0x0408,
        "Greek",
        "Greece",
        ["ELL", "GRC"],
        [1253, 737],
        [",", "."],
        G3,
        ["\u{20AC}", "EUR"],
        [3, 8, 2],
    ),
    loc(
        "tr-TR",
        0x041F,
        "Turkish",
        "Turkey",
        ["TRK", "TUR"],
        [1254, 857],
        [",", "."],
        G3,
        ["\u{20BA}", "TRY"],
        [0, 1, 2],
    ),
    loc(
        "he-IL",
        0x040D,
        "Hebrew",
        "Israel",
        ["HEB", "ISR"],
        [1255, 862],
        [".", ","],
        G3,
        ["\u{20AA}", "ILS"],
        [2, 12, 2],
    ),
    loc(
        "lt-LT",
        0x0427,
        "Lithuanian",
        "Lithuania",
        ["LTH", "LTU"],
        [1257, 775],
        [",", NBSP],
        G3,
        ["\u{20AC}", "EUR"],
        [3, 8, 2],
    ),
    loc(
        "vi-VN",
        0x042A,
        "Vietnamese",
        "Vietnam",
        ["VIT", "VNM"],
        [1258, 1258],
        [",", "."],
        G3,
        ["\u{20AB}", "VND"],
        [3, 8, 0],
    ),
    loc("th-TH", 0x041E, "Thai", "Thailand", ["THA", "THA"], [874, 874], [".", ","], G3, ["\u{E3F}", "THB"], [0, 1, 2]),
    loc("ja-JP", 0x0411, "Japanese", "Japan", ["JPN", "JPN"], [932, 932], [".", ","], G3, ["\u{A5}", "JPY"], [0, 1, 0]),
    loc(
        "zh-CN",
        0x0804,
        "Chinese (Simplified)",
        "China",
        ["CHS", "CHN"],
        [936, 936],
        [".", ","],
        G3,
        ["\u{A5}", "CNY"],
        [0, 2, 2],
    ),
    loc(
        "zh-TW",
        0x0404,
        "Chinese (Traditional)",
        "Taiwan",
        ["CHT", "TWN"],
        [950, 950],
        [".", ","],
        G3,
        ["NT$", "TWD"],
        [0, 1, 2],
    ),
    loc(
        "zh-HK",
        0x0C04,
        "Chinese (Traditional)",
        "Hong Kong SAR",
        ["ZHH", "HKG"],
        [950, 950],
        [".", ","],
        G3,
        ["HK$", "HKD"],
        [0, 1, 2],
    ),
    loc("ko-KR", 0x0412, "Korean", "Korea", ["KOR", "KOR"], [949, 949], [".", ","], G3, ["\u{20A9}", "KRW"], [0, 1, 0]),
];

/// The user's (and system's) locale: en-US.
pub fn user_default() -> &'static Locale {
    &LOCALES[0]
}

/// The primary language of a locale (`PRIMARYLANGID`, `winnt.h`).
pub fn primary_language(l: &Locale) -> u32 {
    l.lcid & 0x3FF
}

fn eq_ci(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).all(|(x, y)| x.eq_ignore_ascii_case(&y))
}

/// The locale a BCP-47 name names (`de-CH`, case-insensitive; `_` for `-`
/// is accepted, `en_US`, as Learn's UTF-8 examples write it).
pub fn by_name(name: &str) -> Option<&'static Locale> {
    if name.len() < 2 {
        return None;
    }
    LOCALES.iter().find(|l| {
        l.name.len() == name.len()
            && l.name.bytes().zip(name.bytes()).all(|(x, y)| x.eq_ignore_ascii_case(&y) || (x == b'-' && y == b'_'))
    })
}

/// The locale with this identifier.
pub fn by_lcid(lcid: u32) -> Option<&'static Locale> {
    LOCALES.iter().find(|l| l.lcid == lcid)
}

/// Learn's "Language strings" that name one locale (language and
/// country/region).
const LANGUAGE_ALIASES: &[(&str, &str)] = &[
    ("american", "en-US"),
    ("american english", "en-US"),
    ("american-english", "en-US"),
    ("english-american", "en-US"),
    ("english-us", "en-US"),
    ("english-usa", "en-US"),
    ("us", "en-US"),
    ("usa", "en-US"),
    ("australian", "en-AU"),
    ("english-aus", "en-AU"),
    ("belgian", "nl-BE"),
    ("dutch-belgian", "nl-BE"),
    ("canadian", "en-CA"),
    ("english-can", "en-CA"),
    ("english-nz", "en-NZ"),
    ("english-ire", "en-IE"),
    ("irish-english", "en-IE"),
    ("english-uk", "en-GB"),
    ("uk", "en-GB"),
    ("chinese", "zh-CN"),
    ("chi", "zh-CN"),
    ("chinese-simplified", "zh-CN"),
    ("chs", "zh-CN"),
    ("chinese-traditional", "zh-TW"),
    ("cht", "zh-TW"),
    ("chinese-hongkong", "zh-HK"),
    ("chh", "zh-HK"),
    ("french-belgian", "fr-BE"),
    ("french-canadian", "fr-CA"),
    ("french-luxembourg", "fr-LU"),
    ("french-swiss", "fr-CH"),
    ("german-austrian", "de-AT"),
    ("german-lichtenstein", "de-LI"),
    ("german-luxembourg", "de-LU"),
    ("german-swiss", "de-CH"),
    ("swiss", "de-CH"),
    ("italian-swiss", "it-CH"),
    ("norwegian-bokmal", "nb-NO"),
    ("portuguese-brazilian", "pt-BR"),
    ("spanish-mexican", "es-MX"),
    ("spanish-modern", "es-ES"),
    ("swedish-finland", "sv-FI"),
];

/// Learn's "Country/Region strings" and the names they stand for.
const COUNTRY_ALIASES: &[(&str, &str)] = &[
    ("america", "United States"),
    ("united-states", "United States"),
    ("us", "United States"),
    ("britain", "United Kingdom"),
    ("england", "United Kingdom"),
    ("great britain", "United Kingdom"),
    ("uk", "United Kingdom"),
    ("united-kingdom", "United Kingdom"),
    ("china", "China"),
    ("pr china", "China"),
    ("pr-china", "China"),
    ("czech", "Czech Republic"),
    ("holland", "Netherlands"),
    ("hong-kong", "Hong Kong SAR"),
    ("new-zealand", "New Zealand"),
    ("nz", "New Zealand"),
    ("slovak", "Slovakia"),
    ("south korea", "Korea"),
    ("south-korea", "Korea"),
];

/// The locale a language string names: an alias of one locale, an
/// English language name, a three-letter abbreviation (`deu`, `des`), or
/// a locale name.
pub fn by_language(s: &str) -> Option<&'static Locale> {
    if let Some(&(_, name)) = LANGUAGE_ALIASES.iter().find(|(a, _)| eq_ci(a, s)) {
        return by_name(name);
    }
    LOCALES
        .iter()
        .find(|l| eq_ci(l.language, s) || eq_ci(l.abbrev_language, s))
        .or_else(|| LOCALES.iter().find(|l| eq_ci(l.language.split(' ').next().unwrap_or(""), s)))
}

/// The locale for a language and a country/region string: the locale of
/// the language's primary language whose country matches (its English
/// name, three-letter abbreviation, or one of Learn's country strings).
pub fn by_language_country(language: &str, country: &str) -> Option<&'static Locale> {
    let lang = by_language(language)?;
    let country = COUNTRY_ALIASES.iter().find(|(a, _)| eq_ci(a, country)).map_or(country, |&(_, c)| c);
    LOCALES.iter().find(|l| {
        primary_language(l) == primary_language(lang) && (eq_ci(l.country, country) || eq_ci(l.abbrev_country, country))
    })
}

/// The C runtime's long form of a locale's name: `Language_Country`
/// (`LOCALE_SENGLISHLANGUAGENAME` `_` `LOCALE_SENGLISHCOUNTRYNAME`).
pub fn long_name(l: &Locale) -> (&'static str, &'static str) {
    (l.language, l.country)
}

/// The parts of C's `lconv` a currency format decides: `(cs_precedes,
/// sep_by_space, sign_posn)` for `LOCALE_INEGCURR`'s sixteen negative
/// formats (Learn's table: 0 `($1.1)`, 1 `-$1.1`, 2 `$-1.1`, 3 `$1.1-`,
/// 4 `(1.1$)`, 5 `-1.1$`, 6 `1.1-$`, 7 `1.1$-`, 8 `-1.1 $`, 9 `-$ 1.1`,
/// 10 `1.1 $-`, 11 `$ 1.1-`, 12 `$ -1.1`, 13 `1.1- $`, 14 `($ 1.1)`,
/// 15 `(1.1 $)`) and C11 7.11.2.1's sign positions (0 parentheses, 1 sign
/// first, 2 sign last, 3 sign just before the symbol, 4 just after).
pub fn negative_format(neg: u8) -> (u8, u8, u8) {
    const T: [(u8, u8, u8); 16] = [
        (1, 0, 0),
        (1, 0, 1),
        (1, 0, 4),
        (1, 0, 2),
        (0, 0, 0),
        (0, 0, 1),
        (0, 0, 3),
        (0, 0, 2),
        (0, 1, 1),
        (1, 1, 1),
        (0, 1, 2),
        (1, 1, 2),
        (1, 1, 4),
        (0, 1, 3),
        (1, 1, 0),
        (0, 1, 0),
    ];
    T[usize::from(neg & 15)]
}

/// `(cs_precedes, sep_by_space)` for `LOCALE_ICURRENCY` (0 `$1.1`, 1
/// `1.1$`, 2 `$ 1.1`, 3 `1.1 $`).
pub fn positive_format(pos: u8) -> (u8, u8) {
    match pos & 3 {
        0 => (1, 0),
        1 => (0, 0),
        2 => (1, 1),
        _ => (0, 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_consistent() {
        for (i, l) in LOCALES.iter().enumerate() {
            assert!(LOCALES[i + 1..].iter().all(|m| m.name != l.name && m.lcid != l.lcid), "{} twice", l.name);
            assert!(crate::codepage::CodePage::get(u32::from(l.ansi)).is_some(), "{}: ANSI {}", l.name, l.ansi);
            assert!(crate::codepage::CodePage::get(u32::from(l.oem)).is_some(), "{}: OEM {}", l.name, l.oem);
            assert!(l.decimal.chars().count() == 1 && l.thousands.chars().count() == 1, "{}", l.name);
            assert!(l.pos_currency < 4 && l.neg_currency < 16 && l.intl_symbol.len() == 3, "{}", l.name);
        }
        for (_, name) in LANGUAGE_ALIASES {
            assert!(by_name(name).is_some(), "{name}");
        }
    }

    #[test]
    fn lookups() {
        assert_eq!(by_name("de-CH").unwrap().lcid, 0x0807);
        assert_eq!(by_name("DE-ch").unwrap().name, "de-CH");
        assert_eq!(by_name("en_US").unwrap().name, "en-US");
        assert!(by_name("xx-YY").is_none() && by_name("").is_none());
        assert_eq!(by_language("German").unwrap().name, "de-DE");
        assert_eq!(by_language("deu").unwrap().name, "de-DE");
        assert_eq!(by_language("DES").unwrap().name, "de-CH");
        assert_eq!(by_language("swiss").unwrap().name, "de-CH");
        assert_eq!(by_language("american english").unwrap().name, "en-US");
        assert_eq!(by_language("Chinese").unwrap().name, "zh-CN");
        assert_eq!(by_language("klingon").map(|l| l.name), None);
        assert_eq!(by_language_country("German", "Switzerland").unwrap().name, "de-CH");
        assert_eq!(by_language_country("english", "uk").unwrap().name, "en-GB");
        assert_eq!(by_language_country("English", "United States").unwrap().name, "en-US");
        assert_eq!(by_language_country("french", "CHE").unwrap().name, "fr-CH");
        assert_eq!(by_language_country("german-swiss", "Austria").unwrap().name, "de-AT");
        assert!(by_language_country("German", "Japan").is_none());
        assert_eq!(by_lcid(0x0411).unwrap().ansi, 932);
        assert_eq!(user_default().name, "en-US");
    }

    #[test]
    fn currency_formats() {
        assert_eq!(negative_format(0), (1, 0, 0));
        assert_eq!(negative_format(8), (0, 1, 1));
        assert_eq!(negative_format(12), (1, 1, 4));
        assert_eq!(positive_format(3), (0, 1));
    }
}
