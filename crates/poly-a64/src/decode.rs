//! The A64 decoder: a pure function from a 32-bit instruction word to an
//! [`Inst`].
//!
//! `decode(word)` looks at the word and nothing else. It never panics and
//! never reads memory or registers, so it can run over arbitrary data (the
//! tests decode millions of random words). Encodings the architecture does
//! not allocate decode to [`Inst::Undefined`]; encodings it allocates but
//! this crate does not implement decode to [`Inst::Unsupported`].
//!
//! The decoder is split from the interpreter on purpose: the later JIT
//! (ADR 0152) decodes a block once into `Inst` values and translates those,
//! so both back ends share one source of truth for the encoding tables.
//!
//! The tables follow the Arm Architecture Reference Manual for A-profile
//! (DDI 0487), chapter C4, "A64 instruction set encoding". The top-level
//! split is on bits 28:25 (`op0`); each class has its own function below.
//! Floating-point and Advanced SIMD classes are in [`crate::decode_simd`].

use crate::decode_simd;
use crate::inst::*;

/// Bits `hi..=lo` of `w`.
#[inline]
pub(crate) const fn bits(w: u32, hi: u32, lo: u32) -> u32 {
    (w >> lo) & ((1u32 << (hi - lo + 1)) - 1)
}

/// Bit `n` of `w`.
#[inline]
pub(crate) const fn bit(w: u32, n: u32) -> bool {
    (w >> n) & 1 != 0
}

/// Sign-extend the low `n` bits of `v`.
#[inline]
pub(crate) const fn sext(v: u32, n: u32) -> i64 {
    (((v as u64) << (64 - n)) as i64) >> (64 - n)
}

/// `n` low ones (`n` from 0 to 64).
const fn ones(n: u32) -> u64 {
    if n >= 64 { u64::MAX } else { (1u64 << n) - 1 }
}

/// Rotate the `esize`-bit value `x` right by `r` within `esize` bits.
const fn ror_in(x: u64, r: u32, esize: u32) -> u64 {
    let r = r % esize;
    if r == 0 { x } else { ((x >> r) | (x << (esize - r))) & ones(esize) }
}

/// `DecodeBitMasks` from the Arm ARM: the `(wmask, tmask)` of a bitfield or
/// logical immediate, for registers of `datasize` bits. `None` for the
/// encodings that are reserved.
pub fn decode_bit_masks(n: u32, imms: u32, immr: u32, immediate: bool, datasize: u32) -> Option<(u64, u64)> {
    let x = (n << 6) | (!imms & 0x3F);
    if x == 0 {
        return None;
    }
    let len = 31 - x.leading_zeros();
    if len < 1 || datasize < (1 << len) {
        return None;
    }
    let levels = (1u32 << len) - 1;
    if immediate && (imms & levels) == levels {
        return None;
    }
    let s = imms & levels;
    let r = immr & levels;
    let d = s.wrapping_sub(r) & levels;
    let esize = 1u32 << len;
    let welem = ones(s + 1);
    let telem = ones(d + 1);
    let wrot = ror_in(welem, r, esize);
    let mut wmask = 0u64;
    let mut tmask = 0u64;
    let mut i = 0;
    while i < datasize {
        wmask |= wrot << i;
        tmask |= telem << i;
        i += esize;
    }
    let m = ones(datasize);
    Some((wmask & m, tmask & m))
}

/// Decode one instruction word.
///
/// ```
/// use poly_a64::{decode, Inst};
/// // add x0, x1, #5
/// assert!(matches!(decode(0x9100_1420), Inst::AddSubImm { imm: 5, .. }));
/// ```
pub fn decode(w: u32) -> Inst {
    match bits(w, 28, 25) {
        0b0000 => {
            // Reserved group; `UDF #imm16` is bits 31:16 all zero.
            if bits(w, 31, 29) == 0 && bits(w, 24, 16) == 0 { Inst::Undefined(w) } else { Inst::Unsupported(w) }
        }
        0b0001 | 0b0011 => Inst::Undefined(w),
        0b0010 => Inst::Unsupported(w), // SVE
        0b1000 | 0b1001 => decode_dp_imm(w),
        0b1010 | 0b1011 => decode_branch_sys(w),
        0b0100 | 0b0110 | 0b1100 | 0b1110 => decode_ldst(w),
        0b0101 | 0b1101 => decode_dp_reg(w),
        _ => decode_simd::decode_fp_simd(w), // 0111 and 1111
    }
}

