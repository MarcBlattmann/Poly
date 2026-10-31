//! The format engine of the `printf` family (C11 7.21.6.1, 7.29.2.1), with
//! the MSVC specifics Learn documents in "Format specification syntax:
//! printf and wprintf functions" and "Size specification":
//!
//! - size prefixes `hh h l ll j z t L` (C), plus `w` (wide, for `c`/`s`/
//!   `Z`), `I` (`size_t`/`ptrdiff_t`: 64-bit on x64), `I32`, `I64`;
//!   `long double` is `double`;
//! - `%C` and `%S` are the character and string of the *other* width, and
//!   with the legacy wide specifiers (the default of the UCRT headers,
//!   `_CRT_INTERNAL_PRINTF_LEGACY_WIDE_SPECIFIERS`) `%c`/`%s` in a wide
//!   function are wide; `%hs`/`%hc` are always narrow, `%ls`/`%lc`/`%ws`
//!   always wide;
//! - `%Z`: the `ANSI_STRING` / `UNICODE_STRING` (`Length` in bytes,
//!   `Buffer` at + 8). The UCRT's documented "known issue": without a
//!   prefix `%Z` is a `UNICODE_STRING` in a narrow function and an
//!   `ANSI_STRING` in a wide one, like `%S`; `%hZ` is ANSI, `%wZ`/`%lZ`
//!   Unicode. Before the UCRT (`msvcrt.dll`) `%Z` was ANSI;
//! - infinities and NaNs print as `inf`, `nan`, `nan(snan)` and, for the
//!   indefinite NaN, `nan(ind)` (capitals for `E F G A`); the legacy
//!   (`msvcrt.dll`) forms are `1.#INF`, `1.#QNAN`, `1.#SNAN`, `1.#IND`;
//! - `%e` exponents have at least two digits, three with
//!   `_CRT_INTERNAL_PRINTF_LEGACY_THREE_DIGIT_EXPONENTS` (`msvcrt.dll`'s
//!   default: Learn, `_set_output_format`);
//! - `%a` has 13 hexadecimal digits by default (Learn: "Default precision
//!   is 13");
//! - `%n` is disabled unless `_set_printf_count_output` enabled it (an
//!   invalid parameter otherwise);
//! - a null `%s` prints `(null)`.
//!
//! The option bits are `corecrt_stdio_config.h`'s
//! `_CRT_INTERNAL_PRINTF_*`.
//!
//! Choices to be confirmed by the spec agent (ADR 0084): `%p` is 16
//! upper-case hexadecimal digits; `%Lx` is 64-bit like `%llx`; in the ISO
//! (non-legacy) wide mode `%C`/`%S` stay wide; with
//! `_CRT_INTERNAL_PRINTF_LEGACY_MSVCRT_COMPATIBILITY` a `double` shows at
//! most 17 significant digits (the rest zeros), an unknown conversion is
//! printed as its letter, and the legacy infinity/NaN text is padded or cut
//! to the precision; `%.0a` may print a leading `2` after rounding.

use crate::float::{self, Class, Digits, Ties};
use crate::unit::{self, Unit};
use alloc::vec::Vec;

/// `_CRT_INTERNAL_PRINTF_LEGACY_VSPRINTF_NULL_TERMINATION`.
pub const LEGACY_VSPRINTF_NULL_TERMINATION: u64 = 1 << 0;
/// `_CRT_INTERNAL_PRINTF_STANDARD_SNPRINTF_BEHAVIOR`.
pub const STANDARD_SNPRINTF_BEHAVIOR: u64 = 1 << 1;
/// `_CRT_INTERNAL_PRINTF_LEGACY_WIDE_SPECIFIERS`.
pub const LEGACY_WIDE_SPECIFIERS: u64 = 1 << 2;
/// `_CRT_INTERNAL_PRINTF_LEGACY_MSVCRT_COMPATIBILITY`.
pub const LEGACY_MSVCRT_COMPATIBILITY: u64 = 1 << 3;
/// `_CRT_INTERNAL_PRINTF_LEGACY_THREE_DIGIT_EXPONENTS`.
pub const LEGACY_THREE_DIGIT_EXPONENTS: u64 = 1 << 4;
/// `_CRT_INTERNAL_PRINTF_STANDARD_ROUNDING` (Visual Studio 2019 16.2+).
pub const STANDARD_ROUNDING: u64 = 1 << 5;

/// Poly's own option bit (not a UCRT one): the positional-parameter
/// functions (`_printf_p`, `__stdio_common_vfprintf_p` …), Learn's
/// "printf_p Positional Parameters". See [`positional`].
pub const POSITIONAL: u64 = 1 << 63;

/// `_ARGMAX`: the most positional parameters a format may name (Learn:
/// "The maximum number of positional parameters allowed in a format string
/// is given by `_ARGMAX`"; 100 in the SDK headers).
pub const ARGMAX: usize = 100;

/// What `msvcrt.dll`'s own functions behave as.
pub const MSVCRT_OPTIONS: u64 = LEGACY_WIDE_SPECIFIERS | LEGACY_MSVCRT_COMPATIBILITY | LEGACY_THREE_DIGIT_EXPONENTS;

/// The source of the variadic arguments: on x64 every argument is one
/// 8-byte slot (an integer zero- or sign-extended by the caller as its type
/// says, a `double` as its bits, a pointer).
pub trait Args {
    fn next(&mut self) -> u64;
}

/// A Windows x64 `va_list`: a pointer to the next 8-byte slot.
#[derive(Debug)]
pub struct VaList(pub *const u64);

impl Args for VaList {
    fn next(&mut self) -> u64 {
        // SAFETY: `VaList` is only made from a caller's va_list, whose slots
        // hold as many arguments as its format string names (C11 7.16).
        let v = unsafe { self.0.read_unaligned() };
        // SAFETY: as above; the pointer moves to the next slot.
        self.0 = unsafe { self.0.add(1) };
        v
    }
}

/// Arguments from a slice (tests, and callers that built the slots).
#[derive(Debug)]
pub struct Slots<'a>(pub &'a [u64]);

impl Args for Slots<'_> {
    fn next(&mut self) -> u64 {
        match self.0.split_first() {
            Some((&v, rest)) => {
                self.0 = rest;
                v
            }
            None => 0,
        }
    }
}

/// Where the output goes, one unit at a time.
pub trait Sink<C> {
    fn put(&mut self, c: C);
}

impl<C: Copy> Sink<C> for Vec<C> {
    fn put(&mut self, c: C) {
        self.push(c);
    }
}

/// Why formatting stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// An invalid conversion, or `%n` while disabled: the invalid parameter
    /// handler, then `EINVAL`.
    Invalid,
    /// A character with no form in the output's encoding: `EILSEQ`.
    Encoding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Size {
    None,
    Hh,
    H,
    L,
    Ll,
    BigL,
    W,
    I,
    I32,
    I64,
    J,
    Z,
    T,
}

#[derive(Clone, Copy, Debug, Default)]
struct Spec {
    left: bool,
    plus: bool,
    space: bool,
    alt: bool,
    zero: bool,
    width: usize,
    precision: Option<usize>,
}

