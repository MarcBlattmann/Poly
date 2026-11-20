//! Execution of the Advanced SIMD forms that `decode_simd2` decodes:
//! vector floating point, long/wide/narrow, by-element, saturating and
//! rounding integer arithmetic, more shifts, pairwise and across-lanes
//! floating point, estimates and dot products.
//!
//! Written from the Arm ARM's pseudocode (`SatQ`, `FPRecipEstimate`,
//! `FPRSqrtEstimate`, `FPRecipStepFused`, `FPRSqrtStepFused`, `FPMulX`,
//! `Reduce`, `PolynomialMult`) and checked against a real Apple silicon CPU
//! (`a64diff native`). Saturation sets `FPSR.QC`.

use crate::cpu::{Cpu, Trap};
use crate::fp::{self, Fmt, FpEnv};
use crate::inst::*;

/// `FPSR.QC`: a saturating instruction saturated.
pub const FPSR_QC: u32 = 1 << 27;

const fn mask(bits: u32) -> u64 {
    if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 }
}

fn get(v: u128, bits: u32, i: u32) -> u64 {
    ((v >> (i * bits)) as u64) & mask(bits)
}

fn set(v: &mut u128, bits: u32, i: u32, x: u64) {
    let m = u128::from(mask(bits)) << (i * bits);
    *v = (*v & !m) | (u128::from(x & mask(bits)) << (i * bits));
}

fn sx(x: u64, bits: u32) -> i64 {
    let s = 64 - bits;
    ((x << s) as i64) >> s
}

/// The lane as a signed or unsigned integer.
fn int(x: u64, bits: u32, signed: bool) -> i128 {
    if signed { i128::from(sx(x, bits)) } else { i128::from(x) }
}

/// `SatQ`: the value saturated to `bits`, and whether it saturated.
fn sat(v: i128, bits: u32, signed: bool) -> (u64, bool) {
    let (lo, hi) = if signed { (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1) } else { (0, (1i128 << bits) - 1) };
    if v < lo {
        (lo as u64 & mask(bits), true)
    } else if v > hi {
        (hi as u64 & mask(bits), true)
    } else {
        (v as u64 & mask(bits), false)
    }
}

/// `v << s` for a signed shift amount (negative shifts right, arithmetic on
/// the unbounded value), with rounding for right shifts when `round`.
fn shift_signed(v: i128, s: i32, round: bool) -> i128 {
    if s >= 0 {
        // Anything shifted by 64 or more is out of every lane's range.
        if s >= 64 { if v == 0 { 0 } else { v.signum() << 100 } } else { v << s }
    } else {
        let n = -s;
        if n > 70 {
            return if v < 0 && !round { -1 } else { 0 };
        }
        let r = if round { 1i128 << (n - 1) } else { 0 };
        (v + r) >> n
    }
}

fn fmt(ft: FpType) -> Fmt {
    match ft {
        FpType::H => fp::HALF,
        FpType::S => fp::SINGLE,
        FpType::D => fp::DOUBLE,
    }
}

fn fbits(ft: FpType) -> u32 {
    match ft {
        FpType::H => 16,
        FpType::S => 32,
        FpType::D => 64,
    }
}

/// Polynomial (carry-less) product of the low `bits` of `a` and `b`.
fn pmul(a: u64, b: u64, bits: u32) -> u128 {
    let mut r = 0u128;
    for i in 0..bits {
        if (b >> i) & 1 != 0 {
            r ^= u128::from(a & mask(bits)) << i;
        }
    }
    r
}

// ---- floating-point helpers ----

fn fp_two(f: Fmt) -> u64 {
    if f == fp::DOUBLE { 0x4000_0000_0000_0000 } else { 0x4000_0000 }
}
fn fp_one_half(f: Fmt) -> u64 {
    if f == fp::DOUBLE { 0x3FF8_0000_0000_0000 } else { 0x3FC0_0000 }
}

fn exp_field(f: Fmt, a: u64) -> u64 {
    (a >> f.frac) & ((1 << f.exp) - 1)
}
fn is_inf(f: Fmt, a: u64) -> bool {
    exp_field(f, a) == (1 << f.exp) - 1 && a & ((1 << f.frac) - 1) == 0
}
fn is_zero(f: Fmt, a: u64) -> bool {
    f.abs(a) == 0
}
fn sign_of(f: Fmt, a: u64) -> bool {
    a >> (f.exp + f.frac) & 1 != 0
}
fn inf(f: Fmt, neg: bool) -> u64 {
    (((1u64 << f.exp) - 1) << f.frac) | (u64::from(neg) << (f.exp + f.frac))
}

/// Halve a value whose exponent allows it exactly (field > 1).
fn halve_exact(f: Fmt, a: u64) -> Option<u64> {
    let e = exp_field(f, a);
    (e > 1 && e != (1 << f.exp) - 1).then(|| a - (1 << f.frac))
}

