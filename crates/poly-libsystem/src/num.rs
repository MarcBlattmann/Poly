//! `strtod` (C11 7.22.1.3): leading white space, an optional sign, then a
//! decimal floating constant, a hexadecimal one (`0x1.8p3`), `INF` or
//! `INFINITY`, or `NAN` with an optional `(n-char-sequence)`, any case.
//!
//! A decimal number is converted by `core`'s correctly rounded parser (the
//! digits are handed over as they are); a hexadecimal one is exact up to
//! the final rounding to nearest, ties to even, done here. The result is
//! `±HUGE_VAL` with `ERANGE` for a finite number too large, and `ERANGE`
//! is also reported when a number with a non-zero digit comes out as zero
//! or subnormal (C allows it; macOS's own choice is for the libc sheets to
//! confirm).

/// What [`strtod`] found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parsed {
    pub value: f64,
    /// How many bytes of the input make up the number (0: none, and the
    /// end pointer is the start of the string).
    pub len: usize,
    /// The result over- or underflowed (`ERANGE`).
    pub range: bool,
}

fn is_space(c: u8) -> bool {
    c == b' ' || (0x09..=0x0d).contains(&c)
}

fn starts_with_ci(s: &[u8], word: &[u8]) -> bool {
    s.len() >= word.len() && s[..word.len()].eq_ignore_ascii_case(word)
}

