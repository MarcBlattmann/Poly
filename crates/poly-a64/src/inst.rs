//! The decoded form of an A64 instruction.
//!
//! [`Inst`] is what [`crate::decode::decode`] returns. It is `Copy`, holds no
//! pointers and no execution state, and is fully decoded: register numbers
//! are plain numbers (0 to 31), immediates are already expanded (logical
//! bitmask immediates, bitfield masks, floating-point immediates, shifted
//! `ADD` immediates), branch and literal offsets are sign-extended byte
//! offsets from the address of the instruction, and condition codes, shifts,
//! extends and addressing modes are enums.
//!
//! **Why a separate type.** The interpreter in [`crate::cpu`] runs an
//! `Inst`, and the later JIT (ADR 0152) translates blocks of `Inst` to
//! x86-64. Both need the same answers to "which instruction is this and
//! what are its operands", so the answer lives here once and neither side
//! looks at raw instruction words again. Decoding is a pure function of the
//! 32-bit word: the same word always gives the same `Inst`, so a decoded
//! block can be cached by address.
//!
//! **Register 31.** The decoder does not decide what register number 31
//! means. Each operand's role (stack pointer or zero register) follows from
//! the instruction class and is documented on the variant: for example the
//! base register of a load is `SP`, the destination of a flag-setting
//! `ADD` is `XZR`. [`Reg`] values keep the raw number 31.
//!
//! Names and field layouts follow the Arm Architecture Reference Manual for
//! A-profile (DDI 0487), chapter C4 "A64 Instruction Set Encoding".

/// A register number, 0 to 31. What 31 means depends on the operand (see the
/// module docs).
pub type Reg = u8;

/// The register 31 number.
pub const R31: Reg = 31;

/// Operand width of an integer instruction: `W` registers (32 bits) or `X`
/// registers (64 bits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    /// 32-bit (`Wn`); results zero-extend into the 64-bit register.
    W,
    /// 64-bit (`Xn`).
    X,
}

impl Width {
    /// 32 or 64.
    pub const fn bits(self) -> u32 {
        match self {
            Width::W => 32,
            Width::X => 64,
        }
    }

    /// All ones in the width.
    pub const fn mask(self) -> u64 {
        match self {
            Width::W => 0xFFFF_FFFF,
            Width::X => u64::MAX,
        }
    }

    /// `X` if `sf` (the instruction's bit 31) is set, else `W`.
    pub const fn from_sf(sf: bool) -> Width {
        if sf { Width::X } else { Width::W }
    }
}

/// A condition code (the `cond` field).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    /// Z set.
    Eq,
    /// Z clear.
    Ne,
    /// C set (unsigned higher or same).
    Cs,
    /// C clear (unsigned lower).
    Cc,
    /// N set.
    Mi,
    /// N clear.
    Pl,
    /// V set.
    Vs,
    /// V clear.
    Vc,
    /// C set and Z clear.
    Hi,
    /// C clear or Z set.
    Ls,
    /// N equals V.
    Ge,
    /// N differs from V.
    Lt,
    /// Z clear and N equals V.
    Gt,
    /// Z set or N differs from V.
    Le,
    /// Always.
    Al,
    /// Encoded as 1111; behaves as always (Arm ARM `ConditionHolds`).
    Nv,
}

impl Cond {
    /// The condition for the 4-bit field value (only the low 4 bits matter).
    pub const fn from_bits(b: u32) -> Cond {
        match b & 15 {
            0 => Cond::Eq,
            1 => Cond::Ne,
            2 => Cond::Cs,
            3 => Cond::Cc,
            4 => Cond::Mi,
            5 => Cond::Pl,
            6 => Cond::Vs,
            7 => Cond::Vc,
            8 => Cond::Hi,
            9 => Cond::Ls,
            10 => Cond::Ge,
            11 => Cond::Lt,
            12 => Cond::Gt,
            13 => Cond::Le,
            14 => Cond::Al,
            _ => Cond::Nv,
        }
    }

    /// The 4-bit field value.
    pub const fn bits(self) -> u32 {
        self as u32
    }

    /// The opposite condition (`cond` with its low bit flipped), as `CINC`
    /// and `CSET` use it.
    pub const fn invert(self) -> Cond {
        Cond::from_bits(self as u32 ^ 1)
    }
}

/// The shift applied to a shifted-register operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    /// Logical shift left.
    Lsl,
    /// Logical shift right.
    Lsr,
    /// Arithmetic shift right.
    Asr,
    /// Rotate right.
    Ror,
}

impl Shift {
    pub(crate) const fn from_bits(b: u32) -> Shift {
        match b & 3 {
            0 => Shift::Lsl,
            1 => Shift::Lsr,
            2 => Shift::Asr,
            _ => Shift::Ror,
        }
    }
}

/// The extension applied to an extended-register operand or a register
/// offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extend {
    /// Zero-extend the low byte.
    Uxtb,
    /// Zero-extend the low halfword.
    Uxth,
    /// Zero-extend the low word.
    Uxtw,
    /// Use all 64 bits (`LSL` when it is the shift of a register offset).
    Uxtx,
    /// Sign-extend the low byte.
    Sxtb,
    /// Sign-extend the low halfword.
    Sxth,
    /// Sign-extend the low word.
    Sxtw,
    /// Use all 64 bits (the `SXTX` spelling).
    Sxtx,
}

impl Extend {
    pub(crate) const fn from_bits(b: u32) -> Extend {
        match b & 7 {
            0 => Extend::Uxtb,
            1 => Extend::Uxth,
            2 => Extend::Uxtw,
            3 => Extend::Uxtx,
            4 => Extend::Sxtb,
            5 => Extend::Sxth,
            6 => Extend::Sxtw,
            _ => Extend::Sxtx,
        }
    }
}

/// Which logical operation (the `opc` field of the logical classes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogOp {
    /// `AND`.
    And,
    /// `ORR`.
    Orr,
    /// `EOR`.
    Eor,
    /// `ANDS` (sets flags N and Z, clears C and V).
    Ands,
}

