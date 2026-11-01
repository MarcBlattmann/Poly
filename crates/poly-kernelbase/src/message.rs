//! `FormatMessageW` / `FormatMessageA`: the system message table, module
//! message tables, and the insert/escape/line-width rules (ADR 0079, ADR
//! 0101).
//!
//! The system messages are those of Learn's "System Error Codes" pages
//! (0-499 through 12000-15999), English (United States), generated into
//! [`crate::sysmsg`] by `cargo xtask system-messages`; a system message
//! ends with CR LF. An `HRESULT` of `FACILITY_WIN32` (`HRESULT_FROM_WIN32`,
//! `0x8007xxxx`) has its Win32 code's message.
//!
//! The rules are Learn's `FormatMessage` page: `%1`–`%99` inserts with an
//! optional `!printf format!` (default `!s!`), `*` width and precision
//! taken from the arguments (with the page's numbering of the inserts that
//! follow, which differs between a `va_list` and an argument array), the
//! escapes `%0` (end, no new line), `%n` (hard line break), `%r`, `%t`,
//! and any other character after `%` copied without the `%` (`%%`, `%.`,
//! `%!`, `% `), and the low byte of the flags: 0 keeps the text's line
//! breaks, `FORMAT_MESSAGE_MAX_WIDTH_MASK` turns them into spaces, and any
//! other value also wraps the output at that many characters, never
//! inside a string delimited by white space; `%n` breaks always stay.
//!
//! A module's message table is the `RT_MESSAGETABLE` resource (Learn's
//! "Message Tables"; `MESSAGE_RESOURCE_DATA`, `MESSAGE_RESOURCE_BLOCK` and
//! `MESSAGE_RESOURCE_ENTRY` in `winnt.h`): blocks of consecutive IDs, each
//! entry a length, flags (0 ANSI, `MESSAGE_RESOURCE_UNICODE` = 1,
//! `MESSAGE_RESOURCE_UTF8` = 2) and NUL-padded text.
//!
//! Choices to be confirmed by the spec agent (ADR 0079, ADR 0101): with
//! `FORMAT_MESSAGE_IGNORE_INSERTS` the whole text is copied unchanged
//! (escapes too); a wrapped line breaks with CR LF in place of the space
//! before the word that does not fit; a specific language other than
//! English (United States), or `LANG_NEUTRAL` with `SUBLANG_DEFAULT` /
//! `SUBLANG_SYS_DEFAULT` (which Poly's defaults make English (United
//! States)), is `ERROR_RESOURCE_LANG_NOT_FOUND` for system messages;
//! Learn's paragraphs are single lines, so a message Windows splits over
//! several lines is one line here.

use crate::error::*;
use crate::sysmsg;
use alloc::vec::Vec;

pub const FORMAT_MESSAGE_ALLOCATE_BUFFER: u32 = 0x100;
pub const FORMAT_MESSAGE_IGNORE_INSERTS: u32 = 0x200;
pub const FORMAT_MESSAGE_FROM_STRING: u32 = 0x400;
pub const FORMAT_MESSAGE_FROM_HMODULE: u32 = 0x800;
pub const FORMAT_MESSAGE_FROM_SYSTEM: u32 = 0x1000;
pub const FORMAT_MESSAGE_ARGUMENT_ARRAY: u32 = 0x2000;
pub const FORMAT_MESSAGE_MAX_WIDTH_MASK: u32 = 0xFF;

/// `RT_MESSAGETABLE` (`winuser.h`); a message table is resource ID 1.
pub const RT_MESSAGETABLE: u32 = 11;
pub const MESSAGE_TABLE_NAME: u32 = 1;

/// English (United States): `MAKELANGID(LANG_ENGLISH, SUBLANG_ENGLISH_US)`,
/// the language of the system table and Poly's thread, user and system
/// default language.
pub const LANG_EN_US: u32 = 0x0409;

