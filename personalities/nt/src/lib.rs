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

//! - [`afd`]: `\Device\Afd`, the sockets of `ws2_32` (ADR 0095).
//! - [`io`]: file I/O (ADR 0067): file and device objects over the VFS,
//!   drive letters, share modes, byte-range locks, IRP-style async I/O.
//! - [`registry`]: the `\REGISTRY` key object and the registry services
//!   (on `poly-registry`, ADR 0066).
//! - [`exception`]: CPU exceptions and `NtRaiseException` delivered to the
//!   process's user-mode dispatcher, `NtContinue` (on `poly-seh`, ADR 0068).
//! - [`layout`], [`params`]: the Windows file system layout on `C:`, the
//!   registry values that follow it, and a new process's environment and
//!   process parameters (ADR 0073).
//! - [`win32k`]: table 1, Poly's own win32k services for `user32`, `gdi32`
//!   and `win32u` (on `poly-win32k`, ADR 0128).
//! - [`wow64`]: 32-bit processes: their address space, layouts and the
//!   conversion of their system calls (ADR 0186).

#![no_std]

extern crate alloc;

pub mod abi;
pub mod afd;
pub mod audio;
pub mod backend;
pub mod consent;
pub mod console;
pub mod consts;
pub mod context;
pub mod diag;
pub mod exception;
pub mod exec;
pub mod ids;
pub mod image;
pub mod imageshare;
pub mod io;
pub mod job;
pub mod kernel;
pub mod layout;
pub mod memory;
pub mod objects;
pub mod oomadj;
pub mod pagemap;
pub mod params;
pub mod pe_writer;
pub mod procattrs;
pub mod proclist;
pub mod protect;
pub mod registry;
pub mod scm;
pub mod section;
pub mod security;
pub mod services;
pub mod sync;
pub mod sysinfo;
pub mod sysproc;
pub mod system;
pub mod threads;
pub mod trace;
pub mod vm;
pub mod win32k;
pub mod winsta;
pub mod wmgen;
pub mod wow64;

#[cfg(test)]
mod tests;

pub use backend::{Backend, SectionSource, ViewMap, ViewSource};
pub use context::NtContext;
pub use kernel::{
    Apc, ClientId, NtKernel, NtTimeout, ProcessCreate, ProcessCreated, ThreadCreate, ThreadCreated, UserFault,
};
pub use poly_kernelbase::mitigation::State as MitigationState;
pub use poly_kernelbase::{cpu::CpuId, kuser};
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
///
/// Never inlined: inlined into the kernel's system call entry, the locals
/// of every service it inlines in turn would stay in that frame for the
/// whole call, and deep calls such as `NtCreateUserProcess` (process
/// start, image mapping, the memory map) would then overflow a 16 KiB
/// kernel stack.
#[inline(never)]
pub fn dispatch_with<K: NtKernel>(k: &mut K, regs: &mut SyscallRegs) -> Outcome {
    let number = regs.rax as u32;
    if decode(number).0 == Table::Win32k {
        return win32k::dispatch(k, regs);
    }
    let Some(service) = Service::from_number(number) else {
        return Nt::dispatch(regs);
    };
    let result = run(k, regs, service);
    regs.rax = u64::from(result.unwrap_or_else(|e| e).0);
    registry::finish(k);
    Outcome::Handled
}

/// A call that reads and changes nothing of the NT state and leaves the kernel nothing to do after
/// it (no wake, no I/O, no output), run on its own: `None` for every other call, which goes to
/// [`dispatch_with`]. The kernel runs these without its locks or its end-of-call work (ADR 0247):
/// `NtQueryPerformanceCounter` is the call Chrome makes most, and taking two kernel locks for a
/// clock reading queued its threads behind each other.
pub fn dispatch_unlocked<K: NtKernel>(k: &mut K, regs: &mut SyscallRegs) -> Option<Outcome> {
    if regs.rax as u32 & 0x3fff != query_performance_counter_number() {
        return None;
    }
    let result = sysinfo::query_performance_counter(k, regs);
    regs.rax = u64::from(result.unwrap_or_else(|e| e).0);
    Some(Outcome::Handled)
}

/// A process's memory calls on its own memory (ADR 0262): `NtAllocateVirtualMemory`,
/// `NtFreeVirtualMemory`, `NtProtectVirtualMemory` and `NtQueryVirtualMemory`
/// (`MemoryBasicInformation`) on `NtCurrentProcess()`, run without [`dispatch_with`]'s end-of-call
/// work and with none of the kernel's: they signal no object, queue no I/O and change no registry
/// key, and that work took the NT state's lock twice for every one of them. `None` for every
/// other call.
///
/// A registry notification completed for this process meanwhile is delivered at the end of its
/// next other call, as before at the end of any call.
pub fn dispatch_own_memory<K: NtKernel>(k: &mut K, regs: &mut SyscallRegs) -> Option<Outcome> {
    use Service::*;
    let number = regs.rax as u32;
    if decode(number).0 == Table::Win32k {
        return None;
    }
    let service = Service::from_number(number)?;
    let [process, _, class] = match service {
        AllocateVirtualMemory | FreeVirtualMemory | ProtectVirtualMemory | QueryVirtualMemory => {
            abi::args::<K, 3>(k, regs).ok()?
        }
        _ => return None,
    };
    if !memory::is_current(process)
        || (service == QueryVirtualMemory && class as u32 != consts::MEMORY_BASIC_INFORMATION_CLASS)
    {
        return None;
    }
    let result = run(k, regs, service);
    regs.rax = u64::from(result.unwrap_or_else(|e| e).0);
    Some(Outcome::Handled)
}

