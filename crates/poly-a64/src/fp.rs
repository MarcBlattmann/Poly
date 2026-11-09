//! Scalar floating-point arithmetic on raw bit patterns, as the Arm ARM
//! defines it.
//!
//! What is implemented:
//!
//! - Formats: IEEE 754 half (conversions only), single and double.
//! - Rounding: round to nearest, ties to even for arithmetic and for the
//!   conversions that do not name a mode; the explicit modes of
//!   `FCVT[NPMZA]` and `FRINT[NPMZA]` are honoured. `FPCR.RMode` is **not**
//!   applied to arithmetic, `FRINTI`/`FRINTX` or `SCVTF`.
//! - NaNs: operand NaNs propagate with the Arm ARM's priority (signalling
//!   before quiet, first operand before second, quieted); invalid
//!   operations give the default NaN (positive, quiet, empty payload).
//!   `FPCR.DN` (default NaN mode) is honoured.
//! - Exception flags: `FPSR.IOC` (invalid operation) and `FPSR.DZC`
//!   (division by zero) are set. Overflow, underflow, inexact and
//!   input-denormal are **not** tracked, and no exception traps are taken.
//! - `FPCR.FZ`, `FZ16` and `AHP` (flush to zero, alternative half
//!   precision) are not implemented: denormals are computed exactly.
//!
//! Addition, subtraction, multiplication and division use the host's IEEE
//! `f32`/`f64` operators (which are round-to-nearest-even) after the
//! explicit NaN and invalid-operation checks. Fused multiply-add and square
//! root are done with integer arithmetic ([`round_pack`]) so that they are
//! correctly rounded on every host.

use core::cmp::Ordering;

use crate::inst::FpRound;

/// `FPSR.IOC`.
pub const IOC: u32 = 1;
/// `FPSR.DZC`.
pub const DZC: u32 = 2;

/// A floating-point format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fmt {
    /// Fraction bits.
    pub frac: u32,
    /// Exponent bits.
    pub exp: u32,
}

/// IEEE half precision.
pub const HALF: Fmt = Fmt { frac: 10, exp: 5 };
/// IEEE single precision.
pub const SINGLE: Fmt = Fmt { frac: 23, exp: 8 };
/// IEEE double precision.
pub const DOUBLE: Fmt = Fmt { frac: 52, exp: 11 };

/// Exception flags raised by operations, plus the `FPCR` bits they depend on.
#[derive(Clone, Copy, Debug, Default)]
pub struct FpEnv {
    /// `FPSR` flag bits raised so far.
    pub flags: u32,
    /// `FPCR.DN`: every NaN result is the default NaN.
    pub dn: bool,
}

impl Fmt {
    const fn bias(self) -> i32 {
        (1 << (self.exp - 1)) - 1
    }
    const fn sign_bit(self) -> u64 {
        1 << (self.exp + self.frac)
    }
    const fn exp_max(self) -> u64 {
        (1 << self.exp) - 1
    }
    const fn frac_mask(self) -> u64 {
        (1 << self.frac) - 1
    }
    const fn exp_field(self, b: u64) -> u64 {
        (b >> self.frac) & self.exp_max()
    }
    const fn quiet_bit(self) -> u64 {
        1 << (self.frac - 1)
    }
    /// Is `b` a NaN?
    pub const fn is_nan(self, b: u64) -> bool {
        self.exp_field(b) == self.exp_max() && (b & self.frac_mask()) != 0
    }
    /// Is `b` a signalling NaN?
    pub const fn is_snan(self, b: u64) -> bool {
        self.is_nan(b) && (b & self.quiet_bit()) == 0
    }
    const fn is_qnan(self, b: u64) -> bool {
        self.is_nan(b) && (b & self.quiet_bit()) != 0
    }
    const fn is_inf(self, b: u64) -> bool {
        self.exp_field(b) == self.exp_max() && (b & self.frac_mask()) == 0
    }
    const fn is_zero(self, b: u64) -> bool {
        (b & !self.sign_bit()) == 0
    }
    const fn sign(self, b: u64) -> bool {
        b & self.sign_bit() != 0
    }
    /// The default NaN: positive, quiet, no payload.
    pub const fn default_nan(self) -> u64 {
        (self.exp_max() << self.frac) | self.quiet_bit()
    }
    const fn inf(self, neg: bool) -> u64 {
        (self.exp_max() << self.frac) | if neg { self.sign_bit() } else { 0 }
    }
    const fn zero(self, neg: bool) -> u64 {
        if neg { self.sign_bit() } else { 0 }
    }
    /// Flip the sign bit (`FPNeg`).
    pub const fn neg(self, b: u64) -> u64 {
        b ^ self.sign_bit()
    }
    /// Clear the sign bit (`FPAbs`).
    pub const fn abs(self, b: u64) -> u64 {
        b & !self.sign_bit()
    }