/// With `FORMAT_MESSAGE_ALLOCATE_BUFFER`, a message longer than this many
/// bytes fails with `ERROR_MORE_DATA` (Learn: 128K bytes).
pub const MAX_ALLOCATED_BYTES: usize = 128 * 1024;

/// `FACILITY_WIN32` `HRESULT`s: `HRESULT_FROM_WIN32(x)` is
/// `0x80070000 | (x & 0xFFFF)` (`winerror.h`).
const HRESULT_WIN32: u32 = 0x8007_0000;

/// The Win32 code a message ID stands for: an `HRESULT_FROM_WIN32` value's
/// low 16 bits, else the ID itself.
pub fn win32_code(id: u32) -> u32 {
    if id & 0xFFFF_0000 == HRESULT_WIN32 { id & 0xFFFF } else { id }
}

/// The system message for `id` (a Win32 code or an `HRESULT_FROM_WIN32`
/// value), without its CR LF.
pub fn system_message(id: u32) -> Option<&'static str> {
    let code = u16::try_from(win32_code(id)).ok()?;
    let i = sysmsg::CODES.binary_search(&code).ok()?;
    let start = if i == 0 { 0 } else { sysmsg::ENDS[i - 1] as usize };
    sysmsg::TEXT.get(start..sysmsg::ENDS[i] as usize)
}

/// A specific language the caller asked for, `None` for 0 (Learn's search
/// order: neutral, thread, user default, system default, English (United
/// States), then any language). `LANG_NEUTRAL` with `SUBLANG_DEFAULT` or
/// `SUBLANG_SYS_DEFAULT` names the user's or the system's default.
pub fn requested_language(lang: u32) -> Option<u32> {
    match lang {
        0 => None,
        0x0400 | 0x0800 => Some(LANG_EN_US),
        l => Some(l),
    }
}

/// Whether the system table (English (United States) only) serves `lang`.
pub fn language_ok(lang: u32) -> bool {
    requested_language(lang).is_none_or(|l| l == LANG_EN_US)
}

/// Where inserts come from: argument `index` (1-based) as a raw 64-bit
/// slot, and a UTF-16 string read from an address (for `!s!` in
/// `FormatMessageW`) or an ANSI one converted (for `!s!` in
/// `FormatMessageA`, `!S!`, `!hs!`).
pub trait Args {
    fn slot(&self, index: usize) -> Option<u64>;
    fn wide(&self, address: u64) -> Vec<u16>;
    fn ansi(&self, address: u64) -> Vec<u16>;
}

const PCT: u16 = b'%' as u16;
const CR: u16 = 13;
const LF: u16 = 10;
const SPACE: u16 = b' ' as u16;
const BANG: u16 = b'!' as u16;
const STAR: u16 = b'*' as u16;

fn digit(c: u16) -> Option<usize> {
    (0x30..=0x39).contains(&c).then(|| usize::from(c - 0x30))
}

/// An insert at `template[i]` (a `%` followed by 1-9): its number, its
/// format (without the `!`s, `None` for the default `s`) and where the
/// text after it starts.
fn parse_insert(template: &[u16], i: usize) -> (usize, Option<&[u16]>, usize) {
    let mut j = i + 1;
    let mut n = 0;
    while j < template.len() && j < i + 3 {
        let Some(d) = digit(template[j]) else { break };
        n = n * 10 + d;
        j += 1;
    }
    if template.get(j) == Some(&BANG)
        && let Some(end) = template[j + 1..].iter().position(|&x| x == BANG)
    {
        return (n, Some(&template[j + 1..j + 1 + end]), j + end + 2);
    }
    (n, None, j)
}

/// The numbers of the inserts whose format has a `*` (each counted once).
fn starred_inserts(template: &[u16]) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < template.len() {
        if template[i] != PCT {
            i += 1;
            continue;
        }
        match template.get(i + 1).copied() {
            Some(c) if (0x31..=0x39).contains(&c) => {
                let (n, spec, next) = parse_insert(template, i);
                if spec.is_some_and(|s| s.contains(&STAR)) && !out.contains(&n) {
                    out.push(n);
                }
                i = next;
            }
            _ => i += 2,
        }
    }
    out
}

