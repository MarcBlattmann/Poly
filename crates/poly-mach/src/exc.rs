//! Mach exceptions (ADR 0238, addendum): the constants of
//! `mach/exception_types.h`, `mach/i386/exception.h` and
//! `mach/i386/thread_status.h`, the thread state flavours the kernel
//! speaks, and the handler tables of tasks, threads and the host.
//!
//! A thread that faults raises an exception: the kernel looks for a handler
//! registered for that exception type, first on the thread, then on its
//! task, then on the host, and sends it an `exception_raise*` (or, with
//! `MACH_EXCEPTION_CODES`, a `mach_exception_raise*`) message
//! (`mach/exc.defs`, `mach/mach_exc.defs`). The handler's reply decides
//! whether the thread goes on (`KERN_SUCCESS`, with the new state for the
//! state behaviours) or the next handler, and last the BSD signal, gets it.

use crate::ipc::PortId;
use alloc::vec::Vec;

/// Exception types (`mach/exception_types.h`).
pub const EXC_BAD_ACCESS: u32 = 1;
pub const EXC_BAD_INSTRUCTION: u32 = 2;
pub const EXC_ARITHMETIC: u32 = 3;
pub const EXC_EMULATION: u32 = 4;
pub const EXC_SOFTWARE: u32 = 5;
pub const EXC_BREAKPOINT: u32 = 6;
pub const EXC_SYSCALL: u32 = 7;
pub const EXC_MACH_SYSCALL: u32 = 8;
pub const EXC_RPC_ALERT: u32 = 9;
pub const EXC_CRASH: u32 = 10;
pub const EXC_RESOURCE: u32 = 11;
pub const EXC_GUARD: u32 = 12;
pub const EXC_CORPSE_NOTIFY: u32 = 13;
/// One past the last type: the size of a handler table.
pub const EXC_TYPES_COUNT: usize = 14;

/// `EXC_MASK_ALL` (`1 << type` for every type but CRASH and CORPSE_NOTIFY;
/// `EXC_MASK_MACHINE` is 0 on x86).
pub const EXC_MASK_ALL: u32 = (1 << EXC_BAD_ACCESS)
    | (1 << EXC_BAD_INSTRUCTION)
    | (1 << EXC_ARITHMETIC)
    | (1 << EXC_EMULATION)
    | (1 << EXC_SOFTWARE)
    | (1 << EXC_BREAKPOINT)
    | (1 << EXC_SYSCALL)
    | (1 << EXC_MACH_SYSCALL)
    | (1 << EXC_RPC_ALERT)
    | (1 << EXC_RESOURCE)
    | (1 << EXC_GUARD);
/// The masks a task or thread may set: every type.
pub const EXC_MASK_VALID: u32 = EXC_MASK_ALL | (1 << EXC_CRASH) | (1 << EXC_CORPSE_NOTIFY);

/// Behaviours (`mach/exception_types.h`).
pub const EXCEPTION_DEFAULT: u32 = 1;
pub const EXCEPTION_STATE: u32 = 2;
pub const EXCEPTION_STATE_IDENTITY: u32 = 3;
pub const EXCEPTION_IDENTITY_PROTECTED: u32 = 4;
pub const EXCEPTION_STATE_IDENTITY_PROTECTED: u32 = 5;
pub const MACH_EXCEPTION_BACKTRACE_PREFERRED: u32 = 0x2000_0000;
pub const MACH_EXCEPTION_ERRORS: u32 = 0x4000_0000;
pub const MACH_EXCEPTION_CODES: u32 = 0x8000_0000;
pub const MACH_EXCEPTION_MASK: u32 = MACH_EXCEPTION_CODES | MACH_EXCEPTION_ERRORS | MACH_EXCEPTION_BACKTRACE_PREFERRED;

/// x86 codes (`mach/i386/exception.h`).
pub const EXC_I386_INVOP: i64 = 1;
pub const EXC_I386_DIV: i64 = 1;
pub const EXC_I386_SGL: i64 = 1;
pub const EXC_I386_BPT: i64 = 2;
pub const EXC_I386_GPFLT: i64 = 13;
/// `KERN_INVALID_ADDRESS` / `KERN_PROTECTION_FAILURE` as `EXC_BAD_ACCESS`
/// codes (`mach/kern_return.h`): an address that is not mapped, an access
/// the protection forbids.
pub const KERN_INVALID_ADDRESS: i64 = 1;
pub const KERN_PROTECTION_FAILURE: i64 = 2;

/// Message ids (`mach/exc.defs` subsystem 2401, `mach/mach_exc.defs` 2405);
/// a reply is the request's id plus 100.
pub const EXCEPTION_RAISE: u32 = 2401;
pub const MACH_EXCEPTION_RAISE: u32 = 2405;

