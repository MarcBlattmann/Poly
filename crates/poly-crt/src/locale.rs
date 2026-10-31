//! Locales of the C runtime (ADR 0094): what `setlocale`, `_wsetlocale`
//! and `_create_locale` accept and answer, and what a locale's categories
//! decide (`LC_CTYPE`'s tables, `LC_NUMERIC`'s and `LC_MONETARY`'s `lconv`).
//!
//! Sources (clean room): Learn's `setlocale`/`_wsetlocale` page (the
//! `locale` argument's forms, "UTF-8 support", the composite
//! `LC_COLLATE=…;LC_CTYPE=…` string), "Locale names, Languages, and
//! Country/Region strings", "Code Pages", `localeconv`, `_create_locale`,
//! `MB_CUR_MAX`; `locale.h` for `LC_*` and `struct lconv`; `ctype.h` for
//! `_LEADBYTE`; the locale table is `poly_nls::locale`'s.
//!
//! **Names.** A category's locale is one of:
//! - `"C"` (also `"POSIX"`, Poly's reading);
//! - `""`: the user's default, English (United States) with its ANSI code
//!   page: `English_United States.1252`;
//! - `.cp`: the user's language with code page `cp` — a number, `ACP`,
//!   `OCP`, or `utf8` / `utf-8` (any case);
//! - `language[_country[.cp]]` with Learn's language and country strings,
//!   answered in the long form `Language_Country.cp` with the English
//!   names (`German_Switzerland.850`);
//! - a locale name (`de-CH`, also `de_CH`), with an optional `.cp`,
//!   answered as the canonical locale name, plus `.cp` when the code page
//!   is not the locale's ANSI code page (`de-CH`, `de-CH.utf8`).
//!
//! The code page must be one Poly converts; UTF-8 only in `ucrtbase`
//! (Learn: UTF-8 locales are a UCRT feature). When every category has the
//! same locale `setlocale(LC_ALL, NULL)` answers its name, else
//! `LC_COLLATE=…;LC_CTYPE=…;LC_MONETARY=…;LC_NUMERIC=…;LC_TIME=…`, which
//! `setlocale(LC_ALL, …)` also accepts.
//!
//! What Windows answers for these names is to be confirmed by the spec
//! agent (ADR 0094 lists the guesses).

use crate::ctype;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use poly_nls::locale::{self as db, Locale};

/// `LC_ALL` … `LC_TIME` (`locale.h`).
pub const LC_ALL: i32 = 0;
pub const LC_COLLATE: i32 = 1;
pub const LC_CTYPE: i32 = 2;
pub const LC_MONETARY: i32 = 3;
pub const LC_NUMERIC: i32 = 4;
pub const LC_TIME: i32 = 5;
pub const LC_MAX: i32 = 5;
/// The categories' names, by number.
pub const CATEGORY_NAMES: [&str; 6] = ["LC_ALL", "LC_COLLATE", "LC_CTYPE", "LC_MONETARY", "LC_NUMERIC", "LC_TIME"];
/// `CP_UTF8`.
pub const UTF8: u32 = 65001;

/// How a category's locale was named, and so how it is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// `Language_Country.cp`.
    Long,
    /// A locale name (`de-CH`).
    Name,
}

/// One category's locale: `None` is "C".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setting {
    pub locale: Option<&'static Locale>,
    /// The category's code page (0 for "C").
    pub code_page: u32,
    pub form: Form,
}

impl Setting {
    /// The "C" locale.
    pub const C: Setting = Setting { locale: None, code_page: 0, form: Form::Long };

    pub fn is_c(&self) -> bool {
        self.locale.is_none()
    }

    /// The category's name as `setlocale` answers it.
    pub fn name(&self) -> String {
        let Some(l) = self.locale else { return String::from("C") };
        let cp = if self.code_page == UTF8 { String::from("utf8") } else { format!("{}", self.code_page) };
        match self.form {
            Form::Long => {
                let (lang, country) = db::long_name(l);
                format!("{lang}_{country}.{cp}")
            }
            Form::Name if self.code_page == u32::from(l.ansi) => String::from(l.name),
            Form::Name => format!("{}.{cp}", l.name),
        }
    }