/// The text of a message: `template` with its inserts and escapes
/// processed. `ansi` is true for `FormatMessageA` (a plain `s` or `c`
/// insert is then ANSI). `args` is `None` when the caller passed none (an
/// insert is then `ERROR_INVALID_PARAMETER`).
pub fn format(template: &[u16], flags: u32, ansi: bool, args: Option<&dyn Args>) -> Result<Vec<u16>, u32> {
    let width = flags & FORMAT_MESSAGE_MAX_WIDTH_MASK;
    let ignore = flags & FORMAT_MESSAGE_IGNORE_INSERTS != 0;
    let array = flags & FORMAT_MESSAGE_ARGUMENT_ARRAY != 0;
    // With a va_list, every earlier insert with a `*` shifts the arguments
    // of the later insert numbers by one (Learn: the next insert number is
    // n+1 after one asterisk, n+2 after two, while the asterisks take one
    // or two arguments of their own besides the value).
    let starred = if array || ignore { Vec::new() } else { starred_inserts(template) };
    let mut out = Vec::new();
    let mut i = 0;
    while i < template.len() {
        let c = template[i];
        if width != 0 && (c == CR || c == LF) {
            // A regular line break: one space for a CR LF pair.
            if c == CR && template.get(i + 1) == Some(&LF) {
                i += 1;
            }
            if out.last() != Some(&SPACE) {
                out.push(SPACE);
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
            0x30 => break, // %0
            0x31..=0x39 => {
                let (n, spec, after) = parse_insert(template, i);
                let first = if array { n } else { n + starred.iter().filter(|&&s| s < n).count() };
                let a = args.ok_or(ERROR_INVALID_PARAMETER)?;
                insert(&mut out, spec.unwrap_or(&[u16::from(b's')]), first, ansi, a)?;
                i = after;
                continue;
            }
            0x6E => out.extend_from_slice(&[CR, LF]), // %n
            0x72 => out.push(CR),                     // %r
            0x74 => out.push(9),                      // %t
            other => out.push(other),                 // %%, %., %!, %space, ...
        }
        i += 2;
    }
    if width != 0 && width != FORMAT_MESSAGE_MAX_WIDTH_MASK {
        out = wrap(&out, width as usize);
    }
    Ok(out)
}

/// Breaks every line (the text between CR LF pairs) into lines of at most
/// `width` characters at spaces; a word longer than `width` stays whole.
fn wrap(text: &[u16], width: usize) -> Vec<u16> {
    let mut out = Vec::with_capacity(text.len() + 16);
    let mut line_start = 0;
    let mut i = 0;
    let mut lines = Vec::new();
    while i <= text.len() {
        if i == text.len() || (text[i] == CR && text.get(i + 1) == Some(&LF)) {
            lines.push(&text[line_start..i]);
            line_start = i + 2;
            i += 2;
        } else {
            i += 1;
        }
    }
    for (k, line) in lines.iter().enumerate() {
        if k > 0 {
            out.extend_from_slice(&[CR, LF]);
        }
        let mut len = 0;
        for (w, word) in line.split(|&c| c == SPACE).enumerate() {
            if w > 0 {
                // A space without a word after it never starts a line.
                if !word.is_empty() && len > 0 && len + 1 + word.len() > width {
                    out.extend_from_slice(&[CR, LF]);
                    len = 0;
                } else {
                    out.push(SPACE);
                    len += 1;
                }
            }
            out.extend_from_slice(word);
            len += word.len();
        }
    }
    out
}

/// One insert: a printf-style conversion
/// (`[flags][width|*][.precision|.*][h|l|w|ll|I|I32|I64]type`) whose `*`s
/// take arguments `first`, `first + 1`, and whose value is the argument
/// after them.
fn insert(out: &mut Vec<u16>, spec: &[u16], first: usize, ansi: bool, a: &dyn Args) -> Result<(), u32> {
    let s: Vec<u8> = spec.iter().map(|&c| if c < 0x80 { c as u8 } else { b'?' }).collect();
    let mut next = first;
    let mut take = || {
        let v = a.slot(next).ok_or(ERROR_INVALID_PARAMETER);
        next += 1;
        v
    };
    let mut k = 0;
    let (mut left, mut zero, mut plus, mut blank, mut alt) = (false, false, false, false, false);
    while let Some(&f) = s.get(k) {
        match f {
            b'-' => left = true,
            b'0' => zero = true,
            b'+' => plus = true,
            b' ' => blank = true,
            b'#' => alt = true,
            _ => break,
        }
        k += 1;
    }
    let mut width = 0usize;
    if s.get(k) == Some(&b'*') {
        let w = take()? as u32 as i32;
        left |= w < 0;
        width = w.unsigned_abs() as usize;
        k += 1;
    } else {
        while let Some(d) = s.get(k).filter(|c| c.is_ascii_digit()) {
            width = width * 10 + usize::from(d - b'0');
            k += 1;
        }
    }
    let mut precision = None;
    if s.get(k) == Some(&b'.') {
        k += 1;
        if s.get(k) == Some(&b'*') {
            let p = take()? as u32 as i32;
            precision = usize::try_from(p).ok();
            k += 1;
        } else {
            let mut p = 0usize;
            while let Some(d) = s.get(k).filter(|c| c.is_ascii_digit()) {
                p = p * 10 + usize::from(d - b'0');
                k += 1;
            }
            precision = Some(p);
        }
    }
    let (mut wide64, mut short, mut long) = (false, false, false);
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
                short = true;
                k += 1;
            }
            Some([b'l' | b'w', ..]) => {
                long = true;
                k += 1;
            }
            _ => break,
        }
    }
    let ty = *s.get(k).ok_or(ERROR_INVALID_PARAMETER)?;
    if k + 1 != s.len() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let v = take()?;
    // Which strings and characters are ANSI: `s`/`c` are the function's
    // own kind, `S`/`C` the other one; `h` makes them ANSI, `l`/`w` wide.
    let ansi_text = |upper: bool| {
        if short {
            true
        } else if long {
            false
        } else {
            ansi != upper
        }
    };
    let number = |t: &str| crate::wide(t);
    let mut sign: &[u16] = &[];
    let mut prefix: Vec<u16> = Vec::new();
    let body: Vec<u16> = match ty {
        b's' | b'S' => {
            let mut b = if ansi_text(ty == b'S') { a.ansi(v) } else { a.wide(v) };
            if let Some(p) = precision {
                b.truncate(p);
            }
            b
        }
        b'c' | b'C' => {
            if ansi_text(ty == b'C') {
                crate::text::from_ansi(&[v as u8])
            } else {
                alloc::vec![v as u16]
            }
        }
        b'd' | b'i' => {
            let n: i64 = if wide64 { v as i64 } else { i64::from(v as u32 as i32) };
            if n < 0 {
                sign = &[b'-' as u16];
            } else if plus {
                sign = &[b'+' as u16];
            } else if blank {
                sign = &[SPACE];
            }
            let mut t = alloc::string::String::new();
            let _ = core::fmt::write(&mut t, format_args!("{}", n.unsigned_abs()));
            number(&t)
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
            if alt && n != 0 {
                prefix = number(match ty {
                    b'x' => "0x",
                    b'X' => "0X",
                    b'o' => "0",
                    _ => "",
                });
            }
            number(&t)
        }
        b'p' => {
            let mut t = alloc::string::String::new();
            let _ = core::fmt::write(&mut t, format_args!("{v:016X}"));
            number(&t)
        }
        // Learn: floating-point formats are not supported.
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    let numeric = !matches!(ty, b's' | b'S' | b'c' | b'C');
    let mut body = body;
    if numeric && let Some(p) = precision {
        // An integer's precision is its minimum number of digits.
        let missing = p.saturating_sub(body.len());
        body.splice(0..0, core::iter::repeat_n(u16::from(b'0'), missing));
    }
    let len = sign.len() + prefix.len() + body.len();
    let pad = width.saturating_sub(len);
    if left {
        out.extend_from_slice(sign);
        out.extend_from_slice(&prefix);
        out.extend_from_slice(&body);
        out.extend(core::iter::repeat_n(SPACE, pad));
    } else if zero && numeric && precision.is_none() {
        // Zeros go after a sign or prefix.
        out.extend_from_slice(sign);
        out.extend_from_slice(&prefix);
        out.extend(core::iter::repeat_n(u16::from(b'0'), pad));
        out.extend_from_slice(&body);
    } else {
        out.extend(core::iter::repeat_n(SPACE, pad));
        out.extend_from_slice(sign);
        out.extend_from_slice(&prefix);
        out.extend_from_slice(&body);
    }
    Ok(())
}

