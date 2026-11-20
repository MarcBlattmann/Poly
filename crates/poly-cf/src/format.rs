//! The format engine of `CFStringCreateWithFormat` and its relatives, as
//! the spec sheet `CFStringCreateWithFormat` describes it: `printf`'s
//! conversions, flags, widths, precisions and sizes, plus `%@` (an
//! object's text), `%C` (a `UniChar`), `%S` (a NUL-terminated `UniChar`
//! string), `%s` read as Mac Roman, positional arguments (`%2$d`) of every
//! kind, an unknown conversion letter written as itself, and a lone `%` at
//! the end dropped.
//!
//! Integer and floating-point conversions go through the `printf` engine of
//! `poly-libsystem` one at a time, so they are formatted exactly as Poly's
//! `printf` formats them.

use crate::encoding;
use alloc::vec::Vec;
use poly_libsystem::printf::{self, Args, Slots};

/// What the engine asks of its caller.
pub trait Host {
    /// Append the text of the object `obj` (a non-null `CFTypeRef`) for `%@`.
    fn describe(&mut self, obj: u64, out: &mut Vec<u16>);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Int,
    Double,
    LongDouble,
}

#[derive(Clone, Copy, Debug)]
enum Amount {
    None,
    Fixed(usize),
    /// From the argument at this position (0-based).
    Arg(usize),
}

#[derive(Clone, Copy, Debug)]
struct Conversion {
    /// The flags as written (`-+ #0'`).
    flags: [u8; 6],
    nflags: usize,
    width: Amount,
    precision: Amount,
    /// The size letters as written (`hh`, `l`, `L`, ...).
    size: [u8; 2],
    nsize: usize,
    conv: u16,
    /// The value's argument position, for the conversions that take one.
    arg: Option<usize>,
}

enum Piece {
    Text(usize, usize),
    Conv(Conversion),
}

fn kind_of(conv: u16, size: &[u8]) -> Option<Kind> {
    let c = u8::try_from(conv).ok()?;
    match c {
        b'd' | b'i' | b'u' | b'o' | b'x' | b'X' | b'p' | b'c' | b'C' | b's' | b'S' | b'@' | b'D' | b'O' | b'U'
        | b'n' => Some(Kind::Int),
        b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A' => {
            Some(if size == b"L" { Kind::LongDouble } else { Kind::Double })
        }
        _ => None,
    }
}

fn digits(fmt: &[u16], i: &mut usize) -> Option<usize> {
    let start = *i;
    let mut n = 0usize;
    while let Some(&c) = fmt.get(*i) {
        if !(0x30..=0x39).contains(&c) {
            break;
        }
        n = n.saturating_mul(10).saturating_add(usize::from(c - 0x30));
        *i += 1;
    }
    (*i > start).then_some(n)
}

/// `n$` at `i`: the 0-based position, consumed; otherwise nothing moves.
fn position(fmt: &[u16], i: &mut usize) -> Option<usize> {
    let mut j = *i;
    let n = digits(fmt, &mut j)?;
    if n == 0 || fmt.get(j) != Some(&u16::from(b'$')) {
        return None;
    }
    *i = j + 1;
    Some(n - 1)
}

