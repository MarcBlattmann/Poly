//! NT personality: the Windows NT system service interface.
//!
//! On x86-64, `ntdll` and `win32u` enter the kernel with `syscall`, putting
//! the service number in `eax` and the arguments in `r10, rdx, r8, r9`, then
//! the stack ([`abi`]). The low 12 bits of the number select the service and
//! bits 12–13 the table (0 = executive services, 1 = win32k GUI services,
//! 3 = Poly's own). Results come back in `rax` as an `NTSTATUS`.
//!
//! Service *numbers* differ between Windows builds, and only our own ntdll
//! calls into this layer, so we pick our own numbering ([`services`], ADR
//! 0063). Behaviour per service comes from Microsoft Learn and public SDK
//! headers until spec sheets under `specs/windows/` exist; every guess is
//! marked "to be confirmed by the spec agent against real Windows".
//!
//! # Layout
//!
//! - [`dispatch_with`]: decode a call and run its service against an
//!   [`NtKernel`] (the kernel's, or a host test's fake).
//! - [`objects`]: handles, directories, symbolic links, events (on
//!   `poly-ntobj`, ADR 0030).
//! - [`vm`], [`memory`]: an NT process's virtual memory (reserve / commit,
//!   protections, guard pages, queries) and its services.
//! - [`section`], [`image`]: sections (pagefile-backed, data file, image)
//!   and views.
//! - [`exec`]: starting an NT process from a PE executable (image mapping,
//!   stack, TEB, PEB), its end, and exceptions.
//! - [`backend`]: what the memory model asks the kernel's memory map to do.
//! - [`pe_writer`]: a minimal PE32+ writer for test programs.
//! - [`registry`]: the `\REGISTRY` key object and the registry services
//!   (on `poly-registry`, ADR 0066).

#![no_std]

extern crate alloc;

pub mod abi;
pub mod backend;
pub mod consts;
pub mod exec;
pub mod image;
pub mod kernel;
pub mod memory;
pub mod objects;
pub mod pe_writer;
pub mod protect;
pub mod registry;
pub mod section;
pub mod services;
pub mod system;
pub mod vm;

#[cfg(test)]
mod tests;

pub use backend::{Backend, SectionSource, ViewMap, ViewSource};
pub use kernel::{NtKernel, UserFault};
pub use protect::HwProt;
pub use services::Service;
pub use system::{NtProcess, NtSystem};

use poly_abi::{Outcome, Personality, SyscallDispatch, SyscallRegs};
use poly_ntobj::NtStatus;

/// `STATUS_NOT_IMPLEMENTED` from the Windows SDK `ntstatus.h`.
pub const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0002;

/// Service tables selected by bits 12–13 of the service number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    Executive,
    Win32k,
    Poly,
    Other(u8),
}

impl Table {
    pub const fn name(self) -> &'static str {
        match self {
            Table::Executive => "nt",
            Table::Win32k => "win32k",
            Table::Poly => "poly",
            Table::Other(_) => "nt-unknown-table",
        }
    }
}

/// Split a service number into its table and index.
pub const fn decode(service: u32) -> (Table, u32) {
    let table = match (service >> 12) & 0x3 {
        0 => Table::Executive,
        1 => Table::Win32k,
        3 => Table::Poly,
        t => Table::Other(t as u8),
    };
    (table, service & 0xfff)
}

/// The personality without kernel services: every call answers
/// `STATUS_NOT_IMPLEMENTED` (bare process records in kernel tests).
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

/// Run the system call in `regs` against `k` and put its `NTSTATUS` in
/// `rax`. A number Poly does not implement answers
/// `STATUS_NOT_IMPLEMENTED` and is reported as
/// [`Outcome::Unimplemented`] (what Windows returns for a service number
/// that does not exist is to be confirmed by the spec agent:
/// `STATUS_INVALID_SYSTEM_SERVICE` is a candidate).
pub fn dispatch_with<K: NtKernel>(k: &mut K, regs: &mut SyscallRegs) -> Outcome {
    let number = regs.rax as u32;
    let Some(service) = Service::from_number(number) else {
        return Nt::dispatch(regs);
    };
    let result = run(k, regs, service);
    regs.rax = u64::from(result.unwrap_or_else(|e| e).0);
    registry::finish(k);
    Outcome::Handled
}

fn run<K: NtKernel>(k: &mut K, regs: &SyscallRegs, service: Service) -> Result<NtStatus, NtStatus> {
    use Service::*;
    match service {
        Close => objects::close(k, regs),
        DuplicateObject => objects::duplicate_object(k, regs),
        QueryObject => objects::query_object(k, regs),
        MakeTemporaryObject => objects::make_temporary(k, regs),
        CreateDirectoryObject => objects::create_directory(k, regs),
        OpenDirectoryObject => objects::open_directory(k, regs),
        QueryDirectoryObject => objects::query_directory(k, regs),
        CreateSymbolicLinkObject => objects::create_symbolic_link(k, regs),
        OpenSymbolicLinkObject => objects::open_symbolic_link(k, regs),
        QuerySymbolicLinkObject => objects::query_symbolic_link(k, regs),
        CreateEvent => objects::create_event(k, regs),
        OpenEvent => objects::open_event(k, regs),
        SetEvent => objects::set_event(k, regs, true),
        ResetEvent => objects::set_event(k, regs, false),
        AllocateVirtualMemory => memory::allocate(k, regs),
        FreeVirtualMemory => memory::free(k, regs),
        ProtectVirtualMemory => memory::protect(k, regs),
        QueryVirtualMemory => memory::query(k, regs),
        CreateSection => memory::create_section(k, regs),
        OpenSection => memory::open_section(k, regs),
        MapViewOfSection => memory::map_view(k, regs),
        UnmapViewOfSection => memory::unmap_view(k, regs),
        TerminateProcess => memory::terminate(k, regs),
        CreateKey => registry::services::create_key(k, regs),
        OpenKey => registry::services::open_key(k, regs),
        OpenKeyEx => registry::services::open_key_ex(k, regs),
        DeleteKey => registry::services::delete_key(k, regs),
        DeleteValueKey => registry::services::delete_value_key(k, regs),
        EnumerateKey => registry::services::enumerate_key(k, regs),
        EnumerateValueKey => registry::services::enumerate_value_key(k, regs),
        QueryKey => registry::services::query_key(k, regs),
        QueryValueKey => registry::services::query_value_key(k, regs),
        SetValueKey => registry::services::set_value_key(k, regs),
        FlushKey => registry::services::flush_key(k, regs),
        NotifyChangeKey => registry::services::notify_change_key(k, regs),
        SetInformationKey => registry::services::set_information_key(k, regs),
        PolyTrace => {
            let values = abi::args::<K, 3>(k, regs)?;
            k.trace(values);
            Ok(poly_ntobj::status::STATUS_SUCCESS)
        }
    }
}
