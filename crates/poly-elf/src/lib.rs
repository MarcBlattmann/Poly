//! ELF64 executables for x86-64: validation, load plans and the initial stack.
//!
//! `execve` of a Linux program goes through three steps, one module each:
//!
//! 1. [`Elf::parse`] validates the file header and the program header table
//!    of an ELF64 little-endian x86-64 executable (`ET_EXEC`, or `ET_DYN` for
//!    position-independent executables and for the dynamic linker itself) and
//!    collects the segments a loader cares about: `PT_LOAD`, `PT_INTERP`,
//!    `PT_PHDR`, `PT_TLS`, `PT_DYNAMIC` and `PT_GNU_STACK`.
//! 2. [`LoadPlan::new`] turns the `PT_LOAD` segments into page-aligned
//!    [`Mapping`]s (file-backed pages with a zeroed tail, then anonymous zero
//!    pages for the rest of `.bss`), picks the load bias of an `ET_DYN`
//!    image, and computes the entry point, the address of the program header
//!    table (`AT_PHDR`) and the initial program break.
//! 3. [`build_initial_stack`] lays out `argc`, `argv`, `envp`, the auxiliary
//!    vector and the strings they point to at the top of the new stack, as
//!    the System V AMD64 psABI §3.4.1 ("Initial Stack and Register State")
//!    describes, and returns the initial `rsp`. [`InitialStack`] reads such a
//!    stack back.
//!
//! Static executables need only the first image. A dynamically linked one
//! names its interpreter in `PT_INTERP` ([`Elf::interp`]); the kernel then
//! plans the interpreter as a second image at another base
//! ([`LoadOptions::dyn_base`]), jumps to the interpreter's entry point and
//! passes both through the auxiliary vector ([`AuxInfo::new`]). ADR 0024
//! records the policies chosen here.
//!
//! Sources: the System V gABI (ELF header, program headers, segment
//! contents), the System V AMD64 psABI (initial process stack, auxiliary
//! vector), the public `<elf.h>` header for the numeric constants and the
//! Linux man pages getauxval(3) and execve(2) for the meaning of the
//! auxiliary vector entries.
//!
//! Nothing here touches hardware, allocates or panics: every offset and size
//! read from a file is bounds- and overflow-checked, and the crate is
//! `no_std` so the kernel uses it directly.

#![no_std]

mod header;
mod plan;
mod stack;

pub use header::*;
pub use plan::*;
pub use stack::*;

/// The page size of x86-64 user mappings (and the value of `AT_PAGESZ`).
pub const PAGE_SIZE: u64 = 4096;

/// Rounds `x` down to a multiple of `align`, a power of two.
const fn align_down(x: u64, align: u64) -> u64 {
    x & !(align - 1)
}

/// Rounds `x` up to a multiple of `align`, a power of two; `None` on overflow.
const fn align_up(x: u64, align: u64) -> Option<u64> {
    match x.checked_add(align - 1) {
        Some(v) => Some(v & !(align - 1)),
        None => None,
    }
}

#[cfg(test)]
mod tests;
