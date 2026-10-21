//! The exact functions of `math.h` (C11 7.12, F.10): rounding to integers,
//! `fmod`, `frexp`, `ldexp`, `modf`, `copysign`, and the UCRT's
//! classification (`_dclass`, `math.h`'s `FP_*` values). Each result is
//! exact, so there is only one right answer; they work on the bits.
//! The transcendental functions (`exp`, `log`, `pow`, `sin` …) are not here
//! yet (ADR 0084).

/// `math.h` (UCRT): `_dclass` results.
pub const FP_INFINITE: i16 = 1;
pub const FP_NAN: i16 = 2;
pub const FP_NORMAL: i16 = -1;
pub const FP_SUBNORMAL: i16 = -2;
pub const FP_ZERO: i16 = 0;

const EXP_MASK: u64 = 0x7FF << 52;
const FRAC_MASK: u64 = (1 << 52) - 1;
const SIGN: u64 = 1 << 63;

fn exponent(x: f64) -> i32 {
    ((x.to_bits() >> 52) & 0x7FF) as i32 - 1023
}

/// `trunc`: toward zero.
pub fn trunc(x: f64) -> f64 {
    let e = exponent(x);
    if e >= 52 {
        return x; // integral, infinite or NaN
    }
    if e < 0 {
        return f64::from_bits(x.to_bits() & SIGN);
    }
    let mask = FRAC_MASK >> e;
    f64::from_bits(x.to_bits() & !mask)
}

/// `floor`.
pub fn floor(x: f64) -> f64 {
    let t = trunc(x);
    if t != x && x < 0.0 { t - 1.0 } else { t }
}

/// `ceil`.
pub fn ceil(x: f64) -> f64 {
    let t = trunc(x);
    if t != x && x > 0.0 { t + 1.0 } else { t }
}

/// `round`: halfway cases away from zero.
pub fn round(x: f64) -> f64 {
    let t = trunc(x);
    let diff = x - t; // exact
    if diff >= 0.5 {
        t + 1.0
    } else if diff <= -0.5 {
        t - 1.0
    } else {
        t
    }
}

/// `modf`: the fractional part (sign of `x`) and the integral part.
pub fn modf(x: f64) -> (f64, f64) {
    if x.is_infinite() {
        return (f64::from_bits(x.to_bits() & SIGN), x);
    }
    let i = trunc(x);
    let f = x - i;
    (f64::from_bits((f.to_bits() & !SIGN) | (x.to_bits() & SIGN)), i)
}

/// `frexp`: `x = m × 2^e` with `0.5 ≤ |m| < 1` (0, infinities and NaNs
/// unchanged with `e = 0`).
pub fn frexp(x: f64) -> (f64, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let bits = x.to_bits();
    let biased = ((bits >> 52) & 0x7FF) as i32;
    if biased == 0 {
        // Subnormal: scale up first.
        let (m, e) = frexp(x * f64::from_bits(0x4350_0000_0000_0000)); // 2^54
        return (m, e - 54);
    }
    let e = biased - 1022;
    (f64::from_bits((bits & !EXP_MASK) | (1022u64 << 52)), e)
}

/// `ldexp` / `scalbn`: `x × 2^n`, correctly rounded (one rounding, also
/// into the subnormal range).
pub fn ldexp(x: f64, n: i32) -> f64 {
    let (neg, class) = crate::float::classify(x);
    let crate::float::Class::Finite { m, e } = class else { return x };
    if m == 0 {
        return x;
    }
    let t = e.saturating_add(n.clamp(-5000, 5000));
    let r = crate::float::round_binary(m, t, false, crate::float::DOUBLE);
    f64::from_bits(r.bits | if neg { SIGN } else { 0 })
}

/// `fmod`: `x − n × y` with `n` the quotient truncated, exact.
pub fn fmod(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() || x.is_infinite() || y == 0.0 {
        return f64::NAN;
    }
    if y.is_infinite() || x == 0.0 {
        return x;
    }
    let (ax, ay) = (x.abs(), y.abs());
    if ax < ay {
        return x;
    }
    // Integer mantissas and exponents.
    let parts = |v: f64| -> (u64, i32) {
        let b = v.to_bits();
        let e = ((b >> 52) & 0x7FF) as i32;
        if e == 0 { (b & FRAC_MASK, -1074) } else { ((b & FRAC_MASK) | (1 << 52), e - 1075) }
    };
    let (mx, ex) = parts(ax);
    let (my, ey) = parts(ay);
    // r = mx * 2^(ex - ey) mod my, one bit at a time.
    let mut r = mx % my;
    for _ in 0..(ex - ey) {
        r = (r << 1) % my;
    }
    let result = ldexp(r as f64, ey);
    f64::from_bits(result.to_bits() | (x.to_bits() & SIGN))
}

