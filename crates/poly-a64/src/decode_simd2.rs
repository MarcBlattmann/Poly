//! Decoding of the Advanced SIMD classes beyond the first subset: vector
//! floating point, three registers of different types, by-element forms,
//! saturating and rounding integer forms, more shifts by immediate,
//! pairwise and across-lanes floating point, dot products.
//!
//! Field names follow the Arm ARM's encoding tables (C4.1.95 and around).
//! What is still not decoded gives [`Inst::Unsupported`].

use crate::decode::{bit, bits};
use crate::inst::*;

fn regs(w: u32) -> (Reg, Reg, Reg) {
    (bits(w, 4, 0) as Reg, bits(w, 9, 5) as Reg, bits(w, 20, 16) as Reg)
}

/// S for `sz` 0, D for 1.
fn fsz(w: u32) -> FpType {
    if bit(w, 22) { FpType::D } else { FpType::S }
}

/// Three same, opcodes 11xxx: floating point (single and double).
pub(crate) fn fp_three_same(w: u32, q: bool, u: bool, scalar: bool) -> Inst {
    let (rd, rn, rm) = regs(w);
    let a = bit(w, 23);
    let ft = fsz(w);
    let op = match (u, a, bits(w, 15, 11)) {
        (false, false, 0b11000) => VFp3Op::MaxNm,
        (false, false, 0b11001) => VFp3Op::Mla,
        (false, false, 0b11010) => VFp3Op::Add,
        (false, false, 0b11011) => VFp3Op::Mulx,
        (false, false, 0b11100) => VFp3Op::Cmeq,
        (false, false, 0b11110) => VFp3Op::Max,
        (false, false, 0b11111) => VFp3Op::Recps,
        (false, true, 0b11000) => VFp3Op::MinNm,
        (false, true, 0b11001) => VFp3Op::Mls,
        (false, true, 0b11010) => VFp3Op::Sub,
        (false, true, 0b11110) => VFp3Op::Min,
        (false, true, 0b11111) => VFp3Op::Rsqrts,
        (true, false, 0b11000) => VFp3Op::MaxNmp,
        (true, false, 0b11010) => VFp3Op::Addp,
        (true, false, 0b11011) => VFp3Op::Mul,
        (true, false, 0b11100) => VFp3Op::Cmge,
        (true, false, 0b11101) => VFp3Op::Acge,
        (true, false, 0b11110) => VFp3Op::Maxp,
        (true, false, 0b11111) => VFp3Op::Div,
        (true, true, 0b11000) => VFp3Op::MinNmp,
        (true, true, 0b11010) => VFp3Op::Abd,
        (true, true, 0b11100) => VFp3Op::Cmgt,
        (true, true, 0b11101) => VFp3Op::Acgt,
        (true, true, 0b11110) => VFp3Op::Minp,
        _ => return Inst::Unsupported(w), // FMLAL/FMLSL (FEAT_FHM) and unallocated
    };
    if scalar {
        // Scalar forms: FMULX, FCMEQ, FCMGE, FCMGT, FACGE, FACGT, FRECPS, FRSQRTS, FABD.
        if !matches!(
            op,
            VFp3Op::Mulx
                | VFp3Op::Cmeq
                | VFp3Op::Cmge
                | VFp3Op::Cmgt
                | VFp3Op::Acge
                | VFp3Op::Acgt
                | VFp3Op::Recps
                | VFp3Op::Rsqrts
                | VFp3Op::Abd
        ) {
            return Inst::Undefined(w);
        }
    } else if ft == FpType::D && !q {
        return Inst::Undefined(w);
    }
    Inst::VecFp3 { op, q, ft, scalar, rd, rn, rm }
}

