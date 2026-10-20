//! Classic BPF: checking and running filter programs, as seccomp(2) uses
//! them (`SECCOMP_SET_MODE_FILTER`).
//!
//! A program is a list of [`Insn`]s (`struct sock_filter` of the UAPI
//! header `linux/filter.h`: a 16-bit opcode, two 8-bit jump offsets and a
//! 32-bit constant). The machine has an accumulator `A`, an index register
//! `X` and [`MEMWORDS`] words of scratch memory, all starting at 0; the
//! opcodes are the classes, sizes, modes, operations and sources of
//! `linux/bpf_common.h` and `linux/filter.h` ([`code`]).
//!
//! [`Program::new`] checks a program before it may run, and refuses
//! (with an [`Error`] naming the instruction) anything that could fail or
//! loop at run time, the rules seccomp(2) describes for its filters:
//!
//! - between 1 and [`MAXINSNS`] instructions;
//! - every opcode one of the classic set; loads only from the input as
//!   aligned 32-bit words inside it (seccomp: `struct seccomp_data`), its
//!   length, constants or scratch memory; no byte, half-word, indirect or
//!   `MSH` loads, which only make sense for packets;
//! - jumps only forward (the offsets are unsigned) and only to an
//!   instruction of the program, so every run ends after at most as many
//!   steps as there are instructions;
//! - the last instruction a return, so no run falls off the end;
//! - no division or remainder by a constant 0 and no shift by a constant
//!   of 32 or more (Poly's reading: the result is not defined).
//!
//! At run time a division or remainder by `X` = 0 ends the program with
//! the value 0, the classic BPF rule, and a shift by `X` of 32 or more
//! gives 0. Scratch memory starts zeroed, so reading a word no
//! instruction stored is defined (0) rather than refused.
//!
//! The input is a slice of 32-bit words in the host's byte order, as
//! seccomp loads `struct seccomp_data`: a load at byte offset `k` reads
//! word `k / 4`.

#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;

/// `BPF_MAXINSNS` (`linux/bpf_common.h`): the longest program.
pub const MAXINSNS: usize = 4096;
/// `BPF_MEMWORDS` (`linux/filter.h`): words of scratch memory.
pub const MEMWORDS: usize = 16;

/// Opcode fields (`linux/bpf_common.h`, `linux/filter.h`).
pub mod code {
    /// Instruction classes (`BPF_CLASS`).
    pub const LD: u16 = 0x00;
    pub const LDX: u16 = 0x01;
    pub const ST: u16 = 0x02;
    pub const STX: u16 = 0x03;
    pub const ALU: u16 = 0x04;
    pub const JMP: u16 = 0x05;
    pub const RET: u16 = 0x06;
    pub const MISC: u16 = 0x07;

    /// Load sizes (`BPF_SIZE`).
    pub const W: u16 = 0x00;
    pub const H: u16 = 0x08;
    pub const B: u16 = 0x10;

    /// Load modes (`BPF_MODE`).
    pub const IMM: u16 = 0x00;
    pub const ABS: u16 = 0x20;
    pub const IND: u16 = 0x40;
    pub const MEM: u16 = 0x60;
    pub const LEN: u16 = 0x80;
    pub const MSH: u16 = 0xa0;

    /// ALU operations (`BPF_OP`).
    pub const ADD: u16 = 0x00;
    pub const SUB: u16 = 0x10;
    pub const MUL: u16 = 0x20;
    pub const DIV: u16 = 0x30;
    pub const OR: u16 = 0x40;
    pub const AND: u16 = 0x50;
    pub const LSH: u16 = 0x60;
    pub const RSH: u16 = 0x70;
    pub const NEG: u16 = 0x80;
    pub const MOD: u16 = 0x90;
    pub const XOR: u16 = 0xa0;

    /// Jump operations (`BPF_OP`).
    pub const JA: u16 = 0x00;
    pub const JEQ: u16 = 0x10;
    pub const JGT: u16 = 0x20;
    pub const JGE: u16 = 0x30;
    pub const JSET: u16 = 0x40;

    /// Operand source (`BPF_SRC`): the constant `k` or `X`.
    pub const K: u16 = 0x00;
    pub const X: u16 = 0x08;

    /// Return value (`BPF_RVAL`): `k`, `X` or `A`.
    pub const A: u16 = 0x10;

    /// Miscellaneous operations (`BPF_MISCOP`).
    pub const TAX: u16 = 0x00;
    pub const TXA: u16 = 0x80;

    /// `BPF_CLASS(code)`.
    pub const fn class(code: u16) -> u16 {
        code & 0x07
    }
}

/// One instruction: `struct sock_filter` (`linux/filter.h`), 8 bytes:
/// `__u16 code; __u8 jt; __u8 jf; __u32 k;`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Insn {
    pub code: u16,
    /// Conditional jumps: how many instructions to skip when true.
    pub jt: u8,
    /// ... and when false.
    pub jf: u8,
    pub k: u32,
}

impl Insn {
    /// `sizeof(struct sock_filter)`.
    pub const SIZE: usize = 8;

