//! Exact floating-point conversions.
//!
//! - **Binary → decimal** ([`exact`]): every finite `double` is `m × 2^e`,
//!   whose decimal expansion is finite; [`exact`] produces all of its
//!   digits with a small big-integer type, and [`Digits::round`] rounds
//!   them. The `printf` family's rounding is therefore exact: ties of an
//!   exactly representable value go to the even digit with
//!   `_CRT_INTERNAL_PRINTF_STANDARD_ROUNDING` and away from zero without it
//!   (Learn, "Format specification syntax": "Starting in Windows 10
//!   version 2004 ... exactly representable floating point numbers ...
//!   round to the closest even digit"; the legacy rule rounded them up).
//! - **Decimal / hexadecimal → binary** ([`parse`]): `strtod`, `strtof`,
//!   `atof` (C11 7.22.1.3): optional sign, a decimal or `0x` significand,
//!   an exponent, `INF`/`INFINITY`/`NAN`/`NAN(chars)`. The result is
//!   correctly rounded to nearest, ties to even (C11 F.5 for decimal
//!   strings with few enough digits; Poly rounds every input exactly).

use alloc::vec::Vec;
use core::cmp::Ordering;

/// An unsigned big integer, little-endian 32-bit limbs, no high zero limbs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Big(Vec<u32>);

impl Big {
    pub fn from_u64(v: u64) -> Self {
        let mut b = Big(alloc::vec![v as u32, (v >> 32) as u32]);
        b.trim();
        b
    }

    fn trim(&mut self) {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
    }

    pub fn is_zero(&self) -> bool {
        self.0.is_empty()
    }

    pub fn mul_small(&mut self, m: u32) {
        let mut carry = 0u64;
        for l in &mut self.0 {
            let v = u64::from(*l) * u64::from(m) + carry;
            *l = v as u32;
            carry = v >> 32;
        }
        if carry != 0 {
            self.0.push(carry as u32);
        }
        self.trim();
    }

    pub fn add_small(&mut self, a: u32) {
        let mut carry = u64::from(a);
        for l in &mut self.0 {
            if carry == 0 {
                break;
            }
            let v = u64::from(*l) + carry;
            *l = v as u32;
            carry = v >> 32;
        }
        if carry != 0 {
            self.0.push(carry as u32);
        }
    }

    /// `self × 5^k`.
    pub fn mul_pow5(&mut self, mut k: u32) {
        const P13: u32 = 1_220_703_125; // 5^13
        while k >= 13 {
            self.mul_small(P13);
            k -= 13;
        }
        self.mul_small(5u32.pow(k));
    }

    /// `self × 10^k`.
    pub fn mul_pow10(&mut self, k: u32) {
        self.mul_pow5(k);
        self.shl(k);
    }

    /// `self << bits`.
    pub fn shl(&mut self, bits: u32) {
        if self.is_zero() {
            return;
        }
        let (limbs, rest) = ((bits / 32) as usize, bits % 32);
        if rest != 0 {
            let mut carry = 0u32;
            for l in &mut self.0 {
                let v = (*l << rest) | carry;
                carry = *l >> (32 - rest);
                *l = v;
            }
            if carry != 0 {
                self.0.push(carry);
            }
        }
        if limbs != 0 {
            self.0.splice(0..0, core::iter::repeat_n(0, limbs));
        }
    }

    /// Divide by `d` in place; the remainder.
    pub fn divrem_small(&mut self, d: u32) -> u32 {
        let mut rem = 0u64;
        for l in self.0.iter_mut().rev() {
            let v = (rem << 32) | u64::from(*l);
            *l = (v / u64::from(d)) as u32;
            rem = v % u64::from(d);
        }
        self.trim();
        rem as u32
    }

    /// The number of significant bits.
    pub fn bits(&self) -> u32 {
        match self.0.last() {
            None => 0,
            Some(&top) => (self.0.len() as u32 - 1) * 32 + (32 - top.leading_zeros()),
        }
    }

    /// `self -= o`; `self >= o`.
    pub fn sub(&mut self, o: &Big) {
        let mut borrow = 0i64;
        for (i, l) in self.0.iter_mut().enumerate() {
            let v = i64::from(*l) - i64::from(o.0.get(i).copied().unwrap_or(0)) - borrow;
            if v < 0 {
                *l = (v + (1 << 32)) as u32;
                borrow = 1;
            } else {
                *l = v as u32;
                borrow = 0;
            }
        }
        self.trim();
    }

