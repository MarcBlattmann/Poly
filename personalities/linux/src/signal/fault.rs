//! CPU exceptions in user mode as signals: which signal and `si_code` a
//! fault raises, for [`crate::Linux::fault_signal`].
//!
//! signal(7) names the signals ("SIGSEGV Invalid memory reference",
//! "SIGBUS Bus error (bad memory access)", "SIGILL Illegal Instruction",
//! "SIGFPE Floating-point exception", "SIGTRAP Trace/breakpoint trap");
//! the codes are `asm-generic/siginfo.h`'s; the exception vectors are the
//! Intel SDM's (Vol. 3A, Table 6-1). Which exception raises which signal
//! where the documents do not say (a general-protection fault, the segment
//! faults) is Poly's choice, to be confirmed by the diff runner.

use crate::abi::SigInfo;
use crate::abi::signal::*;

/// Exception vectors (Intel SDM Vol. 3A, Table 6-1).
pub mod vector {
    pub const DIVIDE_ERROR: u8 = 0;
    pub const DEBUG: u8 = 1;
    pub const BREAKPOINT: u8 = 3;
    pub const OVERFLOW: u8 = 4;
    pub const BOUND_RANGE: u8 = 5;
    pub const INVALID_OPCODE: u8 = 6;
    pub const SEGMENT_NOT_PRESENT: u8 = 11;
    pub const STACK_SEGMENT: u8 = 12;
    pub const GENERAL_PROTECTION: u8 = 13;
    pub const PAGE_FAULT: u8 = 14;
    pub const X87_FLOATING_POINT: u8 = 16;
    pub const ALIGNMENT_CHECK: u8 = 17;
    pub const SIMD_FLOATING_POINT: u8 = 19;
}

/// Why a page fault was not resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageFault {
    /// Nothing is mapped at the address (`SEGV_MAPERR`).
    NotMapped,
    /// The mapping does not allow the access (`SEGV_ACCERR`).
    Access,
    /// The page cannot be brought in: a file page past the end of the file,
    /// or a read error (mmap(2): "SIGBUS Attempted access to a page of the
    /// buffer that lies beyond the end of the mapped file").
    Bus,
}

/// The signal for CPU exception `vector` in user mode, with `si_addr` set
/// for faults on memory: `page` describes a page fault at an address,
/// `rip` is the faulting instruction (`si_addr` for the instruction
/// faults, as sigaction(2) says for SIGILL, SIGFPE: "address of faulting
/// instruction").
pub fn fault_info(vector: u8, page: Option<(u64, PageFault)>, rip: u64) -> SigInfo {
    use vector::*;
    if let Some((addr, why)) = page {
        return match why {
            PageFault::NotMapped => SigInfo::fault(SIGSEGV, SEGV_MAPERR, addr),
            PageFault::Access => SigInfo::fault(SIGSEGV, SEGV_ACCERR, addr),
            PageFault::Bus => SigInfo::fault(SIGBUS, BUS_ADRERR, addr),
        };
    }
    match vector {
        DIVIDE_ERROR => SigInfo::fault(SIGFPE, FPE_INTDIV, rip),
        // The x87 and SIMD exceptions carry their cause in the status
        // registers; which FPE_* code that gives is to come. SI_KERNEL
        // until then.
        X87_FLOATING_POINT | SIMD_FLOATING_POINT => SigInfo::fault(SIGFPE, SI_KERNEL, rip),
        INVALID_OPCODE => SigInfo::fault(SIGILL, ILL_ILLOPN, rip),
        DEBUG => SigInfo::fault(SIGTRAP, TRAP_TRACE, rip),
        // `int3` is the kernel's SIGTRAP (rt_sigreturn's spec sheet:
        // SI_KERNEL; `rip` after the instruction, trapno 3).
        BREAKPOINT => SigInfo::new(SIGTRAP, SI_KERNEL),
        ALIGNMENT_CHECK => SigInfo::fault(SIGBUS, BUS_ADRALN, 0),
        // A page fault without a reason (no memory map): unmapped.
        PAGE_FAULT => SigInfo::fault(SIGSEGV, SEGV_MAPERR, 0),
        // General protection (privileged instructions, non-canonical
        // addresses, `int n`), segment faults, bound, overflow: SIGSEGV
        // from the kernel, without an address.
        OVERFLOW | BOUND_RANGE | SEGMENT_NOT_PRESENT | STACK_SEGMENT | GENERAL_PROTECTION => {
            SigInfo::new(SIGSEGV, SI_KERNEL)
        }
        _ => SigInfo::new(SIGSEGV, SI_KERNEL),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_faults_carry_the_address_and_reason() {
        let i = fault_info(vector::PAGE_FAULT, Some((0x10, PageFault::NotMapped)), 0x40_1000);
        assert_eq!((i.signal(), i.code, i.addr()), (SIGSEGV, SEGV_MAPERR, 0x10));
        let i = fault_info(vector::PAGE_FAULT, Some((0x40_0000, PageFault::Access)), 0);
        assert_eq!((i.signal(), i.code, i.addr()), (SIGSEGV, SEGV_ACCERR, 0x40_0000));
        let i = fault_info(vector::PAGE_FAULT, Some((0x7000_0000, PageFault::Bus)), 0);
        assert_eq!((i.signal(), i.code, i.addr()), (SIGBUS, BUS_ADRERR, 0x7000_0000));
    }

    #[test]
    fn instruction_faults_report_the_instruction() {
        let i = fault_info(vector::INVALID_OPCODE, None, 0x40_1234);
        assert_eq!((i.signal(), i.code, i.addr()), (SIGILL, ILL_ILLOPN, 0x40_1234));
        let i = fault_info(vector::DIVIDE_ERROR, None, 0x40_2000);
        assert_eq!((i.signal(), i.code, i.addr()), (SIGFPE, FPE_INTDIV, 0x40_2000));
        assert_eq!(fault_info(vector::BREAKPOINT, None, 1).signal(), SIGTRAP);
    }

    #[test]
    fn protection_faults_are_sigsegv_from_the_kernel() {
        let i = fault_info(vector::GENERAL_PROTECTION, None, 0x40_0000);
        assert_eq!((i.signal(), i.code, i.addr()), (SIGSEGV, SI_KERNEL, 0));
        assert_eq!(fault_info(200, None, 0).signal(), SIGSEGV);
        assert_eq!(fault_info(vector::ALIGNMENT_CHECK, None, 0).signal(), SIGBUS);
    }
}