    /// Split a finite non-zero value into sign, exponent and integer
    /// significand: value = `m * 2^e`.
    fn unpack(self, b: u64) -> (bool, i32, u64) {
        let f = b & self.frac_mask();
        let e = self.exp_field(b) as i32;
        if e == 0 {
            (self.sign(b), 1 - self.bias() - self.frac as i32, f)
        } else {
            (self.sign(b), e - self.bias() - self.frac as i32, f | (1 << self.frac))
        }
    }
}

fn nan_out(f: Fmt, b: u64, env: &FpEnv) -> u64 {
    if env.dn { f.default_nan() } else { b | f.quiet_bit() }
}

/// `FPProcessNaNs`: if either operand is a NaN, the result.
fn process2(f: Fmt, a: u64, b: u64, env: &mut FpEnv) -> Option<u64> {
    if !f.is_nan(a) && !f.is_nan(b) {
        return None;
    }
    if f.is_snan(a) || f.is_snan(b) {
        env.flags |= IOC;
    }
    let pick = if f.is_snan(a) {
        a
    } else if f.is_snan(b) {
        b
    } else if f.is_nan(a) {
        a
    } else {
        b
    };
    Some(nan_out(f, pick, env))
}

fn invalid(f: Fmt, env: &mut FpEnv) -> u64 {
    env.flags |= IOC;
    f.default_nan()
}

// The host operators, by format.
fn nat(f: Fmt, a: u64, b: u64, op32: fn(f32, f32) -> f32, op64: fn(f64, f64) -> f64) -> u64 {
    if f == SINGLE {
        u64::from(op32(f32::from_bits(a as u32), f32::from_bits(b as u32)).to_bits())
    } else {
        op64(f64::from_bits(a), f64::from_bits(b)).to_bits()
    }
}

fn host_cmp(f: Fmt, a: u64, b: u64) -> Option<Ordering> {
    if f == SINGLE {
        f32::from_bits(a as u32).partial_cmp(&f32::from_bits(b as u32))
    } else {
        f64::from_bits(a).partial_cmp(&f64::from_bits(b))
    }
}

/// `a + b`.
pub fn add(f: Fmt, a: u64, b: u64, env: &mut FpEnv) -> u64 {
    if let Some(r) = process2(f, a, b, env) {
        return r;
    }
    if f.is_inf(a) && f.is_inf(b) && f.sign(a) != f.sign(b) {
        return invalid(f, env);
    }
    nat(f, a, b, |x, y| x + y, |x, y| x + y)
}

/// `a - b`.
pub fn sub(f: Fmt, a: u64, b: u64, env: &mut FpEnv) -> u64 {
    if let Some(r) = process2(f, a, b, env) {
        return r;
    }
    if f.is_inf(a) && f.is_inf(b) && f.sign(a) == f.sign(b) {
        return invalid(f, env);
    }
    nat(f, a, b, |x, y| x - y, |x, y| x - y)
}