    /// The decimal digits (values 0–9), most significant first; empty for 0.
    pub fn decimal(&self) -> Vec<u8> {
        let mut n = self.clone();
        let mut chunks = Vec::new();
        while !n.is_zero() {
            chunks.push(n.divrem_small(1_000_000_000));
        }
        let mut out = Vec::new();
        for (i, c) in chunks.iter().rev().enumerate() {
            let mut d = [0u8; 9];
            let mut v = *c;
            for slot in d.iter_mut().rev() {
                *slot = (v % 10) as u8;
                v /= 10;
            }
            if i == 0 {
                let skip = d.iter().position(|&x| x != 0).unwrap_or(9);
                out.extend_from_slice(&d[skip..]);
            } else {
                out.extend_from_slice(&d);
            }
        }
        out
    }
}

impl PartialOrd for Big {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Big {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.len().cmp(&o.0.len()).then_with(|| self.0.iter().rev().cmp(o.0.iter().rev()))
    }
}

/// A decimal number `0.d₁d₂d₃… × 10^exp`; `digits` has no leading or
/// trailing zeros (empty for zero).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Digits {
    pub digits: Vec<u8>,
    pub exp: i32,
}

/// How a tie (the dropped digits are exactly one half) rounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ties {
    /// To the even digit (IEEE 754, `_CRT_INTERNAL_PRINTF_STANDARD_ROUNDING`).
    Even,
    /// Away from zero (the legacy CRT rule).
    Away,
}

impl Digits {
    fn normalise(mut self) -> Self {
        let lead = self.digits.iter().position(|&d| d != 0).unwrap_or(self.digits.len());
        self.digits.drain(..lead);
        self.exp -= lead as i32;
        while self.digits.last() == Some(&0) {
            self.digits.pop();
        }
        if self.digits.is_empty() {
            self.exp = 0;
        }
        self
    }

    pub fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }

    /// Keep `keep` significant digits (which may be 0 or negative: the
    /// number then rounds to 0 or to one unit of the digit before the
    /// first), rounding the rest away.
    pub fn round(&mut self, keep: i32, ties: Ties) {
        if keep >= self.digits.len() as i32 {
            return;
        }
        if keep < 0 {
            self.digits.clear();
            self.exp = 0;
            return;
        }
        let k = keep as usize;
        let first = self.digits[k];
        let rest_zero = self.digits[k + 1..].iter().all(|&d| d == 0);
        let up = match first.cmp(&5) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal if !rest_zero => true,
            Ordering::Equal => match ties {
                Ties::Away => true,
                Ties::Even => k > 0 && self.digits[k - 1] % 2 == 1,
            },
        };
        self.digits.truncate(k);
        if up {
            let mut i = k;
            loop {
                if i == 0 {
                    self.digits.insert(0, 1);
                    self.exp += 1;
                    break;
                }
                i -= 1;
                if self.digits[i] == 9 {
                    self.digits[i] = 0;
                } else {
                    self.digits[i] += 1;
                    break;
                }
            }
        }
        let n = core::mem::replace(self, Digits { digits: Vec::new(), exp: 0 }).normalise();
        *self = n;
    }

    /// The digit at position `i` (0 = the first significant), 0 beyond.
    pub fn at(&self, i: i32) -> u8 {
        if i < 0 { 0 } else { self.digits.get(i as usize).copied().unwrap_or(0) }
    }
}

/// A `double`'s parts: sign, and finite magnitude `m × 2^e` or a special.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Finite {
        m: u64,
        e: i32,
    },
    Infinite,
    /// A NaN: quiet bit, payload (the 51 low bits).
    Nan {
        quiet: bool,
        payload: u64,
    },
}

/// Split a `double` (IEEE 754 binary64).
pub fn classify(v: f64) -> (bool, Class) {
    let bits = v.to_bits();
    let neg = bits >> 63 != 0;
    let exp = ((bits >> 52) & 0x7FF) as i32;
    let frac = bits & ((1 << 52) - 1);
    let class = match exp {
        0x7FF if frac == 0 => Class::Infinite,
        0x7FF => Class::Nan { quiet: frac >> 51 != 0, payload: frac & ((1 << 51) - 1) },
        0 => Class::Finite { m: frac, e: -1074 },
        _ => Class::Finite { m: frac | (1 << 52), e: exp - 1075 },
    };
    (neg, class)
}