/// Which move-wide operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovOp {
    /// `MOVN`: move the inverted shifted immediate.
    Movn,
    /// `MOVZ`: move the shifted immediate, zeroing the rest.
    Movz,
    /// `MOVK`: replace 16 bits and keep the rest.
    Movk,
}

/// Which bitfield instruction. `SBFM`, `BFM` and `UBFM` carry all their
/// aliases (`LSL`, `LSR`, `ASR`, `SXTB`, `UBFX`, `BFI`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BfOp {
    /// `SBFM`.
    Sbfm,
    /// `BFM`.
    Bfm,
    /// `UBFM`.
    Ubfm,
}

/// The second operand of a conditional compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpRhs {
    /// A register (`XZR` for 31).
    Reg(Reg),
    /// A 5-bit unsigned immediate.
    Imm(u8),
}

/// Which conditional select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CselOp {
    /// `CSEL`: `Rn` or `Rm`.
    Csel,
    /// `CSINC`: `Rn` or `Rm + 1`.
    Csinc,
    /// `CSINV`: `Rn` or `NOT Rm`.
    Csinv,
    /// `CSNEG`: `Rn` or `-Rm`.
    Csneg,
}

/// Which one-source data-processing operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bit1Op {
    /// `RBIT`: reverse the bits.
    Rbit,
    /// `REV16`: reverse the bytes in each halfword.
    Rev16,
    /// `REV32`: reverse the bytes in each word (the 32-bit `REV` is this
    /// with width `W`).
    Rev32,
    /// `REV` with 64-bit registers: reverse all 8 bytes.
    Rev64,
    /// `CLZ`.
    Clz,
    /// `CLS`.
    Cls,
}

/// Which barrier (the `CRm`/`op2` of the barrier group).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Barrier {
    /// `CLREX`: clear the exclusive monitor.
    Clrex,
    /// `DSB`.
    Dsb,
    /// `DMB`.
    Dmb,
    /// `ISB`.
    Isb,
    /// `SB`.
    Sb,
}

/// A system register as `MRS` and `MSR` name it: `op0:op1:CRn:CRm:op2` with
/// `op0` in bit 14 (only the values 2 and 3 are possible; bit 14 is `op0`
/// bit 0), as the 16-bit value Arm writes as `S<op0>_<op1>_C<n>_C<m>_<op2>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SysReg(pub u16);

impl SysReg {
    /// Build from the five fields. `op0` is 2 or 3.
    pub const fn new(op0: u16, op1: u16, crn: u16, crm: u16, op2: u16) -> SysReg {
        SysReg(((op0 & 1) << 14) | (op1 << 11) | (crn << 7) | (crm << 3) | op2)
    }
    /// `NZCV` (S3_3_C4_C2_0).
    pub const NZCV: SysReg = SysReg::new(3, 3, 4, 2, 0);
    /// `FPCR` (S3_3_C4_C4_0).
    pub const FPCR: SysReg = SysReg::new(3, 3, 4, 4, 0);
    /// `FPSR` (S3_3_C4_C4_1).
    pub const FPSR: SysReg = SysReg::new(3, 3, 4, 4, 1);
    /// `TPIDR_EL0` (S3_3_C13_C0_2).
    pub const TPIDR_EL0: SysReg = SysReg::new(3, 3, 13, 0, 2);
    /// `TPIDRRO_EL0` (S3_3_C13_C0_3).
    pub const TPIDRRO_EL0: SysReg = SysReg::new(3, 3, 13, 0, 3);
    /// `CNTVCT_EL0` (S3_3_C14_C0_2).
    pub const CNTVCT_EL0: SysReg = SysReg::new(3, 3, 14, 0, 2);
    /// `CNTFRQ_EL0` (S3_3_C14_C0_0).
    pub const CNTFRQ_EL0: SysReg = SysReg::new(3, 3, 14, 0, 0);
    /// `DCZID_EL0` (S3_3_C0_C0_7).
    pub const DCZID_EL0: SysReg = SysReg::new(3, 3, 0, 0, 7);
}

/// An addressing mode of a load or store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Addr {
    /// `[Xn, #offset]` (unsigned scaled, signed unscaled, or pair offset):
    /// the address is the base plus the byte offset, the base is unchanged.
    /// `rn` 31 is `SP`.
    Imm {
        /// Base register (31 is `SP`).
        rn: Reg,
        /// Byte offset, already scaled.
        offset: i64,
    },
    /// `[Xn, #offset]!`: base plus offset, which is also written back.
    Pre {
        /// Base register (31 is `SP`).
        rn: Reg,
        /// Byte offset, already scaled.
        offset: i64,
    },
    /// `[Xn], #offset`: the address is the base, then the base plus offset
    /// is written back.
    Post {
        /// Base register (31 is `SP`).
        rn: Reg,
        /// Byte offset, already scaled.
        offset: i64,
    },
    /// `[Xn, Rm{, extend {#shift}}]`. The offset register is extended, then
    /// shifted left by `shift` (0 or the access size's log2).
    Reg {
        /// Base register (31 is `SP`).
        rn: Reg,
        /// Offset register (31 is `XZR`).
        rm: Reg,
        /// Extension of `rm` (`Uxtx` is `LSL`).
        ext: Extend,
        /// Left shift applied after the extension.
        shift: u8,
    },
    /// PC-relative literal: the address is the instruction's address plus
    /// the byte offset.
    Literal {
        /// Signed byte offset from the instruction.
        offset: i64,
    },
}

/// What a single load or store does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemKind {
    /// Store `Rt` (or the SIMD register).
    Store,
    /// Load and zero-extend.
    Load,
    /// Load and sign-extend into a `W` or `X` register.
    LoadSigned(Width),
    /// `PRFM`: a hint, no effect here.
    Prefetch,
}

/// What a load or store pair does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairKind {
    /// `STP`.
    Store,
    /// `LDP`.
    Load,
    /// `LDPSW`: load two words, sign-extend to 64 bits.
    LoadSw,
}

