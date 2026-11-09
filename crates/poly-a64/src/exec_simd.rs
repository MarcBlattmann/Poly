//! Advanced SIMD instructions on the 128-bit register file.
//!
//! A vector register is a `u128` whose lane `i` of `bits` bits sits at bit
//! `i * bits` (little-endian element order, as in memory). A write of a
//! 64-bit vector (`Q` = 0) or a scalar zeroes the rest of the register.

use crate::cpu::{Cpu, Memory, Trap};
use crate::exec_mem::sign_extend;
use crate::inst::*;

const fn mask_bits(bits: u32) -> u64 {
    if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 }
}

fn lane_get(v: u128, bits: u32, i: u32) -> u64 {
    ((v >> (i * bits)) as u64) & mask_bits(bits)
}

fn lane_set(v: &mut u128, bits: u32, i: u32, x: u64) {
    let m = u128::from(mask_bits(bits)) << (i * bits);
    *v = (*v & !m) | ((u128::from(x & mask_bits(bits))) << (i * bits));
}

/// Sign-extend a `bits`-wide lane value.
fn sx(x: u64, bits: u32) -> i64 {
    let sh = 64 - bits;
    ((x << sh) as i64) >> sh
}

const LOW64: u128 = u64::MAX as u128;

/// Shift left, giving 0 for shifts of the width or more.
fn shl_lane(a: u64, sh: u32, bits: u32) -> u64 {
    if sh >= bits { 0 } else { (a << sh) & mask_bits(bits) }
}

fn lshr_lane(a: u64, sh: u32) -> u64 {
    if sh >= 64 { 0 } else { a >> sh }
}

fn ashr_lane(a: u64, sh: u32, bits: u32) -> u64 {
    ((sx(a, bits) >> sh.min(63)) as u64) & mask_bits(bits)
}

impl Cpu {
    /// Number of lanes of `bits` in the vector (or the one lane of a scalar).
    fn lanes(q: bool, scalar: bool, bits: u32) -> u32 {
        if scalar { 1 } else { (if q { 128 } else { 64 }) / bits }
    }

    fn write_vec(&mut self, rd: Reg, value: u128, q: bool) {
        self.v[rd as usize] = if q { value } else { value & LOW64 };
    }

