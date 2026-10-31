//! Double-double arithmetic: a value is the unevaluated sum `hi + lo` of
//! two `f64`s with `|lo| ≤ ulp(hi) / 2`, about 106 significant bits.
//!
//! The error-free transformations are the classic ones of the numerical
//! analysis literature: Knuth's two-sum (TAOCP vol. 2, 4.2.2, theorem B),
//! Dekker's fast two-sum and his product by Veltkamp splitting ("A
//! floating-point technique for extending the available precision", 1971).
//! Without a fused multiply-add the product splits each factor into two
//! 26-bit halves, so all four partial products are exact. Each operation
//! below loses at most a few units of 2^-104 relative to its result; the
//! callers' error budgets (each module's header) count on 2^-100.
//!
//! The splitting overflows for factors above about 2^996; every caller
//! keeps its operands far below that.

/// `hi + lo`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DD {
    pub hi: f64,
    pub lo: f64,
}

/// 2^27 + 1: Veltkamp's splitting constant for 53-bit significands.
const SPLITTER: f64 = 134_217_729.0;

// Named methods rather than operator traits: every call site reads as
// the error-free arithmetic it is, and `const fn`s stay possible.
#[allow(clippy::should_implement_trait)]
impl DD {
    pub const ZERO: DD = DD { hi: 0.0, lo: 0.0 };
    pub const ONE: DD = DD { hi: 1.0, lo: 0.0 };

    pub const fn new(hi: f64, lo: f64) -> DD {
        DD { hi, lo }
    }

    pub const fn from_bits(hi: u64, lo: u64) -> DD {
        DD { hi: f64::from_bits(hi), lo: f64::from_bits(lo) }
    }

    pub const fn from_f64(x: f64) -> DD {
        DD { hi: x, lo: 0.0 }
    }

    pub const fn neg(self) -> DD {
        DD { hi: -self.hi, lo: -self.lo }
    }

    /// The nearest `f64` (the pair is normalised, so `hi` already is).
    pub fn to_f64(self) -> f64 {
        self.hi + self.lo
    }

    /// Multiply by `2^n` (exact while both parts stay normal).
    pub fn scale(self, n: i32) -> DD {
        let s = pow2(n);
        DD { hi: self.hi * s, lo: self.lo * s }
    }

    pub fn add(self, b: DD) -> DD {
        let s = two_sum(self.hi, b.hi);
        let t = two_sum(self.lo, b.lo);
        let r = fast_two_sum(s.hi, s.lo + t.hi);
        fast_two_sum(r.hi, r.lo + t.lo)
    }

    pub fn sub(self, b: DD) -> DD {
        self.add(b.neg())
    }

    pub fn add_f64(self, b: f64) -> DD {
        let s = two_sum(self.hi, b);
        fast_two_sum(s.hi, s.lo + self.lo)
    }

    pub fn mul(self, b: DD) -> DD {
        let p = two_prod(self.hi, b.hi);
        fast_two_sum(p.hi, p.lo + (self.hi * b.lo + self.lo * b.hi))
    }

    pub fn mul_f64(self, b: f64) -> DD {
        let p = two_prod(self.hi, b);
        fast_two_sum(p.hi, p.lo + self.lo * b)
    }

    pub fn square(self) -> DD {
        let p = two_prod(self.hi, self.hi);
        fast_two_sum(p.hi, p.lo + 2.0 * self.hi * self.lo)
    }

    /// Long division with three partial quotients.
    pub fn div(self, b: DD) -> DD {
        let q1 = self.hi / b.hi;
        let r = self.sub(b.mul_f64(q1));
        let q2 = r.hi / b.hi;
        let r = r.sub(b.mul_f64(q2));
        let q3 = r.hi / b.hi;
        fast_two_sum(q1, q2).add_f64(q3)
    }

    /// `1 / self`.
    pub fn recip(self) -> DD {
        DD::ONE.div(self)
    }

    /// The square root of a positive value: one Newton step from the
    /// correctly rounded root of `hi` (Karp and Markstein's form).
    pub fn sqrt(self) -> DD {
        if self.hi <= 0.0 {
            return DD::ZERO;
        }
        let s = crate::roots::sqrt(self.hi);
        let r = self.sub(two_prod(s, s));
        fast_two_sum(s, r.hi / (2.0 * s))
    }
}

/// Knuth's two-sum: `a + b = s + e` exactly.
#[inline]
pub fn two_sum(a: f64, b: f64) -> DD {
    let s = a + b;
    let bb = s - a;
    let e = (a - (s - bb)) + (b - bb);
    DD { hi: s, lo: e }
}

/// Dekker's fast two-sum, for `|a| ≥ |b|` (or `a` zero).
#[inline]
pub fn fast_two_sum(a: f64, b: f64) -> DD {
    let s = a + b;
    let e = b - (s - a);
    DD { hi: s, lo: e }
}

#[inline]
fn split(a: f64) -> (f64, f64) {
    let t = SPLITTER * a;
    let hi = t - (t - a);
    (hi, a - hi)
}

/// Dekker's product: `a × b = p + e` exactly (no overflow, no underflow).
#[inline]
pub fn two_prod(a: f64, b: f64) -> DD {
    let p = a * b;
    let (ah, al) = split(a);
    let (bh, bl) = split(b);
    let e = ((ah * bh - p) + ah * bl + al * bh) + al * bl;
    DD { hi: p, lo: e }
}

/// `2^n` for `-1022 ≤ n ≤ 1023` exactly; outside, the product of two such
/// powers (callers multiply a value by it, so `scale` handles any `n`
/// whose result is representable).
pub fn pow2(n: i32) -> f64 {
    if (-1022..=1023).contains(&n) {
        f64::from_bits(((n + 1023) as u64) << 52)
    } else if n > 1023 {
        f64::INFINITY
    } else {
        0.0
    }
}