    /// `LCID` (`lc_handle`): 0 for "C".
    pub fn lcid(&self) -> u32 {
        self.locale.map_or(0, |l| l.lcid)
    }
}

fn eq_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// A code page part (`ACP`, `OCP`, `utf8`, a number) for `locale`.
fn code_page_part(part: &str, locale: &Locale) -> Option<u32> {
    if eq_ci(part, "ACP") {
        return Some(u32::from(locale.ansi));
    }
    if eq_ci(part, "OCP") {
        return Some(u32::from(locale.oem));
    }
    if eq_ci(part, "utf8") || eq_ci(part, "utf-8") {
        return Some(UTF8);
    }
    if part.is_empty() || part.len() > 5 || !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

/// One category's locale from its name; `valid` says whether Poly
/// converts a code page; `utf8` whether UTF-8 locales are allowed.
pub fn parse(name: &str, valid: &dyn Fn(u32) -> bool, utf8: bool) -> Option<Setting> {
    if name == "C" || name == "POSIX" {
        return Some(Setting::C);
    }
    let user = db::user_default();
    let (locale, form, cp) = if name.is_empty() {
        (user, Form::Long, None)
    } else if let Some(cp) = name.strip_prefix('.') {
        (user, Form::Long, Some(cp))
    } else {
        let (base, cp) = match name.rsplit_once('.') {
            Some((b, c)) => (b, Some(c)),
            None => (name, None),
        };
        if let Some(l) = db::by_name(base) {
            (l, Form::Name, cp)
        } else if let Some((lang, country)) = base.split_once('_') {
            (db::by_language_country(lang, country)?, Form::Long, cp)
        } else {
            (db::by_language(base)?, Form::Long, cp)
        }
    };
    let code_page = match cp {
        None => u32::from(locale.ansi),
        Some(p) => code_page_part(p, locale)?,
    };
    if code_page == UTF8 && !utf8 || !valid(code_page) {
        return None;
    }
    Some(Setting { locale: Some(locale), code_page, form })
}

/// The five categories `LC_COLLATE` … `LC_TIME` (index 0 is `LC_COLLATE`).
pub type Settings = [Setting; 5];

/// `setlocale(category, name)` on `current`: the new settings, or `None`
/// (the name is not one Poly has; nothing changes).
pub fn apply(
    current: &Settings,
    category: i32,
    name: &str,
    valid: &dyn Fn(u32) -> bool,
    utf8: bool,
) -> Option<Settings> {
    let mut out = *current;
    if category == LC_ALL {
        if name.contains('=') {
            for part in name.split(';').filter(|p| !p.is_empty()) {
                let (cat, value) = part.split_once('=')?;
                let k = CATEGORY_NAMES[1..].iter().position(|c| *c == cat)?;
                out[k] = parse(value, valid, utf8)?;
            }
        } else {
            out = [parse(name, valid, utf8)?; 5];
        }
    } else {
        out[(category - 1) as usize] = parse(name, valid, utf8)?;
    }
    Some(out)
}

/// `setlocale(category, NULL)`'s answer.
pub fn name_of(settings: &Settings, category: i32) -> String {
    if category != LC_ALL {
        return settings[(category - 1) as usize].name();
    }
    if settings.iter().all(|s| s.name() == settings[0].name()) {
        return settings[0].name();
    }
    let parts: Vec<String> =
        settings.iter().enumerate().map(|(i, s)| format!("{}={}", CATEGORY_NAMES[i + 1], s.name())).collect();
    parts.join(";")
}

// ----- LC_CTYPE ------------------------------------------------------------------------

/// What `LC_CTYPE` decides.
#[derive(Clone, Debug)]
pub struct Ctype {
    /// `_pctype`'s table, entry 0 for `EOF`.
    pub table: [u16; 257],
    /// `tolower` / `toupper` of each byte.
    pub lower: [u8; 256],
    pub upper: [u8; 256],
    /// `MB_CUR_MAX`.
    pub mb_cur_max: i32,
}

impl Ctype {
    /// The "C" locale's.
    pub fn c() -> Ctype {
        let mut lower = [0u8; 256];
        let mut upper = [0u8; 256];
        for b in 0..=255u8 {
            lower[usize::from(b)] = ctype::to_lower(i32::from(b)) as u8;
            upper[usize::from(b)] = ctype::to_upper(i32::from(b)) as u8;
        }
        Ctype { table: ctype::NARROW_TABLE, lower, upper, mb_cur_max: 1 }
    }

    /// The tables of code page `cp` (not "C"): ASCII as in "C"; another
    /// byte classed by its character (`poly_nls::unicode`), a lead byte
    /// `_LEADBYTE` only; case through UTF-16 when the other case is a byte
    /// of the code page too. In UTF-8 the bytes above 0x7F have no class.
    pub fn build(cp: u32, leads: &[(u8, u8)], chars: &dyn crate::mbcs::Chars) -> Ctype {
        let mut t = Ctype::c();
        t.mb_cur_max = if cp == UTF8 {
            4
        } else if leads.is_empty() {
            1
        } else {
            2
        };
        if cp == UTF8 {
            return t;
        }
        for b in 0x80..=0xFFu8 {
            let i = usize::from(b);
            if leads.iter().any(|&(lo, hi)| (lo..=hi).contains(&b)) {
                t.table[i + 1] = ctype::LEADBYTE;
                continue;
            }
            let Some(w) = chars.to_wide(u32::from(b)) else { continue };
            t.table[i + 1] = poly_nls::unicode::class(w) & 0x1FF;
            let back = |o: u16| chars.char_of(o).filter(|&v| v < 0x100).map(|v| v as u8);
            if let Some(u) = back(poly_nls::unicode::to_upper(w)) {
                t.upper[i] = u;
            }
            if let Some(l) = back(poly_nls::unicode::to_lower(w)) {
                t.lower[i] = l;
            }
        }
        t
    }
}

// ----- LC_NUMERIC, LC_MONETARY ----------------------------------------------------------

/// `lconv`'s members as UTF-16 (the runtime converts them to the
/// category's code page) and its `char` members.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lconv {
    pub decimal_point: Vec<u16>,
    pub thousands_sep: Vec<u16>,
    pub grouping: Vec<u8>,
    pub int_curr_symbol: Vec<u16>,
    pub currency_symbol: Vec<u16>,
    pub mon_decimal_point: Vec<u16>,
    pub mon_thousands_sep: Vec<u16>,
    pub mon_grouping: Vec<u8>,
    pub positive_sign: Vec<u16>,
    pub negative_sign: Vec<u16>,
    /// `int_frac_digits`, `frac_digits`, `p_cs_precedes`,
    /// `p_sep_by_space`, `n_cs_precedes`, `n_sep_by_space`,
    /// `p_sign_posn`, `n_sign_posn` (`CHAR_MAX` = 127: not available).
    pub chars: [u8; 8],
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// The numeric members for `LC_NUMERIC`'s locale (C11 7.11.2.1's "C"
/// values for "C": `"."`, `""`, `""`).
pub fn numeric(l: Option<&Locale>, out: &mut Lconv) {
    match l {
        None => {
            out.decimal_point = w(".");
            out.thousands_sep = Vec::new();
            out.grouping = Vec::new();
        }
        Some(l) => {
            out.decimal_point = w(l.decimal);
            out.thousands_sep = w(l.thousands);
            out.grouping = l.grouping.to_vec();
        }
    }
}

/// The monetary members for `LC_MONETARY`'s locale. `int_curr_symbol` is
/// the ISO 4217 code (Poly's reading: without C's fourth, separating
/// character); `p_sign_posn` is 3 (`LOCALE_IPOSSIGNPOSN`'s usual value).
pub fn monetary(l: Option<&Locale>, out: &mut Lconv) {
    let Some(l) = l else {
        out.int_curr_symbol = Vec::new();
        out.currency_symbol = Vec::new();
        out.mon_decimal_point = Vec::new();
        out.mon_thousands_sep = Vec::new();
        out.mon_grouping = Vec::new();
        out.positive_sign = Vec::new();
        out.negative_sign = Vec::new();
        out.chars = [127; 8];
        return;
    };
    out.int_curr_symbol = w(l.intl_symbol);
    out.currency_symbol = w(l.currency);
    out.mon_decimal_point = w(l.decimal);
    out.mon_thousands_sep = w(l.thousands);
    out.mon_grouping = l.grouping.to_vec();
    out.positive_sign = Vec::new();
    out.negative_sign = w("-");
    let (pp, ps) = db::positive_format(l.pos_currency);
    let (np, ns, nsign) = db::negative_format(l.neg_currency);
    out.chars = [l.currency_digits, l.currency_digits, pp, ps, np, ns, 3, nsign];
}

/// The `lconv` of the settings' `LC_NUMERIC` and `LC_MONETARY`.
pub fn lconv(settings: &Settings) -> Lconv {
    let mut out = Lconv::default();
    numeric(settings[(LC_NUMERIC - 1) as usize].locale, &mut out);
    monetary(settings[(LC_MONETARY - 1) as usize].locale, &mut out);
    out
}

/// The bits of the `printf`/`scanf` option word that carry a decimal point
/// other than `.` (Poly's own, above the UCRT's `_CRT_INTERNAL_*` bits,
/// which a program's options never set: the runtime clears them first).
pub const POINT_SHIFT: u32 = 56;
pub const POINT_MASK: u64 = 0xFF << POINT_SHIFT;

/// The option bits for decimal point `point` (`.`: none).
pub fn point_option(point: u8) -> u64 {
    if point == b'.' { 0 } else { u64::from(point) << POINT_SHIFT }
}

/// The decimal point an option word carries.
pub fn point_of(options: u64) -> u8 {
    match (options >> POINT_SHIFT) as u8 {
        0 => b'.',
        p => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(cp: u32) -> bool {
        poly_nls::codepage::CodePage::get(cp).is_some()
    }

    fn name(s: &str) -> Option<String> {
        parse(s, &valid, true).map(|s| s.name())
    }

    #[test]
    fn names() {
        assert_eq!(name("C").as_deref(), Some("C"));
        assert_eq!(name("").as_deref(), Some("English_United States.1252"));
        assert_eq!(name(".ACP").as_deref(), Some("English_United States.1252"));
        assert_eq!(name(".OCP").as_deref(), Some("English_United States.437"));
        assert_eq!(name(".utf8").as_deref(), Some("English_United States.utf8"));
        assert_eq!(name(".UTF-8").as_deref(), Some("English_United States.utf8"));
        assert_eq!(name(".850").as_deref(), Some("English_United States.850"));
        assert_eq!(name("English_United States.1252").as_deref(), Some("English_United States.1252"));
        assert_eq!(name("German_Switzerland.850").as_deref(), Some("German_Switzerland.850"));
        assert_eq!(name("german").as_deref(), Some("German_Germany.1252"));
        assert_eq!(name("german-swiss").as_deref(), Some("German_Switzerland.1252"));
        assert_eq!(name("French_Canada.OCP").as_deref(), Some("French_Canada.850"));
        assert_eq!(name("japanese").as_deref(), Some("Japanese_Japan.932"));
        assert_eq!(name("chs").as_deref(), Some("Chinese (Simplified)_China.936"));
        assert_eq!(name("de-CH").as_deref(), Some("de-CH"));
        assert_eq!(name("de-ch").as_deref(), Some("de-CH"));
        assert_eq!(name("de-CH.utf8").as_deref(), Some("de-CH.utf8"));
        assert_eq!(name("en_US.UTF-8").as_deref(), Some("en-US.utf8"));
        assert_eq!(name("ja-JP").as_deref(), Some("ja-JP"));
        assert_eq!(name("pl-PL.1252").as_deref(), Some("pl-PL.1252"));
        assert_eq!(name("xx-YY"), None);
        assert_eq!(name("German_Japan"), None);
        assert_eq!(name("German.12345"), None);
        assert_eq!(name("German.abc"), None);
        assert_eq!(parse(".utf8", &valid, false), None);
        let s = parse("de-CH", &valid, true).unwrap();
        assert_eq!((s.code_page, s.lcid()), (1252, 0x0807));
    }

    #[test]
    fn categories() {
        let c = [Setting::C; 5];
        assert_eq!(name_of(&c, LC_ALL), "C");
        let s = apply(&c, LC_NUMERIC, "German_Germany.1252", &valid, true).unwrap();
        assert_eq!(name_of(&s, LC_NUMERIC), "German_Germany.1252");
        assert_eq!(
            name_of(&s, LC_ALL),
            "LC_COLLATE=C;LC_CTYPE=C;LC_MONETARY=C;LC_NUMERIC=German_Germany.1252;LC_TIME=C"
        );
        let back = apply(&c, LC_ALL, &name_of(&s, LC_ALL), &valid, true).unwrap();
        assert_eq!(back, s);
        let all = apply(&s, LC_ALL, "", &valid, true).unwrap();
        assert_eq!(name_of(&all, LC_ALL), "English_United States.1252");
        assert!(apply(&s, LC_ALL, "LC_CTYPE=xx", &valid, true).is_none());
        assert!(apply(&s, LC_ALL, "LC_FOO=C", &valid, true).is_none());
        let l = lconv(&s);
        assert_eq!(l.decimal_point, w(","));
        assert_eq!(l.thousands_sep, w("."));
        assert_eq!(l.grouping, [3]);
        assert_eq!(l.chars, [127; 8]);
        let m = lconv(&apply(&c, LC_ALL, "de-CH", &valid, true).unwrap());
        assert_eq!(m.currency_symbol, w("CHF"));
        assert_eq!(m.decimal_point, w("."));
        assert_eq!(m.chars, [2, 2, 1, 1, 1, 1, 3, 4]);
        assert_eq!(lconv(&c).decimal_point, w("."));
    }

    #[test]
    fn ctype_tables() {
        struct Cp(poly_nls::codepage::CodePage);
        impl crate::mbcs::Chars for Cp {
            fn to_wide(&self, c: u32) -> Option<u16> {
                self.0.byte(c as u8)
            }
            fn char_of(&self, w: u16) -> Option<u32> {
                self.0.encode_unit(w).map(u32::from)
            }
        }
        let cp = |id| Cp(poly_nls::codepage::CodePage::get(id).unwrap());
        let t = Ctype::build(1252, &[], &cp(1252));
        assert_eq!(t.mb_cur_max, 1);
        assert_eq!(t.table[0xE9 + 1], ctype::LOWER | 0x100);
        assert_eq!(t.upper[0xE9], 0xC9);
        assert_eq!(t.lower[0xC0], 0xE0);
        assert_eq!(t.table[usize::from(b'A') + 1], ctype::NARROW_TABLE[usize::from(b'A') + 1]);
        let t = Ctype::build(1251, &[], &cp(1251));
        assert_eq!(t.upper[0xE0], 0xC0); // а → А
        let (leads, _) = poly_nls::codepage::dbcs_ranges(932).unwrap();
        let t = Ctype::build(932, leads, &cp(932));
        assert_eq!((t.mb_cur_max, t.table[0x81 + 1]), (2, ctype::LEADBYTE));
        let t = Ctype::build(UTF8, &[], &cp(1252));
        assert_eq!((t.mb_cur_max, t.table[0xE9 + 1]), (4, 0));
        let c = Ctype::c();
        assert_eq!((c.upper[usize::from(b'a')], c.upper[0xE9], c.mb_cur_max), (b'A', 0xE9, 1));
    }

    #[test]
    fn decimal_point_option() {
        assert_eq!(point_option(b'.'), 0);
        assert_eq!(point_of(point_option(b',') | 0x24), b',');
        assert_eq!(point_of(0x24), b'.');
        let mut out: Vec<u8> = Vec::new();
        let args = [1.5f64.to_bits(), 0.25f64.to_bits(), 7];
        // SAFETY: the arguments match the format.
        unsafe {
            crate::printf::format(
                &b"%.2f %g %d"[..],
                &mut crate::printf::Slots(&args),
                &mut out,
                point_option(b','),
                false,
            )
        }
        .unwrap();
        assert_eq!(out, b"1,50 0,25 7");
        let p = crate::float::parse_point(&b"3,25"[..], crate::float::DOUBLE, b',');
        assert_eq!((f64::from_bits(p.bits), p.len), (3.25, 4));
        let p = crate::float::parse_point(&b"3.25"[..], crate::float::DOUBLE, b',');
        assert_eq!((f64::from_bits(p.bits), p.len), (3.0, 1));
    }
}
