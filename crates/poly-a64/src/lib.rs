//! An A64 (64-bit Arm) instruction decoder and reference interpreter.
//!
//! This is step one of the arm64 to x86-64 translator that lets arm64-only
//! macOS apps run on x86-64 PCs (ADR 0152, following ADR 0151). It has two
//! halves with one shared type between them:
//!
//! - [`decode`] turns a 32-bit instruction word into an [`Inst`]: a `Copy`
//!   value with register numbers, sizes, expanded immediates, addressing
//!   modes and condition codes already decoded. It is a pure function. The
//!   JIT that comes later decodes guest code with the same function and
//!   translates `Inst` values, so it never re-reads encodings.
//! - [`Cpu`] is the register state of one EL0 thread and [`Cpu::step`]
//!   executes one instruction against a [`Memory`] the caller provides. The
//!   caller handles system calls ([`Outcome::Svc`]), breakpoints and faults.
//!   This interpreter is the reference the JIT is tested against, and the
//!   fallback for instructions the JIT does not translate.
//!
//! # What is covered
//!
//! - Integer: add/subtract (immediate, shifted and extended register, with
//!   carry, flag setting), logical, move wide, `ADR`/`ADRP`, bitfield moves
//!   and all their aliases, `EXTR`, conditional compare and select,
//!   multiplies (including `SMULH`/`UMULH` and the long forms), divides,
//!   variable shifts, `CLZ`/`CLS`/`RBIT`/`REV*`.
//! - Branches: `B`, `BL`, `B.cond`, `CBZ`/`CBNZ`, `TBZ`/`TBNZ`, `BR`,
//!   `BLR`, `RET`.
//! - System: `SVC`, `BRK`, hints (all no-ops except `XPACLRI`),
//!   `MRS`/`MSR` for `NZCV`, `FPCR`, `FPSR`, `TPIDR_EL0` and read-only
//!   counters, barriers, `DC ZVA` and cache maintenance, `CFINV`, `AXFLAG`,
//!   `XAFLAG`, `RMIF`, `SETF8`/`SETF16`, `CRC32*`.
//! - Pointer authentication as an arm64 (not arm64e) process sees it on
//!   macOS: the keys are disabled, so `PAC*`, `AUT*` and the authenticating
//!   branches and loads leave pointers alone; `XPAC*` strip with a 47-bit
//!   address space; `PACGA` gives an implementation-defined code.
//! - Loads and stores: every size and addressing mode, sign-extending
//!   loads, literal loads, pairs, `PRFM`, exclusives, acquire/release
//!   (`LDAPUR`/`STLUR` too), the LSE atomics (`LDADD` family, `SWP`, `CAS`,
//!   `CASP`) with FEAT_LSE2's alignment rule, and the SIMD register forms,
//!   plus `LD1`..`LD4`/`ST1`..`ST4`.
//! - Scalar floating point on single and double precision: moves,
//!   arithmetic, fused multiply-add, compare, conditional select, `FCVT`,
//!   integer and fixed-point conversions, `FRINT*`, `FMOV` immediate.
//! - Advanced SIMD: integer arithmetic, logic, compares, min/max (also
//!   pairwise), halving, saturating and rounding forms, long, wide and
//!   narrow forms, by-element forms, dot products, shifts by immediate and
//!   register (rounding, saturating, narrowing), permutes, table lookups,
//!   reductions; floating point arithmetic, fused multiply-add, compares,
//!   pairwise and across-lane forms, conversions (also fixed point),
//!   `FRECPE`/`FRSQRTE`/`FRECPS`/`FRSQRTS`, `FCVTL`/`FCVTN`; SHA3's `EOR3`,
//!   `BCAX`, `RAX1`, `XAR`.
//!
//! Where the architecture leaves a choice (CONSTRAINED UNPREDICTABLE
//! encodings, "should be" fields, store-exclusive after an ordinary store),
//! poly-a64 does what Apple silicon does, measured with `a64diff native`.
//!
//! # What is not covered
//!
//! Everything else decodes to [`Inst::Unsupported`] and executes as
//! [`Outcome::Unsupported`]: SVE and SME, AES, SHA1/SHA2/SHA512, SM3/SM4, memory
//! tagging, half-precision arithmetic (scalar and vector), complex numbers
//! (`FCMLA`, `FCADD`), `FMLAL`/`FMLSL`, `FCVTXN` and the half-precision
//! conversions of `FCVTL`/`FCVTN`. Floating point is round-to-nearest-even
//! with `IOC`/`DZC` flags (and `QC` for saturation) only; see [`fp`] for
//! the details. Only EL0 is modelled.
//!
//! Clean room (CLEAN_ROOM.md): written from the Arm Architecture Reference
//! Manual for A-profile (DDI 0487) and checked by assembling test words
//! with the system assembler and comparing runs with `qemu-aarch64`, both
//! used as black boxes (`tools/a64diff`). No emulator, disassembler or
//! other implementation's source was read.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod cpu;
pub mod decode;
mod decode_simd;
mod decode_simd2;
mod exec_fp;
mod exec_int;
mod exec_mem;
mod exec_simd;
mod exec_simd2;
pub mod fp;
pub mod inst;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod vectors;

pub use cpu::{Access, Cpu, MemFault, Memory, Outcome, SliceMemory};
pub use decode::{decode, decode_bit_masks};
pub use exec_int::{add_with_carry, extend_value, shift_value};
pub use inst::*;