/// Every decimal digit of `m × 2^e`.
pub fn exact(m: u64, e: i32) -> Digits {
    if m == 0 {
        return Digits { digits: Vec::new(), exp: 0 };
    }
    let mut n = Big::from_u64(m);
    let point = if e >= 0 {
        n.shl(e as u32);
        0
    } else {
        // m / 2^k = m × 5^k / 10^k.
        n.mul_pow5((-e) as u32);
        e
    };
    let digits = n.decimal();
    let exp = digits.len() as i32 + point;
    Digits { digits, exp }.normalise()
}

/// A binary floating-point format for [`round_binary`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    /// Significand bits, with the hidden one (53, 24).
    pub precision: u32,
    /// The smallest normal exponent (−1022, −126).
    pub emin: i32,
    /// The largest exponent (1023, 127).
    pub emax: i32,
}

pub const DOUBLE: Format = Format { precision: 53, emin: -1022, emax: 1023 };
pub const SINGLE: Format = Format { precision: 24, emin: -126, emax: 127 };

/// What a conversion to binary gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binary {
    /// The encoding without the sign: `(biased exponent << (precision − 1))
    /// | fraction`.
    pub bits: u64,
    /// The magnitude overflowed to infinity.
    pub overflow: bool,
    /// The result is zero or subnormal and not exact (Poly's `ERANGE`
    /// rule for underflow, to be confirmed by the spec agent).
    pub underflow: bool,
}

/// Round `(q + frac) × 2^t` to `fmt`, where `sticky` says `frac > 0` (and
/// `frac < 1`), to nearest with ties to even.
pub fn round_binary(mut q: u64, mut t: i32, sticky: bool, fmt: Format) -> Binary {
    let p = fmt.precision as i32;
    let frac_bits = fmt.precision - 1;
    if q == 0 {
        return Binary { bits: 0, overflow: false, underflow: sticky };
    }
    // Give q at least p + 2 bits so the rounding bits are inside it.
    let l = 64 - q.leading_zeros() as i32;
    if l < p + 2 {
        let s = p + 2 - l;
        q <<= s;
        t -= s;
    }
    let l = 64 - q.leading_zeros() as i32;
    let e = t + l - 1; // value in [2^e, 2^(e+1))
    if e > fmt.emax {
        return Binary { bits: inf_bits(fmt), overflow: true, underflow: false };
    }
    let lsb = if e < fmt.emin { fmt.emin - (p - 1) } else { e - (p - 1) };
    let shift = lsb - t;
    let (mut mant, rem_half, rem_low) = if shift <= 0 {
        (q << (-shift), false, false)
    } else if shift > 64 {
        (0, false, true)
    } else if shift == 64 {
        (0, q >> 63 != 0, q << 1 != 0)
    } else {
        let mant = q >> shift;
        let rem = q & ((1u64 << shift) - 1);
        let half = 1u64 << (shift - 1);
        (mant, rem & half != 0, rem & (half - 1) != 0)
    };
    let inexact = rem_half || rem_low || sticky;
    if rem_half && (rem_low || sticky || mant & 1 == 1) {
        mant += 1;
    }
    let mut lsb = lsb;
    if mant >> fmt.precision != 0 {
        mant >>= 1;
        lsb += 1;
        if lsb + (p - 1) > fmt.emax {
            return Binary { bits: inf_bits(fmt), overflow: true, underflow: false };
        }
    }
    let hidden = 1u64 << frac_bits;
    let bits = if mant & hidden != 0 {
        let biased = (lsb + (p - 1) + fmt.emax) as u64;
        (biased << frac_bits) | (mant & (hidden - 1))
    } else {
        mant
    };
    Binary { bits, overflow: false, underflow: inexact && mant & hidden == 0 }
}

fn inf_bits(fmt: Format) -> u64 {
    ((2 * fmt.emax + 1) as u64) << (fmt.precision - 1)
}

/// The most significant decimal digits [`parse`] keeps; any further
/// non-zero digit becomes one more "1" digit, which decides every tie
/// correctly (a halfway point between two doubles has at most 767
/// significant digits).
const MAX_DIGITS: usize = 800;

