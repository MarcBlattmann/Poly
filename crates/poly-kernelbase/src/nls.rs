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
//! `CP_THREAD_ACP` name 1252, `CP_OEMCP` 437, and 65001 is UTF-8. The
//! other single-byte code pages and the double-byte ones (932, 936, 949,
//! 950) are `poly_nls::codepage`'s (ADR 0094); `CP_MACCP`, `CP_UTF7` and
//! code pages Poly has no table for are `ERROR_INVALID_PARAMETER`.
//!
//! **Choices Learn leaves open** (to be confirmed by the spec agent):
//! - No best-fit mapping: a character the code page lacks becomes the
//!   default character (`?`), as with `WC_NO_BEST_FIT_CHARS`. A surrogate
//!   pair is one character, so one default character.
//! - Bytes 1252 leaves undefined (0x81, 0x8D, 0x8F, 0x90, 0x9D) convert to
//!   the C1 control of the same value and are not "invalid" for
//!   `MB_ERR_INVALID_CHARS`; an unassigned byte of another code page is
//!   `?` (U+003F), invalid for `MB_ERR_INVALID_CHARS` (`poly_nls::codepage`
//!   has the double-byte rules).
//! - Invalid UTF-8 (overlong forms, surrogates, values past U+10FFFF, a
//!   truncated sequence) becomes one U+FFFD per maximal invalid
//!   subpart without `MB_ERR_INVALID_CHARS` (Learn: "Starting with Windows
//!   Vista ... replaced with U+FFFD"); a lone surrogate in UTF-16 becomes
//!   U+FFFD (`EF BF BD`) without `WC_ERR_INVALID_CHARS`.

use crate::error::*;
use alloc::vec::Vec;
pub use poly_nls::codepage::CodePage;
use poly_nls::codepage::{self as cp, DEFAULT_CHAR};
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

/// The code page an identifier names (`CP_ACP` and friends resolved),
/// or `ERROR_INVALID_PARAMETER`.
pub fn code_page(id: u32) -> Result<CodePage, u32> {
    let id = match id {
        CP_ACP | CP_THREAD_ACP => ANSI_CODE_PAGE,
        CP_OEMCP => OEM_CODE_PAGE,
        _ => id,
    };
    CodePage::get(id).ok_or(ERROR_INVALID_PARAMETER)
}

/// `IsValidCodePage`.
pub fn is_valid_code_page(id: u32) -> bool {
    !matches!(id, CP_ACP | CP_OEMCP | CP_MACCP | CP_THREAD_ACP) && CodePage::get(id).is_some()
}

/// `CPINFO` (`winnls.h`): `MaxCharSize` (UINT), `DefaultChar[MAX_DEFAULTCHAR
/// = 2]`, `LeadByte[MAX_LEADBYTES = 12]`: 18 bytes, 20 with the padding
/// of its `UINT` alignment.
pub const CPINFO_SIZE: usize = 20;

/// The `CPINFO` of a code page: the default character is `?`; the lead
/// byte ranges of a double-byte code page, ended by two zero bytes.
pub fn cp_info(cp: CodePage) -> [u8; CPINFO_SIZE] {
    let mut b = [0u8; CPINFO_SIZE];
    b[0..4].copy_from_slice(&cp.max_char_size().to_le_bytes());
    b[4] = DEFAULT_CHAR;
    for (i, &(lo, hi)) in cp.lead_ranges().iter().take(5).enumerate() {
        b[6 + 2 * i] = lo;
        b[7 + 2 * i] = hi;
    }
    b
}

/// `IsDBCSLeadByteEx`: `Err` for a code page Poly lacks.
pub fn is_lead_byte(id: u32, b: u8) -> Result<bool, u32> {
    code_page(id).map(|cp| cp.is_lead(b))
}