/// `a * b`.
pub fn mul(f: Fmt, a: u64, b: u64, env: &mut FpEnv) -> u64 {
    if let Some(r) = process2(f, a, b, env) {
        return r;
    }
    if (f.is_inf(a) && f.is_zero(b)) || (f.is_zero(a) && f.is_inf(b)) {
        return invalid(f, env);
    }
    nat(f, a, b, |x, y| x * y, |x, y| x * y)
}

/// `a / b`.
pub fn div(f: Fmt, a: u64, b: u64, env: &mut FpEnv) -> u64 {
    if let Some(r) = process2(f, a, b, env) {
        return r;
    }
    if (f.is_inf(a) && f.is_inf(b)) || (f.is_zero(a) && f.is_zero(b)) {
        return invalid(f, env);
    }
    if f.is_zero(b) && !f.is_inf(a) {
        env.flags |= DZC;
    }
    nat(f, a, b, |x, y| x / y, |x, y| x / y)
}

/// `FMAX` (`min` false) and `FMIN` (`min` true).
pub fn max_min(f: Fmt, a: u64, b: u64, min: bool, env: &mut FpEnv) -> u64 {
    if let Some(r) = process2(f, a, b, env) {
        return r;
    }
    if f.is_zero(a) && f.is_zero(b) && f.sign(a) != f.sign(b) {
        return f.zero(min);
    }
    let ord = host_cmp(f, a, b);
    let take_a = if min { ord == Some(Ordering::Less) } else { ord == Some(Ordering::Greater) };
    if take_a { a } else { b }
}

/// `FMAXNM` and `FMINNM`: a quiet NaN operand loses against a number.
pub fn max_min_num(f: Fmt, a: u64, b: u64, min: bool, env: &mut FpEnv) -> u64 {
    if f.is_qnan(a) && !f.is_nan(b) {
        return b;
    }
    if f.is_qnan(b) && !f.is_nan(a) {
        return a;
    }
    max_min(f, a, b, min, env)
}

/// `FCMP`/`FCMPE`: the flags (`NZCV` positions). `signal` raises the
/// invalid-operation flag for quiet NaNs too.
pub fn compare(f: Fmt, a: u64, b: u64, signal: bool, env: &mut FpEnv) -> u32 {
    if f.is_nan(a) || f.is_nan(b) {
        if signal || f.is_snan(a) || f.is_snan(b) {
            env.flags |= IOC;
        }
        return 0x3 << 28;
    }
    match host_cmp(f, a, b) {
        Some(Ordering::Equal) => 0x6 << 28,
        Some(Ordering::Less) => 0x8 << 28,
        _ => 0x2 << 28,
    }
}

// ---------------------------------------------------------------------
// Rounding and packing
// ---------------------------------------------------------------------

fn msb(v: u128) -> i32 {
    127 - v.leading_zeros() as i32
}

/// Round the exact value `mant * 2^exp2` (plus an infinitesimal extra when
/// `sticky`) to nearest-even in format `f` and pack it with sign `neg`.
/// Overflow gives infinity; the result may be subnormal or zero.
pub fn round_pack(f: Fmt, neg: bool, exp2: i32, mant: u128, sticky: bool) -> u64 {
    let sign = if neg { f.sign_bit() } else { 0 };
    if mant == 0 {
        return sign;
    }
    let p = f.frac as i32 + 1;
    let m = msb(mant);
    let biased = m + exp2 + f.bias();
    let extra = if biased < 1 { 1 - biased } else { 0 };
    let drop = (m - (p - 1)) + extra;
    let mut e_field = if biased >= 1 { biased as i64 } else { 0 };
    let mut kept;
    if drop <= 0 {
        kept = mant << (-drop) as u32;
    } else if drop > 128 {
        kept = 0;
    } else {
        let d = drop as u32;
        kept = if d >= 128 { 0 } else { mant >> d };
        let rem = if d >= 128 { mant } else { mant & ((1u128 << d) - 1) };
        let half = 1u128 << (d - 1);
        let up = rem > half || (rem == half && (sticky || kept & 1 == 1));
        if up {
            kept += 1;
        }
    }
    if kept >= (1u128 << p) {
        kept >>= 1;
        e_field += 1;
    }
    if kept >= (1u128 << (p - 1)) && e_field == 0 {
        e_field = 1;
    }
    if e_field >= f.exp_max() as i64 {
        return sign | f.inf(false);
    }
    sign | ((e_field as u64) << f.frac) | (kept as u64 & f.frac_mask())
}

