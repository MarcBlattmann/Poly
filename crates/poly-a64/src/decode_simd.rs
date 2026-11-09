//! Decoding of the scalar floating-point and Advanced SIMD classes (the
//! `x111` group of the Arm ARM's top-level table) and of the Advanced SIMD
//! structure loads and stores.
//!
//! Only the documented subset is decoded; other allocated encodings give
//! [`Inst::Unsupported`], unallocated ones [`Inst::Undefined`].

use crate::decode::{bit, bits};
use crate::inst::*;

fn ftype(w: u32) -> Option<FpType> {
    match bits(w, 23, 22) {
        0 => Some(FpType::S),
        1 => Some(FpType::D),
        3 => Some(FpType::H),
        _ => None,
    }
}

/// Decode the group with bits 27:25 = 111.
pub(crate) fn decode_fp_simd(w: u32) -> Inst {
    match bits(w, 28, 24) {
        0b11110 => {
            if bit(w, 30) {
                decode_scalar_simd(w)
            } else {
                decode_fp(w)
            }
        }
        0b11111 => {
            if bits(w, 31, 29) == 0 {
                decode_fp3(w)
            } else if bit(w, 30) && !bit(w, 31) {
                // Scalar shift by immediate (bit 23 = 0) or scalar x indexed element.
                if !bit(w, 23) && bits(w, 22, 19) != 0 && bit(w, 10) {
                    decode_shift_imm(w, true)
                } else {
                    Inst::Unsupported(w)
                }
            } else {
                Inst::Undefined(w)
            }
        }
        0b01110 if !bit(w, 31) => decode_vector(w),
        0b01111 if !bit(w, 31) => {
            if bit(w, 23) {
                Inst::Unsupported(w) // vector x indexed element
            } else if bits(w, 22, 19) == 0 {
                decode_modimm(w)
            } else {
                decode_shift_imm(w, false)
            }
        }
        _ => Inst::Undefined(w),
    }
}

// ---------------------------------------------------------------------
// Scalar floating point
// ---------------------------------------------------------------------

/// Expand an 8-bit `FMOV` immediate to the bits of a single (`VFPExpandImm`).
fn vfp_expand_s(imm8: u32) -> u64 {
    let a = (imm8 >> 7) & 1;
    let b = (imm8 >> 6) & 1;
    let cd = (imm8 >> 4) & 3;
    let efgh = imm8 & 15;
    let exp = ((b ^ 1) << 7) | (if b == 1 { 0x1F } else { 0 } << 2) | cd;
    ((a << 31) | (exp << 23) | (efgh << 19)) as u64
}

/// The same for a double.
fn vfp_expand_d(imm8: u32) -> u64 {
    let a = ((imm8 >> 7) & 1) as u64;
    let b = ((imm8 >> 6) & 1) as u64;
    let cd = ((imm8 >> 4) & 3) as u64;
    let efgh = (imm8 & 15) as u64;
    let exp = ((b ^ 1) << 10) | (if b == 1 { 0xFF } else { 0 } << 2) | cd;
    (a << 63) | (exp << 52) | (efgh << 48)
}

