//! `FormatMessageW`: the system message table and the insert/escape rules.
//!
//! The texts are those of Learn's "System Error Codes (0-499)",
//! "(500-999)" and "(1000-1299)" pages for the codes Poly's functions
//! report; a system message ends with CR LF. The rules are Learn's
//! `FormatMessage` page: `%1`–`%99` inserts with an optional `!printf
//! format!` (default `!s!`), the escapes `%0` (end, no new line), `%n`
//! (hard line break), `%r`, `%t`, `%%`, `%.`, `%!`, `% `, and the
//! maximum-width field (`FORMAT_MESSAGE_MAX_WIDTH_MASK`: ordinary line
//! breaks in the text become spaces, `%n` breaks stay).
//!
//! Choices to be confirmed by the spec agent (ADR 0076): a width other
//! than 0 or `MAX_WIDTH_MASK` does not wrap lines (treated as 0); with
//! `FORMAT_MESSAGE_IGNORE_INSERTS` the whole text is copied unchanged; a
//! language other than neutral or an English one is
//! `ERROR_RESOURCE_LANG_NOT_FOUND`.

use crate::error::*;
use alloc::vec::Vec;

pub const FORMAT_MESSAGE_ALLOCATE_BUFFER: u32 = 0x100;
pub const FORMAT_MESSAGE_IGNORE_INSERTS: u32 = 0x200;
pub const FORMAT_MESSAGE_FROM_STRING: u32 = 0x400;
pub const FORMAT_MESSAGE_FROM_HMODULE: u32 = 0x800;
pub const FORMAT_MESSAGE_FROM_SYSTEM: u32 = 0x1000;
pub const FORMAT_MESSAGE_ARGUMENT_ARRAY: u32 = 0x2000;
pub const FORMAT_MESSAGE_MAX_WIDTH_MASK: u32 = 0xFF;

/// Learn's system error texts for the codes Poly reports.
const SYSTEM: &[(u32, &str)] = &[
    (0, "The operation completed successfully."),
    (1, "Incorrect function."),
    (2, "The system cannot find the file specified."),
    (3, "The system cannot find the path specified."),
    (4, "The system cannot open the file."),
    (5, "Access is denied."),
    (6, "The handle is invalid."),
    (8, "Not enough memory resources are available to process this command."),
    (12, "The access code is invalid."),
    (14, "Not enough memory resources are available to complete this operation."),
    (17, "The system cannot move the file to a different disk drive."),
    (18, "There are no more files."),
    (19, "The media is write protected."),
    (21, "The device is not ready."),
    (31, "A device attached to the system is not functioning."),
    (32, "The process cannot access the file because it is being used by another process."),
    (33, "The process cannot access the file because another process has locked a portion of the file."),
    (38, "Reached the end of the file."),
    (50, "The request is not supported."),
    (80, "The file exists."),
    (87, "The parameter is incorrect."),
    (109, "The pipe has been ended."),
    (112, "There is not enough space on the disk."),
    (122, "The data area passed to a system call is too small."),
    (123, "The filename, directory name, or volume label syntax is incorrect."),
    (126, "The specified module could not be found."),
    (127, "The specified procedure could not be found."),
    (131, "An attempt was made to move the file pointer before the beginning of the file."),
    (145, "The directory is not empty."),
    (161, "The specified path is invalid."),
    (183, "Cannot create a file when that file already exists."),
    (193, "%1 is not a valid Win32 application."),
    (203, "The system could not find the environment option that was entered."),
    (206, "The filename or extension is too long."),
    (234, "More data is available."),
    (258, "The wait operation timed out."),
    (259, "No more data is available."),
    (267, "The directory name is invalid."),
    (288, "Attempt to release mutex not owned by caller."),
    (298, "Too many posts were made to a semaphore."),
    (299, "Only part of a ReadProcessMemory or WriteProcessMemory request was completed."),
    (317, "The system cannot find message text for message number 0x%1 in the message file for %2."),
    (487, "Attempt to access invalid address."),
    (534, "Arithmetic result exceeded 32 bits."),
    (995, "The I/O operation has been aborted because of either a thread exit or an application request."),
    (997, "Overlapped I/O operation is in progress."),
    (998, "Invalid access to memory location."),
    (1004, "Invalid flags."),
    (1114, "A dynamic link library (DLL) initialization routine failed."),
    (1168, "Element not found."),
    (1460, "This operation returned because the timeout period expired."),
];