/// A system message as `FormatMessage` produces it (the text and CR LF,
/// formatted), or the error the call fails with.
pub fn system(id: u32, lang: u32, flags: u32, ansi: bool, args: Option<&dyn Args>) -> Result<Vec<u16>, u32> {
    if !language_ok(lang) {
        return Err(ERROR_RESOURCE_LANG_NOT_FOUND);
    }
    let text = system_message(id).ok_or(ERROR_MR_MID_NOT_FOUND)?;
    let mut t = crate::wide(text);
    t.extend_from_slice(&[CR, LF]);
    format(&t, flags, ansi, args)
}

/// `MESSAGE_RESOURCE_UNICODE` and `MESSAGE_RESOURCE_UTF8` (`winnt.h`).
const MESSAGE_RESOURCE_UNICODE: u16 = 1;
const MESSAGE_RESOURCE_UTF8: u16 = 2;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// Message `id` of a message table resource (`MESSAGE_RESOURCE_DATA`):
/// `None` if no block holds it or the table is malformed.
pub fn table_message(table: &[u8], id: u32) -> Option<Vec<u16>> {
    let blocks = u32_at(table, 0)? as usize;
    for b in 0..blocks.min(table.len() / 12) {
        let at = 4 + b * 12;
        let (low, high, mut off) = (u32_at(table, at)?, u32_at(table, at + 4)?, u32_at(table, at + 8)? as usize);
        if id < low || id > high {
            continue;
        }
        for _ in low..id {
            let len = usize::from(u16_at(table, off)?);
            if len < 4 {
                return None;
            }
            off += len;
        }
        let len = usize::from(u16_at(table, off)?);
        let flags = u16_at(table, off + 2)?;
        let text = table.get(off + 4..off.checked_add(len)?)?;
        return Some(match flags {
            MESSAGE_RESOURCE_UNICODE => {
                let units = text.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
                units.take_while(|&u| u != 0).collect()
            }
            MESSAGE_RESOURCE_UTF8 => {
                let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
                let s = alloc::string::String::from_utf8_lossy(&text[..end]);
                s.encode_utf16().collect()
            }
            _ => {
                let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
                crate::text::from_ansi(&text[..end])
            }
        });
    }
    None
}

