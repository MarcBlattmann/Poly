//! membarrier(2): memory barriers on the other CPUs that run threads of
//! the caller (or of every process), so threads can order their memory
//! accesses without each issuing a barrier (ADR 0111).
//!
//! The commands and the registration rules are here; the kernel keeps the
//! registrations with the memory (shared by every task that uses it,
//! copied by fork, gone at execve) and issues the barriers
//! ([`LinuxKernel::memory_barrier`]).

use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::LinuxKernel;
use crate::table::{Handler, nr};

/// The calls of this module, for [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::MEMBARRIER, membarrier)];

/// Commands (UAPI `linux/membarrier.h`, `enum membarrier_cmd`).
pub const MEMBARRIER_CMD_QUERY: u32 = 0;
pub const MEMBARRIER_CMD_GLOBAL: u32 = 1 << 0;
pub const MEMBARRIER_CMD_GLOBAL_EXPEDITED: u32 = 1 << 1;
pub const MEMBARRIER_CMD_REGISTER_GLOBAL_EXPEDITED: u32 = 1 << 2;
pub const MEMBARRIER_CMD_PRIVATE_EXPEDITED: u32 = 1 << 3;
pub const MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED: u32 = 1 << 4;
pub const MEMBARRIER_CMD_PRIVATE_EXPEDITED_SYNC_CORE: u32 = 1 << 5;
pub const MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED_SYNC_CORE: u32 = 1 << 6;
pub const MEMBARRIER_CMD_PRIVATE_EXPEDITED_RSEQ: u32 = 1 << 7;
pub const MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED_RSEQ: u32 = 1 << 8;
pub const MEMBARRIER_CMD_GET_REGISTRATIONS: u32 = 1 << 9;

/// `MEMBARRIER_CMD_FLAG_CPU` (`enum membarrier_cmd_flag`): restrict
/// `PRIVATE_EXPEDITED_RSEQ` to one CPU.
pub const MEMBARRIER_CMD_FLAG_CPU: u32 = 1 << 0;

/// Every command Poly supports: what `MEMBARRIER_CMD_QUERY` returns.
pub const SUPPORTED: u32 = MEMBARRIER_CMD_GLOBAL
    | MEMBARRIER_CMD_GLOBAL_EXPEDITED
    | MEMBARRIER_CMD_REGISTER_GLOBAL_EXPEDITED
    | MEMBARRIER_CMD_PRIVATE_EXPEDITED
    | MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED
    | MEMBARRIER_CMD_PRIVATE_EXPEDITED_SYNC_CORE
    | MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED_SYNC_CORE
    | MEMBARRIER_CMD_PRIVATE_EXPEDITED_RSEQ
    | MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED_RSEQ
    | MEMBARRIER_CMD_GET_REGISTRATIONS;

/// What a command asks for, once its arguments are checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Return this value (`QUERY`, `GET_REGISTRATIONS`).
    Value(u64),
    /// Record these registration bits.
    Register(u32),
    /// A barrier on the CPUs running the caller's memory (`true`) or on
    /// every CPU (`false`).
    Barrier { private: bool },
}

/// The registration a private command needs.
fn registration_for(cmd: u32) -> Option<u32> {
    match cmd {
        MEMBARRIER_CMD_PRIVATE_EXPEDITED => Some(MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED),
        MEMBARRIER_CMD_PRIVATE_EXPEDITED_SYNC_CORE => Some(MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED_SYNC_CORE),
        MEMBARRIER_CMD_PRIVATE_EXPEDITED_RSEQ => Some(MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED_RSEQ),
        _ => None,
    }
}