fn decode_fp(w: u32) -> Inst {
    let sf = bit(w, 31);
    if bit(w, 29) {
        return Inst::Undefined(w);
    }
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let rm = bits(w, 20, 16) as Reg;
    if !bit(w, 21) {
        return decode_fp_fixed(w);
    }
    if bits(w, 15, 10) == 0 {
        return decode_fp_int(w);
    }
    // Everything else needs M = 0.
    if sf {
        return Inst::Undefined(w);
    }
    let Some(ft) = ftype(w) else { return Inst::Undefined(w) };
    if bits(w, 14, 10) == 0b10000 {
        // One source.
        let opcode = bits(w, 20, 15);
        let op = match opcode {
            0b000000 => Fp1Op::Mov,
            0b000001 => Fp1Op::Abs,
            0b000010 => Fp1Op::Neg,
            0b000011 => Fp1Op::Sqrt,
            0b001000 => Fp1Op::RintN,
            0b001001 => Fp1Op::RintP,
            0b001010 => Fp1Op::RintM,
            0b001011 => Fp1Op::RintZ,
            0b001100 => Fp1Op::RintA,
            0b001110 => Fp1Op::RintX,
            0b001111 => Fp1Op::RintI,
            0b000100 | 0b000101 | 0b000111 => {
                // FCVT: opcode<1:0> is the destination format.
                let to = match opcode & 3 {
                    0 => FpType::S,
                    1 => FpType::D,
                    _ => FpType::H,
                };
                if to == ft {
                    return Inst::Undefined(w);
                }
                return Inst::FpCvt { from: ft, to, rd, rn };
            }
            _ => return Inst::Unsupported(w),
        };
        return Inst::FpUnary { op, ft, rd, rn };
    }
    match bits(w, 11, 10) {
        0b00 => {
            if bit(w, 12) {
                // FMOV (scalar, immediate): imm5 (9:5) must be zero.
                if bits(w, 9, 5) != 0 {
                    return Inst::Undefined(w);
                }
                let imm8 = bits(w, 20, 13);
                let bits = match ft {
                    FpType::S => vfp_expand_s(imm8),
                    FpType::D => vfp_expand_d(imm8),
                    FpType::H => return Inst::Unsupported(w),
                };
                Inst::FpMovImm { ft, rd, bits }
            } else if bits(w, 13, 10) == 0b1000 && bits(w, 15, 14) == 0 {
                // Compare: opcode2 (4:0) must be x?000.
                let opc2 = bits(w, 4, 0);
                if opc2 & 7 != 0 {
                    return Inst::Undefined(w);
                }
                let zero = opc2 & 8 != 0;
                let signal = opc2 & 16 != 0;
                if zero && rm != 0 {
                    return Inst::Undefined(w);
                }
                if ft == FpType::H {
                    return Inst::Unsupported(w);
                }
                Inst::FpCmp { ft, signal, zero, rn, rm }
            } else {
                Inst::Undefined(w)
            }
        }
        0b01 => {
            if ft == FpType::H {
                return Inst::Unsupported(w);
            }
            Inst::FpCondCmp {
                ft,
                signal: bit(w, 4),
                rn,
                rm,
                cond: Cond::from_bits(bits(w, 15, 12)),
                nzcv: bits(w, 3, 0) as u8,
            }
        }
        0b10 => {
            if ft == FpType::H {
                return Inst::Unsupported(w);
            }
            let op = match bits(w, 15, 12) {
                0 => Fp2Op::Mul,
                1 => Fp2Op::Div,
                2 => Fp2Op::Add,
                3 => Fp2Op::Sub,
                4 => Fp2Op::Max,
                5 => Fp2Op::Min,
                6 => Fp2Op::MaxNm,
                7 => Fp2Op::MinNm,
                8 => Fp2Op::Nmul,
                _ => return Inst::Undefined(w),
            };
            Inst::FpBinary { op, ft, rd, rn, rm }
        }
        _ => {
            if ft == FpType::H {
                return Inst::Unsupported(w);
            }
            Inst::FpCondSel { ft, rd, rn, rm, cond: Cond::from_bits(bits(w, 15, 12)) }
        }
    }
}

fn decode_fp3(w: u32) -> Inst {
    let Some(ft) = ftype(w) else { return Inst::Undefined(w) };
    if ft == FpType::H {
        return Inst::Unsupported(w);
    }
    let o1 = bit(w, 21);
    let o0 = bit(w, 15);
    Inst::FpFused {
        ft,
        neg_p: o0 ^ o1,
        neg_a: o1,
        rd: bits(w, 4, 0) as Reg,
        rn: bits(w, 9, 5) as Reg,
        rm: bits(w, 20, 16) as Reg,
        ra: bits(w, 14, 10) as Reg,
    }
}

/// Conversions between floating point and fixed point.
fn decode_fp_fixed(w: u32) -> Inst {
    let sf = bit(w, 31);
    let wd = Width::from_sf(sf);
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let scale = bits(w, 15, 10);
    let Some(ft) = ftype(w) else { return Inst::Undefined(w) };
    if !sf && !bit(w, 15) {
        return Inst::Undefined(w);
    }
    if ft == FpType::H {
        return Inst::Unsupported(w);
    }
    let fbits = (64 - scale) as u8;
    match (bits(w, 20, 19), bits(w, 18, 16)) {
        (0b00, 0b010) => Inst::IntToFp { ft, w: wd, unsigned: false, rd, rn, fbits },
        (0b00, 0b011) => Inst::IntToFp { ft, w: wd, unsigned: true, rd, rn, fbits },
        (0b11, 0b000) => Inst::FpToInt { ft, w: wd, unsigned: false, round: FpRound::Zero, rd, rn, fbits },
        (0b11, 0b001) => Inst::FpToInt { ft, w: wd, unsigned: true, round: FpRound::Zero, rd, rn, fbits },
        _ => Inst::Undefined(w),
    }
}

