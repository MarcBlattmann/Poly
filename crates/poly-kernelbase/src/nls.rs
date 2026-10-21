//! National-language support basics: the code pages Poly serves and the
//! conversions between them and UTF-16 (`MultiByteToWideChar`,
//! `WideCharToMultiByte`), the code page queries (`GetACP`, `GetOEMCP`,
//! `GetCPInfo`, `IsValidCodePage`), and ordinal string comparison
//! (`CompareStringOrdinal`, `lstrcmpW`, `lstrcmpiW`).
//!
//! Sources (clean room): Learn's reference pages of those functions, "Code
//! Page Identifiers", "Unicode in the Windows API"; `winnls.h` and
//! `stringapiset.h` for the constants; the code page tables are the
//! public charts (Unicode Consortium mapping files `CP1252.TXT` and
//! `CP437.TXT`, Microsoft's code page charts).
//!
//! **Code pages.** Poly is an English (United States) system: the ANSI
//! code page is 1252 ([`poly_rtl::codepage`]) and the OEM code page 437
//! ("OEM United States", Learn's "Code Page Identifiers"). `CP_ACP` and
//! `CP_THREAD_ACP` name 1252, `CP_OEMCP` 437, and 65001 is UTF-8. Every
//! other code page (`CP_MACCP`, `CP_UTF7`, the double-byte ones) is
//! `ERROR_INVALID_PARAMETER` (no table yet, ADR 0079).
//!
//! **Choices Learn leaves open** (to be confirmed by the spec agent):
//! - Single-byte code pages use no best-fit mapping: a character the code
//!   page lacks becomes the default character (`?`), as with
//!   `WC_NO_BEST_FIT_CHARS`. A surrogate pair is one character, so one
//!   default character.
//! - Bytes 1252 leaves undefined (0x81, 0x8D, 0x8F, 0x90, 0x9D) convert to
//!   the C1 control of the same value and are not "invalid" for
//!   `MB_ERR_INVALID_CHARS`.
//! - Invalid UTF-8 (overlong forms, surrogates, values past U+10FFFF, a
//!   truncated sequence) becomes one U+FFFD per maximal invalid
//!   subpart without `MB_ERR_INVALID_CHARS` (Learn: "Starting with Windows
//!   Vista ... replaced with U+FFFD"); a lone surrogate in UTF-16 becomes
//!   U+FFFD (`EF BF BD`) without `WC_ERR_INVALID_CHARS`.

use crate::error::*;
use alloc::vec::Vec;
use poly_rtl::codepage;

/// `CP_ACP`, `CP_OEMCP`, `CP_MACCP`, `CP_THREAD_ACP`, `CP_UTF7`,
/// `CP_UTF8` (`winnls.h`).
pub const CP_ACP: u32 = 0;
pub const CP_OEMCP: u32 = 1;
pub const CP_MACCP: u32 = 2;
pub const CP_THREAD_ACP: u32 = 3;
pub const CP_UTF7: u32 = 65000;
pub const CP_UTF8: u32 = 65001;

/// The ANSI and OEM code pages Poly reports (`GetACP`, `GetOEMCP`).
pub const ANSI_CODE_PAGE: u32 = codepage::ANSI_CODE_PAGE;
pub const OEM_CODE_PAGE: u32 = 437;

/// `MultiByteToWideChar` flags (`winnls.h`).
pub const MB_PRECOMPOSED: u32 = 0x1;
pub const MB_COMPOSITE: u32 = 0x2;
pub const MB_USEGLYPHCHARS: u32 = 0x4;
pub const MB_ERR_INVALID_CHARS: u32 = 0x8;
/// `WideCharToMultiByte` flags (`winnls.h`).
pub const WC_DISCARDNS: u32 = 0x10;
pub const WC_SEPCHARS: u32 = 0x20;
pub const WC_DEFAULTCHAR: u32 = 0x40;
pub const WC_ERR_INVALID_CHARS: u32 = 0x80;
pub const WC_COMPOSITECHECK: u32 = 0x200;
pub const WC_NO_BEST_FIT_CHARS: u32 = 0x400;

/// `ERROR_NO_UNICODE_TRANSLATION` (`winerror.h`).
pub const ERROR_NO_UNICODE_TRANSLATION: u32 = 1113;