/// `MultiByteToWideChar` on its input (the length already resolved: with
/// `cbMultiByte == -1` the caller includes the NUL). Returns the UTF-16
/// result or the Win32 error.
pub fn multi_byte_to_wide(cp: u32, flags: u32, bytes: &[u8]) -> Result<Vec<u16>, u32> {
    let cp = code_page(cp)?;
    let strict = flags & MB_ERR_INVALID_CHARS != 0;
    if cp.is_utf8() {
        // Learn: for UTF-8 the flags must be 0 or MB_ERR_INVALID_CHARS.
        if flags & !MB_ERR_INVALID_CHARS != 0 {
            return Err(ERROR_INVALID_FLAGS);
        }
    } else {
        const KNOWN: u32 = MB_PRECOMPOSED | MB_COMPOSITE | MB_USEGLYPHCHARS | MB_ERR_INVALID_CHARS;
        if flags & !KNOWN != 0 || flags & (MB_PRECOMPOSED | MB_COMPOSITE) == (MB_PRECOMPOSED | MB_COMPOSITE) {
            return Err(ERROR_INVALID_FLAGS);
        }
    }
    cp.decode(bytes, strict).map_err(|()| ERROR_NO_UNICODE_TRANSLATION)
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
    let cp = code_page(cp)?;
    if cp.is_utf8() {
        if flags & !WC_ERR_INVALID_CHARS != 0 {
            return Err(ERROR_INVALID_FLAGS);
        }
        if default_char.is_some() || wants_used_default {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let bytes =
            cp::utf16_to_utf8(units, flags & WC_ERR_INVALID_CHARS != 0).map_err(|()| ERROR_NO_UNICODE_TRANSLATION)?;
        return Ok(Narrowed { bytes, used_default: false });
    }
    const KNOWN: u32 =
        WC_COMPOSITECHECK | WC_DISCARDNS | WC_SEPCHARS | WC_DEFAULTCHAR | WC_NO_BEST_FIT_CHARS | WC_ERR_INVALID_CHARS;
    if flags & !KNOWN != 0 || flags & WC_ERR_INVALID_CHARS != 0 {
        return Err(ERROR_INVALID_FLAGS);
    }
    let (bytes, used_default) = cp.encode(units, default_char.unwrap_or(codepage::DEFAULT_CHAR));
    Ok(Narrowed { bytes, used_default })
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
        assert_eq!(code_page(CP_ACP).map(CodePage::id), Ok(1252));
        assert_eq!(code_page(CP_THREAD_ACP).map(CodePage::id), Ok(1252));
        assert_eq!(code_page(CP_OEMCP).map(CodePage::id), Ok(437));
        assert_eq!(code_page(CP_UTF8).map(CodePage::id), Ok(CP_UTF8));
        assert_eq!(code_page(932).map(CodePage::id), Ok(932));
        assert_eq!(code_page(CP_UTF7), Err(ERROR_INVALID_PARAMETER));
        assert!(is_valid_code_page(1252) && is_valid_code_page(437) && is_valid_code_page(65001));
        assert!(is_valid_code_page(850) && is_valid_code_page(936));
        assert!(!is_valid_code_page(CP_ACP) && !is_valid_code_page(12345));
        let info = cp_info(CodePage::get(CP_UTF8).unwrap());
        assert_eq!(u32::from_le_bytes(info[0..4].try_into().unwrap()), 4);
        assert_eq!(&info[4..6], b"?\0");
        assert!(info[6..].iter().all(|&b| b == 0));
        assert_eq!(cp_info(CodePage::get(437).unwrap())[0], 1);
        let sjis = cp_info(CodePage::get(932).unwrap());
        assert_eq!(sjis[0], 2);
        assert_eq!(&sjis[6..12], &[0x81, 0x9F, 0xE0, 0xFC, 0, 0]);
        assert_eq!(is_lead_byte(932, 0x81), Ok(true));
        assert_eq!(is_lead_byte(CP_ACP, 0x81), Ok(false));
        assert_eq!(is_lead_byte(12345, 0x81), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn cp437_is_a_bijection_on_its_bytes() {
        let cp437 = CodePage::get(437).unwrap();
        let mut seen = alloc::collections::BTreeSet::new();
        for b in 0..=255u8 {
            let c = cp437.byte(b).unwrap();
            assert!(seen.insert(c), "{b:#x} maps to a code point already used");
            assert_eq!(cp437.encode_unit(c), Some(u16::from(b)));
        }
        assert_eq!(cp437.byte(0x82), Some(0xE9)); // é
        assert_eq!(cp437.byte(0xC4), Some(0x2500)); // ─
        assert_eq!(cp437.encode_unit(0x20AC), None); // €
    }

    #[test]
    fn double_byte_pages() {
        let w = multi_byte_to_wide(936, 0, b"\xC4\xE3\xBA\xC3").unwrap();
        assert_eq!(w, [0x4F60, 0x597D]);
        let n = wide_to_multi_byte(936, 0, &[0x4F60, 0x41, 0x0416], None, true).unwrap();
        assert_eq!(n, Narrowed { bytes: b"\xC4\xE3A\xA7\xA8".to_vec(), used_default: false });
        let n = wide_to_multi_byte(932, 0, &[0x05D0], Some(b'*'), true).unwrap();
        assert_eq!(n, Narrowed { bytes: b"*".to_vec(), used_default: true });
        assert_eq!(multi_byte_to_wide(932, MB_ERR_INVALID_CHARS, b"\x81"), Err(ERROR_NO_UNICODE_TRANSLATION));
        assert_eq!(multi_byte_to_wide(932, 0, b"\x81").unwrap(), [0x3F]);
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
        assert_eq!(multi_byte_to_wide(CP_UTF7, 0, b"a"), Err(ERROR_INVALID_PARAMETER));
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