/// Conversions and moves between floating point and integer registers.
fn decode_fp_int(w: u32) -> Inst {
    let sf = bit(w, 31);
    let wd = Width::from_sf(sf);
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let rmode = bits(w, 20, 19);
    let opcode = bits(w, 18, 16);
    let ft_bits = bits(w, 23, 22);
    let Some(ft) = ftype(w).or(if ft_bits == 2 { Some(FpType::D) } else { None }) else {
        return Inst::Undefined(w);
    };
    // FMOV to and from the top half of a 128-bit register.
    if ft_bits == 2 {
        return match (sf, rmode, opcode) {
            (true, 1, 6) => Inst::FmovToGen { w: Width::X, top: true, rd, rn },
            (true, 1, 7) => Inst::FmovFromGen { w: Width::X, top: true, rd, rn },
            _ => Inst::Undefined(w),
        };
    }
    if ft == FpType::H {
        return Inst::Unsupported(w);
    }
    // sf and ftype must agree for the moves.
    match (rmode, opcode) {
        (0, 6) | (0, 7) => {
            let ok = matches!((sf, ft), (false, FpType::S) | (true, FpType::D));
            if !ok {
                return Inst::Undefined(w);
            }
            if opcode == 6 {
                Inst::FmovToGen { w: wd, top: false, rd, rn }
            } else {
                Inst::FmovFromGen { w: wd, top: false, rd, rn }
            }
        }
        (0, 2) => Inst::IntToFp { ft, w: wd, unsigned: false, rd, rn, fbits: 0 },
        (0, 3) => Inst::IntToFp { ft, w: wd, unsigned: true, rd, rn, fbits: 0 },
        (0..=3, 0) | (0..=3, 1) | (0, 4) | (0, 5) => {
            let round = match (rmode, opcode) {
                (0, 0) | (0, 1) => FpRound::NearestEven,
                (1, _) => FpRound::PosInf,
                (2, _) => FpRound::NegInf,
                (3, _) => FpRound::Zero,
                _ => FpRound::Away, // (0, 4 | 5)
            };
            Inst::FpToInt { ft, w: wd, unsigned: opcode & 1 != 0, round, rd, rn, fbits: 0 }
        }
        (3, 6) => Inst::Unsupported(w), // FJCVTZS
        _ => Inst::Undefined(w),
    }
}

// ---------------------------------------------------------------------
// Advanced SIMD: vectors
// ---------------------------------------------------------------------

fn decode_vector(w: u32) -> Inst {
    let q = bit(w, 30);
    let u = bit(w, 29);
    let size = bits(w, 23, 22) as u8;
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let rm = bits(w, 20, 16) as Reg;
    if bit(w, 21) {
        if bit(w, 10) {
            return decode_three_same(w, q, u, size, false);
        }
        if !bit(w, 11) {
            return Inst::Unsupported(w); // three registers of different types
        }
        return match bits(w, 21, 17) {
            0b10000 => decode_two_misc(w, q, u, size, false),
            0b11000 => decode_across(w, q, u, size),
            _ => Inst::Unsupported(w),
        };
    }
    if bit(w, 10) {
        if bit(w, 15) {
            return Inst::Unsupported(w); // three same extra
        }
        if bits(w, 23, 22) != 0 {
            return Inst::Undefined(w);
        }
        return decode_copy(w, q);
    }
    if bit(w, 15) {
        return Inst::Undefined(w);
    }
    if u {
        // EXT: op2 (23:22) = 00, imm4 (14:11).
        if size != 0 {
            return Inst::Undefined(w);
        }
        let index = bits(w, 14, 11) as u8;
        if !q && index >= 8 {
            return Inst::Undefined(w);
        }
        return Inst::VecExt { q, rd, rn, rm, index };
    }
    if bit(w, 11) {
        // Permute.
        let op = match bits(w, 14, 12) {
            0b001 => PermOp::Uzp1,
            0b010 => PermOp::Trn1,
            0b011 => PermOp::Zip1,
            0b101 => PermOp::Uzp2,
            0b110 => PermOp::Trn2,
            0b111 => PermOp::Zip2,
            _ => return Inst::Undefined(w),
        };
        if size == 3 && !q {
            return Inst::Undefined(w);
        }
        return Inst::VecPerm { op, q, size, rd, rn, rm };
    }
    // Table lookup: len (14:13), op (12).
    if size != 0 {
        return Inst::Undefined(w);
    }
    Inst::VecTbl { q, len: bits(w, 14, 13) as u8 + 1, ext: bit(w, 12), rd, rn, rm }
}