// ---------------------------------------------------------------------
// Data processing, immediate
// ---------------------------------------------------------------------

fn decode_dp_imm(w: u32) -> Inst {
    let sf = bit(w, 31);
    let wd = Width::from_sf(sf);
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    match bits(w, 25, 23) {
        0b000 | 0b001 => {
            let imm = (bits(w, 23, 5) << 2) | bits(w, 30, 29);
            let off = sext(imm, 21);
            if sf {
                Inst::Adr { rd, offset: off << 12, page: true }
            } else {
                Inst::Adr { rd, offset: off, page: false }
            }
        }
        0b010 => {
            let imm12 = bits(w, 21, 10);
            let imm = if bit(w, 22) { imm12 << 12 } else { imm12 };
            Inst::AddSubImm { w: wd, sub: bit(w, 30), s: bit(w, 29), rd, rn, imm }
        }
        0b011 => Inst::Unsupported(w), // add/subtract with tags (MTE)
        0b100 => {
            let n = bits(w, 22, 22);
            if !sf && n == 1 {
                return Inst::Undefined(w);
            }
            let Some((wmask, _)) = decode_bit_masks(n, bits(w, 15, 10), bits(w, 21, 16), true, wd.bits()) else {
                return Inst::Undefined(w);
            };
            let op = match bits(w, 30, 29) {
                0 => LogOp::And,
                1 => LogOp::Orr,
                2 => LogOp::Eor,
                _ => LogOp::Ands,
            };
            Inst::LogicalImm { w: wd, op, rd, rn, imm: wmask }
        }
        0b101 => {
            let hw = bits(w, 22, 21);
            if !sf && hw >= 2 {
                return Inst::Undefined(w);
            }
            let op = match bits(w, 30, 29) {
                0 => MovOp::Movn,
                2 => MovOp::Movz,
                3 => MovOp::Movk,
                _ => return Inst::Undefined(w),
            };
            Inst::MoveWide { w: wd, op, rd, imm: bits(w, 20, 5) as u16, shift: (hw * 16) as u8 }
        }
        0b110 => {
            let n = bits(w, 22, 22);
            if bit(w, 22) != sf {
                return Inst::Undefined(w);
            }
            let op = match bits(w, 30, 29) {
                0 => BfOp::Sbfm,
                1 => BfOp::Bfm,
                2 => BfOp::Ubfm,
                _ => return Inst::Undefined(w),
            };
            let immr = bits(w, 21, 16);
            let imms = bits(w, 15, 10);
            if !sf && (bit(w, 21) || bit(w, 15)) {
                // immr<5> and imms<5> must be 0 for 32-bit registers.
                return Inst::Undefined(w);
            }
            let Some((wmask, tmask)) = decode_bit_masks(n, imms, immr, false, wd.bits()) else {
                return Inst::Undefined(w);
            };
            Inst::Bitfield { w: wd, op, rd, rn, immr: immr as u8, imms: imms as u8, wmask, tmask }
        }
        _ => {
            // 0b111: extract.
            if bits(w, 30, 29) != 0 || bit(w, 21) || bit(w, 22) != sf {
                return Inst::Undefined(w);
            }
            let imms = bits(w, 15, 10);
            if !sf && imms >= 32 {
                return Inst::Undefined(w);
            }
            Inst::Extr { w: wd, rd, rn, rm: bits(w, 20, 16) as Reg, lsb: imms as u8 }
        }
    }
}

// ---------------------------------------------------------------------
// Branches, exception generation, system
// ---------------------------------------------------------------------

