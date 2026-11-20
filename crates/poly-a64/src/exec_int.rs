//! Integer data processing and branches.
//!
//! Flag handling follows the Arm ARM's `AddWithCarry` pseudocode exactly:
//! `N` is the top bit of the result, `Z` is set if the result is zero, `C`
//! is the unsigned carry out and `V` is the signed overflow, all at the
//! width of the operation (32 or 64 bits).

use crate::cpu::{Cpu, NZCV_C, NZCV_N, NZCV_V, NZCV_Z};
use crate::inst::*;

/// `AddWithCarry(x, y, carry_in)` for `w`-bit operands (already masked to
/// the width). Returns the result and the new `NZCV`.
pub fn add_with_carry(x: u64, y: u64, carry_in: bool, w: Width) -> (u64, u32) {
    let c = u64::from(carry_in);
    let (result, carry) = match w {
        Width::X => {
            let usum = u128::from(x) + u128::from(y) + u128::from(c);
            (usum as u64, (usum >> 64) != 0)
        }
        Width::W => {
            let usum = (x & 0xFFFF_FFFF) + (y & 0xFFFF_FFFF) + c;
            (usum & 0xFFFF_FFFF, (usum >> 32) != 0)
        }
    };
    let top = w.bits() - 1;
    let n = (result >> top) & 1 != 0;
    let z = result == 0;
    // Signed overflow: operands have the same sign and the result's differs.
    let v = ((!(x ^ y) & (x ^ result)) >> top) & 1 != 0;
    let mut nzcv = 0;
    if n {
        nzcv |= NZCV_N;
    }
    if z {
        nzcv |= NZCV_Z;
    }
    if carry {
        nzcv |= NZCV_C;
    }
    if v {
        nzcv |= NZCV_V;
    }
    (result, nzcv)
}

/// The flags of a logical operation: `N` and `Z` from the result, `C` and
/// `V` cleared.
fn logic_flags(result: u64, w: Width) -> u32 {
    let mut f = 0;
    if (result >> (w.bits() - 1)) & 1 != 0 {
        f |= NZCV_N;
    }
    if result == 0 {
        f |= NZCV_Z;
    }
    f
}

/// Shift or rotate `v` (already masked to the width) by `amount`.
pub fn shift_value(v: u64, kind: Shift, amount: u32, w: Width) -> u64 {
    let bits = w.bits();
    let mask = w.mask();
    let a = amount % bits.max(1);
    let amount = if amount >= bits && !matches!(kind, Shift::Ror) { bits } else { a };
    match kind {
        Shift::Lsl => {
            if amount >= bits {
                0
            } else {
                (v << amount) & mask
            }
        }
        Shift::Lsr => {
            if amount >= bits {
                0
            } else {
                v >> amount
            }
        }
        Shift::Asr => {
            let sv = match w {
                Width::X => v as i64,
                Width::W => i64::from(v as u32 as i32),
            };
            let sh = amount.min(63);
            ((sv >> sh) as u64) & mask
        }
        Shift::Ror => {
            if a == 0 {
                v
            } else {
                ((v >> a) | (v << (bits - a))) & mask
            }
        }
    }
}

/// Extend the low bits of `v` as `ext` says and shift left by `amount`; the
/// result is a 64-bit value to be used at the instruction's width.
pub fn extend_value(v: u64, ext: Extend, amount: u32) -> u64 {
    let e = match ext {
        Extend::Uxtb => v & 0xFF,
        Extend::Uxth => v & 0xFFFF,
        Extend::Uxtw => v & 0xFFFF_FFFF,
        Extend::Uxtx | Extend::Sxtx => v,
        Extend::Sxtb => v as u8 as i8 as i64 as u64,
        Extend::Sxth => v as u16 as i16 as i64 as u64,
        Extend::Sxtw => v as u32 as i32 as i64 as u64,
    };
    e << amount
}