/// Split the format into text and conversions, giving each argument a
/// position (sequential where the format names none).
fn parse(fmt: &[u16]) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut next_arg = 0usize;
    let mut i = 0;
    let pct = u16::from(b'%');
    while i < fmt.len() {
        let start = i;
        while i < fmt.len() && fmt[i] != pct {
            i += 1;
        }
        if i > start {
            pieces.push(Piece::Text(start, i));
        }
        if i >= fmt.len() {
            break;
        }
        i += 1;
        if i >= fmt.len() {
            break; // a lone '%' at the end is dropped
        }
        if fmt[i] == pct {
            pieces.push(Piece::Text(i, i + 1));
            i += 1;
            continue;
        }
        let explicit = position(fmt, &mut i);
        let mut c = Conversion {
            flags: [0; 6],
            nflags: 0,
            width: Amount::None,
            precision: Amount::None,
            size: [0; 2],
            nsize: 0,
            conv: 0,
            arg: None,
        };
        while let Some(&f) = fmt.get(i) {
            let Ok(b) = u8::try_from(f) else { break };
            if !b"-+ #0'".contains(&b) {
                break;
            }
            if c.nflags < c.flags.len() && !c.flags[..c.nflags].contains(&b) {
                c.flags[c.nflags] = b;
                c.nflags += 1;
            }
            i += 1;
        }
        let star = u16::from(b'*');
        let mut amount = |i: &mut usize| -> Amount {
            if fmt.get(*i) == Some(&star) {
                *i += 1;
                let p = position(fmt, i).unwrap_or_else(|| {
                    next_arg += 1;
                    next_arg - 1
                });
                Amount::Arg(p)
            } else {
                digits(fmt, i).map_or(Amount::None, Amount::Fixed)
            }
        };
        c.width = amount(&mut i);
        if fmt.get(i) == Some(&u16::from(b'.')) {
            i += 1;
            c.precision = match amount(&mut i) {
                Amount::None => Amount::Fixed(0),
                a => a,
            };
        }
        while let Some(&s) = fmt.get(i) {
            let Ok(b) = u8::try_from(s) else { break };
            if !b"hlqLjzt".contains(&b) || c.nsize == 2 {
                break;
            }
            c.size[c.nsize] = if b == b'q' { b'l' } else { b };
            if b == b'q' {
                c.size[1] = b'l';
                c.nsize = 1;
            }
            c.nsize += 1;
            i += 1;
        }
        let Some(&conv) = fmt.get(i) else { break };
        i += 1;
        c.conv = conv;
        if kind_of(conv, &c.size[..c.nsize]).is_some() {
            c.arg = Some(explicit.unwrap_or_else(|| {
                next_arg += 1;
                next_arg - 1
            }));
        }
        pieces.push(Piece::Conv(c));
    }
    pieces
}

#[derive(Clone, Copy, Debug)]
enum Value {
    Int(u64),
    Double(f64),
}

impl Value {
    fn int(self) -> u64 {
        match self {
            Value::Int(v) => v,
            Value::Double(d) => d as i64 as u64,
        }
    }
    fn double(self) -> f64 {
        match self {
            Value::Double(d) => d,
            Value::Int(v) => f64::from_bits(v),
        }
    }
}

/// Read every argument the pieces name, by position (an unnamed gap is
/// read as an integer, as the ABI lays out arguments of unknown type).
fn read_args<A: Args>(pieces: &[Piece], args: &mut A) -> Vec<Value> {
    let mut kinds: Vec<Option<Kind>> = Vec::new();
    let mut note = |p: usize, k: Kind| {
        if kinds.len() <= p {
            kinds.resize(p + 1, None);
        }
        kinds[p].get_or_insert(k);
    };
    for piece in pieces {
        if let Piece::Conv(c) = piece {
            for a in [c.width, c.precision] {
                if let Amount::Arg(p) = a {
                    note(p, Kind::Int);
                }
            }
            if let (Some(p), Some(k)) = (c.arg, kind_of(c.conv, &c.size[..c.nsize])) {
                note(p, k);
            }
        }
    }
    kinds
        .iter()
        .map(|k| match k.unwrap_or(Kind::Int) {
            Kind::Int => Value::Int(args.int()),
            Kind::Double => Value::Double(args.double()),
            Kind::LongDouble => {
                let (m, se) = args.long_double();
                Value::Double(printf::x87_to_f64(m, se))
            }
        })
        .collect()
}

struct Units<'a>(&'a mut Vec<u16>);

impl printf::Sink for Units<'_> {
    fn put(&mut self, bytes: &[u8]) {
        self.0.extend(bytes.iter().map(|&b| u16::from(b)));
    }
}

/// Pad `body` to `width` (spaces; on the right with `-`).
fn padded(out: &mut Vec<u16>, body: &[u16], width: usize, left: bool) {
    let pad = width.saturating_sub(body.len());
    if !left {
        out.extend(core::iter::repeat_n(u16::from(b' '), pad));
    }
    out.extend_from_slice(body);
    if left {
        out.extend(core::iter::repeat_n(u16::from(b' '), pad));
    }
}

