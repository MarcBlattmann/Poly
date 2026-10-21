//! Integer conversions: `strtol`, `strtoul`, `strtoll`, `strtoull`,
//! `_strtoi64`, `_strtoui64` and their wide forms (C11 7.22.1.4), `atoi`,
//! `atol`, `atoll`, `_atoi64` (Learn: `atoi`: "In the case of overflow with
//! large positive integral values, atol returns LONG_MAX; ... atoi returns
//! INT_MAX and INT_MIN on these conditions", `errno` = `ERANGE`), and
//! `_itoa`, `_ltoa`, `_ultoa`, `_i64toa`, `_ui64toa` (Learn: "If radix
//! equals 10 and value is negative, the first character of the stored
//! string is the minus sign").

use crate::ctype;
use crate::unit::Unit;
use alloc::vec::Vec;

/// What a `strto*` conversion gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Integer {
    /// The value, already clamped and negated in the result type
    /// (two's-complement bits of an `i64`/`u64`, or of the 32-bit type
    /// sign- or zero-extended).
    pub value: u64,
    /// Units consumed (0: no conversion; `endptr` is the start).
    pub len: usize,
    /// `ERANGE`.
    pub overflow: bool,
}

/// The integer type a conversion produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `long` (32-bit on Windows).
    I32,
    /// `unsigned long`.
    U32,
    /// `long long`, `__int64`.
    I64,
    /// `unsigned long long`.
    U64,
}

impl Kind {
    fn max_magnitude(self, negative: bool) -> u64 {
        match (self, negative) {
            (Kind::I32, false) => i32::MAX as u64,
            (Kind::I32, true) => 1 << 31,
            (Kind::I64, false) => i64::MAX as u64,
            (Kind::I64, true) => 1 << 63,
            (Kind::U32, _) => u64::from(u32::MAX),
            (Kind::U64, _) => u64::MAX,
        }
    }
}

/// `strtol`'s grammar; `base` must be 0 or 2–36 (the caller reports any
/// other as an invalid parameter). `s` need not hold the NUL.
pub fn parse<C: Unit>(s: &[C], base: u32, kind: Kind) -> Integer {
    let at = |i: usize| s.get(i).map_or(0, |c| c.to_u32());
    let none = Integer { value: 0, len: 0, overflow: false };
    let mut i = 0;
    while ctype::is_space_unit(at(i), C::WIDE) {
        i += 1;
    }
    let mut negative = false;
    if at(i) == u32::from(b'+') || at(i) == u32::from(b'-') {
        negative = at(i) == u32::from(b'-');
        i += 1;
    }
    let digit = |c: u32| char::from_u32(c).and_then(|ch| ch.to_digit(36));
    let mut base = base;
    let x = |c: u32| c == u32::from(b'x') || c == u32::from(b'X');
    if (base == 0 || base == 16) && at(i) == u32::from(b'0') && x(at(i + 1)) && digit(at(i + 2)).is_some_and(|d| d < 16)
    {
        i += 2;
        base = 16;
    } else if base == 0 {
        base = if at(i) == u32::from(b'0') { 8 } else { 10 };
    }
    let start = i;
    let limit = kind.max_magnitude(negative);
    let mut magnitude: u64 = 0;
    let mut overflow = false;
    while let Some(d) = digit(at(i)).filter(|&d| d < base) {
        match magnitude.checked_mul(u64::from(base)).and_then(|m| m.checked_add(u64::from(d))) {
            Some(m) if m <= limit => magnitude = m,
            _ => overflow = true,
        }
        i += 1;
    }
    if i == start {
        return none;
    }
    let value = if overflow {
        match kind {
            Kind::I32 if negative => i64::from(i32::MIN) as u64,
            Kind::I32 => i32::MAX as u64,
            Kind::I64 if negative => i64::MIN as u64,
            Kind::I64 => i64::MAX as u64,
            Kind::U32 => u64::from(u32::MAX),
            Kind::U64 => u64::MAX,
        }
    } else if negative {
        match kind {
            // C11: "the value resulting from the conversion is negated (in
            // the return type)".
            Kind::U32 => u64::from((magnitude as u32).wrapping_neg()),
            Kind::I32 => i64::from((magnitude as u32).wrapping_neg() as i32) as u64,
            _ => magnitude.wrapping_neg(),
        }
    } else {
        magnitude
    };
    Integer { value, len: i, overflow }
}

/// `atoi`: `strtol` in base 10, clamped to `int` with `ERANGE`.
pub fn atoi<C: Unit>(s: &[C]) -> (i32, bool) {
    let r = parse(s, 10, Kind::I32);
    (r.value as i64 as i32, r.overflow)
}

/// `_atoi64` / `atoll`.
pub fn atoi64<C: Unit>(s: &[C]) -> (i64, bool) {
    let r = parse(s, 10, Kind::I64);
    (r.value as i64, r.overflow)
}