/// Which exclusive access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExclKind {
    /// `LDXR`, `LDAXR`.
    Load,
    /// `STXR`, `STLXR`.
    Store,
    /// `LDXP`, `LDAXP`.
    LoadPair,
    /// `STXP`, `STLXP`.
    StorePair,
}

/// Which LSE atomic read-modify-write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtomOp {
    /// `LDADD`.
    Add,
    /// `LDCLR`: `mem AND NOT Rs`.
    Clr,
    /// `LDEOR`.
    Eor,
    /// `LDSET`: `mem OR Rs`.
    Set,
    /// `LDSMAX`.
    Smax,
    /// `LDSMIN`.
    Smin,
    /// `LDUMAX`.
    Umax,
    /// `LDUMIN`.
    Umin,
    /// `SWP`.
    Swp,
}

/// The flag-manipulation instructions without operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagOp {
    /// `CFINV`: invert C.
    Cfinv,
    /// `AXFLAG`: Arm floating-point flags to the "external" format.
    Axflag,
    /// `XAFLAG`: the reverse of `AXFLAG`.
    Xaflag,
}

/// A floating-point format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpType {
    /// Half precision (16 bits).
    H,
    /// Single precision (32 bits).
    S,
    /// Double precision (64 bits).
    D,
}

/// Two-operand scalar floating-point operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fp2Op {
    /// `FMUL`.
    Mul,
    /// `FDIV`.
    Div,
    /// `FADD`.
    Add,
    /// `FSUB`.
    Sub,
    /// `FMAX`.
    Max,
    /// `FMIN`.
    Min,
    /// `FMAXNM`.
    MaxNm,
    /// `FMINNM`.
    MinNm,
    /// `FNMUL`.
    Nmul,
}

/// One-operand scalar floating-point operations (not conversions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fp1Op {
    /// `FMOV` register.
    Mov,
    /// `FABS`.
    Abs,
    /// `FNEG`.
    Neg,
    /// `FSQRT`.
    Sqrt,
    /// `FRINTN`: round to nearest, ties to even.
    RintN,
    /// `FRINTP`: round towards +infinity.
    RintP,
    /// `FRINTM`: round towards -infinity.
    RintM,
    /// `FRINTZ`: round towards zero.
    RintZ,
    /// `FRINTA`: round to nearest, ties away from zero.
    RintA,
    /// `FRINTX`: round using FPCR (to nearest even here), signalling inexact.
    RintX,
    /// `FRINTI`: round using FPCR (to nearest even here).
    RintI,
}

/// The rounding of a floating-point to integer conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpRound {
    /// To nearest, ties to even (`FCVTNS`/`FCVTNU`).
    NearestEven,
    /// Towards +infinity (`FCVTPS`/`FCVTPU`).
    PosInf,
    /// Towards -infinity (`FCVTMS`/`FCVTMU`).
    NegInf,
    /// Towards zero (`FCVTZS`/`FCVTZU`).
    Zero,
    /// To nearest, ties away from zero (`FCVTAS`/`FCVTAU`).
    Away,
}

/// Which three-register same-type vector (or scalar) operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V3Op {
    /// `ADD`.
    Add,
    /// `SUB`.
    Sub,
    /// `AND`.
    And,
    /// `BIC`.
    Bic,
    /// `ORR`.
    Orr,
    /// `ORN`.
    Orn,
    /// `EOR`.
    Eor,
    /// `BSL`: `Vd` selects between `Vn` and `Vm`.
    Bsl,
    /// `BIT`: insert `Vn` bits where `Vm` is 1.
    Bit,
    /// `BIF`: insert `Vn` bits where `Vm` is 0.
    Bif,
    /// `CMEQ` (register).
    Cmeq,
    /// `CMHI`: unsigned higher.
    Cmhi,
    /// `CMHS`: unsigned higher or same.
    Cmhs,
    /// `CMGT`: signed greater.
    Cmgt,
    /// `CMGE`: signed greater or equal.
    Cmge,
    /// `CMTST`: test bits.
    Cmtst,
    /// `MUL` (not for 64-bit elements).
    Mul,
    /// `MLA`.
    Mla,
    /// `MLS`.
    Mls,
    /// `SMAX`.
    Smax,
    /// `SMIN`.
    Smin,
    /// `UMAX`.
    Umax,
    /// `UMIN`.
    Umin,
    /// `ADDP`: add adjacent pairs of `Vn:Vm`.
    Addp,
    /// `SSHL`: shift left by a signed per-lane amount (negative shifts right).
    Sshl,
    /// `USHL`.
    Ushl,
    /// `SHADD`/`UHADD` (`u` in the name: the unsigned form of each pair below).
    Shadd,
    /// `UHADD`.
    Uhadd,
    /// `SRHADD`.
    Srhadd,
    /// `URHADD`.
    Urhadd,
    /// `SHSUB`.
    Shsub,
    /// `UHSUB`.
    Uhsub,
    /// `SQADD`.
    Sqadd,
    /// `UQADD`.
    Uqadd,
    /// `SQSUB`.
    Sqsub,
    /// `UQSUB`.
    Uqsub,
    /// `SQSHL` (register).
    Sqshl,
    /// `UQSHL` (register).
    Uqshl,
    /// `SRSHL`.
    Srshl,
    /// `URSHL`.
    Urshl,
    /// `SQRSHL`.
    Sqrshl,
    /// `UQRSHL`.
    Uqrshl,
    /// `SABD`.
    Sabd,
    /// `UABD`.
    Uabd,
    /// `SABA`.
    Saba,
    /// `UABA`.
    Uaba,
    /// `SMAXP`.
    Smaxp,
    /// `SMINP`.
    Sminp,
    /// `UMAXP`.
    Umaxp,
    /// `UMINP`.
    Uminp,
    /// `SQDMULH`.
    Sqdmulh,
    /// `SQRDMULH`.
    Sqrdmulh,
    /// `PMUL` (polynomial, bytes).
    Pmul,
    /// `SQRDMLAH` (three same extra).
    Sqrdmlah,
    /// `SQRDMLSH`.
    Sqrdmlsh,
}