/// Three same, integer opcodes the first subset leaves out.
pub(crate) fn int_three_same(w: u32, q: bool, u: bool, size: u8, scalar: bool) -> Inst {
    let (rd, rn, rm) = regs(w);
    let opcode = bits(w, 15, 11);
    let op = match (u, opcode) {
        (false, 0b00000) => V3Op::Shadd,
        (true, 0b00000) => V3Op::Uhadd,
        (false, 0b00001) => V3Op::Sqadd,
        (true, 0b00001) => V3Op::Uqadd,
        (false, 0b00010) => V3Op::Srhadd,
        (true, 0b00010) => V3Op::Urhadd,
        (false, 0b00100) => V3Op::Shsub,
        (true, 0b00100) => V3Op::Uhsub,
        (false, 0b00101) => V3Op::Sqsub,
        (true, 0b00101) => V3Op::Uqsub,
        (false, 0b01001) => V3Op::Sqshl,
        (true, 0b01001) => V3Op::Uqshl,
        (false, 0b01010) => V3Op::Srshl,
        (true, 0b01010) => V3Op::Urshl,
        (false, 0b01011) => V3Op::Sqrshl,
        (true, 0b01011) => V3Op::Uqrshl,
        (false, 0b01110) => V3Op::Sabd,
        (true, 0b01110) => V3Op::Uabd,
        (false, 0b01111) => V3Op::Saba,
        (true, 0b01111) => V3Op::Uaba,
        (false, 0b10100) => V3Op::Smaxp,
        (false, 0b10101) => V3Op::Sminp,
        (true, 0b10100) => V3Op::Umaxp,
        (true, 0b10101) => V3Op::Uminp,
        (false, 0b10110) => V3Op::Sqdmulh,
        (true, 0b10110) => V3Op::Sqrdmulh,
        (true, 0b10011) => V3Op::Pmul,
        _ => return Inst::Unsupported(w),
    };
    let sat = matches!(
        op,
        V3Op::Sqadd | V3Op::Uqadd | V3Op::Sqsub | V3Op::Uqsub | V3Op::Sqshl | V3Op::Uqshl | V3Op::Sqrshl | V3Op::Uqrshl
    );
    let shifts = matches!(op, V3Op::Srshl | V3Op::Urshl);
    let dmulh = matches!(op, V3Op::Sqdmulh | V3Op::Sqrdmulh);
    if scalar {
        let ok = sat || (shifts && size == 3) || (dmulh && (size == 1 || size == 2));
        if !ok {
            return Inst::Undefined(w);
        }
    } else {
        let no64 = !(sat || shifts);
        if size == 3 && (!q || no64) {
            return Inst::Undefined(w);
        }
        if dmulh && (size == 0 || size == 3) {
            return Inst::Undefined(w);
        }
        if op == V3Op::Pmul && size != 0 {
            return Inst::Undefined(w);
        }
    }
    Inst::Vec3Same { op, q, size, scalar, rd, rn, rm }
}

/// Three same extra (bit 21 = 0, bit 15 = 1, bit 10 = 1): dot products and
/// rounding doubling multiply-accumulate.
pub(crate) fn three_same_extra(w: u32, q: bool, u: bool, size: u8, scalar: bool) -> Inst {
    let (rd, rn, rm) = regs(w);
    match (u, bits(w, 14, 11)) {
        (_, 0b0010) if !scalar && size == 2 => {
            Inst::VecElem { op: ElemOp::Dot(u), q, size: 2, scalar: false, rd, rn, rm, index: 0xFF }
        }
        (true, 0b0000 | 0b0001) => {
            if size == 0 || size == 3 || (!scalar && false) {
                return Inst::Undefined(w);
            }
            let op = if bits(w, 14, 11) == 0 { V3Op::Sqrdmlah } else { V3Op::Sqrdmlsh };
            Inst::Vec3Same { op, q, size, scalar, rd, rn, rm }
        }
        _ => Inst::Unsupported(w),
    }
}