/// Round `D × 10^q` (`digits` are D's decimal digits, values 0–9).
pub fn decimal_to_binary(digits: &[u8], q: i32, fmt: Format) -> Binary {
    let lead = digits.iter().position(|&d| d != 0).unwrap_or(digits.len());
    let digits = &digits[lead..];
    let end = digits.iter().rposition(|&d| d != 0).map_or(0, |i| i + 1);
    let mut q = q + (digits.len() - end) as i32;
    let digits = &digits[..end];
    if digits.is_empty() {
        return Binary { bits: 0, overflow: false, underflow: false };
    }
    // Far outside the range: no big arithmetic needed.
    let magnitude = q + digits.len() as i32; // value < 10^magnitude
    let max10 = (f64::from(fmt.emax + 1) * core::f64::consts::LOG10_2) as i32 + 2;
    let min10 = (f64::from(fmt.emin - fmt.precision as i32) * core::f64::consts::LOG10_2) as i32 - 2;
    if magnitude > max10 {
        return Binary { bits: inf_bits(fmt), overflow: true, underflow: false };
    }
    if magnitude < min10 {
        return Binary { bits: 0, overflow: false, underflow: true };
    }
    let mut d = Big::default();
    let kept = digits.len().min(MAX_DIGITS);
    for &x in &digits[..kept] {
        d.mul_small(10);
        d.add_small(u32::from(x));
    }
    q += (digits.len() - kept) as i32;
    if digits[kept..].iter().any(|&x| x != 0) {
        d.mul_small(10);
        d.add_small(1);
        q -= 1;
    }
    let mut s = Big::from_u64(1);
    if q >= 0 {
        d.mul_pow10(q as u32);
    } else {
        s.mul_pow10((-q) as u32);
    }
    // Scale so that the quotient has p + 3 or p + 4 bits.
    let want = fmt.precision as i32 + 3;
    let t = d.bits() as i32 - s.bits() as i32 - want;
    if t >= 0 {
        s.shl(t as u32);
    } else {
        d.shl((-t) as u32);
    }
    let mut quotient = 0u64;
    let top = d.bits() as i32 - s.bits() as i32;
    for i in (0..=top.max(0)).rev() {
        let mut shifted = s.clone();
        shifted.shl(i as u32);
        if d >= shifted {
            d.sub(&shifted);
            quotient |= 1 << i;
        }
    }
    round_binary(quotient, t, !d.is_zero(), fmt)
}

/// What [`parse`] read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parsed {
    /// The value's encoding (sign included) in the format asked for.
    pub bits: u64,
    /// Units consumed (0: no conversion; `endptr` is the start).
    pub len: usize,
    /// `ERANGE`: overflow, or underflow by [`Binary::underflow`].
    pub range_error: bool,
}

fn lower(c: u32) -> u32 {
    if (u32::from(b'A')..=u32::from(b'Z')).contains(&c) { c + 32 } else { c }
}

fn matches_ci<C: crate::Unit>(s: &[C], at: usize, word: &[u8]) -> bool {
    word.iter().enumerate().all(|(i, &w)| s.get(at + i).is_some_and(|c| lower(c.to_u32()) == u32::from(w)))
}

/// `strtod`'s grammar (C11 7.22.1.3), after leading white space (the
/// caller skips it with `isspace`). `s` need not hold the NUL.
pub fn parse<C: crate::Unit>(s: &[C], fmt: Format) -> Parsed {
    parse_point(s, fmt, b'.')
}