/// Floating-point three-register vector (or scalar) operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VFp3Op {
    /// `FADD`.
    Add,
    /// `FSUB`.
    Sub,
    /// `FMUL`.
    Mul,
    /// `FDIV`.
    Div,
    /// `FMAX`.
    Max,
    /// `FMIN`.
    Min,
    /// `FMAXNM`.
    MaxNm,
    /// `FMINNM`.
    MinNm,
    /// `FMLA` (fused).
    Mla,
    /// `FMLS` (fused).
    Mls,
    /// `FABD`.
    Abd,
    /// `FMULX`.
    Mulx,
    /// `FCMEQ`.
    Cmeq,
    /// `FCMGE`.
    Cmge,
    /// `FCMGT`.
    Cmgt,
    /// `FACGE`.
    Acge,
    /// `FACGT`.
    Acgt,
    /// `FADDP`.
    Addp,
    /// `FMAXP`.
    Maxp,
    /// `FMINP`.
    Minp,
    /// `FMAXNMP`.
    MaxNmp,
    /// `FMINNMP`.
    MinNmp,
    /// `FRECPS`.
    Recps,
    /// `FRSQRTS`.
    Rsqrts,
}

/// Floating-point two-register vector (or scalar) operations, including
/// conversions and estimates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VFp2Op {
    /// `FABS`.
    Abs,
    /// `FNEG`.
    Neg,
    /// `FSQRT`.
    Sqrt,
    /// `FRINT*` with this rounding (`exact` is `FRINTX`).
    Rint(FpRound),
    /// `FRINTX` (rounds to nearest even here) and `FRINTI`.
    RintX,
    /// `FCVT[NPMZA]S`/`U` to integers of the element size.
    ToInt(FpRound, bool),
    /// `SCVTF`/`UCVTF` (`true` = unsigned) from integers of the element size.
    FromInt(bool),
    /// `FCMGT`/`FCMGE`/`FCMEQ`/`FCMLE`/`FCMLT` with zero.
    CmZero(CmpZero),
    /// `FRECPE`.
    Recpe,
    /// `FRSQRTE`.
    Rsqrte,
    /// `URECPE` (32-bit integer lanes).
    Urecpe,
    /// `URSQRTE`.
    Ursqrte,
    /// `FCVTL`/`FCVTL2`: widen (from the lower or, with `Q`, upper half).
    Fcvtl,
    /// `FCVTN`/`FCVTN2`: narrow into the lower or upper half.
    Fcvtn,
}

/// The SHA3 extension's operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sha3Op {
    /// `EOR3`: three-way exclusive or.
    Eor3,
    /// `BCAX`: `Vn EOR (Vm AND NOT Va)`.
    Bcax,
    /// `RAX1`: `Vn EOR ROL(Vm, 1)` on 64-bit lanes.
    Rax1,
    /// `XAR`: `ROR(Vn EOR Vm, imm)` on 64-bit lanes.
    Xar,
}

/// The comparison of an `FCM*` with zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpZero {
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `==`.
    Eq,
    /// `<=`.
    Le,
    /// `<`.
    Lt,
}

/// Floating-point across-lanes reductions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FAcrossOp {
    /// `FMAXNMV`.
    MaxNmv,
    /// `FMINNMV`.
    MinNmv,
    /// `FMAXV`.
    Maxv,
    /// `FMINV`.
    Minv,
}

/// Three registers of different types (long, wide, narrow).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffOp {
    /// `SADDL`/`UADDL`.
    Addl,
    /// `SADDW`/`UADDW`.
    Addw,
    /// `SSUBL`/`USUBL`.
    Subl,
    /// `SSUBW`/`USUBW`.
    Subw,
    /// `ADDHN` (`RADDHN` with `round`).
    Addhn,
    /// `SUBHN`/`RSUBHN`.
    Subhn,
    /// `SABAL`/`UABAL`.
    Abal,
    /// `SABDL`/`UABDL`.
    Abdl,
    /// `SMLAL`/`UMLAL`.
    Mlal,
    /// `SMLSL`/`UMLSL`.
    Mlsl,
    /// `SMULL`/`UMULL`.
    Mull,
    /// `SQDMLAL`.
    Sqdmlal,
    /// `SQDMLSL`.
    Sqdmlsl,
    /// `SQDMULL`.
    Sqdmull,
    /// `PMULL` (8-bit or 64-bit polynomial).
    Pmull,
}

/// Vector by-element operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElemOp {
    /// `MUL`.
    Mul,
    /// `MLA`.
    Mla,
    /// `MLS`.
    Mls,
    /// `SQDMULH`.
    Sqdmulh,
    /// `SQRDMULH`.
    Sqrdmulh,
    /// `SQRDMLAH`.
    Sqrdmlah,
    /// `SQRDMLSH`.
    Sqrdmlsh,
    /// A long form: `SMULL`, `UMLAL`, `SQDMULL`, ... (the `DiffOp` and signedness).
    Long(DiffOp, bool),
    /// `SDOT`/`UDOT` (`true` = unsigned).
    Dot(bool),
    /// `FMUL`.
    Fmul,
    /// `FMLA`.
    Fmla,
    /// `FMLS`.
    Fmls,
    /// `FMULX`.
    Fmulx,
}

/// More two-register integer operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Misc2Op {
    /// `SADDLP`/`UADDLP` (`u`).
    Addlp(bool),
    /// `SADALP`/`UADALP`.
    Adalp(bool),
    /// `SQABS`.
    Sqabs,
    /// `SQNEG`.
    Sqneg,
    /// `SUQADD`.
    Suqadd,
    /// `USQADD`.
    Usqadd,
    /// `SQXTN`.
    Sqxtn,
    /// `UQXTN`.
    Uqxtn,
    /// `SQXTUN`.
    Sqxtun,
    /// `SHLL`: widen and shift by the element size.
    Shll,
}