fn shr_sticky(v: u128, n: u32) -> u128 {
    if n >= 128 { u128::from(v != 0) } else { (v >> n) | u128::from(v & ((1u128 << n) - 1) != 0) }
}

/// `addend + x * y` with a single rounding (`FPMulAdd`).
pub fn fma(f: Fmt, addend: u64, x: u64, y: u64, env: &mut FpEnv) -> u64 {
    let inf_zero = (f.is_inf(x) && f.is_zero(y)) || (f.is_zero(x) && f.is_inf(y));
    if f.is_qnan(addend) && inf_zero {
        return invalid(f, env);
    }
    if f.is_nan(addend) || f.is_nan(x) || f.is_nan(y) {
        if f.is_snan(addend) || f.is_snan(x) || f.is_snan(y) {
            env.flags |= IOC;
        }
        let pick = [addend, x, y]
            .into_iter()
            .find(|&v| f.is_snan(v))
            .or_else(|| [addend, x, y].into_iter().find(|&v| f.is_nan(v)))
            .unwrap_or(addend);
        return nan_out(f, pick, env);
    }
    if inf_zero {
        return invalid(f, env);
    }
    let p_neg = f.sign(x) != f.sign(y);
    if f.is_inf(x) || f.is_inf(y) {
        if f.is_inf(addend) && f.sign(addend) != p_neg {
            return invalid(f, env);
        }
        return f.inf(p_neg);
    }
    if f.is_inf(addend) {
        return addend;
    }
    if f.is_zero(x) || f.is_zero(y) {
        if f.is_zero(addend) {
            let neg = if p_neg == f.sign(addend) { p_neg } else { false };
            return f.zero(neg);
        }
        return addend;
    }
    let (_, ex, mx) = f.unpack(x);
    let (_, ey, my) = f.unpack(y);
    let pm = u128::from(mx) * u128::from(my);
    let pe = ex + ey;
    if f.is_zero(addend) {
        return round_pack(f, p_neg, pe, pm, false);
    }
    let (a_neg, ea, ma) = f.unpack(addend);
    // Normalise both significands to have their top bit at bit 125.
    let ps = 125 - msb(pm);
    let p_sig = pm << ps as u32;
    let p_exp = pe - ps;
    let as_ = 125 - msb(u128::from(ma));
    let a_sig = u128::from(ma) << as_ as u32;
    let a_exp = ea - as_;
    let d = p_exp - a_exp;
    let (big, big_neg, small, small_neg, scale);
    if d >= 0 {
        // The product has the larger scale; the addend moves right.
        big = p_sig;
        big_neg = p_neg;
        small = shr_sticky(a_sig, d as u32);
        small_neg = a_neg;
        scale = p_exp;
    } else {
        big = a_sig;
        big_neg = a_neg;
        small = shr_sticky(p_sig, (-d) as u32);
        small_neg = p_neg;
        scale = a_exp;
    }
    let (sum, neg) = if big_neg == small_neg {
        (big + small, big_neg)
    } else if big >= small {
        (big - small, big_neg)
    } else {
        (small - big, small_neg)
    };
    if sum == 0 {
        return f.zero(false);
    }
    round_pack(f, neg, scale, sum, false)
}

/// Integer square root of a `u128` (floor).
fn isqrt(n: u128) -> u128 {
    let mut rem = n;
    let mut root = 0u128;
    let mut bit = 1u128 << 126;
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
    root
}

