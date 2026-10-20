//! System call arguments.
//!
//! x86-64 Linux passes up to six arguments in `rdi, rsi, rdx, r10, r8, r9`
//! (x86-64 psABI, appendix "Linux conventions"). Each register is 64 bits;
//! an argument declared `int` or `unsigned int` in the man page's synopsis
//! uses only the low 32 bits, and the upper half is ignored, so the helpers
//! here truncate rather than reject.

use crate::kernel::UserPtr;
use poly_abi::SyscallRegs;

/// The six argument registers of one system call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Args(pub [u64; 6]);

impl Args {
    /// The arguments in their x86-64 Linux order.
    pub const fn from_regs(r: &SyscallRegs) -> Args {
        Args([r.rdi, r.rsi, r.rdx, r.r10, r.r8, r.r9])
    }

    /// Argument `i` (0-5) as the raw 64-bit register.
    pub const fn raw(&self, i: usize) -> u64 {
        self.0[i]
    }

    /// An `unsigned long` / `size_t` argument.
    pub const fn ulong(&self, i: usize) -> u64 {
        self.0[i]
    }

    /// A `long` / `ssize_t` / `off_t` argument.
    pub const fn long(&self, i: usize) -> i64 {
        self.0[i] as i64
    }

    /// An `int` argument: the low 32 bits, signed.
    pub const fn int(&self, i: usize) -> i32 {
        self.0[i] as u32 as i32
    }

    /// An `unsigned int` argument: the low 32 bits.
    pub const fn uint(&self, i: usize) -> u32 {
        self.0[i] as u32
    }

    /// A file descriptor (`int fd`).
    pub const fn fd(&self, i: usize) -> i32 {
        self.int(i)
    }

    /// A pointer into user memory. Never dereferenced directly.
    pub const fn ptr(&self, i: usize) -> UserPtr {
        UserPtr(self.0[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_map_to_arguments_in_order() {
        let r = SyscallRegs { rdi: 1, rsi: 2, rdx: 3, r10: 4, r8: 5, r9: 6, rax: 99, ..Default::default() };
        assert_eq!(Args::from_regs(&r), Args([1, 2, 3, 4, 5, 6]));
    }

    #[test]
    fn int_arguments_use_the_low_half() {
        let a = Args([0xffff_ffff_0000_0001, u64::MAX, 0x1_ffff_ffff, 0, 0, 0]);
        assert_eq!(a.int(0), 1);
        assert_eq!(a.fd(1), -1);
        assert_eq!(a.uint(2), u32::MAX);
        assert_eq!(a.long(1), -1);
        assert_eq!(a.ptr(2), UserPtr(0x1_ffff_ffff));
    }
}