/// Two-register miscellaneous, the forms the first subset leaves out.
pub(crate) fn two_misc(w: u32, q: bool, u: bool, size: u8, scalar: bool) -> Inst {
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let opcode = bits(w, 16, 12);
    let a = bit(w, 23);
    let ft = fsz(w);
    let fp = |op: VFp2Op| {
        if !scalar && ft == FpType::D && !q { Inst::Undefined(w) } else { Inst::VecFp2 { op, q, ft, scalar, rd, rn } }
    };
    let misc = |op: Misc2Op| Inst::VecMisc2 { op, q, size, scalar, rd, rn };
    match (u, opcode) {
        // FP opcodes 0b011xx (a = 1) and 0b1100x..0b11111.
        (false, 0b01100) if a => fp(VFp2Op::CmZero(CmpZero::Gt)),
        (false, 0b01101) if a => fp(VFp2Op::CmZero(CmpZero::Eq)),
        (false, 0b01110) if a => fp(VFp2Op::CmZero(CmpZero::Lt)),
        (true, 0b01100) if a => fp(VFp2Op::CmZero(CmpZero::Ge)),
        (true, 0b01101) if a => fp(VFp2Op::CmZero(CmpZero::Le)),
        (false, 0b01111) if a && !scalar => fp(VFp2Op::Abs),
        (true, 0b01111) if a && !scalar => fp(VFp2Op::Neg),
        (true, 0b11111) if a && !scalar => fp(VFp2Op::Sqrt),
        (false, 0b11000) if !scalar => fp(VFp2Op::Rint(if a { FpRound::PosInf } else { FpRound::NearestEven })),
        (false, 0b11001) if !scalar => fp(VFp2Op::Rint(if a { FpRound::Zero } else { FpRound::NegInf })),
        (true, 0b11000) if !a && !scalar => fp(VFp2Op::Rint(FpRound::Away)),
        (true, 0b11001) if !scalar => fp(VFp2Op::RintX),
        (_, 0b11010) => fp(VFp2Op::ToInt(if a { FpRound::PosInf } else { FpRound::NearestEven }, u)),
        (_, 0b11011) => fp(VFp2Op::ToInt(if a { FpRound::Zero } else { FpRound::NegInf }, u)),
        (_, 0b11100) if !a => fp(VFp2Op::ToInt(FpRound::Away, u)),
        (_, 0b11101) if !a => fp(VFp2Op::FromInt(u)),
        (false, 0b11101) if a => fp(VFp2Op::Recpe),
        (true, 0b11101) if a => fp(VFp2Op::Rsqrte),
        (false, 0b11100) if a && !scalar => {
            if ft == FpType::D {
                Inst::Undefined(w)
            } else {
                Inst::VecFp2 { op: VFp2Op::Urecpe, q, ft, scalar, rd, rn }
            }
        }
        (true, 0b11100) if a && !scalar => {
            if ft == FpType::D {
                Inst::Undefined(w)
            } else {
                Inst::VecFp2 { op: VFp2Op::Ursqrte, q, ft, scalar, rd, rn }
            }
        }
        (false, 0b10111) if !scalar => {
            // FCVTL: sz 0 = half to single (not implemented), 1 = single to double.
            if ft == FpType::D {
                Inst::VecFp2 { op: VFp2Op::Fcvtl, q, ft, scalar, rd, rn }
            } else {
                Inst::Unsupported(w)
            }
        }
        (false, 0b10110) if !scalar => {
            if ft == FpType::D {
                Inst::VecFp2 { op: VFp2Op::Fcvtn, q, ft, scalar, rd, rn }
            } else {
                Inst::Unsupported(w)
            }
        }
        // Integer.
        (_, 0b00010) if !scalar => {
            if size == 3 {
                Inst::Undefined(w)
            } else {
                misc(Misc2Op::Addlp(u))
            }
        }
        (_, 0b00110) if !scalar => {
            if size == 3 {
                Inst::Undefined(w)
            } else {
                misc(Misc2Op::Adalp(u))
            }
        }
        (false, 0b00111) | (true, 0b00111) | (false, 0b00011) | (true, 0b00011) => {
            if !scalar && size == 3 && !q {
                return Inst::Undefined(w);
            }
            misc(match (u, opcode) {
                (false, 0b00111) => Misc2Op::Sqabs,
                (true, 0b00111) => Misc2Op::Sqneg,
                (false, _) => Misc2Op::Suqadd,
                (true, _) => Misc2Op::Usqadd,
            })
        }
        (false, 0b10100) | (true, 0b10100) | (true, 0b10010) => {
            if size == 3 {
                return Inst::Undefined(w);
            }
            misc(match (u, opcode) {
                (false, 0b10100) => Misc2Op::Sqxtn,
                (true, 0b10100) => Misc2Op::Uqxtn,
                _ => Misc2Op::Sqxtun,
            })
        }
        (true, 0b10011) if !scalar => {
            if size == 3 {
                Inst::Undefined(w)
            } else {
                misc(Misc2Op::Shll)
            }
        }
        _ => Inst::Unsupported(w),
    }
}

