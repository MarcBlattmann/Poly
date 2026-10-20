//! NT personality: the Windows NT system service interface.
//!
//! On x86-64, `ntdll` and `win32u` enter the kernel with `syscall`, putting
//! the service number in `eax` and the arguments in `r10, rdx, r8, r9`, then
//! the stack. The low 12 bits of the number select the service and bits
//! 12–13 the table (0 = executive services, 1 = win32k GUI services).
//! Results come back in `rax` as an `NTSTATUS`.
//!
//! Service *numbers* differ between Windows builds, and only our own ntdll
//! calls into this layer, so we pick our own numbering. Behaviour per service
//! comes from spec sheets under `specs/windows/`.

#![no_std]

use poly_abi::{Outcome, Personality, SyscallDispatch, SyscallRegs};

/// `STATUS_NOT_IMPLEMENTED` from the Windows SDK `ntstatus.h`.
pub const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0002;

/// Service tables selected by bits 12–13 of the service number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    Executive,
    Win32k,
    Other(u8),
}

impl Table {
    pub const fn name(self) -> &'static str {
        match self {
            Table::Executive => "nt",
            Table::Win32k => "win32k",
            Table::Other(_) => "nt-unknown-table",
        }
    }
}

/// Split a service number into its table and index.
pub const fn decode(service: u32) -> (Table, u32) {
    let table = match (service >> 12) & 0x3 {
        0 => Table::Executive,
        1 => Table::Win32k,
        t => Table::Other(t as u8),
    };
    (table, service & 0xfff)
}

#[derive(Debug)]
pub struct Nt;

impl SyscallDispatch for Nt {
    const PERSONALITY: Personality = Personality::Nt;

    fn dispatch(regs: &mut SyscallRegs) -> Outcome {
        let (table, index) = decode(regs.rax as u32);
        // Provisional: what real Windows returns for a service that does not
        // exist must come from a spec sheet; until then every call answers
        // STATUS_NOT_IMPLEMENTED.
        regs.rax = STATUS_NOT_IMPLEMENTED as u64;
        Outcome::Unimplemented { table: table.name(), number: index as u64 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_tables() {
        assert_eq!(decode(0x0055), (Table::Executive, 0x55));
        assert_eq!(decode(0x1001), (Table::Win32k, 1));
        assert_eq!(decode(0x2000), (Table::Other(2), 0));
    }

    #[test]
    fn unimplemented_returns_status_not_implemented() {
        let mut r = SyscallRegs { rax: 0x1042, ..Default::default() };
        assert_eq!(Nt::dispatch(&mut r), Outcome::Unimplemented { table: "win32k", number: 0x42 });
        assert_eq!(r.rax, 0xC000_0002);
    }
}