/// Format `fmt` (without its NUL) into `out`; the number of units written.
///
/// `count_output` is `_get_printf_count_output()`. With [`POSITIONAL`] in
/// `options` a format may number its arguments (`%2$s`, `*1$`).
///
/// # Safety
/// The arguments are those `fmt` describes (C11 7.21.6.1: "If any argument
/// is not the correct type for the corresponding conversion specification,
/// the behavior is undefined"): string, `%Z` and `%n` arguments are valid
/// pointers or null.
pub unsafe fn format<C: Unit, A: Args, S: Sink<C>>(
    fmt: &[C],
    args: &mut A,
    out: &mut S,
    options: u64,
    count_output: bool,
) -> Result<usize, Error> {
    if options & POSITIONAL != 0
        && let Some(n) = positional(fmt)?
    {
        // Every argument is one 8-byte slot on x64, so the slots can be
        // read in order up to the highest number, then picked by number.
        let mut slots = [0u64; ARGMAX];
        for s in slots.iter_mut().take(n) {
            *s = args.next();
        }
        let mut src = Source::<A>::Numbered(&slots[..n]);
        // SAFETY: the caller's contract.
        return unsafe { run(fmt, &mut src, out, options, count_output) };
    }
    // SAFETY: the caller's contract.
    unsafe { run(fmt, &mut Source::InOrder(args), out, options, count_output) }
}

/// Where a conversion's argument comes from.
enum Source<'a, A> {
    /// The next one (C).
    InOrder(&'a mut A),
    /// By its number (`%n$`), from the slots read up front.
    Numbered(&'a [u64]),
}

impl<A: Args> Source<'_, A> {
    fn numbered(&self) -> bool {
        matches!(self, Source::Numbered(_))
    }

    fn arg(&mut self, position: Option<usize>) -> u64 {
        match self {
            Source::InOrder(a) => a.next(),
            Source::Numbered(s) => position.and_then(|p| s.get(p.wrapping_sub(1))).copied().unwrap_or(0),
        }
    }

    /// After a `*` in a numbered format: its `n$`.
    fn star<C: Unit>(&self, fmt: &[C], i: &mut usize) -> Option<usize> {
        if !self.numbered() {
            return None;
        }
        let (p, j) = dollar(fmt, *i);
        *i = j;
        p
    }
}

/// `n$` at `fmt[i..]`: the number and the index after the `$` (or `i`).
fn dollar<C: Unit>(fmt: &[C], i: usize) -> (Option<usize>, usize) {
    let mut j = i;
    let mut n = 0usize;
    while let Some(d) = fmt.get(j).and_then(|c| digit(c.to_u32())) {
        n = n.saturating_mul(10).saturating_add(d);
        j += 1;
    }
    if j > i && fmt.get(j).map(|c| c.to_u32()) == Some(u32::from(b'$')) { (Some(n), j + 1) } else { (None, i) }
}

/// Whether `fmt` numbers its arguments (Learn, "printf_p Positional
/// Parameters"): `Ok(None)` if no conversion does (the format is read as by
/// `printf`), `Ok(Some(n))` with the highest number if every conversion and
/// every `*` does, each number from 1 to `n` is used at least once ("All
/// arguments must be used at least once") and `n ≤ _ARGMAX`. Mixing
/// numbered and unnumbered conversions ("If any positional formatting is
/// used, all format specifiers must use positional formatting"), a number
/// 0, a gap or a number above `_ARGMAX` is an invalid format: the invalid
/// parameter handler, then `EINVAL`, as for any invalid format. A number
/// may be used more than once. Poly does not compare the types of
/// conversions sharing a number (every argument is an 8-byte slot on x64).
pub fn positional<C: Unit>(fmt: &[C]) -> Result<Option<usize>, Error> {
    let at = |i: usize| fmt.get(i).map_or(0, |c| c.to_u32());
    let in_set = |c: u32, set: &[u8]| set.iter().any(|&b| u32::from(b) == c);
    let mut used = [false; ARGMAX + 1];
    let (mut numbered, mut plain) = (false, false);
    let mut note = |p: Option<usize>| -> Result<(), Error> {
        match p {
            Some(n) if n == 0 || n > ARGMAX => return Err(Error::Invalid),
            Some(n) => {
                used[n] = true;
                numbered = true;
            }
            None => plain = true,
        }
        Ok(())
    };
    let mut i = 0;
    while i < fmt.len() {
        if at(i) != u32::from(b'%') {
            i += 1;
            continue;
        }
        i += 1;
        if at(i) == u32::from(b'%') {
            i += 1;
            continue;
        }
        let (p, j) = dollar(fmt, i);
        i = j;
        note(p)?;
        while in_set(at(i), b"-+ #0") {
            i += 1;
        }
        for k in 0..2 {
            if k == 1 {
                if at(i) != u32::from(b'.') {
                    break;
                }
                i += 1;
            }
            if at(i) == u32::from(b'*') {
                let (p, j) = dollar(fmt, i + 1);
                i = j.max(i + 1);
                note(p)?;
            } else {
                while digit(at(i)).is_some() {
                    i += 1;
                }
            }
        }
        while in_set(at(i), b"hlLwjztI") {
            if at(i) == u32::from(b'I') && in_set(at(i + 1), b"36") && in_set(at(i + 2), b"24") {
                i += 2;
            }
            i += 1;
        }
        i += 1; // the conversion
    }
    if !numbered {
        return Ok(None);
    }
    if plain {
        return Err(Error::Invalid);
    }
    let n = used.iter().rposition(|&u| u).unwrap_or(0);
    if used[1..=n].iter().any(|&u| !u) {
        return Err(Error::Invalid);
    }
    Ok(Some(n))
}

