//! Linux personality: the x86-64 Linux system call ABI.
//!
//! Calling convention (from the public x86-64 psABI and the kernel's
//! documented user ABI): number in `rax`, arguments in `rdi, rsi, rdx, r10,
//! r8, r9`, result in `rax`, errors returned as `-errno`.
//!
//! Implementation status lives in `coverage/coverage.json`, not here.

#![no_std]

use poly_abi::{Outcome, Personality, SyscallDispatch, SyscallRegs};

/// `ENOSYS` from the Linux UAPI headers (`asm-generic/errno.h`).
pub const ENOSYS: u64 = 38;

/// Encode an errno the way Linux returns it in `rax`.
pub const fn err(errno: u64) -> u64 {
    errno.wrapping_neg()
}

#[derive(Debug)]
pub struct Linux;

impl SyscallDispatch for Linux {
    const PERSONALITY: Personality = Personality::Linux;

    fn dispatch(regs: &mut SyscallRegs) -> Outcome {
        let number = regs.rax;
        // No syscalls are implemented yet. Each one is added from a spec
        // sheet under specs/linux/ by the agent pipeline.
        regs.rax = err(ENOSYS);
        Outcome::Unimplemented { table: "linux", number }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unimplemented_returns_negative_enosys() {
        let mut r = SyscallRegs { rax: 39, ..Default::default() };
        assert_eq!(Linux::dispatch(&mut r), Outcome::Unimplemented { table: "linux", number: 39 });
        assert_eq!(r.rax as i64, -38);
    }
}
