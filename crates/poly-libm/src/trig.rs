//! `sin`, `cos`, `tan`.
//!
//! **Reduction** (Payne and Hanek, "Radian reduction for trigonometric
//! functions", SIGNUM Newsletter 18, 1983): `x = (4j + q) π/2 + r` with
//! `|r| ≤ π/4`. Writing `x = m × 2^e` (`m` the 53-bit integer
//! significand), `x × 2/π mod 4` only needs the bits of 2/π from position
//! `e − 2` on (earlier bits contribute multiples of 4); 192 of them times
//! `m` give `q` and 190 to 245 bits of the fraction, exact but for the
//! truncated rest of 2/π, below 2^-137. The nearest double to a multiple of
//! π/2 is about 2^-61 away from it (over the whole double range), so the
//! fraction keeps a relative error below 2^-75 even then. Its top 128 bits
//! become a double-double, times π/2 in double-double. One method for every
//! `|x| > π/4`, so there is no threshold where accuracy changes.
//!
//! **Kernels** on `|r| ≤ π/4` (`z = r² ≤ 0.617`): `sin r = r Σ (−1)^n
//! z^n/(2n+1)!`, `n ≤ 12` (first omitted term below 2^-90), `cos r = Σ
//! (−1)^n z^n/(2n)!`, `n ≤ 13`; terms up to `n = 6` in double-double, the
//! rest (below 2^-38 of the result) in `f64`. `tan = sin/cos` (or `−cos/sin`
//! for odd `q`) as a double-double quotient. Relative error before the one
//! final rounding is below 2^-74, so every result is within 0.5 + 2^-21 ulp
//! of the exact value (target one ulp), for every finite argument.

use crate::consts::{INV_FACT, PI_2, PI_4, TWO_OVER_PI};
use crate::dd::{DD, fast_two_sum};

const fn alt(n: usize, d: DD) -> DD {
    if n % 2 == 1 { d.neg() } else { d }
}

const SIN_DD: [DD; 7] = [
    alt(0, INV_FACT[1]),
    alt(1, INV_FACT[3]),
    alt(2, INV_FACT[5]),
    alt(3, INV_FACT[7]),
    alt(4, INV_FACT[9]),
    alt(5, INV_FACT[11]),
    alt(6, INV_FACT[13]),
];
const SIN_TAIL: [f64; 6] = [
    alt(7, INV_FACT[15]).hi,
    alt(8, INV_FACT[17]).hi,
    alt(9, INV_FACT[19]).hi,
    alt(10, INV_FACT[21]).hi,
    alt(11, INV_FACT[23]).hi,
    alt(12, INV_FACT[25]).hi,
];
const COS_DD: [DD; 7] = [
    alt(0, INV_FACT[0]),
    alt(1, INV_FACT[2]),
    alt(2, INV_FACT[4]),
    alt(3, INV_FACT[6]),
    alt(4, INV_FACT[8]),
    alt(5, INV_FACT[10]),
    alt(6, INV_FACT[12]),
];
const COS_TAIL: [f64; 7] = [
    alt(7, INV_FACT[14]).hi,
    alt(8, INV_FACT[16]).hi,
    alt(9, INV_FACT[18]).hi,
    alt(10, INV_FACT[20]).hi,
    alt(11, INV_FACT[22]).hi,
    alt(12, INV_FACT[24]).hi,
    alt(13, INV_FACT[26]).hi,
];

fn sin_kernel(r: DD) -> DD {
    crate::dd::poly_mixed(r.square(), &SIN_DD, &SIN_TAIL).mul(r)
}

fn cos_kernel(r: DD) -> DD {
    crate::dd::poly_mixed(r.square(), &COS_DD, &COS_TAIL)
}

/// A 256-bit unsigned integer, least significant word first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct U256([u64; 4]);

impl U256 {
    fn bit(&self, n: u32) -> bool {
        (self.0[(n / 64) as usize] >> (n % 64)) & 1 == 1
    }

    fn shr(&self, n: u32) -> U256 {
        let (words, bits) = ((n / 64) as usize, n % 64);
        let mut out = [0u64; 4];
        for (i, o) in out.iter_mut().enumerate() {
            let lo = self.0.get(i + words).copied().unwrap_or(0);
            let hi = self.0.get(i + words + 1).copied().unwrap_or(0);
            *o = if bits == 0 { lo } else { (lo >> bits) | (hi << (64 - bits)) };
        }
        U256(out)
    }