fn mulx(f: Fmt, a: u64, b: u64, env: &mut FpEnv) -> u64 {
    if f.is_nan(a) || f.is_nan(b) {
        return fp::mul(f, a, b, env);
    }
    if (is_inf(f, a) && is_zero(f, b)) || (is_zero(f, a) && is_inf(f, b)) {
        let s = sign_of(f, a) ^ sign_of(f, b);
        return fp_two(f) | (u64::from(s) << (f.exp + f.frac));
    }
    fp::mul(f, a, b, env)
}

fn recps(f: Fmt, a: u64, b: u64, env: &mut FpEnv) -> u64 {
    let a = f.neg(a);
    if f.is_nan(a) || f.is_nan(b) {
        return fp::add(f, a, b, env);
    }
    if (is_inf(f, a) && is_zero(f, b)) || (is_zero(f, a) && is_inf(f, b)) {
        return fp_two(f);
    }
    if is_inf(f, a) || is_inf(f, b) {
        return inf(f, sign_of(f, a) ^ sign_of(f, b));
    }
    fp::fma(f, fp_two(f), a, b, env)
}

fn rsqrts(f: Fmt, a: u64, b: u64, env: &mut FpEnv) -> u64 {
    let a = f.neg(a);
    if f.is_nan(a) || f.is_nan(b) {
        return fp::add(f, a, b, env);
    }
    if (is_inf(f, a) && is_zero(f, b)) || (is_zero(f, a) && is_inf(f, b)) {
        return fp_one_half(f);
    }
    if is_inf(f, a) || is_inf(f, b) {
        return inf(f, sign_of(f, a) ^ sign_of(f, b));
    }
    // (3 + a*b) / 2 with one rounding: 1.5 + (a/2)*b.
    if let Some(h) = halve_exact(f, a) {
        fp::fma(f, fp_one_half(f), h, b, env)
    } else if let Some(h) = halve_exact(f, b) {
        fp::fma(f, fp_one_half(f), a, h, env)
    } else {
        fp::fma(f, fp_one_half(f), a, b, env)
    }
}

/// `RecipEstimate`: 9-bit `a` (256..511) to an 8-bit-precision estimate.
fn recip_estimate(a: u64) -> u64 {
    let a = a * 2 + 1;
    let b = (1u64 << 19) / a;
    b.div_ceil(2)
}

/// `RecipSqrtEstimate`.
fn recip_sqrt_estimate(a: u64) -> u64 {
    let a = if a < 256 { a * 2 + 1 } else { (((a >> 1) << 1) + 1) * 2 };
    let mut b = 512u64;
    while a * (b + 1) * (b + 1) < (1 << 28) {
        b += 1;
    }
    b.div_ceil(2)
}

/// `FPRecipEstimate` (round to nearest, no flush to zero).
fn recpe(f: Fmt, a: u64, env: &mut FpEnv) -> u64 {
    let n = f.exp + f.frac + 1;
    let neg = sign_of(f, a);
    if f.is_nan(a) {
        return fp::add(f, a, a, env);
    }
    if is_inf(f, a) {
        return u64::from(neg) << (n - 1);
    }
    if is_zero(f, a) {
        env.flags |= fp::DZC;
        return inf(f, neg);
    }
    // Below 2^-(2^(E-1)): the result overflows (to infinity when rounding to nearest).
    let e = exp_field(f, a) as i64;
    let frac_bits = f.frac;
    // fraction as a 52-bit field.
    let mut fraction = (a & ((1 << frac_bits) - 1)) << (52 - frac_bits);
    let mut exp = e;
    if exp == 0 {
        // Abs(value) < 2^-(2^(E-1)) when the leading two fraction bits are 0.
        if fraction >> 50 == 0 {
            return inf(f, neg);
        }
        if fraction >> 51 & 1 == 0 {
            exp = -1;
            fraction = (fraction << 2) & ((1 << 52) - 1);
        } else {
            fraction = (fraction << 1) & ((1 << 52) - 1);
        }
    }
    let scaled = (1 << 8) | (fraction >> 44);
    let mut result_exp = if f == fp::DOUBLE { 2045 - exp } else { 253 - exp };
    let estimate = recip_estimate(scaled);
    let mut fr = (estimate & 0xFF) << 44;
    if result_exp == 0 {
        fr = (1 << 51) | (fr >> 1);
    } else if result_exp == -1 {
        fr = (1 << 50) | (fr >> 2);
        result_exp = 0;
    }
    let exp_mask = (1u64 << f.exp) - 1;
    (u64::from(neg) << (n - 1)) | ((result_exp as u64 & exp_mask) << frac_bits) | (fr >> (52 - frac_bits))
}

