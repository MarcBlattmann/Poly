//! The preferred UI languages (`winnls.h`; Learn: "GetThreadPreferredUILanguages
//! function", "User Interface Language Management"). Poly has one UI language, English (United
//! States), for the system and the user, and no thread or process list (nothing sets one), so
//! every list is built from that one language.
//!
//! Chrome asks for `MUI_LANGUAGE_NAME | MUI_UI_FALLBACK` when its main DLL starts and treats a
//! failure as fatal (ADR 0155).

use crate::error::{ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER};
use crate::locale;
use alloc::vec::Vec;

/// `MUI_LANGUAGE_ID` (`winnls.h`): languages as four hexadecimal digits.
pub const MUI_LANGUAGE_ID: u32 = 0x4;
/// `MUI_LANGUAGE_NAME` (`winnls.h`): languages as names (the default).
pub const MUI_LANGUAGE_NAME: u32 = 0x8;
/// `MUI_MERGE_SYSTEM_FALLBACK` (`winnls.h`): with the user fallback, the fallback and neutral
/// languages are in the list too.
pub const MUI_MERGE_SYSTEM_FALLBACK: u32 = 0x10;
/// `MUI_MERGE_USER_FALLBACK` (`winnls.h`): the thread's, the process's, the user's languages
/// and the system default.
pub const MUI_MERGE_USER_FALLBACK: u32 = 0x20;
/// `MUI_UI_FALLBACK` (`winnls.h`): both merge flags.
pub const MUI_UI_FALLBACK: u32 = MUI_MERGE_SYSTEM_FALLBACK | MUI_MERGE_USER_FALLBACK;
/// `MUI_THREAD_LANGUAGES` (`winnls.h`): only the thread's own list.
pub const MUI_THREAD_LANGUAGES: u32 = 0x40;

/// The languages `GetThreadPreferredUILanguages(flags)` lists, or the Win32 error for flags
/// Learn rules out: both forms at once, the system fallback without the user fallback, the
/// thread's own list together with a merge, or a flag the function does not have.
///
/// * `MUI_THREAD_LANGUAGES`: the thread's own list, which is empty.
/// * no merge flag or `MUI_MERGE_USER_FALLBACK`: the composite list, which is the user's one
///   language.
/// * `MUI_UI_FALLBACK`: that, and its neutral language behind it ("en-US", "en").
pub fn thread_preferred(flags: u32) -> Result<Vec<Vec<u16>>, u32> {
    let known = MUI_LANGUAGE_ID | MUI_LANGUAGE_NAME | MUI_UI_FALLBACK | MUI_THREAD_LANGUAGES;
    let merge = flags & MUI_UI_FALLBACK;
    if flags & !known != 0
        || flags & MUI_LANGUAGE_ID != 0 && flags & MUI_LANGUAGE_NAME != 0
        || merge == MUI_MERGE_SYSTEM_FALLBACK
        || flags & MUI_THREAD_LANGUAGES != 0 && merge != 0
    {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if flags & MUI_THREAD_LANGUAGES != 0 {
        return Ok(Vec::new());
    }
    let ids = flags & MUI_LANGUAGE_ID != 0;
    let langid = locale::LCID_EN_US as u16;
    let hex = |id: u16| alloc::format!("{id:04X}").encode_utf16().collect::<Vec<u16>>();
    let name: Vec<u16> = locale::NAME_EN_US.encode_utf16().collect();
    let mut list = alloc::vec![if ids { hex(langid) } else { name.clone() }];
    if merge == MUI_UI_FALLBACK {
        // The neutral language: the primary language id (`PRIMARYLANGID`, the low ten bits),
        // or the name up to its first hyphen.
        let neutral = name.split(|&c| c == u16::from(b'-')).next().unwrap_or(&name).to_vec();
        list.push(if ids { hex(langid & 0x3FF) } else { neutral });
    }
    Ok(list)
}

/// What a call writes: the count, the size in characters (the two NULs that end the list
/// included) and, when there is a buffer that holds it, the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub count: u32,
    pub size: u32,
    /// The languages, each ended by a NUL, and one more NUL: `Some` when it is to be copied.
    pub text: Option<Vec<u16>>,
    /// `None` for success (`TRUE`), else the last error (`FALSE`).
    pub error: Option<u32>,
}