    fn shl(&self, n: u32) -> U256 {
        let (words, bits) = ((n / 64) as usize, n % 64);
        let mut out = [0u64; 4];
        for (i, o) in out.iter_mut().enumerate() {
            if i < words {
                continue;
            }
            let lo = self.0[i - words];
            let below = if i > words { self.0[i - words - 1] } else { 0 };
            *o = if bits == 0 { lo } else { (lo << bits) | (below >> (64 - bits)) };
        }
        U256(out)
    }

    /// The low `n` bits (`n < 256`).
    fn low(&self, n: u32) -> U256 {
        let mut out = self.0;
        for (i, w) in out.iter_mut().enumerate() {
            let start = i as u32 * 64;
            if start >= n {
                *w = 0;
            } else if n - start < 64 {
                *w &= (1u64 << (n - start)) - 1;
            }
        }
        U256(out)
    }

    fn sub(&self, b: &U256) -> U256 {
        let mut out = [0u64; 4];
        let mut borrow = 0u64;
        for (i, o) in out.iter_mut().enumerate() {
            let (d1, b1) = self.0[i].overflowing_sub(b.0[i]);
            let (d2, b2) = d1.overflowing_sub(borrow);
            *o = d2;
            borrow = u64::from(b1 || b2);
        }
        U256(out)
    }

    fn leading_zeros(&self) -> u32 {
        for i in (0..4).rev() {
            if self.0[i] != 0 {
                return (3 - i as u32) * 64 + self.0[i].leading_zeros();
            }
        }
        256
    }

    fn low_u128(&self) -> u128 {
        u128::from(self.0[0]) | (u128::from(self.0[1]) << 64)
    }
}

/// 192 bits of the fraction of 2/π starting after its first `skip` bits,
/// most significant word first.
fn window(skip: usize) -> [u64; 3] {
    let (wi, sh) = (skip / 64, (skip % 64) as u32);
    let get = |i: usize| TWO_OVER_PI.get(i).copied().unwrap_or(0);
    let word = |j: usize| if sh == 0 { get(wi + j) } else { (get(wi + j) << sh) | (get(wi + j + 1) >> (64 - sh)) };
    [word(0), word(1), word(2)]
}

/// `m × W` for a 53-bit `m` and a 192-bit `W` (most significant word
/// first).
fn mul_window(m: u64, w: [u64; 3]) -> U256 {
    let m = u128::from(m);
    let p2 = m * u128::from(w[2]);
    let p1 = m * u128::from(w[1]);
    let p0 = m * u128::from(w[0]);
    let l0 = p2 as u64;
    let c = (p2 >> 64) + u128::from(p1 as u64);
    let l1 = c as u64;
    let c = (c >> 64) + (p1 >> 64) + u128::from(p0 as u64);
    let l2 = c as u64;
    let c = (c >> 64) + (p0 >> 64);
    U256([l0, l1, l2, c as u64])
}

/// `a = (4j + q) π/2 + r` for a finite `a > π/4`: `(r, q)`, `|r| ≤ π/4`.
pub(crate) fn rem_pio2(a: f64) -> (DD, u32) {
    let b = a.to_bits();
    let m = (b & ((1 << 52) - 1)) | (1 << 52);
    let e = ((b >> 52) & 0x7FF) as i32 - 1075; // a = m × 2^e
    let skip = (e - 2).max(0);
    let p = mul_window(m, window(skip as usize));
    // a × 2/π (mod 4) = p / 2^point.
    let point = (192 + skip - e) as u32;
    let mut q = (p.shr(point).0[0] & 3) as u32;
    let mut frac = p.low(point);
    let neg = p.bit(point - 1);
    if neg {
        // The fraction is at least ½: take it from the next integer.
        let one = U256([1, 0, 0, 0]).shl(point);
        frac = one.sub(&frac);
        q += 1;
    }
    let lz = frac.leading_zeros();
    if lz == 256 {
        return (DD::ZERO, q & 3);
    }
    let msb = 255 - lz;
    let top: u128 = if msb >= 127 { frac.shr(msb - 127).low_u128() } else { frac.shl(127 - msb).low_u128() };
    // frac ≈ top × 2^(msb − 127); the fraction is frac / 2^point.
    let hi = (top >> 75) as u64 as f64 * crate::dd::pow2(75);
    let lo = (top & ((1u128 << 75) - 1)) as f64;
    let f = fast_two_sum(hi, lo).scale(msb as i32 - 127 - point as i32);
    let r = f.mul(PI_2);
    (if neg { r.neg() } else { r }, q & 3)
}