fn decode_branch_sys(w: u32) -> Inst {
    let rt = bits(w, 4, 0) as Reg;
    if bits(w, 30, 26) == 0b00101 {
        return Inst::Branch { link: bit(w, 31), offset: sext(bits(w, 25, 0), 26) << 2 };
    }
    match bits(w, 31, 25) {
        0b0011010 | 0b1011010 => Inst::CompareBranch {
            w: Width::from_sf(bit(w, 31)),
            nz: bit(w, 24),
            rt,
            offset: sext(bits(w, 23, 5), 19) << 2,
        },
        0b0011011 | 0b1011011 => Inst::TestBranch {
            nz: bit(w, 24),
            rt,
            bit: ((bits(w, 31, 31) << 5) | bits(w, 23, 19)) as u8,
            offset: sext(bits(w, 18, 5), 14) << 2,
        },
        0b0101010 => {
            // Conditional branch (immediate): o1 = bit 24, o0 = bit 4 both 0.
            if bit(w, 24) || bit(w, 4) {
                Inst::Unsupported(w) // BC.cond (FEAT_HBC)
            } else {
                Inst::BranchCond { cond: Cond::from_bits(bits(w, 3, 0)), offset: sext(bits(w, 23, 5), 19) << 2 }
            }
        }
        0b1101010 if bits(w, 24, 24) == 0 => decode_exception(w),
        0b1101010 if bits(w, 31, 22) == 0b1101010100 => decode_system(w),
        0b1101011 => decode_branch_reg(w),
        _ => Inst::Undefined(w),
    }
}

fn decode_exception(w: u32) -> Inst {
    // 1101 0100 opc(23:21) imm16(20:5) op2(4:2) LL(1:0)
    let opc = bits(w, 23, 21);
    let imm16 = bits(w, 20, 5) as u16;
    let op2 = bits(w, 4, 2);
    let ll = bits(w, 1, 0);
    match (opc, op2, ll) {
        (0b000, 0, 0b01) => Inst::Svc(imm16),
        (0b001, 0, 0b00) => Inst::Brk(imm16),
        _ => Inst::Undefined(w), // HVC, SMC, HLT, DCPS: not for user mode
    }
}

fn decode_branch_reg(w: u32) -> Inst {
    // 1101011 opc(24:21) op2(20:16) op3(15:10) Rn op4(4:0)
    let rn = bits(w, 9, 5) as Reg;
    let opc = bits(w, 24, 21);
    let op2 = bits(w, 20, 16);
    let op3 = bits(w, 15, 10);
    let op4 = bits(w, 4, 0);
    if op2 != 0b11111 {
        return Inst::Undefined(w);
    }
    if op3 == 0 && op4 == 0 {
        return match opc {
            0b0000 => Inst::BranchReg { rn },
            0b0001 => Inst::BranchLinkReg { rn },
            0b0010 => Inst::Ret { rn },
            _ => Inst::Undefined(w),
        };
    }
    if bits(w, 15, 11) == 0b00001 {
        return Inst::Unsupported(w); // pointer-authenticated branches
    }
    Inst::Undefined(w)
}

fn decode_system(w: u32) -> Inst {
    let l = bit(w, 21);
    let op0 = bits(w, 20, 19);
    let op1 = bits(w, 18, 16);
    let crn = bits(w, 15, 12);
    let crm = bits(w, 11, 8);
    let op2 = bits(w, 7, 5);
    let rt = bits(w, 4, 0) as Reg;
    if op0 == 0 {
        if l {
            return Inst::Undefined(w);
        }
        // Hints, barriers and PSTATE moves.
        return match crn {
            2 if op1 == 3 && rt == 31 => Inst::Hint { crm_op2: ((crm << 3) | op2) as u8 },
            3 if op1 == 3 && rt == 31 => match op2 {
                2 => Inst::Barrier { kind: Barrier::Clrex, crm: crm as u8 },
                4 => Inst::Barrier { kind: Barrier::Dsb, crm: crm as u8 },
                5 => Inst::Barrier { kind: Barrier::Dmb, crm: crm as u8 },
                6 => Inst::Barrier { kind: Barrier::Isb, crm: crm as u8 },
                7 => Inst::Barrier { kind: Barrier::Sb, crm: crm as u8 },
                _ => Inst::Unsupported(w), // DSB nXS, reserved
            },
            4 if rt == 31 => Inst::Unsupported(w), // MSR (immediate): PSTATE fields
            _ => Inst::Unsupported(w),
        };
    }
    if op0 == 1 {
        // SYS (L = 0) and SYSL (L = 1).
        if l {
            return Inst::Unsupported(w);
        }
        return Inst::Sys { op1: op1 as u8, crn: crn as u8, crm: crm as u8, op2: op2 as u8, rt };
    }
    let reg = SysReg::new(op0 as u16, op1 as u16, crn as u16, crm as u16, op2 as u16);
    if l { Inst::Mrs { rt, reg } } else { Inst::Msr { rt, reg } }
}