impl Cpu {
    pub(crate) fn exec_int(&mut self, inst: Inst) {
        match inst {
            Inst::AddSubImm { w, sub, s, rd, rn, imm } => {
                let a = self.xsp(rn) & w.mask();
                self.add_sub(w, sub, s, rd, a, u64::from(imm), !s);
            }
            Inst::AddSubShifted { w, sub, s, rd, rn, rm, shift, amount } => {
                let a = self.rw(rn, w);
                let b = shift_value(self.rw(rm, w), shift, u32::from(amount), w);
                self.add_sub(w, sub, s, rd, a, b, false);
            }
            Inst::AddSubExt { w, sub, s, rd, rn, rm, ext, amount } => {
                let a = self.xsp(rn) & w.mask();
                let b = extend_value(self.xr(rm), ext, u32::from(amount)) & w.mask();
                self.add_sub(w, sub, s, rd, a, b, !s);
            }
            Inst::AddSubCarry { w, sub, s, rd, rn, rm } => {
                let a = self.rw(rn, w);
                let mut b = self.rw(rm, w);
                if sub {
                    b = !b & w.mask();
                }
                let (r, f) = add_with_carry(a, b, self.nzcv & NZCV_C != 0, w);
                if s {
                    self.nzcv = f;
                }
                self.set_rw(rd, w, r);
            }
            Inst::LogicalImm { w, op, rd, rn, imm } => {
                let a = self.rw(rn, w);
                let r = match op {
                    LogOp::And | LogOp::Ands => a & imm,
                    LogOp::Orr => a | imm,
                    LogOp::Eor => a ^ imm,
                } & w.mask();
                if op == LogOp::Ands {
                    self.nzcv = logic_flags(r, w);
                    self.set_rw(rd, w, r);
                } else {
                    self.set_rw_sp(rd, w, r);
                }
            }
            Inst::LogicalShifted { w, op, invert, rd, rn, rm, shift, amount } => {
                let a = self.rw(rn, w);
                let mut b = shift_value(self.rw(rm, w), shift, u32::from(amount), w);
                if invert {
                    b = !b & w.mask();
                }
                let r = match op {
                    LogOp::And | LogOp::Ands => a & b,
                    LogOp::Orr => a | b,
                    LogOp::Eor => a ^ b,
                } & w.mask();
                if op == LogOp::Ands {
                    self.nzcv = logic_flags(r, w);
                }
                self.set_rw(rd, w, r);
            }
            Inst::MoveWide { w, op, rd, imm, shift } => {
                let v = u64::from(imm) << shift;
                let r = match op {
                    MovOp::Movn => !v,
                    MovOp::Movz => v,
                    MovOp::Movk => (self.rw(rd, w) & !(0xFFFFu64 << shift)) | v,
                };
                self.set_rw(rd, w, r);
            }
            Inst::Adr { rd, offset, page } => {
                let base = if page { self.pc & !0xFFF } else { self.pc };
                self.set_xr(rd, base.wrapping_add(offset as u64));
            }
            Inst::Bitfield { w, op, rd, rn, immr, imms, wmask, tmask } => {
                let src = self.rw(rn, w);
                let dst = if op == BfOp::Bfm { self.rw(rd, w) } else { 0 };
                let r = u32::from(immr);
                let rotated = shift_value(src, Shift::Ror, r, w);
                let bot = (dst & !wmask) | (rotated & wmask);
                let top = match op {
                    BfOp::Sbfm => {
                        if (src >> imms) & 1 != 0 {
                            w.mask()
                        } else {
                            0
                        }
                    }
                    BfOp::Ubfm => 0,
                    BfOp::Bfm => dst,
                };
                let res = (top & !tmask) | (bot & tmask);
                self.set_rw(rd, w, res);
            }
            Inst::Extr { w, rd, rn, rm, lsb } => {
                let hi = self.rw(rn, w);
                let lo = self.rw(rm, w);
                let r = match w {
                    Width::X => {
                        if lsb == 0 {
                            lo
                        } else {
                            (hi << (64 - u32::from(lsb))) | (lo >> lsb)
                        }
                    }
                    Width::W => (((hi << 32) | lo) >> lsb) & 0xFFFF_FFFF,
                };
                self.set_rw(rd, w, r);
            }
            Inst::CondCmp { w, neg, rn, rhs, cond, nzcv } => {
                if self.cond_holds(cond) {
                    let a = self.rw(rn, w);
                    let b = match rhs {
                        CmpRhs::Reg(r) => self.rw(r, w),
                        CmpRhs::Imm(i) => u64::from(i),
                    };
                    let (_, f) =
                        if neg { add_with_carry(a, b, false, w) } else { add_with_carry(a, !b & w.mask(), true, w) };
                    self.nzcv = f;
                } else {
                    self.nzcv = u32::from(nzcv) << 28;
                }
            }
            Inst::CondSel { w, op, rd, rn, rm, cond } => {
                let r = if self.cond_holds(cond) {
                    self.rw(rn, w)
                } else {
                    let m = self.rw(rm, w);
                    match op {
                        CselOp::Csel => m,
                        CselOp::Csinc => m.wrapping_add(1),
                        CselOp::Csinv => !m,
                        CselOp::Csneg => m.wrapping_neg(),
                    }
                };
                self.set_rw(rd, w, r);
            }
            Inst::Madd { w, sub, rd, rn, rm, ra } => {
                let p = self.rw(rn, w).wrapping_mul(self.rw(rm, w));
                let a = self.rw(ra, w);
                let r = if sub { a.wrapping_sub(p) } else { a.wrapping_add(p) };
                self.set_rw(rd, w, r);
            }
            Inst::MaddLong { signed, sub, rd, rn, rm, ra } => {
                let (a, b) = if signed {
                    (i64::from(self.xr(rn) as u32 as i32) as u64, i64::from(self.xr(rm) as u32 as i32) as u64)
                } else {
                    (u64::from(self.xr(rn) as u32), u64::from(self.xr(rm) as u32))
                };
                let p = a.wrapping_mul(b);
                let acc = self.xr(ra);
                self.set_xr(rd, if sub { acc.wrapping_sub(p) } else { acc.wrapping_add(p) });
            }
            Inst::MulHigh { signed, rd, rn, rm } => {
                let (a, b) = (self.xr(rn), self.xr(rm));
                let r = if signed {
                    ((i128::from(a as i64) * i128::from(b as i64)) >> 64) as u64
                } else {
                    ((u128::from(a) * u128::from(b)) >> 64) as u64
                };
                self.set_xr(rd, r);
            }
            Inst::Div { w, signed, rd, rn, rm } => {
                let a = self.rw(rn, w);
                let b = self.rw(rm, w);
                let r = if b == 0 {
                    0
                } else if signed {
                    match w {
                        Width::X => (a as i64).wrapping_div(b as i64) as u64,
                        Width::W => (a as u32 as i32).wrapping_div(b as u32 as i32) as u32 as u64,
                    }
                } else {
                    a / b
                };
                self.set_rw(rd, w, r);
            }
            Inst::ShiftVar { w, op, rd, rn, rm } => {
                let amount = (self.xr(rm) % u64::from(w.bits())) as u32;
                let r = shift_value(self.rw(rn, w), op, amount, w);
                self.set_rw(rd, w, r);
            }
            Inst::Crc32 { c, size, rd, rn, rm } => {
                let acc = self.xr(rn) as u32;
                let val = self.xr(rm);
                self.set_xr(rd, u64::from(crc32_update(acc, val, 1 << size, c)));
            }
            // Keys disabled: PAC* and AUT* leave the register alone.
            Inst::Pac { .. } => {}
            Inst::Xpac { data, rd } => {
                let v = self.strip_pac(self.xr(rd), data);
                self.set_xr(rd, v);
            }
            Inst::Pacga { rd, rn, rm } => {
                let h = pacga_code(self.xr(rn), self.xsp(rm));
                self.set_xr(rd, u64::from(h) << 32);
            }
            Inst::Rmif { rn, shift, mask } => {
                let t = (self.xr(rn).rotate_right(u32::from(shift)) & 0xF) as u32;
                let m = u32::from(mask) << 28;
                self.nzcv = (self.nzcv & !m) | ((t << 28) & m);
            }
            Inst::Setf { rn, sixteen } => {
                let v = self.xr(rn);
                let msb = if sixteen { 15 } else { 7 };
                let n = (v >> msb) & 1;
                let z = u64::from(v & ((1u64 << (msb + 1)) - 1) == 0);
                let ov = ((v >> (msb + 1)) ^ (v >> msb)) & 1;
                self.nzcv = (self.nzcv & NZCV_C) | ((n as u32) << 31) | ((z as u32) << 30) | ((ov as u32) << 28);
            }
            Inst::FlagOp { op } => {
                let n = self.nzcv & NZCV_N != 0;
                let z = self.nzcv & NZCV_Z != 0;
                let c = self.nzcv & NZCV_C != 0;
                let v = self.nzcv & NZCV_V != 0;
                let (n, z, c, v) = match op {
                    FlagOp::Cfinv => (n, z, !c, v),
                    FlagOp::Axflag => (false, z | v, c & !v, false),
                    FlagOp::Xaflag => (!c & !z, z & c, c | z, !c & z),
                };
                self.nzcv = (u32::from(n) << 31) | (u32::from(z) << 30) | (u32::from(c) << 29) | (u32::from(v) << 28);
            }
            Inst::Bit1 { w, op, rd, rn } => {
                let a = self.rw(rn, w);
                let r = match op {
                    Bit1Op::Rbit => match w {
                        Width::X => a.reverse_bits(),
                        Width::W => u64::from((a as u32).reverse_bits()),
                    },
                    Bit1Op::Rev16 => match w {
                        Width::X => ((a & 0x00FF_00FF_00FF_00FF) << 8) | ((a >> 8) & 0x00FF_00FF_00FF_00FF),
                        Width::W => ((a & 0x00FF_00FF) << 8) | ((a >> 8) & 0x00FF_00FF),
                    },
                    Bit1Op::Rev32 => match w {
                        Width::X => {
                            u64::from((a as u32).swap_bytes()) | (u64::from(((a >> 32) as u32).swap_bytes()) << 32)
                        }
                        Width::W => u64::from((a as u32).swap_bytes()),
                    },
                    Bit1Op::Rev64 => a.swap_bytes(),
                    Bit1Op::Clz => match w {
                        Width::X => u64::from(a.leading_zeros()),
                        Width::W => u64::from((a as u32).leading_zeros()),
                    },
                    Bit1Op::Cls => {
                        let sign = (a >> (w.bits() - 1)) & 1 != 0;
                        let y = if sign { !a & w.mask() } else { a };
                        let lz = match w {
                            Width::X => y.leading_zeros(),
                            Width::W => (y as u32).leading_zeros(),
                        };
                        u64::from(lz - 1)
                    }
                };
                self.set_rw(rd, w, r);
            }
            _ => {}
        }
    }