/// `FPRSqrtEstimate`.
fn rsqrte(f: Fmt, a: u64, env: &mut FpEnv) -> u64 {
    let neg = sign_of(f, a);
    if f.is_nan(a) {
        return fp::add(f, a, a, env);
    }
    if is_zero(f, a) {
        env.flags |= fp::DZC;
        return inf(f, neg);
    }
    if neg {
        env.flags |= fp::IOC;
        return f.default_nan();
    }
    if is_inf(f, a) {
        return 0;
    }
    let frac_bits = f.frac;
    let mut fraction = (a & ((1 << frac_bits) - 1)) << (52 - frac_bits);
    let mut exp = exp_field(f, a) as i64;
    if exp == 0 {
        while fraction >> 51 & 1 == 0 {
            fraction = (fraction << 1) & ((1 << 52) - 1);
            exp -= 1;
        }
        fraction = (fraction << 1) & ((1 << 52) - 1);
    }
    let scaled = if exp & 1 == 0 { (1 << 8) | (fraction >> 44) } else { (1 << 7) | (fraction >> 45) };
    let result_exp = if f == fp::DOUBLE { (3068 - exp) / 2 } else { (380 - exp) / 2 };
    let estimate = recip_sqrt_estimate(scaled);
    let exp_mask = (1u64 << f.exp) - 1;
    ((result_exp as u64 & exp_mask) << frac_bits) | ((estimate & 0xFF) << (frac_bits - 8))
}

/// One lane of a floating-point three-register operation.
fn fp3(op: VFp3Op, f: Fmt, a: u64, b: u64, d: u64, env: &mut FpEnv) -> u64 {
    let ones = mask(f.exp + f.frac + 1);
    let t = |c: bool| if c { ones } else { 0 };
    let cmp = |x: u64, y: u64, signal: bool, env: &mut FpEnv| fp::compare(f, x, y, signal, env) >> 28;
    match op {
        VFp3Op::Add | VFp3Op::Addp => fp::add(f, a, b, env),
        VFp3Op::Sub => fp::sub(f, a, b, env),
        VFp3Op::Mul => fp::mul(f, a, b, env),
        VFp3Op::Div => fp::div(f, a, b, env),
        VFp3Op::Max | VFp3Op::Maxp => fp::max_min(f, a, b, false, env),
        VFp3Op::Min | VFp3Op::Minp => fp::max_min(f, a, b, true, env),
        VFp3Op::MaxNm | VFp3Op::MaxNmp => fp::max_min_num(f, a, b, false, env),
        VFp3Op::MinNm | VFp3Op::MinNmp => fp::max_min_num(f, a, b, true, env),
        VFp3Op::Mla => fp::fma(f, d, a, b, env),
        VFp3Op::Mls => fp::fma(f, d, f.neg(a), b, env),
        VFp3Op::Abd => f.abs(fp::sub(f, a, b, env)),
        VFp3Op::Mulx => mulx(f, a, b, env),
        // compare() gives NZCV: 0110 equal, 1000 less, 0010 greater, 0011 unordered.
        VFp3Op::Cmeq => t(cmp(a, b, false, env) == 0b0110),
        VFp3Op::Cmge => t(matches!(cmp(a, b, true, env), 0b0110 | 0b0010)),
        VFp3Op::Cmgt => t(cmp(a, b, true, env) == 0b0010),
        VFp3Op::Acge => t(matches!(cmp(f.abs(a), f.abs(b), true, env), 0b0110 | 0b0010)),
        VFp3Op::Acgt => t(cmp(f.abs(a), f.abs(b), true, env) == 0b0010),
        VFp3Op::Recps => recps(f, a, b, env),
        VFp3Op::Rsqrts => rsqrts(f, a, b, env),
    }
}

impl Cpu {
    pub(crate) fn fp_env_simd(&self) -> FpEnv {
        FpEnv { flags: 0, dn: self.fpcr & (1 << 25) != 0 }
    }

    fn put(&mut self, rd: Reg, v: u128, q: bool) {
        self.v[rd as usize] = if q { v } else { v & u128::from(u64::MAX) };
    }

    fn qc(&mut self, saturated: bool) {
        if saturated {
            self.fpsr |= FPSR_QC;
        }
    }

