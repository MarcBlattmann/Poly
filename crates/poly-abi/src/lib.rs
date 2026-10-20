//! The contract between the shared kernel core and the three personalities.
//!
//! All three worlds enter the kernel through the same x86-64 `syscall`
//! instruction. The core saves the registers into a [`SyscallRegs`], looks up
//! the calling process's [`Personality`] tag and hands the registers to that
//! personality's dispatcher. Each personality reads arguments and writes its
//! result using its own conventions (Linux: `-errno` in `rax`; NT: an
//! `NTSTATUS`; XNU: carry flag + `errno`, or a `kern_return_t` for Mach traps).

#![no_std]

/// Which OS world a process lives in. Fixed at exec time from the binary
/// format and never changes for the life of the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Personality {
    Linux,
    Nt,
    Xnu,
}

impl Personality {
    pub const ALL: [Personality; 3] = [Personality::Linux, Personality::Nt, Personality::Xnu];

    pub const fn name(self) -> &'static str {
        match self {
            Personality::Linux => "linux",
            Personality::Nt => "nt",
            Personality::Xnu => "xnu",
        }
    }
}

/// Carry flag bit in RFLAGS.
pub const RFLAGS_CF: u64 = 1 << 0;

/// User registers at a `syscall` instruction (x86-64).
///
/// The core fills this on entry and copies it back on exit, so a personality
/// returns values by writing fields. Which registers carry the call number and
/// the arguments depends on the personality, so the struct just names them.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyscallRegs {
    pub rax: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub r10: u64,
    pub r8: u64,
    pub r9: u64,
    /// RFLAGS restored on return to user mode.
    pub rflags: u64,
    /// User stack pointer (NT passes arguments beyond the fourth on the stack).
    pub rsp: u64,
}

/// What a personality did with a call. The core uses this for logging and to
/// feed the API tracer; the user-visible result is already in the registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The call ran (successfully or not) and wrote its result.
    Handled,
    /// No implementation exists yet. The personality has already written its
    /// native "not implemented" result. `table` names the call table
    /// (for example `"bsd"` or `"mach"` on XNU) and `number` the index in it.
    Unimplemented { table: &'static str, number: u64 },
}

/// Implemented by each personality crate.
pub trait SyscallDispatch {
    const PERSONALITY: Personality;

    /// Handle one system call. `regs` holds the caller's registers on entry
    /// and must hold the values to return to user mode on exit.
    fn dispatch(regs: &mut SyscallRegs) -> Outcome;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique() {
        for (i, a) in Personality::ALL.iter().enumerate() {
            for b in &Personality::ALL[i + 1..] {
                assert_ne!(a.name(), b.name());
            }
        }
    }
}