/// [`format`]'s work, the arguments from `args`.
///
/// # Safety
/// As [`format`].
unsafe fn run<C: Unit, A: Args, S: Sink<C>>(
    fmt: &[C],
    args: &mut Source<'_, A>,
    out: &mut S,
    options: u64,
    count_output: bool,
) -> Result<usize, Error> {
    let mut f = Formatter { out, count: 0, options };
    let at = |i: usize| fmt.get(i).map_or(0, |c| c.to_u32());
    let is = |i: usize, b: u8| at(i) == u32::from(b);
    let mut i = 0;
    while i < fmt.len() {
        if !is(i, b'%') {
            f.put(fmt[i]);
            i += 1;
            continue;
        }
        i += 1;
        let mut spec = Spec::default();
        let mut pos = None;
        if args.numbered() && !is(i, b'%') {
            let (p, j) = dollar(fmt, i);
            pos = p;
            i = j;
        }
        loop {
            match at(i) {
                0x2D => spec.left = true,
                0x2B => spec.plus = true,
                0x20 => spec.space = true,
                0x23 => spec.alt = true,
                0x30 => spec.zero = true,
                _ => break,
            }
            i += 1;
        }
        if is(i, b'*') {
            i += 1;
            let at_star = args.star(fmt, &mut i);
            let w = args.arg(at_star) as i32;
            if w < 0 {
                spec.left = true;
            }
            spec.width = w.unsigned_abs() as usize;
        } else {
            while let Some(d) = digit(at(i)) {
                spec.width = spec.width.saturating_mul(10).saturating_add(d);
                i += 1;
            }
        }
        if is(i, b'.') {
            i += 1;
            if is(i, b'*') {
                i += 1;
                let at_star = args.star(fmt, &mut i);
                let p = args.arg(at_star) as i32;
                spec.precision = if p < 0 { None } else { Some(p as usize) };
            } else {
                let mut p = 0usize;
                while let Some(d) = digit(at(i)) {
                    p = p.saturating_mul(10).saturating_add(d);
                    i += 1;
                }
                spec.precision = Some(p);
            }
        }
        let size = match at(i) {
            0x68 if is(i + 1, b'h') => {
                i += 2;
                Size::Hh
            }
            0x68 => {
                i += 1;
                Size::H
            }
            0x6C if is(i + 1, b'l') => {
                i += 2;
                Size::Ll
            }
            0x6C => {
                i += 1;
                Size::L
            }
            0x4C => {
                i += 1;
                Size::BigL
            }
            0x77 => {
                i += 1;
                Size::W
            }
            0x6A => {
                i += 1;
                Size::J
            }
            0x7A => {
                i += 1;
                Size::Z
            }
            0x74 => {
                i += 1;
                Size::T
            }
            0x49 if is(i + 1, b'6') && is(i + 2, b'4') => {
                i += 3;
                Size::I64
            }
            0x49 if is(i + 1, b'3') && is(i + 2, b'2') => {
                i += 3;
                Size::I32
            }
            0x49 => {
                i += 1;
                Size::I
            }
            _ => Size::None,
        };
        let Some(conv) = fmt.get(i).and_then(|c| c.as_ascii()) else {
            if f.legacy() {
                // msvcrt: a stray '%' at the end is dropped.
                i += 1;
                continue;
            }
            return Err(Error::Invalid);
        };
        i += 1;
        match conv {
            b'%' => f.put(C::ascii(b'%')),
            b'd' | b'i' => {
                let raw = args.arg(pos);
                let v: i64 = match size {
                    Size::Hh => i64::from(raw as i8),
                    Size::H => i64::from(raw as i16),
                    Size::None | Size::L | Size::I32 | Size::W => i64::from(raw as i32),
                    _ => raw as i64,
                };
                f.integer(v.unsigned_abs(), v < 0, 10, false, &spec, true);
            }
            b'u' | b'o' | b'x' | b'X' => {
                let raw = args.arg(pos);
                let v: u64 = match size {
                    Size::Hh => u64::from(raw as u8),
                    Size::H => u64::from(raw as u16),
                    Size::None | Size::L | Size::I32 | Size::W => u64::from(raw as u32),
                    _ => raw,
                };
                let base = match conv {
                    b'u' => 10,
                    b'o' => 8,
                    _ => 16,
                };
                f.integer(v, false, base, conv == b'X', &spec, false);
            }
            b'p' => {
                let v = args.arg(pos);
                let mut digits = [0u8; 16];
                for (k, d) in digits.iter_mut().enumerate() {
                    *d = b"0123456789ABCDEF"[((v >> (60 - 4 * k)) & 0xF) as usize];
                }
                f.pad_out(&[], &digits, &Spec { zero: false, precision: None, ..spec });
            }
            b'c' | b'C' => {
                let wide = f.wide_arg::<C>(size, conv == b'C');
                let raw = args.arg(pos);
                let ch: u32 = if wide { u32::from(raw as u16) } else { u32::from(raw as u8) };
                let mut units: Vec<C> = Vec::new();
                f.convert_char::<C>(ch, wide, &mut units)?;
                f.pad_units(&units, &Spec { zero: false, ..spec });
            }
            b's' | b'S' => {
                let wide = f.wide_arg::<C>(size, conv == b'S');
                let p = args.arg(pos) as usize;
                let mut units: Vec<C> = Vec::new();
                if p == 0 {
                    f.null_text(&mut units, spec.precision);
                } else if wide {
                    // SAFETY: a valid wide string (the caller's contract),
                    // read no further than the precision.
                    let s = unsafe { unit::slice_max(p as *const u16, spec.precision.unwrap_or(usize::MAX)) };
                    f.convert_str(s.iter().map(|&c| u32::from(c)), true, spec.precision, &mut units)?;
                } else {
                    // SAFETY: as above, narrow.
                    let s = unsafe { unit::slice_max(p as *const u8, spec.precision.unwrap_or(usize::MAX)) };
                    f.convert_str(s.iter().map(|&c| u32::from(c)), false, spec.precision, &mut units)?;
                }
                f.pad_units(&units, &Spec { zero: false, ..spec });
            }
            b'Z' => {
                let wide = match size {
                    Size::H | Size::Hh => false,
                    Size::L | Size::W => true,
                    _ if f.legacy() => false,
                    _ => !C::WIDE,
                };
                let p = args.arg(pos) as usize;
                let mut units: Vec<C> = Vec::new();
                let (bytes, buffer) = if p == 0 {
                    (0, 0)
                } else {
                    // SAFETY: a valid ANSI_STRING / UNICODE_STRING (winternl.h:
                    // USHORT Length, USHORT MaximumLength, the Buffer pointer
                    // at + 8).
                    unsafe { ((p as *const u16).read_unaligned(), ((p + 8) as *const usize).read_unaligned()) }
                };
                if p == 0 || buffer == 0 {
                    f.null_text(&mut units, spec.precision);
                } else if wide {
                    // SAFETY: Length bytes of the Buffer are readable.
                    let s = unsafe { core::slice::from_raw_parts(buffer as *const u16, usize::from(bytes) / 2) };
                    let s = &s[..s.len().min(spec.precision.unwrap_or(usize::MAX))];
                    f.convert_str(s.iter().map(|&c| u32::from(c)), true, spec.precision, &mut units)?;
                } else {
                    // SAFETY: as above.
                    let s = unsafe { core::slice::from_raw_parts(buffer as *const u8, usize::from(bytes)) };
                    let s = &s[..s.len().min(spec.precision.unwrap_or(usize::MAX))];
                    f.convert_str(s.iter().map(|&c| u32::from(c)), false, spec.precision, &mut units)?;
                }
                f.pad_units(&units, &Spec { zero: false, ..spec });
            }
            b'n' => {
                if !count_output {
                    return Err(Error::Invalid);
                }
                let p = args.arg(pos) as usize;
                let n = f.count as u64;
                // SAFETY: `%n`'s argument points to an integer of the size
                // its prefix names.
                unsafe {
                    match size {
                        Size::Hh => (p as *mut u8).write_unaligned(n as u8),
                        Size::H => (p as *mut u16).write_unaligned(n as u16),
                        Size::None | Size::L | Size::I32 | Size::W => (p as *mut u32).write_unaligned(n as u32),
                        _ => (p as *mut u64).write_unaligned(n),
                    }
                }
            }
            b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'a' | b'A' => {
                let v = f64::from_bits(args.arg(pos));
                f.float(v, conv, &spec);
            }
            other => {
                if f.legacy() {
                    f.put(C::ascii(other));
                } else {
                    return Err(Error::Invalid);
                }
            }
        }
    }
    Ok(f.count)
}

fn digit(c: u32) -> Option<usize> {
    if (0x30..=0x39).contains(&c) { Some((c - 0x30) as usize) } else { None }
}

struct Formatter<'a, S> {
    out: &'a mut S,
    count: usize,
    options: u64,
}