/// `sqrt(a)`, correctly rounded.
pub fn sqrt(f: Fmt, a: u64, env: &mut FpEnv) -> u64 {
    if f.is_nan(a) {
        if f.is_snan(a) {
            env.flags |= IOC;
        }
        return nan_out(f, a, env);
    }
    if f.is_zero(a) {
        return a;
    }
    if f.sign(a) {
        return invalid(f, env);
    }
    if f.is_inf(a) {
        return a;
    }
    let (_, e, m) = f.unpack(a);
    // Normalise so that the significand has 54 or 55 bits and the exponent is even (subnormal
    // inputs have fewer bits and would otherwise lose precision in the root).
    let mut k = 53 - (63 - m.leading_zeros() as i32);
    if (e - k) & 1 != 0 {
        k += 1;
    }
    let (m, e) = (u128::from(m) << k as u32, e - k);
    // value = m * 2^e with e even; radicand = m * 2^72 has at most 127 bits.
    let radicand = m << 72;
    let r = isqrt(radicand);
    let exact = r * r == radicand;
    round_pack(f, false, (e - 72) / 2, r, !exact)
}

// ---------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------

/// `FCVT`: convert between formats (round to nearest even).
pub fn convert(from: Fmt, to: Fmt, a: u64, env: &mut FpEnv) -> u64 {
    if from.is_nan(a) {
        if from.is_snan(a) {
            env.flags |= IOC;
        }
        if env.dn {
            return to.default_nan();
        }
        let fr = a & from.frac_mask();
        let nf = if to.frac >= from.frac { fr << (to.frac - from.frac) } else { fr >> (from.frac - to.frac) };
        let sign = if from.sign(a) { to.sign_bit() } else { 0 };
        return sign | (to.exp_max() << to.frac) | nf | to.quiet_bit();
    }
    if from.is_inf(a) {
        return to.inf(from.sign(a));
    }
    if from.is_zero(a) {
        return to.zero(from.sign(a));
    }
    let (neg, e, m) = from.unpack(a);
    round_pack(to, neg, e, u128::from(m), false)
}

/// Split `|a|` (finite, non-zero) scaled by `2^fbits` into an integer part
/// and rounding information, then round per `mode`. Returns the rounded
/// magnitude, or `None` when it exceeds what fits in 128 bits (certainly
/// out of range).
fn round_magnitude(f: Fmt, a: u64, mode: FpRound, fbits: u32) -> Option<(bool, u128)> {
    let (neg, e, m) = f.unpack(a);
    let e = e + fbits as i32;
    let (q, rem, half): (u128, u128, u128);
    if e >= 0 {
        if e > 70 {
            return None;
        }
        q = u128::from(m) << e as u32;
        rem = 0;
        half = 1;
    } else {
        let s = (-e) as u32;
        if s >= 128 {
            q = 0;
            rem = 1;
            half = 2; // below half
        } else {
            q = u128::from(m) >> s;
            rem = u128::from(m) & ((1u128 << s) - 1);
            half = 1u128 << (s - 1);
        }
    }
    let inc = match mode {
        FpRound::NearestEven => rem > half || (rem == half && q & 1 == 1),
        FpRound::Away => rem >= half && rem != 0,
        FpRound::PosInf => !neg && rem != 0,
        FpRound::NegInf => neg && rem != 0,
        FpRound::Zero => false,
    };
    Some((neg, q + u128::from(inc)))
}

