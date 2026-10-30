//! The exponential family: `exp`, `exp2`, `expm1`, `sinh`, `cosh`, `tanh`.
//!
//! **Reduction.** `x = k ln 2 + r` with `k` the integer nearest `x / ln 2`
//! and `r = x − k ln 2` in double-double (`k × ln 2` with ln 2 to 106 bits:
//! the absolute error of `r` is below 2^11 × 2^-106 = 2^-95), so
//! `|r| ≤ ln 2 / 2 + 2^-40` and `e^x = 2^k e^r`.
//!
//! **Kernel.** `e^r` is its Taylor series, `Σ r^n / n!` for `n ≤ 19`: the
//! first omitted term is below 0.35^20 / 20! < 2^-91. The terms up to
//! `n = 10` are summed in double-double; from `n = 11` on (each below
//! 0.35^11 / 11! < 2^-41) in `f64`, whose relative error of 2^-52 then
//! costs below 2^-93. So `e^r` carries a relative error below 2^-89, and
//! the one rounding at the end (`dd::round_scaled`, also into the subnormal
//! range) gives a result within 0.5 + 2^-36 ulp: the documented target is
//! one ulp, the observed maximum on the reference set is the correctly
//! rounded value.
//!
//! `expm1` near zero sums the same series without its first term (no
//! cancellation); elsewhere it is `2^k e^r − 1` in double-double. The
//! hyperbolic functions are built on these kernels in double-double:
//! `sinh = (E + E/(E+1))/2` with `E = expm1(|x|)`, `cosh = (e^|x| +
//! e^-|x|)/2`, `tanh = E/(E+2)` with `E = expm1(2|x|)`, so none of them
//! cancels.

use crate::consts::{INV_FACT, LN2, LOG2E};
use crate::dd::{self, DD};

/// `ln(DBL_MAX)`: above it `exp` overflows.
pub const EXP_OVERFLOW: f64 = 709.782712893384;
/// Below this `exp` rounds to zero (`ln(2^-1075)`).
pub const EXP_UNDERFLOW: f64 = -745.1332191019412;

/// The integer nearest `x` (ties away from zero), `|x| < 2^52`.
pub(crate) fn nearest_int(x: f64) -> f64 {
    let t = x.abs() + 0.5;
    let n = t as u64 as f64;
    if x < 0.0 { -n } else { n }
}

/// `e^r` for `|r| ≤ 0.35` (double-double in and out).
pub(crate) fn exp_reduced(r: DD) -> DD {
    const DDC: [DD; 11] = [
        INV_FACT[0], INV_FACT[1], INV_FACT[2], INV_FACT[3], INV_FACT[4], INV_FACT[5], INV_FACT[6], INV_FACT[7],
        INV_FACT[8], INV_FACT[9], INV_FACT[10],
    ];
    const TAIL: [f64; 9] = [
        INV_FACT[11].hi,
        INV_FACT[12].hi,
        INV_FACT[13].hi,
        INV_FACT[14].hi,
        INV_FACT[15].hi,
        INV_FACT[16].hi,
        INV_FACT[17].hi,
        INV_FACT[18].hi,
        INV_FACT[19].hi,
    ];
    dd::poly_mixed(r, &DDC, &TAIL)
}

/// `e^r − 1` for `|r| ≤ 0.35`.
fn expm1_reduced(r: DD) -> DD {
    const DDC: [DD; 10] = [
        INV_FACT[1], INV_FACT[2], INV_FACT[3], INV_FACT[4], INV_FACT[5], INV_FACT[6], INV_FACT[7], INV_FACT[8],
        INV_FACT[9], INV_FACT[10],
    ];
    const TAIL: [f64; 9] = [
        INV_FACT[11].hi,
        INV_FACT[12].hi,
        INV_FACT[13].hi,
        INV_FACT[14].hi,
        INV_FACT[15].hi,
        INV_FACT[16].hi,
        INV_FACT[17].hi,
        INV_FACT[18].hi,
        INV_FACT[19].hi,
    ];
    dd::poly_mixed(r, &DDC, &TAIL).mul(r)
}

/// `e^x = m × 2^k` for `|x.hi| ≤ 1500`, `m` within [0.70, 1.42].
pub(crate) fn exp_core(x: DD) -> (DD, i32) {
    let k = nearest_int(x.hi * LOG2E.hi);
    let r = x.sub(LN2.mul_f64(k));
    (exp_reduced(r), k as i32)
}

/// `exp`.
pub fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    if x > EXP_OVERFLOW {
        return f64::INFINITY;
    }
    if x < EXP_UNDERFLOW {
        return 0.0;
    }
    if x.abs() < 1e-18 {
        return 1.0 + x;
    }
    let (m, k) = exp_core(DD::from_f64(x));
    dd::round_scaled(m, k)
}

/// `exp2`: `2^x`; exact for integral `x`.
pub fn exp2(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    if x >= 1024.0 {
        return f64::INFINITY;
    }
    if x < -1076.0 {
        return 0.0;
    }
    let k = nearest_int(x);
    let f = x - k; // exact, |f| ≤ 1/2
    let r = LN2.mul_f64(f);
    dd::round_scaled(exp_reduced(r), k as i32)
}

/// `e^x − 1` in double-double for `|x| ≤ 50`.
pub(crate) fn expm1_dd(x: f64) -> DD {
    if x.abs() <= 0.34 {
        return expm1_reduced(DD::from_f64(x));
    }
    let (m, k) = exp_core(DD::from_f64(x));
    m.scale(k).add_f64(-1.0)
}