/// [`parse`] with the locale's decimal point (ADR 0094).
pub fn parse_point<C: crate::Unit>(s: &[C], fmt: Format, point: u8) -> Parsed {
    let at = |i: usize| s.get(i).map_or(0, |c| c.to_u32());
    let sign_bit = 1u64 << (fmt.precision - 1 + bits_of_exponent(fmt));
    let mut i = 0;
    let mut neg = false;
    if at(0) == u32::from(b'+') || at(0) == u32::from(b'-') {
        neg = at(0) == u32::from(b'-');
        i = 1;
    }
    let sign = if neg { sign_bit } else { 0 };
    let none = Parsed { bits: 0, len: 0, range_error: false };
    // INF, INFINITY, NAN, NAN(n-char-sequence).
    if matches_ci(s, i, b"inf") {
        let len = if matches_ci(s, i, b"infinity") { i + 8 } else { i + 3 };
        return Parsed { bits: sign | inf_bits(fmt), len, range_error: false };
    }
    if matches_ci(s, i, b"nan") {
        let mut len = i + 3;
        if at(len) == u32::from(b'(') {
            let mut j = len + 1;
            while at(j) < 0x80 && (at(j) as u8).is_ascii_alphanumeric() || at(j) == u32::from(b'_') {
                j += 1;
            }
            if at(j) == u32::from(b')') {
                len = j + 1;
            }
        }
        let quiet = 1u64 << (fmt.precision - 2);
        return Parsed { bits: sign | inf_bits(fmt) | quiet, len, range_error: false };
    }
    let digit = |c: u32, radix: u32| char::from_u32(c).and_then(|ch| ch.to_digit(radix));
    // Hexadecimal: 0x h* [. h*] [p [sign] d+].
    if at(i) == u32::from(b'0') && lower(at(i + 1)) == u32::from(b'x') {
        let mut j = i + 2;
        let mut mant = 0u64;
        let mut exp2 = 0i64;
        let mut sticky = false;
        let mut any = false;
        let mut seen_point = false;
        loop {
            let c = at(j);
            if c == u32::from(point) && !seen_point {
                seen_point = true;
            } else if let Some(v) = digit(c, 16) {
                any = true;
                if mant >> 60 == 0 {
                    mant = (mant << 4) | u64::from(v);
                    if seen_point {
                        exp2 -= 4;
                    }
                } else {
                    sticky |= v != 0;
                    if !seen_point {
                        exp2 += 4;
                    }
                }
            } else {
                break;
            }
            j += 1;
        }
        if !any {
            // "0x" with no hex digit: the "0" is the subject sequence.
            return Parsed { bits: sign, len: i + 1, range_error: false };
        }
        if lower(at(j)) == u32::from(b'p') {
            let (e, n) = parse_exponent(s, j + 1);
            if n > 0 {
                exp2 += e;
                j = j + 1 + n;
            }
        }
        let t = exp2.clamp(-100_000, 100_000) as i32;
        let b = round_binary(mant, t, sticky, fmt);
        return Parsed { bits: sign | b.bits, len: j, range_error: b.overflow || b.underflow };
    }
    // Decimal: d* [. d*] [e [sign] d+], at least one digit.
    let mut j = i;
    let mut digits: Vec<u8> = Vec::new();
    let mut point_shift = 0i64;
    let mut seen_point = false;
    let mut any = false;
    loop {
        let c = at(j);
        if c == u32::from(point) && !seen_point {
            seen_point = true;
        } else if let Some(v) = digit(c, 10) {
            any = true;
            // Leading zeros only move the point; `decimal_to_binary` keeps
            // the first MAX_DIGITS of the rest exactly.
            if !(digits.is_empty() && v == 0) {
                digits.push(v as u8);
            }
            if seen_point {
                point_shift -= 1;
            }
        } else {
            break;
        }
        j += 1;
    }
    if !any {
        return none;
    }
    let mut exp10 = point_shift;
    if lower(at(j)) == u32::from(b'e') {
        let (e, n) = parse_exponent(s, j + 1);
        if n > 0 {
            exp10 += e;
            j = j + 1 + n;
        }
    }
    let q = exp10.clamp(-100_000, 100_000) as i32;
    let b = decimal_to_binary(&digits, q, fmt);
    Parsed { bits: sign | b.bits, len: j, range_error: b.overflow || b.underflow }
}

fn bits_of_exponent(fmt: Format) -> u32 {
    (2 * fmt.emax + 2).trailing_zeros()
}

