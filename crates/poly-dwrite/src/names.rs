//! The `name` table and the locale-keyed strings DirectWrite hands out
//! (`IDWriteLocalizedStrings`). Source: the OpenType specification (`name`)
//! and Microsoft Learn ("Language Identifier Constants and Strings").

use alloc::string::String;
use alloc::vec::Vec;

use crate::sfnt::be16;

/// Windows LCIDs and their BCP 47 tags (lower case, as DirectWrite
/// reports them).
const LCIDS: &[(u16, &str)] = &[
    (0x0401, "ar-sa"),
    (0x0402, "bg-bg"),
    (0x0403, "ca-es"),
    (0x0404, "zh-tw"),
    (0x0405, "cs-cz"),
    (0x0406, "da-dk"),
    (0x0407, "de-de"),
    (0x0408, "el-gr"),
    (0x0409, "en-us"),
    (0x040A, "es-es"),
    (0x040B, "fi-fi"),
    (0x040C, "fr-fr"),
    (0x040D, "he-il"),
    (0x040E, "hu-hu"),
    (0x040F, "is-is"),
    (0x0410, "it-it"),
    (0x0411, "ja-jp"),
    (0x0412, "ko-kr"),
    (0x0413, "nl-nl"),
    (0x0414, "nb-no"),
    (0x0415, "pl-pl"),
    (0x0416, "pt-br"),
    (0x0418, "ro-ro"),
    (0x0419, "ru-ru"),
    (0x041A, "hr-hr"),
    (0x041B, "sk-sk"),
    (0x041C, "sq-al"),
    (0x041D, "sv-se"),
    (0x041E, "th-th"),
    (0x041F, "tr-tr"),
    (0x0421, "id-id"),
    (0x0422, "uk-ua"),
    (0x0423, "be-by"),
    (0x0424, "sl-si"),
    (0x0425, "et-ee"),
    (0x0426, "lv-lv"),
    (0x0427, "lt-lt"),
    (0x042A, "vi-vn"),
    (0x042D, "eu-es"),
    (0x0436, "af-za"),
    (0x0439, "hi-in"),
    (0x0804, "zh-cn"),
    (0x0807, "de-ch"),
    (0x0809, "en-gb"),
    (0x080A, "es-mx"),
    (0x080C, "fr-be"),
    (0x0810, "it-ch"),
    (0x0813, "nl-be"),
    (0x0814, "nn-no"),
    (0x0816, "pt-pt"),
    (0x0C04, "zh-hk"),
    (0x0C07, "de-at"),
    (0x0C09, "en-au"),
    (0x0C0A, "es-es"),
    (0x0C0C, "fr-ca"),
    (0x1004, "zh-sg"),
    (0x1007, "de-lu"),
    (0x1009, "en-ca"),
    (0x100C, "fr-ch"),
    (0x1409, "en-nz"),
    (0x1809, "en-ie"),
    (0x1C09, "en-za"),
    (0x4009, "en-in"),
];

/// The locale tag of a Windows language id.
pub fn lcid_tag(lcid: u16) -> Option<&'static str> {
    LCIDS.iter().find(|e| e.0 == lcid).map(|e| e.1)
}

/// One string of the `name` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameRecord {
    pub name_id: u16,
    pub locale: &'static str,
    pub text: Vec<u16>,
}

/// Every Windows (platform 3), Unicode (platform 0) and Mac Roman English
/// (platform 1, language 0, ASCII only) string of a `name` table.
pub fn parse_names(table: &[u8]) -> Vec<NameRecord> {
    let mut out = Vec::new();
    let (Some(count), Some(str_off)) = (be16(table, 2), be16(table, 4)) else { return out };
    for i in 0..usize::from(count) {
        let r = 6 + 12 * i;
        let (Some(plat), Some(enc), Some(lang), Some(id), Some(len), Some(off)) = (
            be16(table, r),
            be16(table, r + 2),
            be16(table, r + 4),
            be16(table, r + 6),
            be16(table, r + 8),
            be16(table, r + 10),
        ) else {
            break;
        };
        let start = usize::from(str_off) + usize::from(off);
        let Some(raw) = table.get(start..start + usize::from(len)) else { continue };
        let (locale, text): (&'static str, Vec<u16>) = match (plat, enc) {
            (3, 1) | (3, 10) | (0, _) => {
                let Some(tag) = (if plat == 0 { Some("en-us") } else { lcid_tag(lang) }) else { continue };
                (tag, raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect())
            }
            (1, 0) if lang == 0 && raw.iter().all(|&b| b < 0x80) => {
                ("en-us", raw.iter().map(|&b| u16::from(b)).collect())
            }
            _ => continue,
        };
        out.push(NameRecord { name_id: id, locale, text });
    }
    out
}