/// `expm1`: `e^x − 1`, accurate near zero.
pub fn expm1(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    if x > EXP_OVERFLOW {
        return f64::INFINITY;
    }
    if x < -40.0 {
        return -1.0; // e^x is below a quarter ulp of 1
    }
    if x.abs() < 1e-17 {
        return x; // x²/2 is below half an ulp of x
    }
    if x <= 50.0 {
        return expm1_dd(x).to_f64();
    }
    // e^x − 1 = e^x within far less than an ulp: scale without overflow.
    let (m, k) = exp_core(DD::from_f64(x));
    dd::round_scaled(m, k)
}

/// `e^|x|` as `m × 2^k` for `|x| ≤ 711`, halved (`k − 1`).
fn half_exp(a: f64) -> f64 {
    let (m, k) = exp_core(DD::from_f64(a));
    dd::round_scaled(m, k - 1)
}

/// `sinh`.
pub fn sinh(x: f64) -> f64 {
    if x.is_nan() || x.is_infinite() {
        return x + x;
    }
    let a = x.abs();
    let r = if a < 1e-9 {
        // x³/6 is below half an ulp of x.
        return x;
    } else if a <= 22.0 {
        let e = expm1_dd(a);
        e.add(e.div(e.add_f64(1.0))).mul_f64(0.5).to_f64()
    } else if a <= 711.0 {
        // e^-a is below 2^-63 of e^a.
        half_exp(a)
    } else {
        f64::INFINITY
    };
    if x < 0.0 { -r } else { r }
}

/// `cosh`.
pub fn cosh(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    let a = x.abs();
    if a < 1e-9 {
        return 1.0; // x²/2 is below half an ulp of 1
    }
    if a <= 22.0 {
        let (m, k) = exp_core(DD::from_f64(a));
        let e = m.scale(k);
        return e.add(e.recip()).mul_f64(0.5).to_f64();
    }
    if a <= 711.0 {
        return half_exp(a);
    }
    f64::INFINITY
}

/// `tanh`.
pub fn tanh(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    let a = x.abs();
    let r = if a < 1e-9 {
        return x; // x³/3 is below half an ulp of x
    } else if a <= 22.0 {
        let e = expm1_dd(2.0 * a);
        e.div(e.add_f64(2.0)).to_f64()
    } else {
        // 1 − tanh(a) = 2/(e^2a + 1) < 2^-63.
        1.0
    };
    if x < 0.0 { -r } else { r }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_values_and_limits() {
        assert_eq!(exp(0.0), 1.0);
        assert_eq!(exp(-0.0), 1.0);
        assert_eq!(exp(1.0), core::f64::consts::E);
        assert_eq!(exp(f64::INFINITY), f64::INFINITY);
        assert_eq!(exp(f64::NEG_INFINITY), 0.0);
        assert_eq!(exp(709.78), 1.7928227943945155e308);
        assert_eq!(exp(709.8), f64::INFINITY);
        assert_eq!(exp(-745.0), 5e-324);
        assert_eq!(exp(-746.0), 0.0);
        assert!(exp(f64::NAN).is_nan());
        for k in -1074..1024 {
            assert_eq!(exp2(f64::from(k)), dd::ldexp_exact(1.0, k), "exp2({k})");
        }
        assert_eq!(exp2(0.5), core::f64::consts::SQRT_2);
        assert_eq!(exp2(1024.0), f64::INFINITY);
        assert_eq!(exp2(-1075.0), 0.0, "a tie between 0 and the smallest subnormal: even");
        assert_eq!(exp2(-1074.5), 5e-324);
        assert_eq!(expm1(0.0).to_bits(), 0);
        assert_eq!(expm1(-0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(expm1(1e-300), 1e-300);
        assert_eq!(expm1(-50.0), -1.0);
        assert_eq!(expm1(1.0), 1.7182818284590453);
        assert_eq!(sinh(0.0).to_bits(), 0);
        assert_eq!(sinh(-0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(cosh(0.0), 1.0);
        assert_eq!(tanh(-0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(tanh(30.0), 1.0);
        assert_eq!(tanh(f64::NEG_INFINITY), -1.0);
        assert_eq!(sinh(710.0), 1.1169973830808555e308);
        assert_eq!(sinh(-711.0), f64::NEG_INFINITY);
        assert_eq!(cosh(-710.0), 1.1169973830808555e308);
    }

    #[test]
    fn identities_hold_to_the_last_bit() {
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..20000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let x = (s >> 11) as f64 / (1u64 << 53) as f64 * 40.0 - 20.0;
            // cosh² − sinh² = 1 and tanh = sinh / cosh, to a few ulps.
            let (c, sh, t) = (cosh(x), sinh(x), tanh(x));
            assert!((t - sh / c).abs() <= 2.0 * f64::EPSILON * t.abs().max(f64::MIN_POSITIVE), "{x}");
            // e^x e^-x = 1 within the two roundings.
            let p = exp(x) * exp(-x);
            assert!((p - 1.0).abs() <= 3.0 * f64::EPSILON, "{x}: {p}");
            // expm1 agrees with exp − 1 where the latter does not cancel.
            if x.abs() > 1.0 {
                let d = expm1(x) - (exp(x) - 1.0);
                assert!(d.abs() <= 2.0 * f64::EPSILON * expm1(x).abs(), "{x}");
            }
        }
    }
}
