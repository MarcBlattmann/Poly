//! The Darwin system call ABI on x86-64, as the spec sheets of
//! `specs/macos/` record it: `syscall` with the call class in bits 24-31 of
//! `rax` (Mach traps 1, BSD calls 2) and the number below; arguments in
//! `rdi, rsi, rdx, r10, r8, r9`. A BSD call fails with the carry flag set
//! and the errno in `rax`; a Mach trap returns its `kern_return_t` in
//! `rax`. The numbers are the coverage map's (`coverage/macos/syscall/`,
//! from the system call tables of the open-source XNU: names and numbers
//! are ABI facts).

/// The class shift and the classes.
pub const CLASS_SHIFT: u32 = 24;
pub const CLASS_MACH: u64 = 1;
pub const CLASS_UNIX: u64 = 2;

/// `rax` for BSD call `n`.
pub const fn unix(n: u64) -> u64 {
    (CLASS_UNIX << CLASS_SHIFT) | n
}

/// `rax` for Mach trap `n`.
pub const fn mach(n: u64) -> u64 {
    (CLASS_MACH << CLASS_SHIFT) | n
}

/// Darwin errno values (`<sys/errno.h>`).
pub mod errno {
    pub const EPERM: i32 = 1;
    pub const ENOENT: i32 = 2;
    pub const EINTR: i32 = 4;
    pub const EIO: i32 = 5;
    pub const EBADF: i32 = 9;
    pub const ENOMEM: i32 = 12;
    pub const EFAULT: i32 = 14;
    pub const EINVAL: i32 = 22;
    pub const ERANGE: i32 = 34;
    pub const ENOSYS: i32 = 78;
    pub const EOVERFLOW: i32 = 84;
    pub const EILSEQ: i32 = 92;
}

/// `<sys/mman.h>` values `malloc` needs.
pub mod mman {
    pub const PROT_READ: i32 = 0x01;
    pub const PROT_WRITE: i32 = 0x02;
    pub const MAP_PRIVATE: i32 = 0x0002;
    pub const MAP_ANON: i32 = 0x1000;
    /// `MAP_FAILED`.
    pub const FAILED: usize = usize::MAX;
}

/// `<sys/signal.h>`: `SIGABRT`.
pub const SIGABRT: i32 = 6;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes() {
        // The values the sheets give: write is 0x2000004, task_self_trap
        // 0x100001c.
        assert_eq!(unix(4), 0x200_0004);
        assert_eq!(mach(28), 0x100_001c);
    }
}