impl<S> Formatter<'_, S> {
    fn legacy(&self) -> bool {
        self.options & LEGACY_MSVCRT_COMPATIBILITY != 0
    }

    fn put<C: Unit>(&mut self, c: C)
    where
        S: Sink<C>,
    {
        self.out.put(c);
        self.count += 1;
    }

    fn put_ascii<C: Unit>(&mut self, s: &[u8])
    where
        S: Sink<C>,
    {
        for &b in s {
            self.put(C::ascii(b));
        }
    }

    fn fill<C: Unit>(&mut self, b: u8, n: usize)
    where
        S: Sink<C>,
    {
        for _ in 0..n {
            self.put(C::ascii(b));
        }
    }

    /// Whether a `c`/`s` argument is wide: `other` for `C`/`S`.
    fn wide_arg<C: Unit>(&self, size: Size, other: bool) -> bool {
        match size {
            Size::H | Size::Hh => false,
            Size::L | Size::W => true,
            _ if self.options & LEGACY_WIDE_SPECIFIERS != 0 => C::WIDE != other,
            // ISO mode: %c/%s are narrow, %C/%S wide.
            _ => other,
        }
    }

    fn null_text<C: Unit>(&self, units: &mut Vec<C>, precision: Option<usize>) {
        let n = precision.unwrap_or(usize::MAX).min(6);
        units.extend(b"(null)"[..n].iter().map(|&b| C::ascii(b)));
    }

    /// One character of the argument's width in the output's width (the
    /// "C" locale's conversions).
    fn convert_char<C: Unit>(&self, ch: u32, wide: bool, units: &mut Vec<C>) -> Result<(), Error> {
        if C::WIDE || !wide {
            units.push(C::from_u32(if !C::WIDE || wide { ch } else { u32::from(unit::c_locale_to_wide(ch as u8)) }));
            return Ok(());
        }
        match unit::c_locale_to_narrow(ch as u16) {
            Some(b) => {
                units.push(C::from_u32(u32::from(b)));
                Ok(())
            }
            None => Err(Error::Encoding),
        }
    }

    /// A string of the argument's width in the output's width, cut at
    /// `precision` output units.
    fn convert_str<C: Unit>(
        &self,
        s: impl Iterator<Item = u32>,
        wide: bool,
        precision: Option<usize>,
        units: &mut Vec<C>,
    ) -> Result<(), Error> {
        let max = precision.unwrap_or(usize::MAX);
        for ch in s {
            if units.len() >= max {
                break;
            }
            self.convert_char(ch, wide, units)?;
        }
        Ok(())
    }

    fn pad_units<C: Unit>(&mut self, units: &[C], spec: &Spec)
    where
        S: Sink<C>,
    {
        let pad = spec.width.saturating_sub(units.len());
        if !spec.left {
            self.fill::<C>(b' ', pad);
        }
        for &u in units {
            self.put(u);
        }
        if spec.left {
            self.fill::<C>(b' ', pad);
        }
    }

    /// `prefix` (sign, `0x`), then `body`, padded to the width: spaces on
    /// the left or right, or zeros after the prefix with the `0` flag.
    fn pad_out<C: Unit>(&mut self, prefix: &[u8], body: &[u8], spec: &Spec)
    where
        S: Sink<C>,
    {
        let len = prefix.len() + body.len();
        let pad = spec.width.saturating_sub(len);
        if spec.left {
            self.put_ascii::<C>(prefix);
            self.put_ascii::<C>(body);
            self.fill::<C>(b' ', pad);
        } else if spec.zero {
            self.put_ascii::<C>(prefix);
            self.fill::<C>(b'0', pad);
            self.put_ascii::<C>(body);
        } else {
            self.fill::<C>(b' ', pad);
            self.put_ascii::<C>(prefix);
            self.put_ascii::<C>(body);
        }
    }

    fn sign(neg: bool, spec: &Spec) -> &'static [u8] {
        if neg {
            b"-"
        } else if spec.plus {
            b"+"
        } else if spec.space {
            b" "
        } else {
            b""
        }
    }

    fn integer<C: Unit>(&mut self, v: u64, neg: bool, base: u64, upper: bool, spec: &Spec, signed: bool)
    where
        S: Sink<C>,
    {
        let table: &[u8; 16] = if upper { b"0123456789ABCDEF" } else { b"0123456789abcdef" };
        let mut digits: Vec<u8> = Vec::new();
        let mut x = v;
        while x != 0 {
            digits.push(table[(x % base) as usize]);
            x /= base;
        }
        let precision = spec.precision.unwrap_or(1);
        while digits.len() < precision {
            digits.push(b'0');
        }
        if spec.alt && base == 8 && digits.last() != Some(&b'0') {
            digits.push(b'0');
        }
        digits.reverse();
        let mut prefix: Vec<u8> = Vec::new();
        if signed {
            prefix.extend_from_slice(Self::sign(neg, spec));
        }
        if spec.alt && base == 16 && v != 0 {
            prefix.extend_from_slice(if upper { b"0X" } else { b"0x" });
        }
        let zero = spec.zero && spec.precision.is_none();
        self.pad_out::<C>(&prefix, &digits, &Spec { zero, ..*spec });
    }

    fn float<C: Unit>(&mut self, v: f64, conv: u8, spec: &Spec)
    where
        S: Sink<C>,
    {
        let upper = conv.is_ascii_uppercase();
        let (neg, class) = float::classify(v);
        let sign = Self::sign(neg, spec);
        let (m, e) = match class {
            Class::Finite { m, e } => (m, e),
            _ => {
                let body = self.special(class, neg, conv, spec);
                self.pad_out::<C>(sign, &body, &Spec { zero: false, ..*spec });
                return;
            }
        };
        let ties = if self.options & STANDARD_ROUNDING != 0 { Ties::Even } else { Ties::Away };
        let lower = conv.to_ascii_lowercase();
        let body = if lower == b'a' {
            hex_float(m, e, spec.precision, spec.alt, upper, ties)
        } else {
            let mut d = float::exact(m, e);
            if self.legacy() && d.digits.len() > 17 {
                d.round(17, Ties::Away);
            }
            let three = self.options & LEGACY_THREE_DIGIT_EXPONENTS != 0;
            let p = spec.precision.unwrap_or(6);
            match lower {
                b'e' => exp_form(d, p, spec.alt, upper, three, ties),
                b'f' => fixed_form(d, p, spec.alt, ties),
                _ => general_form(d, p, spec.alt, upper, three, ties),
            }
        };
        // The locale's decimal point (ADR 0094), carried in the option word.
        let point = crate::locale::point_of(self.options);
        let body =
            if point == b'.' { body } else { body.into_iter().map(|c| if c == b'.' { point } else { c }).collect() };
        let mut prefix: Vec<u8> = Vec::from(sign);
        let body = if lower == b'a' {
            prefix.extend_from_slice(&body[..2]);
            body[2..].to_vec()
        } else {
            body
        };
        self.pad_out::<C>(&prefix, &body, spec);
    }

    /// The text of an infinity or NaN, without the sign.
    fn special(&self, class: Class, neg: bool, conv: u8, spec: &Spec) -> Vec<u8> {
        let upper = conv.is_ascii_uppercase();
        if self.legacy() {
            let word: &[u8] = match class {
                Class::Infinite => b"#INF",
                Class::Nan { quiet: true, payload: 0 } if neg => b"#IND",
                Class::Nan { quiet: true, .. } => b"#QNAN",
                _ => b"#SNAN",
            };
            let lower = conv.to_ascii_lowercase();
            let p = match lower {
                b'g' => 0,
                _ => spec.precision.unwrap_or(6),
            };
            let mut out: Vec<u8> = Vec::from(&b"1"[..]);
            if lower == b'g' {
                out.push(b'.');
                out.extend_from_slice(word);
            } else if p > 0 {
                out.push(b'.');
                let mut frac: Vec<u8> = word.to_vec();
                frac.resize(p.max(frac.len()), b'0');
                frac.truncate(p);
                out.extend_from_slice(&frac);
            }
            if lower == b'e' {
                out.extend_from_slice(if upper { b"E+000" } else { b"e+000" });
            }
            return out;
        }
        let text: &[u8] = match class {
            Class::Infinite => b"inf",
            Class::Nan { quiet: true, payload: 0 } if neg => b"nan(ind)",
            Class::Nan { quiet: true, .. } => b"nan",
            _ => b"nan(snan)",
        };
        if upper { text.to_ascii_uppercase() } else { text.to_vec() }
    }
}

