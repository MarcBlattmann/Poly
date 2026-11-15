//! The 32-bit signature of every NT service (ADR 0186): what each 4-byte argument of a 32-bit
//! caller is, so [`super::thunk`] widens it the right way. The i686 `ntdll`'s `build.rs` reads
//! this table too: a stub's `ret N` pops as many arguments as its row lists (`stdcall`), so the
//! table is the one source of both. One row per service, `(Service::Name, &[kinds])`, the
//! arguments in the order of `services.rs`.
//!
//! The kinds ([`Arg`]):
//!
//! | Kind | Argument | Widened to |
//! |---|---|---|
//! | `V` | `ULONG`, `BOOLEAN`, `ACCESS_MASK`, a class, `SIZE_T`/`ULONG_PTR`/`PVOID` *values*, a 32-bit code address (an APC routine, a start routine: the kernel calls it in compatibility mode) | zero extended |
//! | `H` | `HANDLE` | sign extended (`NtCurrentProcess()` is -1 in both widths) |
//! | `P` | a pointer to data laid out the same in both widths (buffers, `ULONG*`, `LARGE_INTEGER*`, self-relative descriptors), or to data the service lays out in the caller's own width (`NtQueryDirectoryObject`'s entries, `NtCreateUserProcess`'s parameter block, the token and object information classes) | zero extended |
//! | `OH` | `HANDLE*` (out) | a 64-bit copy; the low 32 bits written back |
//! | `OS` | `SIZE_T*`, `PVOID*`, `ULONG_PTR*` (in and out) | a 64-bit copy, zero extended; the low 32 bits written back |
//! | `OA` | `OBJECT_ATTRIBUTES*` (in), with its name | a 64-bit copy |
//! | `US` | `UNICODE_STRING*` (in) | a 64-bit copy |
//! | `UO` | `UNICODE_STRING*` (in and out: the service fills the buffer and sets `Length`) | a 64-bit copy; `Length` and `MaximumLength` written back |
//! | `IO` | `IO_STATUS_BLOCK*` (out, now or when an asynchronous request completes) | the caller's address marked `IOSB32`: the 8-byte block is written there |
//! | `IR` | `IO_STATUS_BLOCK*` naming a request to cancel | marked like `IO`, not read |
//! | `CI` | `CLIENT_ID*` (in) | a 64-bit copy |
//! | `HA` | a `HANDLE` array whose count is the first argument (`NtWaitForMultipleObjects`) | a 64-bit copy |
//! | `CX` | `CONTEXT*` (in): the x86 `CONTEXT` ([`super::context`]) | a 64-bit `CONTEXT` |
//! | `CO` | `CONTEXT*` (in and out, `NtGetContextThread`) | a 64-bit `CONTEXT`; the groups asked for written back in the x86 layout |
//! | `CE` | `FILE_IO_COMPLETION_INFORMATION*` (out), counted by the next argument | 32-byte entries; written back as 16-byte ones |
//! | `ER` | `EXCEPTION_RECORD*` (in) | a 64-bit `EXCEPTION_RECORD` |
//! | `X` | a structure not converted yet: a null pointer passes, anything else makes the call `STATUS_NOT_IMPLEMENTED` | — |

use super::thunk::Arg;
use super::thunk::Arg::*;
use crate::services::Service;
use poly_win32k::services::Win32kService;

