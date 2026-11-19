//! The first BSD system calls of the XNU personality: what a program loaded
//! by Poly's dyld needs to say something and end (ADR 0237). The kernel
//! provides the services through [`BsdKernel`]; this module decodes the
//! registers and encodes the results as XNU does (spec sheets
//! `specs/macos/bsd/{write,exit}.yaml`): on success the carry flag is clear
//! and `rax` holds the result; on failure the carry flag is set and `rax`
//! holds the errno.

use crate::{Class, ENOSYS, decode};
use poly_abi::{Outcome, RFLAGS_CF, SyscallRegs};

/// BSD system call numbers (macOS SDK `<sys/syscall.h>`).
pub const SYS_EXIT: u64 = 1;
pub const SYS_WRITE: u64 = 4;

/// Darwin errno values (macOS SDK `<sys/errno.h>`).
pub const EBADF: u64 = 9;
pub const EFAULT: u64 = 14;
pub const EINVAL: u64 = 22;

/// What the BSD calls need from the kernel.
pub trait BsdKernel {
    /// Write `len` bytes from user memory at `buf` to descriptor `fd`;
    /// the number written, or an errno.
    fn write(&mut self, fd: i32, buf: u64, len: u64) -> Result<u64, u64>;

    /// End the process with exit status `status` (the parent sees its low
    /// 8 bits). Does not come back to user mode.
    fn exit(&mut self, status: u8);
}

/// Handle the call in `regs` with the kernel's services; calls that are not
/// here fail with `ENOSYS` (Mach traps with `KERN_FAILURE`) as
/// [`Xnu::dispatch`](crate::Xnu) answers them.
pub fn dispatch_with<K: BsdKernel>(k: &mut K, regs: &mut SyscallRegs) -> Outcome {
    let (class, number) = decode(regs.rax);
    if class != Class::Bsd {
        return <crate::Xnu as poly_abi::SyscallDispatch>::dispatch(regs);
    }
    let result = match number {
        SYS_EXIT => {
            k.exit(regs.rdi as u8);
            Ok(0)
        }
        SYS_WRITE => k.write(regs.rdi as i32, regs.rsi, regs.rdx),
        _ => return <crate::Xnu as poly_abi::SyscallDispatch>::dispatch(regs),
    };
    match result {
        Ok(v) => {
            regs.rax = v;
            regs.rflags &= !RFLAGS_CF;
        }
        Err(errno) => {
            regs.rax = if errno == 0 { ENOSYS } else { errno };
            regs.rflags |= RFLAGS_CF;
        }
    }
    Outcome::Handled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct K {
        out: Vec<u8>,
        exited: Option<u8>,
    }

    impl BsdKernel for K {
        fn write(&mut self, fd: i32, buf: u64, len: u64) -> Result<u64, u64> {
            if fd != 1 {
                return Err(EBADF);
            }
            if buf < 4096 && len > 0 {
                return Err(EFAULT);
            }
            self.out.extend(core::iter::repeat_n(b'x', len as usize));
            Ok(len)
        }

        fn exit(&mut self, status: u8) {
            self.exited = Some(status);
        }
    }

    fn call(k: &mut K, n: u64, a: [u64; 3]) -> (Outcome, SyscallRegs) {
        let mut r = SyscallRegs {
            rax: 0x200_0000 | n,
            rdi: a[0],
            rsi: a[1],
            rdx: a[2],
            rflags: RFLAGS_CF,
            ..Default::default()
        };
        let o = dispatch_with(k, &mut r);
        (o, r)
    }

    #[test]
    fn write_returns_the_count_with_carry_clear() {
        let mut k = K::default();
        let (o, r) = call(&mut k, SYS_WRITE, [1, 0x10000, 24]);
        assert_eq!(o, Outcome::Handled);
        assert_eq!(r.rax, 24);
        assert_eq!(r.rflags & RFLAGS_CF, 0);
        // A count of 0 writes nothing and returns 0.
        let (_, r) = call(&mut k, SYS_WRITE, [1, 0x10000, 0]);
        assert_eq!(r.rax, 0);
        assert_eq!(k.out.len(), 24);
    }

    #[test]
    fn write_errors_set_carry_and_errno() {
        let mut k = K::default();
        let (_, r) = call(&mut k, SYS_WRITE, [99, 0x10000, 4]);
        assert_eq!((r.rax, r.rflags & RFLAGS_CF != 0), (EBADF, true));
        let (_, r) = call(&mut k, SYS_WRITE, [1, 8, 4]);
        assert_eq!((r.rax, r.rflags & RFLAGS_CF != 0), (EFAULT, true));
    }

    #[test]
    fn exit_keeps_the_low_8_bits() {
        let mut k = K::default();
        call(&mut k, SYS_EXIT, [263, 0, 0]);
        assert_eq!(k.exited, Some(7));
        call(&mut k, SYS_EXIT, [u64::MAX, 0, 0]);
        assert_eq!(k.exited, Some(255));
    }

    #[test]
    fn other_calls_stay_unimplemented() {
        let mut k = K::default();
        let (o, r) = call(&mut k, 20, [0; 3]);
        assert_eq!(o, Outcome::Unimplemented { table: "bsd", number: 20 });
        assert_eq!(r.rax, ENOSYS);
        let mut r = SyscallRegs { rax: 0x100_001f, ..Default::default() };
        assert_eq!(dispatch_with(&mut k, &mut r), Outcome::Unimplemented { table: "mach", number: 0x1f });
    }
}