/// The `e` style: `d.ddde±xx`.
fn exp_form(mut d: Digits, p: usize, alt: bool, upper: bool, three: bool, ties: Ties) -> Vec<u8> {
    d.round(p as i32 + 1, ties);
    let x = if d.is_zero() { 0 } else { d.exp - 1 };
    let mut out = Vec::new();
    out.push(b'0' + d.at(0));
    if p > 0 || alt {
        out.push(b'.');
    }
    for k in 0..p {
        out.push(b'0' + d.at(k as i32 + 1));
    }
    out.push(if upper { b'E' } else { b'e' });
    out.push(if x < 0 { b'-' } else { b'+' });
    let mut digits: Vec<u8> = alloc::format!("{}", x.unsigned_abs()).into_bytes();
    let min = if three { 3 } else { 2 };
    while digits.len() < min {
        digits.insert(0, b'0');
    }
    out.extend_from_slice(&digits);
    out
}

/// The `f` style: `ddd.ddd`.
fn fixed_form(mut d: Digits, p: usize, alt: bool, ties: Ties) -> Vec<u8> {
    let keep = d.exp.saturating_add(p as i32);
    d.round(keep, ties);
    let mut out = Vec::new();
    if d.is_zero() || d.exp <= 0 {
        out.push(b'0');
    } else {
        for k in 0..d.exp {
            out.push(b'0' + d.at(k));
        }
    }
    if p > 0 || alt {
        out.push(b'.');
    }
    for k in 0..p as i32 {
        out.push(if d.is_zero() { b'0' } else { b'0' + d.at(d.exp + k) });
    }
    out
}

/// The `g` style (C11 7.21.6.1).
fn general_form(mut d: Digits, p: usize, alt: bool, upper: bool, three: bool, ties: Ties) -> Vec<u8> {
    let p = if p == 0 { 1 } else { p };
    d.round(p as i32, ties);
    let x = if d.is_zero() { 0 } else { d.exp - 1 };
    let mut out = if (p as i32) > x && x >= -4 {
        fixed_form(d, (p as i32 - 1 - x) as usize, alt, ties)
    } else {
        exp_form(d, p - 1, alt, upper, three, ties)
    };
    if !alt {
        let exp_at = out.iter().position(|&c| c == b'e' || c == b'E').unwrap_or(out.len());
        let (mant, exp) = out.split_at(exp_at);
        let mut mant = mant.to_vec();
        if mant.contains(&b'.') {
            while mant.last() == Some(&b'0') {
                mant.pop();
            }
            if mant.last() == Some(&b'.') {
                mant.pop();
            }
        }
        mant.extend_from_slice(exp);
        out = mant;
    }
    out
}

/// The `a` style: `0xh.hhhp±d` (the `0x` included, the sign not).
fn hex_float(m: u64, e: i32, precision: Option<usize>, alt: bool, upper: bool, ties: Ties) -> Vec<u8> {
    let (mut lead, mut frac, exp) = if m == 0 {
        (0u64, 0u64, 0i32)
    } else if m >> 52 != 0 {
        (1, m & ((1 << 52) - 1), e + 52)
    } else {
        (0, m, -1022)
    };
    let p = precision.unwrap_or(13);
    if p < 13 {
        // Round lead.frac to p hexadecimal digits; the parity of a tie is
        // that of the last kept digit (the leading one when p is 0).
        let drop = 4 * (13 - p) as u32;
        let all = (lead << 52) | frac;
        let rem = all & ((1u64 << drop) - 1);
        let half = 1u64 << (drop - 1);
        let mut kept = all >> drop;
        if rem > half || (rem == half && (ties == Ties::Away || kept & 1 == 1)) {
            kept += 1;
        }
        lead = kept >> (4 * p);
        frac = kept & ((1u64 << (4 * p)) - 1);
    }
    let table: &[u8; 16] = if upper { b"0123456789ABCDEF" } else { b"0123456789abcdef" };
    let mut out: Vec<u8> = Vec::from(if upper { &b"0X"[..] } else { &b"0x"[..] });
    out.push(table[lead as usize]);
    if p > 0 || alt {
        out.push(b'.');
    }
    let shown = p.min(13);
    for k in 0..shown {
        out.push(table[((frac >> (4 * (shown - 1 - k))) & 0xF) as usize]);
    }
    out.extend(core::iter::repeat_n(b'0', p - shown));
    out.push(if upper { b'P' } else { b'p' });
    out.push(if exp < 0 { b'-' } else { b'+' });
    out.extend_from_slice(alloc::format!("{}", exp.unsigned_abs()).as_bytes());
    out
}

/// What a `sprintf`-style function does with its buffer once the output
/// (`total` units) is known; the sink kept at most `capacity` units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Finish {
    /// Where to put the NUL, if anywhere.
    pub nul_at: Option<usize>,
    /// The function's return value.
    pub result: i32,
    /// The buffer is too small for a function that must not truncate: the
    /// invalid parameter handler, `ERANGE`, the buffer emptied.
    pub invalid: bool,
}

/// `__stdio_common_vsprintf` / `vswprintf`'s buffer rules by option
/// (Learn: `_snprintf`, `snprintf`, `vsprintf`): with
/// `STANDARD_SNPRINTF_BEHAVIOR` the output is cut to `capacity − 1` units,
/// always terminated, and the full length returned; with
/// `LEGACY_VSPRINTF_NULL_TERMINATION` an output of exactly `capacity` units
/// is not terminated and a longer one returns −1 unterminated; with neither
/// a longer output is cut, terminated, and −1 returned (Poly's reading, to
/// be confirmed). A null buffer of capacity 0 only counts.
pub fn finish_sprintf(options: u64, capacity: usize, total: usize) -> Finish {
    let result = i32::try_from(total).unwrap_or(-1);
    if capacity == 0 {
        let result = if options & STANDARD_SNPRINTF_BEHAVIOR != 0 || options & LEGACY_VSPRINTF_NULL_TERMINATION != 0 {
            result
        } else {
            -1
        };
        return Finish { nul_at: None, result, invalid: false };
    }
    if total < capacity {
        return Finish { nul_at: Some(total), result, invalid: false };
    }
    if options & STANDARD_SNPRINTF_BEHAVIOR != 0 {
        Finish { nul_at: Some(capacity - 1), result, invalid: false }
    } else if options & LEGACY_VSPRINTF_NULL_TERMINATION != 0 {
        Finish { nul_at: None, result: if total == capacity { result } else { -1 }, invalid: false }
    } else {
        Finish { nul_at: Some(capacity - 1), result: -1, invalid: false }
    }
}