/// `copysign`.
pub fn copysign(x: f64, y: f64) -> f64 {
    f64::from_bits((x.to_bits() & !SIGN) | (y.to_bits() & SIGN))
}

/// `_dclass` / `fpclassify`.
pub fn class(x: f64) -> i16 {
    let b = x.to_bits();
    match ((b >> 52) & 0x7FF, b & FRAC_MASK) {
        (0, 0) => FP_ZERO,
        (0, _) => FP_SUBNORMAL,
        (0x7FF, 0) => FP_INFINITE,
        (0x7FF, _) => FP_NAN,
        _ => FP_NORMAL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLES: [f64; 17] = [
        0.0,
        -0.0,
        0.5,
        -0.5,
        1.5,
        -1.5,
        2.5,
        3.7,
        -3.7,
        1e300,
        -1e-300,
        4503599627370495.5,
        1e16 + 2.0,
        0.49999999999999994,
        5e-324,
        123.456,
        -7.0,
    ];

    #[test]
    fn rounding_matches_the_standard_library() {
        for &x in &SAMPLES {
            assert_eq!(trunc(x).to_bits(), x.trunc().to_bits(), "trunc {x}");
            assert_eq!(floor(x).to_bits(), x.floor().to_bits(), "floor {x}");
            assert_eq!(ceil(x).to_bits(), x.ceil().to_bits(), "ceil {x}");
            assert_eq!(round(x).to_bits(), x.round().to_bits(), "round {x}");
            let (f, i) = modf(x);
            assert_eq!(i.to_bits(), x.trunc().to_bits(), "modf {x}");
            assert_eq!(f + i, x);
            assert_eq!(f.is_sign_negative(), x.is_sign_negative());
        }
        assert!(trunc(f64::NAN).is_nan());
        assert_eq!(floor(f64::NEG_INFINITY), f64::NEG_INFINITY);
        assert_eq!(modf(f64::INFINITY), (0.0, f64::INFINITY));
        assert_eq!(modf(-2.25), (-0.25, -2.0));
    }

    #[test]
    fn exponents() {
        assert_eq!(frexp(8.0), (0.5, 4));
        assert_eq!(frexp(-3.0), (-0.75, 2));
        assert_eq!(frexp(0.0), (0.0, 0));
        assert_eq!(frexp(5e-324), (0.5, -1073));
        assert_eq!(ldexp(0.5, 4), 8.0);
        assert_eq!(ldexp(1.0, 1024), f64::INFINITY);
        assert_eq!(ldexp(1.0, -1074), 5e-324);
        assert_eq!(ldexp(1.0, -1075), 0.0);
        assert_eq!(ldexp(1.5, -1074), 1e-323, "rounds to even");
        assert_eq!(ldexp(5e-324, 1074), 1.0);
        assert_eq!(ldexp(1.0, -1022), f64::MIN_POSITIVE);
        assert_eq!(ldexp(f64::MAX, -2097), 1e-323);
        for &x in &SAMPLES {
            let (m, e) = frexp(x);
            assert_eq!(ldexp(m, e).to_bits(), x.to_bits(), "{x}");
        }
    }

    #[test]
    fn remainders() {
        for (x, y) in [
            (5.5, 2.0),
            (-5.5, 2.0),
            (5.5, -2.0),
            (1e300, 3.0),
            (0.3, 0.1),
            (7.0, 7.0),
            (1.0, 5e-324),
            (5e-324, 1.0),
            (1e-300, 3e-310),
        ] {
            assert_eq!(fmod(x, y).to_bits(), (x % y).to_bits(), "fmod({x}, {y})");
        }
        assert!(fmod(1.0, 0.0).is_nan() && fmod(f64::INFINITY, 1.0).is_nan());
        assert_eq!(fmod(3.0, f64::INFINITY), 3.0);
        assert_eq!(fmod(-0.0, 1.0).to_bits(), (-0.0f64).to_bits());
    }

    #[test]
    fn signs_and_classes() {
        assert_eq!(copysign(3.0, -0.0), -3.0);
        assert_eq!(class(0.0), FP_ZERO);
        assert_eq!(class(5e-324), FP_SUBNORMAL);
        assert_eq!(class(1.0), FP_NORMAL);
        assert_eq!(class(f64::INFINITY), FP_INFINITE);
        assert_eq!(class(f64::NAN), FP_NAN);
    }
}
