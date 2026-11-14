//! `IDWriteFactory::CreateNumberSubstitution`: which arguments are valid
//! (`specs/windows/dwrite.dll/IDWriteFactory.CreateNumberSubstitution.yaml`).
//!
//! The object only carries its arguments back to DirectWrite; Poly has no
//! text analyzer, so nothing here changes how digits are drawn.

/// `DWRITE_NUMBER_SUBSTITUTION_METHOD` (`dwrite.h`).
pub const FROM_CULTURE: u32 = 0;
/// The last defined method (`DWRITE_NUMBER_SUBSTITUTION_METHOD_TRADITIONAL`).
pub const LAST_METHOD: u32 = 4;

/// Why a call fails: only `E_INVALIDARG` is observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    Method,
    Locale,
}

/// A locale name `FROM_CULTURE` accepts: the sheet records that well formed
/// names that name no locale (`xx-YY`) and other spellings (`EN-US`,
/// `en_US`) are fine and that the empty string is too, while a sentence is
/// not. Poly takes a name to be well formed when it has only ASCII letters,
/// digits, `-` and `_` (Learn: locale names are such tags, "en-US",
/// "sr-Latn-RS"); that rule is a reading of the observations (see ADR 0181).
pub fn culture_name_ok(name: &[u16]) -> bool {
    name.iter().all(|&c| u8::try_from(c).is_ok_and(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
}

/// Check the arguments of `CreateNumberSubstitution`. `locale` is `None` for
/// a NULL pointer. The locale is looked at only for `FROM_CULTURE`, where a
/// NULL name is invalid too.
pub fn check(method: u32, locale: Option<&[u16]>) -> Result<(), Invalid> {
    if method > LAST_METHOD {
        return Err(Invalid::Method);
    }
    if method == FROM_CULTURE {
        match locale {
            None => return Err(Invalid::Locale),
            Some(n) if !culture_name_ok(n) => return Err(Invalid::Locale),
            Some(_) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn methods_above_four_are_invalid_whatever_the_locale() {
        for m in [5, 6, 0xFFFF_FFFF] {
            assert_eq!(check(m, Some(&w("en-us"))), Err(Invalid::Method));
            assert_eq!(check(m, None), Err(Invalid::Method));
        }
    }

    #[test]
    fn every_defined_method_takes_the_usual_locales() {
        for m in 0..=4 {
            for l in ["", "en-us", "de-CH", "ar-EG", "th-TH", "zh-CN", "xx-YY", "EN-US", "en_US"] {
                assert_eq!(check(m, Some(&w(l))), Ok(()), "method {m} locale {l:?}");
            }
        }
    }

    #[test]
    fn only_from_culture_looks_at_the_locale() {
        assert_eq!(check(2, None), Ok(()));
        assert_eq!(check(2, Some(&w("not a locale at all"))), Ok(()));
        assert_eq!(check(2, Some(&[b'a' as u16; 94])), Ok(()));
        assert_eq!(check(0, None), Err(Invalid::Locale));
        assert_eq!(check(0, Some(&w("not a locale at all"))), Err(Invalid::Locale));
        assert_eq!(check(0, Some(&w("a sentence"))), Err(Invalid::Locale));
    }
}