/// `__stdio_common_vsprintf_s` (`vsprintf_s`, `sprintf_s`): the output and
/// its NUL must fit, or it is an invalid parameter (Learn: `sprintf_s`).
pub fn finish_sprintf_s(capacity: usize, total: usize) -> Finish {
    if total < capacity {
        Finish { nul_at: Some(total), result: i32::try_from(total).unwrap_or(-1), invalid: false }
    } else {
        Finish { nul_at: if capacity > 0 { Some(0) } else { None }, result: -1, invalid: true }
    }
}

/// `_TRUNCATE` (`corecrt.h`).
pub const TRUNCATE: usize = usize::MAX;

/// `__stdio_common_vsnprintf_s` (`_vsnprintf_s`): at most `count` units
/// (or as many as fit, with `_TRUNCATE`) and a NUL (Learn: `_snprintf_s`).
/// The units the sink may keep are `min(count, capacity)`.
pub fn finish_snprintf_s(capacity: usize, count: usize, total: usize) -> Finish {
    let ok = |n: usize| Finish { nul_at: Some(n), result: i32::try_from(n).unwrap_or(-1), invalid: false };
    if capacity == 0 {
        return Finish { nul_at: None, result: -1, invalid: true };
    }
    if count == TRUNCATE {
        if total < capacity {
            return ok(total);
        }
        return Finish { nul_at: Some(capacity - 1), result: -1, invalid: false };
    }
    let kept = total.min(count);
    if kept >= capacity {
        return Finish { nul_at: Some(0), result: -1, invalid: true };
    }
    if total <= count { ok(total) } else { Finish { nul_at: Some(count), result: -1, invalid: false } }
}

/// A sink into a caller's buffer that keeps at most `capacity` units and
/// counts the rest.
#[derive(Debug)]
pub struct Buffer<C> {
    pub ptr: *mut C,
    pub capacity: usize,
    pub kept: usize,
}