/// Floating-point across lanes (single precision; the half forms are U = 0).
pub(crate) fn fp_across(w: u32, q: bool, u: bool) -> Inst {
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    if !u {
        return Inst::Unsupported(w); // half precision
    }
    let op = match (bit(w, 23), bits(w, 16, 12)) {
        (false, 0b01100) => FAcrossOp::MaxNmv,
        (false, 0b01111) => FAcrossOp::Maxv,
        (true, 0b01100) => FAcrossOp::MinNmv,
        (true, 0b01111) => FAcrossOp::Minv,
        _ => return Inst::Undefined(w),
    };
    if bit(w, 22) || !q {
        return Inst::Undefined(w);
    }
    Inst::VecFpAcross { op, rd, rn }
}

/// Scalar pairwise (`01 U 11110 size 11000 opcode 10`).
pub(crate) fn scalar_pairwise(w: u32) -> Inst {
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let u = bit(w, 29);
    let opcode = bits(w, 16, 12);
    if !u {
        if opcode == 0b11011 && bits(w, 23, 22) == 3 {
            return Inst::ScalarPair { op: None, ft: FpType::D, rd, rn };
        }
        if bits(w, 23, 22) == 3 && opcode == 0b11011 {
            return Inst::Undefined(w);
        }
        return Inst::Unsupported(w); // half-precision pairwise
    }
    let ft = fsz(w);
    let op = match (bit(w, 23), opcode) {
        (false, 0b01100) => VFp3Op::MaxNmp,
        (false, 0b01101) => VFp3Op::Addp,
        (false, 0b01111) => VFp3Op::Maxp,
        (true, 0b01100) => VFp3Op::MinNmp,
        (true, 0b01111) => VFp3Op::Minp,
        _ => return Inst::Undefined(w),
    };
    Inst::ScalarPair { op: Some(op), ft, rd, rn }
}

/// Three registers of different types (`0 Q U 01110 size 1 Rm opcode 00`).
pub(crate) fn three_diff(w: u32) -> Inst {
    let (rd, rn, rm) = regs(w);
    let q = bit(w, 30);
    let u = bit(w, 29);
    let size = bits(w, 23, 22) as u8;
    let (op, round) = match (u, bits(w, 15, 12)) {
        (_, 0b0000) => (DiffOp::Addl, false),
        (_, 0b0001) => (DiffOp::Addw, false),
        (_, 0b0010) => (DiffOp::Subl, false),
        (_, 0b0011) => (DiffOp::Subw, false),
        (_, 0b0100) => (DiffOp::Addhn, u),
        (_, 0b0101) => (DiffOp::Abal, false),
        (_, 0b0110) => (DiffOp::Subhn, u),
        (_, 0b0111) => (DiffOp::Abdl, false),
        (_, 0b1000) => (DiffOp::Mlal, false),
        (_, 0b1010) => (DiffOp::Mlsl, false),
        (_, 0b1100) => (DiffOp::Mull, false),
        (false, 0b1001) => (DiffOp::Sqdmlal, false),
        (false, 0b1011) => (DiffOp::Sqdmlsl, false),
        (false, 0b1101) => (DiffOp::Sqdmull, false),
        (false, 0b1110) => (DiffOp::Pmull, false),
        _ => return Inst::Undefined(w),
    };
    let unsigned = u && !matches!(op, DiffOp::Addhn | DiffOp::Subhn);
    match op {
        DiffOp::Pmull => {
            if size == 1 || size == 2 {
                return Inst::Undefined(w);
            }
        }
        DiffOp::Sqdmlal | DiffOp::Sqdmlsl | DiffOp::Sqdmull => {
            if size == 0 || size == 3 {
                return Inst::Undefined(w);
            }
        }
        _ => {
            if size == 3 {
                return Inst::Undefined(w);
            }
        }
    }
    Inst::VecDiff { op, unsigned, round, q, size, rd, rn, rm }
}