    /// `BPF_STMT(code, k)`.
    pub const fn stmt(code: u16, k: u32) -> Insn {
        Insn { code, jt: 0, jf: 0, k }
    }

    /// `BPF_JUMP(code, k, jt, jf)`.
    pub const fn jump(code: u16, k: u32, jt: u8, jf: u8) -> Insn {
        Insn { code, jt, jf, k }
    }

    /// An instruction from its bytes in the host's (little-endian) order.
    pub fn from_bytes(b: &[u8; Insn::SIZE]) -> Insn {
        Insn {
            code: u16::from_le_bytes([b[0], b[1]]),
            jt: b[2],
            jf: b[3],
            k: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        }
    }

    /// The instructions in `bytes`, a whole number of them.
    pub fn parse(bytes: &[u8]) -> Vec<Insn> {
        bytes.chunks_exact(Insn::SIZE).map(|c| Insn::from_bytes(c.try_into().expect("8 bytes"))).collect()
    }
}

/// Why a program was refused; `pc` is the index of the instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No instructions.
    Empty,
    /// More than [`MAXINSNS`].
    TooLong,
    /// Not an opcode of the classic set, or one filters may not use.
    BadOpcode { pc: usize },
    /// A load outside the input, or not an aligned word.
    BadLoad { pc: usize },
    /// Scratch memory index of [`MEMWORDS`] or more.
    BadMemory { pc: usize },
    /// A jump past the last instruction.
    BadJump { pc: usize },
    /// Division or remainder by a constant 0, or a shift by 32 or more.
    BadConstant { pc: usize },
    /// The last instruction is not a return.
    NoReturn,
}

/// What a checked program may load: the input's size in 32-bit words
/// (seccomp: `sizeof(struct seccomp_data) / 4` = 16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    pub data_words: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Src {
    K(u32),
    X,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AluOp {
    Add,
    Sub,
    Mul,
    Div,
    Or,
    And,
    Lsh,
    Rsh,
    Mod,
    Xor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cond {
    Eq,
    Gt,
    Ge,
    Set,
}

/// A checked instruction, decoded once so a run does not decode again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    /// `A = data[word]`.
    LoadWord(u32),
    /// `A = k` (constants and the input's length).
    LoadA(u32),
    LoadAMem(u8),
    LoadX(u32),
    LoadXMem(u8),
    StoreA(u8),
    StoreX(u8),
    Alu(AluOp, Src),
    Neg,
    /// Skip this many instructions.
    Ja(u32),
    Jump(Cond, Src, u8, u8),
    Ret(Src),
    RetA,
    Tax,
    Txa,
}

/// A program [`Program::new`] accepted, ready to [`run`](Program::run).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    ops: Box<[Op]>,
}

impl Program {
    /// Check `insns` against the rules of the module docs and `rules`.
    pub fn new(insns: &[Insn], rules: Rules) -> Result<Program, Error> {
        if insns.is_empty() {
            return Err(Error::Empty);
        }
        if insns.len() > MAXINSNS {
            return Err(Error::TooLong);
        }
        let len = insns.len();
        let mut ops = Vec::with_capacity(len);
        for (pc, insn) in insns.iter().enumerate() {
            ops.push(decode(pc, insn, len, rules)?);
        }
        if !matches!(ops[len - 1], Op::Ret(_) | Op::RetA) {
            return Err(Error::NoReturn);
        }
        Ok(Program { ops: ops.into_boxed_slice() })
    }

    /// How many instructions the program has.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Never true: an empty program is refused.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Run the program on `data` (at least `rules.data_words` words; a
    /// shorter slice reads as zeros past its end) and return its value.
    pub fn run(&self, data: &[u32]) -> u32 {
        let mut a: u32 = 0;
        let mut x: u32 = 0;
        let mut mem = [0u32; MEMWORDS];
        let mut pc = 0usize;
        // Every jump goes forward and stays inside, and the last
        // instruction returns: the loop ends within `len` steps.
        while let Some(op) = self.ops.get(pc) {
            pc += 1;
            match *op {
                Op::LoadWord(w) => a = data.get(w as usize).copied().unwrap_or(0),
                Op::LoadA(k) => a = k,
                Op::LoadAMem(i) => a = mem[usize::from(i)],
                Op::LoadX(k) => x = k,
                Op::LoadXMem(i) => x = mem[usize::from(i)],
                Op::StoreA(i) => mem[usize::from(i)] = a,
                Op::StoreX(i) => mem[usize::from(i)] = x,
                Op::Alu(op, src) => {
                    let v = match src {
                        Src::K(k) => k,
                        Src::X => x,
                    };
                    a = match op {
                        AluOp::Add => a.wrapping_add(v),
                        AluOp::Sub => a.wrapping_sub(v),
                        AluOp::Mul => a.wrapping_mul(v),
                        AluOp::Div | AluOp::Mod if v == 0 => return 0,
                        AluOp::Div => a / v,
                        AluOp::Mod => a % v,
                        AluOp::Or => a | v,
                        AluOp::And => a & v,
                        AluOp::Xor => a ^ v,
                        AluOp::Lsh => a.checked_shl(v).unwrap_or(0),
                        AluOp::Rsh => a.checked_shr(v).unwrap_or(0),
                    };
                }
                Op::Neg => a = a.wrapping_neg(),
                Op::Ja(k) => pc += k as usize,
                Op::Jump(cond, src, jt, jf) => {
                    let v = match src {
                        Src::K(k) => k,
                        Src::X => x,
                    };
                    let taken = match cond {
                        Cond::Eq => a == v,
                        Cond::Gt => a > v,
                        Cond::Ge => a >= v,
                        Cond::Set => a & v != 0,
                    };
                    pc += usize::from(if taken { jt } else { jf });
                }
                Op::Ret(Src::K(k)) => return k,
                Op::Ret(Src::X) => return x,
                Op::RetA => return a,
                Op::Tax => x = a,
                Op::Txa => a = x,
            }
        }
        // Unreachable for a checked program; 0 is classic BPF's "drop".
        0
    }
}