/// The 32-bit signatures, one row per service.
pub const SIGNATURES: &[(Service, &[Arg])] = &[
    (Service::Close, &[H]),
    (Service::DuplicateObject, &[H, H, H, OH, V, V, V]),
    (Service::QueryObject, &[H, V, P, V, P]),
    (Service::MakeTemporaryObject, &[H]),
    (Service::SetInformationObject, &[H, V, P, V]),
    (Service::CreateDirectoryObject, &[OH, V, OA]),
    (Service::OpenDirectoryObject, &[OH, V, OA]),
    (Service::QueryDirectoryObject, &[H, P, V, V, V, P, P]),
    (Service::CreateSymbolicLinkObject, &[OH, V, OA, US]),
    (Service::OpenSymbolicLinkObject, &[OH, V, OA]),
    (Service::QuerySymbolicLinkObject, &[H, UO, P]),
    (Service::CreateEvent, &[OH, V, OA, V, V]),
    (Service::OpenEvent, &[OH, V, OA]),
    (Service::SetEvent, &[H, P]),
    (Service::ResetEvent, &[H, P]),
    (Service::PulseEvent, &[H, P]),
    (Service::ClearEvent, &[H]),
    (Service::QueryEvent, &[H, V, P, V, P]),
    (Service::CreateMutant, &[OH, V, OA, V]),
    (Service::OpenMutant, &[OH, V, OA]),
    (Service::ReleaseMutant, &[H, P]),
    (Service::QueryMutant, &[H, V, P, V, P]),
    (Service::CreateSemaphore, &[OH, V, OA, V, V]),
    (Service::OpenSemaphore, &[OH, V, OA]),
    (Service::ReleaseSemaphore, &[H, V, P]),
    (Service::QuerySemaphore, &[H, V, P, V, P]),
    (Service::CreateTimer, &[OH, V, OA, V]),
    (Service::OpenTimer, &[OH, V, OA]),
    (Service::SetTimer, &[H, P, V, V, V, V, P]),
    (Service::CancelTimer, &[H, P]),
    (Service::QueryTimer, &[H, V, P, V, P]),
    (Service::WaitForSingleObject, &[H, V, P]),
    (Service::WaitForMultipleObjects, &[V, HA, V, V, P]),
    (Service::SignalAndWaitForSingleObject, &[H, H, V, P]),
    (Service::QueueApcThread, &[H, V, V, V, V]),
    (Service::TestAlert, &[]),
    (Service::DelayExecution, &[V, P]),
    (Service::YieldExecution, &[]),
    (Service::AllocateVirtualMemory, &[H, OS, V, OS, V, V]),
    (Service::FreeVirtualMemory, &[H, OS, OS, V]),
    (Service::ProtectVirtualMemory, &[H, OS, OS, V, P]),
    (Service::QueryVirtualMemory, &[H, V, V, P, V, OS]),
    (Service::CreateSection, &[OH, V, OA, P, V, V, H]),
    (Service::OpenSection, &[OH, V, OA]),
    (Service::MapViewOfSection, &[H, H, OS, V, V, P, OS, V, V, V]),
    (Service::UnmapViewOfSection, &[H, V]),
    (Service::FlushVirtualMemory, &[H, OS, OS, IO]),
    (Service::ReadVirtualMemory, &[H, V, P, V, OS]),
    (Service::WriteVirtualMemory, &[H, V, P, V, OS]),
    (Service::TerminateProcess, &[H, V]),
    (Service::TerminateThread, &[H, V]),
    (Service::CreateThreadEx, &[OH, V, OA, H, V, V, V, V, V, V, X]),
    (Service::OpenProcess, &[OH, V, OA, CI]),
    (Service::OpenThread, &[OH, V, OA, CI]),
    (Service::SetInformationProcess, &[H, V, P, V]),
    (Service::QueryInformationProcess, &[H, V, P, V, P]),
    (Service::QueryInformationThread, &[H, V, P, V, P]),
    (Service::SuspendThread, &[H, P]),
    (Service::ResumeThread, &[H, P]),
    (Service::GetContextThread, &[H, CO]),
    (Service::SetContextThread, &[H, CX]),
    (Service::CreateUserProcess, &[OH, OH, V, V, OA, OA, V, V, P, V, P]),
    (Service::CreateFile, &[OH, V, OA, IO, P, V, V, V, V, P, V]),
    (Service::OpenFile, &[OH, V, OA, IO, V, V]),
    (Service::ReadFile, &[H, H, V, V, IO, P, V, P, P]),
    (Service::WriteFile, &[H, H, V, V, IO, P, V, P, P]),
    (Service::QueryInformationFile, &[H, IO, P, V, V]),
    (Service::SetInformationFile, &[H, IO, P, V, V]),
    (Service::QueryDirectoryFile, &[H, H, V, V, IO, P, V, V, V, US, V]),
    (Service::QueryVolumeInformationFile, &[H, IO, P, V, V]),
    (Service::FlushBuffersFile, &[H, IO]),
    (Service::LockFile, &[H, H, V, V, IO, P, P, V, V, V]),
    (Service::UnlockFile, &[H, IO, P, P, V]),
    (Service::DeleteFile, &[OA]),
    (Service::CancelIoFile, &[H, IO]),
    (Service::CancelIoFileEx, &[H, IR, IO]),
    (Service::QueryAttributesFile, &[OA, P]),
    (Service::QueryFullAttributesFile, &[OA, P]),
    (Service::PolyTrace, &[V, V, V]),
    (Service::PolyResumeApc, &[]),
    (Service::PolyDebugPrint, &[P, V]),
    (Service::CreateKey, &[OH, V, OA, V, US, V, P]),
    (Service::OpenKey, &[OH, V, OA]),
    (Service::OpenKeyEx, &[OH, V, OA, V]),
    (Service::DeleteKey, &[H]),
    (Service::DeleteValueKey, &[H, US]),
    (Service::EnumerateKey, &[H, V, V, P, V, P]),
    (Service::EnumerateValueKey, &[H, V, V, P, V, P]),
    (Service::QueryKey, &[H, V, P, V, P]),
    (Service::QueryValueKey, &[H, US, V, P, V, P]),
    (Service::SetValueKey, &[H, US, V, V, P, V]),
    (Service::FlushKey, &[H]),
    (Service::NotifyChangeKey, &[H, H, V, V, IO, V, V, P, V, V]),
    (Service::SetInformationKey, &[H, V, P, V]),
    (Service::RaiseException, &[ER, CX, V]),
    (Service::Continue, &[CX, V]),
    (Service::QuerySystemTime, &[P]),
    (Service::AllocateLocallyUniqueId, &[P]),
    (Service::QueryPerformanceCounter, &[P, P]),
    (Service::QuerySystemInformation, &[V, P, V, P]),
    (Service::DeviceIoControlFile, &[H, H, V, V, IO, V, P, V, P, V]),
    (Service::OpenProcessToken, &[H, V, OH]),
    (Service::OpenProcessTokenEx, &[H, V, V, OH]),
    (Service::OpenThreadToken, &[H, V, V, OH]),
    (Service::OpenThreadTokenEx, &[H, V, V, V, OH]),
    (Service::QueryInformationToken, &[H, V, P, V, P]),
    (Service::AdjustPrivilegesToken, &[H, V, P, V, P, P]),
    (Service::DuplicateToken, &[H, V, OA, V, V, OH]),
    (Service::FilterToken, &[H, V, P, P, P, OH]),
    (Service::AdjustGroupsToken, &[H, V, P, V, P, P]),
    (Service::SetInformationToken, &[H, V, P, V]),
    (Service::CompareTokens, &[H, H, P]),
    (Service::PrivilegeCheck, &[H, P, P]),
    (Service::AccessCheck, &[P, H, V, P, P, P, P, P]),
    (Service::QuerySecurityObject, &[H, V, P, V, P]),
    (Service::SetSecurityObject, &[H, V, P]),
    (Service::SetInformationThread, &[H, V, P, V]),
    (Service::FsControlFile, &[H, H, V, V, IO, V, P, V, P, V]),
    (Service::CreateNamedPipeFile, &[OH, V, OA, IO, V, V, V, V, V, V, V, V, V, P]),
    (Service::CancelSynchronousIoFile, &[H, IR, IO]),
    (Service::CreateIoCompletion, &[OH, V, OA, V]),
    (Service::SetIoCompletion, &[H, V, V, V, V]),
    (Service::RemoveIoCompletion, &[H, OS, OS, IO, P]),
    (Service::RemoveIoCompletionEx, &[H, CE, V, P, P, V]),
    (Service::QueryIoCompletion, &[H, V, P, V, P]),
    (Service::CreateJobObject, &[OH, V, OA]),
    (Service::OpenJobObject, &[OH, V, OA]),
    (Service::AssignProcessToJobObject, &[H, H]),
    (Service::TerminateJobObject, &[H, V]),
    (Service::QueryInformationJobObject, &[H, V, P, V, P]),
    (Service::SetInformationJobObject, &[H, V, P, V]),
    (Service::IsProcessInJob, &[H, H]),
    (Service::QuerySection, &[H, V, P, V, OS]),
    (Service::UnmapViewOfSectionEx, &[H, V, V]),
    (Service::SuspendProcess, &[H]),
    (Service::ResumeProcess, &[H]),
];

/// The 32-bit signature of `service`, if it has one.
pub fn signature(service: Service) -> Option<&'static [Arg]> {
    SIGNATURES.iter().find(|(s, _)| *s == service).map(|(_, a)| *a)
}

/// The 32-bit signatures of the win32k services (table 1, `poly_win32k::services`; ADR 0186
/// stage 4). `win32u`'s i686 `build.rs` reads these rows for its stubs' `ret N`. The window
/// manager's requests travel as `poly_win32k::wire` records of fixed-width fields, so a 32-bit
/// `user32` sends the same bytes as a 64-bit one: only the reply length is pointer-sized.
pub const WIN32K_SIGNATURES: &[(Win32kService, &[Arg])] = &[
    // PolyUserCall(request, request_length, reply, reply_capacity, reply_length*)
    (Win32kService::Call, &[P, V, P, V, OS]),
    // PolyUserCommitSurface(hwnd, pixels, width, height, stride)
    (Win32kService::CommitSurface, &[V, P, V, V, V]),
];

/// The 32-bit signature of win32k service `service`.
pub fn win32k_signature(service: Win32kService) -> Option<&'static [Arg]> {
    WIN32K_SIGNATURES.iter().find(|(s, _)| *s == service).map(|(_, a)| *a)
}