fn decode_three_same(w: u32, q: bool, u: bool, size: u8, scalar: bool) -> Inst {
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let rm = bits(w, 20, 16) as Reg;
    let opcode = bits(w, 15, 11);
    let logic = |op| Inst::Vec3Same { op, q, size: 0, scalar, rd, rn, rm };
    let int = |op: V3Op, no64: bool| {
        // 64-bit elements need Q = 1 in the vector form and are the only size of the scalar form.
        let bad = if scalar { size != 3 } else { (size == 3 && (!q || no64)) || (no64 && size == 3) };
        if bad { Inst::Undefined(w) } else { Inst::Vec3Same { op, q, size, scalar, rd, rn, rm } }
    };
    if scalar {
        return match (u, opcode) {
            (false, 0b10000) => int(V3Op::Add, false),
            (true, 0b10000) => int(V3Op::Sub, false),
            (true, 0b10001) => int(V3Op::Cmeq, false),
            (false, 0b10001) => int(V3Op::Cmtst, false),
            (false, 0b00110) => int(V3Op::Cmgt, false),
            (false, 0b00111) => int(V3Op::Cmge, false),
            (true, 0b00110) => int(V3Op::Cmhi, false),
            (true, 0b00111) => int(V3Op::Cmhs, false),
            _ => Inst::Unsupported(w),
        };
    }
    match (u, opcode) {
        (false, 0b00011) => match size {
            0 => logic(V3Op::And),
            1 => logic(V3Op::Bic),
            2 => logic(V3Op::Orr),
            _ => logic(V3Op::Orn),
        },
        (true, 0b00011) => match size {
            0 => logic(V3Op::Eor),
            1 => logic(V3Op::Bsl),
            2 => logic(V3Op::Bit),
            _ => logic(V3Op::Bif),
        },
        (false, 0b10000) => int(V3Op::Add, false),
        (true, 0b10000) => int(V3Op::Sub, false),
        (true, 0b10001) => int(V3Op::Cmeq, false),
        (false, 0b10001) => int(V3Op::Cmtst, false),
        (false, 0b00110) => int(V3Op::Cmgt, false),
        (false, 0b00111) => int(V3Op::Cmge, false),
        (true, 0b00110) => int(V3Op::Cmhi, false),
        (true, 0b00111) => int(V3Op::Cmhs, false),
        (false, 0b10011) => int(V3Op::Mul, true),
        (false, 0b10010) => int(V3Op::Mla, true),
        (true, 0b10010) => int(V3Op::Mls, true),
        (false, 0b01100) => int(V3Op::Smax, true),
        (false, 0b01101) => int(V3Op::Smin, true),
        (true, 0b01100) => int(V3Op::Umax, true),
        (true, 0b01101) => int(V3Op::Umin, true),
        (false, 0b10111) => int(V3Op::Addp, false),
        (false, 0b01000) => int(V3Op::Sshl, false),
        (true, 0b01000) => int(V3Op::Ushl, false),
        _ => Inst::Unsupported(w),
    }
}