/// `FCVT*S`/`FCVT*U` and the fixed-point forms: convert to an integer of
/// 32 or 64 bits (`w64`), saturating, NaN to zero; both raise
/// invalid-operation when the result is out of range. Returns the register
/// value (the low 32 bits for a 32-bit destination).
pub fn to_int(f: Fmt, a: u64, signed: bool, w64: bool, mode: FpRound, fbits: u32, env: &mut FpEnv) -> u64 {
    let bits = if w64 { 64 } else { 32 };
    let smax: u128 = (1u128 << (bits - 1)) - 1;
    let umax: u128 = (1u128 << bits) - 1;
    let sat = |neg: bool| -> u64 {
        if signed {
            if neg { (1u64 << (bits - 1)).wrapping_neg() & ((umax) as u64) } else { smax as u64 }
        } else if neg {
            0
        } else {
            umax as u64
        }
    };
    if f.is_nan(a) {
        env.flags |= IOC;
        return 0;
    }
    if f.is_inf(a) {
        env.flags |= IOC;
        return sat(f.sign(a));
    }
    if f.is_zero(a) {
        return 0;
    }
    let Some((neg, q)) = round_magnitude(f, a, mode, fbits) else {
        env.flags |= IOC;
        return sat(f.sign(a));
    };
    if signed {
        if neg {
            if q > smax + 1 {
                env.flags |= IOC;
                return sat(true);
            }
            ((q as u64).wrapping_neg()) & (umax as u64)
        } else {
            if q > smax {
                env.flags |= IOC;
                return sat(false);
            }
            q as u64
        }
    } else if neg {
        if q != 0 {
            env.flags |= IOC;
        }
        0
    } else {
        if q > umax {
            env.flags |= IOC;
            return sat(false);
        }
        q as u64
    }
}

/// `SCVTF`/`UCVTF`: `value / 2^fbits` rounded to nearest even. `value` is
/// the register contents at the source width (already masked).
pub fn from_int(f: Fmt, value: u64, signed: bool, w64: bool, fbits: u32) -> u64 {
    let (neg, mag) = if signed {
        let v = if w64 { value as i64 } else { i64::from(value as u32 as i32) };
        (v < 0, u128::from(v.unsigned_abs()))
    } else {
        (false, u128::from(value))
    };
    round_pack(f, neg, -(fbits as i32), mag, false)
}