/// `CSTR_LESS_THAN`, `CSTR_EQUAL`, `CSTR_GREATER_THAN` (`winnls.h`).
pub const CSTR_LESS_THAN: i32 = 1;
pub const CSTR_EQUAL: i32 = 2;
pub const CSTR_GREATER_THAN: i32 = 3;

/// A code page Poly can convert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodePage {
    /// Windows-1252.
    Ansi1252,
    /// OEM United States.
    Oem437,
    Utf8,
}

impl CodePage {
    /// The code page an identifier names (`CP_ACP` and friends resolved),
    /// or `ERROR_INVALID_PARAMETER`.
    pub fn of(id: u32) -> Result<CodePage, u32> {
        match id {
            CP_ACP | CP_THREAD_ACP | 1252 => Ok(CodePage::Ansi1252),
            CP_OEMCP | 437 => Ok(CodePage::Oem437),
            CP_UTF8 => Ok(CodePage::Utf8),
            _ => Err(ERROR_INVALID_PARAMETER),
        }
    }

    /// The identifier (`CPINFOEX.CodePage`).
    pub fn id(self) -> u32 {
        match self {
            CodePage::Ansi1252 => 1252,
            CodePage::Oem437 => 437,
            CodePage::Utf8 => CP_UTF8,
        }
    }

    /// `CPINFO.MaxCharSize`.
    pub fn max_char_size(self) -> u32 {
        if self == CodePage::Utf8 { 4 } else { 1 }
    }
}

/// `IsValidCodePage`.
pub fn is_valid_code_page(id: u32) -> bool {
    !matches!(id, CP_ACP | CP_OEMCP | CP_MACCP | CP_THREAD_ACP) && CodePage::of(id).is_ok()
}

/// `CPINFO` (`winnls.h`): `MaxCharSize` (UINT), `DefaultChar[MAX_DEFAULTCHAR
/// = 2]`, `LeadByte[MAX_LEADBYTES = 12]`: 18 bytes, 20 with the padding
/// of its `UINT` alignment.
pub const CPINFO_SIZE: usize = 20;

/// The `CPINFO` of a code page. None of Poly's code pages has lead bytes;
/// the default character is `?`.
pub fn cp_info(cp: CodePage) -> [u8; CPINFO_SIZE] {
    let mut b = [0u8; CPINFO_SIZE];
    b[0..4].copy_from_slice(&cp.max_char_size().to_le_bytes());
    b[4] = codepage::DEFAULT_CHAR;
    b
}

/// Code points of code page 437's bytes 0x80-0xFF (bytes below are ASCII).
const CP437_HIGH: [u16; 128] = [
    0x00C7, 0x00FC, 0x00E9, 0x00E2, 0x00E4, 0x00E0, 0x00E5, 0x00E7, // 0x80
    0x00EA, 0x00EB, 0x00E8, 0x00EF, 0x00EE, 0x00EC, 0x00C4, 0x00C5, // 0x88
    0x00C9, 0x00E6, 0x00C6, 0x00F4, 0x00F6, 0x00F2, 0x00FB, 0x00F9, // 0x90
    0x00FF, 0x00D6, 0x00DC, 0x00A2, 0x00A3, 0x00A5, 0x20A7, 0x0192, // 0x98
    0x00E1, 0x00ED, 0x00F3, 0x00FA, 0x00F1, 0x00D1, 0x00AA, 0x00BA, // 0xA0
    0x00BF, 0x2310, 0x00AC, 0x00BD, 0x00BC, 0x00A1, 0x00AB, 0x00BB, // 0xA8
    0x2591, 0x2592, 0x2593, 0x2502, 0x2524, 0x2561, 0x2562, 0x2556, // 0xB0
    0x2555, 0x2563, 0x2551, 0x2557, 0x255D, 0x255C, 0x255B, 0x2510, // 0xB8
    0x2514, 0x2534, 0x252C, 0x251C, 0x2500, 0x253C, 0x255E, 0x255F, // 0xC0
    0x255A, 0x2554, 0x2569, 0x2566, 0x2560, 0x2550, 0x256C, 0x2567, // 0xC8
    0x2568, 0x2564, 0x2565, 0x2559, 0x2558, 0x2552, 0x2553, 0x256B, // 0xD0
    0x256A, 0x2518, 0x250C, 0x2588, 0x2584, 0x258C, 0x2590, 0x2580, // 0xD8
    0x03B1, 0x00DF, 0x0393, 0x03C0, 0x03A3, 0x03C3, 0x00B5, 0x03C4, // 0xE0
    0x03A6, 0x0398, 0x03A9, 0x03B4, 0x221E, 0x03C6, 0x03B5, 0x2229, // 0xE8
    0x2261, 0x00B1, 0x2265, 0x2264, 0x2320, 0x2321, 0x00F7, 0x2248, // 0xF0
    0x00B0, 0x2219, 0x00B7, 0x221A, 0x207F, 0x00B2, 0x25A0, 0x00A0, // 0xF8
];

