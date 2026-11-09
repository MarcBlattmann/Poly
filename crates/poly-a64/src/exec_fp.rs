//! Scalar floating-point instructions. The arithmetic itself is in
//! [`crate::fp`]; this file moves values between registers and the helpers.

use crate::cpu::{Cpu, Trap};
use crate::fp::{self, Fmt, FpEnv};
use crate::inst::*;

fn fmt_of(ft: FpType) -> Fmt {
    match ft {
        FpType::H => fp::HALF,
        FpType::S => fp::SINGLE,
        FpType::D => fp::DOUBLE,
    }
}

impl Cpu {
    /// The low bits of `Vn` as the format's raw value.
    fn fp_in(&self, ft: FpType, r: Reg) -> u64 {
        let v = self.v[r as usize] as u64;
        match ft {
            FpType::H => v & 0xFFFF,
            FpType::S => v & 0xFFFF_FFFF,
            FpType::D => v,
        }
    }

    /// Write a scalar result; the rest of the register is zeroed.
    fn fp_out(&mut self, r: Reg, bits: u64) {
        self.v[r as usize] = u128::from(bits);
    }

    fn fp_env(&self) -> FpEnv {
        FpEnv { flags: 0, dn: self.fpcr & (1 << 25) != 0 }
    }

    fn fp_commit(&mut self, env: FpEnv) {
        self.fpsr |= env.flags;
    }

    pub(crate) fn exec_fp(&mut self, inst: Inst) -> Result<(), Trap> {
        let mut env = self.fp_env();
        match inst {
            Inst::FpUnary { op, ft, rd, rn } => {
                let f = fmt_of(ft);
                let a = self.fp_in(ft, rn);
                if ft == FpType::H && !matches!(op, Fp1Op::Mov | Fp1Op::Abs | Fp1Op::Neg) {
                    return Err(Trap::Unsupported);
                }
                let r = match op {
                    Fp1Op::Mov => a,
                    Fp1Op::Abs => f.abs(a),
                    Fp1Op::Neg => f.neg(a),
                    Fp1Op::Sqrt => fp::sqrt(f, a, &mut env),
                    Fp1Op::RintN | Fp1Op::RintX | Fp1Op::RintI => {
                        fp::round_to_int(f, a, FpRound::NearestEven, &mut env)
                    }
                    Fp1Op::RintP => fp::round_to_int(f, a, FpRound::PosInf, &mut env),
                    Fp1Op::RintM => fp::round_to_int(f, a, FpRound::NegInf, &mut env),
                    Fp1Op::RintZ => fp::round_to_int(f, a, FpRound::Zero, &mut env),
                    Fp1Op::RintA => fp::round_to_int(f, a, FpRound::Away, &mut env),
                };
                self.fp_out(rd, r);
            }
            Inst::FpCvt { from, to, rd, rn } => {
                let a = self.fp_in(from, rn);
                let r = fp::convert(fmt_of(from), fmt_of(to), a, &mut env);
                self.fp_out(rd, r);
            }
            Inst::FpBinary { op, ft, rd, rn, rm } => {
                if ft == FpType::H {
                    return Err(Trap::Unsupported);
                }
                let f = fmt_of(ft);
                let a = self.fp_in(ft, rn);
                let b = self.fp_in(ft, rm);
                let r = match op {
                    Fp2Op::Add => fp::add(f, a, b, &mut env),
                    Fp2Op::Sub => fp::sub(f, a, b, &mut env),
                    Fp2Op::Mul => fp::mul(f, a, b, &mut env),
                    Fp2Op::Nmul => f.neg(fp::mul(f, a, b, &mut env)),
                    Fp2Op::Div => fp::div(f, a, b, &mut env),
                    Fp2Op::Max => fp::max_min(f, a, b, false, &mut env),
                    Fp2Op::Min => fp::max_min(f, a, b, true, &mut env),
                    Fp2Op::MaxNm => fp::max_min_num(f, a, b, false, &mut env),
                    Fp2Op::MinNm => fp::max_min_num(f, a, b, true, &mut env),
                };
                self.fp_out(rd, r);
            }
            Inst::FpFused { ft, neg_p, neg_a, rd, rn, rm, ra } => {
                if ft == FpType::H {
                    return Err(Trap::Unsupported);
                }
                let f = fmt_of(ft);
                let mut a = self.fp_in(ft, ra);
                let mut x = self.fp_in(ft, rn);
                let y = self.fp_in(ft, rm);
                if neg_a {
                    a = f.neg(a);
                }
                if neg_p {
                    x = f.neg(x);
                }
                let r = fp::fma(f, a, x, y, &mut env);
                self.fp_out(rd, r);
            }
            Inst::FpCmp { ft, signal, zero, rn, rm } => {
                let f = fmt_of(ft);
                let a = self.fp_in(ft, rn);
                let b = if zero { 0 } else { self.fp_in(ft, rm) };
                self.nzcv = fp::compare(f, a, b, signal, &mut env);
            }
            Inst::FpCondCmp { ft, signal, rn, rm, cond, nzcv } => {
                if self.cond_holds(cond) {
                    let f = fmt_of(ft);
                    let a = self.fp_in(ft, rn);
                    let b = self.fp_in(ft, rm);
                    self.nzcv = fp::compare(f, a, b, signal, &mut env);
                } else {
                    self.nzcv = u32::from(nzcv) << 28;
                }
            }
            Inst::FpCondSel { ft, rd, rn, rm, cond } => {
                let r = if self.cond_holds(cond) { self.fp_in(ft, rn) } else { self.fp_in(ft, rm) };
                self.fp_out(rd, r);
            }
            Inst::FpMovImm { rd, bits, .. } => self.fp_out(rd, bits),
            Inst::FpToInt { ft, w, unsigned, round, rd, rn, fbits } => {
                let f = fmt_of(ft);
                let a = self.fp_in(ft, rn);
                let r = fp::to_int(f, a, !unsigned, w == Width::X, round, u32::from(fbits), &mut env);
                self.set_rw(rd, w, r);
            }
            Inst::IntToFp { ft, w, unsigned, rd, rn, fbits } => {
                let f = fmt_of(ft);
                let a = self.rw(rn, w);
                let r = fp::from_int(f, a, !unsigned, w == Width::X, u32::from(fbits));
                self.fp_out(rd, r);
            }
            Inst::FmovToGen { w, top, rd, rn } => {
                let v = self.v[rn as usize];
                let x = if top { (v >> 64) as u64 } else { v as u64 };
                self.set_rw(rd, w, x);
            }
            Inst::FmovFromGen { w, top, rd, rn } => {
                let x = self.rw(rn, w);
                if top {
                    let lo = self.v[rd as usize] as u64;
                    self.v[rd as usize] = u128::from(lo) | (u128::from(x) << 64);
                } else {
                    self.v[rd as usize] = u128::from(x);
                }
            }
            _ => {}
        }
        self.fp_commit(env);
        Ok(())
    }
}
