//! The inverse circular functions: `atan`, `atan2`, `asin`, `acos`.
//!
//! All four go through one double-double kernel, `atan_dd(t)` for `t ≥ 0`:
//! `atan t = π/2 − atan(1/t)` above 1, then two halvings
//! `atan t = 2 atan(t / (1 + √(1 + t²)))` bring `t` below tan(π/16) =
//! 0.199, where `atan u = u Σ (−1)^n u^(2n)/(2n+1)` for `n ≤ 20` (first
//! omitted term below 2^-93), terms up to `n = 8` in double-double, the
//! rest (below 2^-38) in `f64`. Every step is a double-double operation,
//! so the kernel's relative error stays below 2^-95.
//!
//! `asin x = atan(x / √((1 − x)(1 + x)))` and `acos x = atan(√((1 −
//! x)(1 + x)) / x)` (`π −` that for negative `x`): `1 − |x|` is exact for
//! `|x| ≥ ½` (Sterbenz) and the products and roots are double-double, so
//! nothing cancels near ±1. `atan2` divides in double-double after scaling
//! both arguments by the same power of two, and follows C11 F.10.1.4 for
//! zeros and infinities. Results are within 0.5 + 2^-40 ulp (target one
//! ulp).

use crate::consts::{INV_ODD, PI, PI_2, PI_4};
use crate::dd::{self, DD, two_sum};

const fn alt(n: usize, d: DD) -> DD {
    if n % 2 == 1 { d.neg() } else { d }
}

const ATAN_DD: [DD; 9] = [
    alt(0, INV_ODD[0]),
    alt(1, INV_ODD[1]),
    alt(2, INV_ODD[2]),
    alt(3, INV_ODD[3]),
    alt(4, INV_ODD[4]),
    alt(5, INV_ODD[5]),
    alt(6, INV_ODD[6]),
    alt(7, INV_ODD[7]),
    alt(8, INV_ODD[8]),
];
const ATAN_TAIL: [f64; 12] = [
    alt(9, INV_ODD[9]).hi,
    alt(10, INV_ODD[10]).hi,
    alt(11, INV_ODD[11]).hi,
    alt(12, INV_ODD[12]).hi,
    alt(13, INV_ODD[13]).hi,
    alt(14, INV_ODD[14]).hi,
    alt(15, INV_ODD[15]).hi,
    alt(16, INV_ODD[16]).hi,
    alt(17, INV_ODD[17]).hi,
    alt(18, INV_ODD[18]).hi,
    alt(19, INV_ODD[19]).hi,
    alt(20, INV_ODD[20]).hi,
];

/// `atan t` for a double-double `t ≥ 0` (finite, at most about 2^900).
pub(crate) fn atan_dd(t: DD) -> DD {
    if t.hi > 1.0 {
        return PI_2.sub(atan_dd(t.recip()));
    }
    if t.hi == 1.0 && t.lo == 0.0 {
        return PI_4;
    }
    let mut u = t;
    for _ in 0..2 {
        u = u.div(u.square().add_f64(1.0).sqrt().add_f64(1.0));
    }
    dd::poly_mixed(u.square(), &ATAN_DD, &ATAN_TAIL).mul(u).mul_f64(4.0)
}

/// `atan`.
pub fn atan(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    let a = x.abs();
    let r = if a < 1e-8 {
        return x; // x³/3 is below half an ulp of x
    } else if a > 1e300 {
        PI_2.hi // π/2 − 1/x rounds to π/2
    } else {
        atan_dd(DD::from_f64(a)).to_f64()
    };
    if x < 0.0 { -r } else { r }
}

/// `atan2(y, x)`: the angle of `(x, y)` in (−π, π].
pub fn atan2(y: f64, x: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return x + y;
    }
    let neg = y.is_sign_negative();
    let sign = |v: f64| if neg { -v } else { v };
    if y == 0.0 {
        // ±0 for x > 0 or +0, ±π for x < 0 or −0.
        return if x.is_sign_negative() { sign(PI.hi) } else { y };
    }
    if x == 0.0 {
        return sign(PI_2.hi);
    }
    if y.is_infinite() {
        return sign(match x {
            f64::INFINITY => PI_4.hi,
            f64::NEG_INFINITY => PI.sub(PI_4).hi,
            _ => PI_2.hi,
        });
    }
    if x.is_infinite() {
        return if x > 0.0 { sign(0.0) } else { sign(PI.hi) };
    }
    let (ay, ax) = (y.abs(), x.abs());
    let (ey, ex) = (dd::exponent_of(ay), dd::exponent_of(ax));
    let a = if ey - ex > 60 {
        // |y/x| > 2^59: π/2 − x/y within far less than an ulp.
        PI_2.sub(DD::from_f64(ax / ay))
    } else if ex - ey > 60 {
        if x > 0.0 {
            // atan(q) = q (1 − q²/3 …) with q² < 2^-118: the correctly
            // rounded quotient (which may underflow, as it should).
            return sign(ay / ax);
        }
        DD::from_f64(dd::ldexp_exact(ay, -ex) / dd::ldexp_exact(ax, -ex))
    } else {
        let q = DD::from_f64(dd::ldexp_exact(ay, -ex)).div(DD::from_f64(dd::ldexp_exact(ax, -ex)));
        atan_dd(q)
    };
    let r = if x < 0.0 { PI.sub(a) } else { a };
    sign(r.to_f64())
}