    pub(crate) fn exec_simd2(&mut self, inst: Inst) -> Result<(), Trap> {
        match inst {
            Inst::Vec3Same { op, q, size, scalar, rd, rn, rm } => self.int3(op, q, size, scalar, rd, rn, rm),
            Inst::VecFp3 { op, q, ft, scalar, rd, rn, rm } => {
                let f = fmt(ft);
                let bits = fbits(ft);
                let lanes = if scalar { 1 } else { (if q { 128 } else { 64 }) / bits };
                let (n, m, d) = (self.v[rn as usize], self.v[rm as usize], self.v[rd as usize]);
                let mut env = self.fp_env_simd();
                let mut r = 0u128;
                if matches!(op, VFp3Op::Addp | VFp3Op::Maxp | VFp3Op::Minp | VFp3Op::MaxNmp | VFp3Op::MinNmp) {
                    let half = lanes / 2;
                    for i in 0..half {
                        let x = fp3(op, f, get(n, bits, 2 * i), get(n, bits, 2 * i + 1), 0, &mut env);
                        set(&mut r, bits, i, x);
                    }
                    for i in 0..half {
                        let x = fp3(op, f, get(m, bits, 2 * i), get(m, bits, 2 * i + 1), 0, &mut env);
                        set(&mut r, bits, half + i, x);
                    }
                } else {
                    for i in 0..lanes {
                        let x = fp3(op, f, get(n, bits, i), get(m, bits, i), get(d, bits, i), &mut env);
                        set(&mut r, bits, i, x);
                    }
                }
                self.fpsr |= env.flags;
                if scalar {
                    self.v[rd as usize] = r & u128::from(mask(bits));
                } else {
                    self.put(rd, r, q);
                }
            }
            Inst::VecFp2 { op, q, ft, scalar, rd, rn } => self.fp2(op, q, ft, scalar, rd, rn),
            Inst::VecFpAcross { op, rd, rn } => {
                let n = self.v[rn as usize];
                let mut env = self.fp_env_simd();
                let f = fp::SINGLE;
                let o = |a: u64, b: u64, env: &mut FpEnv| match op {
                    FAcrossOp::MaxNmv => fp::max_min_num(f, a, b, false, env),
                    FAcrossOp::MinNmv => fp::max_min_num(f, a, b, true, env),
                    FAcrossOp::Maxv => fp::max_min(f, a, b, false, env),
                    FAcrossOp::Minv => fp::max_min(f, a, b, true, env),
                };
                let lo = o(get(n, 32, 0), get(n, 32, 1), &mut env);
                let hi = o(get(n, 32, 2), get(n, 32, 3), &mut env);
                let r = o(lo, hi, &mut env);
                self.fpsr |= env.flags;
                self.v[rd as usize] = u128::from(r);
            }
            Inst::ScalarPair { op, ft, rd, rn } => {
                let n = self.v[rn as usize];
                match op {
                    None => self.v[rd as usize] = u128::from(get(n, 64, 0).wrapping_add(get(n, 64, 1))),
                    Some(op) => {
                        let bits = fbits(ft);
                        let mut env = self.fp_env_simd();
                        let r = fp3(op, fmt(ft), get(n, bits, 0), get(n, bits, 1), 0, &mut env);
                        self.fpsr |= env.flags;
                        self.v[rd as usize] = u128::from(r);
                    }
                }
            }
            Inst::VecDiff { op, unsigned, round, q, size, rd, rn, rm } => {
                self.diff(op, unsigned, round, q, size, rd, rn, rm, None);
            }
            Inst::VecElem { op, q, size, scalar, rd, rn, rm, index } => {
                self.elem(op, q, size, scalar, rd, rn, rm, index)
            }
            Inst::VecMisc2 { op, q, size, scalar, rd, rn } => self.misc2(op, q, size, scalar, rd, rn),
            Inst::VecShift2 { op, q, size, scalar, shift, rd, rn } => {
                self.shift2(op, q, size, scalar, u32::from(shift), rd, rn)
            }
            Inst::Sha3 { op, rd, rn, rm, ra, imm } => {
                let (n, m, a) = (self.v[rn as usize], self.v[rm as usize], self.v[ra as usize]);
                let lanes = |f: &dyn Fn(u64, u64) -> u64| {
                    let mut r = 0u128;
                    for i in 0..2 {
                        set(&mut r, 64, i, f(get(n, 64, i), get(m, 64, i)));
                    }
                    r
                };
                self.v[rd as usize] = match op {
                    Sha3Op::Eor3 => n ^ m ^ a,
                    Sha3Op::Bcax => n ^ (m & !a),
                    Sha3Op::Rax1 => lanes(&|x, y| x ^ y.rotate_left(1)),
                    Sha3Op::Xar => lanes(&|x, y| (x ^ y).rotate_right(u32::from(imm))),
                };
            }
            _ => return Err(Trap::Unsupported),
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn int3(&mut self, op: V3Op, q: bool, size: u8, scalar: bool, rd: Reg, rn: Reg, rm: Reg) {
        let bits = 8u32 << size;
        let lanes = if scalar { 1 } else { (if q { 128 } else { 64 }) / bits };
        let (n, m, d) = (self.v[rn as usize], self.v[rm as usize], self.v[rd as usize]);
        let mut r = 0u128;
        let mut saturated = false;
        if matches!(op, V3Op::Smaxp | V3Op::Sminp | V3Op::Umaxp | V3Op::Uminp) {
            let signed = matches!(op, V3Op::Smaxp | V3Op::Sminp);
            let max = matches!(op, V3Op::Smaxp | V3Op::Umaxp);
            let half = lanes / 2;
            let pick = |a: u64, b: u64| {
                let (x, y) = (int(a, bits, signed), int(b, bits, signed));
                if (x >= y) == max { a } else { b }
            };
            for i in 0..half {
                set(&mut r, bits, i, pick(get(n, bits, 2 * i), get(n, bits, 2 * i + 1)));
                set(&mut r, bits, half + i, pick(get(m, bits, 2 * i), get(m, bits, 2 * i + 1)));
            }
            return self.put(rd, r, q);
        }
        for i in 0..lanes {
            let (a, b, c) = (get(n, bits, i), get(m, bits, i), get(d, bits, i));
            let s_ops = matches!(
                op,
                V3Op::Shadd
                    | V3Op::Srhadd
                    | V3Op::Shsub
                    | V3Op::Sqadd
                    | V3Op::Sqsub
                    | V3Op::Sqshl
                    | V3Op::Srshl
                    | V3Op::Sqrshl
                    | V3Op::Sabd
                    | V3Op::Saba
                    | V3Op::Sqdmulh
                    | V3Op::Sqrdmulh
                    | V3Op::Sqrdmlah
                    | V3Op::Sqrdmlsh
            );
            let (x, y, z) = (int(a, bits, s_ops), int(b, bits, s_ops), int(c, bits, s_ops));
            let sh = b as u8 as i8 as i32;
            let mut s = |v: i128| {
                let (o, f) = sat(v, bits, s_ops);
                saturated |= f;
                o
            };
            let res = match op {
                V3Op::Shadd | V3Op::Uhadd => ((x + y) >> 1) as u64,
                V3Op::Srhadd | V3Op::Urhadd => ((x + y + 1) >> 1) as u64,
                V3Op::Shsub | V3Op::Uhsub => ((x - y) >> 1) as u64,
                V3Op::Sqadd | V3Op::Uqadd => s(x + y),
                V3Op::Sqsub | V3Op::Uqsub => s(x - y),
                V3Op::Sqshl | V3Op::Uqshl => s(shift_signed(x, sh, false)),
                V3Op::Sqrshl | V3Op::Uqrshl => s(shift_signed(x, sh, true)),
                V3Op::Srshl | V3Op::Urshl => shift_signed(x, sh, true) as u64,
                V3Op::Sabd | V3Op::Uabd => (x - y).unsigned_abs() as u64,
                V3Op::Saba | V3Op::Uaba => (c as i128 + (x - y).abs()) as u64,
                V3Op::Sqdmulh => s((2 * x * y) >> bits),
                V3Op::Sqrdmulh => s((2 * x * y + (1 << (bits - 1))) >> bits),
                V3Op::Sqrdmlah => s(((z << bits) + 2 * x * y + (1 << (bits - 1))) >> bits),
                V3Op::Sqrdmlsh => s(((z << bits) - 2 * x * y + (1 << (bits - 1))) >> bits),
                V3Op::Pmul => pmul(a, b, 8) as u64,
                _ => 0,
            };
            set(&mut r, bits, i, res);
        }
        self.qc(saturated);
        if scalar {
            self.v[rd as usize] = r & u128::from(mask(bits));
        } else {
            self.put(rd, r, q);
        }
    }

    fn fp2(&mut self, op: VFp2Op, q: bool, ft: FpType, scalar: bool, rd: Reg, rn: Reg) {
        let f = fmt(ft);
        let bits = fbits(ft);
        let n = self.v[rn as usize];
        let mut env = self.fp_env_simd();
        let mut r = 0u128;
        match op {
            VFp2Op::Fcvtl => {
                // Single to double from the lower or upper half.
                let base = if q { 2 } else { 0 };
                for i in 0..2 {
                    let x = fp::convert(fp::SINGLE, fp::DOUBLE, get(n, 32, base + i), &mut env);
                    set(&mut r, 64, i, x);
                }
                self.fpsr |= env.flags;
                self.v[rd as usize] = r;
                return;
            }
            VFp2Op::Fcvtn => {
                let mut out = if q { self.v[rd as usize] & u128::from(u64::MAX) } else { 0 };
                let base = if q { 2 } else { 0 };
                for i in 0..2 {
                    let x = fp::convert(fp::DOUBLE, fp::SINGLE, get(n, 64, i), &mut env);
                    set(&mut out, 32, base + i, x);
                }
                self.fpsr |= env.flags;
                self.v[rd as usize] = out;
                return;
            }
            _ => {}
        }
        let lanes = if scalar { 1 } else { (if q { 128 } else { 64 }) / bits };
        let ones = mask(bits);
        for i in 0..lanes {
            let a = get(n, bits, i);
            let t = |c: bool| if c { ones } else { 0 };
            let x = match op {
                VFp2Op::Abs => f.abs(a),
                VFp2Op::Neg => f.neg(a),
                VFp2Op::Sqrt => fp::sqrt(f, a, &mut env),
                VFp2Op::Rint(mode) => fp::round_to_int(f, a, mode, &mut env),
                VFp2Op::RintX => fp::round_to_int(f, a, FpRound::NearestEven, &mut env),
                VFp2Op::ToInt(mode, u) => fp::to_int(f, a, !u, bits == 64, mode, 0, &mut env) & ones,
                VFp2Op::FromInt(u) => fp::from_int(f, a, !u, bits == 64, 0),
                VFp2Op::CmZero(c) => {
                    let signal = c != CmpZero::Eq;
                    let nz = fp::compare(f, a, 0, signal, &mut env) >> 28;
                    t(match c {
                        CmpZero::Gt => nz == 0b0010,
                        CmpZero::Ge => matches!(nz, 0b0010 | 0b0110),
                        CmpZero::Eq => nz == 0b0110,
                        CmpZero::Le => matches!(nz, 0b1000 | 0b0110),
                        CmpZero::Lt => nz == 0b1000,
                    })
                }
                VFp2Op::Recpe => recpe(f, a, &mut env),
                VFp2Op::Rsqrte => rsqrte(f, a, &mut env),
                VFp2Op::Urecpe => {
                    if a >> 31 == 0 {
                        0xFFFF_FFFF
                    } else {
                        recip_estimate(a >> 23) << 23
                    }
                }
                VFp2Op::Ursqrte => {
                    if a >> 30 == 0 {
                        0xFFFF_FFFF
                    } else {
                        recip_sqrt_estimate(a >> 23) << 23
                    }
                }
                VFp2Op::Fcvtl | VFp2Op::Fcvtn => 0,
            };
            set(&mut r, bits, i, x);
        }
        self.fpsr |= env.flags;
        if scalar {
            self.v[rd as usize] = r & u128::from(ones);
        } else {
            self.put(rd, r, q);
        }
    }

    /// Long, wide and narrow operations; `elem` replaces `Vm`'s lanes with one
    /// value (the by-element long forms).
    #[allow(clippy::too_many_arguments)]
    fn diff(
        &mut self,
        op: DiffOp,
        unsigned: bool,
        round: bool,
        q: bool,
        size: u8,
        rd: Reg,
        rn: Reg,
        rm: Reg,
        elem: Option<u64>,
    ) {
        let b = 8u32 << size;
        let w = 2 * b;
        let (n, m, d) = (self.v[rn as usize], self.v[rm as usize], self.v[rd as usize]);
        let signed = !unsigned;
        let part = if q { 64 / b } else { 0 };
        let count = 64 / b;
        let mut r = 0u128;
        let mut saturated = false;
        if op == DiffOp::Pmull {
            if size == 3 {
                let (x, y) = (get(n, 64, u32::from(q)), get(m, 64, u32::from(q)));
                self.v[rd as usize] = pmul(x, y, 64);
                return;
            }
            for i in 0..count {
                let p = pmul(get(n, b, part + i), get(m, b, part + i), b) as u64;
                set(&mut r, w, i, p);
            }
            self.v[rd as usize] = r;
            return;
        }
        if matches!(op, DiffOp::Addhn | DiffOp::Subhn) {
            let mut out = if q { d & u128::from(u64::MAX) } else { 0 };
            for i in 0..count {
                let (x, y) = (u128::from(get(n, w, i)), u128::from(get(m, w, i)));
                let mut v = if op == DiffOp::Addhn { x.wrapping_add(y) } else { x.wrapping_sub(y) };
                if round {
                    v = v.wrapping_add(1 << (b - 1));
                }
                set(&mut out, b, part + i, (v >> b) as u64);
            }
            self.v[rd as usize] = out;
            return;
        }
        for i in 0..count {
            let x_wide = matches!(op, DiffOp::Addw | DiffOp::Subw);
            let x = if x_wide { int(get(n, w, i), w, signed) } else { int(get(n, b, part + i), b, signed) };
            let y = match elem {
                Some(e) => int(e, b, signed),
                None => int(get(m, b, part + i), b, signed),
            };
            let acc = int(get(d, w, i), w, true);
            let mut s = |v: i128| {
                let (o, f) = sat(v, w, true);
                saturated |= f;
                o
            };
            let res = match op {
                DiffOp::Addl | DiffOp::Addw => (x + y) as u64,
                DiffOp::Subl | DiffOp::Subw => (x - y) as u64,
                DiffOp::Abdl => (x - y).unsigned_abs() as u64,
                DiffOp::Abal => (acc + (x - y).abs()) as u64,
                DiffOp::Mlal => (acc + x * y) as u64,
                DiffOp::Mlsl => (acc - x * y) as u64,
                DiffOp::Mull => (x * y) as u64,
                DiffOp::Sqdmull => s(2 * x * y),
                DiffOp::Sqdmlal => {
                    let p = i128::from(sx(s(2 * x * y), w));
                    s(acc + p)
                }
                DiffOp::Sqdmlsl => {
                    let p = i128::from(sx(s(2 * x * y), w));
                    s(acc - p)
                }
                _ => 0,
            };
            set(&mut r, w, i, res);
        }
        self.qc(saturated);
        self.v[rd as usize] = r;
    }

    #[allow(clippy::too_many_arguments)]
    fn elem(&mut self, op: ElemOp, q: bool, size: u8, scalar: bool, rd: Reg, rn: Reg, rm: Reg, index: u8) {
        let bits = 8u32 << size;
        let m = self.v[rm as usize];
        match op {
            ElemOp::Long(dop, u) => {
                let e = get(m, bits, u32::from(index));
                return self.diff(dop, u, false, q, size, rd, rn, rm, Some(e));
            }
            ElemOp::Dot(u) => {
                let (n, d) = (self.v[rn as usize], self.v[rd as usize]);
                let lanes = if q { 4 } else { 2 };
                let mut r = 0u128;
                for i in 0..lanes {
                    let j = if index == 0xFF { i } else { u32::from(index) };
                    let mut acc = get(d, 32, i) as u32;
                    for k in 0..4 {
                        let (a, b) = (get(n, 8, 4 * i + k), get(m, 8, 4 * j + k));
                        let p = if u { (a * b) as u32 } else { (sx(a, 8) * sx(b, 8)) as u32 };
                        acc = acc.wrapping_add(p);
                    }
                    set(&mut r, 32, i, u64::from(acc));
                }
                return self.put(rd, r, q);
            }
            _ => {}
        }
        let e = get(m, bits, u32::from(index));
        let lanes = if scalar { 1 } else { (if q { 128 } else { 64 }) / bits };
        let (n, d) = (self.v[rn as usize], self.v[rd as usize]);
        let mut r = 0u128;
        let mut env = self.fp_env_simd();
        let mut saturated = false;
        for i in 0..lanes {
            let (a, c) = (get(n, bits, i), get(d, bits, i));
            let (x, y, z) = (int(a, bits, true), int(e, bits, true), int(c, bits, true));
            let mut s = |v: i128| {
                let (o, f) = sat(v, bits, true);
                saturated |= f;
                o
            };
            let f = if bits == 64 { fp::DOUBLE } else { fp::SINGLE };
            let res = match op {
                ElemOp::Mul => a.wrapping_mul(e),
                ElemOp::Mla => c.wrapping_add(a.wrapping_mul(e)),
                ElemOp::Mls => c.wrapping_sub(a.wrapping_mul(e)),
                ElemOp::Sqdmulh => s((2 * x * y) >> bits),
                ElemOp::Sqrdmulh => s((2 * x * y + (1 << (bits - 1))) >> bits),
                ElemOp::Sqrdmlah => s(((z << bits) + 2 * x * y + (1 << (bits - 1))) >> bits),
                ElemOp::Sqrdmlsh => s(((z << bits) - 2 * x * y + (1 << (bits - 1))) >> bits),
                ElemOp::Fmul => fp::mul(f, a, e, &mut env),
                ElemOp::Fmulx => mulx(f, a, e, &mut env),
                ElemOp::Fmla => fp::fma(f, c, a, e, &mut env),
                ElemOp::Fmls => fp::fma(f, c, f.neg(a), e, &mut env),
                ElemOp::Long(..) | ElemOp::Dot(_) => 0,
            };
            set(&mut r, bits, i, res);
        }
        self.qc(saturated);
        self.fpsr |= env.flags;
        if scalar {
            self.v[rd as usize] = r & u128::from(mask(bits));
        } else {
            self.put(rd, r, q);
        }
    }

    fn misc2(&mut self, op: Misc2Op, q: bool, size: u8, scalar: bool, rd: Reg, rn: Reg) {
        let bits = 8u32 << size;
        let (n, d) = (self.v[rn as usize], self.v[rd as usize]);
        let mut r = 0u128;
        let mut saturated = false;
        match op {
            Misc2Op::Addlp(u) | Misc2Op::Adalp(u) => {
                let lanes = (if q { 128 } else { 64 }) / bits;
                let w = 2 * bits;
                for i in 0..lanes / 2 {
                    let s = int(get(n, bits, 2 * i), bits, !u) + int(get(n, bits, 2 * i + 1), bits, !u);
                    let acc = if matches!(op, Misc2Op::Adalp(_)) { i128::from(get(d, w, i)) } else { 0 };
                    set(&mut r, w, i, (acc + s) as u64);
                }
                return self.put(rd, r, q);
            }
            Misc2Op::Sqxtn | Misc2Op::Uqxtn | Misc2Op::Sqxtun => {
                // Narrow from 2 * bits to bits.
                let w = 2 * bits;
                let count = if scalar { 1 } else { 64 / bits };
                let mut out = if q && !scalar { d & u128::from(u64::MAX) } else { 0 };
                let part = if q && !scalar { 64 / bits } else { 0 };
                for i in 0..count {
                    let src_signed = op != Misc2Op::Uqxtn;
                    let x = int(get(n, w, i), w, src_signed);
                    let (o, f) = sat(x, bits, op == Misc2Op::Sqxtn);
                    saturated |= f;
                    set(&mut out, bits, part + i, o);
                }
                self.qc(saturated);
                self.v[rd as usize] = out;
                return;
            }
            Misc2Op::Shll => {
                let part = if q { 64 / bits } else { 0 };
                for i in 0..64 / bits {
                    set(&mut r, 2 * bits, i, get(n, bits, part + i) << bits);
                }
                self.v[rd as usize] = r;
                return;
            }
            _ => {}
        }
        let lanes = if scalar { 1 } else { (if q { 128 } else { 64 }) / bits };
        for i in 0..lanes {
            let (a, c) = (get(n, bits, i), get(d, bits, i));
            let (res, f) = match op {
                Misc2Op::Sqabs => sat(int(a, bits, true).abs(), bits, true),
                Misc2Op::Sqneg => sat(-int(a, bits, true), bits, true),
                Misc2Op::Suqadd => sat(int(c, bits, true) + int(a, bits, false), bits, true),
                Misc2Op::Usqadd => sat(int(c, bits, false) + int(a, bits, true), bits, false),
                _ => (0, false),
            };
            saturated |= f;
            set(&mut r, bits, i, res);
        }
        self.qc(saturated);
        if scalar {
            self.v[rd as usize] = r & u128::from(mask(bits));
        } else {
            self.put(rd, r, q);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn shift2(&mut self, op: Shift2Op, q: bool, size: u8, scalar: bool, shift: u32, rd: Reg, rn: Reg) {
        let bits = 8u32 << size;
        let (n, d) = (self.v[rn as usize], self.v[rd as usize]);
        let mut saturated = false;
        let narrow = matches!(op, Shift2Op::Rshrn | Shift2Op::Qshrn { .. } | Shift2Op::Qshrun { .. });
        if narrow {
            let w = 2 * bits;
            let count = if scalar { 1 } else { 64 / bits };
            let mut out = if q && !scalar { d & u128::from(u64::MAX) } else { 0 };
            let part = if q && !scalar { 64 / bits } else { 0 };
            for i in 0..count {
                let (src_signed, round, dst_signed, saturate) = match op {
                    Shift2Op::Rshrn => (false, true, false, false),
                    Shift2Op::Qshrn { unsigned, round } => (!unsigned, round, !unsigned, true),
                    Shift2Op::Qshrun { round } => (true, round, false, true),
                    _ => (false, false, false, false),
                };
                let x = int(get(n, w, i), w, src_signed);
                let v = shift_signed(x, -(shift as i32), round);
                let o = if saturate {
                    let (o, f) = sat(v, bits, dst_signed);
                    saturated |= f;
                    o
                } else {
                    v as u64 & mask(bits)
                };
                set(&mut out, bits, part + i, o);
            }
            self.qc(saturated);
            self.v[rd as usize] = out;
            return;
        }
        let lanes = if scalar { 1 } else { (if q { 128 } else { 64 }) / bits };
        let mut r = 0u128;
        let mut env = self.fp_env_simd();
        for i in 0..lanes {
            let (a, c) = (get(n, bits, i), get(d, bits, i));
            let res = match op {
                Shift2Op::Rshr(u) => shift_signed(int(a, bits, !u), -(shift as i32), true) as u64,
                Shift2Op::Rsra(u) => c.wrapping_add(shift_signed(int(a, bits, !u), -(shift as i32), true) as u64),
                Shift2Op::Qshl(u) => {
                    let (o, f) = sat(int(a, bits, !u) << shift, bits, !u);
                    saturated |= f;
                    o
                }
                Shift2Op::Sqshlu => {
                    let (o, f) = sat(int(a, bits, true) << shift, bits, false);
                    saturated |= f;
                    o
                }
                Shift2Op::CvtFrom(u) => {
                    let f = if bits == 64 { fp::DOUBLE } else { fp::SINGLE };
                    fp::from_int(f, a, !u, bits == 64, shift)
                }
                Shift2Op::CvtTo(u) => {
                    let f = if bits == 64 { fp::DOUBLE } else { fp::SINGLE };
                    fp::to_int(f, a, !u, bits == 64, FpRound::Zero, shift, &mut env) & mask(bits)
                }
                _ => 0,
            };
            set(&mut r, bits, i, res);
        }
        self.qc(saturated);
        self.fpsr |= env.flags;
        if scalar {
            self.v[rd as usize] = r & u128::from(mask(bits));
        } else {
            self.put(rd, r, q);
        }
    }
}