/// `NtQueryPerformanceCounter`'s number (table and index), looked up once.
fn query_performance_counter_number() -> u32 {
    use core::sync::atomic::{AtomicU32, Ordering};
    static NUMBER: AtomicU32 = AtomicU32::new(u32::MAX);
    let n = NUMBER.load(Ordering::Relaxed);
    if n != u32::MAX {
        return n;
    }
    let n = services::all()
        .find(|&&(s, _, _)| s == Service::QueryPerformanceCounter)
        .map_or(u32::MAX - 1, |&(_, n, _)| n & 0x3fff);
    NUMBER.store(n, Ordering::Relaxed);
    n
}

fn run<K: NtKernel>(k: &mut K, regs: &SyscallRegs, service: Service) -> Result<NtStatus, NtStatus> {
    use Service::*;
    match service {
        Close => objects::close(k, regs),
        DuplicateObject => objects::duplicate_object(k, regs),
        QueryObject => objects::query_object(k, regs),
        MakeTemporaryObject => objects::make_temporary(k, regs),
        SetInformationObject => objects::set_information_object(k, regs),
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
        SetInformationProcess => threads::set_information_process(k, regs),
        QueryInformationThread => threads::query_information_thread(k, regs),
        SuspendThread => threads::suspend_thread(k, regs),
        ResumeThread => threads::resume_thread(k, regs),
        SuspendProcess => threads::suspend_process(k, regs),
        ResumeProcess => threads::resume_process(k, regs),
        GetCurrentProcessorNumber => threads::get_current_processor_number(k, regs),
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
        SaveKey => registry::hivefile::save_key(k, regs),
        LoadKey => registry::hivefile::load_key(k, regs),
        UnloadKey => registry::hivefile::unload_key(k, regs),

        PolyTrace => {
            let values = abi::args::<K, 3>(k, regs)?;
            k.trace(values);
            Ok(poly_ntobj::status::STATUS_SUCCESS)
        }
        PolyResumeApc => Ok(k.resume_apc()),
        PolyDebugPrint => {
            let [text, length] = abi::args::<K, 2>(k, regs)?;
            let mut buf = [0u8; 512];
            let n = (length as usize).min(buf.len());
            abi::read(k, text, &mut buf[..n])?;
            k.debug_print(&buf[..n]);
            Ok(poly_ntobj::status::STATUS_SUCCESS)
        }
        PolyRandomBytes => sysinfo::random_bytes(k, regs),
        PolyPhysicalMemory => sysinfo::physical_memory(k, regs),
        PolyServiceCall => scm::service_call(k, regs),
        PolyProcessList => proclist::process_list(k, regs),
        NotifyChangeDirectoryFile => io::notify::notify_change_directory_file(k, regs),
        PolyRequestElevation => consent::request_elevation(k, regs),
        PolyStartSystemProcess => sysproc::start_system_process(k, regs),
        RaiseException => exception::raise_exception(k, regs),
        Continue => exception::continue_context(k, regs),
        QuerySystemTime => sysinfo::query_system_time(k, regs),
        QueryPerformanceCounter => sysinfo::query_performance_counter(k, regs),
        QuerySystemInformation => sysinfo::query_system_information(k, regs),
        AllocateLocallyUniqueId => security::allocate_locally_unique_id(k, regs),
        ReadVirtualMemory => memory::read_virtual(k, regs),
        WriteVirtualMemory => memory::write_virtual(k, regs),
        DeviceIoControlFile => console::services::device_io_control_file(k, regs),
        QuerySection => memory::query_section(k, regs),
        UnmapViewOfSectionEx => memory::unmap_view_ex(k, regs),
        CreateJobObject => job::create_job_object(k, regs),
        OpenJobObject => job::open_job_object(k, regs),
        AssignProcessToJobObject => job::assign_process_to_job_object(k, regs),
        TerminateJobObject => job::terminate_job_object(k, regs),
        QueryInformationJobObject => job::query_information_job_object(k, regs),
        SetInformationJobObject => job::set_information_job_object(k, regs),
        IsProcessInJob => job::is_process_in_job(k, regs),
        FsControlFile => io::npservices::fs_control_file(k, regs),
        CreateNamedPipeFile => io::npservices::create_named_pipe_file(k, regs),
        CancelSynchronousIoFile => io::npservices::cancel_synchronous_io_file(k, regs),
        CreateIoCompletion => io::iocp::create_io_completion(k, regs),
        SetIoCompletion => io::iocp::set_io_completion(k, regs),
        RemoveIoCompletion => io::iocp::remove_io_completion(k, regs),
        RemoveIoCompletionEx => io::iocp::remove_io_completion_ex(k, regs),
        QueryIoCompletion => io::iocp::query_io_completion(k, regs),
        OpenProcessToken => security::open_process_token_service(k, regs),
        OpenProcessTokenEx => security::open_process_token_ex(k, regs),
        OpenThreadToken => security::open_thread_token(k, regs, false),
        OpenThreadTokenEx => security::open_thread_token(k, regs, true),
        QueryInformationToken => security::query_information_token(k, regs),
        AdjustPrivilegesToken => security::adjust_privileges_token(k, regs),
        DuplicateToken => security::duplicate_token(k, regs),
        FilterToken => security::filter_token(k, regs),
        CreateLowBoxToken => security::create_lowbox_token(k, regs),
        AdjustGroupsToken => security::adjust_groups_token(k, regs),
        SetInformationToken => security::set_information_token(k, regs),
        CompareTokens => security::compare_tokens(k, regs),
        PrivilegeCheck => security::privilege_check(k, regs),
        AccessCheck => security::access_check(k, regs),
        QuerySecurityObject => security::query_security_object(k, regs),
        SetSecurityObject => security::set_security_object(k, regs),
        SetInformationThread => security::set_information_thread(k, regs),
    }
}