/// Check and decode the instruction at `pc` of a program of `len`.
fn decode(pc: usize, insn: &Insn, len: usize, rules: Rules) -> Result<Op, Error> {
    use code::*;
    let c = insn.code;
    let k = insn.k;
    let bad = Error::BadOpcode { pc };
    let mem = |k: u32| if (k as usize) < MEMWORDS { Ok(k as u8) } else { Err(Error::BadMemory { pc }) };
    // The instruction after the jump is pc + 1; skipping `n` lands on
    // pc + 1 + n, which must be an instruction.
    let target_ok = |n: u64| (pc as u64) + 1 + n < len as u64;
    let data_bytes = rules.data_words.saturating_mul(4);
    Ok(match class(c) {
        LD => match c & !0x07 {
            m if m == W | ABS => {
                if !k.is_multiple_of(4) || k >= data_bytes {
                    return Err(Error::BadLoad { pc });
                }
                Op::LoadWord(k / 4)
            }
            m if m == W | LEN => Op::LoadA(data_bytes),
            m if m == W | IMM => Op::LoadA(k),
            m if m == W | MEM => Op::LoadAMem(mem(k)?),
            m if m & 0xe0 == ABS || m & 0xe0 == IND || m & 0xe0 == MSH => return Err(Error::BadLoad { pc }),
            _ => return Err(bad),
        },
        LDX => match c & !0x07 {
            m if m == W | LEN => Op::LoadX(data_bytes),
            m if m == W | IMM => Op::LoadX(k),
            m if m == W | MEM => Op::LoadXMem(mem(k)?),
            m if m == B | MSH => return Err(Error::BadLoad { pc }),
            _ => return Err(bad),
        },
        ST if c == ST => Op::StoreA(mem(k)?),
        STX if c == STX => Op::StoreX(mem(k)?),
        ALU => {
            let src = if c & X != 0 { Src::X } else { Src::K(k) };
            if c & !(0x07 | 0xf0 | X) != 0 {
                return Err(bad);
            }
            let op = match c & 0xf0 {
                ADD => AluOp::Add,
                SUB => AluOp::Sub,
                MUL => AluOp::Mul,
                DIV => AluOp::Div,
                OR => AluOp::Or,
                AND => AluOp::And,
                LSH => AluOp::Lsh,
                RSH => AluOp::Rsh,
                MOD => AluOp::Mod,
                XOR => AluOp::Xor,
                NEG if c & X == 0 => return Ok(Op::Neg),
                _ => return Err(bad),
            };
            match (op, src) {
                (AluOp::Div | AluOp::Mod, Src::K(0)) => return Err(Error::BadConstant { pc }),
                (AluOp::Lsh | AluOp::Rsh, Src::K(k)) if k >= 32 => return Err(Error::BadConstant { pc }),
                _ => {}
            }
            Op::Alu(op, src)
        }
        JMP => {
            if c & !(0x07 | 0xf0 | X) != 0 {
                return Err(bad);
            }
            let src = if c & X != 0 { Src::X } else { Src::K(k) };
            let cond = match c & 0xf0 {
                JA if c & X == 0 => {
                    if !target_ok(u64::from(k)) {
                        return Err(Error::BadJump { pc });
                    }
                    return Ok(Op::Ja(k));
                }
                JEQ => Cond::Eq,
                JGT => Cond::Gt,
                JGE => Cond::Ge,
                JSET => Cond::Set,
                _ => return Err(bad),
            };
            if !target_ok(u64::from(insn.jt)) || !target_ok(u64::from(insn.jf)) {
                return Err(Error::BadJump { pc });
            }
            Op::Jump(cond, src, insn.jt, insn.jf)
        }
        RET => match c & !0x07 {
            K => Op::Ret(Src::K(k)),
            X => Op::Ret(Src::X),
            A => Op::RetA,
            _ => return Err(bad),
        },
        MISC => match c & !0x07 {
            TAX => Op::Tax,
            TXA => Op::Txa,
            _ => return Err(bad),
        },
        _ => return Err(bad),
    })
}

#[cfg(test)]
mod tests;