/// By-element forms (vector `0 Q U 01111 size L M Rm opcode H 0`, scalar
/// `01 U 11111 ...`).
pub(crate) fn by_element(w: u32, scalar: bool) -> Inst {
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let q = bit(w, 30);
    let u = bit(w, 29);
    let size = bits(w, 23, 22) as u8;
    let (l, m, h) = (bits(w, 21, 21), bits(w, 20, 20), bits(w, 11, 11));
    let opcode = bits(w, 15, 12);
    // Integer element: H gives a 4-bit Rm and index H:L:M; S a 5-bit Rm and H:L.
    let int_elem = |size: u8| -> Option<(Reg, u8)> {
        match size {
            1 => Some((bits(w, 19, 16) as Reg, ((h << 2) | (l << 1) | m) as u8)),
            2 => Some((bits(w, 20, 16) as Reg, ((h << 1) | l) as u8)),
            _ => None,
        }
    };
    let fp_elem = || -> Option<(FpType, Reg, u8)> {
        match size {
            2 => Some((FpType::S, bits(w, 20, 16) as Reg, ((h << 1) | l) as u8)),
            3 if l == 0 => Some((FpType::D, bits(w, 20, 16) as Reg, h as u8)),
            _ => None,
        }
    };
    let op = match (u, opcode) {
        (false, 0b0001) => ElemOp::Fmla,
        (false, 0b0101) => ElemOp::Fmls,
        (false, 0b1001) => ElemOp::Fmul,
        (true, 0b1001) => ElemOp::Fmulx,
        (true, 0b0000) if !scalar => ElemOp::Mla,
        (true, 0b0100) if !scalar => ElemOp::Mls,
        (false, 0b1000) if !scalar => ElemOp::Mul,
        (false, 0b1100) => ElemOp::Sqdmulh,
        (false, 0b1101) => ElemOp::Sqrdmulh,
        (true, 0b1101) => ElemOp::Sqrdmlah,
        (true, 0b1111) => ElemOp::Sqrdmlsh,
        (_, 0b0010) if !scalar => ElemOp::Long(DiffOp::Mlal, u),
        (_, 0b0110) if !scalar => ElemOp::Long(DiffOp::Mlsl, u),
        (_, 0b1010) if !scalar => ElemOp::Long(DiffOp::Mull, u),
        (false, 0b0011) => ElemOp::Long(DiffOp::Sqdmlal, false),
        (false, 0b0111) => ElemOp::Long(DiffOp::Sqdmlsl, false),
        (false, 0b1011) => ElemOp::Long(DiffOp::Sqdmull, false),
        (_, 0b1110) if !scalar => ElemOp::Dot(u),
        _ => return Inst::Unsupported(w),
    };
    match op {
        ElemOp::Fmla | ElemOp::Fmls | ElemOp::Fmul | ElemOp::Fmulx => {
            if size < 2 {
                return Inst::Unsupported(w); // half precision
            }
            let Some((ft, rm, index)) = fp_elem() else { return Inst::Undefined(w) };
            if !scalar && ft == FpType::D && !q {
                return Inst::Undefined(w);
            }
            Inst::VecElem { op, q, size: if ft == FpType::D { 3 } else { 2 }, scalar, rd, rn, rm, index }
        }
        ElemOp::Dot(_) => {
            if size != 2 {
                return Inst::Undefined(w);
            }
            Inst::VecElem {
                op,
                q,
                size: 2,
                scalar: false,
                rd,
                rn,
                rm: bits(w, 20, 16) as Reg,
                index: ((h << 1) | l) as u8,
            }
        }
        ElemOp::Long(DiffOp::Sqdmlal | DiffOp::Sqdmlsl | DiffOp::Sqdmull, _) if scalar => {
            // The scalar forms of the doubling long multiplies: not yet.
            Inst::Unsupported(w)
        }
        _ => {
            let Some((rm, index)) = int_elem(size) else { return Inst::Undefined(w) };
            Inst::VecElem { op, q, size, scalar, rd, rn, rm, index }
        }
    }
}

