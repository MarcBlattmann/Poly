//! The C threading error codes of `std::_Throw_Cpp_error` and the texts of the generic error
//! category (`_Throw_Cpp_error.yaml`).
//!
//! `_Throw_Cpp_error(code)` throws a `std::system_error` whose `error_code` is the `std::errc`
//! value of the code in the generic category and whose `what()` is the category's text twice,
//! `"<text>: <text>"`. The seven codes and their texts are the sheet's; the numbers of the
//! `std::errc` values are the Universal C runtime's `errno.h` (Windows SDK). `_Syserror_map`
//! (the generic category's message) gives the same texts: the sheet's seven, and for the other
//! `errno.h` values the `std::errc` enumerator name (C++ [system.error.syn]) with spaces for
//! underscores, which is the rule the sheet's seven follow.

/// The `std::errc` value (an `errno.h` number) of each `_Throw_Cpp_error` code
/// (`_DEVICE_OR_RESOURCE_BUSY` 0 .. `_RESOURCE_UNAVAILABLE_TRY_AGAIN` 6).
const CPP_ERRORS: [i32; 7] = [EBUSY, EINVAL, ESRCH, ENOMEM, EPERM, EDEADLK, EAGAIN];

// errno.h (Universal C runtime, Windows SDK).
const EPERM: i32 = 1;
const ESRCH: i32 = 3;
const EAGAIN: i32 = 11;
const ENOMEM: i32 = 12;
const EBUSY: i32 = 16;
const EINVAL: i32 = 22;
const EDEADLK: i32 = 36;

/// The `std::errc` value `_Throw_Cpp_error(code)` reports, or `None` for a code the sheet
/// does not list.
pub fn cpp_error_value(code: i32) -> Option<i32> {
    usize::try_from(code).ok().and_then(|i| CPP_ERRORS.get(i)).copied()
}

/// One text of the generic category: the `errno.h` value, the text, and the text twice as
/// `system_error::what()` has it (NUL-terminated, for the exception object).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Text {
    pub value: i32,
    pub text: &'static str,
    pub what: &'static str,
}

macro_rules! texts {
    ($($value:literal $text:literal),* $(,)?) => {
        &[$(Text { value: $value, text: concat!($text, "\0"), what: concat!($text, ": ", $text, "\0") }),*]
    };
}

/// The generic category's texts by `errno.h` value (NUL-terminated).
pub const TEXTS: &[Text] = texts![
    1 "operation not permitted",
    2 "no such file or directory",
    3 "no such process",
    4 "interrupted",
    5 "io error",
    6 "no such device or address",
    7 "argument list too long",
    8 "executable format error",
    9 "bad file descriptor",
    10 "no child process",
    11 "resource unavailable try again",
    12 "not enough memory",
    13 "permission denied",
    14 "bad address",
    16 "device or resource busy",
    17 "file exists",
    18 "cross device link",
    19 "no such device",
    20 "not a directory",
    21 "is a directory",
    22 "invalid argument",
    23 "too many files open in system",
    24 "too many files open",
    25 "inappropriate io control operation",
    27 "file too large",
    28 "no space on device",
    29 "invalid seek",
    30 "read only file system",
    31 "too many links",
    32 "broken pipe",
    33 "argument out of domain",
    34 "result out of range",
    36 "resource deadlock would occur",
    38 "filename too long",
    39 "no lock available",
    40 "function not supported",
    41 "directory not empty",
    42 "illegal byte sequence",
];

/// The text for a value with no entry (NUL-terminated).
pub const UNKNOWN: &str = "unknown error\0";

/// The generic category's text of an `errno.h` value.
pub fn text(value: i32) -> Option<&'static Text> {
    TEXTS.iter().find(|t| t.value == value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sheets_seven_codes() {
        let want = [
            (0, 16, "device or resource busy: device or resource busy"),
            (1, 22, "invalid argument: invalid argument"),
            (2, 3, "no such process: no such process"),
            (3, 12, "not enough memory: not enough memory"),
            (4, 1, "operation not permitted: operation not permitted"),
            (5, 36, "resource deadlock would occur: resource deadlock would occur"),
            (6, 11, "resource unavailable try again: resource unavailable try again"),
        ];
        for (code, value, what) in want {
            assert_eq!(cpp_error_value(code), Some(value), "code {code}");
            let t = text(value).unwrap();
            assert_eq!(t.what.strip_suffix('\0'), Some(what));
        }
        assert_eq!(cpp_error_value(7), None);
        assert_eq!(cpp_error_value(-1), None);
    }

    #[test]
    fn texts_are_terminated_and_unique() {
        for (i, t) in TEXTS.iter().enumerate() {
            assert!(t.text.ends_with('\0') && t.what.ends_with('\0'));
            assert_eq!(t.text.matches('\0').count(), 1);
            assert!(TEXTS[..i].iter().all(|u| u.value != t.value), "{} twice", t.value);
            assert!(!t.text.contains('_'));
        }
        assert!(text(15).is_none());
        assert!(UNKNOWN.ends_with('\0'));
    }
}
