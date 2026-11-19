//! The arm64 macOS system call convention (ADR 0151, ADR 0239).
//!
//! An arm64 Mac program enters the kernel with `svc #0x80`. The call number
//! is in `x16`: a positive number is a BSD system call, a negative one
//! (`-n`) the Mach trap `n`, and `0x8000_0000` the machine-dependent call
//! (`platform_syscall`). Arguments are in `x0` to `x7`. A BSD call reports
//! failure by setting the carry flag (`C` in `NZCV`) and putting the errno
//! in `x0`; it returns its results in `x0` and `x1`. Mach traps return a
//! `kern_return_t` in `x0` and leave the flags alone. These are ABI facts
//! (call names and numbers, CLEAN_ROOM.md); the spec sheets confirm the
//! details call by call.
//!
//! The personality's handlers take the x86-64 register frame
//! ([`SyscallRegs`]), so a translated arm64 thread's call is converted to
//! that frame (class in the top byte of `rax`, as on x86-64 macOS), handled
//! by the same code, and converted back. A translated process therefore
//! needs no second copy of any system call.

use poly_abi::{Outcome, RFLAGS_CF, SyscallDispatch, SyscallRegs};

use crate::{CLASS_SHIFT, Class};

/// The immediate of the `svc` instruction macOS uses.
pub const SVC_IMM: u16 = 0x80;
/// `x16` of the machine-dependent call.
pub const MACHDEP: u64 = 0x8000_0000;
/// `C` in `NZCV`.
pub const NZCV_C: u32 = 1 << 29;
/// The guest page size of arm64 macOS: 16 KiB (`vm_page_size`).
pub const PAGE_SIZE: u64 = 16 * 1024;

/// Split `x16` into call class and number.
pub const fn decode(x16: u64) -> (Class, u64) {
    if x16 == MACHDEP {
        (Class::MachineDependent, 0)
    } else if (x16 as i64) < 0 {
        (Class::Mach, (x16 as i64).unsigned_abs())
    } else {
        (Class::Bsd, x16)
    }
}

/// The x86-64 frame for an arm64 call: `rax` gets the class and number,
/// `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9` the first six arguments. `sp` is
/// the guest stack pointer.
pub fn to_frame(x: &[u64; 31], sp: u64) -> SyscallRegs {
    let (class, number) = decode(x[16]);
    let c = match class {
        Class::Mach => 1,
        Class::Bsd => 2,
        Class::MachineDependent => 3,
        Class::Other(c) => u64::from(c),
    };
    SyscallRegs {
        rax: (c << CLASS_SHIFT) | number,
        rdi: x[0],
        rsi: x[1],
        rdx: x[2],
        r10: x[3],
        r8: x[4],
        r9: x[5],
        rflags: 0,
        rsp: sp,
    }
}

/// Write a handled frame back: `x0` from `rax`, `x1` from `rdx` (the second
/// result of BSD calls such as `fork` and `pipe`), and for BSD calls the
/// carry flag.
pub fn from_frame(frame: &SyscallRegs, x16: u64, x: &mut [u64; 31], nzcv: &mut u32) {
    let (class, _) = decode(x16);
    x[0] = frame.rax;
    if class == Class::Bsd {
        x[1] = frame.rdx;
        if frame.rflags & RFLAGS_CF != 0 {
            *nzcv |= NZCV_C;
        } else {
            *nzcv &= !NZCV_C;
        }
    }
}

/// Run one arm64 system call through a personality's handlers.
pub fn dispatch<D: SyscallDispatch>(x: &mut [u64; 31], sp: u64, nzcv: &mut u32) -> Outcome {
    let x16 = x[16];
    let mut frame = to_frame(x, sp);
    // The handlers return the second result in rdx; start it as the third
    // argument like the x86-64 entry does, then keep it only for BSD calls.
    let out = D::dispatch(&mut frame);
    from_frame(&frame, x16, x, nzcv);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ENOSYS, KERN_FAILURE, Xnu};

    #[test]
    fn decodes_x16() {
        assert_eq!(decode(4), (Class::Bsd, 4));
        assert_eq!(decode((-31i64) as u64), (Class::Mach, 31));
        assert_eq!(decode(MACHDEP), (Class::MachineDependent, 0));
    }

    #[test]
    fn frame_carries_class_number_and_arguments() {
        let mut x = [0u64; 31];
        x[16] = 4;
        x[..6].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        let f = to_frame(&x, 0x1000);
        assert_eq!(crate::decode(f.rax), (Class::Bsd, 4));
        assert_eq!((f.rdi, f.rsi, f.rdx, f.r10, f.r8, f.r9, f.rsp), (1, 2, 3, 4, 5, 6, 0x1000));
    }

    #[test]
    fn bsd_error_sets_carry_and_errno() {
        let mut x = [0u64; 31];
        x[16] = 0x7777;
        let mut nzcv = 0;
        let out = dispatch::<Xnu>(&mut x, 0, &mut nzcv);
        assert_eq!(out, Outcome::Unimplemented { table: "bsd", number: 0x7777 });
        assert_eq!(x[0], ENOSYS);
        assert_ne!(nzcv & NZCV_C, 0);
    }

    #[test]
    fn bsd_success_clears_carry_and_returns_two_values() {
        let mut x = [0u64; 31];
        x[16] = 4;
        let mut nzcv = NZCV_C | 1 << 31;
        let frame = SyscallRegs { rax: 7, rdx: 9, rflags: 0, ..Default::default() };
        from_frame(&frame, 4, &mut x, &mut nzcv);
        assert_eq!((x[0], x[1], nzcv), (7, 9, 1 << 31));
    }

    #[test]
    fn mach_trap_leaves_flags_and_x1() {
        let mut x = [0u64; 31];
        x[16] = (-26i64) as u64;
        x[1] = 55;
        let mut nzcv = NZCV_C;
        dispatch::<Xnu>(&mut x, 0, &mut nzcv);
        assert_eq!((x[0], x[1], nzcv), (KERN_FAILURE, 55, NZCV_C));
    }
}