    pub(crate) fn exec_simd(&mut self, inst: Inst) -> Result<(), Trap> {
        match inst {
            Inst::Vec3Same { op, q, size, scalar, rd, rn, rm } => {
                self.exec_3same(op, q, size, scalar, rd, rn, rm);
                Ok(())
            }
            Inst::Vec2Misc { op, q, size, scalar, rd, rn } => {
                self.exec_2misc(op, q, size, scalar, rd, rn);
                Ok(())
            }
            Inst::VecAcross { op, q, size, rd, rn } => {
                let bits = 8u32 << size;
                let n = self.v[rn as usize];
                let lanes = Self::lanes(q, false, bits);
                let lane = |i| lane_get(n, bits, i);
                let r = match op {
                    AcrossOp::Addv => (0..lanes).fold(0u64, |s, i| s.wrapping_add(lane(i))) & mask_bits(bits),
                    AcrossOp::Umaxv => (0..lanes).map(lane).max().unwrap_or(0),
                    AcrossOp::Uminv => (0..lanes).map(lane).min().unwrap_or(0),
                    AcrossOp::Smaxv => {
                        (0..lanes).map(|i| sx(lane(i), bits)).max().unwrap_or(0) as u64 & mask_bits(bits)
                    }
                    AcrossOp::Sminv => {
                        (0..lanes).map(|i| sx(lane(i), bits)).min().unwrap_or(0) as u64 & mask_bits(bits)
                    }
                    AcrossOp::Uaddlv => (0..lanes).fold(0u64, |s, i| s.wrapping_add(lane(i))) & mask_bits(bits * 2),
                    AcrossOp::Saddlv => {
                        (0..lanes).fold(0u64, |s, i| s.wrapping_add(sx(lane(i), bits) as u64)) & mask_bits(bits * 2)
                    }
                };
                self.v[rd as usize] = u128::from(r);
                Ok(())
            }
            Inst::DupElem { q, size, scalar, rd, rn, index } => {
                let bits = 8u32 << size;
                let x = lane_get(self.v[rn as usize], bits, u32::from(index));
                self.v[rd as usize] = Self::replicate(x, bits, q, scalar);
                Ok(())
            }
            Inst::DupGen { q, size, rd, rn } => {
                let bits = 8u32 << size;
                let x = self.xr(rn) & mask_bits(bits);
                self.v[rd as usize] = Self::replicate(x, bits, q, false);
                Ok(())
            }
            Inst::InsElem { size, rd, rn, di, si } => {
                let bits = 8u32 << size;
                let x = lane_get(self.v[rn as usize], bits, u32::from(si));
                lane_set(&mut self.v[rd as usize], bits, u32::from(di), x);
                Ok(())
            }
            Inst::InsGen { size, rd, rn, index } => {
                let bits = 8u32 << size;
                let x = self.xr(rn);
                lane_set(&mut self.v[rd as usize], bits, u32::from(index), x);
                Ok(())
            }
            Inst::MovToGen { signed, w, size, rd, rn, index } => {
                let bits = 8u32 << size;
                let x = lane_get(self.v[rn as usize], bits, u32::from(index));
                let x = if signed { sign_extend(x, 1 << size) } else { x };
                self.set_rw(rd, w, x);
                Ok(())
            }
            Inst::Movi { q, rd, imm } => {
                let v = u128::from(imm) | (u128::from(imm) << 64);
                self.write_vec(rd, v, q);
                Ok(())
            }
            Inst::ModImmLogic { bic, q, rd, imm } => {
                let pat = u128::from(imm) | (u128::from(imm) << 64);
                let d = self.v[rd as usize];
                let r = if bic { d & !pat } else { d | pat };
                self.write_vec(rd, r, q);
                Ok(())
            }
            Inst::VecExt { q, rd, rn, rm, index } => {
                let r = if q {
                    let (n, m) = (self.v[rn as usize], self.v[rm as usize]);
                    if index == 0 { n } else { (n >> (8 * u32::from(index))) | (m << (128 - 8 * u32::from(index))) }
                } else {
                    let (n, m) = (self.v[rn as usize] & LOW64, self.v[rm as usize] & LOW64);
                    let cat = n | (m << 64);
                    (cat >> (8 * u32::from(index))) & LOW64
                };
                self.v[rd as usize] = r;
                Ok(())
            }
            Inst::VecTbl { q, len, ext, rd, rn, rm } => {
                let idxs = self.v[rm as usize];
                let old = self.v[rd as usize];
                let n = if q { 16 } else { 8 };
                let mut out = 0u128;
                for i in 0..n {
                    let idx = lane_get(idxs, 8, i) as u32;
                    let byte = if idx < 16 * u32::from(len) {
                        let reg = (rn as usize + (idx / 16) as usize) % 32;
                        lane_get(self.v[reg], 8, idx % 16)
                    } else if ext {
                        lane_get(old, 8, i)
                    } else {
                        0
                    };
                    lane_set(&mut out, 8, i, byte);
                }
                self.v[rd as usize] = out;
                Ok(())
            }
            Inst::VecPerm { op, q, size, rd, rn, rm } => {
                let bits = 8u32 << size;
                let l = Self::lanes(q, false, bits);
                let (n, m) = (self.v[rn as usize], self.v[rm as usize]);
                let mut r = 0u128;
                let half = l / 2;
                for i in 0..half {
                    let odd = u32::from(op == PermOp::Uzp2);
                    match op {
                        PermOp::Zip1 | PermOp::Zip2 => {
                            let base = if op == PermOp::Zip2 { half } else { 0 };
                            lane_set(&mut r, bits, 2 * i, lane_get(n, bits, base + i));
                            lane_set(&mut r, bits, 2 * i + 1, lane_get(m, bits, base + i));
                        }
                        PermOp::Trn1 | PermOp::Trn2 => {
                            let o = u32::from(op == PermOp::Trn2);
                            lane_set(&mut r, bits, 2 * i, lane_get(n, bits, 2 * i + o));
                            lane_set(&mut r, bits, 2 * i + 1, lane_get(m, bits, 2 * i + o));
                        }
                        PermOp::Uzp1 | PermOp::Uzp2 => {
                            lane_set(&mut r, bits, i, lane_get(n, bits, 2 * i + odd));
                            lane_set(&mut r, bits, half + i, lane_get(m, bits, 2 * i + odd));
                        }
                    }
                }
                self.write_vec(rd, r, q);
                Ok(())
            }
            Inst::VecShiftImm { op, q, size, scalar, shift, rd, rn } => {
                self.exec_shift_imm(op, q, size, scalar, u32::from(shift), rd, rn);
                Ok(())
            }
            _ => Err(Trap::Unsupported),
        }
    }