/// Decide command `cmd` with `flags`, given the memory's registrations
/// `registered`: `EINVAL` for anything that is not one supported command
/// or for flags other than `MEMBARRIER_CMD_FLAG_CPU` with
/// `PRIVATE_EXPEDITED_RSEQ`, then `EPERM` for a private command whose
/// registration was not done. The CPU argument is never an error.
pub fn decide(cmd: u32, flags: u32, registered: u32) -> Result<Action, Errno> {
    if cmd != MEMBARRIER_CMD_QUERY && (!cmd.is_power_of_two() || cmd & SUPPORTED == 0) {
        return Err(Errno::EINVAL);
    }
    let allowed_flags = if cmd == MEMBARRIER_CMD_PRIVATE_EXPEDITED_RSEQ { MEMBARRIER_CMD_FLAG_CPU } else { 0 };
    if flags & !allowed_flags != 0 {
        return Err(Errno::EINVAL);
    }
    if let Some(needed) = registration_for(cmd) {
        if registered & needed == 0 {
            return Err(Errno::EPERM);
        }
        return Ok(Action::Barrier { private: true });
    }
    Ok(match cmd {
        MEMBARRIER_CMD_QUERY => Action::Value(u64::from(SUPPORTED)),
        MEMBARRIER_CMD_GET_REGISTRATIONS => Action::Value(u64::from(registered)),
        MEMBARRIER_CMD_GLOBAL | MEMBARRIER_CMD_GLOBAL_EXPEDITED => Action::Barrier { private: false },
        register => Action::Register(register),
    })
}

/// membarrier(cmd, flags, cpu_id): `cmd` and `flags` are `int` and
/// `unsigned int` (the upper halves of the registers are ignored).
pub fn membarrier(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    match decide(a.uint(0), a.uint(1), k.membarrier_registrations())? {
        Action::Value(v) => Ok(v),
        Action::Register(bits) => {
            k.membarrier_register(bits);
            Ok(0)
        }
        Action::Barrier { private } => {
            k.memory_barrier(private);
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_lists_every_command() {
        assert_eq!(decide(0, 0, 0), Ok(Action::Value(0x3ff)));
        assert_eq!(decide(0, 1, 0), Err(Errno::EINVAL), "QUERY takes no flags");
    }

    #[test]
    fn anything_but_one_known_command_is_einval() {
        for cmd in [3, 6, 1024, u32::MAX, 0x8000_0000] {
            assert_eq!(decide(cmd, 0, u32::MAX), Err(Errno::EINVAL), "cmd {cmd:#x}");
        }
    }

    #[test]
    fn private_commands_need_their_registration() {
        assert_eq!(decide(MEMBARRIER_CMD_PRIVATE_EXPEDITED, 0, 0), Err(Errno::EPERM));
        assert_eq!(
            decide(MEMBARRIER_CMD_PRIVATE_EXPEDITED, 0, MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED),
            Ok(Action::Barrier { private: true })
        );
        assert_eq!(
            decide(MEMBARRIER_CMD_PRIVATE_EXPEDITED_SYNC_CORE, 0, MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED),
            Err(Errno::EPERM),
            "each private command has its own registration"
        );
        assert_eq!(decide(MEMBARRIER_CMD_GLOBAL_EXPEDITED, 0, 0), Ok(Action::Barrier { private: false }));
        assert_eq!(decide(MEMBARRIER_CMD_GLOBAL, 0, 0), Ok(Action::Barrier { private: false }));
    }

    #[test]
    fn flags_are_checked_before_the_registration() {
        assert_eq!(decide(MEMBARRIER_CMD_PRIVATE_EXPEDITED, 1, 0), Err(Errno::EINVAL));
        assert_eq!(decide(MEMBARRIER_CMD_GLOBAL, 1, 0), Err(Errno::EINVAL));
        let rseq = MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED_RSEQ;
        assert_eq!(decide(MEMBARRIER_CMD_PRIVATE_EXPEDITED_RSEQ, 1, rseq), Ok(Action::Barrier { private: true }));
        assert_eq!(decide(MEMBARRIER_CMD_PRIVATE_EXPEDITED_RSEQ, 2, rseq), Err(Errno::EINVAL));
        assert_eq!(decide(MEMBARRIER_CMD_PRIVATE_EXPEDITED_RSEQ, 1, 0), Err(Errno::EPERM));
    }

    #[test]
    fn registrations_accumulate_and_read_back() {
        assert_eq!(decide(MEMBARRIER_CMD_REGISTER_GLOBAL_EXPEDITED, 0, 0), Ok(Action::Register(4)));
        assert_eq!(decide(MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED, 0, 4), Ok(Action::Register(16)));
        assert_eq!(decide(MEMBARRIER_CMD_GET_REGISTRATIONS, 0, 4 | 16), Ok(Action::Value(20)));
        assert_eq!(decide(MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED, 1, 0), Err(Errno::EINVAL));
    }
}