/// Thread state flavours (`mach/i386/thread_status.h`).
pub const X86_THREAD_STATE64: u32 = 4;
pub const X86_FLOAT_STATE64: u32 = 5;
pub const X86_EXCEPTION_STATE64: u32 = 6;
pub const X86_THREAD_STATE: u32 = 7;
pub const THREAD_STATE_NONE: u32 = 13;
/// `x86_THREAD_STATE64_COUNT`: 21 64-bit registers in 32-bit words.
pub const X86_THREAD_STATE64_COUNT: u32 = 42;
/// `x86_EXCEPTION_STATE64_COUNT`: trapno, cpu (16 bits each), err, faultvaddr.
pub const X86_EXCEPTION_STATE64_COUNT: u32 = 4;
/// `x86_THREAD_STATE_COUNT`: a flavour/count header and the union.
pub const X86_THREAD_STATE_COUNT: u32 = 44;

/// The x86-64 user registers in `x86_thread_state64_t` order: rax, rbx,
/// rcx, rdx, rdi, rsi, rbp, rsp, r8..r15, rip, rflags, cs, fs, gs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThreadState64(pub [u64; 21]);

impl ThreadState64 {
    pub const RAX: usize = 0;
    pub const RSP: usize = 7;
    pub const RIP: usize = 16;
    pub const RFLAGS: usize = 17;

    pub fn to_words(&self) -> Vec<u32> {
        self.0.iter().flat_map(|r| [*r as u32, (*r >> 32) as u32]).collect()
    }

    pub fn from_words(w: &[u32]) -> Option<ThreadState64> {
        if w.len() < X86_THREAD_STATE64_COUNT as usize {
            return None;
        }
        let mut s = ThreadState64::default();
        for (i, r) in s.0.iter_mut().enumerate() {
            *r = u64::from(w[2 * i]) | u64::from(w[2 * i + 1]) << 32;
        }
        Some(s)
    }
}

/// `x86_exception_state64_t`: the trap number, the CPU, the error code and
/// the faulting address.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExceptionState64 {
    pub trapno: u16,
    pub cpu: u16,
    pub err: u32,
    pub faultvaddr: u64,
}

impl ExceptionState64 {
    pub fn to_words(&self) -> Vec<u32> {
        alloc::vec![
            u32::from(self.trapno) | u32::from(self.cpu) << 16,
            self.err,
            self.faultvaddr as u32,
            (self.faultvaddr >> 32) as u32
        ]
    }
}

/// A thread's state in a flavour, as words: `None` for a flavour Poly does
/// not have.
pub fn state_words(flavor: u32, regs: &ThreadState64, exc: &ExceptionState64) -> Option<Vec<u32>> {
    match flavor {
        X86_THREAD_STATE64 => Some(regs.to_words()),
        X86_THREAD_STATE => {
            let mut w = alloc::vec![X86_THREAD_STATE64, X86_THREAD_STATE64_COUNT];
            w.extend(regs.to_words());
            Some(w)
        }
        X86_EXCEPTION_STATE64 => Some(exc.to_words()),
        _ => None,
    }
}

/// The registers a state in `flavor` sets, `None` if the flavour or its
/// size is not one Poly sets.
pub fn regs_from_words(flavor: u32, w: &[u32]) -> Option<ThreadState64> {
    match flavor {
        X86_THREAD_STATE64 => ThreadState64::from_words(w),
        X86_THREAD_STATE if w.len() >= 2 && w[0] == X86_THREAD_STATE64 => ThreadState64::from_words(&w[2..]),
        _ => None,
    }
}

/// Whether `flavor` is one a handler may ask for with a state behaviour.
pub fn valid_flavor(flavor: u32) -> bool {
    matches!(flavor, X86_THREAD_STATE64 | X86_THREAD_STATE | X86_EXCEPTION_STATE64 | X86_FLOAT_STATE64)
}

/// A registered handler: the port the kernel holds a send right to, the
/// behaviour (with its flags) and the state flavour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handler {
    pub port: PortId,
    pub behavior: u32,
    pub flavor: u32,
}

/// The handlers of one task, thread or the host, by exception type.
#[derive(Clone, Copy, Debug, Default)]
pub struct HandlerTable(pub [Option<Handler>; EXC_TYPES_COUNT]);

