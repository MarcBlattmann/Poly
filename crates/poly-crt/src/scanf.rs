//! The format engine of the `scanf` family (C11 7.21.6.2), with the MSVC
//! size prefixes of [`crate::printf`] (`I`, `I32`, `I64`, `w`, and `%S`/`%C`
//! / legacy wide specifiers as there) and the secure forms' buffer sizes
//! (`_CRT_INTERNAL_SCANF_SECURECRT`: `%c`, `%s` and `%[` take the size of
//! their buffer, in units, after the pointer; Learn: `scanf_s`).
//!
//! The option bits are `corecrt_stdio_config.h`'s `_CRT_INTERNAL_SCANF_*`.
//! Choices to be confirmed by the spec agent (ADR 0084): an integer that
//! overflows its type keeps its low bits; a scanset accepts `a-z` ranges
//! (Learn: "a hyphen ... is implementation-specific; MSVC treats it as a
//! range"); a secure buffer that is too small stops the scan with an empty
//! string.

use crate::convert::{self, Kind};
use crate::ctype;
use crate::float;
use crate::printf::Args;
use crate::unit::{self, Unit};
use alloc::vec::Vec;

/// `_CRT_INTERNAL_SCANF_SECURECRT`.
pub const SECURECRT: u64 = 1 << 0;
/// `_CRT_INTERNAL_SCANF_LEGACY_WIDE_SPECIFIERS`.
pub const LEGACY_WIDE_SPECIFIERS: u64 = 1 << 1;
/// `_CRT_INTERNAL_SCANF_LEGACY_MSVCRT_COMPATIBILITY`.
pub const LEGACY_MSVCRT_COMPATIBILITY: u64 = 1 << 2;

/// An invalid format string (the invalid parameter handler, `EINVAL`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidFormat;

/// Where the characters come from (a string, or a stream).
pub trait Input {
    /// The next character, without taking it; `None` at the end.
    fn peek(&mut self) -> Option<u32>;
    /// Take the character `peek` returned.
    fn bump(&mut self);
}

/// A string as input.
#[derive(Debug)]
pub struct Str<'a, C> {
    pub s: &'a [C],
    pub at: usize,
}

impl<C: Unit> Input for Str<'_, C> {
    fn peek(&mut self) -> Option<u32> {
        self.s.get(self.at).map(|c| c.to_u32()).filter(|&c| c != 0)
    }
    fn bump(&mut self) {
        self.at += 1;
    }
}

struct Counted<'a, I> {
    input: &'a mut I,
    count: usize,
}