fn decode_two_misc(w: u32, q: bool, u: bool, size: u8, scalar: bool) -> Inst {
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let opcode = bits(w, 16, 12);
    let mk = |op, size| Inst::Vec2Misc { op, q, size, scalar, rd, rn };
    let bad64 = if scalar { size != 3 } else { size == 3 && !q };
    if scalar {
        return match (u, opcode) {
            (false, 0b01000) if !bad64 => mk(V2Op::CmgtZ, size),
            (false, 0b01001) if !bad64 => mk(V2Op::CmeqZ, size),
            (false, 0b01010) if !bad64 => mk(V2Op::CmltZ, size),
            (false, 0b01011) if !bad64 => mk(V2Op::Abs, size),
            (true, 0b01000) if !bad64 => mk(V2Op::CmgeZ, size),
            (true, 0b01001) if !bad64 => mk(V2Op::CmleZ, size),
            (true, 0b01011) if !bad64 => mk(V2Op::Neg, size),
            _ => Inst::Unsupported(w),
        };
    }
    match (u, opcode) {
        (false, 0b00000) if size < 3 => mk(V2Op::Rev64, size),
        (false, 0b00001) if size == 0 => mk(V2Op::Rev16, size),
        (true, 0b00000) if size < 2 => mk(V2Op::Rev32, size),
        (false, 0b00100) if size < 3 => mk(V2Op::Cls, size),
        (true, 0b00100) if size < 3 => mk(V2Op::Clz, size),
        (false, 0b00101) if size == 0 => mk(V2Op::Cnt, 0),
        (true, 0b00101) if size == 0 => mk(V2Op::Not, 0),
        (true, 0b00101) if size == 1 => mk(V2Op::Rbit, 0),
        (false, 0b01000) if !bad64 => mk(V2Op::CmgtZ, size),
        (false, 0b01001) if !bad64 => mk(V2Op::CmeqZ, size),
        (false, 0b01010) if !bad64 => mk(V2Op::CmltZ, size),
        (false, 0b01011) if !bad64 => mk(V2Op::Abs, size),
        (true, 0b01000) if !bad64 => mk(V2Op::CmgeZ, size),
        (true, 0b01001) if !bad64 => mk(V2Op::CmleZ, size),
        (true, 0b01011) if !bad64 => mk(V2Op::Neg, size),
        (false, 0b10010) if size < 3 => mk(if q { V2Op::Xtn2 } else { V2Op::Xtn }, size),
        (false, 0b00000) | (false, 0b00001) | (true, 0b00000) | (false, 0b00100) | (true, 0b00100) => {
            Inst::Undefined(w)
        }
        _ => Inst::Unsupported(w),
    }
}

fn decode_across(w: u32, q: bool, u: bool, size: u8) -> Inst {
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let op = match (u, bits(w, 16, 12)) {
        (false, 0b00011) => AcrossOp::Saddlv,
        (true, 0b00011) => AcrossOp::Uaddlv,
        (false, 0b01010) => AcrossOp::Smaxv,
        (true, 0b01010) => AcrossOp::Umaxv,
        (false, 0b11010) => AcrossOp::Sminv,
        (true, 0b11010) => AcrossOp::Uminv,
        (false, 0b11011) => AcrossOp::Addv,
        _ => return Inst::Unsupported(w),
    };
    if size == 3 || (size == 2 && !q) {
        return Inst::Undefined(w);
    }
    Inst::VecAcross { op, q, size, rd, rn }
}

fn decode_copy(w: u32, q: bool) -> Inst {
    let op = bit(w, 29);
    let imm5 = bits(w, 20, 16);
    let imm4 = bits(w, 14, 11);
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    if imm5 == 0 {
        return Inst::Undefined(w);
    }
    let size = imm5.trailing_zeros() as u8;
    if size > 3 {
        return Inst::Undefined(w);
    }
    let index = (imm5 >> (size + 1)) as u8;
    if op {
        // INS (element): Q must be 1.
        if !q {
            return Inst::Undefined(w);
        }
        let si = (imm4 >> size) as u8;
        return Inst::InsElem { size, rd, rn, di: index, si };
    }
    match imm4 {
        0b0000 => {
            if size == 3 && !q {
                return Inst::Undefined(w);
            }
            Inst::DupElem { q, size, scalar: false, rd, rn, index }
        }
        0b0001 => {
            if size == 3 && !q {
                return Inst::Undefined(w);
            }
            Inst::DupGen { q, size, rd, rn }
        }
        0b0011 => {
            if !q {
                return Inst::Undefined(w);
            }
            Inst::InsGen { size, rd, rn, index }
        }
        0b0101 => {
            // SMOV: B and H to W or X, S to X.
            if size == 3 || (size == 2 && !q) {
                return Inst::Undefined(w);
            }
            Inst::MovToGen { signed: true, w: Width::from_sf(q), size, rd, rn, index }
        }
        0b0111 => {
            // UMOV: B, H, S to W (Q = 0); D to X (Q = 1).
            if (size == 3) != q {
                return Inst::Undefined(w);
            }
            Inst::MovToGen { signed: false, w: Width::from_sf(q), size, rd, rn, index }
        }
        _ => Inst::Undefined(w),
    }
}