/// Shifts by immediate the first subset leaves out. `size`, `right` and
/// `left` are worked out by the caller.
pub(crate) fn shift_imm(w: u32, scalar: bool, size: u8, right: u8, left: u8) -> Inst {
    let q = bit(w, 30);
    let u = bit(w, 29);
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let opcode = bits(w, 15, 11);
    let (op, shift) = match (u, opcode) {
        (_, 0b00100) => (Shift2Op::Rshr(u), right),
        (_, 0b00110) => (Shift2Op::Rsra(u), right),
        (false, 0b10001) => (Shift2Op::Rshrn, right),
        (_, 0b01110) => (Shift2Op::Qshl(u), left),
        (true, 0b01100) => (Shift2Op::Sqshlu, left),
        (false, 0b10010) => (Shift2Op::Qshrn { unsigned: false, round: false }, right),
        (false, 0b10011) => (Shift2Op::Qshrn { unsigned: false, round: true }, right),
        (true, 0b10010) => (Shift2Op::Qshrn { unsigned: true, round: false }, right),
        (true, 0b10011) => (Shift2Op::Qshrn { unsigned: true, round: true }, right),
        (true, 0b10000) => (Shift2Op::Qshrun { round: false }, right),
        (true, 0b10001) => (Shift2Op::Qshrun { round: true }, right),
        (_, 0b11100) => (Shift2Op::CvtFrom(u), right),
        (_, 0b11111) => (Shift2Op::CvtTo(u), right),
        _ => return Inst::Undefined(w),
    };
    let narrow = matches!(op, Shift2Op::Rshrn | Shift2Op::Qshrn { .. } | Shift2Op::Qshrun { .. });
    let cvt = matches!(op, Shift2Op::CvtFrom(_) | Shift2Op::CvtTo(_));
    if cvt {
        // Only S and D elements (half precision is FEAT_FP16, not here).
        if size < 2 {
            return Inst::Unsupported(w);
        }
        if !scalar && size == 3 && !q {
            return Inst::Undefined(w);
        }
    } else if narrow {
        if size == 3 || (scalar && op == Shift2Op::Rshrn) {
            return Inst::Undefined(w);
        }
    } else if scalar {
        if matches!(op, Shift2Op::Rshr(_) | Shift2Op::Rsra(_)) && size != 3 {
            return Inst::Undefined(w);
        }
    } else if size == 3 && !q {
        return Inst::Undefined(w);
    }
    Inst::VecShift2 { op, q, size, scalar, shift, rd, rn }
}

/// The SHA3 instructions (`EOR3`, `BCAX`, `RAX1`, `XAR`); the other
/// `0xCE` encodings (SHA512, SM3, SM4) are not implemented.
pub(crate) fn crypto_sha3(w: u32) -> Inst {
    let (rd, rn, rm) = regs(w);
    let ra = bits(w, 14, 10) as Reg;
    match bits(w, 23, 21) {
        0b000 if !bit(w, 15) => Inst::Sha3 { op: Sha3Op::Eor3, rd, rn, rm, ra, imm: 0 },
        0b001 if !bit(w, 15) => Inst::Sha3 { op: Sha3Op::Bcax, rd, rn, rm, ra, imm: 0 },
        0b011 if bits(w, 15, 10) == 0b100011 => Inst::Sha3 { op: Sha3Op::Rax1, rd, rn, rm, ra: 0, imm: 0 },
        0b100 => Inst::Sha3 { op: Sha3Op::Xar, rd, rn, rm, ra: 0, imm: bits(w, 15, 10) as u8 },
        0b000 | 0b001 | 0b010 | 0b011 | 0b110 => Inst::Unsupported(w),
        _ => Inst::Undefined(w),
    }
}