// ---------------------------------------------------------------------
// Loads and stores
// ---------------------------------------------------------------------

/// The kind and log2 size of a single load or store from its `size` and
/// `opc` fields. `None` for unallocated combinations.
fn ldst_kind(size: u32, opc: u32, v: bool) -> Option<(MemKind, u8)> {
    if !v {
        match (size, opc) {
            (_, 0) => Some((MemKind::Store, size as u8)),
            (_, 1) => Some((MemKind::Load, size as u8)),
            (0 | 1, 2) => Some((MemKind::LoadSigned(Width::X), size as u8)),
            (0 | 1, 3) => Some((MemKind::LoadSigned(Width::W), size as u8)),
            (2, 2) => Some((MemKind::LoadSigned(Width::X), 2)),
            (3, 2) => Some((MemKind::Prefetch, 3)),
            _ => None,
        }
    } else {
        match (size, opc) {
            (_, 0) => Some((MemKind::Store, size as u8)),
            (_, 1) => Some((MemKind::Load, size as u8)),
            (0, 2) => Some((MemKind::Store, 4)),
            (0, 3) => Some((MemKind::Load, 4)),
            _ => None,
        }
    }
}

fn decode_ldst(w: u32) -> Inst {
    let v = bit(w, 26);
    let size = bits(w, 31, 30);
    let opc = bits(w, 23, 22);
    let rt = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    match bits(w, 29, 27) {
        0b001 => {
            if v {
                return decode_simd::decode_simd_ldst_struct(w);
            }
            decode_exclusive(w)
        }
        0b011 => {
            // Load register (literal): bits 25:24 = 00.
            if bits(w, 25, 24) != 0 {
                return Inst::Undefined(w);
            }
            let offset = sext(bits(w, 23, 5), 19) << 2;
            let addr = Addr::Literal { offset };
            if v {
                let sz = match size {
                    0 => 2,
                    1 => 3,
                    2 => 4,
                    _ => return Inst::Undefined(w),
                };
                return Inst::LoadStore { kind: MemKind::Load, size: sz, fp: true, rt, addr };
            }
            match size {
                0 => Inst::LoadStore { kind: MemKind::Load, size: 2, fp: false, rt, addr },
                1 => Inst::LoadStore { kind: MemKind::Load, size: 3, fp: false, rt, addr },
                2 => Inst::LoadStore { kind: MemKind::LoadSigned(Width::X), size: 2, fp: false, rt, addr },
                _ => Inst::LoadStore { kind: MemKind::Prefetch, size: 3, fp: false, rt, addr },
            }
        }
        0b101 => decode_pair(w),
        0b111 => {
            if bit(w, 24) {
                // Unsigned scaled immediate.
                let Some((kind, sz)) = ldst_kind(size, opc, v) else { return Inst::Undefined(w) };
                let offset = (bits(w, 21, 10) as i64) << sz;
                return Inst::LoadStore { kind, size: sz, fp: v, rt, addr: Addr::Imm { rn, offset } };
            }
            if bits(w, 25, 24) != 0 {
                return Inst::Undefined(w);
            }
            if !bit(w, 21) {
                // Unscaled, post-index, unprivileged, pre-index (imm9).
                let Some((kind, sz)) = ldst_kind(size, opc, v) else { return Inst::Undefined(w) };
                let offset = sext(bits(w, 20, 12), 9);
                let (addr, ok_prefetch) = match bits(w, 11, 10) {
                    0b00 => (Addr::Imm { rn, offset }, true),
                    0b01 => (Addr::Post { rn, offset }, false),
                    0b10 => (Addr::Imm { rn, offset }, false), // LDTR/STTR
                    _ => (Addr::Pre { rn, offset }, false),
                };
                if kind == MemKind::Prefetch && !ok_prefetch {
                    return Inst::Undefined(w);
                }
                if bits(w, 11, 10) == 0b10 && (v || kind == MemKind::Prefetch) {
                    return Inst::Undefined(w);
                }
                return Inst::LoadStore { kind, size: sz, fp: v, rt, addr };
            }
            match bits(w, 11, 10) {
                0b00 => {
                    if v {
                        return Inst::Undefined(w);
                    }
                    decode_atomic(w)
                }
                0b10 => {
                    // Register offset.
                    let Some((kind, sz)) = ldst_kind(size, opc, v) else { return Inst::Undefined(w) };
                    let option = bits(w, 15, 13);
                    if option & 2 == 0 {
                        return Inst::Undefined(w);
                    }
                    let shift = if bit(w, 12) { sz } else { 0 };
                    Inst::LoadStore {
                        kind,
                        size: sz,
                        fp: v,
                        rt,
                        addr: Addr::Reg { rn, rm: bits(w, 20, 16) as Reg, ext: Extend::from_bits(option), shift },
                    }
                }
                _ => Inst::Unsupported(w), // PAC loads (LDRAA/LDRAB)
            }
        }
        _ => Inst::Undefined(w),
    }
}