/// More shifts by immediate (rounding, saturating, fixed point).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift2Op {
    /// `SRSHR`/`URSHR` (`u`).
    Rshr(bool),
    /// `SRSRA`/`URSRA`.
    Rsra(bool),
    /// `RSHRN`.
    Rshrn,
    /// `SQSHL`/`UQSHL` (immediate).
    Qshl(bool),
    /// `SQSHLU`.
    Sqshlu,
    /// `SQSHRN`/`UQSHRN`, `SQRSHRN`/`UQRSHRN` (`round`).
    Qshrn { unsigned: bool, round: bool },
    /// `SQSHRUN`/`SQRSHRUN`.
    Qshrun { round: bool },
    /// `SCVTF`/`UCVTF` with fraction bits.
    CvtFrom(bool),
    /// `FCVTZS`/`FCVTZU` with fraction bits.
    CvtTo(bool),
}

/// Which two-register miscellaneous vector (or scalar) operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V2Op {
    /// `CMEQ` with zero.
    CmeqZ,
    /// `CMGT` with zero.
    CmgtZ,
    /// `CMGE` with zero.
    CmgeZ,
    /// `CMLE` with zero.
    CmleZ,
    /// `CMLT` with zero.
    CmltZ,
    /// `NOT` (a.k.a. `MVN`).
    Not,
    /// `NEG`.
    Neg,
    /// `ABS`.
    Abs,
    /// `CNT`: population count per byte.
    Cnt,
    /// `REV64`.
    Rev64,
    /// `REV32`.
    Rev32,
    /// `REV16`.
    Rev16,
    /// `XTN`: narrow to the lower half of the result.
    Xtn,
    /// `XTN2`: narrow into the upper half of the result.
    Xtn2,
    /// `RBIT` (byte-wise bit reverse).
    Rbit,
    /// `CLZ` per element.
    Clz,
    /// `CLS` per element.
    Cls,
}

/// Which across-lanes reduction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcrossOp {
    /// `ADDV`.
    Addv,
    /// `UMAXV`.
    Umaxv,
    /// `UMINV`.
    Uminv,
    /// `SMAXV`.
    Smaxv,
    /// `SMINV`.
    Sminv,
    /// `UADDLV`: unsigned sum into a wider element.
    Uaddlv,
    /// `SADDLV`: signed sum into a wider element.
    Saddlv,
}

/// Which vector permute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermOp {
    /// `UZP1`.
    Uzp1,
    /// `UZP2`.
    Uzp2,
    /// `ZIP1`.
    Zip1,
    /// `ZIP2`.
    Zip2,
    /// `TRN1`.
    Trn1,
    /// `TRN2`.
    Trn2,
}

/// Which vector shift by immediate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShiftImmOp {
    /// `SHL`.
    Shl,
    /// `USHR`.
    Ushr,
    /// `SSHR`.
    Sshr,
    /// `USRA`.
    Usra,
    /// `SSRA`.
    Ssra,
    /// `SLI`.
    Sli,
    /// `SRI`.
    Sri,
    /// `SHRN`/`SHRN2`: narrow with a right shift.
    Shrn,
    /// `USHLL`/`USHLL2` (`UXTL` when the shift is 0).
    Ushll,
    /// `SSHLL`/`SSHLL2` (`SXTL` when the shift is 0).
    Sshll,
}

/// How a SIMD structure load/store changes its base register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostIdx {
    /// No writeback.
    None,
    /// Post-index by the immediate (the number of bytes transferred).
    Imm(u8),
    /// Post-index by a register.
    Reg(Reg),
}