/// The answer for `languages`, a caller's buffer (`has_buffer`) and its size `cch` in
/// characters (Learn): with no buffer and size 0 only the count and the size needed; a
/// buffer that is too small is `ERROR_INSUFFICIENT_BUFFER` with the size needed; no buffer
/// with a size is `ERROR_INVALID_PARAMETER`. An empty list is one NUL more than nothing: two
/// characters.
pub fn answer(languages: &[Vec<u16>], has_buffer: bool, cch: u32) -> Answer {
    let mut text: Vec<u16> = Vec::new();
    for l in languages {
        text.extend_from_slice(l);
        text.push(0);
    }
    if languages.is_empty() {
        text.push(0);
    }
    text.push(0);
    let (count, size) = (languages.len() as u32, text.len() as u32);
    match (has_buffer, cch) {
        (false, 0) => Answer { count, size, text: None, error: None },
        (false, _) => Answer { count: 0, size: 0, text: None, error: Some(ERROR_INVALID_PARAMETER) },
        (true, n) if n < size => Answer { count, size, text: None, error: Some(ERROR_INSUFFICIENT_BUFFER) },
        (true, _) => Answer { count, size, text: Some(text), error: None },
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::string::String;

    fn names(flags: u32) -> Result<Vec<String>, u32> {
        thread_preferred(flags).map(|l| l.iter().map(|w| String::from_utf16(w).unwrap()).collect())
    }

    #[test]
    fn the_list_chrome_asks_for_is_the_language_and_its_neutral() {
        assert_eq!(names(MUI_LANGUAGE_NAME | MUI_UI_FALLBACK).unwrap(), ["en-US", "en"]);
        assert_eq!(names(MUI_UI_FALLBACK).unwrap(), ["en-US", "en"]);
        assert_eq!(names(MUI_LANGUAGE_ID | MUI_UI_FALLBACK).unwrap(), ["0409", "0009"]);
    }

    #[test]
    fn without_the_system_fallback_it_is_the_users_language() {
        assert_eq!(names(0).unwrap(), ["en-US"]);
        assert_eq!(names(MUI_LANGUAGE_NAME).unwrap(), ["en-US"]);
        assert_eq!(names(MUI_MERGE_USER_FALLBACK).unwrap(), ["en-US"]);
        assert_eq!(names(MUI_LANGUAGE_ID).unwrap(), ["0409"]);
        // The thread has no list of its own.
        assert!(names(MUI_THREAD_LANGUAGES).unwrap().is_empty());
    }

    #[test]
    fn flag_combinations_learn_rules_out() {
        for flags in [
            MUI_LANGUAGE_ID | MUI_LANGUAGE_NAME,
            MUI_MERGE_SYSTEM_FALLBACK,
            MUI_THREAD_LANGUAGES | MUI_MERGE_USER_FALLBACK,
            MUI_THREAD_LANGUAGES | MUI_UI_FALLBACK,
            0x1,
            0x400,
        ] {
            assert_eq!(names(flags), Err(ERROR_INVALID_PARAMETER), "{flags:#x}");
        }
    }

    #[test]
    fn the_size_query_then_the_list() {
        let list = thread_preferred(MUI_LANGUAGE_NAME | MUI_UI_FALLBACK).unwrap();
        // "en-US\0en\0\0": 10 characters.
        assert_eq!(answer(&list, false, 0), Answer { count: 2, size: 10, text: None, error: None });
        let full = answer(&list, true, 10);
        assert_eq!((full.count, full.size, full.error), (2, 10, None));
        assert_eq!(String::from_utf16(&full.text.unwrap()).unwrap(), "en-US\0en\0\0");
        assert_eq!(answer(&list, true, 64).text.unwrap().len(), 10);
        // Too small: the size needed and ERROR_INSUFFICIENT_BUFFER.
        assert_eq!(
            answer(&list, true, 9),
            Answer { count: 2, size: 10, text: None, error: Some(ERROR_INSUFFICIENT_BUFFER) }
        );
        assert_eq!(answer(&list, false, 10).error, Some(ERROR_INVALID_PARAMETER));
        // An empty list is two NULs.
        assert_eq!(answer(&[], true, 2).text.unwrap(), [0, 0]);
        assert_eq!(answer(&[], false, 0).size, 2);
    }
}