/// Strings keyed by locale name, sorted by locale name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Localized {
    pub entries: Vec<(String, Vec<u16>)>,
}

impl Localized {
    /// The strings of name `id`; the first record of a locale wins (a
    /// Windows record before a Mac one, as `parse_names` lists them).
    pub fn from_records(records: &[NameRecord], id: u16) -> Localized {
        let mut l = Localized::default();
        for r in records.iter().filter(|r| r.name_id == id) {
            l.insert(r.locale, &r.text);
        }
        l
    }

    /// A single en-us string.
    pub fn en_us(text: &str) -> Localized {
        let mut l = Localized::default();
        l.insert("en-us", &utf16(text));
        l
    }

    /// Add `text` for `locale` unless the locale has a string already.
    pub fn insert(&mut self, locale: &str, text: &[u16]) {
        if self.entries.iter().any(|e| e.0 == locale) {
            return;
        }
        let pos = self.entries.iter().position(|e| e.0.as_str() > locale).unwrap_or(self.entries.len());
        self.entries.insert(pos, (String::from(locale), text.to_vec()));
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The en-us string, else the first one.
    pub fn primary(&self) -> Option<&[u16]> {
        self.entries.iter().find(|e| e.0 == "en-us").or(self.entries.first()).map(|e| e.1.as_slice())
    }

    /// The entry of `locale`, compared ignoring case.
    pub fn find(&self, locale: &[u16]) -> Option<usize> {
        self.entries.iter().position(|e| eq_ignore_case(&e.0, locale))
    }
}

/// `true` when `locale` is a malformed locale name for `FindLocaleName`
/// (a trailing dash or space).
pub fn locale_malformed(locale: &[u16]) -> bool {
    matches!(locale.last(), Some(&c) if c == u16::from(b'-') || c == u16::from(b' '))
}

fn eq_ignore_case(a: &str, b: &[u16]) -> bool {
    let a = a.as_bytes();
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| u16::from(x.to_ascii_lowercase()) == lower(y))
}

/// ASCII lower case of a UTF-16 unit.
pub fn lower(c: u16) -> u16 {
    if (0x41..=0x5A).contains(&c) { c + 32 } else { c }
}

/// Compare two UTF-16 strings ignoring ASCII case.
pub fn eq_ci(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| lower(x) == lower(y))
}

/// Order two UTF-16 strings by code unit after ASCII lower casing.
pub fn cmp_ci(a: &[u16], b: &[u16]) -> core::cmp::Ordering {
    a.iter().map(|&c| lower(c)).cmp(b.iter().map(|&c| lower(c)))
}

/// UTF-16 of a Rust string.
pub fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// The string of UTF-16 units, lossily.
pub fn to_string(s: &[u16]) -> String {
    char::decode_utf16(s.iter().copied()).map(|r| r.unwrap_or('\u{FFFD}')).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name_table(recs: &[(u16, u16, u16, &str)]) -> Vec<u8> {
        let mut strings = Vec::new();
        let mut hdr = Vec::new();
        hdr.extend_from_slice(&0u16.to_be_bytes());
        hdr.extend_from_slice(&(recs.len() as u16).to_be_bytes());
        hdr.extend_from_slice(&((6 + 12 * recs.len()) as u16).to_be_bytes());
        for &(plat, lang, id, s) in recs {
            let bytes: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
            for v in [plat, if plat == 3 { 1 } else { 0 }, lang, id, bytes.len() as u16, strings.len() as u16] {
                hdr.extend_from_slice(&v.to_be_bytes());
            }
            strings.extend_from_slice(&bytes);
        }
        hdr.extend_from_slice(&strings);
        hdr
    }

    #[test]
    fn sorted_by_locale() {
        let t = name_table(&[(3, 0x409, 1, "Four"), (3, 0x407, 1, "Vier"), (3, 0xC09, 1, "Fourr"), (3, 0x40C, 1, "Quatre")]);
        let l = Localized::from_records(&parse_names(&t), 1);
        let tags: Vec<&str> = l.entries.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(tags, ["de-de", "en-au", "en-us", "fr-fr"]);
        assert_eq!(l.find(&utf16("EN-US")), Some(2));
        assert_eq!(l.find(&utf16("en")), None);
    }

    #[test]
    fn malformed_locales() {
        assert!(locale_malformed(&utf16("en-")));
        assert!(locale_malformed(&utf16("en-us ")));
        assert!(!locale_malformed(&utf16("")));
        assert!(!locale_malformed(&utf16("en_us")));
    }
}