/// `(r, q)` for any finite `a ≥ 0`.
fn reduce(a: f64) -> (DD, u32) {
    if a <= PI_4.hi { (DD::from_f64(a), 0) } else { rem_pio2(a) }
}

/// `sin`.
pub fn sin(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let a = x.abs();
    if a < 1e-8 {
        return x; // x³/6 is below half an ulp of x
    }
    let (r, q) = reduce(a);
    let v = match q {
        0 => sin_kernel(r),
        1 => cos_kernel(r),
        2 => sin_kernel(r).neg(),
        _ => cos_kernel(r).neg(),
    };
    let v = v.to_f64();
    if x < 0.0 { -v } else { v }
}

/// `cos`.
pub fn cos(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let a = x.abs();
    if a < 1e-8 {
        return 1.0; // x²/2 is below half an ulp of 1
    }
    let (r, q) = reduce(a);
    let v = match q {
        0 => cos_kernel(r),
        1 => sin_kernel(r).neg(),
        2 => cos_kernel(r).neg(),
        _ => sin_kernel(r),
    };
    v.to_f64()
}

/// `tan`.
pub fn tan(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let a = x.abs();
    if a < 1e-8 {
        return x; // x³/3 is below half an ulp of x
    }
    let (r, q) = reduce(a);
    let (s, c) = (sin_kernel(r), cos_kernel(r));
    let v = if q & 1 == 0 { s.div(c) } else { c.div(s).neg() };
    let v = v.to_f64();
    if x < 0.0 { -v } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(sin(0.0).to_bits(), 0);
        assert_eq!(sin(-0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(cos(0.0), 1.0);
        assert!(sin(f64::INFINITY).is_nan() && cos(f64::NEG_INFINITY).is_nan() && tan(f64::NAN).is_nan());
        let pi = core::f64::consts::PI;
        // sin(fl(π)) = π − fl(π) to first order: 1.2246467991473532e-16.
        assert_eq!(sin(pi), 1.2246467991473532e-16);
        assert_eq!(cos(pi), -1.0);
        assert_eq!(sin(pi / 2.0), 1.0);
        assert_eq!(cos(pi / 2.0), 6.123233995736766e-17);
        assert_eq!(tan(pi / 4.0), 0.9999999999999999);
        assert_eq!(sin(1.0), 0.8414709848078965);
        assert_eq!(cos(1.0), 0.5403023058681398);
        // The classic hard case: 2^1023 and the double nearest a multiple
        // of π/2 (Kahan, Muller: 6381956970095103 × 2^797).
        assert_eq!(sin(8.98846567431158e307), 0.563127779850884);
        let hard = 6381956970095103.0 * 2f64.powi(797);
        assert_eq!(cos(hard), -4.687165924254628e-19);
        assert_eq!(sin(hard), 1.0);
        assert_eq!(sin(1e22), -0.8522008497671888);
    }

    #[test]
    fn pythagoras_and_quotients() {
        let mut s: u64 = 0x0123_4567_89AB_CDEF;
        for i in 0..20000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let x = if i % 2 == 0 {
                (s >> 11) as f64 / (1u64 << 53) as f64 * 200.0 - 100.0
            } else {
                f64::from_bits(s & 0x7FEF_FFFF_FFFF_FFFF)
            };
            let (sn, c, t) = (sin(x), cos(x), tan(x));
            assert!((sn * sn + c * c - 1.0).abs() <= 4.0 * f64::EPSILON, "{x:e}");
            if c.abs() > 1e-3 {
                assert!((t - sn / c).abs() <= 4.0 * f64::EPSILON * t.abs(), "{x:e}");
            }
            assert_eq!(sin(-x), -sn);
            assert_eq!(cos(-x), c);
        }
    }

    #[test]
    fn wide_integers() {
        let mut a = U256([u64::MAX, 1, 0, 0]);
        assert_eq!(a.shl(64), U256([0, u64::MAX, 1, 0]));
        assert_eq!(a.shr(1), U256([u64::MAX, 0, 0, 0]));
        assert_eq!(a.low(64), U256([u64::MAX, 0, 0, 0]));
        assert_eq!(a.leading_zeros(), 191);
        a = U256([0, 0, 0, 1]).sub(&U256([1, 0, 0, 0]));
        assert_eq!(a, U256([u64::MAX, u64::MAX, u64::MAX, 0]));
    }
}