impl<C: Unit> Sink<C> for Buffer<C> {
    fn put(&mut self, c: C) {
        if self.kept < self.capacity {
            // SAFETY: the buffer holds `capacity` units (the caller's
            // contract when it made the sink).
            unsafe { self.ptr.add(self.kept).write(c) };
            self.kept += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec;

    const UCRT: u64 = LEGACY_WIDE_SPECIFIERS | STANDARD_ROUNDING;

    fn fmt_opts(f: &str, args: &[u64], options: u64) -> Result<String, Error> {
        let mut out: Vec<u8> = Vec::new();
        // SAFETY: the tests pass arguments matching their formats.
        let n = unsafe { format(f.as_bytes(), &mut Slots(args), &mut out, options, true) }?;
        assert_eq!(n, out.len());
        // Narrow output is single bytes: show them as Latin-1.
        Ok(out.iter().map(|&b| char::from(b)).collect())
    }

    fn p(f: &str, args: &[u64]) -> String {
        fmt_opts(f, args, UCRT).unwrap()
    }

    fn wp(f: &str, args: &[u64], options: u64) -> Result<String, Error> {
        let fw: Vec<u16> = f.encode_utf16().collect();
        let mut out: Vec<u16> = Vec::new();
        // SAFETY: as above.
        unsafe { format(&fw, &mut Slots(args), &mut out, options, true) }?;
        Ok(String::from_utf16(&out).unwrap())
    }

    fn d(v: f64) -> u64 {
        v.to_bits()
    }

    #[test]
    fn integers() {
        assert_eq!(p("%d %i %u", &[(-5i64) as u64, 7, (-1i64) as u64]), "-5 7 4294967295");
        assert_eq!(p("[%5d|%-5d|%05d|%+d|% d]", &[42, 42, 42, 42, 42]), "[   42|42   |00042|+42| 42]");
        assert_eq!(p("%.3d %.0d|%5.0d|", &[7, 0, 0]), "007 |     |");
        assert_eq!(p("%x %X %#x %#X %#x %o %#o %#o", &[255, 255, 255, 255, 0, 8, 8, 0]), "ff FF 0xff 0XFF 0 10 010 0");
        assert_eq!(p("%#08x|%-#8x|", &[255, 255]), "0x0000ff|0xff    |");
        assert_eq!(p("%05.3d", &[7]), "  007");
        assert_eq!(p("%hd %hhd %hu %hhu", &[0x18000, 0x1FF, 0x1FFFF, 0x1FF]), "-32768 -1 65535 255");
        let big = 0x8000_0000_0000_0000u64;
        assert_eq!(p("%lld %I64d %Id %zd %jd", &[big, big, big, big, big]), {
            let s = "-9223372036854775808";
            alloc::format!("{s} {s} {s} {s} {s}")
        });
        assert_eq!(p("%I32d %ld %llu %I64x %Iu", &[u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX]), {
            alloc::format!("-1 -1 {} ffffffffffffffff {}", u64::MAX, u64::MAX)
        });
        assert_eq!(p("%*d|%-*d|%.*d", &[4, 1, 3, 2, 3, 5]), "   1|2  |005");
        assert_eq!(p("%*d", &[(-4i64) as u64, 1]), "1   ");
        assert_eq!(p("%.*d", &[(-1i64) as u64, 0]), "0");
        assert_eq!(p("%+u", &[1]), "1");
        assert_eq!(p("100%%", &[]), "100%");
    }

    #[test]
    fn pointers() {
        assert_eq!(p("%p", &[0x12FF7C]), "000000000012FF7C");
        assert_eq!(p("[%20p]", &[1]), "[    0000000000000001]");
    }

    #[test]
    fn characters_and_strings() {
        let s = b"hello\0";
        let w: Vec<u16> = "w\u{e9}de\0".encode_utf16().collect();
        let sp = s.as_ptr() as u64;
        let wp_ = w.as_ptr() as u64;
        assert_eq!(p("%s|%.3s|%8s|%-8s|", &[sp, sp, sp, sp]), "hello|hel|   hello|hello   |");
        assert_eq!(p("%c%c%5c%-3c|", &[b'a' as u64, 0x141, b'b' as u64, b'c' as u64]), "aA    bc  |");
        assert_eq!(p("%S|%ls|%ws|%C|%lc", &[wp_, wp_, wp_, 0xE9, 0x41]), "w\u{e9}de|w\u{e9}de|w\u{e9}de|\u{e9}|A");
        assert_eq!(p("%s|%.2s|%S", &[0, 0, 0]), "(null)|(n|(null)");
        // A wide character with no "C" locale form is an encoding error.
        let euro: Vec<u16> = "a\u{20ac}\0".encode_utf16().collect();
        assert_eq!(fmt_opts("%ls", &[euro.as_ptr() as u64], UCRT), Err(Error::Encoding));
        assert_eq!(fmt_opts("%.1ls", &[euro.as_ptr() as u64], UCRT), Ok(String::from("a")));
        assert_eq!(fmt_opts("%C", &[0x20AC], UCRT), Err(Error::Encoding));
    }

    #[test]
    fn wide_functions_and_the_legacy_specifiers() {
        let s = b"narrow\0";
        let w: Vec<u16> = "wide\u{20ac}\0".encode_utf16().collect();
        let (sp, wp_) = (s.as_ptr() as u64, w.as_ptr() as u64);
        // Legacy (the default): %s is the function's own width.
        assert_eq!(
            wp("%s|%S|%hs|%ls|%c|%C", &[wp_, sp, sp, wp_, 0x20AC, b'n' as u64], UCRT).unwrap(),
            "wide\u{20ac}|narrow|narrow|wide\u{20ac}|\u{20ac}|n"
        );
        // ISO: %s and %c are narrow.
        assert_eq!(
            wp("%s|%ls|%c|%S", &[sp, wp_, b'x' as u64, wp_], STANDARD_ROUNDING).unwrap(),
            "narrow|wide\u{20ac}|x|wide\u{20ac}"
        );
        assert_eq!(wp("%s %d", &[0, 5], UCRT).unwrap(), "(null) 5");
        // A narrow byte is widened by the "C" locale (Latin-1).
        let latin = [0xE9u8, 0];
        assert_eq!(wp("%hs", &[latin.as_ptr() as u64], UCRT).unwrap(), "\u{e9}");
    }

    #[test]
    fn counted_strings() {
        #[repr(C)]
        struct Counted {
            length: u16,
            maximum: u16,
            buffer: *const u8,
        }
        let a = b"ansi text";
        let u: Vec<u16> = "unicode".encode_utf16().collect();
        let ansi = Counted { length: 4, maximum: 9, buffer: a.as_ptr() };
        let uni = Counted { length: 6, maximum: 14, buffer: u.as_ptr() as *const u8 };
        let (ap, up) = (&ansi as *const Counted as u64, &uni as *const Counted as u64);
        // The UCRT: %Z in a narrow function is a UNICODE_STRING, %hZ ANSI.
        assert_eq!(p("%Z|%hZ|%wZ|%lZ", &[up, ap, up, up]), "uni|ansi|uni|uni");
        assert_eq!(wp("%Z|%wZ", &[ap, up], UCRT).unwrap(), "ansi|uni");
        // msvcrt: %Z is ANSI.
        assert_eq!(fmt_opts("%Z|%wZ", &[ap, up], MSVCRT_OPTIONS).unwrap(), "ansi|uni");
        assert_eq!(p("%Z", &[0]), "(null)");
        assert_eq!(p("[%6.2hZ]", &[ap]), "[    an]");
    }

    #[test]
    fn floats_fixed_exponent_general() {
        assert_eq!(p("%f %e %g", &[d(1.23456), d(1.23456), d(1.23456)]), "1.234560 1.234560e+00 1.23456");
        assert_eq!(p("%.2f %.0f %.0f %#.0f", &[d(2.675), d(0.5), d(1.5), d(2.0)]), "2.67 0 2 2.");
        assert_eq!(p("%.1f %.1f", &[d(0.25), d(0.35)]), "0.2 0.3");
        assert_eq!(p("%.0f %.0f %.0f", &[d(2.5), d(3.5), d(-2.5)]), "2 4 -2");
        assert_eq!(p("%E %.3e %e", &[d(12345.678), d(0.0), d(1e-300)]), "1.234568E+04 0.000e+00 1.000000e-300");
        assert_eq!(
            p("%g %g %g %g %g", &[d(100000.0), d(1000000.0), d(0.0001), d(0.00001), d(0.0)]),
            "100000 1e+06 0.0001 1e-05 0"
        );
        assert_eq!(
            p("%G %#g %.3g %.10g", &[d(1e-10), d(1.5), d(1234.5), d(1.0 / 3.0)]),
            "1E-10 1.50000 1.23e+03 0.3333333333"
        );
        assert_eq!(p("%.0g %g", &[d(25.0), d(123456789.0)]), "2e+01 1.23457e+08");
        assert_eq!(
            p("%+.2f|% .2f|%08.2f|%-8.2f|%+08.2f", &[d(1.0), d(1.0), d(-1.5), d(1.5), d(1.5)]),
            "+1.00| 1.00|-0001.50|1.50    |+0001.50"
        );
        assert_eq!(p("%f %.3f", &[d(-0.0), d(1e-10)]), "-0.000000 0.000");
        assert_eq!(p("%.20f", &[d(0.1)]), "0.10000000000000000555");
        assert_eq!(p("%.0f", &[d(1e22)]), "10000000000000000000000");
        assert_eq!(p("%f", &[d(1.7976931348623157e308)]).len(), 309 + 7);
        assert_eq!(p("%.3e", &[d(9.9996)]), "1.000e+01");
        assert_eq!(p("%.1f", &[d(9.96)]), "10.0");
        assert_eq!(p("%Lf %lf", &[d(1.5), d(2.5)]), "1.500000 2.500000");
    }

    #[test]
    fn rounding_of_exact_ties_by_option() {
        assert_eq!(fmt_opts("%.0f %.0f %.1f", &[d(1.5), d(2.5), d(0.125)], 0).unwrap(), "2 3 0.1");
        assert_eq!(fmt_opts("%.0f %.0f %.2f", &[d(1.5), d(2.5), d(0.125)], STANDARD_ROUNDING).unwrap(), "2 2 0.12");
        // Not a tie: 2.35 is 2.35000000000000008882 in binary.
        assert_eq!(fmt_opts("%.1f", &[d(2.35)], STANDARD_ROUNDING).unwrap(), "2.4");
    }

    #[test]
    fn infinities_and_nans() {
        let ind = f64::from_bits(0xFFF8_0000_0000_0000);
        let qnan = f64::from_bits(0x7FF8_0000_0000_0000);
        let snan = f64::from_bits(0x7FF0_0000_0000_0001);
        assert_eq!(
            p("%f %e %g %F", &[d(f64::INFINITY), d(f64::NEG_INFINITY), d(qnan), d(f64::INFINITY)]),
            "inf -inf nan INF"
        );
        assert_eq!(p("%f %G %f", &[d(ind), d(ind), d(snan)]), "-nan(ind) -NAN(IND) nan(snan)");
        assert_eq!(p("%08f|%-6f|%+f", &[d(f64::INFINITY), d(f64::INFINITY), d(f64::INFINITY)]), "     inf|inf   |+inf");
        let legacy = MSVCRT_OPTIONS;
        assert_eq!(
            fmt_opts("%f %f %e", &[d(f64::INFINITY), d(ind), d(f64::NEG_INFINITY)], legacy).unwrap(),
            "1.#INF00 -1.#IND00 -1.#INF00e+000"
        );
        assert_eq!(
            fmt_opts("%g %.2f %f", &[d(qnan), d(f64::INFINITY), d(snan)], legacy).unwrap(),
            "1.#QNAN 1.#I 1.#SNAN0"
        );
    }

    #[test]
    fn the_legacy_runtime() {
        let legacy = MSVCRT_OPTIONS;
        assert_eq!(fmt_opts("%e %E", &[d(1.0), d(1e-5)], legacy).unwrap(), "1.000000e+000 1.000000E-005");
        assert_eq!(fmt_opts("%g", &[d(1e10)], legacy).unwrap(), "1e+010");
        assert_eq!(fmt_opts("%.20f", &[d(0.1)], legacy).unwrap(), "0.10000000000000001000");
        assert_eq!(fmt_opts("%y|", &[], legacy).unwrap(), "y|");
        assert_eq!(fmt_opts("%y", &[], UCRT), Err(Error::Invalid));
        assert_eq!(fmt_opts("abc%", &[], UCRT), Err(Error::Invalid));
        assert_eq!(fmt_opts("abc%", &[], legacy).unwrap(), "abc");
    }

    #[test]
    fn hexadecimal_floats() {
        assert_eq!(p("%a %A", &[d(1.0), d(-2.5)]), "0x1.0000000000000p+0 -0X1.4000000000000P+1");
        assert_eq!(p("%.1a %.0a %#.0a", &[d(1.0), d(1.5), d(1.0)]), "0x1.0p+0 0x2p+0 0x1.p+0");
        assert_eq!(p("%a", &[d(0.0)]), "0x0.0000000000000p+0");
        assert_eq!(p("%a", &[d(f64::from_bits(1))]), "0x0.0000000000001p-1022");
        assert_eq!(p("%.15a", &[d(0.1)]), "0x1.999999999999a00p-4");
        assert_eq!(p("%.3a", &[d(0.1)]), "0x1.99ap-4");
        assert_eq!(p("%12.1a|%-12.1a|%012.1a", &[d(1.0), d(1.0), d(1.0)]), "    0x1.0p+0|0x1.0p+0    |0x00001.0p+0");
    }

    #[test]
    fn count_output_needs_enabling() {
        let mut n: i32 = -1;
        let mut out: Vec<u8> = Vec::new();
        let args = [(&mut n as *mut i32) as u64];
        // SAFETY: a valid int pointer.
        let r = unsafe { format(b"abc%n", &mut Slots(&args), &mut out, UCRT, false) };
        assert_eq!(r, Err(Error::Invalid));
        // SAFETY: as above.
        let r = unsafe { format(b"abcd%nx", &mut Slots(&args), &mut out, UCRT, true) };
        assert_eq!(r, Ok(5));
        assert_eq!(n, 4);
    }

    #[test]
    fn buffer_rules() {
        let std = STANDARD_SNPRINTF_BEHAVIOR;
        let legacy = LEGACY_VSPRINTF_NULL_TERMINATION;
        assert_eq!(finish_sprintf(std, 10, 5), Finish { nul_at: Some(5), result: 5, invalid: false });
        assert_eq!(finish_sprintf(std, 5, 5), Finish { nul_at: Some(4), result: 5, invalid: false });
        assert_eq!(finish_sprintf(std, 0, 5), Finish { nul_at: None, result: 5, invalid: false });
        assert_eq!(finish_sprintf(legacy, 5, 4), Finish { nul_at: Some(4), result: 4, invalid: false });
        assert_eq!(finish_sprintf(legacy, 5, 5), Finish { nul_at: None, result: 5, invalid: false });
        assert_eq!(finish_sprintf(legacy, 5, 6), Finish { nul_at: None, result: -1, invalid: false });
        assert_eq!(finish_sprintf(legacy, 0, 6), Finish { nul_at: None, result: 6, invalid: false });
        assert_eq!(finish_sprintf(0, 5, 6), Finish { nul_at: Some(4), result: -1, invalid: false });
        assert_eq!(finish_sprintf_s(5, 4), Finish { nul_at: Some(4), result: 4, invalid: false });
        assert_eq!(finish_sprintf_s(5, 5), Finish { nul_at: Some(0), result: -1, invalid: true });
        assert_eq!(finish_snprintf_s(10, TRUNCATE, 12), Finish { nul_at: Some(9), result: -1, invalid: false });
        assert_eq!(finish_snprintf_s(10, TRUNCATE, 9), Finish { nul_at: Some(9), result: 9, invalid: false });
        assert_eq!(finish_snprintf_s(10, 3, 12), Finish { nul_at: Some(3), result: -1, invalid: false });
        assert_eq!(finish_snprintf_s(10, 30, 12), Finish { nul_at: Some(0), result: -1, invalid: true });
        assert_eq!(finish_snprintf_s(10, 5, 5), Finish { nul_at: Some(5), result: 5, invalid: false });
    }

    #[test]
    fn buffer_sink_keeps_its_capacity() {
        let mut buf = [0xAAu8; 4];
        let mut sink = Buffer { ptr: buf.as_mut_ptr(), capacity: 3, kept: 0 };
        // SAFETY: no arguments.
        let n = unsafe { format(b"abcdef", &mut Slots(&[]), &mut sink, UCRT, false) }.unwrap();
        assert_eq!((n, sink.kept, buf), (6, 3, [b'a', b'b', b'c', 0xAA]));
        let _ = vec![0u8];
    }

    #[test]
    fn positional_parameters() {
        let pp = |f: &str, args: &[u64]| fmt_opts(f, args, UCRT | POSITIONAL);
        let (nov, ten) = (c"November".as_ptr() as u64, c"10".as_ptr() as u64);
        // Learn's examples ("printf_p Positional Parameters").
        assert_eq!(pp("%1$s %2$s", &[nov, ten]).unwrap(), "November 10");
        assert_eq!(pp("%2$s %1$s", &[nov, ten]).unwrap(), "10 November");
        assert_eq!(pp("%1$d times %1$d is %2$d", &[10, 100]).unwrap(), "10 times 10 is 100");
        assert_eq!(pp("%d %d %d", &[1, 2, 3]).unwrap(), "1 2 3");
        assert_eq!(pp("%3$d %1$d %2$d", &[1, 2, 3]).unwrap(), "3 1 2");
        let hello = c"Hello".as_ptr() as u64;
        assert_eq!(pp("%1$*2$s|", &[hello, 10]).unwrap(), "     Hello|");
        assert_eq!(pp("%2$*1$s|", &[10, hello]).unwrap(), "     Hello|");
        assert_eq!(
            pp("%3$*5$f %2$.*4$f %1$*4$.*5$f", &[d(0.1), d(2.22), d(333.3333), 2, 3]).unwrap(),
            "333.333300 2.22 0.100"
        );
        assert_eq!(pp("%1$s%%%1$s", &[ten]).unwrap(), "10%10");
        assert_eq!(pp("%1$-5d|%1$05I64x", &[42]).unwrap(), "42   |0002a");
        // Errors: mixed, a gap, 0, above _ARGMAX.
        assert_eq!(pp("%1$d %d", &[1, 2]), Err(Error::Invalid));
        assert_eq!(pp("%1$*d", &[1, 2]), Err(Error::Invalid));
        assert_eq!(pp("%1$d %3$d", &[1, 2, 3]), Err(Error::Invalid));
        assert_eq!(pp("%0$d", &[1]), Err(Error::Invalid));
        assert_eq!(pp("%101$d", &[1]), Err(Error::Invalid));
        // Without the option `$` is no positional syntax (as before).
        assert_eq!(fmt_opts("%1$d", &[1], UCRT), Err(Error::Invalid));
        assert_eq!(positional(b"%5d %s".as_slice()), Ok(None));
        let w: Vec<u16> = "%2$s".encode_utf16().collect();
        assert_eq!(positional(&w), Err(Error::Invalid));
        assert_eq!(wp("%2$hs-%1$d", &[7, ten], LEGACY_WIDE_SPECIFIERS | POSITIONAL), Ok("10-7".into()));
    }
}