/// One byte of a single-byte code page to UTF-16.
fn sbcs_to_unicode(cp: CodePage, b: u8) -> u16 {
    match cp {
        CodePage::Oem437 if b >= 0x80 => CP437_HIGH[usize::from(b - 0x80)],
        CodePage::Oem437 => u16::from(b),
        _ => codepage::to_unicode(b),
    }
}

/// One UTF-16 unit to a byte of a single-byte code page, if it has one.
fn sbcs_from_unicode(cp: CodePage, c: u16) -> Option<u8> {
    match cp {
        CodePage::Oem437 if c < 0x80 => Some(c as u8),
        CodePage::Oem437 => CP437_HIGH.iter().position(|&h| h == c).map(|i| 0x80 + i as u8),
        _ => {
            let b = codepage::from_unicode(c);
            (b != codepage::DEFAULT_CHAR || c == u16::from(codepage::DEFAULT_CHAR)).then_some(b)
        }
    }
}

/// UTF-8 to UTF-16. `Err(())` is invalid input when `strict`.
fn utf8_to_utf16(bytes: &[u8], strict: bool) -> Result<Vec<u16>, ()> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x80 {
            out.push(u16::from(b));
            i += 1;
            continue;
        }
        // The length and the valid range of the second byte (Unicode's
        // table of well-formed UTF-8 byte sequences).
        let (len, lo, hi) = match b {
            0xC2..=0xDF => (2, 0x80, 0xBF),
            0xE0 => (3, 0xA0, 0xBF),
            0xE1..=0xEC | 0xEE..=0xEF => (3, 0x80, 0xBF),
            0xED => (3, 0x80, 0x9F),
            0xF0 => (4, 0x90, 0xBF),
            0xF1..=0xF3 => (4, 0x80, 0xBF),
            0xF4 => (4, 0x80, 0x8F),
            _ => (0, 0, 0),
        };
        let mut good = 0;
        if len > 0 {
            good = 1;
            while good < len && i + good < bytes.len() {
                let c = bytes[i + good];
                let ok = if good == 1 { (lo..=hi).contains(&c) } else { (0x80..=0xBF).contains(&c) };
                if !ok {
                    break;
                }
                good += 1;
            }
        }
        if len == 0 || good < len {
            if strict {
                return Err(());
            }
            out.push(0xFFFD);
            i += good.max(1);
            continue;
        }
        let mut cp = u32::from(b) & (0x7F >> len);
        for &c in &bytes[i + 1..i + len] {
            cp = (cp << 6) | u32::from(c & 0x3F);
        }
        push_code_point(&mut out, cp);
        i += len;
    }
    Ok(out)
}

fn push_code_point(out: &mut Vec<u16>, cp: u32) {
    if cp >= 0x1_0000 {
        let v = cp - 0x1_0000;
        out.push(0xD800 | (v >> 10) as u16);
        out.push(0xDC00 | (v & 0x3FF) as u16);
    } else {
        out.push(cp as u16);
    }
}

/// The characters (code points, `None` for a lone surrogate) of UTF-16.
fn chars(units: &[u16]) -> impl Iterator<Item = (Option<u32>, usize)> + '_ {
    let mut i = 0;
    core::iter::from_fn(move || {
        let c = *units.get(i)?;
        let next = units.get(i + 1).copied();
        if (0xD800..0xDC00).contains(&c)
            && let Some(n) = next.filter(|n| (0xDC00..0xE000).contains(n))
        {
            i += 2;
            return Some((Some(0x1_0000 + ((u32::from(c) - 0xD800) << 10) + (u32::from(n) - 0xDC00)), 2));
        }
        i += 1;
        if (0xD800..0xE000).contains(&c) { Some((None, 1)) } else { Some((Some(u32::from(c)), 1)) }
    })
}