fn decode_pair(w: u32) -> Inst {
    let opc = bits(w, 31, 30);
    let v = bit(w, 26);
    let l = bit(w, 22);
    let rt = bits(w, 4, 0) as Reg;
    let rt2 = bits(w, 14, 10) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let (kind, size) = if v {
        match opc {
            0 => (if l { PairKind::Load } else { PairKind::Store }, 2u8),
            1 => (if l { PairKind::Load } else { PairKind::Store }, 3),
            2 => (if l { PairKind::Load } else { PairKind::Store }, 4),
            _ => return Inst::Undefined(w),
        }
    } else {
        match opc {
            0 => (if l { PairKind::Load } else { PairKind::Store }, 2u8),
            1 => {
                if l {
                    (PairKind::LoadSw, 2)
                } else {
                    return Inst::Unsupported(w); // STGP (MTE)
                }
            }
            2 => (if l { PairKind::Load } else { PairKind::Store }, 3),
            _ => return Inst::Undefined(w),
        }
    };
    let offset = sext(bits(w, 21, 15), 7) << size;
    let addr = match bits(w, 24, 23) {
        0b00 | 0b10 => Addr::Imm { rn, offset },
        0b01 => Addr::Post { rn, offset },
        _ => Addr::Pre { rn, offset },
    };
    Inst::LoadStorePair { kind, size, fp: v, rt, rt2, addr }
}

fn decode_exclusive(w: u32) -> Inst {
    let size = bits(w, 31, 30) as u8;
    let o2 = bit(w, 23);
    let l = bit(w, 22);
    let o1 = bit(w, 21);
    let o0 = bit(w, 15);
    let rs = bits(w, 20, 16) as Reg;
    let rt2 = bits(w, 14, 10) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let rt = bits(w, 4, 0) as Reg;
    match (o2, o1) {
        (false, false) => {
            let kind = if l { ExclKind::Load } else { ExclKind::Store };
            Inst::Exclusive { kind, size, ordered: o0, rs, rt, rt2, rn }
        }
        (false, true) => {
            if size >= 2 {
                // LDXP/STXP/LDAXP/STLXP: the size bit selects W or X pairs.
                let kind = if l { ExclKind::LoadPair } else { ExclKind::StorePair };
                Inst::Exclusive { kind, size, ordered: o0, rs, rt, rt2, rn }
            } else if rt2 == 31 && rs & 1 == 0 && rt & 1 == 0 {
                Inst::Casp { size: 2 + size, acq: l, rel: o0, rs, rt, rn }
            } else {
                Inst::Undefined(w)
            }
        }
        (true, false) => Inst::Ordered { store: !l, size, rt, rn },
        (true, true) => Inst::Cas { size, acq: l, rel: o0, rs, rt, rn },
    }
}

