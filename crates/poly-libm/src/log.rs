//! The logarithms: `log`, `log2`, `log10`, `log1p`.
//!
//! **Reduction.** `x = 2^k m` with `m ∈ [√½, √2)`, then
//! `ln m = 2 atanh(s) = 2 (s + s³/3 + s⁵/5 + …)` with `s = (m − 1)/(m + 1)`,
//! `|s| ≤ 0.1716`. `m − 1` is exact (Sterbenz), the quotient is a
//! double-double division, so `s` is good to 2^-104 relative and nothing
//! cancels near `x = 1`.
//!
//! **Kernel.** In `z = s²` (≤ 0.0295) the series is `Σ z^j / (2j + 1)`;
//! the first omitted term (`j = 19`) is below 2^-96. Terms up to `j = 7`
//! are summed in double-double, the rest (each below 0.0295^8 < 2^-40) in
//! `f64`. Then `ln x = k ln 2 + 2 s Σ` in double-double: relative error
//! below 2^-90 before the one final rounding, so the result is within
//! 0.5 + 2^-37 ulp (target one ulp). `log2` adds `k` exactly to
//! `log2(e) ln m`, so powers of two give exact integers, and `log10`
//! multiplies by log10(e) in double-double (so `log10(10^n) = n` exactly
//! for every power of ten a double holds exactly). `log1p` takes `1 + x`
//! exactly as a double-double.

use crate::consts::{INV_ODD, LN2, LOG2E, LOG10E};
use crate::dd::{self, DD, two_sum};

/// `ln m` for a double-double `m ∈ [√½, √2)`.
fn log_reduced(m: DD) -> DD {
    const DDC: [DD; 8] = [INV_ODD[0], INV_ODD[1], INV_ODD[2], INV_ODD[3], INV_ODD[4], INV_ODD[5], INV_ODD[6], INV_ODD[7]];
    const TAIL: [f64; 11] = [
        INV_ODD[8].hi,
        INV_ODD[9].hi,
        INV_ODD[10].hi,
        INV_ODD[11].hi,
        INV_ODD[12].hi,
        INV_ODD[13].hi,
        INV_ODD[14].hi,
        INV_ODD[15].hi,
        INV_ODD[16].hi,
        INV_ODD[17].hi,
        INV_ODD[18].hi,
    ];
    let num = two_sum(m.hi - 1.0, m.lo);
    let den = m.add_f64(1.0);
    let s = num.div(den);
    let z = s.square();
    dd::poly_mixed(z, &DDC, &TAIL).mul(s).mul_f64(2.0)
}

/// `x = 2^k m`, `m ∈ [√½, √2)` (double-double, positive finite `x`).
fn split(x: DD) -> (DD, i32) {
    let mut k = dd::exponent_of(x.hi);
    let mut m = DD::new(dd::ldexp_exact(x.hi, -k), dd::ldexp_exact(x.lo, -k));
    if m.hi > core::f64::consts::SQRT_2 {
        m = m.scale(-1);
        k += 1;
    }
    (m, k)
}

/// `ln x` in double-double for a positive finite double-double `x`.
pub(crate) fn log_dd(x: DD) -> DD {
    let (m, k) = split(x);
    LN2.mul_f64(f64::from(k)).add(log_reduced(m))
}

/// The common special cases: `Some(result)` for NaN, zero, negative and
/// infinite `x`.
fn special(x: f64) -> Option<f64> {
    if x.is_nan() {
        Some(x + x)
    } else if x == 0.0 {
        Some(f64::NEG_INFINITY)
    } else if x < 0.0 {
        Some(f64::NAN)
    } else if x == f64::INFINITY {
        Some(x)
    } else {
        None
    }
}

/// `log`: the natural logarithm.
pub fn log(x: f64) -> f64 {
    if let Some(r) = special(x) {
        return r;
    }
    if x == 1.0 {
        return 0.0;
    }
    log_dd(DD::from_f64(x)).to_f64()
}

/// `log2`.
pub fn log2(x: f64) -> f64 {
    if let Some(r) = special(x) {
        return r;
    }
    let (m, k) = split(DD::from_f64(x));
    if m.hi == 1.0 && m.lo == 0.0 {
        return f64::from(k);
    }
    log_reduced(m).mul(LOG2E).add_f64(f64::from(k)).to_f64()
}

/// `log10`.
pub fn log10(x: f64) -> f64 {
    if let Some(r) = special(x) {
        return r;
    }
    if x == 1.0 {
        return 0.0;
    }
    log_dd(DD::from_f64(x)).mul(LOG10E).to_f64()
}

/// `log1p`: `ln(1 + x)`, accurate near zero.
pub fn log1p(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    if x == -1.0 {
        return f64::NEG_INFINITY;
    }
    if x < -1.0 {
        return f64::NAN;
    }
    if x == f64::INFINITY {
        return x;
    }
    if x.abs() < 1e-17 {
        return x; // x²/2 is below half an ulp of x
    }
    let u = if x.abs() > 1.0 { two_sum(x, 1.0) } else { two_sum(1.0, x) };
    log_dd(u).to_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_values_and_special_cases() {
        assert_eq!(log(1.0).to_bits(), 0);
        assert_eq!(log(core::f64::consts::E), 1.0);
        assert_eq!(log(0.0), f64::NEG_INFINITY);
        assert_eq!(log(-0.0), f64::NEG_INFINITY);
        assert!(log(-1.0).is_nan() && log(f64::NAN).is_nan() && log(f64::NEG_INFINITY).is_nan());
        assert_eq!(log(f64::INFINITY), f64::INFINITY);
        assert_eq!(log(2.0), core::f64::consts::LN_2);
        assert_eq!(log(10.0), core::f64::consts::LN_10);
        assert_eq!(log(5e-324), -744.4400719213812);
        assert_eq!(log(f64::MAX), 709.782712893384);
        for k in -1074..1024 {
            assert_eq!(log2(dd::ldexp_exact(1.0, k)), f64::from(k), "log2(2^{k})");
        }
        let mut p = 1.0;
        for n in 0..23 {
            assert_eq!(log10(p), f64::from(n), "log10(1e{n})");
            p *= 10.0;
        }
        assert_eq!(log10(0.001), -3.0);
        assert_eq!(log1p(0.0).to_bits(), 0);
        assert_eq!(log1p(-0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(log1p(-1.0), f64::NEG_INFINITY);
        assert!(log1p(-2.0).is_nan());
        assert_eq!(log1p(1e-300), 1e-300);
        assert_eq!(log1p(1.0), core::f64::consts::LN_2);
        assert_eq!(log1p(f64::MAX), 709.782712893384);
    }

    #[test]
    fn near_one_and_inverse_of_exp() {
        let mut s: u64 = 0xDEAD_BEEF_CAFE_F00D;
        for _ in 0..20000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let x = f64::from_bits((s >> 2) | 0x0010_0000_0000_0000) % 1e300 + 1e-300;
            let l = log(x);
            // exp(log x) = x within the rounding of log (|log x| ≤ 710 makes
            // its rounding worth up to 710 ulp of x... relative 2^-52 × |log x|).
            let back = crate::exp::exp(l);
            assert!((back - x).abs() <= x * f64::EPSILON * (l.abs() + 1.0), "{x}");
            let d = 1e-10 * ((s >> 40) as f64 / (1u64 << 24) as f64 - 0.5);
            // log1p(d) ≈ d − d²/2 to far more than double precision.
            let want = d - d * d / 2.0 + d * d * d / 3.0;
            assert!((log1p(d) - want).abs() <= want.abs() * 2.0 * f64::EPSILON, "{d}");
        }
    }
}
