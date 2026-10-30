//! `remainder`: IEEE 754's remainder, `x − n y` with `n` the integer
//! nearest `x / y` (ties to even). The result is always exact (IEEE
//! 754-2019, 5.3.1), so this works on the integer significands: the
//! remainder of `mx × 2^(ex − ey)` modulo `my`, one bit of the quotient at a
//! time, keeping the quotient's last bit for the tie.
//!
//! `remainder(x, ±0)` and `remainder(±inf, y)` are NaN (domain errors);
//! `remainder(x, ±inf) = x`; a zero result has the sign of `x` (C11
//! F.10.7.2).

/// `x = m × 2^e` with `m` an integer below 2^53 (finite nonzero `x`).
fn parts(x: f64) -> (u64, i32) {
    let b = x.to_bits();
    let e = ((b >> 52) & 0x7FF) as i32;
    let frac = b & ((1 << 52) - 1);
    if e == 0 { (frac, -1074) } else { (frac | (1 << 52), e - 1075) }
}

/// `remainder`.
pub fn remainder(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return x + y;
    }
    if x.is_infinite() || y == 0.0 {
        return f64::NAN;
    }
    if y.is_infinite() || x == 0.0 {
        return x;
    }
    let (mx, ex) = parts(x);
    let (my, ey) = parts(y);
    let r = if ex < ey {
        // |x| < |y| · 2 at most: n is 0 or 1 by comparing 2|x| with |y|.
        let d = (ey - ex) as u32;
        if d > 60 {
            return x;
        }
        let twice = u128::from(mx) << 1;
        let ybig = u128::from(my) << d;
        if twice <= ybig {
            return x; // n = 0 (a tie also goes to 0, the even one)
        }
        // |x| − |y|, exact (Sterbenz: |y|/2 < |x| < |y|).
        let v = x.abs() - y.abs();
        return if x < 0.0 { -v } else { v };
    } else {
        let mut r = mx % my;
        let mut odd = (mx / my) & 1 == 1;
        for _ in 0..(ex - ey) {
            r <<= 1;
            odd = r >= my;
            if odd {
                r -= my;
            }
        }
        // r < my; round the quotient to nearest-even.
        let twice = r << 1;
        if twice > my || (twice == my && odd) {
            -((my - r) as f64)
        } else {
            r as f64
        }
    };
    // r × 2^ey is exact (an integer below 2^53 at an exponent ≥ −1074).
    let v = crate::dd::ldexp_exact(r, ey);
    if x < 0.0 { -v } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_quotients() {
        assert_eq!(remainder(5.0, 2.0), 1.0); // 5/2 = 2.5 → 2
        assert_eq!(remainder(7.0, 2.0), -1.0); // 3.5 → 4
        assert_eq!(remainder(-7.0, 2.0), 1.0);
        assert_eq!(remainder(6.0, 4.0), -2.0); // 1.5 → 2
        assert_eq!(remainder(2.0, 4.0), 2.0); // 0.5 → 0
        assert_eq!(remainder(3.0, 4.0), -1.0);
        assert_eq!(remainder(1.0, 4.0), 1.0);
        assert_eq!(remainder(4.0, 2.0).to_bits(), 0);
        assert_eq!(remainder(-4.0, 2.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(remainder(1e300, 3.0), remainder(1e300 % 3.0, 3.0));
        assert_eq!(remainder(5e-324, 1e-323), 5e-324);
        assert_eq!(remainder(1.5e-323, 1e-323), -5e-324);
        assert_eq!(remainder(3.0, f64::INFINITY), 3.0);
        assert!(remainder(1.0, 0.0).is_nan() && remainder(f64::INFINITY, 1.0).is_nan());
        assert_eq!(remainder(-0.0, 1.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(remainder(1.0, 3e-300), remainder(1.0 % 3e-300, 3e-300));
    }

    #[test]
    fn agrees_with_its_definition() {
        let mut s: u64 = 0x1357_9BDF_2468_ACE0;
        for _ in 0..20000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let x = ((s >> 34) as f64) - 2f64.powi(29);
            let y = ((s & 0xFFFF) as f64 + 1.0) * if s & 0x10000 == 0 { 1.0 } else { -1.0 };
            // Small integers: n = round-half-even(x / y) exactly in f64.
            let q = x / y;
            let mut n = q.round();
            if (q - q.trunc()).abs() == 0.5 && n % 2.0 != 0.0 {
                n -= q.signum();
            }
            let want = x - n * y;
            let got = remainder(x, y);
            assert_eq!(got, want, "remainder({x}, {y})");
            assert!(got.abs() <= y.abs() / 2.0);
        }
    }
}
