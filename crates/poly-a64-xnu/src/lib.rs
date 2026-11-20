//! A translated arm64 XNU process (ADR 0257): what lets an arm64-only Mach-O
//! program run on x86-64 Poly through the interpreter and the block JIT.
//!
//! An arm64 program cannot run on the CPU, so its process runs a small
//! x86-64 program of Poly's instead, the **runtime** (`userland/native/a64`,
//! `/usr/libexec/poly-a64`), with the arm64 program mapped beside it:
//!
//! ```text
//!  exec of an arm64 Mach-O (kernel, crate::xnu)
//!    poly_dyld::link (Options::ARM64)  ─► load(): segments, fixups, the arm64
//!    start code, thread-local blocks, the stack, the TSD page ─► Handoff
//!    the runtime's ELF, its heap and code area; rdi = the hand-off block
//!  the runtime (x86-64, user space)
//!    Guest::new(handoff): a GuestMem over the window, the Cpu at the entry
//!    poly_a64_jit::Engine::run ─► svc #0x80 ─► Bridge ─► arm64::to_frame
//!      ─► a real x86-64 `syscall` into the XNU personality ─► from_frame
//! ```
//!
//! - [`layout`]: where the guest's memory goes (the guest window) and where
//!   the runtime's own memory goes.
//! - [`handoff`]: the block the kernel writes for the runtime.
//! - [`load`]: everything the kernel does to an arm64 program after
//!   `poly_dyld::link`, through a [`load::Target`] (the kernel's address
//!   space, or a [`poly_a64_jit::mem::GuestMem`] in tests).
//! - [`run`]: the runtime's side: the guest's memory and registers from a
//!   hand-off, and its system calls ([`run::Bridge`]): almost all of them go
//!   to the kernel unchanged in the x86-64 convention, so the XNU
//!   personality has one copy of every call; the memory calls the guest
//!   window has to know about are kept here.
//!
//! Clean room (CLEAN_ROOM.md): the Arm Architecture Reference Manual,
//! Apple's public arm64 calling convention, the macOS SDK's headers for
//! numbers and layouts (`sys/syscall.h`, `sys/mman.h`, `mach-o/loader.h`)
//! and the spec sheets under `specs/macos`. No source of XNU, dyld,
//! Rosetta-related material or any emulator was read; no Apple binary was
//! disassembled.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod handoff;
pub mod layout;
pub mod load;
pub mod run;

#[cfg(test)]
mod tests;

pub use handoff::{EngineKind, Handoff, Region};
pub use layout::Layout;
pub use poly_macho::CpuKind;

use poly_macho::{Fat, FileKind, MachO};

/// The CPU Poly runs a Mach-O file as: x86-64 natively when the file has
/// an x86-64 image (thin, or a slice of a universal file), else arm64
/// through the translator; `None` for anything else (32-bit, arm64e only,
/// not a Mach-O file).
pub fn runnable_cpu(data: &[u8]) -> Option<CpuKind> {
    match FileKind::detect(data).ok()? {
        FileKind::Fat | FileKind::Fat64 => {
            let fat = Fat::parse(data).ok()?;
            [CpuKind::X86_64, CpuKind::Arm64].into_iter().find(|&k| fat.find_kind(k).is_some())
        }
        FileKind::Thin64 => match MachO::parse(data).ok()?.cpu().kind() {
            k @ (CpuKind::X86_64 | CpuKind::Arm64) => Some(k),
            _ => None,
        },
        _ => None,
    }
}