fn decode_atomic(w: u32) -> Inst {
    let size = bits(w, 31, 30) as u8;
    let acq = bit(w, 23);
    let rel = bit(w, 22);
    let rs = bits(w, 20, 16) as Reg;
    let o3 = bit(w, 15);
    let opc = bits(w, 14, 12);
    let rn = bits(w, 9, 5) as Reg;
    let rt = bits(w, 4, 0) as Reg;
    let op = if !o3 {
        match opc {
            0 => AtomOp::Add,
            1 => AtomOp::Clr,
            2 => AtomOp::Eor,
            3 => AtomOp::Set,
            4 => AtomOp::Smax,
            5 => AtomOp::Smin,
            6 => AtomOp::Umax,
            _ => AtomOp::Umin,
        }
    } else {
        match opc {
            0 => AtomOp::Swp,
            4 if acq && !rel && rs == 31 => return Inst::Ordered { store: false, size, rt, rn }, // LDAPR
            _ => return Inst::Unsupported(w),                                                    // LD64B, ST64B, ...
        }
    };
    Inst::Atomic { op, size, acq, rel, rs, rt, rn }
}

// ---------------------------------------------------------------------
// Data processing, register
// ---------------------------------------------------------------------

fn decode_dp_reg(w: u32) -> Inst {
    let sf = bit(w, 31);
    let wd = Width::from_sf(sf);
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let rm = bits(w, 20, 16) as Reg;
    if !bit(w, 28) {
        // Logical and add/subtract (shifted and extended register).
        if !bit(w, 24) {
            let amount = bits(w, 15, 10);
            if !sf && amount >= 32 {
                return Inst::Undefined(w);
            }
            let op = match bits(w, 30, 29) {
                0 => LogOp::And,
                1 => LogOp::Orr,
                2 => LogOp::Eor,
                _ => LogOp::Ands,
            };
            return Inst::LogicalShifted {
                w: wd,
                op,
                invert: bit(w, 21),
                rd,
                rn,
                rm,
                shift: Shift::from_bits(bits(w, 23, 22)),
                amount: amount as u8,
            };
        }
        let sub = bit(w, 30);
        let s = bit(w, 29);
        if !bit(w, 21) {
            let shift = bits(w, 23, 22);
            let amount = bits(w, 15, 10);
            if shift == 3 || (!sf && amount >= 32) {
                return Inst::Undefined(w);
            }
            return Inst::AddSubShifted {
                w: wd,
                sub,
                s,
                rd,
                rn,
                rm,
                shift: Shift::from_bits(shift),
                amount: amount as u8,
            };
        }
        let amount = bits(w, 12, 10);
        if bits(w, 23, 22) != 0 || amount > 4 {
            return Inst::Undefined(w);
        }
        return Inst::AddSubExt {
            w: wd,
            sub,
            s,
            rd,
            rn,
            rm,
            ext: Extend::from_bits(bits(w, 15, 13)),
            amount: amount as u8,
        };
    }
    if bit(w, 24) {
        return decode_dp3(w);
    }
    match bits(w, 23, 21) {
        0b000 => {
            // Add/subtract with carry (op3 = 000000); rotate and flag-set forms are FEAT_FlagM.
            if bits(w, 15, 10) == 0 {
                Inst::AddSubCarry { w: wd, sub: bit(w, 30), s: bit(w, 29), rd, rn, rm }
            } else {
                Inst::Unsupported(w)
            }
        }
        0b010 => {
            // Conditional compare (register / immediate): S must be 1, o2 = bit 10 = 0, o3 = bit 4 = 0.
            if !bit(w, 29) || bit(w, 10) || bit(w, 4) {
                return Inst::Undefined(w);
            }
            let rhs = if bit(w, 11) { CmpRhs::Imm(bits(w, 20, 16) as u8) } else { CmpRhs::Reg(rm) };
            Inst::CondCmp {
                w: wd,
                neg: !bit(w, 30),
                rn,
                rhs,
                cond: Cond::from_bits(bits(w, 15, 12)),
                nzcv: bits(w, 3, 0) as u8,
            }
        }
        0b100 => {
            // Conditional select: S = 0 and op2 bit 11 = 0.
            if bit(w, 29) || bit(w, 11) {
                return Inst::Undefined(w);
            }
            let op = match (bit(w, 30), bit(w, 10)) {
                (false, false) => CselOp::Csel,
                (false, true) => CselOp::Csinc,
                (true, false) => CselOp::Csinv,
                (true, true) => CselOp::Csneg,
            };
            Inst::CondSel { w: wd, op, rd, rn, rm, cond: Cond::from_bits(bits(w, 15, 12)) }
        }
        0b110 => {
            if bit(w, 29) {
                return Inst::Unsupported(w); // SUBPS, flag forms
            }
            if bit(w, 30) { decode_dp1(w) } else { decode_dp2(w) }
        }
        _ => Inst::Undefined(w),
    }
}