/// Format `fmt` with `args` into UTF-16 units.
///
/// # Safety
/// The arguments are those the format describes: `%s` a NUL-terminated
/// byte string or null, `%S` a NUL-terminated `UniChar` string or null,
/// `%@` an object `host` can describe or null.
pub unsafe fn format<A: Args, H: Host>(fmt: &[u16], args: &mut A, host: &mut H) -> Vec<u16> {
    let pieces = parse(fmt);
    let values = read_args(&pieces, args);
    let get = |p: usize| values.get(p).copied().unwrap_or(Value::Int(0));
    let mut out = Vec::with_capacity(fmt.len() + 16);
    for piece in &pieces {
        let c = match piece {
            Piece::Text(a, b) => {
                out.extend_from_slice(&fmt[*a..*b]);
                continue;
            }
            Piece::Conv(c) => c,
        };
        let mut left = c.flags[..c.nflags].contains(&b'-');
        let width = match c.width {
            Amount::None => 0,
            Amount::Fixed(n) => n,
            Amount::Arg(p) => {
                let w = get(p).int() as i32;
                if w < 0 {
                    left = true;
                }
                w.unsigned_abs() as usize
            }
        };
        let precision = match c.precision {
            Amount::None => None,
            Amount::Fixed(n) => Some(n),
            Amount::Arg(p) => {
                let v = get(p).int() as i32;
                (v >= 0).then_some(v as usize)
            }
        };
        let Some(arg) = c.arg else {
            // An unknown conversion is written as itself.
            out.push(c.conv);
            continue;
        };
        let v = get(arg);
        let conv = c.conv as u8;
        match conv {
            b'@' => {
                let mut text = Vec::new();
                if v.int() == 0 {
                    text.extend(b"(null)".iter().map(|&b| u16::from(b)));
                } else {
                    host.describe(v.int(), &mut text);
                }
                if let Some(p) = precision {
                    text.truncate(p);
                }
                padded(&mut out, &text, width, left);
            }
            b's' | b'S' if c.size[..c.nsize] != *b"l" || conv == b'S' => {
                let p = v.int();
                let mut text = Vec::new();
                if p == 0 {
                    text.extend(b"(null)".iter().map(|&b| u16::from(b)));
                } else if conv == b's' {
                    let mut q = p as *const u8;
                    // SAFETY: the caller's contract: a NUL-terminated string;
                    // read no further than the precision.
                    unsafe {
                        while *q != 0 && precision.is_none_or(|n| text.len() < n) {
                            text.push(encoding::byte_to_unit(*q, encoding::MAC_ROMAN));
                            q = q.add(1);
                        }
                    }
                } else {
                    let mut q = p as *const u16;
                    // SAFETY: as above, a NUL-terminated `UniChar` string.
                    unsafe {
                        while q.read_unaligned() != 0 && precision.is_none_or(|n| text.len() < n) {
                            text.push(q.read_unaligned());
                            q = q.add(1);
                        }
                    }
                }
                padded(&mut out, &text, width, left);
            }
            b'c' | b'C' => {
                let unit = if conv == b'C' || c.size[..c.nsize] == *b"l" {
                    v.int() as u16
                } else {
                    encoding::byte_to_unit(v.int() as u8, encoding::MAC_ROMAN)
                };
                padded(&mut out, &[unit], width, left);
            }
            b'n' => {}
            _ => {
                // One conversion through the printf engine, its width and
                // precision written out as numbers.
                let mut spec: Vec<u8> = Vec::with_capacity(32);
                spec.push(b'%');
                spec.extend_from_slice(&c.flags[..c.nflags]);
                if left && !spec.contains(&b'-') {
                    spec.push(b'-');
                }
                push_number(&mut spec, width);
                if let Some(p) = precision {
                    spec.push(b'.');
                    push_number(&mut spec, p);
                }
                spec.extend_from_slice(&c.size[..c.nsize]);
                spec.push(conv);
                let (ints, doubles) = match v {
                    Value::Int(i) => ([i], [0.0]),
                    Value::Double(d) => ([d.to_bits()], [d]),
                };
                let mut slots = Slots { ints: &ints, doubles: &doubles };
                let _ = v.double();
                // SAFETY: the one argument is in the slots; the conversion is
                // a number one (no pointers are read).
                let _ = unsafe { printf::format(&spec, &mut slots, &mut Units(&mut out)) };
            }
        }
    }
    out
}