    fn replicate(x: u64, bits: u32, q: bool, scalar: bool) -> u128 {
        if scalar {
            return u128::from(x);
        }
        let lanes = (if q { 128 } else { 64 }) / bits;
        let mut v = 0u128;
        for i in 0..lanes {
            lane_set(&mut v, bits, i, x);
        }
        v
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_3same(&mut self, op: V3Op, q: bool, size: u8, scalar: bool, rd: Reg, rn: Reg, rm: Reg) {
        let n = self.v[rn as usize];
        let m = self.v[rm as usize];
        let d = self.v[rd as usize];
        match op {
            V3Op::And => return self.write_vec(rd, n & m, q),
            V3Op::Bic => return self.write_vec(rd, n & !m, q),
            V3Op::Orr => return self.write_vec(rd, n | m, q),
            V3Op::Orn => return self.write_vec(rd, n | !m, q),
            V3Op::Eor => return self.write_vec(rd, n ^ m, q),
            V3Op::Bsl => return self.write_vec(rd, (d & n) | (!d & m), q),
            V3Op::Bit => return self.write_vec(rd, (d & !m) | (n & m), q),
            V3Op::Bif => return self.write_vec(rd, (d & m) | (n & !m), q),
            _ => {}
        }
        let bits = 8u32 << size;
        let lanes = Self::lanes(q, scalar, bits);
        let mk = mask_bits(bits);
        let mut r = 0u128;
        if op == V3Op::Addp {
            // Pairwise: first half of the result from Vn's pairs, second half from Vm's.
            let half = lanes / 2;
            for i in 0..half {
                let a = lane_get(n, bits, 2 * i).wrapping_add(lane_get(n, bits, 2 * i + 1));
                let b = lane_get(m, bits, 2 * i).wrapping_add(lane_get(m, bits, 2 * i + 1));
                lane_set(&mut r, bits, i, a);
                lane_set(&mut r, bits, half + i, b);
            }
            return self.write_vec(rd, r, q);
        }
        for i in 0..lanes {
            let a = lane_get(n, bits, i);
            let b = lane_get(m, bits, i);
            let c = lane_get(d, bits, i);
            let (sa, sb) = (sx(a, bits), sx(b, bits));
            let t = |c: bool| if c { mk } else { 0 };
            let x = match op {
                V3Op::Add => a.wrapping_add(b),
                V3Op::Sub => a.wrapping_sub(b),
                V3Op::Cmeq => t(a == b),
                V3Op::Cmhi => t(a > b),
                V3Op::Cmhs => t(a >= b),
                V3Op::Cmgt => t(sa > sb),
                V3Op::Cmge => t(sa >= sb),
                V3Op::Cmtst => t(a & b != 0),
                V3Op::Mul => a.wrapping_mul(b),
                V3Op::Mla => c.wrapping_add(a.wrapping_mul(b)),
                V3Op::Mls => c.wrapping_sub(a.wrapping_mul(b)),
                V3Op::Smax => sa.max(sb) as u64,
                V3Op::Smin => sa.min(sb) as u64,
                V3Op::Umax => a.max(b),
                V3Op::Umin => a.min(b),
                V3Op::Sshl => {
                    let s = b as u8 as i8 as i32;
                    if s >= 0 { shl_lane(a, s as u32, bits) } else { ashr_lane(a, (-s) as u32, bits) }
                }
                V3Op::Ushl => {
                    let s = b as u8 as i8 as i32;
                    if s >= 0 { shl_lane(a, s as u32, bits) } else { lshr_lane(a, (-s) as u32) }
                }
                _ => 0,
            };
            lane_set(&mut r, bits, i, x);
        }
        self.write_vec(rd, r, q);
    }

    fn exec_2misc(&mut self, op: V2Op, q: bool, size: u8, scalar: bool, rd: Reg, rn: Reg) {
        let n = self.v[rn as usize];
        let bits = 8u32 << size;
        match op {
            V2Op::Not => return self.write_vec(rd, !n, q),
            V2Op::Xtn | V2Op::Xtn2 => {
                let lanes = 64 / bits;
                let mut r = 0u128;
                for i in 0..lanes {
                    let a = lane_get(n, bits * 2, i);
                    lane_set(&mut r, bits, i, a);
                }
                if op == V2Op::Xtn {
                    self.v[rd as usize] = r;
                } else {
                    self.v[rd as usize] = (self.v[rd as usize] & LOW64) | (r << 64);
                }
                return;
            }
            V2Op::Rev64 | V2Op::Rev32 | V2Op::Rev16 => {
                let container = match op {
                    V2Op::Rev64 => 64,
                    V2Op::Rev32 => 32,
                    _ => 16,
                };
                let k = container / bits;
                let lanes = Self::lanes(q, false, bits);
                let mut r = 0u128;
                for i in 0..lanes {
                    let dst = (i / k) * k + (k - 1 - i % k);
                    lane_set(&mut r, bits, dst, lane_get(n, bits, i));
                }
                return self.write_vec(rd, r, q);
            }
            _ => {}
        }
        let lanes = Self::lanes(q, scalar, bits);
        let mut r = 0u128;
        for i in 0..lanes {
            let a = lane_get(n, bits, i);
            let sa = sx(a, bits);
            let mk = mask_bits(bits);
            let t = |c: bool| if c { mk } else { 0 };
            let x = match op {
                V2Op::CmeqZ => t(sa == 0),
                V2Op::CmgtZ => t(sa > 0),
                V2Op::CmgeZ => t(sa >= 0),
                V2Op::CmleZ => t(sa <= 0),
                V2Op::CmltZ => t(sa < 0),
                V2Op::Neg => a.wrapping_neg(),
                V2Op::Abs => sa.wrapping_abs() as u64,
                V2Op::Cnt => u64::from(a.count_ones()),
                V2Op::Rbit => u64::from((a as u8).reverse_bits()),
                V2Op::Clz => u64::from((a << (64 - bits)).leading_zeros().min(bits)),
                V2Op::Cls => {
                    let y = if sa < 0 { !a & mk } else { a };
                    u64::from((y << (64 - bits)).leading_zeros().min(bits) - 1)
                }
                _ => 0,
            };
            lane_set(&mut r, bits, i, x);
        }
        self.write_vec(rd, r, q);
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_shift_imm(&mut self, op: ShiftImmOp, q: bool, size: u8, scalar: bool, shift: u32, rd: Reg, rn: Reg) {
        let n = self.v[rn as usize];
        let d = self.v[rd as usize];
        let bits = 8u32 << size;
        match op {
            ShiftImmOp::Shrn => {
                let lanes = 64 / bits;
                let mut r = 0u128;
                for i in 0..lanes {
                    let a = lane_get(n, bits * 2, i);
                    lane_set(&mut r, bits, i, lshr_lane(a, shift));
                }
                self.v[rd as usize] = if q { (d & LOW64) | (r << 64) } else { r };
                return;
            }
            ShiftImmOp::Sshll | ShiftImmOp::Ushll => {
                let lanes = 64 / bits;
                let src = if q { n >> 64 } else { n };
                let mut r = 0u128;
                for i in 0..lanes {
                    let a = lane_get(src, bits, i);
                    let w = if op == ShiftImmOp::Sshll { sx(a, bits) as u64 } else { a };
                    lane_set(&mut r, bits * 2, i, w << shift);
                }
                self.v[rd as usize] = r;
                return;
            }
            _ => {}
        }
        let lanes = Self::lanes(q, scalar, bits);
        let mk = mask_bits(bits);
        let mut r = 0u128;
        for i in 0..lanes {
            let a = lane_get(n, bits, i);
            let c = lane_get(d, bits, i);
            let x = match op {
                ShiftImmOp::Shl => shl_lane(a, shift, bits),
                ShiftImmOp::Ushr => lshr_lane(a, shift),
                ShiftImmOp::Sshr => ashr_lane(a, shift, bits),
                ShiftImmOp::Usra => c.wrapping_add(lshr_lane(a, shift)),
                ShiftImmOp::Ssra => c.wrapping_add(ashr_lane(a, shift, bits)),
                ShiftImmOp::Sli => {
                    let m = shl_lane(mk, shift, bits);
                    (c & !m) | (shl_lane(a, shift, bits) & m)
                }
                ShiftImmOp::Sri => {
                    let m = lshr_lane(mk, shift);
                    (c & !m) | (lshr_lane(a, shift) & m)
                }
                _ => 0,
            };
            lane_set(&mut r, bits, i, x & mk);
        }
        self.write_vec(rd, r, q);
    }

    /// `LD1`..`LD4`, `ST1`..`ST4` and `LD1R`..`LD4R`.
    pub(crate) fn exec_simd_mem<M: Memory + ?Sized>(&mut self, inst: Inst, mem: &mut M) -> Result<(), Trap> {
        match inst {
            Inst::VecLdstMulti { load, q, size, regs, interleave, rt, rn, post } => {
                let base = self.xsp(rn);
                let dsize: u64 = if q { 16 } else { 8 };
                let esz = 1u64 << size;
                let elems = (dsize / esz) as u32;
                let mut tmp = [0u128; 4];
                if !interleave {
                    for r in 0..u64::from(regs) {
                        let a = base.wrapping_add(r * dsize);
                        if load {
                            tmp[r as usize] = mem.read_uint(a, dsize as usize)?;
                        } else {
                            let reg = (rt as usize + r as usize) % 32;
                            mem.write_uint(a, dsize as usize, self.v[reg])?;
                        }
                    }
                } else {
                    for e in 0..elems {
                        for s in 0..u32::from(regs) {
                            let a = base.wrapping_add((u64::from(e) * u64::from(regs) + u64::from(s)) * esz);
                            if load {
                                let x = mem.read_uint(a, esz as usize)? as u64;
                                lane_set(&mut tmp[s as usize], 8 << size, e, x);
                            } else {
                                let reg = (rt as usize + s as usize) % 32;
                                let x = lane_get(self.v[reg], 8 << size, e);
                                mem.write_uint(a, esz as usize, u128::from(x))?;
                            }
                        }
                    }
                }
                if load {
                    for (r, t) in tmp.iter().enumerate().take(regs as usize) {
                        self.v[(rt as usize + r) % 32] = *t;
                    }
                }
                self.post_index(rn, post, base);
                Ok(())
            }
            Inst::VecLdstSingle { load, replicate, q, size, selem, index, rt, rn, post } => {
                let base = self.xsp(rn);
                let esz = 1u64 << size;
                let bits = 8u32 << size;
                let mut tmp = [0u128; 4];
                for (s, t) in tmp.iter_mut().enumerate().take(selem as usize) {
                    let a = base.wrapping_add(s as u64 * esz);
                    let reg = (rt as usize + s) % 32;
                    if load {
                        let x = mem.read_uint(a, esz as usize)? as u64;
                        *t = if replicate {
                            Self::replicate(x, bits, q, false)
                        } else {
                            let mut v = self.v[reg];
                            lane_set(&mut v, bits, u32::from(index), x);
                            v
                        };
                    } else {
                        let x = lane_get(self.v[reg], bits, u32::from(index));
                        mem.write_uint(a, esz as usize, u128::from(x))?;
                    }
                }
                if load {
                    for (s, t) in tmp.iter().enumerate().take(selem as usize) {
                        self.v[(rt as usize + s) % 32] = *t;
                    }
                }
                self.post_index(rn, post, base);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn post_index(&mut self, rn: Reg, post: PostIdx, base: u64) {
        match post {
            PostIdx::None => {}
            PostIdx::Imm(n) => self.set_xsp(rn, base.wrapping_add(u64::from(n))),
            PostIdx::Reg(rm) => {
                let off = self.xr(rm);
                self.set_xsp(rn, base.wrapping_add(off));
            }
        }
    }
}
