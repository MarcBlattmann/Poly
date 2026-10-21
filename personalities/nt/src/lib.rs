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

//! - [`io`]: file I/O (ADR 0067): file and device objects over the VFS,
//!   drive letters, share modes, byte-range locks, IRP-style async I/O.
//! - [`registry`]: the `\REGISTRY` key object and the registry services
//!   (on `poly-registry`, ADR 0066).
//! - [`exception`]: CPU exceptions and `NtRaiseException` delivered to the
//!   process's user-mode dispatcher, `NtContinue` (on `poly-seh`, ADR 0068).
//! - [`layout`], [`params`]: the Windows file system layout on `C:`, the
//!   registry values that follow it, and a new process's environment and
//!   process parameters (ADR 0073).

#![no_std]

extern crate alloc;

pub mod abi;
pub mod backend;
pub mod console;
pub mod consts;
pub mod context;
pub mod exception;
pub mod exec;
pub mod image;
pub mod io;
pub mod kernel;
pub mod layout;
pub mod memory;
pub mod objects;
pub mod params;
pub mod pe_writer;
pub mod protect;
pub mod registry;
pub mod section;
pub mod security;
pub mod services;
pub mod sync;
pub mod sysinfo;
pub mod system;
pub mod threads;
pub mod vm;

#[cfg(test)]
mod tests;

pub use backend::{Backend, SectionSource, ViewMap, ViewSource};
pub use context::NtContext;
pub use kernel::{
    Apc, ClientId, NtKernel, NtTimeout, ProcessCreate, ProcessCreated, ThreadCreate, ThreadCreated, UserFault,
};
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
        PulseEvent => sync::pulse_event(k, regs),
        ClearEvent => sync::clear_event(k, regs),
        QueryEvent => sync::query_event(k, regs),
        CreateMutant => sync::create_mutant(k, regs),
        OpenMutant => sync::open_mutant(k, regs),
        ReleaseMutant => sync::release_mutant(k, regs),
        QueryMutant => sync::query_mutant(k, regs),
        CreateSemaphore => sync::create_semaphore(k, regs),
        OpenSemaphore => sync::open_semaphore(k, regs),
        ReleaseSemaphore => sync::release_semaphore(k, regs),
        QuerySemaphore => sync::query_semaphore(k, regs),
        CreateTimer => sync::create_timer(k, regs),
        OpenTimer => sync::open_timer(k, regs),
        SetTimer => sync::set_timer(k, regs),
        CancelTimer => sync::cancel_timer(k, regs),
        QueryTimer => sync::query_timer(k, regs),
        WaitForSingleObject => sync::wait_single(k, regs),
        WaitForMultipleObjects => sync::wait_multiple(k, regs),
        SignalAndWaitForSingleObject => sync::signal_and_wait(k, regs),
        QueueApcThread => sync::queue_apc_thread(k, regs),
        TestAlert => sync::test_alert(k, regs),
        DelayExecution => sync::delay_execution(k, regs),
        YieldExecution => sync::yield_execution(k, regs),
        AllocateVirtualMemory => memory::allocate(k, regs),
        FreeVirtualMemory => memory::free(k, regs),
        ProtectVirtualMemory => memory::protect(k, regs),
        QueryVirtualMemory => memory::query(k, regs),
        CreateSection => memory::create_section(k, regs),
        OpenSection => memory::open_section(k, regs),
        MapViewOfSection => memory::map_view(k, regs),
        UnmapViewOfSection => memory::unmap_view(k, regs),
        FlushVirtualMemory => memory::flush(k, regs),
        TerminateProcess => memory::terminate(k, regs),
        TerminateThread => threads::terminate_thread(k, regs),
        CreateThreadEx => threads::create_thread_ex(k, regs),
        CreateUserProcess => threads::create_user_process(k, regs),
        OpenProcess => threads::open_process(k, regs),
        OpenThread => threads::open_thread(k, regs),
        QueryInformationProcess => threads::query_information_process(k, regs),
        QueryInformationThread => threads::query_information_thread(k, regs),
        SuspendThread => threads::suspend_thread(k, regs),
        ResumeThread => threads::resume_thread(k, regs),
        GetContextThread => threads::get_context_thread(k, regs),
        SetContextThread => threads::set_context_thread(k, regs),

        CreateFile => io::services::create_file(k, regs),
        OpenFile => io::services::open_file(k, regs),
        ReadFile => io::services::read_file(k, regs),
        WriteFile => io::services::write_file(k, regs),
        QueryInformationFile => io::services::query_information_file(k, regs),
        SetInformationFile => io::services::set_information_file(k, regs),
        QueryDirectoryFile => io::services::query_directory_file(k, regs),
        QueryVolumeInformationFile => io::services::query_volume_information_file(k, regs),
        FlushBuffersFile => io::services::flush_buffers_file(k, regs),
        LockFile => io::services::lock_file(k, regs),
        UnlockFile => io::services::unlock_file(k, regs),
        DeleteFile => io::services::delete_file(k, regs),
        CancelIoFile => io::services::cancel_io_file(k, regs),
        CancelIoFileEx => io::services::cancel_io_file_ex(k, regs),
        QueryAttributesFile => io::services::query_attributes_file(k, regs),
        QueryFullAttributesFile => io::services::query_full_attributes_file(k, regs),
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
        PolyResumeApc => Ok(k.resume_apc()),
        RaiseException => exception::raise_exception(k, regs),
        Continue => exception::continue_context(k, regs),
        QuerySystemTime => sysinfo::query_system_time(k, regs),
        QueryPerformanceCounter => sysinfo::query_performance_counter(k, regs),
        QuerySystemInformation => sysinfo::query_system_information(k, regs),
        DeviceIoControlFile => console::services::device_io_control_file(k, regs),
        OpenProcessToken => security::open_process_token_service(k, regs),
        OpenProcessTokenEx => security::open_process_token_ex(k, regs),
        OpenThreadToken | OpenThreadTokenEx => security::open_thread_token(k, regs),
        QueryInformationToken => security::query_information_token(k, regs),
    }
}
