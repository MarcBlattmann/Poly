//! Roots and distances: `sqrt` (correctly rounded, in integer arithmetic,
//! so the crate needs no hardware square root), `cbrt` and `hypot`.
//!
//! - `sqrt`: the integer square root of the significand widened to 110
//!   bits gives 55 bits of the root and whether the remainder is zero; the
//!   root is rounded once to nearest-even (IEEE 754-2019, 5.4.1 requires
//!   it correctly rounded, so there is only one right answer).
//! - `cbrt`: Newton's iteration `y ← y − (y³ − a)/(3y²)` on the significand
//!   scaled to [1/2, 4), then one correction with the residual `a − y³`
//!   computed exactly in double-double. The result is within 0.5 + 2^-40
//!   ulp (the correction's own error is far below the last bit); exact cubes
//!   come out exact.
//! - `hypot`: both arguments scaled by the same power of two, the sum of
//!   squares exact in double-double, its double-double square root, one
//!   rounding at the end (also into the subnormal range): within 0.5 +
//!   2^-90 ulp, no spurious overflow or underflow.

use crate::dd::{self, DD, two_prod};

/// `sqrt` of a non-negative finite or infinite `x`, correctly rounded; NaN
/// for negative `x` (but `sqrt(-0) = -0`, IEEE 754).
pub fn sqrt(x: f64) -> f64 {
    if x.is_nan() || x == f64::INFINITY || x == 0.0 {
        return x;
    }
    if x < 0.0 {
        return f64::NAN;
    }
    let b = x.to_bits();
    let biased = ((b >> 52) & 0x7FF) as i32;
    // x = m × 2^e with m an integer.
    let (mut m, mut e) = if biased == 0 {
        (b & ((1 << 52) - 1), -1074)
    } else {
        ((b & ((1 << 52) - 1)) | (1 << 52), biased - 1075)
    };
    // Widen m to 109 or 110 bits with an even shift, so the root has 55.
    let bits = 64 - m.leading_zeros() as i32;
    let mut shift = 110 - bits;
    if (e - shift) & 1 != 0 {
        shift -= 1;
    }
    let big = u128::from(m) << shift;
    e -= shift;
    let (r, exact) = isqrt(big);
    // r has 54 or 55 bits; keep 53 and round to nearest-even.
    let extra = 128 - r.leading_zeros() as i32 - 53;
    let mut q = (r >> extra) as u64;
    let rest = r & ((1u128 << extra) - 1);
    let half = 1u128 << (extra - 1);
    if rest > half || (rest == half && (!exact || q & 1 == 1)) {
        q += 1;
    }
    let mut re = e / 2 + extra;
    if q == 1 << 53 {
        q >>= 1;
        re += 1;
    }
    m = q;
    // q × 2^re with q of 53 bits: always normal (the root of the smallest
    // subnormal is about 2^-537).
    let biased = (re + 52 + 1023) as u64;
    f64::from_bits((biased << 52) | (m & ((1 << 52) - 1)))
}