/// `_itoa` and its family: the digits of `value` (whose bits are those of a
/// `bits`-wide integer) in `radix` (2–36), lower case. A negative value
/// shows its sign only in radix 10 (`signed` for `_itoa`, `_ltoa`,
/// `_i64toa`); otherwise its bits are taken unsigned.
pub fn itoa(value: u64, bits: u32, signed: bool, radix: u32) -> Vec<u8> {
    let mask = if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 };
    let v = value & mask;
    let negative = signed && radix == 10 && bits > 0 && (v >> (bits - 1)) & 1 == 1;
    let mut magnitude = if negative { (v.wrapping_neg()) & mask } else { v };
    let mut out = Vec::new();
    loop {
        let d = (magnitude % u64::from(radix)) as u8;
        out.push(if d < 10 { b'0' + d } else { b'a' + d - 10 });
        magnitude /= u64::from(radix);
        if magnitude == 0 {
            break;
        }
    }
    if negative {
        out.push(b'-');
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str, base: u32, kind: Kind) -> (u64, usize, bool) {
        let r = parse(s.as_bytes(), base, kind);
        (r.value, r.len, r.overflow)
    }

    #[test]
    fn strtol_bases_and_prefixes() {
        assert_eq!(p("  -123xyz", 10, Kind::I32), ((-123i64) as u64, 6, false));
        assert_eq!(p("\t\n+42", 0, Kind::I32), (42, 5, false));
        assert_eq!(p("0x1F", 0, Kind::I32), (31, 4, false));
        assert_eq!(p("0X1f", 16, Kind::I32), (31, 4, false));
        assert_eq!(p("1f", 16, Kind::I32), (31, 2, false));
        assert_eq!(p("017", 0, Kind::I32), (15, 3, false));
        assert_eq!(p("019", 0, Kind::I32), (1, 2, false));
        assert_eq!(p("0x", 0, Kind::I32), (0, 1, false), "the 0 is the subject; endptr after it");
        assert_eq!(p("0xg", 16, Kind::I32), (0, 1, false));
        assert_eq!(p("zz", 36, Kind::I32), (36 * 35 + 35, 2, false));
        assert_eq!(p("102", 2, Kind::I32), (2, 2, false));
        assert_eq!(p("", 10, Kind::I32), (0, 0, false));
        assert_eq!(p("  -", 10, Kind::I32), (0, 0, false));
        assert_eq!(p("abc", 10, Kind::I32), (0, 0, false));
    }

    #[test]
    fn strtol_overflow_and_negation() {
        assert_eq!(p("2147483647", 10, Kind::I32), (2147483647, 10, false));
        assert_eq!(p("2147483648", 10, Kind::I32), (i32::MAX as u64, 10, true));
        assert_eq!(p("-2147483648", 10, Kind::I32), (i64::from(i32::MIN) as u64, 11, false));
        assert_eq!(p("-2147483649", 10, Kind::I32), (i64::from(i32::MIN) as u64, 11, true));
        assert_eq!(p("4294967295", 10, Kind::U32), (u64::from(u32::MAX), 10, false));
        assert_eq!(p("4294967296", 10, Kind::U32), (u64::from(u32::MAX), 10, true));
        assert_eq!(p("-1", 10, Kind::U32), (u64::from(u32::MAX), 2, false));
        assert_eq!(p("-4294967295", 10, Kind::U32), (1, 11, false));
        assert_eq!(p("-4294967296", 10, Kind::U32), (u64::from(u32::MAX), 11, true));
        assert_eq!(p("9223372036854775808", 10, Kind::I64), (i64::MAX as u64, 19, true));
        assert_eq!(p("-9223372036854775808", 10, Kind::I64), (i64::MIN as u64, 20, false));
        assert_eq!(p("18446744073709551615", 10, Kind::U64), (u64::MAX, 20, false));
        assert_eq!(p("18446744073709551616", 10, Kind::U64), (u64::MAX, 20, true));
        assert_eq!(p("-1", 0, Kind::U64), (u64::MAX, 2, false));
        assert_eq!(p("99999999999999999999999x", 10, Kind::I32), (i32::MAX as u64, 23, true));
    }

    #[test]
    fn wide_and_atoi() {
        let w: Vec<u16> = " -77z".encode_utf16().collect();
        assert_eq!(parse(&w, 10, Kind::I32).value as i64, -77);
        assert_eq!(atoi(b"  12abc"), (12, false));
        assert_eq!(atoi(b"99999999999"), (i32::MAX, true));
        assert_eq!(atoi(b"-99999999999"), (i32::MIN, true));
        assert_eq!(atoi(b"x"), (0, false));
        assert_eq!(atoi64(b"-9223372036854775809"), (i64::MIN, true));
    }

    #[test]
    fn itoa_radixes() {
        assert_eq!(itoa((-42i64) as u64, 32, true, 10), b"-42");
        assert_eq!(itoa((-1i64) as u64, 32, true, 16), b"ffffffff");
        assert_eq!(itoa((-1i64) as u64, 64, true, 16), b"ffffffffffffffff");
        assert_eq!(itoa(255, 32, false, 2), b"11111111");
        assert_eq!(itoa(0, 32, true, 10), b"0");
        assert_eq!(itoa(35, 32, false, 36), b"z");
        assert_eq!(itoa(u64::MAX, 64, false, 10), b"18446744073709551615");
        assert_eq!(itoa(i64::MIN as u64, 64, true, 10), b"-9223372036854775808");
        assert_eq!(itoa(0x8000_0000, 32, true, 10), b"-2147483648");
    }
}