/// Parse a number at the start of `s` (the bytes of a C string, without
/// its NUL).
pub fn strtod(s: &[u8]) -> Parsed {
    let none = Parsed { value: 0.0, len: 0, range: false };
    let mut i = 0;
    while i < s.len() && is_space(s[i]) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let sign = |v: f64| if neg { -v } else { v };
    let rest = &s[i..];
    if starts_with_ci(rest, b"infinity") {
        return Parsed { value: sign(f64::INFINITY), len: i + 8, range: false };
    }
    if starts_with_ci(rest, b"inf") {
        return Parsed { value: sign(f64::INFINITY), len: i + 3, range: false };
    }
    if starts_with_ci(rest, b"nan") {
        let mut len = i + 3;
        if s.get(len) == Some(&b'(') {
            let mut j = len + 1;
            while j < s.len() && (s[j].is_ascii_alphanumeric() || s[j] == b'_') {
                j += 1;
            }
            if s.get(j) == Some(&b')') {
                len = j + 1;
            }
        }
        return Parsed { value: sign(f64::NAN), len, range: false };
    }
    if rest.len() >= 2 && rest[0] == b'0' && (rest[1] | 0x20) == b'x' {
        if let Some((v, n, range)) = hex(&rest[2..]) {
            return Parsed { value: sign(v), len: i + 2 + n, range };
        }
        // "0x" without hex digits: the number is the 0.
        return Parsed { value: sign(0.0), len: i + 1, range: false };
    }
    let start = i;
    let mut digits = 0;
    let mut nonzero = false;
    while i < s.len() && s[i].is_ascii_digit() {
        nonzero |= s[i] != b'0';
        digits += 1;
        i += 1;
    }
    if i < s.len() && s[i] == b'.' {
        i += 1;
        while i < s.len() && s[i].is_ascii_digit() {
            nonzero |= s[i] != b'0';
            digits += 1;
            i += 1;
        }
    }
    if digits == 0 {
        return none;
    }
    if i < s.len() && (s[i] | 0x20) == b'e' {
        let mut j = i + 1;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            j += 1;
        }
        if j < s.len() && s[j].is_ascii_digit() {
            while j < s.len() && s[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    // SAFETY-free: the bytes are ASCII digits, '.', 'e', a sign.
    let text = core::str::from_utf8(&s[start..i]).unwrap_or("0");
    let v: f64 = text.parse().unwrap_or(0.0);
    let range = v.is_infinite() || (nonzero && (v == 0.0 || v.is_subnormal()));
    Parsed { value: sign(v), len: i, range }
}

/// The digits after `0x`: (value, bytes used, range error), or `None`
/// without a hex digit.
fn hex(s: &[u8]) -> Option<(f64, usize, bool)> {
    let mut m: u64 = 0;
    let mut exp: i64 = 0;
    let mut sticky = false;
    let mut digits = 0;
    let mut i = 0;
    let mut after_point = false;
    loop {
        let Some(&c) = s.get(i) else { break };
        if c == b'.' && !after_point {
            after_point = true;
            i += 1;
            continue;
        }
        let Some(d) = (c as char).to_digit(16) else { break };
        digits += 1;
        if m < 1 << 59 {
            m = m << 4 | u64::from(d);
            if after_point {
                exp -= 4;
            }
        } else {
            sticky |= d != 0;
            if !after_point {
                exp += 4;
            }
        }
        i += 1;
    }
    if digits == 0 {
        return None;
    }
    if i < s.len() && (s[i] | 0x20) == b'p' {
        let mut j = i + 1;
        let mut eneg = false;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            eneg = s[j] == b'-';
            j += 1;
        }
        if j < s.len() && s[j].is_ascii_digit() {
            let mut e: i64 = 0;
            while j < s.len() && s[j].is_ascii_digit() {
                e = (e * 10 + i64::from(s[j] - b'0')).min(1 << 20);
                j += 1;
            }
            exp += if eneg { -e } else { e };
            i = j;
        }
    }
    let nonzero = m != 0 || sticky;
    let v = compose(m, exp, sticky);
    let range = v.is_infinite() || (nonzero && (v == 0.0 || v.is_subnormal()));
    Some((v, i, range))
}

/// `m × 2^exp` (with `sticky`: more non-zero bits below `m`) rounded to
/// the nearest double, ties to even.
pub fn compose(m: u64, exp: i64, sticky: bool) -> f64 {
    if m == 0 {
        return 0.0;
    }
    // Put the leading one at bit 63.
    let shift = m.leading_zeros();
    let m = m << shift;
    let exp = exp - i64::from(shift);
    // The value is 1.f × 2^(e) with e = exp + 63.
    let e = exp + 63;
    if e > 1023 {
        return f64::INFINITY;
    }
    // Bits to drop to keep 53 (fewer below the normal range).
    let drop = if e >= -1022 { 11 } else { 11 + (-1022 - e) };
    if drop > 64 {
        return 0.0;
    }
    let (kept, half, rest) = if drop == 64 {
        (0u64, m >> 63 != 0, m << 1 != 0 || sticky)
    } else {
        let kept = m >> drop;
        let half = (m >> (drop - 1)) & 1 != 0;
        let rest = (m & ((1u64 << (drop - 1)) - 1)) != 0 || sticky;
        (kept, half, rest)
    };
    let mut q = kept;
    if half && (rest || q & 1 != 0) {
        q += 1;
    }
    if e >= -1022 {
        // Normal: q has 53 bits (or 54 after rounding up).
        let (q, e) = if q >> 53 != 0 { (q >> 1, e + 1) } else { (q, e) };
        if e > 1023 {
            return f64::INFINITY;
        }
        f64::from_bits(((e + 1023) as u64) << 52 | (q & ((1 << 52) - 1)))
    } else {
        // Subnormal (q may round up into the smallest normal, which the
        // bit pattern handles).
        f64::from_bits(q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> (f64, usize) {
        let p = strtod(s.as_bytes());
        (p.value, p.len)
    }

    #[test]
    fn decimal() {
        assert_eq!(v("  3.25xyz"), (3.25, 6));
        assert_eq!(v("-1e3"), (-1000.0, 4));
        assert_eq!(v("1e"), (1.0, 1));
        assert_eq!(v("1e+"), (1.0, 1));
        assert_eq!(v(".5"), (0.5, 2));
        assert_eq!(v("5."), (5.0, 2));
        assert_eq!(v("0.1"), (0.1, 3));
        assert_eq!(v("1.7976931348623157e308").0, f64::MAX);
        assert_eq!(v("4.9406564584124654e-324").0, f64::from_bits(1));
        assert_eq!(v("."), (0.0, 0));
        assert_eq!(v("-"), (0.0, 0));
        assert_eq!(v("abc"), (0.0, 0));
        assert_eq!(v(""), (0.0, 0));
        let p = strtod(b"1e400");
        assert!(p.value.is_infinite() && p.range);
        let p = strtod(b"1e-400");
        assert!(p.value == 0.0 && p.range);
        assert!(!strtod(b"0e-400").range);
    }

    #[test]
    fn special() {
        assert_eq!(v("inf"), (f64::INFINITY, 3));
        assert_eq!(v("-Infinity!"), (f64::NEG_INFINITY, 9));
        let (n, len) = v("nan(0x1)z");
        assert!(n.is_nan());
        assert_eq!(len, 8);
        let (n, len) = v("NaN(");
        assert!(n.is_nan());
        assert_eq!(len, 3);
    }

    #[test]
    fn hexadecimal() {
        assert_eq!(v("0x1p4"), (16.0, 5));
        assert_eq!(v("0x1.8p1"), (3.0, 7));
        assert_eq!(v("-0X.8"), (-0.5, 5));
        assert_eq!(v("0x10"), (16.0, 4));
        assert_eq!(v("0xg"), (0.0, 1));
        assert_eq!(v("0x1p-1074").0, f64::from_bits(1));
        assert_eq!(v("0x1.fffffffffffff8p0").0, 2.0); // a tie rounds to even
        assert_eq!(v("0x1.fffffffffffff7p0").0, f64::from_bits(0x3fff_ffff_ffff_ffff));
        assert_eq!(v("0x1.0000000000000800000001p0").0, f64::from_bits(0x3ff0_0000_0000_0001));
        assert!(strtod(b"0x1p2000").value.is_infinite());
        assert_eq!(v("0x123456789abcdef123p0").0, 0x1234_5678_9abc_def1_23u128 as f64);
    }

    #[test]
    fn compose_rounds() {
        assert_eq!(compose(3, 0, false), 3.0);
        assert_eq!(compose(1, -1075, false), 0.0); // half the smallest, tie to even
        assert_eq!(compose(1, -1075, true), f64::from_bits(1));
        assert_eq!(compose(u64::MAX, 0, false), 18446744073709551616.0);
    }
}