/// The integer square root of `n` and whether it is exact (digit by digit,
/// two bits at a time).
fn isqrt(n: u128) -> (u128, bool) {
    let mut rem = n;
    let mut root: u128 = 0;
    let mut bit: u128 = 1 << 126;
    while bit > n {
        bit >>= 2;
    }
    while bit != 0 {
        if rem >= root + bit {
            rem -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    (root, rem == 0)
}

/// `cbrt`: the real cube root (odd, so `cbrt(-8) = -2`).
pub fn cbrt(x: f64) -> f64 {
    if x.is_nan() || x.is_infinite() || x == 0.0 {
        return x;
    }
    let neg = x < 0.0;
    let mut a = x.abs();
    let mut bias = 0;
    if a < f64::MIN_POSITIVE {
        a *= dd::pow2(54);
        bias = -18; // cbrt(2^54) = 2^18
    }
    // a = m × 2^(3q + j), m ∈ [1/2, 1), then fold j into m: m ∈ [1/2, 4).
    let e = dd::exponent_of(a) + 1;
    let m = a * dd::pow2(-e);
    let q = e.div_euclid(3);
    let j = e.rem_euclid(3);
    let m = m * dd::pow2(j);
    // Newton from a guess at most 1.6 times off: quadratic convergence,
    // eight steps reach full precision with room to spare.
    let mut y = 0.5 + m / 3.0;
    for _ in 0..8 {
        y -= (y * y * y - m) / (3.0 * y * y);
    }
    // One correction with the exact residual m − y³.
    let y2 = two_prod(y, y);
    let y3 = y2.mul_f64(y);
    let resid = DD::from_f64(m).sub(y3);
    let corr = resid.to_f64() / (3.0 * y * y);
    let r = dd::fast_two_sum(y, corr);
    let v = dd::round_scaled(r, q + bias);
    if neg { -v } else { v }
}

/// `hypot`: `sqrt(x² + y²)` without undue overflow or underflow. An
/// infinite argument gives +inf even when the other is a NaN (C11
/// F.10.4.3).
pub fn hypot(x: f64, y: f64) -> f64 {
    if x.is_infinite() || y.is_infinite() {
        return f64::INFINITY;
    }
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    let (mut a, mut b) = (x.abs(), y.abs());
    if a < b {
        core::mem::swap(&mut a, &mut b);
    }
    if b == 0.0 {
        return a;
    }
    let e = dd::exponent_of(a);
    if e - dd::exponent_of(b) > 60 {
        // b² is below half an ulp of a²: hypot is a, correctly rounded.
        return a;
    }
    let a1 = dd::ldexp_exact(a, -e);
    let b1 = dd::ldexp_exact(b, -e);
    let s = two_prod(a1, a1).add(two_prod(b1, b1));
    let r = s.sqrt();
    dd::round_scaled(r, e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    #[test]
    fn square_roots_are_correctly_rounded() {
        // The hardware's sqrt is correctly rounded too: one right answer.
        let mut xs: Vec<f64> = std::vec![
            0.0, -0.0, 1.0, 2.0, 4.0, 0.25, 3.0, 5e-324, 1e-323, 2.2250738585072014e-308,
            2.225073858507201e-308, f64::MAX, 1e300, 1e-300, 0.5, 1.0000000000000002,
            0.9999999999999999, f64::INFINITY,
        ];
        let mut s: u64 = 0x1234_5678_9ABC_DEF1;
        for _ in 0..20000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            xs.push(f64::from_bits(s & 0x7FFF_FFFF_FFFF_FFFF));
        }
        for x in xs {
            if x.is_nan() {
                continue;
            }
            assert_eq!(sqrt(x).to_bits(), x.sqrt().to_bits(), "sqrt({x:e})");
        }
        assert!(sqrt(-1.0).is_nan() && sqrt(f64::NAN).is_nan());
    }

    #[test]
    fn cube_roots() {
        for (x, want) in [(27.0, 3.0), (-8.0, -2.0), (1.0, 1.0), (0.125, 0.5), (1e300, 1e100), (1e-300, 1e-100)] {
            assert_eq!(cbrt(x), want, "cbrt({x})");
        }
        for k in -340..340 {
            let y = 1.5f64 * 2f64.powi(k);
            assert_eq!(cbrt(y * y * y), y, "cbrt of a cube");
        }
        assert_eq!(cbrt(5e-324), 2f64.powf(-358.0));
        assert_eq!(cbrt(-0.0).to_bits(), (-0.0f64).to_bits());
        assert!(cbrt(f64::NAN).is_nan());
        assert_eq!(cbrt(f64::NEG_INFINITY), f64::NEG_INFINITY);
    }

    #[test]
    fn distances() {
        assert_eq!(hypot(3.0, 4.0), 5.0);
        assert_eq!(hypot(-5.0, 12.0), 13.0);
        assert_eq!(hypot(2f64.powi(1000), 2f64.powi(1000)), core::f64::consts::SQRT_2 * 2f64.powi(1000));
        assert_eq!(hypot(f64::from_bits(3), f64::from_bits(4)), f64::from_bits(5));
        assert_eq!(hypot(f64::MAX, f64::MAX), f64::INFINITY);
        assert_eq!(hypot(f64::INFINITY, f64::NAN), f64::INFINITY);
        assert_eq!(hypot(f64::NAN, f64::NEG_INFINITY), f64::INFINITY);
        assert!(hypot(f64::NAN, 1.0).is_nan());
        assert_eq!(hypot(1.0, 1e-30), 1.0);
        assert_eq!(hypot(0.0, -0.0), 0.0);
    }
}