/// `AdvSIMDExpandImm`: the 64-bit pattern of a modified immediate.
fn adv_simd_expand_imm(op: bool, cmode: u32, imm8: u32) -> u64 {
    let i = imm8 as u64;
    let rep32 = |x: u64| x | (x << 32);
    let rep16 = |x: u64| rep32(x | (x << 16));
    match cmode >> 1 {
        0b000 => rep32(i),
        0b001 => rep32(i << 8),
        0b010 => rep32(i << 16),
        0b011 => rep32(i << 24),
        0b100 => rep16(i),
        0b101 => rep16(i << 8),
        0b110 => {
            if cmode & 1 == 0 {
                rep32((i << 8) | 0xFF)
            } else {
                rep32((i << 16) | 0xFFFF)
            }
        }
        _ => {
            if cmode & 1 == 0 {
                if !op {
                    i * 0x0101_0101_0101_0101
                } else {
                    let mut x = 0u64;
                    for b in 0..8 {
                        if (imm8 >> b) & 1 != 0 {
                            x |= 0xFFu64 << (8 * b);
                        }
                    }
                    x
                }
            } else if !op {
                rep32(vfp_expand_s(imm8))
            } else {
                vfp_expand_d(imm8)
            }
        }
    }
}

fn decode_modimm(w: u32) -> Inst {
    if !bit(w, 10) {
        return Inst::Undefined(w);
    }
    let q = bit(w, 30);
    let op = bit(w, 29);
    let cmode = bits(w, 15, 12);
    let rd = bits(w, 4, 0) as Reg;
    let imm8 = (bits(w, 18, 16) << 5) | bits(w, 9, 5);
    let imm = adv_simd_expand_imm(op, cmode, imm8);
    let c3 = cmode >> 1;
    if cmode & 1 == 1 && c3 < 0b110 {
        // ORR (op = 0) / BIC (op = 1) with a shifted immediate.
        return Inst::ModImmLogic { bic: op, q, rd, imm };
    }
    match (op, cmode) {
        (false, 0b1111) => Inst::Movi { q, rd, imm },
        (true, 0b1111) => {
            if !q {
                Inst::Undefined(w)
            } else {
                Inst::Movi { q, rd, imm }
            }
        }
        (true, 0b1110) => Inst::Movi { q, rd, imm },
        (false, _) => Inst::Movi { q, rd, imm },
        (true, _) => Inst::Movi { q, rd, imm: !imm }, // MVNI
    }
}

fn decode_shift_imm(w: u32, scalar: bool) -> Inst {
    let q = bit(w, 30);
    let u = bit(w, 29);
    let immh = bits(w, 22, 19);
    let ib = bits(w, 22, 16); // immh:immb
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    if immh == 0 {
        return Inst::Undefined(w);
    }
    let size = (31 - immh.leading_zeros()) as u8; // highest set bit of immh
    let esize = 8u32 << size;
    let right = (2 * esize - ib) as u8;
    let left = (ib - esize) as u8;
    let opcode = bits(w, 15, 11);
    let (op, shift, esz) = match (u, opcode) {
        (false, 0b00000) => (ShiftImmOp::Sshr, right, size),
        (true, 0b00000) => (ShiftImmOp::Ushr, right, size),
        (false, 0b00010) => (ShiftImmOp::Ssra, right, size),
        (true, 0b00010) => (ShiftImmOp::Usra, right, size),
        (true, 0b01000) => (ShiftImmOp::Sri, right, size),
        (false, 0b01010) => (ShiftImmOp::Shl, left, size),
        (true, 0b01010) => (ShiftImmOp::Sli, left, size),
        (false, 0b10000) if !scalar => (ShiftImmOp::Shrn, right, size),
        (false, 0b10100) if !scalar => (ShiftImmOp::Sshll, left, size),
        (true, 0b10100) if !scalar => (ShiftImmOp::Ushll, left, size),
        _ => return Inst::Unsupported(w),
    };
    if scalar {
        if size != 3 {
            return Inst::Undefined(w);
        }
    } else if size == 3 {
        // 64-bit elements need Q = 1, and the narrowing and widening forms have no 64-bit source.
        let narrow_wide = matches!(op, ShiftImmOp::Shrn | ShiftImmOp::Sshll | ShiftImmOp::Ushll);
        if !q || narrow_wide {
            return Inst::Undefined(w);
        }
    }
    Inst::VecShiftImm { op, q, size: esz, scalar, shift, rd, rn }
}

// ---------------------------------------------------------------------
// Advanced SIMD: scalar forms
// ---------------------------------------------------------------------