/// `MultiByteToWideChar` on its input (the length already resolved: with
/// `cbMultiByte == -1` the caller includes the NUL). Returns the UTF-16
/// result or the Win32 error.
pub fn multi_byte_to_wide(cp: u32, flags: u32, bytes: &[u8]) -> Result<Vec<u16>, u32> {
    let cp = CodePage::of(cp)?;
    let strict = flags & MB_ERR_INVALID_CHARS != 0;
    match cp {
        CodePage::Utf8 => {
            // Learn: for UTF-8 the flags must be 0 or MB_ERR_INVALID_CHARS.
            if flags & !MB_ERR_INVALID_CHARS != 0 {
                return Err(ERROR_INVALID_FLAGS);
            }
            utf8_to_utf16(bytes, strict).map_err(|()| ERROR_NO_UNICODE_TRANSLATION)
        }
        _ => {
            const KNOWN: u32 = MB_PRECOMPOSED | MB_COMPOSITE | MB_USEGLYPHCHARS | MB_ERR_INVALID_CHARS;
            if flags & !KNOWN != 0 || flags & (MB_PRECOMPOSED | MB_COMPOSITE) == (MB_PRECOMPOSED | MB_COMPOSITE) {
                return Err(ERROR_INVALID_FLAGS);
            }
            Ok(bytes.iter().map(|&b| sbcs_to_unicode(cp, b)).collect())
        }
    }
}

/// What `WideCharToMultiByte` produced: the bytes and whether a default
/// character was used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Narrowed {
    pub bytes: Vec<u8>,
    pub used_default: bool,
}