/// A module's message as `FormatMessage` produces it. `tables` are the
/// module's `RT_MESSAGETABLE` resources named 1, by language (empty: the
/// module has none, `ERROR_RESOURCE_TYPE_NOT_FOUND`).
pub fn module(
    tables: &[(u32, &[u8])],
    id: u32,
    lang: u32,
    flags: u32,
    ansi: bool,
    args: Option<&dyn Args>,
) -> Result<Vec<u16>, u32> {
    if tables.is_empty() {
        return Err(ERROR_RESOURCE_TYPE_NOT_FOUND);
    }
    let table = match requested_language(lang) {
        Some(l) => tables.iter().find(|t| t.0 == l),
        // Neutral, then the thread's, user's and system's defaults and
        // English (United States) (all `LANG_EN_US` on Poly), then any.
        None => {
            tables.iter().find(|t| t.0 == 0).or_else(|| tables.iter().find(|t| t.0 == LANG_EN_US)).or(tables.first())
        }
    };
    let table = table.ok_or(ERROR_RESOURCE_LANG_NOT_FOUND)?;
    let text = table_message(table.1, id).ok_or(ERROR_MR_MID_NOT_FOUND)?;
    format(&text, flags, ansi, args)
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
            let mut s = self.1[address as usize].clone();
            s.insert(0, u16::from(b'A'));
            s
        }
    }

    fn f(t: &str, flags: u32, args: &Slots) -> String {
        String::from_utf16_lossy(&format(&w(t), flags, false, Some(args)).unwrap())
    }

    fn s(id: u32) -> String {
        String::from_utf16_lossy(&system(id, 0, FORMAT_MESSAGE_IGNORE_INSERTS, false, None).unwrap())
    }

    #[test]
    fn system_messages() {
        assert_eq!(s(0), "The operation completed successfully.\r\n");
        assert_eq!(s(2), "The system cannot find the file specified.\r\n");
        assert_eq!(s(5), "Access is denied.\r\n");
        assert_eq!(s(193), "%1 is not a valid Win32 application.\r\n");
        assert_eq!(s(1813), "The specified resource type cannot be found in the image file.\r\n");
        assert_eq!(s(10054), "An existing connection was forcibly closed by the remote host.\r\n");
        assert_eq!(system(0xDEAD, 0, 0, false, None), Err(ERROR_MR_MID_NOT_FOUND));
        assert_eq!(system(0x1_0002, 0, 0, false, None), Err(ERROR_MR_MID_NOT_FOUND));
        // An insert without arguments.
        assert_eq!(system(193, 0, 0, false, None), Err(ERROR_INVALID_PARAMETER));
        let a = Slots(vec![0], vec![w("x.exe")]);
        assert_eq!(
            String::from_utf16_lossy(&system(193, 0, 0, false, Some(&a)).unwrap()),
            "x.exe is not a valid Win32 application.\r\n"
        );
        // FormatMessageA: the insert is an ANSI string.
        assert_eq!(
            String::from_utf16_lossy(&system(193, 0, 0, true, Some(&a)).unwrap()),
            "Ax.exe is not a valid Win32 application.\r\n"
        );
    }

    #[test]
    fn the_whole_table_is_there() {
        assert_eq!(sysmsg::CODES.len(), sysmsg::ENDS.len());
        assert!(sysmsg::CODES.windows(2).all(|p| p[0] < p[1]));
        assert_eq!(*sysmsg::ENDS.last().unwrap() as usize, sysmsg::TEXT.len());
        assert!(sysmsg::CODES.len() > 2500);
        for &code in &sysmsg::CODES {
            let t = system_message(u32::from(code)).unwrap();
            assert!(!t.is_empty() && !t.contains('\n') && t.trim() == t, "{code}: {t:?}");
        }
        // Every range Learn lists, including registry (1009-1021), RPC
        // (1700-1999) and Winsock (10004-11999) codes.
        for code in [1, 487, 500, 998, 1009, 1021, 1299, 1300, 1700, 2250, 4000, 6000, 8200, 9001, 10060, 13000, 15841]
        {
            assert!(system_message(code).is_some(), "{code}");
        }
    }

    #[test]
    fn hresults_of_facility_win32() {
        assert_eq!(s(0x8007_0002), "The system cannot find the file specified.\r\n");
        assert_eq!(s(0x8007_0005), "Access is denied.\r\n");
        assert_eq!(win32_code(0x8007_00B7), 183);
        assert_eq!(win32_code(0x8000_4005), 0x8000_4005);
        assert_eq!(system(0x8007_FFFF, 0, 0, false, None), Err(ERROR_MR_MID_NOT_FOUND));
        // Other facilities are not Win32 codes.
        assert_eq!(system(0x8008_0002, 0, 0, false, None), Err(ERROR_MR_MID_NOT_FOUND));
    }

    #[test]
    fn languages() {
        assert!(system(2, 0x0409, 0, false, None).is_ok());
        assert!(system(2, 0x0400, 0, false, None).is_ok());
        assert!(system(2, 0x0800, 0, false, None).is_ok());
        assert_eq!(system(2, 0x0407, 0, false, None), Err(ERROR_RESOURCE_LANG_NOT_FOUND));
        assert_eq!(system(2, 0x0809, 0, false, None), Err(ERROR_RESOURCE_LANG_NOT_FOUND));
    }

    #[test]
    fn inserts_and_formats() {
        let a = Slots(vec![0, 42, (-5i32) as u32 as u64, 0xBEEF, 1], vec![w("one"), w("two")]);
        assert_eq!(f("[%1] [%2!d!] [%3!d!] [%4!x!] [%4!08X!]", 0, &a), "[one] [42] [-5] [beef] [0000BEEF]");
        assert_eq!(f("%2!5u!|%2!-5u!|%3!05d!", 0, &a), "   42|42   |-0005");
        assert_eq!(f("%5!s!%1!.2s!", 0, &a), "twoon");
        assert_eq!(f("%1%2!u!", 0, &a), "one42");
        assert_eq!(f("%2!+d!|%2! d!|%4!#x!|%2!.4u!|%3!.3d!", 0, &a), "+42| 42|0xbeef|0042|-005");
        assert_eq!(f("%1!S!|%1!hs!|%1!ls!|%1!ws!", 0, &a), "Aone|Aone|one|one");
        assert_eq!(format(&w("%12"), 0, false, Some(&a)), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn ansi_inserts() {
        let a = Slots(vec![0, 0x41], vec![w("one")]);
        let g = |t: &str| String::from_utf16_lossy(&format(&w(t), 0, true, Some(&a)).unwrap());
        assert_eq!(g("%1|%1!s!|%1!S!|%1!hs!|%1!ls!"), "Aone|Aone|one|Aone|one");
        assert_eq!(g("%2!c!%2!C!"), "AA");
    }

    #[test]
    fn star_width_and_precision() {
        // Learn's two examples: an argument array and a va_list.
        let array = Slots(vec![4, 2, 0, 1, 6, 0], vec![w("Bill"), w("Bob")]);
        assert_eq!(f("%1!*.*s! %4 %5!*s!", FORMAT_MESSAGE_ARGUMENT_ARRAY, &array), "  Bi Bob   Bill");
        let va = Slots(vec![4, 2, 0, 1, 6, 0], vec![w("Bill"), w("Bob")]);
        assert_eq!(f("%1!*.*s! %3 %4!*s!", 0, &va), "  Bi Bob   Bill");
        let n = Slots(vec![5, 42], vec![]);
        assert_eq!(f("[%1!*u!]", FORMAT_MESSAGE_ARGUMENT_ARRAY, &n), "[   42]");
        let neg = Slots(vec![(-5i32) as u32 as u64, 42], vec![]);
        assert_eq!(f("[%1!*u!]", FORMAT_MESSAGE_ARGUMENT_ARRAY, &neg), "[42   ]");
    }

    #[test]
    fn escapes() {
        let a = Slots(vec![], vec![]);
        assert_eq!(f("a%nb%rc%td%%e%.f%!g% h", 0, &a), "a\r\nb\rc\td%e.f!g h");
        assert_eq!(f("stop%0more", 0, &a), "stop");
        assert_eq!(f("keep %1 %n", FORMAT_MESSAGE_IGNORE_INSERTS, &a), "keep %1 %n");
        assert_eq!(f("one\r\ntwo%nthree\r\n", FORMAT_MESSAGE_MAX_WIDTH_MASK, &a), "one two\r\nthree ");
        assert_eq!(f("one\r\ntwo\r\n", 0, &a), "one\r\ntwo\r\n");
        assert_eq!(f("trailing%", 0, &a), "trailing%");
    }

    #[test]
    fn line_width() {
        let a = Slots(vec![], vec![]);
        assert_eq!(f("aaa bbb ccc\r\ndd%neeeeeeee f\r\n", 10, &a), "aaa bbb\r\nccc dd\r\neeeeeeee f ");
        assert_eq!(f("abcdefghijkl mn", 5, &a), "abcdefghijkl\r\nmn");
        assert_eq!(
            String::from_utf16_lossy(&system(2, 0, 20, false, None).unwrap()),
            "The system cannot\r\nfind the file\r\nspecified. "
        );
    }

    #[test]
    fn bad_inserts() {
        let a = Slots(vec![1], vec![]);
        assert_eq!(format(&w("%2"), 0, false, Some(&a)), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(format(&w("%1!q!"), 0, false, Some(&a)), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(format(&w("%1!f!"), 0, false, Some(&a)), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(format(&w("%1!*s!"), 0, false, Some(&a)), Err(ERROR_INVALID_PARAMETER));
    }

    /// A message table entry: `(flags, bytes)`.
    type TestEntry = (u16, Vec<u8>);

    /// A `MESSAGE_RESOURCE_DATA` with blocks `(low, entries)`.
    fn table(blocks: &[(u32, Vec<TestEntry>)]) -> Vec<u8> {
        let mut head = Vec::from((blocks.len() as u32).to_le_bytes());
        let mut body = Vec::new();
        let base = 4 + 12 * blocks.len();
        for (low, entries) in blocks {
            head.extend_from_slice(&low.to_le_bytes());
            head.extend_from_slice(&(low + entries.len() as u32 - 1).to_le_bytes());
            head.extend_from_slice(&((base + body.len()) as u32).to_le_bytes());
            for (flags, text) in entries {
                let mut t = text.clone();
                t.push(0);
                while t.len() % 4 != 0 {
                    t.push(0);
                }
                body.extend_from_slice(&((t.len() + 4) as u16).to_le_bytes());
                body.extend_from_slice(&flags.to_le_bytes());
                body.extend_from_slice(&t);
            }
        }
        head.extend_from_slice(&body);
        head
    }

    fn utf16(s: &str) -> Vec<u8> {
        let mut b: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        b.push(0);
        b
    }

    #[test]
    fn module_tables() {
        let t = table(&[
            (1, vec![(0, b"ansi one\r\n".to_vec()), (1, utf16("wide %1\r\n"))]),
            (0x100, vec![(2, "utf8 \u{e9}\r\n".as_bytes().to_vec())]),
        ]);
        let m = |id| table_message(&t, id).map(|v| String::from_utf16_lossy(&v));
        assert_eq!(m(1).as_deref(), Some("ansi one\r\n"));
        assert_eq!(m(2).as_deref(), Some("wide %1\r\n"));
        assert_eq!(m(0x100).as_deref(), Some("utf8 \u{e9}\r\n"));
        assert_eq!(m(3), None);
        assert_eq!(m(0), None);
        assert_eq!(table_message(&[1, 0, 0, 0], 1), None);

        let german = table(&[(1, vec![(0, b"deutsch\r\n".to_vec())])]);
        let a = Slots(vec![0], vec![w("x")]);
        let both = [(0x0407, german.as_slice()), (0x0409, t.as_slice())];
        let g = |tables: &[(u32, &[u8])], id, lang| {
            module(tables, id, lang, 0, false, Some(&a)).map(|v| String::from_utf16_lossy(&v))
        };
        assert_eq!(g(&both, 2, 0).as_deref(), Ok("wide x\r\n"));
        assert_eq!(g(&both, 1, 0x0407).as_deref(), Ok("deutsch\r\n"));
        assert_eq!(g(&both, 1, 0x040C), Err(ERROR_RESOURCE_LANG_NOT_FOUND));
        assert_eq!(g(&both, 9, 0), Err(ERROR_MR_MID_NOT_FOUND));
        assert_eq!(g(&[], 1, 0), Err(ERROR_RESOURCE_TYPE_NOT_FOUND));
        // Language 0 takes any language when there is no neutral or English one.
        assert_eq!(g(&both[..1], 1, 0).as_deref(), Ok("deutsch\r\n"));
    }
}