/// `√((1 − a)(1 + a))` for `0 ≤ a < 1`, in double-double.
fn cofactor(a: f64) -> DD {
    let one_minus = two_sum(1.0, -a);
    let one_plus = two_sum(1.0, a);
    one_minus.mul(one_plus).sqrt()
}

/// `asin`; NaN for `|x| > 1`.
pub fn asin(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    let a = x.abs();
    if a > 1.0 {
        return f64::NAN;
    }
    let r = if a < 1e-8 {
        return x; // x³/6 is below half an ulp of x
    } else if a == 1.0 {
        PI_2.hi
    } else {
        atan_dd(DD::from_f64(a).div(cofactor(a))).to_f64()
    };
    if x < 0.0 { -r } else { r }
}

/// `acos`; NaN for `|x| > 1`.
pub fn acos(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    let a = x.abs();
    if a > 1.0 {
        return f64::NAN;
    }
    if x == 1.0 {
        return 0.0;
    }
    if x == -1.0 {
        return PI.hi;
    }
    if a < 1e-17 {
        return PI_2.hi; // π/2 − x rounds to π/2
    }
    let t = atan_dd(cofactor(a).div(DD::from_f64(a)));
    if x < 0.0 { PI.sub(t).to_f64() } else { t.to_f64() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI as P};

    #[test]
    fn known_values() {
        assert_eq!(atan(1.0), FRAC_PI_4);
        assert_eq!(atan(-1.0), -FRAC_PI_4);
        assert_eq!(atan(f64::INFINITY), FRAC_PI_2);
        assert_eq!(atan(-0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(asin(0.5), 0.5235987755982989);
        assert_eq!(asin(1.0), FRAC_PI_2);
        assert_eq!(asin(-1.0), -FRAC_PI_2);
        assert!(asin(1.0000000000000002).is_nan() && acos(-2.0).is_nan());
        assert_eq!(acos(1.0).to_bits(), 0);
        assert_eq!(acos(-1.0), P);
        assert_eq!(acos(0.0), FRAC_PI_2);
        assert_eq!(acos(0.5), 1.0471975511965979);
        assert_eq!(atan(3f64.sqrt()), 1.0471975511965976);
    }

    #[test]
    fn atan2_special_cases() {
        let (z, nz, inf, ninf) = (0.0f64, -0.0f64, f64::INFINITY, f64::NEG_INFINITY);
        assert_eq!(atan2(z, z).to_bits(), 0);
        assert_eq!(atan2(nz, z).to_bits(), nz.to_bits());
        assert_eq!(atan2(z, nz), P);
        assert_eq!(atan2(nz, nz), -P);
        assert_eq!(atan2(z, -1.0), P);
        assert_eq!(atan2(nz, -1.0), -P);
        assert_eq!(atan2(1.0, z), FRAC_PI_2);
        assert_eq!(atan2(-1.0, nz), -FRAC_PI_2);
        assert_eq!(atan2(inf, inf), FRAC_PI_4);
        assert_eq!(atan2(ninf, ninf), -2.356194490192345);
        assert_eq!(atan2(inf, 5.0), FRAC_PI_2);
        assert_eq!(atan2(1.0, inf).to_bits(), 0);
        assert_eq!(atan2(-1.0, ninf), -P);
        assert_eq!(atan2(1.0, 1.0), FRAC_PI_4);
        assert_eq!(atan2(1.0, -1.0), 2.356194490192345);
        assert_eq!(atan2(5e-324, 1e300), 0.0);
        assert_eq!(atan2(2f64.powi(-1000), 2f64.powi(20)), 2f64.powi(-1020));
        assert_eq!(atan2(1e300, 1e-300), FRAC_PI_2);
        assert!(atan2(f64::NAN, 1.0).is_nan());
    }

    #[test]
    fn inverses() {
        let mut s: u64 = 0xA5A5_5A5A_1234_8765;
        for _ in 0..20000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let x = (s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
            // sin(asin x) = x and cos(acos x) = x to a few ulps of 1.
            assert!((crate::trig::sin(asin(x)) - x).abs() <= 2.0 * f64::EPSILON, "{x}");
            assert!((crate::trig::cos(acos(x)) - x).abs() <= 2.0 * f64::EPSILON, "{x}");
            // asin + acos = π/2.
            assert!((asin(x) + acos(x) - FRAC_PI_2).abs() <= 2.0 * f64::EPSILON, "{x}");
            let t = x * 1e3;
            assert!((atan2(t, 1.0) - atan(t)).abs() <= f64::EPSILON * atan(t).abs(), "{t}");
        }
    }
}