fn push_number(spec: &mut Vec<u8>, n: usize) {
    if n == 0 {
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    let mut v = n;
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    spec.extend_from_slice(&buf[i..]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec;

    struct Names;
    impl Host for Names {
        fn describe(&mut self, obj: u64, out: &mut Vec<u16>) {
            // In the tests an "object" is a pointer to a Rust string slice.
            // SAFETY: the tests pass `&&str` values.
            let s: &&str = unsafe { &*(obj as *const &str) };
            out.extend(s.encode_utf16());
        }
    }

    fn f(fmt: &str, ints: &[u64], doubles: &[f64]) -> String {
        let fmt: Vec<u16> = fmt.encode_utf16().collect();
        let mut a = Slots { ints, doubles };
        // SAFETY: the formats below name only what the slots hold.
        let out = unsafe { format(&fmt, &mut a, &mut Names) };
        String::from_utf16(&out).unwrap()
    }

    #[test]
    fn integers() {
        assert_eq!(f("%d", &[(-42i64) as u64], &[]), "-42");
        assert_eq!(f("%ld", &[(-4000000000i64) as u64], &[]), "-4000000000");
        assert_eq!(f("%lu", &[u64::MAX], &[]), "18446744073709551615");
        assert_eq!(f("%hhd", &[(-3i64) as u64], &[]), "-3");
        assert_eq!(f("%hu", &[65535], &[]), "65535");
        assert_eq!(f("%#x", &[255], &[]), "0xff");
        assert_eq!(f("%08x", &[0xbeef], &[]), "0000beef");
        assert_eq!(f("[%5d]", &[42], &[]), "[   42]");
        assert_eq!(f("[%-5d]", &[42], &[]), "[42   ]");
        assert_eq!(f("[%.3d]", &[7], &[]), "[007]");
        assert_eq!(f("[%*d]", &[6, 42], &[]), "[    42]");
        assert_eq!(f("%td %zu %qd", &[5, 7, 9], &[]), "5 7 9");
        assert_eq!(f("%p", &[0], &[]), "0x0");
    }

    #[test]
    fn floats() {
        assert_eq!(f("%f", &[], &[3.25]), "3.250000");
        assert_eq!(f("[%10.3f]", &[], &[1.23456]), "[     1.235]");
        assert_eq!(f("%.0f", &[], &[2.5]), "2");
        assert_eq!(f("%e", &[], &[12345.678]), "1.234568e+04");
        assert_eq!(f("%g %g", &[], &[0.0001, 1e10]), "0.0001 1e+10");
        assert_eq!(f("%f", &[], &[f64::INFINITY]), "inf");
        assert_eq!(f("%d %f %d", &[1, 2], &[0.5]), "1 0.500000 2");
    }

    #[test]
    fn cf_conversions() {
        let one: &str = "one";
        let two: &str = "two";
        let (p1, p2) = (&one as *const &str as u64, &two as *const &str as u64);
        assert_eq!(f("%2$@ %1$@", &[p1, p2], &[]), "two one");
        assert_eq!(f("%2$d-%1$d", &[1, 2], &[]), "2-1");
        assert_eq!(f("%3$d", &[1, 2, 3], &[]), "3");
        assert_eq!(f("%@", &[0], &[]), "(null)");
        assert_eq!(f("%y", &[], &[]), "y");
        assert_eq!(f("abc%", &[], &[]), "abc");
        assert_eq!(f("100%%", &[], &[]), "100%");
        assert_eq!(f("%C", &[0x20ac], &[]), "€");
        let s = b"h\xc3\xa9\0";
        assert_eq!(f("%s", &[s.as_ptr() as u64], &[]), "h√©");
        let ab = b"ab\0";
        assert_eq!(f("[%5s]", &[ab.as_ptr() as u64], &[]), "[   ab]");
        let abc = b"abc\0";
        assert_eq!(f("[%.1s]", &[abc.as_ptr() as u64], &[]), "[a]");
        let w = [0x68u16, 0x69, 0];
        assert_eq!(f("%S", &[w.as_ptr() as u64], &[]), "hi");
        assert_eq!(f("%2$f %1$d", &[7], &[1.5]), "1.500000 7");
        let _ = vec![0];
    }
}