    /// `ADD`/`SUB` common tail. `to_sp` selects `SP` for register 31 of the
    /// destination (the non-flag-setting immediate and extended forms).
    #[allow(clippy::too_many_arguments)]
    fn add_sub(&mut self, w: Width, sub: bool, s: bool, rd: Reg, a: u64, b: u64, to_sp: bool) {
        let (op2, carry) = if sub { (!b & w.mask(), true) } else { (b, false) };
        let (r, f) = add_with_carry(a, op2, carry, w);
        if s {
            self.nzcv = f;
        }
        if to_sp {
            self.set_rw_sp(rd, w, r);
        } else {
            self.set_rw(rd, w, r);
        }
    }

    /// Execute a branch. Returns the target if taken.
    pub(crate) fn exec_branch(&mut self, inst: Inst) -> Option<u64> {
        let pc = self.pc;
        match inst {
            Inst::Branch { link, offset } => {
                if link {
                    self.x[30] = pc.wrapping_add(4);
                }
                Some(pc.wrapping_add(offset as u64))
            }
            Inst::BranchCond { cond, offset } => self.cond_holds(cond).then(|| pc.wrapping_add(offset as u64)),
            Inst::CompareBranch { w, nz, rt, offset } => {
                ((self.rw(rt, w) == 0) != nz).then(|| pc.wrapping_add(offset as u64))
            }
            Inst::TestBranch { nz, rt, bit, offset } => {
                (((self.xr(rt) >> bit) & 1 != 0) == nz).then(|| pc.wrapping_add(offset as u64))
            }
            Inst::BranchReg { rn } | Inst::Ret { rn } => Some(self.xr(rn)),
            // With the keys disabled, authentication returns the pointer
            // unchanged.
            Inst::BranchAuth { link, rn, .. } => {
                let t = self.xr(rn);
                if link {
                    self.x[30] = pc.wrapping_add(4);
                }
                Some(t)
            }
            Inst::BranchLinkReg { rn } => {
                let t = self.xr(rn);
                self.x[30] = pc.wrapping_add(4);
                Some(t)
            }
            _ => None,
        }
    }
}

/// One `CRC32*` step: the CRC of the low `bytes` bytes of `val`, least
/// significant first, continued from `acc`, with the bit-reflected
/// polynomial (CRC-32 0x04C11DB7, or CRC-32C 0x1EDC6F41 when `c`). No
/// inversion before or after, as the Arm ARM's `CRC32` pseudocode does.
pub fn crc32_update(acc: u32, val: u64, bytes: u32, c: bool) -> u32 {
    let poly: u32 = if c { 0x82F6_3B78 } else { 0xEDB8_8320 };
    let mut crc = acc;
    for i in 0..bytes {
        crc ^= (val >> (8 * i)) as u32 & 0xFF;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (poly & (crc & 1).wrapping_neg());
        }
    }
    crc
}

/// `PACGA`'s code. The algorithm is IMPLEMENTATION DEFINED and keyed with a
/// key the program cannot read; this one is a fixed keyed mix, so equal
/// inputs give equal codes within a process, which is all programs rely on.
fn pacga_code(x: u64, modifier: u64) -> u32 {
    let mut h = x ^ 0x9E37_79B9_7F4A_7C15 ^ modifier.rotate_left(17);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (h ^ (h >> 31)) as u32
}