/// `WideCharToMultiByte` on its input (length resolved as for
/// [`multi_byte_to_wide`]). `default_char` is `lpDefaultChar`'s first
/// byte, if given; `wants_used_default` whether `lpUsedDefaultChar` is not
/// null (Learn: both must be null for UTF-8, `ERROR_INVALID_PARAMETER`).
pub fn wide_to_multi_byte(
    cp: u32,
    flags: u32,
    units: &[u16],
    default_char: Option<u8>,
    wants_used_default: bool,
) -> Result<Narrowed, u32> {
    let cp = CodePage::of(cp)?;
    if cp == CodePage::Utf8 {
        if flags & !WC_ERR_INVALID_CHARS != 0 {
            return Err(ERROR_INVALID_FLAGS);
        }
        if default_char.is_some() || wants_used_default {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let mut out = Vec::with_capacity(units.len());
        for (c, _) in chars(units) {
            let cp = match c {
                Some(c) => c,
                None if flags & WC_ERR_INVALID_CHARS != 0 => return Err(ERROR_NO_UNICODE_TRANSLATION),
                None => 0xFFFD,
            };
            let mut buf = [0u8; 4];
            let s = char::from_u32(cp).unwrap_or('\u{FFFD}').encode_utf8(&mut buf);
            out.extend_from_slice(s.as_bytes());
        }
        return Ok(Narrowed { bytes: out, used_default: false });
    }
    const KNOWN: u32 =
        WC_COMPOSITECHECK | WC_DISCARDNS | WC_SEPCHARS | WC_DEFAULTCHAR | WC_NO_BEST_FIT_CHARS | WC_ERR_INVALID_CHARS;
    if flags & !KNOWN != 0 || flags & WC_ERR_INVALID_CHARS != 0 {
        return Err(ERROR_INVALID_FLAGS);
    }
    let default = default_char.unwrap_or(codepage::DEFAULT_CHAR);
    let mut out = Vec::with_capacity(units.len());
    let mut used_default = false;
    for (c, n) in chars(units) {
        let byte = if n == 1 { c.and_then(|c| sbcs_from_unicode(cp, c as u16)) } else { None };
        match byte {
            Some(b) => out.push(b),
            None => {
                out.push(default);
                used_default = true;
            }
        }
    }
    Ok(Narrowed { bytes: out, used_default })
}

/// The documented outcome of a conversion's output buffer: `capacity` 0
/// asks for the size; a result that does not fit is
/// `ERROR_INSUFFICIENT_BUFFER` (Learn: both functions).
pub fn fits(len: usize, capacity: i32) -> Result<bool, u32> {
    if capacity < 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if capacity == 0 {
        return Ok(false);
    }
    if len > capacity as usize { Err(ERROR_INSUFFICIENT_BUFFER) } else { Ok(true) }
}

/// `CompareStringOrdinal`: code unit by code unit; with `ignore_case`
/// both strings are first upcased with the operating system's table
/// (Learn: "uses the operating system uppercase table"; Poly's is the
/// object manager's, [`poly_ntobj::name::upcase`]).
pub fn compare_ordinal(a: &[u16], b: &[u16], ignore_case: bool) -> i32 {
    let fold = |c: u16| if ignore_case { poly_ntobj::name::upcase(c) } else { c };
    let ord = a.iter().map(|&c| fold(c)).cmp(b.iter().map(|&c| fold(c)));
    match ord {
        core::cmp::Ordering::Less => CSTR_LESS_THAN,
        core::cmp::Ordering::Equal => CSTR_EQUAL,
        core::cmp::Ordering::Greater => CSTR_GREATER_THAN,
    }
}

/// `lstrcmpW` / `lstrcmpiW`: Learn: negative, zero or positive like
/// `CompareString` minus 2. Poly compares ordinally (after upcasing for
/// `lstrcmpiW`), not with the user locale's linguistic sort (to be
/// confirmed by the spec agent; differs for punctuation and accents).
pub fn lstrcmp(a: &[u16], b: &[u16], ignore_case: bool) -> i32 {
    compare_ordinal(a, b, ignore_case) - CSTR_EQUAL
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wide;

    #[test]
    fn code_pages_resolve() {
        assert_eq!(CodePage::of(CP_ACP), Ok(CodePage::Ansi1252));
        assert_eq!(CodePage::of(CP_THREAD_ACP), Ok(CodePage::Ansi1252));
        assert_eq!(CodePage::of(CP_OEMCP), Ok(CodePage::Oem437));
        assert_eq!(CodePage::of(CP_UTF8), Ok(CodePage::Utf8));
        assert_eq!(CodePage::of(932), Err(ERROR_INVALID_PARAMETER));
        assert!(is_valid_code_page(1252) && is_valid_code_page(437) && is_valid_code_page(65001));
        assert!(!is_valid_code_page(CP_ACP) && !is_valid_code_page(12345));
        let info = cp_info(CodePage::Utf8);
        assert_eq!(u32::from_le_bytes(info[0..4].try_into().unwrap()), 4);
        assert_eq!(&info[4..6], b"?\0");
        assert!(info[6..].iter().all(|&b| b == 0));
        assert_eq!(cp_info(CodePage::Oem437)[0], 1);
    }

    #[test]
    fn cp437_is_a_bijection_on_its_bytes() {
        let mut seen = alloc::collections::BTreeSet::new();
        for b in 0..=255u8 {
            let c = sbcs_to_unicode(CodePage::Oem437, b);
            assert!(seen.insert(c), "{b:#x} maps to a code point already used");
            assert_eq!(sbcs_from_unicode(CodePage::Oem437, c), Some(b));
        }
        assert_eq!(sbcs_to_unicode(CodePage::Oem437, 0x82), 0xE9); // é
        assert_eq!(sbcs_to_unicode(CodePage::Oem437, 0xC4), 0x2500); // ─
        assert_eq!(sbcs_from_unicode(CodePage::Oem437, 0x20AC), None); // €
    }

    #[test]
    fn utf8_round_trips_and_replaces_invalid_input() {
        let s = "a\u{E9}\u{20AC}\u{1F600}";
        let w = multi_byte_to_wide(CP_UTF8, 0, s.as_bytes()).unwrap();
        assert_eq!(w, s.encode_utf16().collect::<Vec<_>>());
        let n = wide_to_multi_byte(CP_UTF8, 0, &w, None, false).unwrap();
        assert_eq!(n.bytes, s.as_bytes());
        // Invalid sequences: one U+FFFD per maximal invalid subpart.
        let bad = [b'x', 0xC0, 0x80, 0xE2, 0x82, b'y', 0xED, 0xA0, 0x80, 0xF4, 0x90];
        let w = multi_byte_to_wide(CP_UTF8, 0, &bad).unwrap();
        assert_eq!(w, [0x78, 0xFFFD, 0xFFFD, 0xFFFD, 0x79, 0xFFFD, 0xFFFD, 0xFFFD, 0xFFFD, 0xFFFD]);
        assert_eq!(multi_byte_to_wide(CP_UTF8, MB_ERR_INVALID_CHARS, &bad), Err(ERROR_NO_UNICODE_TRANSLATION));
        assert_eq!(multi_byte_to_wide(CP_UTF8, MB_PRECOMPOSED, b"a"), Err(ERROR_INVALID_FLAGS));
        // A lone surrogate.
        let n = wide_to_multi_byte(CP_UTF8, 0, &[0x41, 0xD800, 0x42], None, false).unwrap();
        assert_eq!(n.bytes, b"A\xEF\xBF\xBDB");
        assert_eq!(
            wide_to_multi_byte(CP_UTF8, WC_ERR_INVALID_CHARS, &[0xDC00], None, false),
            Err(ERROR_NO_UNICODE_TRANSLATION)
        );
        assert_eq!(wide_to_multi_byte(CP_UTF8, 0, &[0x41], Some(b'#'), false), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(wide_to_multi_byte(CP_UTF8, 0, &[0x41], None, true), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(wide_to_multi_byte(CP_UTF8, WC_NO_BEST_FIT_CHARS, &[0x41], None, false), Err(ERROR_INVALID_FLAGS));
    }

    #[test]
    fn single_byte_pages_use_the_default_character() {
        let w = multi_byte_to_wide(CP_ACP, 0, b"caf\xE9 \x80").unwrap();
        assert_eq!(w, [0x63, 0x61, 0x66, 0xE9, 0x20, 0x20AC]);
        let n = wide_to_multi_byte(CP_ACP, 0, &w, None, true).unwrap();
        assert_eq!(n, Narrowed { bytes: b"caf\xE9 \x80".to_vec(), used_default: false });
        let n = wide_to_multi_byte(1252, 0, &[0x41, 0x4E2D, 0xD83D, 0xDE00], None, true).unwrap();
        assert_eq!(n, Narrowed { bytes: b"A??".to_vec(), used_default: true });
        let n = wide_to_multi_byte(CP_OEMCP, 0, &[0xE9, 0x20AC], Some(b'*'), true).unwrap();
        assert_eq!(n, Narrowed { bytes: b"\x82*".to_vec(), used_default: true });
        assert_eq!(wide_to_multi_byte(CP_ACP, WC_ERR_INVALID_CHARS, &[0x41], None, false), Err(ERROR_INVALID_FLAGS));
        assert_eq!(multi_byte_to_wide(CP_ACP, MB_PRECOMPOSED | MB_COMPOSITE, b"a"), Err(ERROR_INVALID_FLAGS));
        assert_eq!(multi_byte_to_wide(CP_OEMCP, MB_PRECOMPOSED, b"\x82").unwrap(), [0xE9]);
        assert_eq!(multi_byte_to_wide(950, 0, b"a"), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn buffer_rules() {
        assert_eq!(fits(5, 0), Ok(false));
        assert_eq!(fits(5, 5), Ok(true));
        assert_eq!(fits(5, 4), Err(ERROR_INSUFFICIENT_BUFFER));
        assert_eq!(fits(1, -1), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn ordinal_comparison() {
        assert_eq!(compare_ordinal(&wide("abc"), &wide("abc"), false), CSTR_EQUAL);
        assert_eq!(compare_ordinal(&wide("ABC"), &wide("abc"), false), CSTR_LESS_THAN);
        assert_eq!(compare_ordinal(&wide("ABC"), &wide("abc"), true), CSTR_EQUAL);
        assert_eq!(compare_ordinal(&wide("ab"), &wide("abc"), true), CSTR_LESS_THAN);
        assert_eq!(compare_ordinal(&wide("\u{E9}"), &wide("\u{C9}"), true), CSTR_EQUAL);
        assert_eq!(compare_ordinal(&wide("b"), &wide("a"), false), CSTR_GREATER_THAN);
        assert_eq!(lstrcmp(&wide("Hello"), &wide("hello"), true), 0);
        assert!(lstrcmp(&wide("a"), &wide("b"), false) < 0);
    }
}