fn decode_dp2(w: u32) -> Inst {
    let wd = Width::from_sf(bit(w, 31));
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let rm = bits(w, 20, 16) as Reg;
    match bits(w, 15, 10) {
        0b000010 => Inst::Div { w: wd, signed: false, rd, rn, rm },
        0b000011 => Inst::Div { w: wd, signed: true, rd, rn, rm },
        0b001000 => Inst::ShiftVar { w: wd, op: Shift::Lsl, rd, rn, rm },
        0b001001 => Inst::ShiftVar { w: wd, op: Shift::Lsr, rd, rn, rm },
        0b001010 => Inst::ShiftVar { w: wd, op: Shift::Asr, rd, rn, rm },
        0b001011 => Inst::ShiftVar { w: wd, op: Shift::Ror, rd, rn, rm },
        0b000000 | 0b000001 | 0b000100 | 0b000101 | 0b001100 | 0b010000..=0b010111 => Inst::Unsupported(w),
        _ => Inst::Undefined(w),
    }
}

fn decode_dp1(w: u32) -> Inst {
    let sf = bit(w, 31);
    let wd = Width::from_sf(sf);
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    if bits(w, 20, 16) != 0 {
        return if bits(w, 20, 16) == 1 { Inst::Unsupported(w) } else { Inst::Undefined(w) };
    }
    let op = match (bits(w, 15, 10), sf) {
        (0b000000, _) => Bit1Op::Rbit,
        (0b000001, _) => Bit1Op::Rev16,
        (0b000010, _) => Bit1Op::Rev32,
        (0b000011, true) => Bit1Op::Rev64,
        (0b000100, _) => Bit1Op::Clz,
        (0b000101, _) => Bit1Op::Cls,
        _ => return Inst::Undefined(w),
    };
    Inst::Bit1 { w: wd, op, rd, rn }
}

fn decode_dp3(w: u32) -> Inst {
    let sf = bit(w, 31);
    let wd = Width::from_sf(sf);
    let rd = bits(w, 4, 0) as Reg;
    let rn = bits(w, 9, 5) as Reg;
    let rm = bits(w, 20, 16) as Reg;
    let ra = bits(w, 14, 10) as Reg;
    let o0 = bit(w, 15);
    if bits(w, 30, 29) != 0 {
        return Inst::Undefined(w);
    }
    match bits(w, 23, 21) {
        0b000 => Inst::Madd { w: wd, sub: o0, rd, rn, rm, ra },
        0b001 if sf => Inst::MaddLong { signed: true, sub: o0, rd, rn, rm, ra },
        0b101 if sf => Inst::MaddLong { signed: false, sub: o0, rd, rn, rm, ra },
        0b010 if sf && !o0 && ra == 31 => Inst::MulHigh { signed: true, rd, rn, rm },
        0b110 if sf && !o0 && ra == 31 => Inst::MulHigh { signed: false, rd, rn, rm },
        _ => Inst::Undefined(w),
    }
}
