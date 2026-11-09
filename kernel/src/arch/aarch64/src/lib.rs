//! The parts of the aarch64 boot stub (ADR 0154) that touch no hardware, so
//! they build and are unit tested on the host: the devicetree reader, register
//! and exception decoding, page table entries, the PL011's register model,
//! the semihosting call block, and the self-test report format. The binary
//! (`main.rs`) has the assembly, the registers and the QEMU `virt` specifics.

#![cfg_attr(not(test), no_std)]

pub mod cmdline;
pub mod fdt;
pub mod mmu;
pub mod pl011;
pub mod psci;
pub mod regs;
pub mod report;
pub mod semihost;

#[cfg(test)]
extern crate std;
