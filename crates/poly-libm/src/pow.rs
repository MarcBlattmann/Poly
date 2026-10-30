//! `pow`.
//!
//! `x^y = 2^k e^r` with `y ln|x|` in double-double: `ln|x|` from the log
//! kernel (relative error below 2^-90), its product with `y` (absolute
//! error below 746 × 2^-90 < 2^-80 wherever the result is finite and
//! nonzero), then the exp kernel on that double-double argument. The
//! relative error before the final rounding is below 2^-79, so results are
//! within 0.5 + 2^-26 ulp (target one ulp), and exact powers (`pow(10, 22)`,
//! `pow(2, -1074)`, `pow(x, 2)` when `x²` is a double) come out exact.
//!
//! Special cases follow C11 F.10.4.4: `pow(x, ±0) = 1` and `pow(1, y) = 1`
//! even for NaN, `pow(−1, ±inf) = 1`, the poles at zero keep the sign of
//! zero for odd integral `y`, and a negative finite `x` with a non-integral
//! finite `y` is a NaN (a domain error). Every double of magnitude 2^53 or
//! more is an even integer.

use crate::dd::{self, DD};
use crate::{exp, log};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Parity {
    NotInteger,
    Even,
    Odd,
}

fn parity(y: f64) -> Parity {
    if y.abs() >= 9_007_199_254_740_992.0 {
        return Parity::Even;
    }
    let t = y as i64;
    if t as f64 != y {
        Parity::NotInteger
    } else if t & 1 == 1 {
        Parity::Odd
    } else {
        Parity::Even
    }
}

/// `pow`.
pub fn pow(x: f64, y: f64) -> f64 {
    if y == 0.0 || x == 1.0 {
        return 1.0;
    }
    if x.is_nan() || y.is_nan() {
        return x + y;
    }
    let p = parity(y);
    if y.is_infinite() {
        let ax = x.abs();
        if ax == 1.0 {
            return 1.0;
        }
        return if (ax < 1.0) == (y < 0.0) { f64::INFINITY } else { 0.0 };
    }
    if x == 0.0 {
        let odd = p == Parity::Odd;
        return match (y < 0.0, odd) {
            (true, true) => 1.0 / x,
            (true, false) => f64::INFINITY,
            (false, true) => x,
            (false, false) => 0.0,
        };
    }
    if x.is_infinite() {
        let odd = p == Parity::Odd;
        return match (x > 0.0, y < 0.0) {
            (true, true) => 0.0,
            (true, false) => f64::INFINITY,
            (false, true) => {
                if odd {
                    -0.0
                } else {
                    0.0
                }
            }
            (false, false) => {
                if odd {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                }
            }
        };
    }
    let neg = if x < 0.0 {
        match p {
            Parity::NotInteger => return f64::NAN,
            Parity::Odd => true,
            Parity::Even => false,
        }
    } else {
        false
    };
    let l = log::log_dd(DD::from_f64(x.abs()));
    let rough = y * l.hi;
    let r = if rough > 710.0 {
        f64::INFINITY
    } else if rough < -746.0 {
        0.0
    } else {
        let (m, k) = exp::exp_core(l.mul_f64(y));
        dd::round_scaled(m, k)
    };
    if neg { -r } else { r }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c11_special_cases() {
        let (inf, ninf, nan) = (f64::INFINITY, f64::NEG_INFINITY, f64::NAN);
        assert_eq!(pow(nan, 0.0), 1.0);
        assert_eq!(pow(1.0, nan), 1.0);
        assert_eq!(pow(-1.0, inf), 1.0);
        assert_eq!(pow(-1.0, ninf), 1.0);
        assert!(pow(nan, 1.0).is_nan() && pow(2.0, nan).is_nan());
        assert_eq!(pow(0.5, ninf), inf);
        assert_eq!(pow(2.0, ninf), 0.0);
        assert_eq!(pow(0.5, inf), 0.0);
        assert_eq!(pow(-2.0, inf), inf);
        assert_eq!(pow(0.0, -3.0), inf);
        assert_eq!(pow(-0.0, -3.0), ninf);
        assert_eq!(pow(-0.0, -2.0), inf);
        assert_eq!(pow(-0.0, -0.5), inf);
        assert_eq!(pow(-0.0, 3.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(pow(-0.0, 2.0).to_bits(), 0);
        assert_eq!(pow(ninf, -3.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(pow(ninf, -2.0).to_bits(), 0);
        assert_eq!(pow(ninf, 3.0), ninf);
        assert_eq!(pow(ninf, 2.0), inf);
        assert_eq!(pow(inf, -1.0), 0.0);
        assert_eq!(pow(inf, 0.5), inf);
        assert!(pow(-2.0, 0.5).is_nan());
        assert_eq!(pow(-2.0, 1e300), inf);
        assert_eq!(pow(-0.5, 1e300), 0.0);
    }

    #[test]
    fn exact_powers() {
        assert_eq!(pow(2.0, 10.0), 1024.0);
        assert_eq!(pow(10.0, 22.0), 1e22);
        assert_eq!(pow(10.0, -1.0), 0.1);
        assert_eq!(pow(2.0, -1074.0), 5e-324);
        assert_eq!(pow(2.0, -1075.0), 0.0);
        assert_eq!(pow(2.0, 1023.0), 2f64.powi(1023));
        assert_eq!(pow(2.0, 1024.0), f64::INFINITY);
        assert_eq!(pow(-2.0, 3.0), -8.0);
        assert_eq!(pow(-3.0, 2.0), 9.0);
        assert_eq!(pow(4.0, 0.5), 2.0);
        assert_eq!(pow(2.0, 0.5), core::f64::consts::SQRT_2);
        assert_eq!(pow(27.0, 1.0 / 3.0), 3.0);
        let mut s: u64 = 0x5555_AAAA_3333_CCCC;
        for _ in 0..5000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let x = f64::from_bits((s >> 12) | 0x3FF0_0000_0000_0000) * 2f64.powi((s % 400) as i32 - 200);
            assert_eq!(pow(x, 1.0), x);
            assert_eq!(pow(x, 2.0), x * x);
            assert_eq!(pow(x, -1.0), 1.0 / x);
        }
    }
}