fn decode_scalar_simd(w: u32) -> Inst {
    let u = bit(w, 29);
    let size = bits(w, 23, 22) as u8;
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    if bit(w, 21) {
        if bit(w, 10) {
            return decode_three_same(w, true, u, size, true);
        }
        if bit(w, 11) && bits(w, 21, 17) == 0b10000 {
            return decode_two_misc(w, true, u, size, true);
        }
        return Inst::Unsupported(w);
    }
    // Copy: DUP (element) scalar; bits 28:21 = 11110000.
    if bits(w, 28, 21) == 0b1111_0000 && bit(w, 10) && !bit(w, 15) && !u && bits(w, 14, 11) == 0 {
        let imm5 = bits(w, 20, 16);
        if imm5 == 0 || imm5.trailing_zeros() > 3 {
            return Inst::Undefined(w);
        }
        let size = imm5.trailing_zeros() as u8;
        return Inst::DupElem { q: false, size, scalar: true, rd, rn, index: (imm5 >> (size + 1)) as u8 };
    }
    Inst::Unsupported(w)
}

// ---------------------------------------------------------------------
// Advanced SIMD structure loads and stores
// ---------------------------------------------------------------------

pub(crate) fn decode_simd_ldst_struct(w: u32) -> Inst {
    // bit 31 = 0, bit 30 = Q, bits 29:27 = 001, bit 26 = 1.
    if bit(w, 31) {
        return Inst::Undefined(w);
    }
    let q = bit(w, 30);
    let load = bit(w, 22);
    let rn = bits(w, 9, 5) as Reg;
    let rt = bits(w, 4, 0) as Reg;
    let rm = bits(w, 20, 16) as Reg;
    let post_bit = bit(w, 23);
    match bits(w, 24, 24) {
        0 => {
            // Multiple structures.
            if !post_bit && (bit(w, 21) || bits(w, 20, 16) != 0) {
                return Inst::Undefined(w);
            }
            if post_bit && bit(w, 21) {
                return Inst::Undefined(w);
            }
            let size = bits(w, 11, 10) as u8;
            let (regs, interleave) = match bits(w, 15, 12) {
                0b0000 => (4, true),
                0b0010 => (4, false),
                0b0100 => (3, true),
                0b0110 => (3, false),
                0b0111 => (1, false),
                0b1000 => (2, true),
                0b1010 => (2, false),
                _ => return Inst::Undefined(w),
            };
            if interleave && size == 3 && !q {
                return Inst::Undefined(w);
            }
            let bytes = regs as u32 * if q { 16 } else { 8 };
            let post = if !post_bit {
                PostIdx::None
            } else if rm == 31 {
                PostIdx::Imm(bytes as u8)
            } else {
                PostIdx::Reg(rm)
            };
            Inst::VecLdstMulti { load, q, size, regs, interleave, rt, rn, post }
        }
        _ => {
            // Single structure.
            if !post_bit && bits(w, 20, 16) != 0 {
                return Inst::Undefined(w);
            }
            let r = bit(w, 21);
            let opcode = bits(w, 15, 13);
            let s = bit(w, 12);
            let size_f = bits(w, 11, 10);
            let selem = (((opcode & 1) << 1) | r as u32) as u8 + 1;
            let scale = opcode >> 1;
            let (esize, index, replicate) = match scale {
                0 => (0u8, (u32::from(q) << 3) | (u32::from(s) << 2) | size_f, false),
                1 => {
                    if size_f & 1 != 0 {
                        return Inst::Undefined(w);
                    }
                    (1, (u32::from(q) << 2) | (u32::from(s) << 1) | (size_f >> 1), false)
                }
                2 => match (size_f, s) {
                    (0, _) => (2, (u32::from(q) << 1) | u32::from(s), false),
                    (1, false) => (3, u32::from(q), false),
                    _ => return Inst::Undefined(w),
                },
                _ => {
                    if !load || s {
                        return Inst::Undefined(w);
                    }
                    (size_f as u8, 0, true)
                }
            };
            let bytes = selem as u32 * (1 << esize);
            let post = if !post_bit {
                PostIdx::None
            } else if rm == 31 {
                PostIdx::Imm(bytes as u8)
            } else {
                PostIdx::Reg(rm)
            };
            // Replicate forms transfer one element per structure to all lanes: bytes as above.
            Inst::VecLdstSingle { load, replicate, q, size: esize, selem, index: index as u8, rt, rn, post }
        }
    }
}