impl<I: Input> Counted<'_, I> {
    fn peek(&mut self) -> Option<u32> {
        self.input.peek()
    }
    fn bump(&mut self) {
        self.input.bump();
        self.count += 1;
    }
    fn skip_space(&mut self, wide: bool) {
        while self.peek().is_some_and(|c| ctype::is_space_unit(c, wide)) {
            self.bump();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Size {
    None,
    Hh,
    H,
    L,
    Ll,
    W,
    Big,
}

/// Scan `input` by `fmt`; the number of fields assigned, or −1 (`EOF`) if
/// the input ended before the first conversion.
///
/// # Safety
/// The arguments are pointers of the types `fmt` names (and, with
/// [`SECURECRT`], the buffer sizes).
pub unsafe fn scan<C: Unit, I: Input, A: Args>(
    fmt: &[C],
    input: &mut I,
    args: &mut A,
    options: u64,
) -> Result<i32, InvalidFormat> {
    let mut inp = Counted { input, count: 0 };
    let at = |i: usize| fmt.get(i).map_or(0, |c| c.to_u32());
    let mut assigned = 0i32;
    let mut converted_any = false;
    let mut i = 0;
    let eof_result = |assigned: i32, any: bool| if any || assigned > 0 { assigned } else { -1 };
    while i < fmt.len() {
        let c = at(i);
        if ctype::is_space_unit(c, C::WIDE) {
            inp.skip_space(C::WIDE);
            i += 1;
            continue;
        }
        if c != u32::from(b'%') {
            match inp.peek() {
                Some(x) if x == c => {
                    inp.bump();
                    i += 1;
                    continue;
                }
                None => return Ok(eof_result(assigned, converted_any)),
                _ => return Ok(assigned),
            }
        }
        i += 1;
        let suppress = at(i) == u32::from(b'*');
        if suppress {
            i += 1;
        }
        let mut width = 0usize;
        while (0x30..=0x39).contains(&at(i)) {
            width = width * 10 + (at(i) - 0x30) as usize;
            i += 1;
        }
        let width = if width == 0 { None } else { Some(width) };
        let size = match at(i) {
            0x68 if at(i + 1) == 0x68 => {
                i += 2;
                Size::Hh
            }
            0x68 => {
                i += 1;
                Size::H
            }
            0x6C if at(i + 1) == 0x6C => {
                i += 2;
                Size::Ll
            }
            0x6C => {
                i += 1;
                Size::L
            }
            0x4C | 0x6A | 0x7A | 0x74 => {
                i += 1;
                Size::Big
            }
            0x77 => {
                i += 1;
                Size::W
            }
            0x49 if at(i + 1) == 0x36 && at(i + 2) == 0x34 => {
                i += 3;
                Size::Big
            }
            0x49 if at(i + 1) == 0x33 && at(i + 2) == 0x32 => {
                i += 3;
                Size::L
            }
            0x49 => {
                i += 1;
                Size::Big
            }
            _ => Size::None,
        };
        let Some(conv) = fmt.get(i).and_then(|c| c.as_ascii()) else { return Err(InvalidFormat) };
        i += 1;
        if !matches!(conv, b'c' | b'C' | b'[' | b'n') {
            inp.skip_space(C::WIDE);
        }
        match conv {
            b'%' => match inp.peek() {
                Some(0x25) => inp.bump(),
                None => return Ok(eof_result(assigned, converted_any)),
                _ => return Ok(assigned),
            },
            b'n' => {
                if !suppress {
                    let p = args.next() as usize;
                    // SAFETY: %n's argument is an int pointer of its size.
                    unsafe { store_int(p, size, inp.count as u64) };
                }
            }
            b'd' | b'i' | b'u' | b'o' | b'x' | b'X' | b'p' => {
                let base = match conv {
                    b'd' | b'u' => 10,
                    b'i' => 0,
                    b'o' => 8,
                    _ => 16,
                };
                let text = collect_integer(&mut inp, width.unwrap_or(usize::MAX), base);
                if text.is_empty() {
                    return Ok(if inp.peek().is_none() { eof_result(assigned, converted_any) } else { assigned });
                }
                let r = convert::parse(&text, base, Kind::U64);
                if r.len == 0 {
                    return Ok(assigned);
                }
                let v = r.value;
                converted_any = true;
                if !suppress {
                    let p = args.next() as usize;
                    let size = if conv == b'p' { Size::Big } else { size };
                    // SAFETY: the argument is an integer pointer of its size.
                    unsafe { store_int(p, size, v) };
                    assigned += 1;
                }
            }
            b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'a' | b'A' => {
                let text = collect_float(&mut inp, width.unwrap_or(usize::MAX), crate::locale::point_of(options));
                let fmt_ = if matches!(size, Size::L | Size::Big | Size::Ll) { float::DOUBLE } else { float::SINGLE };
                let parsed = float::parse(&text, fmt_);
                if parsed.len == 0 {
                    return Ok(if inp.peek().is_none() && text.is_empty() {
                        eof_result(assigned, converted_any)
                    } else {
                        assigned
                    });
                }
                converted_any = true;
                if !suppress {
                    let p = args.next() as usize;
                    // SAFETY: a float or double pointer by the size.
                    unsafe {
                        if fmt_ == float::DOUBLE {
                            (p as *mut u64).write_unaligned(parsed.bits);
                        } else {
                            (p as *mut u32).write_unaligned(parsed.bits as u32);
                        }
                    }
                    assigned += 1;
                }
            }
            b'c' | b'C' | b's' | b'S' | b'[' => {
                let set = if conv == b'[' {
                    let (set, used) = scanset(&fmt[i..]).ok_or(InvalidFormat)?;
                    i += used;
                    Some(set)
                } else {
                    None
                };
                let wide = match size {
                    Size::H | Size::Hh => false,
                    Size::L | Size::W => true,
                    _ if options & LEGACY_WIDE_SPECIFIERS != 0 => C::WIDE != matches!(conv, b'C' | b'S'),
                    _ => matches!(conv, b'C' | b'S'),
                };
                let limit = width.unwrap_or(if matches!(conv, b'c' | b'C') { 1 } else { usize::MAX });
                let mut got: Vec<u32> = Vec::new();
                while got.len() < limit {
                    let Some(ch) = inp.peek() else { break };
                    let take = match (&set, conv) {
                        (Some(s), _) => s.contains(ch),
                        (None, b'c' | b'C') => true,
                        _ => !ctype::is_space_unit(ch, C::WIDE),
                    };
                    if !take {
                        break;
                    }
                    got.push(ch);
                    inp.bump();
                }
                if got.is_empty() || (matches!(conv, b'c' | b'C') && got.len() < limit) {
                    if got.is_empty() {
                        return Ok(if inp.peek().is_none() { eof_result(assigned, converted_any) } else { assigned });
                    }
                    return Ok(assigned);
                }
                converted_any = true;
                if !suppress {
                    let p = args.next() as usize;
                    let terminate = !matches!(conv, b'c' | b'C');
                    let need = got.len() + usize::from(terminate);
                    if options & SECURECRT != 0 {
                        let room = args.next() as u32 as usize;
                        if room < need {
                            if room > 0 {
                                // SAFETY: a buffer of at least one unit.
                                unsafe { if wide { (p as *mut u16).write(0) } else { (p as *mut u8).write(0) } }
                            }
                            return Ok(assigned);
                        }
                    }
                    for (k, &ch) in got.iter().enumerate() {
                        let v = if wide {
                            if C::WIDE { ch } else { u32::from(unit::c_locale_to_wide(ch as u8)) }
                        } else if C::WIDE {
                            u32::from(unit::c_locale_to_narrow(ch as u16).unwrap_or(b'?'))
                        } else {
                            ch
                        };
                        // SAFETY: the buffer holds the field (and its size
                        // was checked in the secure form).
                        unsafe {
                            if wide {
                                (p as *mut u16).add(k).write(v as u16)
                            } else {
                                (p as *mut u8).add(k).write(v as u8)
                            }
                        }
                    }
                    if terminate {
                        // SAFETY: as above, the NUL's unit.
                        unsafe {
                            if wide {
                                (p as *mut u16).add(got.len()).write(0)
                            } else {
                                (p as *mut u8).add(got.len()).write(0)
                            }
                        }
                    }
                    assigned += 1;
                }
            }
            _ => return Err(InvalidFormat),
        }
    }
    Ok(assigned)
}

/// # Safety
/// `p` points to an integer of `size`'s width.
unsafe fn store_int(p: usize, size: Size, v: u64) {
    // SAFETY: the caller's contract.
    unsafe {
        match size {
            Size::Hh => (p as *mut u8).write_unaligned(v as u8),
            Size::H => (p as *mut u16).write_unaligned(v as u16),
            Size::None | Size::L | Size::W => (p as *mut u32).write_unaligned(v as u32),
            Size::Ll | Size::Big => (p as *mut u64).write_unaligned(v),
        }
    }
}

fn collect_integer<I: Input>(inp: &mut Counted<'_, I>, width: usize, base: u32) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let take = |inp: &mut Counted<'_, I>, out: &mut Vec<u8>, ok: &dyn Fn(u32) -> bool| -> bool {
        match inp.peek() {
            Some(c) if out.len() < width && ok(c) => {
                out.push(c as u8);
                inp.bump();
                true
            }
            _ => false,
        }
    };
    take(inp, &mut out, &|c| c == 0x2B || c == 0x2D);
    let mut base = base;
    if (base == 0 || base == 16) && take(inp, &mut out, &|c| c == 0x30) {
        if take(inp, &mut out, &|c| c == 0x78 || c == 0x58) {
            base = 16;
        } else if base == 0 {
            base = 8;
        }
    }
    let base = if base == 0 { 10 } else { base };
    while take(inp, &mut out, &|c| char::from_u32(c).and_then(|ch| ch.to_digit(base)).is_some()) {}
    out
}

fn collect_float<I: Input>(inp: &mut Counted<'_, I>, width: usize, point: u8) -> Vec<u8> {
    // Take the longest prefix the float grammar could extend.
    let mut out: Vec<u8> = Vec::new();
    while out.len() < width {
        let Some(c) = inp.peek() else { break };
        if c >= 0x80 {
            break;
        }
        // The locale's decimal point stands for '.' (ADR 0094).
        let b = match c as u8 {
            b'.' if point != b'.' => break,
            b if b == point => b'.',
            b => b,
        };
        let mut candidate = out.clone();
        candidate.push(b);
        if !float_prefix(&candidate) {
            break;
        }
        out = candidate;
        inp.bump();
    }
    out
}

/// Whether `s` can begin a `strtod` subject sequence.
fn float_prefix(s: &[u8]) -> bool {
    let body = s.strip_prefix(b"+").or_else(|| s.strip_prefix(b"-")).unwrap_or(s);
    let l: Vec<u8> = body.to_ascii_lowercase();
    if b"infinity".starts_with(&l) || b"nan".starts_with(&l) {
        return true;
    }
    if l.starts_with(b"nan") {
        return nan_tail(&l[3..]);
    }
    let (hex, digits) = match l.strip_prefix(b"0x") {
        Some(rest) => (true, rest),
        None => (false, &l[..]),
    };
    let exp_char = if hex { b'p' } else { b'e' };
    let (mut point, mut exp, mut mantissa_digits) = (false, false, false);
    let mut prev = 0u8;
    for &c in digits {
        let digit = if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() };
        if exp {
            if !(c.is_ascii_digit() || ((c == b'+' || c == b'-') && prev == exp_char)) {
                return false;
            }
        } else if c == b'.' && !point {
            point = true;
        } else if c == exp_char && (mantissa_digits || hex) {
            exp = true;
        } else if digit {
            mantissa_digits = true;
        } else {
            return false;
        }
        prev = c;
    }
    true
}

fn nan_tail(rest: &[u8]) -> bool {
    match rest.split_first() {
        None => true,
        Some((b'(', inner)) => {
            let close = inner.iter().position(|&c| c == b')');
            match close {
                Some(k) => k + 1 == inner.len() && inner[..k].iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_'),
                None => inner.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_'),
            }
        }
        _ => false,
    }
}

/// A `%[` scanset.
#[derive(Debug)]
struct Set {
    negate: bool,
    ranges: Vec<(u32, u32)>,
}

impl Set {
    fn contains(&self, c: u32) -> bool {
        self.ranges.iter().any(|&(a, b)| (a..=b).contains(&c)) != self.negate
    }
}

/// Parse the scanset after `%[`; the set and the units used (with `]`).
fn scanset<C: Unit>(s: &[C]) -> Option<(Set, usize)> {
    let at = |i: usize| s.get(i).map(|c| c.to_u32());
    let mut i = 0;
    let negate = at(0) == Some(0x5E);
    if negate {
        i += 1;
    }
    let mut ranges = Vec::new();
    let mut first = true;
    loop {
        let c = at(i)?;
        if c == 0x5D && !first {
            return Some((Set { negate, ranges }, i + 1));
        }
        first = false;
        if at(i + 1) == Some(0x2D) && at(i + 2).is_some_and(|e| e != 0x5D) {
            let e = at(i + 2)?;
            ranges.push((c.min(e), c.max(e)));
            i += 3;
        } else {
            ranges.push((c, c));
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printf::Slots;

    fn sc(input: &str, fmt: &str, args: &[u64]) -> i32 {
        let mut inp = Str { s: input.as_bytes(), at: 0 };
        // SAFETY: the tests pass matching pointers.
        unsafe { scan(fmt.as_bytes(), &mut inp, &mut Slots(args), LEGACY_WIDE_SPECIFIERS) }.unwrap()
    }

    #[test]
    fn integers_and_counts() {
        let (mut a, mut b, mut c) = (0i32, 0u32, 0i64);
        let mut n = 0i32;
        let r = sc(
            "  -12 0x1f 077 rest",
            "%d %x %lli%n",
            &[
                &mut a as *mut i32 as u64,
                &mut b as *mut u32 as u64,
                &mut c as *mut i64 as u64,
                &mut n as *mut i32 as u64,
            ],
        );
        assert_eq!((r, a, b, c, n), (3, -12, 31, 63, 14));
        let mut h = 0i16;
        assert_eq!(sc("70000", "%hd", &[&mut h as *mut i16 as u64]), 1);
        assert_eq!(h, 70000u32 as i16);
        let (mut x, mut y) = (0i32, 0i32);
        assert_eq!(sc("12345", "%2d%3d", &[&mut x as *mut i32 as u64, &mut y as *mut i32 as u64]), 2);
        assert_eq!((x, y), (12, 345));
        assert_eq!(sc("abc", "%d", &[&mut x as *mut i32 as u64]), 0);
        assert_eq!(sc("", "%d", &[&mut x as *mut i32 as u64]), -1);
        assert_eq!(sc("   ", "%d", &[&mut x as *mut i32 as u64]), -1);
        assert_eq!(sc("5 x", "%*d %d", &[&mut x as *mut i32 as u64]), 0);
        let mut p = 0u64;
        assert_eq!(sc("00000000DEADBEEF", "%p", &[&mut p as *mut u64 as u64]), 1);
        assert_eq!(p, 0xDEADBEEF);
    }

    #[test]
    fn floats() {
        let (mut f, mut d) = (0f32, 0f64);
        assert_eq!(sc("3.5 -1e-3x", "%f %lf", &[&mut f as *mut f32 as u64, &mut d as *mut f64 as u64]), 2);
        assert_eq!((f, d), (3.5, -0.001));
        assert_eq!(sc("inf nan", "%lf %f", &[&mut d as *mut f64 as u64, &mut f as *mut f32 as u64]), 2);
        assert!(d.is_infinite() && f.is_nan());
        assert_eq!(sc("0x1.8p1", "%lg", &[&mut d as *mut f64 as u64]), 1);
        assert_eq!(d, 3.0);
        assert_eq!(sc("1.25e", "%lf", &[&mut d as *mut f64 as u64]), 1);
        assert_eq!(d, 1.25);
        assert_eq!(sc("12.5", "%3lf", &[&mut d as *mut f64 as u64]), 1);
        assert_eq!(d, 12.0);
    }

    #[test]
    fn strings_sets_and_characters() {
        let mut buf = [0u8; 16];
        let mut ch = [0u8; 3];
        let r = sc("  hello world", "%s %2c", &[buf.as_mut_ptr() as u64, ch.as_mut_ptr() as u64]);
        assert_eq!(r, 2);
        assert_eq!(&buf[..6], b"hello\0");
        assert_eq!(&ch[..2], b"wo");
        let mut rest = [0u8; 8];
        let r = sc("abc123def", "%[a-c]%[^e]", &[buf.as_mut_ptr() as u64, rest.as_mut_ptr() as u64]);
        assert_eq!(r, 2);
        assert_eq!((&buf[..4], &rest[..5]), (&b"abc\0"[..], &b"123d\0"[..]));
        let mut w = [0u16; 8];
        let r = sc("wide", "%ls", &[w.as_mut_ptr() as u64]);
        assert_eq!((r, &w[..5]), (1, &[0x77u16, 0x69, 0x64, 0x65, 0][..]));
        let r = sc("x]y", "%[]x]", &[buf.as_mut_ptr() as u64]);
        assert_eq!((r, &buf[..3]), (1, &b"x]\0"[..]));
        let r = sc("key=value", "%[^=]=%s", &[buf.as_mut_ptr() as u64, w.as_mut_ptr() as u64]);
        assert_eq!(r, 2);
        assert_eq!(&buf[..4], b"key\0");
        assert_eq!(sc("50%", "%d%%", &[&mut 0i32 as *mut i32 as u64]), 1);
        assert_eq!(sc("a", "%c%c", &[ch.as_mut_ptr() as u64, ch.as_mut_ptr() as u64]), 1);
    }

    #[test]
    fn secure_sizes() {
        let mut buf = [0xAAu8; 4];
        let mut inp = Str { s: b"toolong", at: 0 };
        // SAFETY: a 4-byte buffer and its size.
        let r = unsafe { scan(b"%s", &mut inp, &mut Slots(&[buf.as_mut_ptr() as u64, 4]), SECURECRT) }.unwrap();
        assert_eq!((r, buf[0]), (0, 0));
        let mut inp = Str { s: b"ok", at: 0 };
        // SAFETY: as above.
        let r = unsafe { scan(b"%s", &mut inp, &mut Slots(&[buf.as_mut_ptr() as u64, 4]), SECURECRT) }.unwrap();
        assert_eq!((r, &buf[..3]), (1, &b"ok\0"[..]));
        let mut inp = Str { s: b"x", at: 0 };
        // SAFETY: no arguments needed before the error.
        assert_eq!(unsafe { scan(b"%y", &mut inp, &mut Slots(&[]), 0) }, Err(InvalidFormat));
    }

    #[test]
    fn wide_input() {
        let s: Vec<u16> = "42 \u{e9}t\u{e9}".encode_utf16().collect();
        let f: Vec<u16> = "%d %hs".encode_utf16().collect();
        let mut n = 0i32;
        let mut buf = [0u8; 8];
        let mut inp = Str { s: &s, at: 0 };
        // SAFETY: matching pointers.
        let r = unsafe {
            scan(
                &f,
                &mut inp,
                &mut Slots(&[&mut n as *mut i32 as u64, buf.as_mut_ptr() as u64]),
                LEGACY_WIDE_SPECIFIERS,
            )
        }
        .unwrap();
        assert_eq!((r, n, &buf[..4]), (2, 42, &[0xE9u8, b't', 0xE9, 0][..]));
    }
}