/// `x × 2^n` for an `f64`, exact when the result is normal (done in steps
/// so no intermediate power overflows).
pub fn ldexp_exact(mut x: f64, mut n: i32) -> f64 {
    while n > 1000 {
        x *= pow2(1000);
        n -= 1000;
    }
    while n < -1000 {
        x *= pow2(-1000);
        n += 1000;
    }
    x * pow2(n)
}

/// The `f64` nearest to `m × 2^k` (round to nearest, ties to even), with a
/// single rounding also when the result is subnormal: `m` is a normalised
/// double-double of magnitude about 1.
pub fn round_scaled(m: DD, k: i32) -> f64 {
    let hi_exp = exponent_of(m.hi);
    let e = hi_exp + k; // the result's binary exponent
    if e > 1023 {
        return f64::INFINITY.copysign(m.hi);
    }
    if e >= -1022 {
        // Normal: `hi` is already the double nearest to `m`; the scaling is
        // exact. (An `hi` on the edge of rounding into the next binade
        // stays normal.)
        return ldexp_exact(m.to_f64(), k);
    }
    // Subnormal: in units of 2^-1074 the result is the integer nearest to
    // `m × 2^(k + 1074)`, which is below 2^53, so the scaled pair is exact.
    let t = m.scale_wide(k + 1074);
    let n = round_half_even_dd(t);
    f64::from_bits(n) * if m.hi < 0.0 { -1.0 } else { 1.0 }
}

impl DD {
    /// `scale` for any `n` whose result keeps both parts finite.
    fn scale_wide(self, n: i32) -> DD {
        DD { hi: ldexp_exact(self.hi, n), lo: ldexp_exact(self.lo, n) }
    }
}

/// The integer nearest to `|t|` (ties to even), `|t| < 2^53`.
fn round_half_even_dd(t: DD) -> u64 {
    let (hi, lo) = if t.hi < 0.0 { (-t.hi, -t.lo) } else { (t.hi, t.lo) };
    let n = hi as u64; // truncates
    // `hi − n` is exact; `lo` (at most half an ulp of `hi`) only decides
    // when that is exactly a half or zero.
    let f = hi - n as f64;
    let up = f > 0.5 || (f == 0.5 && (lo > 0.0 || (lo == 0.0 && n & 1 == 1)));
    // (`|lo| < ½` here, so a zero `f` with a negative `lo` stays at `n`.)
    if up { n + 1 } else { n }
}

/// The unbiased binary exponent of a finite nonzero `x` (`x = 1.f × 2^e`),
/// also for subnormals.
pub fn exponent_of(x: f64) -> i32 {
    let b = x.to_bits();
    let e = ((b >> 52) & 0x7FF) as i32;
    if e == 0 {
        let frac = b & ((1 << 52) - 1);
        if frac == 0 {
            return -1075;
        }
        // 2^-1074 × frac: the top bit of frac decides.
        return -1074 + (63 - frac.leading_zeros() as i32);
    }
    e - 1023
}

/// Horner's rule over `coeffs` (lowest power first), the first `dd.len()`
/// coefficients in double-double and the remaining (`tail`) in `f64` at
/// `z.hi`: the terms of a series whose tail is far below the target
/// accuracy only need `f64`.
pub fn poly_mixed(z: DD, dd: &[DD], tail: &[f64]) -> DD {
    let mut t = 0.0;
    for &c in tail.iter().rev() {
        t = t * z.hi + c;
    }
    let mut acc = DD::from_f64(t);
    for &c in dd.iter().rev() {
        acc = acc.mul(z).add(c);
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_free_transformations() {
        let s = two_sum(1.0, 1e-20);
        assert_eq!((s.hi, s.lo), (1.0, 1e-20));
        let p = two_prod(1.0 + f64::EPSILON, 1.0 - f64::EPSILON);
        assert_eq!(p.hi, 1.0);
        assert_eq!(p.lo, -f64::EPSILON * f64::EPSILON);
        let third = DD::ONE.div(DD::from_f64(3.0));
        // 3 × (1/3) is 1 within 2^-104.
        let back = third.mul_f64(3.0).sub(DD::ONE);
        assert!(back.to_f64().abs() < 1e-31, "{back:?}");
        let r2 = DD::from_f64(2.0).sqrt();
        let err = r2.square().sub(DD::from_f64(2.0));
        assert!(err.to_f64().abs() < 1e-31, "{err:?}");
    }

    #[test]
    fn scaled_rounding() {
        assert_eq!(round_scaled(DD::ONE, 10), 1024.0);
        assert_eq!(round_scaled(DD::ONE, -1074), 5e-324);
        // 1.5 × 2^-1074 is a tie between 1 and 2 units: even.
        assert_eq!(round_scaled(DD::new(1.5, 0.0), -1074), 1e-323);
        // Just above the tie rounds up, just below rounds down.
        assert_eq!(round_scaled(DD::new(1.25, 1e-30), -1073), f64::from_bits(3));
        assert_eq!(round_scaled(DD::new(1.25, -1e-30), -1073), f64::from_bits(2));
        assert_eq!(round_scaled(DD::new(0.5, 1e-30), -1074), 5e-324);
        assert_eq!(round_scaled(DD::new(0.5, -1e-30), -1074), 0.0);
        assert_eq!(round_scaled(DD::ONE, 1024), f64::INFINITY);
        assert_eq!(round_scaled(DD::new(0.75, 0.0), 1024), 0.75 * 2f64.powi(1023) * 2.0);
        assert_eq!(exponent_of(1.0), 0);
        assert_eq!(exponent_of(5e-324), -1074);
        assert_eq!(exponent_of(f64::MIN_POSITIVE), -1022);
    }
}