/// Check the arguments of `*_set_exception_ports`: the mask (only exception
/// types), the behaviour (one of the five, with flags) and, for the state
/// behaviours, the flavour. `KERN_INVALID_ARGUMENT` otherwise.
pub fn check_set(mask: u32, behavior: u32, flavor: u32) -> Result<(), u32> {
    const KERN_INVALID_ARGUMENT: u32 = 4;
    if mask & !EXC_MASK_VALID != 0 {
        return Err(KERN_INVALID_ARGUMENT);
    }
    let base = behavior & !MACH_EXCEPTION_MASK;
    if !(EXCEPTION_DEFAULT..=EXCEPTION_STATE_IDENTITY_PROTECTED).contains(&base) {
        return Err(KERN_INVALID_ARGUMENT);
    }
    let wants_state = matches!(base, EXCEPTION_STATE | EXCEPTION_STATE_IDENTITY | EXCEPTION_STATE_IDENTITY_PROTECTED);
    if wants_state && !valid_flavor(flavor) {
        return Err(KERN_INVALID_ARGUMENT);
    }
    Ok(())
}

/// The exception a CPU fault raises on x86-64 and its two codes (as
/// `mach/i386/exception.h` defines them): a page fault is `EXC_BAD_ACCESS`
/// with `KERN_INVALID_ADDRESS` or `KERN_PROTECTION_FAILURE` and the
/// address; #GP is `EXC_BAD_ACCESS` with `EXC_I386_GPFLT`; #UD is
/// `EXC_BAD_INSTRUCTION`; #DE is `EXC_ARITHMETIC`; #BP (int3) and #DB are
/// `EXC_BREAKPOINT`. `None` for a vector that raises nothing.
pub fn x86_exception(vector: u8, address: Option<u64>, mapped: bool) -> Option<(u32, [i64; 2])> {
    Some(match vector {
        0 => (EXC_ARITHMETIC, [EXC_I386_DIV, 0]),
        1 => (EXC_BREAKPOINT, [EXC_I386_SGL, 0]),
        3 => (EXC_BREAKPOINT, [EXC_I386_BPT, 0]),
        6 => (EXC_BAD_INSTRUCTION, [EXC_I386_INVOP, 0]),
        13 => (EXC_BAD_ACCESS, [EXC_I386_GPFLT, 0]),
        14 => {
            let code = if mapped { KERN_PROTECTION_FAILURE } else { KERN_INVALID_ADDRESS };
            (EXC_BAD_ACCESS, [code, address.unwrap_or(0) as i64])
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_arguments() {
        assert_eq!(check_set(1 << EXC_BAD_ACCESS, EXCEPTION_DEFAULT, THREAD_STATE_NONE), Ok(()));
        assert_eq!(
            check_set(EXC_MASK_ALL, EXCEPTION_STATE_IDENTITY | MACH_EXCEPTION_CODES, X86_THREAD_STATE64),
            Ok(())
        );
        assert_eq!(check_set(1 << 20, EXCEPTION_DEFAULT, 0), Err(4));
        assert_eq!(check_set(1 << EXC_BAD_ACCESS, 9, 0), Err(4));
        assert_eq!(check_set(1 << EXC_BAD_ACCESS, EXCEPTION_STATE, 999), Err(4));
    }

    #[test]
    fn states_round_trip() {
        let mut r = ThreadState64::default();
        r.0[ThreadState64::RIP] = 0x1_0000_1234;
        r.0[ThreadState64::RAX] = u64::MAX;
        let w = state_words(X86_THREAD_STATE64, &r, &ExceptionState64::default()).unwrap();
        assert_eq!(w.len(), 42);
        assert_eq!(regs_from_words(X86_THREAD_STATE64, &w), Some(r));
        let u = state_words(X86_THREAD_STATE, &r, &ExceptionState64::default()).unwrap();
        assert_eq!((u.len(), u[0], u[1]), (44, 4, 42));
        assert_eq!(regs_from_words(X86_THREAD_STATE, &u), Some(r));
        assert_eq!(regs_from_words(X86_FLOAT_STATE64, &u), None);
    }

    #[test]
    fn faults_map_to_exceptions() {
        assert_eq!(x86_exception(14, Some(0x10), false), Some((EXC_BAD_ACCESS, [1, 0x10])));
        assert_eq!(x86_exception(14, Some(0x10), true), Some((EXC_BAD_ACCESS, [2, 0x10])));
        assert_eq!(x86_exception(6, None, false), Some((EXC_BAD_INSTRUCTION, [1, 0])));
        assert_eq!(x86_exception(3, None, false), Some((EXC_BREAKPOINT, [2, 0])));
        assert_eq!(x86_exception(0, None, false), Some((EXC_ARITHMETIC, [1, 0])));
        assert_eq!(x86_exception(2, None, false), None);
    }
}
