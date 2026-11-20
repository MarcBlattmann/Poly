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
//!
//! The calls with a kernel behind them go through [`dispatch_with`] and the
//! [`XnuKernel`] trait: the Mach core of `poly-mach` (ports, rights,
//! messages, semaphores; ADR 0238), Mach VM, the BSD memory calls, ulock and
//! psynch. [`call`] is the architecture-neutral entry the arm64 translator
//! uses (its `x16` decoded with [`decode_arm64`], arguments `x0..x7`).

#![cfg_attr(not(test), no_std)]

pub mod arm64;
pub mod bsd;

extern crate alloc;

pub mod errno;
pub mod kernel;
pub mod mach;
pub mod mem;
pub mod sync;
mod wait;

pub use kernel::{Arch, Backing, Inherit, MachState, PageInfo, Region, ThreadStart, Wakeup, XnuKernel};

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

/// Split an arm64 `x16` into call class and number: negative numbers are
/// Mach traps (`-n`), positive ones BSD calls.
pub const fn decode_arm64(x16: u64) -> (Class, u64) {
    let v = x16 as i64;
    if v < 0 { (Class::Mach, v.unsigned_abs()) } else { (Class::Bsd, v as u64) }
}

/// BSD call numbers (`sys/syscall.h` names; numbers are ABI facts).
pub mod sys {
    pub const MSYNC: u64 = 65;
    pub const MPROTECT: u64 = 74;
    pub const MADVISE: u64 = 75;
    pub const MINCORE: u64 = 78;
    pub const MLOCK: u64 = 203;
    pub const MUNLOCK: u64 = 204;
    pub const MINHERIT: u64 = 250;
    pub const PSYNCH_MUTEXWAIT: u64 = 301;
    pub const PSYNCH_MUTEXDROP: u64 = 302;
    pub const PSYNCH_CVBROAD: u64 = 303;
    pub const PSYNCH_CVSIGNAL: u64 = 304;
    pub const PSYNCH_CVWAIT: u64 = 305;
    pub const BSDTHREAD_CREATE: u64 = 360;
    pub const ULOCK_WAIT: u64 = 515;
    pub const ULOCK_WAKE: u64 = 516;
    pub const ULOCK_WAIT2: u64 = 544;
}

/// The result of a call, before the architecture encodes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ret {
    /// A Mach trap's `kern_return_t` or port name.
    Mach(u64),
    /// A BSD call that succeeded.
    Ok(u64),
    /// A BSD call that failed with this errno.
    Err(u32),
}

/// Run one call with the kernel behind it; `None` if Poly does not have it.
pub fn call(k: &mut dyn XnuKernel, class: Class, number: u64, a: &[u64; 8]) -> Option<Ret> {
    match class {
        Class::Mach => mach::trap(k, number, a).map(Ret::Mach),
        Class::Bsd => bsd(k, number, a).map(|r| match r {
            Ok(v) => Ret::Ok(v),
            Err(e) => Ret::Err(e),
        }),
        _ => None,
    }
}

fn bsd(k: &mut dyn XnuKernel, number: u64, a: &[u64; 8]) -> Option<Result<u64, u32>> {
    use sys::*;
    let r = match number {
        MSYNC => mem::msync(k, a[0], a[1], a[2] as u32),
        MPROTECT => mem::mprotect(k, a[0], a[1], a[2] as u32),
        MADVISE => mem::madvise(k, a[0], a[1], a[2] as u32),
        MINCORE => mem::mincore(k, a[0], a[1], a[2]),
        MLOCK => mem::mlock(k, a[0], a[1], true),
        MUNLOCK => mem::mlock(k, a[0], a[1], false),
        MINHERIT => mem::minherit(k, a[0], a[1], a[2] as u32),
        PSYNCH_MUTEXWAIT => sync::mutexwait(k, a),
        PSYNCH_MUTEXDROP => sync::mutexdrop(k, a),
        PSYNCH_CVBROAD => sync::cvbroad(k, a),
        PSYNCH_CVSIGNAL => sync::cvsignal(k, a),
        PSYNCH_CVWAIT => sync::cvwait(k, a),
        BSDTHREAD_CREATE => sync::bsdthread_create(k, a),
        ULOCK_WAIT => sync::ulock_wait(k, a[0] as u32, a[1], a[2], a[3].saturating_mul(1000)),
        ULOCK_WAIT2 => sync::ulock_wait(k, a[0] as u32, a[1], a[2], a[3]),
        ULOCK_WAKE => sync::ulock_wake(k, a[0] as u32, a[1]),
        _ => return None,
    };
    Some(r)
}

/// The x86-64 entry with a kernel: arguments in `rdi, rsi, rdx, r10, r8,
/// r9`, the seventh and eighth on the user stack above the return address
/// (`rsp + 8`, `rsp + 16`). Calls Poly does not have get [`Xnu::dispatch`]'s
/// "not implemented" results.
pub fn dispatch_with(k: &mut dyn XnuKernel, regs: &mut SyscallRegs) -> Outcome {
    let (class, number) = decode(regs.rax);
    let mut a = [regs.rdi, regs.rsi, regs.rdx, regs.r10, regs.r8, regs.r9, 0, 0];
    for (i, slot) in a[6..].iter_mut().enumerate() {
        let mut b = [0u8; 8];
        if k.copy_in(regs.rsp.wrapping_add(8 + 8 * i as u64), &mut b).is_ok() {
            *slot = u64::from_le_bytes(b);
        }
    }
    match call(k, class, number, &a) {
        Some(r) => {
            encode_x86_64(regs, r);
            Outcome::Handled
        }
        None => Xnu::dispatch(regs),
    }
}

/// Put a result into the x86-64 registers: `rax`, and the carry flag for
/// BSD calls.
pub fn encode_x86_64(regs: &mut SyscallRegs, r: Ret) {
    match r {
        Ret::Mach(v) => regs.rax = v,
        Ret::Ok(v) => {
            regs.rax = v;
            regs.rflags &= !RFLAGS_CF;
        }
        Ret::Err(e) => {
            regs.rax = u64::from(e);
            regs.rflags |= RFLAGS_CF;
        }
    }
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
        assert_eq!(decode_arm64(-31i64 as u64), (Class::Mach, 31));
        assert_eq!(decode_arm64(74), (Class::Bsd, 74));
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