/// A decoded A64 instruction.
///
/// See the [module docs](self) for the conventions. Variants are grouped by
/// instruction class in the order of the Arm ARM's encoding chapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inst {
    /// An unallocated or reserved encoding, including `UDF`. Executing it is
    /// an undefined-instruction exception.
    Undefined(u32),
    /// An allocated encoding this crate does not implement (SVE, crypto,
    /// pointer-authentication register forms, most of Advanced SIMD beyond
    /// the documented subset, ...).
    Unsupported(u32),

    // ---- data processing, immediate and register ----
    /// `ADD`/`ADDS`/`SUB`/`SUBS` (immediate). `rn` is `SP` for 31; `rd` is
    /// `SP` for 31 unless `s` (then `XZR`). `imm` is already shifted.
    AddSubImm { w: Width, sub: bool, s: bool, rd: Reg, rn: Reg, imm: u32 },
    /// `ADD`/`SUB` (shifted register), optionally setting flags. All
    /// register 31s are `XZR`.
    AddSubShifted { w: Width, sub: bool, s: bool, rd: Reg, rn: Reg, rm: Reg, shift: Shift, amount: u8 },
    /// `ADD`/`SUB` (extended register). `rn` is `SP` for 31, `rd` is `SP`
    /// for 31 unless `s`, `rm` is `XZR`.
    AddSubExt { w: Width, sub: bool, s: bool, rd: Reg, rn: Reg, rm: Reg, ext: Extend, amount: u8 },
    /// `ADC`/`ADCS`/`SBC`/`SBCS`.
    AddSubCarry { w: Width, sub: bool, s: bool, rd: Reg, rn: Reg, rm: Reg },
    /// `AND`/`ORR`/`EOR`/`ANDS` (immediate). `imm` is the decoded bitmask,
    /// for `W` already truncated to 32 bits. `rd` is `SP` for 31 unless
    /// `Ands`.
    LogicalImm { w: Width, op: LogOp, rd: Reg, rn: Reg, imm: u64 },
    /// `AND`/`ORR`/`EOR`/`ANDS` and `BIC`/`ORN`/`EON`/`BICS` (shifted
    /// register); `invert` is the `N` bit.
    LogicalShifted { w: Width, op: LogOp, invert: bool, rd: Reg, rn: Reg, rm: Reg, shift: Shift, amount: u8 },
    /// `MOVN`/`MOVZ`/`MOVK`. `shift` is 0, 16, 32 or 48.
    MoveWide { w: Width, op: MovOp, rd: Reg, imm: u16, shift: u8 },
    /// `ADR`/`ADRP`. `offset` is in bytes from the instruction's address (for
    /// `ADRP`, from its 4 KiB page; the offset already has the `<< 12`).
    Adr { rd: Reg, offset: i64, page: bool },
    /// `SBFM`/`BFM`/`UBFM` with the Arm ARM's `DecodeBitMasks` results:
    /// `wmask` and `tmask` are already truncated to the width.
    Bitfield { w: Width, op: BfOp, rd: Reg, rn: Reg, immr: u8, imms: u8, wmask: u64, tmask: u64 },
    /// `EXTR` (`ROR` immediate when `rn == rm`).
    Extr { w: Width, rd: Reg, rn: Reg, rm: Reg, lsb: u8 },
    /// `CCMP`/`CCMN` (register or immediate); `neg` is `CCMN`.
    CondCmp { w: Width, neg: bool, rn: Reg, rhs: CmpRhs, cond: Cond, nzcv: u8 },
    /// `CSEL`/`CSINC`/`CSINV`/`CSNEG`.
    CondSel { w: Width, op: CselOp, rd: Reg, rn: Reg, rm: Reg, cond: Cond },
    /// `MADD`/`MSUB` (`MUL` and `MNEG` are the aliases with `ra` 31).
    Madd { w: Width, sub: bool, rd: Reg, rn: Reg, rm: Reg, ra: Reg },
    /// `SMADDL`/`SMSUBL`/`UMADDL`/`UMSUBL`: 32 x 32 to 64 bits plus `ra`.
    MaddLong { signed: bool, sub: bool, rd: Reg, rn: Reg, rm: Reg, ra: Reg },
    /// `SMULH`/`UMULH`: the high 64 bits of the 128-bit product.
    MulHigh { signed: bool, rd: Reg, rn: Reg, rm: Reg },
    /// `UDIV`/`SDIV`.
    Div { w: Width, signed: bool, rd: Reg, rn: Reg, rm: Reg },
    /// `LSLV`/`LSRV`/`ASRV`/`RORV`.
    ShiftVar { w: Width, op: Shift, rd: Reg, rn: Reg, rm: Reg },
    /// `RBIT`/`REV*`/`CLZ`/`CLS`.
    Bit1 { w: Width, op: Bit1Op, rd: Reg, rn: Reg },

    /// `CRC32B/H/W/X` and `CRC32CB/H/W/X` (`c`): `size` is log2 of the
    /// bytes taken from `rm` (3 only with an `X` register).
    Crc32 { c: bool, size: u8, rd: Reg, rn: Reg, rm: Reg },
    /// `PACIA`/`PACIB`/`PACDA`/`PACDB`/`AUTIA`/... and their `Z` forms
    /// (`modifier` `None`) on `rd`. `modifier` 31 is `SP`. With the keys
    /// disabled (what an arm64 process on macOS gets) they leave `rd`
    /// unchanged; see [`crate::cpu`].
    Pac { auth: bool, data: bool, key_b: bool, rd: Reg, modifier: Option<Reg> },
    /// `XPACI`/`XPACD`: strip the authentication code from `rd`.
    Xpac { data: bool, rd: Reg },
    /// `PACGA Xd, Xn, Xm|SP`: a 32-bit code of `rn` with modifier `rm` in
    /// the top half of `rd`.
    Pacga { rd: Reg, rn: Reg, rm: Reg },
    /// `RMIF Xn, #shift, #mask` (FEAT_FlagM).
    Rmif { rn: Reg, shift: u8, mask: u8 },
    /// `SETF8`/`SETF16 Wn` (FEAT_FlagM).
    Setf { rn: Reg, sixteen: bool },
    /// `CFINV`, `AXFLAG`, `XAFLAG` (FEAT_FlagM, FEAT_FlagM2).
    FlagOp { op: FlagOp },

    // ---- branches ----
    /// `B`/`BL`: `offset` is a byte offset from this instruction.
    Branch { link: bool, offset: i64 },
    /// `B.cond`.
    BranchCond { cond: Cond, offset: i64 },
    /// `CBZ`/`CBNZ` (`nz`).
    CompareBranch { w: Width, nz: bool, rt: Reg, offset: i64 },
    /// `TBZ`/`TBNZ` (`nz`): test bit `bit` (0 to 63) of `rt`.
    TestBranch { nz: bool, rt: Reg, bit: u8, offset: i64 },
    /// `BR`.
    BranchReg { rn: Reg },
    /// `BLR`.
    BranchLinkReg { rn: Reg },
    /// `RET`.
    Ret { rn: Reg },
    /// `BRAA`/`BRAB`/`BLRAA`/`BLRAB`/`RETAA`/`RETAB` and the `Z` forms:
    /// a branch to `rn` (`X30` for `ret`) after authenticating it with
    /// `modifier` (`None` for the `Z` forms, 31 is `SP` for `RETA*`).
    BranchAuth { link: bool, ret: bool, key_b: bool, rn: Reg, modifier: Option<Reg> },

    // ---- system ----
    /// `SVC #imm`: a system call (the caller handles it).
    Svc(u16),
    /// `BRK #imm`: a breakpoint.
    Brk(u16),
    /// A hint (`NOP`, `YIELD`, `WFE`, `WFI`, `SEV`, `BTI`, `PACIASP`, ...):
    /// `crm_op2` is `CRm << 3 | op2`. All hints are no-ops here.
    Hint { crm_op2: u8 },
    /// `DMB`/`DSB`/`ISB`/`SB`/`CLREX`; `crm` is the option field.
    Barrier { kind: Barrier, crm: u8 },
    /// `MRS Xt, <sysreg>`.
    Mrs { rt: Reg, reg: SysReg },
    /// `MSR <sysreg>, Xt`.
    Msr { rt: Reg, reg: SysReg },
    /// `SYS #op1, Cn, Cm, #op2, Xt` (the `DC`, `IC`, `TLBI`, `AT` family).
    Sys { op1: u8, crn: u8, crm: u8, op2: u8, rt: Reg },

    // ---- loads and stores ----
    /// `LDR`/`STR`/`LDRS*`/`PRFM` and the unscaled, unprivileged and SIMD
    /// forms. `size` is log2 of the access size in bytes: 0 to 3, or 4 for a
    /// 128-bit `Q` register (SIMD only). `fp` selects the SIMD&FP register
    /// file for `rt`. For integer accesses `rt` 31 is `XZR`; the base is
    /// `SP` for 31 (see [`Addr`]).
    LoadStore { kind: MemKind, size: u8, fp: bool, rt: Reg, addr: Addr },
    /// `LDP`/`STP`/`LDPSW`/`LDNP`/`STNP` (integer or SIMD). `size` is log2 of
    /// the size of each register: 2, 3, or 4 (SIMD only).
    LoadStorePair { kind: PairKind, size: u8, fp: bool, rt: Reg, rt2: Reg, addr: Addr },
    /// `LDXR`/`STXR`/`LDXP`/`STXP` and the acquire/release forms. `rs` is the
    /// status register of the stores; `rt2` is used by pairs. `size` is log2
    /// bytes (0 to 3; pairs 2 or 3). `ordered` is whether it has acquire or
    /// release semantics (no effect here).
    Exclusive { kind: ExclKind, size: u8, ordered: bool, rs: Reg, rt: Reg, rt2: Reg, rn: Reg },
    /// `LDAR`/`STLR` (and `LDLAR`/`STLLR`, `LDAPR`): ordered, aligned
    /// accesses. `store` is the direction.
    Ordered { store: bool, size: u8, rt: Reg, rn: Reg },
    /// `LDAPUR*`/`STLUR` (FEAT_LRCPC2): ordered accesses at a signed
    /// unscaled offset. `kind` is `Store`, `Load` or `LoadSigned`.
    OrderedImm { kind: MemKind, size: u8, rt: Reg, rn: Reg, offset: i64 },
    /// `LDRAA`/`LDRAB`: load 64 bits from `rn` (authenticated as a data
    /// address) plus `offset`; `writeback` is the pre-indexed form.
    LoadAuth { key_b: bool, rt: Reg, rn: Reg, offset: i64, writeback: bool },
    /// LSE atomics `LDADD`, `LDCLR`, `LDEOR`, `LDSET`, `LDSMAX`, `LDSMIN`,
    /// `LDUMAX`, `LDUMIN`, `SWP` (and the `ST*` aliases when `rt` is 31): the
    /// old memory value goes to `rt` (unless `rt` is 31), the new value is
    /// computed from `rs`. `acq` and `rel` are the ordering bits.
    Atomic { op: AtomOp, size: u8, acq: bool, rel: bool, rs: Reg, rt: Reg, rn: Reg },
    /// `CAS`: compare `rs` with memory, store `rt` if equal, `rs` gets the
    /// old value.
    Cas { size: u8, acq: bool, rel: bool, rs: Reg, rt: Reg, rn: Reg },
    /// `CASP`: as `CAS` on register pairs (`rs`/`rs+1`, `rt`/`rt+1`);
    /// `size` is 2 (two words) or 3 (two doublewords).
    Casp { size: u8, acq: bool, rel: bool, rs: Reg, rt: Reg, rn: Reg },

    // ---- scalar floating point ----
    /// `FABS`/`FNEG`/`FSQRT`/`FMOV` (register)/`FRINT*`.
    FpUnary { op: Fp1Op, ft: FpType, rd: Reg, rn: Reg },
    /// `FCVT` between half, single and double.
    FpCvt { from: FpType, to: FpType, rd: Reg, rn: Reg },
    /// `FMUL`/`FDIV`/`FADD`/`FSUB`/`FMAX`/`FMIN`/`FMAXNM`/`FMINNM`/`FNMUL`.
    FpBinary { op: Fp2Op, ft: FpType, rd: Reg, rn: Reg, rm: Reg },
    /// `FMADD`/`FMSUB`/`FNMADD`/`FNMSUB`: `(neg_a ? -ra : ra) + (neg_p ? -(rn * rm) : rn * rm)`,
    /// fused (one rounding).
    FpFused { ft: FpType, neg_p: bool, neg_a: bool, rd: Reg, rn: Reg, rm: Reg, ra: Reg },
    /// `FCMP`/`FCMPE`: `zero` compares with +0.0 instead of `rm`.
    FpCmp { ft: FpType, signal: bool, zero: bool, rn: Reg, rm: Reg },
    /// `FCCMP`/`FCCMPE`.
    FpCondCmp { ft: FpType, signal: bool, rn: Reg, rm: Reg, cond: Cond, nzcv: u8 },
    /// `FCSEL`.
    FpCondSel { ft: FpType, rd: Reg, rn: Reg, rm: Reg, cond: Cond },
    /// `FMOV` (scalar, immediate): `bits` is the expanded raw bit pattern of
    /// the constant in the format.
    FpMovImm { ft: FpType, rd: Reg, bits: u64 },
    /// `FCVT[NPMZA][SU]` to an integer register; `fbits` is the number of
    /// fraction bits of the fixed-point forms (0 for plain conversion).
    FpToInt { ft: FpType, w: Width, unsigned: bool, round: FpRound, rd: Reg, rn: Reg, fbits: u8 },
    /// `SCVTF`/`UCVTF` from an integer register; `fbits` as above.
    IntToFp { ft: FpType, w: Width, unsigned: bool, rd: Reg, rn: Reg, fbits: u8 },
    /// `FMOV Wd, Sn` / `FMOV Xd, Dn` / `FMOV Xd, Vn.D[1]` (`top`).
    FmovToGen { w: Width, top: bool, rd: Reg, rn: Reg },
    /// `FMOV Sd, Wn` / `FMOV Dd, Xn` / `FMOV Vd.D[1], Xn` (`top`).
    FmovFromGen { w: Width, top: bool, rd: Reg, rn: Reg },

    // ---- Advanced SIMD ----
    /// Three-register same-type operations. `size` is log2 of the element
    /// size in bytes (0 to 3); the logical operations ignore it. `scalar`
    /// is the scalar form (`ADD Dd, Dn, Dm`; one 64-bit element).
    Vec3Same { op: V3Op, q: bool, size: u8, scalar: bool, rd: Reg, rn: Reg, rm: Reg },
    /// Two-register miscellaneous operations.
    Vec2Misc { op: V2Op, q: bool, size: u8, scalar: bool, rd: Reg, rn: Reg },
    /// `ADDV`/`UMAXV`/`UMINV`/`SMAXV`/`SMINV`/`UADDLV`/`SADDLV`.
    VecAcross { op: AcrossOp, q: bool, size: u8, rd: Reg, rn: Reg },
    /// `DUP Vd.T, Vn.Ts[index]` (or the scalar `DUP Sd, Vn.S[i]`/`MOV`).
    DupElem { q: bool, size: u8, scalar: bool, rd: Reg, rn: Reg, index: u8 },
    /// `DUP Vd.T, Rn`.
    DupGen { q: bool, size: u8, rd: Reg, rn: Reg },
    /// `INS Vd.Ts[di], Vn.Ts[si]`.
    InsElem { size: u8, rd: Reg, rn: Reg, di: u8, si: u8 },
    /// `INS Vd.Ts[index], Rn` (`MOV`).
    InsGen { size: u8, rd: Reg, rn: Reg, index: u8 },
    /// `UMOV`/`SMOV Rd, Vn.Ts[index]`; `w` is the destination register
    /// width.
    MovToGen { signed: bool, w: Width, size: u8, rd: Reg, rn: Reg, index: u8 },
    /// `MOVI`/`MVNI`/`FMOV` (vector, immediate): the whole 64-bit pattern,
    /// copied into both halves when `q`.
    Movi { q: bool, rd: Reg, imm: u64 },
    /// `ORR`/`BIC` (vector, immediate): `imm` is the 64-bit pattern, as for
    /// `Movi`; `bic` clears those bits, otherwise they are set.
    ModImmLogic { bic: bool, q: bool, rd: Reg, imm: u64 },
    /// `EXT Vd.T, Vn.T, Vm.T, #index` (index in bytes).
    VecExt { q: bool, rd: Reg, rn: Reg, rm: Reg, index: u8 },
    /// `TBL`/`TBX` with `len` (1 to 4) table registers starting at `rn`.
    VecTbl { q: bool, len: u8, ext: bool, rd: Reg, rn: Reg, rm: Reg },
    /// `UZP`/`ZIP`/`TRN`.
    VecPerm { op: PermOp, q: bool, size: u8, rd: Reg, rn: Reg, rm: Reg },
    /// Shifts by immediate. `size` is log2 bytes of the (source) element
    /// and `shift` the shift amount; `second` is the `2` form of the
    /// narrowing and widening ones (use the upper half). `scalar` is the
    /// 64-bit scalar form.
    VecShiftImm { op: ShiftImmOp, q: bool, size: u8, scalar: bool, shift: u8, rd: Reg, rn: Reg },
    /// Floating-point three-register operations (`ft` S or D; `scalar`:
    /// one element).
    VecFp3 { op: VFp3Op, q: bool, ft: FpType, scalar: bool, rd: Reg, rn: Reg, rm: Reg },
    /// Floating-point two-register operations. For `Fcvtl`/`Fcvtn`, `ft` is
    /// the wider format.
    VecFp2 { op: VFp2Op, q: bool, ft: FpType, scalar: bool, rd: Reg, rn: Reg },
    /// `FMAXNMV`/`FMINNMV`/`FMAXV`/`FMINV` (single precision, four lanes).
    VecFpAcross { op: FAcrossOp, rd: Reg, rn: Reg },
    /// Scalar pairwise: `ADDP Dd, Vn.2D` (`ft` `None`) or the `F*P` forms on the two
    /// lanes of a 64-bit (S) or 128-bit (D) vector.
    ScalarPair { op: Option<VFp3Op>, ft: FpType, rd: Reg, rn: Reg },
    /// Three registers of different types. `size` is the narrow element
    /// size (log2 bytes); `q` selects the upper halves (the `2` forms).
    VecDiff { op: DiffOp, unsigned: bool, round: bool, q: bool, size: u8, rd: Reg, rn: Reg, rm: Reg },
    /// By-element operations: `size` is the element size (log2 bytes; for
    /// the long forms the narrow one), `rm` and `index` the element.
    VecElem { op: ElemOp, q: bool, size: u8, scalar: bool, rd: Reg, rn: Reg, rm: Reg, index: u8 },
    /// More two-register integer operations; `size` as in `Vec2Misc` (for
    /// the narrowing ones the narrow size).
    VecMisc2 { op: Misc2Op, q: bool, size: u8, scalar: bool, rd: Reg, rn: Reg },
    /// More shifts by immediate; `size` and `shift` as in `VecShiftImm`.
    VecShift2 { op: Shift2Op, q: bool, size: u8, scalar: bool, shift: u8, rd: Reg, rn: Reg },
    /// `EOR3`/`BCAX`/`RAX1`/`XAR` (FEAT_SHA3).
    Sha3 { op: Sha3Op, rd: Reg, rn: Reg, rm: Reg, ra: Reg, imm: u8 },
    /// `LD1`..`LD4`/`ST1`..`ST4` (multiple structures). `regs` is the number
    /// of registers (1 to 4); `interleave` is true for `LD2`..`LD4`.
    VecLdstMulti { load: bool, q: bool, size: u8, regs: u8, interleave: bool, rt: Reg, rn: Reg, post: PostIdx },
    /// `LD1`..`LD4`/`ST1`..`ST4` (single structure, one lane) and
    /// `LD1R`..`LD4R` (`replicate`). `selem` is the structure size (1 to 4).
    VecLdstSingle {
        load: bool,
        replicate: bool,
        q: bool,
        size: u8,
        selem: u8,
        index: u8,
        rt: Reg,
        rn: Reg,
        post: PostIdx,
    },
}