/// The system message for `id`, if Poly has it.
pub fn system_message(id: u32) -> Option<&'static str> {
    SYSTEM.iter().find(|&&(n, _)| n == id).map(|&(_, t)| t)
}

/// Whether `FormatMessageW` serves language `lang` (`LANGID` in the low 16
/// bits): 0 (neutral: the default order), the user/system defaults, or any
/// English sublanguage (primary language `LANG_ENGLISH` = 9).
pub fn language_ok(lang: u32) -> bool {
    let primary = lang & 0x3FF;
    lang == 0 || primary == 0 || primary == 0x09
}

/// Where inserts come from: argument `index` (1-based) as a raw 64-bit
/// slot, and a UTF-16 string read from an address (for `!s!`) or an ANSI
/// one (for `!S!` / `!hs!`).
pub trait Args {
    fn slot(&self, index: usize) -> Option<u64>;
    fn wide(&self, address: u64) -> Vec<u16>;
    fn ansi(&self, address: u64) -> Vec<u16>;
}

const PCT: u16 = b'%' as u16;
const CR: u16 = 13;
const LF: u16 = 10;
const BANG: u16 = b'!' as u16;

/// The text of a message: `template` with its inserts and escapes
/// processed. `args` is `None` when the caller passed none (an insert is
/// then `ERROR_INVALID_PARAMETER`).
pub fn format(template: &[u16], flags: u32, args: Option<&dyn Args>) -> Result<Vec<u16>, u32> {
    let max_width = flags & FORMAT_MESSAGE_MAX_WIDTH_MASK == FORMAT_MESSAGE_MAX_WIDTH_MASK;
    let ignore = flags & FORMAT_MESSAGE_IGNORE_INSERTS != 0;
    let mut out = Vec::new();
    let mut i = 0;
    while i < template.len() {
        let c = template[i];
        if max_width && (c == CR || c == LF) {
            // An ordinary line break: one space for a CR LF pair.
            if c == CR && template.get(i + 1) == Some(&LF) {
                i += 1;
            }
            if !out.last().is_some_and(|&l| l == u16::from(b' ')) {
                out.push(u16::from(b' '));
            }
            i += 1;
            continue;
        }
        if c != PCT || ignore {
            out.push(c);
            i += 1;
            continue;
        }
        let Some(&next) = template.get(i + 1) else {
            out.push(c);
            break;
        };
        match next {
            0x30 => return Ok(out), // %0
            0x31..=0x39 => {
                // %1..%99, optionally !format!.
                let mut j = i + 1;
                let mut index = 0usize;
                while j < template.len() && (0x30..=0x39).contains(&template[j]) && j < i + 3 {
                    index = index * 10 + usize::from(template[j] - 0x30);
                    j += 1;
                }
                let mut spec: Vec<u16> = crate::wide("s");
                if template.get(j) == Some(&BANG)
                    && let Some(end) = template[j + 1..].iter().position(|&x| x == BANG)
                {
                    spec = template[j + 1..j + 1 + end].to_vec();
                    j += end + 2;
                }
                let a = args.ok_or(ERROR_INVALID_PARAMETER)?;
                let v = a.slot(index).ok_or(ERROR_INVALID_PARAMETER)?;
                insert(&mut out, &spec, v, a)?;
                i = j;
                continue;
            }
            0x6E => {
                out.extend_from_slice(&[CR, LF]); // %n
            }
            0x72 => out.push(CR),     // %r
            0x74 => out.push(9),      // %t
            other => out.push(other), // %%, %., %!, %space, ...
        }
        i += 2;
    }
    Ok(out)
}

