//! XNU personality: BSD system calls and Mach traps.
//!
//! On x86-64 macOS, user space enters with `syscall`. The top byte of `rax`
//! picks the call class (open-source XNU, `osfmk/mach/i386/syscall_sw.h`):
//! 1 = Mach trap, 2 = BSD ("unix") syscall, 3 = machine-dependent. The low 24
//! bits are the number within that class. Arguments are in `rdi, rsi, rdx,
//! r10, r8, r9`.
//!
//! BSD calls report failure by setting the carry flag and putting the errno
//! in `rax`; Mach traps return a `kern_return_t` in `rax`.

#![no_std]

use poly_abi::{Outcome, Personality, RFLAGS_CF, SyscallDispatch, SyscallRegs};

/// `ENOSYS` on Darwin (macOS SDK `sys/errno.h`).
pub const ENOSYS: u64 = 78;
/// `KERN_FAILURE` (macOS SDK `mach/kern_return.h`).
pub const KERN_FAILURE: u64 = 5;

pub const CLASS_SHIFT: u32 = 24;
pub const NUMBER_MASK: u64 = (1 << CLASS_SHIFT) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Mach,
    Bsd,
    MachineDependent,
    Other(u8),
}

impl Class {
    pub const fn name(self) -> &'static str {
        match self {
            Class::Mach => "mach",
            Class::Bsd => "bsd",
            Class::MachineDependent => "mdep",
            Class::Other(_) => "xnu-unknown-class",
        }
    }
}

/// Split `rax` into call class and number.
pub const fn decode(rax: u64) -> (Class, u64) {
    let class = match (rax >> CLASS_SHIFT) & 0xff {
        1 => Class::Mach,
        2 => Class::Bsd,
        3 => Class::MachineDependent,
        c => Class::Other(c as u8),
    };
    (class, rax & NUMBER_MASK)
}

#[derive(Debug)]
pub struct Xnu;

impl SyscallDispatch for Xnu {
    const PERSONALITY: Personality = Personality::Xnu;

    fn dispatch(regs: &mut SyscallRegs) -> Outcome {
        let (class, number) = decode(regs.rax);
        // Provisional results until spec sheets exist. Real macOS may also
        // deliver SIGSYS for unknown BSD calls; the spec agent must confirm.
        match class {
            Class::Mach => {
                regs.rax = KERN_FAILURE;
            }
            _ => {
                regs.rax = ENOSYS;
                regs.rflags |= RFLAGS_CF;
            }
        }
        Outcome::Unimplemented { table: class.name(), number }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_classes() {
        assert_eq!(decode(0x0200_0004), (Class::Bsd, 4));
        assert_eq!(decode(0x0100_001f), (Class::Mach, 0x1f));
        assert_eq!(decode(0x0300_0003), (Class::MachineDependent, 3));
    }

    #[test]
    fn bsd_error_sets_carry() {
        let mut r = SyscallRegs { rax: 0x0200_0004, ..Default::default() };
        assert_eq!(Xnu::dispatch(&mut r), Outcome::Unimplemented { table: "bsd", number: 4 });
        assert_eq!(r.rax, ENOSYS);
        assert_ne!(r.rflags & RFLAGS_CF, 0);
    }

    #[test]
    fn mach_trap_returns_kern_return() {
        let mut r = SyscallRegs { rax: 0x0100_001f, ..Default::default() };
        Xnu::dispatch(&mut r);
        assert_eq!(r.rax, KERN_FAILURE);
        assert_eq!(r.rflags & RFLAGS_CF, 0);
    }
}