/// `[+|-] digits` at `from`: the value (saturated) and the units read (0 if
/// there is no digit).
fn parse_exponent<C: crate::Unit>(s: &[C], from: usize) -> (i64, usize) {
    let at = |i: usize| s.get(i).map_or(0, |c| c.to_u32());
    let mut j = from;
    let mut neg = false;
    if at(j) == u32::from(b'+') || at(j) == u32::from(b'-') {
        neg = at(j) == u32::from(b'-');
        j += 1;
    }
    let start = j;
    let mut v: i64 = 0;
    while (u32::from(b'0')..=u32::from(b'9')).contains(&at(j)) {
        v = (v * 10 + i64::from(at(j) - u32::from(b'0'))).min(1_000_000_000);
        j += 1;
    }
    if j == start {
        return (0, 0);
    }
    (if neg { -v } else { v }, j - from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec;

    fn digits_str(d: &Digits) -> String {
        d.digits.iter().map(|&x| char::from(b'0' + x)).collect()
    }

    #[test]
    fn exact_expansions() {
        let (_, Class::Finite { m, e }) = classify(0.1) else { panic!() };
        let d = exact(m, e);
        assert_eq!(digits_str(&d), "1000000000000000055511151231257827021181583404541015625");
        assert_eq!(d.exp, 0);
        let (_, Class::Finite { m, e }) = classify(1234.5) else { panic!() };
        let d = exact(m, e);
        assert_eq!((digits_str(&d).as_str(), d.exp), ("12345", 4));
        let (_, Class::Finite { m, e }) = classify(1e300) else { panic!() };
        assert_eq!(exact(m, e).exp, 301);
        let (_, Class::Finite { m, e }) = classify(f64::from_bits(1)) else { panic!() };
        let d = exact(m, e);
        assert_eq!(d.exp, -323);
        assert!(digits_str(&d).starts_with("494065645841246544"));
        assert_eq!(d.digits.len(), 751);
        assert!(exact(0, 0).is_zero());
    }

    #[test]
    fn rounding_ties() {
        let mut d = Digits { digits: vec![2, 5], exp: 1 };
        d.round(1, Ties::Even);
        assert_eq!((d.digits.clone(), d.exp), (vec![2], 1));
        let mut d = Digits { digits: vec![2, 5], exp: 1 };
        d.round(1, Ties::Away);
        assert_eq!((d.digits.clone(), d.exp), (vec![3], 1));
        let mut d = Digits { digits: vec![9, 9, 5], exp: 0 };
        d.round(2, Ties::Even);
        assert_eq!((d.digits.clone(), d.exp), (vec![1], 1));
        let mut d = Digits { digits: vec![5], exp: 0 };
        d.round(0, Ties::Even);
        assert!(d.is_zero());
        let mut d = Digits { digits: vec![5, 1], exp: 0 };
        d.round(0, Ties::Even);
        assert_eq!((d.digits.clone(), d.exp), (vec![1], 1));
        let mut d = Digits { digits: vec![5], exp: 0 };
        d.round(-1, Ties::Away);
        assert!(d.is_zero());
    }

    fn strtod(s: &str) -> (f64, usize, bool) {
        let p = parse(s.as_bytes(), DOUBLE);
        (f64::from_bits(p.bits), p.len, p.range_error)
    }

    #[test]
    fn decimal_parsing_is_correctly_rounded() {
        for s in [
            "0.1",
            "1e23",
            "8.98846567431158e307",
            "2.2250738585072011e-308",
            "2.2250738585072014e-308",
            "4.9406564584124654e-324",
            "1.7976931348623157e308",
            "123456789012345678901234567890",
            "0.000001",
            "3.14159265358979323846264338327950288",
            "9007199254740993",
            "9007199254740995",
            "2.4703282292062327e-324",
            "2.4703282292062328e-324",
            "1.5e-323",
            "6.02214076e23",
        ] {
            let want: f64 = s.parse().unwrap();
            let (got, len, _) = strtod(s);
            assert_eq!(got.to_bits(), want.to_bits(), "{s}");
            assert_eq!(len, s.len(), "{s}");
        }
        // The halfway point between 1 and the next double, then a hair more.
        let (v, _, _) = strtod("1.00000000000000011102230246251565404236316680908203125");
        assert_eq!(v, 1.0);
        let (v, _, _) = strtod("1.00000000000000011102230246251565404236316680908203125000000000001");
        assert_eq!(v, 1.0000000000000002);
        // A thousand digits: kept and sticky.
        let mut long = String::from("1.");
        long.extend(core::iter::repeat_n('0', 1000));
        long.push('1');
        assert_eq!(strtod(&long).0, 1.0);
    }

    #[test]
    fn specials_signs_and_ends() {
        assert_eq!(strtod("-0"), (-0.0, 2, false));
        assert!(strtod("-0").0.is_sign_negative());
        assert_eq!(strtod("1e400"), (f64::INFINITY, 5, true));
        assert_eq!(strtod("-1e400"), (f64::NEG_INFINITY, 6, true));
        let (v, n, r) = strtod("1e-400");
        assert_eq!((v, n, r), (0.0, 6, true));
        let (v, _, r) = strtod("1e-310");
        assert!(v > 0.0 && r, "a subnormal result is inexact: ERANGE");
        assert_eq!(strtod("InFiNiTy"), (f64::INFINITY, 8, false));
        assert_eq!(strtod("infinit"), (f64::INFINITY, 3, false));
        let (v, n, _) = strtod("nan(abc_1)x");
        assert!(v.is_nan() && n == 10);
        let (v, n, _) = strtod("-nan(");
        assert!(v.is_nan() && v.is_sign_negative() && n == 4);
        assert_eq!(strtod("1.5e"), (1.5, 3, false));
        assert_eq!(strtod("1.5e+"), (1.5, 3, false));
        assert_eq!(strtod(".5"), (0.5, 2, false));
        assert_eq!(strtod("5."), (5.0, 2, false));
        assert_eq!(strtod("."), (0.0, 0, false));
        assert_eq!(strtod("+"), (0.0, 0, false));
        assert_eq!(strtod("x"), (0.0, 0, false));
        assert_eq!(strtod("12abc"), (12.0, 2, false));
    }

    #[test]
    fn hexadecimal_parsing() {
        assert_eq!(strtod("0x1.8p3"), (12.0, 7, false));
        assert_eq!(strtod("0X10"), (16.0, 4, false));
        assert_eq!(strtod("0x.8"), (0.5, 4, false));
        assert_eq!(strtod("0x1p-1074"), (f64::from_bits(1), 9, false));
        assert_eq!(strtod("0x1p-1075").0, 0.0, "a tie rounds to even (zero)");
        assert_eq!(strtod("0x1.8p-1074").0, f64::from_bits(2));
        assert_eq!(strtod("0x1.fffffffffffff8p0").0, 2.0);
        assert_eq!(strtod("0x1.fffffffffffff7p0").0, 1.9999999999999998);
        assert_eq!(strtod("0x1p1024").0, f64::INFINITY);
        assert_eq!(strtod("0xg"), (0.0, 1, false));
        assert_eq!(strtod("0x1p"), (1.0, 3, false));
        assert_eq!(strtod("0x123456789abcdef0123p0").0, 0x123456789abcdef0123u128 as f64);
    }

    #[test]
    fn single_precision() {
        for s in ["0.1", "3.4028235e38", "1e-45", "1.17549435e-38", "16777217", "7e-46"] {
            let p = parse(s.as_bytes(), SINGLE);
            let want: f32 = s.parse().unwrap();
            assert_eq!(p.bits as u32, want.to_bits(), "{s}");
        }
        let p = parse(b"3.5e38", SINGLE);
        assert_eq!((f32::from_bits(p.bits as u32), p.range_error), (f32::INFINITY, true));
        let p = parse(b"-inf", SINGLE);
        assert_eq!(f32::from_bits(p.bits as u32), f32::NEG_INFINITY);
    }

    #[test]
    fn wide_input() {
        let w: alloc::vec::Vec<u16> = "2.5e1z".encode_utf16().collect();
        let p = parse(&w, DOUBLE);
        assert_eq!((f64::from_bits(p.bits), p.len), (25.0, 5));
    }

    #[test]
    fn big_arithmetic() {
        let mut b = Big::from_u64(u64::MAX);
        b.mul_small(10);
        b.add_small(5);
        assert_eq!(b.decimal(), [1, 8, 4, 4, 6, 7, 4, 4, 0, 7, 3, 7, 0, 9, 5, 5, 1, 6, 1, 5, 5]);
        let r = b.divrem_small(10);
        assert_eq!((r, b.clone()), (5, Big::from_u64(u64::MAX)));
        let mut c = Big::from_u64(1);
        c.shl(100);
        assert_eq!(c.bits(), 101);
        let mut d = c.clone();
        d.sub(&Big::from_u64(1));
        assert_eq!(d.bits(), 100);
        assert!(d < c);
    }
}