/// One insert: a printf-style conversion (`[-][0][width][.prec][h|l|ll|I64|I32|I]type`).
fn insert(out: &mut Vec<u16>, spec: &[u16], v: u64, a: &dyn Args) -> Result<(), u32> {
    let s: Vec<u8> = spec.iter().map(|&c| if c < 0x80 { c as u8 } else { b'?' }).collect();
    let mut k = 0;
    let (mut left, mut zero) = (false, false);
    while k < s.len() && (s[k] == b'-' || s[k] == b'0' || s[k] == b'+' || s[k] == b' ' || s[k] == b'#') {
        left |= s[k] == b'-';
        zero |= s[k] == b'0';
        k += 1;
    }
    let mut width = 0usize;
    while k < s.len() && s[k].is_ascii_digit() {
        width = width * 10 + usize::from(s[k] - b'0');
        k += 1;
    }
    let mut precision = None;
    if k < s.len() && s[k] == b'.' {
        k += 1;
        let mut p = 0usize;
        while k < s.len() && s[k].is_ascii_digit() {
            p = p * 10 + usize::from(s[k] - b'0');
            k += 1;
        }
        precision = Some(p);
    }
    let mut wide64 = false;
    let mut narrow = false;
    loop {
        match s.get(k..) {
            Some([b'I', b'6', b'4', ..]) => {
                wide64 = true;
                k += 3;
            }
            Some([b'I', b'3', b'2', ..]) => k += 3,
            Some([b'l', b'l', ..]) => {
                wide64 = true;
                k += 2;
            }
            Some([b'I', ..]) => {
                wide64 = true;
                k += 1;
            }
            Some([b'h', ..]) => {
                narrow = true;
                k += 1;
            }
            Some([b'l' | b'w', ..]) => k += 1,
            _ => break,
        }
    }
    let ty = *s.get(k).ok_or(ERROR_INVALID_PARAMETER)?;
    let body: Vec<u16> = match ty {
        b's' if narrow => a.ansi(v),
        b's' => a.wide(v),
        b'S' => a.ansi(v),
        b'c' => alloc::vec![v as u16],
        b'C' => alloc::vec![v as u8 as u16],
        b'd' | b'i' => {
            let n: i64 = if wide64 { v as i64 } else { i64::from(v as u32 as i32) };
            let mut t = alloc::string::String::new();
            let _ = core::fmt::write(&mut t, format_args!("{n}"));
            crate::wide(&t)
        }
        b'u' | b'x' | b'X' | b'o' => {
            let n: u64 = if wide64 { v } else { u64::from(v as u32) };
            let mut t = alloc::string::String::new();
            let _ = match ty {
                b'u' => core::fmt::write(&mut t, format_args!("{n}")),
                b'x' => core::fmt::write(&mut t, format_args!("{n:x}")),
                b'X' => core::fmt::write(&mut t, format_args!("{n:X}")),
                _ => core::fmt::write(&mut t, format_args!("{n:o}")),
            };
            crate::wide(&t)
        }
        b'p' => {
            let mut t = alloc::string::String::new();
            let _ = core::fmt::write(&mut t, format_args!("{v:016X}"));
            crate::wide(&t)
        }
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    let mut body = body;
    if matches!(ty, b's' | b'S')
        && let Some(p) = precision
    {
        body.truncate(p);
    }
    let pad = width.saturating_sub(body.len());
    let numeric = !matches!(ty, b's' | b'S' | b'c' | b'C');
    if left {
        out.extend_from_slice(&body);
        out.extend(core::iter::repeat_n(u16::from(b' '), pad));
    } else if zero && numeric {
        // Zeros go after a sign.
        let (sign, digits) =
            if body.first() == Some(&u16::from(b'-')) { (&body[..1], &body[1..]) } else { (&body[..0], &body[..]) };
        out.extend_from_slice(sign);
        out.extend(core::iter::repeat_n(u16::from(b'0'), pad));
        out.extend_from_slice(digits);
    } else {
        out.extend(core::iter::repeat_n(u16::from(b' '), pad));
        out.extend_from_slice(&body);
    }
    Ok(())
}

/// A system message as `FormatMessageW` produces it (the text and CR LF),
/// or the error the call fails with.
pub fn system(id: u32, lang: u32, flags: u32, args: Option<&dyn Args>) -> Result<Vec<u16>, u32> {
    if !language_ok(lang) {
        return Err(ERROR_RESOURCE_LANG_NOT_FOUND);
    }
    let text = system_message(id).ok_or(ERROR_MR_MID_NOT_FOUND)?;
    let mut t = crate::wide(text);
    t.extend_from_slice(&[CR, LF]);
    format(&t, flags, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use crate::wide as w;
    use std::string::String;
    use std::vec;

    struct Slots(Vec<u64>, Vec<Vec<u16>>);
    impl Args for Slots {
        fn slot(&self, index: usize) -> Option<u64> {
            self.0.get(index.checked_sub(1)?).copied()
        }
        fn wide(&self, address: u64) -> Vec<u16> {
            self.1[address as usize].clone()
        }
        fn ansi(&self, address: u64) -> Vec<u16> {
            self.1[address as usize].clone()
        }
    }

    fn f(t: &str, flags: u32, args: &Slots) -> String {
        String::from_utf16_lossy(&format(&w(t), flags, Some(args)).unwrap())
    }

    #[test]
    fn system_messages() {
        let s = |id| String::from_utf16_lossy(&system(id, 0, FORMAT_MESSAGE_IGNORE_INSERTS, None).unwrap());
        assert_eq!(s(2), "The system cannot find the file specified.\r\n");
        assert_eq!(s(5), "Access is denied.\r\n");
        assert_eq!(s(193), "%1 is not a valid Win32 application.\r\n");
        assert_eq!(system(0xDEAD, 0, 0, None), Err(ERROR_MR_MID_NOT_FOUND));
        assert_eq!(system(2, 0x0407, 0, None), Err(ERROR_RESOURCE_LANG_NOT_FOUND));
        assert!(system(2, 0x0409, 0, None).is_ok());
        assert!(system(2, 0x0400, 0, None).is_ok());
        // An insert without arguments.
        assert_eq!(system(193, 0, 0, None), Err(ERROR_INVALID_PARAMETER));
        let a = Slots(vec![0], vec![w("x.exe")]);
        assert_eq!(
            String::from_utf16_lossy(&system(193, 0, 0, Some(&a)).unwrap()),
            "x.exe is not a valid Win32 application.\r\n"
        );
    }

    #[test]
    fn inserts_and_formats() {
        let a = Slots(vec![0, 42, (-5i32) as u32 as u64, 0xBEEF, 1], vec![w("one"), w("two")]);
        assert_eq!(f("[%1] [%2!d!] [%3!d!] [%4!x!] [%4!08X!]", 0, &a), "[one] [42] [-5] [beef] [0000BEEF]");
        assert_eq!(f("%2!5u!|%2!-5u!|%3!05d!", 0, &a), "   42|42   |-0005");
        assert_eq!(f("%5!s!%1!.2s!", 0, &a), "twoon");
        assert_eq!(f("%1%2!u!", 0, &a), "one42");
        assert_eq!(format(&w("%12"), 0, Some(&a)), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn escapes() {
        let a = Slots(vec![], vec![]);
        assert_eq!(f("a%nb%rc%td%%e%.f%!g% h", 0, &a), "a\r\nb\rc\td%e.f!g h");
        assert_eq!(f("stop%0more", 0, &a), "stop");
        assert_eq!(f("keep %1 %n", FORMAT_MESSAGE_IGNORE_INSERTS, &a), "keep %1 %n");
        assert_eq!(f("one\r\ntwo%nthree\r\n", FORMAT_MESSAGE_MAX_WIDTH_MASK, &a), "one two\r\nthree ");
        assert_eq!(f("trailing%", 0, &a), "trailing%");
    }

    #[test]
    fn bad_inserts() {
        let a = Slots(vec![1], vec![]);
        assert_eq!(format(&w("%2"), 0, Some(&a)), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(format(&w("%1!q!"), 0, Some(&a)), Err(ERROR_INVALID_PARAMETER));
    }
}