/// `FRINT*`: round to an integral value in the same format.
pub fn round_to_int(f: Fmt, a: u64, mode: FpRound, env: &mut FpEnv) -> u64 {
    if f.is_nan(a) {
        if f.is_snan(a) {
            env.flags |= IOC;
        }
        return nan_out(f, a, env);
    }
    if f.is_inf(a) || f.is_zero(a) {
        return a;
    }
    let (_, e, _) = f.unpack(a);
    if e >= 0 {
        return a; // already integral
    }
    match round_magnitude(f, a, mode, 0) {
        Some((neg, 0)) => f.zero(neg),
        Some((neg, q)) => round_pack(f, neg, 0, q, false),
        None => a,
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    fn d(x: f64) -> u64 {
        x.to_bits()
    }

    #[test]
    fn fma_matches_host_fused() {
        // A deterministic spread of operands including cancellation cases.
        let mut s: u64 = 0x1234_5678_9abc_def1;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let mut env = FpEnv::default();
        for i in 0..200_000 {
            let mut v = [0u64; 3];
            for x in v.iter_mut() {
                let r = next();
                // Mix of exponents; sometimes tiny or huge, sometimes nearby values.
                let e = ((r >> 52) % 80) + 1000;
                *x = (r & 0x800F_FFFF_FFFF_FFFF) | (e << 52);
            }
            if i % 3 == 0 {
                // Make the addend nearly cancel the product.
                let p = f64::from_bits(v[1]) * f64::from_bits(v[2]);
                v[0] = (-p).to_bits() ^ (next() & 0xFF);
            }
            let got = fma(DOUBLE, v[0], v[1], v[2], &mut env);
            let want = f64::from_bits(v[1]).mul_add(f64::from_bits(v[2]), f64::from_bits(v[0]));
            if want.is_nan() {
                continue;
            }
            assert_eq!(got, want.to_bits(), "fma {:x} + {:x} * {:x}", v[0], v[1], v[2]);
        }
    }

    #[test]
    fn fma_subnormal_and_overflow() {
        let mut env = FpEnv::default();
        let tiny = f64::from_bits(1);
        assert_eq!(fma(DOUBLE, d(0.0), d(tiny), d(0.5), &mut env), d(tiny.mul_add(0.5, 0.0)));
        assert_eq!(fma(DOUBLE, d(0.0), d(tiny), d(1.5), &mut env), d(tiny.mul_add(1.5, 0.0)));
        assert_eq!(fma(DOUBLE, d(1.0), d(f64::MAX), d(2.0), &mut env), d(f64::INFINITY));
        assert_eq!(fma(DOUBLE, d(-0.0), d(-0.0), d(0.0), &mut env), d(-0.0));
        assert_eq!(fma(DOUBLE, d(0.0), d(-0.0), d(0.0), &mut env), d(0.0));
        assert_eq!(fma(DOUBLE, d(-0.0), d(-1.0), d(0.0), &mut env), d(-0.0));
    }

    #[test]
    fn fma_single_matches_host() {
        let mut s: u64 = 0xdead_beef_1234_5678;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let mut env = FpEnv::default();
        for _ in 0..200_000 {
            let mut v = [0u32; 3];
            for x in v.iter_mut() {
                let r = next();
                let e = ((r >> 23) % 40) as u32 + 110;
                *x = ((r as u32) & 0x807F_FFFF) | (e << 23);
            }
            let got = fma(SINGLE, u64::from(v[0]), u64::from(v[1]), u64::from(v[2]), &mut env);
            let want = f32::from_bits(v[1]).mul_add(f32::from_bits(v[2]), f32::from_bits(v[0]));
            assert_eq!(got as u32, want.to_bits());
        }
    }

    #[test]
    fn sqrt_is_correctly_rounded() {
        let mut s: u64 = 99;
        let mut env = FpEnv::default();
        for _ in 0..100_000 {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let x = f64::from_bits((s >> 1) & 0x7FEF_FFFF_FFFF_FFFF);
            // Compare against the host's exact square root through the identity
            // r*r vs x checked with fma: the neighbours must be no closer.
            let r = f64::from_bits(sqrt(DOUBLE, x.to_bits(), &mut env));
            let below = f64::from_bits(r.to_bits() - 1);
            let above = f64::from_bits(r.to_bits() + 1);
            let err = |c: f64| (c.mul_add(c, -x)).abs();
            if x > 1e-300 || x == 0.0 {
                assert!(err(r) <= err(below) && err(r) <= err(above), "sqrt({x})");
            }
        }
        // Subnormal inputs: compare with the host's sqrt, which is correctly rounded.
        for i in 1..5000u64 {
            let x = f64::from_bits(i.wrapping_mul(0x0123_4567_89AB) & 0x000F_FFFF_FFFF_FFFF);
            assert_eq!(sqrt(DOUBLE, x.to_bits(), &mut env), x.sqrt().to_bits(), "sqrt of subnormal {x:e}");
        }
        assert_eq!(sqrt(DOUBLE, d(4.0), &mut env), d(2.0));
        assert_eq!(sqrt(SINGLE, u64::from(2f32.to_bits()), &mut env) as u32, core::f32::consts::SQRT_2.to_bits());
    }

    #[test]
    fn conversions() {
        let mut env = FpEnv::default();
        assert_eq!(to_int(DOUBLE, d(-1.5), true, false, FpRound::Zero, 0, &mut env), 0xFFFF_FFFF);
        assert_eq!(to_int(DOUBLE, d(1e20), true, true, FpRound::Zero, 0, &mut env), i64::MAX as u64);
        assert_eq!(env.flags & IOC, IOC);
        assert_eq!(from_int(DOUBLE, u64::MAX, false, true, 0), d(18446744073709551615.0));
        assert_eq!(from_int(SINGLE, 16777217, true, false, 0), u64::from(16777216f32.to_bits()));
    }
}
