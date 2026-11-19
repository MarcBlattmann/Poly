//! An arm64 to x86-64 translator on poly-a64's decoder (ADR 0239).
//!
//! Step two of ADR 0151's translator: arm64-only Mac programs run on x86-64
//! PCs by translating their code, one basic block at a time, to x86-64.
//!
//! - [`mem`]: the guest's memory, with 16 KiB pages, per-page permissions,
//!   `MAP_JIT` regions and the code tracking that invalidation needs.
//! - [`x64`]: an x86-64 machine-code emitter of our own.
//! - [`translate`]: one basic block of decoded A64 instructions
//!   ([`poly_a64::Inst`]) to x86-64.
//! - [`jit`]: the code cache, block chaining, invalidation and the run loop.
//!   Instructions the translator does not handle run in poly-a64's
//!   interpreter, which stays the reference and the fallback.
//! - [`process`]: an arm64 Mach-O program in guest memory (the minimal
//!   loading a translated test program needs) and its system calls in the
//!   arm64 convention.
//!
//! Translated code runs only on an x86-64 host. On any other host (and for
//! comparison) the same interface runs the interpreter.
//!
//! Clean room (CLEAN_ROOM.md): written from the Arm Architecture Reference
//! Manual (DDI 0487) and Intel's and AMD's architecture manuals. Encodings
//! are checked against the system assembler (a black box); the arm64 side
//! against a real Apple silicon CPU running the same instructions. No
//! emulator's, translator's or JIT's source was read.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod jit;
pub mod mem;
pub mod process;
pub mod translate;
pub mod x64;

#[cfg(test)]
mod tests;
